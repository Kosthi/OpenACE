# OpenACE: 开源 Augment-级 Context Engine SDK

## Context

Augment Code 的核心竞争力是其 Context Engine —— 一个代码库级语义理解和检索系统。它在 SWE-Bench Pro 上以 51.80% 排名第一（同模型下比 Cursor 高 15 题，比 Claude Code 高 17 题），差异化来自 **agent 架构和上下文检索质量**，而非模型本身。

当前开源生态中没有一个项目同时具备：语义代码搜索 + 精确符号导航（LSP）+ 代码依赖图 + 实时增量索引 + 上下文压缩排序。Serena 只有 LSP，Codanna 有语义搜索但嵌入模型弱且无 LSP，两者都缺图检索。

**目标**：构建一个开源 Context Engine SDK，融合 Serena 的 LSP 精确度 + Codanna 的 Rust 性能 + SOTA 论文的算法，通过 MCP 协议暴露给任何 AI Agent/IDE。

## 产品定义

- **定位**: Context Engine SDK/MCP Server —— 代码索引 + 语义检索基础设施
- **技术栈**: Rust 核心（解析/索引/检索/图）+ Python 上层（MCP/嵌入/配置）
- **LSP**: 集成，提供精确符号导航
- **嵌入模型**: 可插拔（本地 Jina Code / API OpenAI / Voyage）

## 整体架构

```
┌──────────────────────────────────────────────────────────────┐
│                     Python 层 (PyO3 Bridge)                   │
│                                                              │
│  ┌─────────────┐  ┌──────────────┐  ┌──────────────────────┐ │
│  │ MCP Server  │  │ Embedding    │  │ Context Builder      │ │
│  │ (stdio/HTTP)│  │ Manager      │  │ (压缩+排序→LLM上下文) │ │
│  │             │  │ (可插拔后端) │  │                      │ │
│  └──────┬──────┘  └──────┬───────┘  └──────────┬───────────┘ │
│         │                │                     │             │
│  ┌──────┴────────────────┴─────────────────────┴───────────┐ │
│  │              Python SDK (High-Level API)                 │ │
│  └──────────────────────┬──────────────────────────────────┘ │
├─────────────────────────┼────────────────────────────────────┤
│                    PyO3 FFI Bridge                            │
├─────────────────────────┼────────────────────────────────────┤
│                     Rust 核心层                               │
│                                                              │
│  ┌──────────────┐  ┌──────────────┐  ┌───────────────────┐  │
│  │ Code Parser  │  │ LSP Client   │  │ File Watcher      │  │
│  │ (tree-sitter)│  │ (tower-lsp)  │  │ (notify-rs)       │  │
│  └──────┬───────┘  └──────┬───────┘  └─────────┬─────────┘  │
│         │                 │                     │            │
│  ┌──────┴─────────────────┴─────────────────────┴─────────┐  │
│  │              Indexing Pipeline (增量)                    │  │
│  └──────────────────────┬─────────────────────────────────┘  │
│                         │                                    │
│  ┌──────────────────────┴─────────────────────────────────┐  │
│  │                    Storage Layer                        │  │
│  │  ┌────────────┐  ┌────────────┐  ┌──────────────────┐  │  │
│  │  │ Graph DB   │  │ Vector Idx │  │ Full-Text Index  │  │  │
│  │  │ (SQLite +  │  │ (HNSW via  │  │ (Tantivy)       │  │  │
│  │  │  邻接表)   │  │  usearch)  │  │                  │  │  │
│  │  └────────────┘  └────────────┘  └──────────────────┘  │  │
│  └────────────────────────────────────────────────────────┘  │
│                                                              │
│  ┌────────────────────────────────────────────────────────┐  │
│  │              Retrieval Engine (多信号融合)               │  │
│  │  向量召回 + BM25 + 图遍历 + 元数据过滤 → RRF 排序       │  │
│  └────────────────────────────────────────────────────────┘  │
└──────────────────────────────────────────────────────────────┘
```

---

## 模块详细设计

### Module 1: Code Parser (Rust)

**职责**: 多语言 AST 解析，提取代码符号和结构关系

**技术选型**:
- tree-sitter (Rust native) 做 AST 解析
- 自定义 visitor 提取符号（函数、类、方法、变量、模块）
- 从 AST 提取静态关系（调用、导入、继承、实现）

**参考**: Codanna 的 tree-sitter 集成（91K 符号/秒）、RepoGraph (ICLR'25) 的行级依赖图

**输出数据结构**:
```rust
struct CodeSymbol {
    id: Uuid,
    name: String,
    qualified_name: String,        // e.g., "module.Class.method"
    kind: SymbolKind,              // Function, Class, Method, Variable, Module, Interface
    language: Language,
    file_path: PathBuf,
    byte_range: Range<usize>,
    line_range: Range<u32>,
    signature: Option<String>,     // 函数签名
    doc_comment: Option<String>,   // 文档注释
    body_hash: u64,                // 内容 hash，用于增量更新检测
}

struct CodeRelation {
    source_id: Uuid,
    target_id: Uuid,
    kind: RelationKind,            // Calls, Imports, Inherits, Implements, Uses, Contains
    file_path: PathBuf,
    line: u32,
    confidence: f32,               // tree-sitter: 0.7-0.9, LSP: 1.0
}

enum SymbolKind {
    Function, Method, Class, Struct, Interface, Trait,
    Module, Package, Variable, Constant, Enum, TypeAlias,
}
```

**语言支持（Phase 1）**: Python, TypeScript/JavaScript, Rust, Go, Java
**语言支持（Phase 2）**: C/C++, C#, Ruby, PHP, Kotlin, Swift + 其他 tree-sitter 支持的语言

### Module 2: LSP Integration (Rust + Python)

**职责**: 通过 LSP 获取精确的类型信息、引用查找、定义跳转

**技术选型**:
- Rust 侧: tower-lsp client 或直接 JSON-RPC over stdio
- Python 侧: 参考 Serena 的 multilspy 库做 LSP 管理
- 按需启动语言服务器（Pyright, tsserver, rust-analyzer, gopls 等）

**LSP 提供的能力（tree-sitter 做不到的）**:
1. `textDocument/definition` — 精确定义跳转（含跨文件）
2. `textDocument/references` — 100% 准确的引用查找
3. `textDocument/hover` — 类型信息和文档
4. `textDocument/documentSymbol` — 文件符号大纲
5. `workspace/symbol` — 全局符号搜索

**关键设计决策**:
- LSP 作为 tree-sitter 的**增强层**，不替代 tree-sitter
- tree-sitter 负责快速、离线的符号提取和基础关系（首次索引）
- LSP 负责精确的引用解析和类型推导（按需查询 + 后台补充）
- 当 LSP 不可用时，优雅降级到 tree-sitter only 模式

**LSP 服务器映射**:
| 语言 | LSP Server | 备注 |
|------|------------|------|
| Python | Pyright | 类型推导最强 |
| TypeScript/JS | tsserver | 原生 |
| Rust | rust-analyzer | 最成熟的 LSP |
| Go | gopls | 官方 |
| Java | Eclipse JDT LS | |
| C/C++ | clangd | |

### Module 3: Storage Layer (Rust)

**职责**: 持久化存储代码图、向量索引、全文索引

#### 3.1 代码图存储 (SQLite)

**为什么是 SQLite**:
- 零外部依赖，嵌入式
- 支持递归 CTE（图遍历）
- WAL 模式支持并发读写
- 50 万文件规模下性能足够（~5000 万节点）

**Schema**:
```sql
-- 代码符号表
CREATE TABLE symbols (
    id          TEXT PRIMARY KEY,   -- UUID
    name        TEXT NOT NULL,
    qualified_name TEXT NOT NULL,
    kind        INTEGER NOT NULL,   -- SymbolKind enum
    language    INTEGER NOT NULL,
    file_path   TEXT NOT NULL,
    line_start  INTEGER NOT NULL,
    line_end    INTEGER NOT NULL,
    byte_start  INTEGER NOT NULL,
    byte_end    INTEGER NOT NULL,
    signature   TEXT,
    doc_comment TEXT,
    body_hash   INTEGER NOT NULL,   -- 增量更新用
    embedding   BLOB,               -- 可选，内联存储小向量
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL
);

CREATE INDEX idx_symbols_file ON symbols(file_path);
CREATE INDEX idx_symbols_name ON symbols(name);
CREATE INDEX idx_symbols_qualified ON symbols(qualified_name);
CREATE INDEX idx_symbols_kind ON symbols(kind);

-- 关系表
CREATE TABLE relations (
    id          TEXT PRIMARY KEY,
    source_id   TEXT NOT NULL REFERENCES symbols(id),
    target_id   TEXT NOT NULL REFERENCES symbols(id),
    kind        INTEGER NOT NULL,   -- RelationKind enum
    file_path   TEXT NOT NULL,
    line        INTEGER NOT NULL,
    confidence  REAL NOT NULL,      -- tree-sitter vs LSP
    UNIQUE(source_id, target_id, kind, file_path, line)
);

CREATE INDEX idx_relations_source ON relations(source_id);
CREATE INDEX idx_relations_target ON relations(target_id);
CREATE INDEX idx_relations_kind ON relations(kind);

-- 文件元数据表（增量索引用）
CREATE TABLE files (
    path        TEXT PRIMARY KEY,
    content_hash INTEGER NOT NULL,  -- 文件内容 hash
    language    INTEGER NOT NULL,
    size_bytes  INTEGER NOT NULL,
    symbol_count INTEGER NOT NULL,
    last_indexed TEXT NOT NULL,
    last_modified TEXT NOT NULL
);

-- 仓库元数据
CREATE TABLE repositories (
    id          TEXT PRIMARY KEY,
    path        TEXT NOT NULL,
    name        TEXT NOT NULL,
    created_at  TEXT NOT NULL
);
```

**图遍历**（递归 CTE，参考 RepoGraph 的 k-hop ego-graph）:
```sql
-- 从 symbol_id 出发，遍历 k 跳关系
WITH RECURSIVE graph_walk(id, depth, path) AS (
    SELECT id, 0, id FROM symbols WHERE id = ?
    UNION ALL
    SELECT r.target_id, gw.depth + 1, gw.path || ',' || r.target_id
    FROM graph_walk gw
    JOIN relations r ON r.source_id = gw.id
    WHERE gw.depth < ?                          -- max_depth
      AND instr(gw.path, r.target_id) = 0       -- cycle detection
)
SELECT DISTINCT s.* FROM graph_walk gw
JOIN symbols s ON s.id = gw.id;
```

#### 3.2 向量索引 (USearch HNSW)

**选型**: usearch (Rust crate)
- HNSW 算法，O(log n) 近似最近邻
- 支持增量 add/remove（不需要全量重建）
- 支持多种距离度量（cosine, L2, inner product）
- 内存映射文件，启动快

**配置**:
- 维度: 可配置（384 for AllMiniLM, 1536 for Jina Code/OpenAI）
- 度量: cosine similarity
- 构建参数: M=32, ef_construction=200, ef_search=100

**索引内容**: 每个 CodeSymbol 生成一个嵌入向量（基于 signature + doc_comment + 代码片段）

#### 3.3 全文索引 (Tantivy)

**选型**: Tantivy (Rust native Lucene 等价物)
- BM25 排序
- 分词：代码感知分词器（camelCase/snake_case 拆分）
- 支持精确匹配和模糊匹配

**索引字段**:
- `name` (STRING): 精确符号名匹配
- `qualified_name` (STRING): 限定名匹配
- `content` (TEXT): 代码体 + 文档注释，分词搜索
- `file_path` (STRING): 路径过滤
- `language` (STRING): 语言过滤

### Module 4: Indexing Pipeline (Rust)

**职责**: 首次全量索引 + 文件变更增量索引

#### 4.1 首次索引流程

```
扫描文件 → 过滤(.gitignore等) → 并行解析(tree-sitter)
  → 提取符号和关系 → 写入 SQLite
  → 生成嵌入向量(批量) → 写入 HNSW
  → 构建全文索引(Tantivy)
  → 后台启动 LSP 补充精确引用
```

**性能目标**（参考 Codanna 91K symbols/sec）:
- 解析: >50K 符号/秒
- 嵌入生成: 取决于模型（本地 1.5B ~2000 文件/分钟）
- 全量索引 5 万文件: <10 分钟（不含嵌入生成）
- 全量索引 5 万文件含嵌入: <30 分钟

#### 4.2 增量索引流程

```
notify-rs 文件监听
  → 检测变更文件
  → 比较 content_hash（files 表）
  → 仅重新解析变更文件
  → diff 旧符号 vs 新符号
  → 增量更新:
      ├── SQLite: DELETE 旧符号/关系 + INSERT 新的
      ├── HNSW: remove 旧向量 + add 新向量
      └── Tantivy: delete 旧文档 + add 新文档
  → 触发 LSP 增量分析（如果 LSP 已启动）
```

**性能目标**:
- 单文件变更增量更新: <500ms（不含嵌入）
- 单文件含嵌入更新: <2s

**参考**: Augment 声称 ~100ms 检索延迟，我们目标 <300ms 全管线

### Module 5: Retrieval Engine (Rust)

**职责**: 多信号融合检索，返回最相关的代码片段

#### 5.1 多信号检索管线

```
用户查询 (自然语言或符号名)
  │
  ├─→ ① 向量召回: query embedding → HNSW top-K (K = limit × 3)
  ├─→ ② BM25 召回: query → Tantivy top-K
  ├─→ ③ 符号精确匹配: query → SQLite name/qualified_name
  ├─→ ④ 图扩展: 对 ①②③ 的命中结果，沿关系边扩展 k-hop
  │
  └─→ RRF (Reciprocal Rank Fusion) 融合排序
      score(item) = Σ 1/(rank_i + k) for each signal
      k = 60 (smoothing constant)
      │
      └─→ 元数据加权:
          - 文件修改 recency bonus
          - 符号类型权重（函数/类 > 变量）
          - 语言匹配 bonus
          │
          └─→ Top-N 结果
```

**参考**:
- RRF 算法: 标准 Information Retrieval 技术
- 多信号融合: Augment 的核心差异化能力
- 图扩展: RepoGraph (ICLR'25) 的 k-hop ego-graph 方法

#### 5.2 LSP 增强查询（精确模式）

当查询是明确的符号名时，直接调用 LSP:
```
"UserService.login" → LSP definition + references + hover
  → 构建精确的上下文（定义 + 所有调用点 + 类型信息）
```

### Module 6: Embedding Manager (Python)

**职责**: 可插拔的嵌入生成，支持本地和 API 模式

**接口**:
```python
class EmbeddingBackend(Protocol):
    def embed_batch(self, texts: list[str]) -> list[list[float]]: ...
    def embed_query(self, query: str) -> list[float]: ...
    @property
    def dimension(self) -> int: ...
```

**内置后端**:

| 后端 | 模型 | 维度 | 速度 | 质量 | 备注 |
|------|------|------|------|------|------|
| `local-small` | AllMiniLM-L6 (ONNX) | 384 | 最快 | 一般 | 零成本，CPU 即可 |
| `local-code` | Jina Code 0.5B (ONNX) | 1536 | 中等 | 好 | 代码专用，需较强 CPU/GPU |
| `local-code-large` | Jina Code 1.5B | 1536 | 慢 | 最好 | 代码专用，需 GPU |
| `openai` | text-embedding-3-small | 1536 | 快(API) | 好 | 需 API key |
| `voyage` | voyage-code-3 | 1024 | 快(API) | 好 | 代码专用 API |

**嵌入内容策略**（参考 Jina Code 训练数据构成）:
```
嵌入文本 = f"{symbol.kind}: {symbol.qualified_name}\n"
           f"Signature: {symbol.signature}\n"
           f"Doc: {symbol.doc_comment}\n"
           f"Body snippet: {symbol.body[:500]}"
```

### Module 7: Context Builder (Python)

**职责**: 将检索结果压缩排序为 LLM 可消费的上下文

**参考**: InlineCoder (arXiv:2601.00376) 的调用链内联、层级摘要方法

**策略**:

```python
def build_context(query: str, results: list[SearchResult],
                  max_tokens: int = 8000) -> str:
    """
    1. 按 RRF 分数排序
    2. 对每个结果，提取:
       - 符号签名 + 文档注释（必选）
       - 函数体（如果 token 预算允许）
       - 调用链上下文（caller/callee 的签名）
    3. 贪心填充直到 token 预算用完
    4. 格式化为结构化上下文:
       - 文件路径 + 行号
       - 代码片段 + 关系说明
    """
```

**上下文格式**:
```
## Relevant Code Context

### File: src/services/auth.py (lines 45-78)
\```python
class AuthService:
    def login(self, username: str, password: str) -> Token:
        """Authenticate user and return JWT token."""
        user = self.user_repo.find_by_username(username)
        ...
\```
> Called by: `api/routes/auth.py:handle_login` (line 23)
> Calls: `UserRepository.find_by_username`, `TokenService.generate`
> References: 5 call sites across 3 files
```

### Module 8: MCP Server (Python)

**职责**: 通过 MCP 协议暴露所有能力

**MCP Tools 定义**:

```python
# 语义搜索 — 自然语言查代码
@tool("semantic_search")
def semantic_search(query: str, language: str = None,
                    limit: int = 10) -> list[CodeResult]: ...

# 符号查找 — 精确查定义
@tool("find_symbol")
def find_symbol(name: str, kind: str = None) -> list[SymbolInfo]: ...

# 引用查找 — 谁调用了这个符号
@tool("find_references")
def find_references(symbol_name: str) -> list[Reference]: ...

# 依赖分析 — 某个符号的调用图
@tool("get_call_graph")
def get_call_graph(symbol_name: str, depth: int = 2,
                   direction: str = "both") -> CallGraph: ...

# 文件概览 — 文件中的所有符号
@tool("get_file_outline")
def get_file_outline(file_path: str) -> list[SymbolInfo]: ...

# 上下文构建 — 为 LLM 构建最优上下文
@tool("build_context")
def build_context(query: str, max_tokens: int = 8000) -> str: ...

# 代码结构 — 项目架构概览
@tool("get_project_structure")
def get_project_structure(depth: int = 2) -> ProjectTree: ...

# 相似代码 — 查找语义相似的代码片段
@tool("find_similar_code")
def find_similar_code(code_snippet: str, limit: int = 5) -> list[CodeResult]: ...
```

**传输方式**: stdio（Claude Code/Cursor）+ HTTP（通用）

### Module 9: Python SDK

**高级 API**:
```python
from openace import Engine

# 初始化
engine = Engine(
    project_path="/path/to/project",
    embedding="local-code",          # 可插拔
    enable_lsp=True,                 # 可选
)

# 索引
engine.index()                       # 首次全量
engine.watch()                       # 启动增量监听

# 检索
results = engine.search("authentication logic", limit=10)
symbol = engine.find_symbol("UserService.login")
refs = engine.find_references("UserService.login")
graph = engine.get_call_graph("UserService.login", depth=2)
context = engine.build_context("fix the login bug", max_tokens=8000)
```

---

## 开发阶段

### Phase 1: MVP (核心检索) — 目标 8 周

**目标**: 可用的语义搜索 + 符号导航 + 基础图检索，通过 MCP 暴露

**构建内容**:
- [ ] Rust: tree-sitter 解析器（Python, TypeScript, Rust, Go, Java）
- [ ] Rust: SQLite 存储层（symbols + relations + files 表）
- [ ] Rust: Tantivy 全文索引（代码感知分词）
- [ ] Rust: USearch HNSW 向量索引
- [ ] Rust: 基础检索引擎（向量 + BM25 + RRF 融合）
- [ ] Python: PyO3 bridge
- [ ] Python: 嵌入管理器（AllMiniLM-L6 本地 + OpenAI API）
- [ ] Python: MCP Server（semantic_search, find_symbol, get_file_outline）
- [ ] Python: 基础 SDK

**参考 SOTA**:
- Codanna: Rust + tree-sitter + Tantivy + HNSW 的工程参考
- Jina Code Embeddings: 嵌入质量基准

**成功指标**:
- 5 万文件项目索引时间 < 15 分钟
- 语义搜索延迟 < 500ms
- 符号查找延迟 < 50ms
- MCP 集成可在 Claude Code 中工作

### Phase 2: LSP + 增量索引 — 目标 6 周

**目标**: 精确符号导航 + 文件变更实时更新

**构建内容**:
- [ ] Rust/Python: LSP 客户端集成（Pyright, tsserver, rust-analyzer, gopls）
- [ ] Rust: LSP 结果合并到 relations 表（confidence=1.0 覆盖 tree-sitter 的 0.7-0.9）
- [ ] Rust: notify-rs 文件监听 + 增量索引管线
- [ ] Rust: 增量向量更新（HNSW add/remove）
- [ ] Rust: 增量全文更新（Tantivy delete/add）
- [ ] Python: MCP 新增 find_references, get_call_graph
- [ ] Python: LSP 启动/管理/降级逻辑

**参考 SOTA**:
- Serena: LSP 集成方式和 multilspy 库
- RepoGraph (ICLR'25): k-hop ego-graph 遍历

**成功指标**:
- find_references 准确率 > 95%（有 LSP 时 100%）
- 单文件变更增量更新 < 2s
- 支持至少 5 种语言的 LSP

### Phase 3: 高级检索 + 上下文构建 — 目标 6 周

**目标**: 多信号融合排序 + 智能上下文压缩 + 代码专用嵌入

**构建内容**:
- [ ] Rust: 多信号融合检索（向量 + BM25 + 图距离 + recency + 符号类型权重）
- [ ] Python: Jina Code 0.5B/1.5B ONNX 本地推理
- [ ] Python: Context Builder（调用链内联 + 贪心 token 填充 + 结构化格式）
- [ ] Python: MCP 新增 build_context, find_similar_code
- [ ] Python: 查询意图分类（符号查询 vs 语义查询 vs 架构查询）
- [ ] 基准测试: 对标 Codanna/Serena 的检索质量

**参考 SOTA**:
- InlineCoder (arXiv:2601.00376): 调用链内联策略
- CodeXEmbed (COLM'25): 嵌入质量基准
- Hierarchical Summarization (ICCSA'25): 层级摘要

**成功指标**:
- 语义搜索 Recall@10 > 0.7（在 CodeSearchNet 子集上）
- build_context 输出质量：在简单 SWE-bench 任务上 resolve rate > 30%
- 全管线延迟 < 500ms

### Phase 4: 规模化 + 生产级 — 目标 8 周

**目标**: 50 万文件、多仓库、生产稳定性

**构建内容**:
- [ ] Rust: 多仓库支持（repository 表 + 跨仓库引用解析）
- [ ] Rust: 大规模优化（内存映射、分片索引、并行解析）
- [ ] Rust: 索引持久化和恢复（崩溃恢复、WAL）
- [ ] Python: 配置管理（.openace.toml 项目配置）
- [ ] Python: CLI 工具（init, index, search, status, config）
- [ ] 文档 + 集成指南（Claude Code, Cursor, Windsurf, Codex）
- [ ] CI/CD + 发布管线（cargo publish + pip install）
- [ ] 性能基准测试套件

**成功指标**:
- 50 万文件索引时间 < 2 小时（含嵌入生成）
- 50 万文件检索延迟 < 500ms
- 内存占用 < 4GB（50 万文件）
- 零崩溃连续运行 72 小时

---

## 关键技术决策

### 1. 为什么是 Rust + Python 而不是纯 Rust 或纯 Python

| 层 | 语言 | 原因 |
|---|---|---|
| AST 解析 | Rust | tree-sitter 是 C/Rust 原生，性能关键 |
| 存储/索引 | Rust | SQLite/Tantivy/USearch 都有优秀 Rust 绑定 |
| 图遍历/检索 | Rust | 性能关键路径，需要 <10ms |
| 嵌入模型 | Python | ML 生态（ONNX Runtime, transformers）Python 最成熟 |
| MCP 协议 | Python | Python MCP SDK 最成熟，社区最大 |
| LSP 管理 | Python | 参考 Serena 的 multilspy，Python 实现更灵活 |

**桥接**: PyO3，Rust 编译为 Python C extension，pip install 一步安装

### 2. 为什么 SQLite 图而不是 Neo4j

- 零外部依赖（嵌入式）
- 50 万符号 + 数百万关系，SQLite + 递归 CTE 性能足够
- WAL 模式支持并发
- 用户不需要安装/配置任何额外服务
- 如果后期需要，可以加 Neo4j 作为可选后端

### 3. 嵌入维度策略

- 默认 384 维（AllMiniLM-L6）：零成本，CPU 即可，快速验证
- 推荐 1536 维（Jina Code / OpenAI）：代码专用，质量显著提升
- 向量索引在初始化时根据嵌入后端自动选择维度
- 切换嵌入后端需要重建向量索引（提供 `reindex --embeddings-only` 命令）

### 4. tree-sitter vs LSP 的协作模式

```
首次索引: tree-sitter only（快速，离线）
         ↓
后台补充: LSP 启动后增量补充精确引用
         ↓
查询时:  优先返回 LSP 结果（confidence=1.0）
         降级到 tree-sitter 结果（confidence=0.7-0.9）
```

---

## 与竞品的差异化

| 能力 | OpenACE | Augment | Serena | Codanna |
|------|-----------|---------|--------|---------|
| 语义搜索 | ✅ 代码专用嵌入 | ✅ 专有模型 | ❌ | ⚠️ 通用模型 |
| 精确导航 | ✅ LSP | ✅ 专有 | ✅ LSP | ⚠️ tree-sitter |
| 代码图 | ✅ SQLite 图 | ✅ 专有 | ❌ | ⚠️ 基础调用图 |
| 全文搜索 | ✅ Tantivy | ✅ | ❌ | ✅ Tantivy |
| 多信号融合 | ✅ RRF | ✅ 专有 | ❌ | ❌ |
| 增量索引 | ✅ notify-rs | ✅ | ✅ LSP 实时 | ❓ |
| 上下文压缩 | ✅ | ✅ | ❌ | ❌ |
| 嵌入可插拔 | ✅ | ❌ 专有 | N/A | ❌ 固定 |
| 开源 | ✅ | ❌ | ✅ | ✅ |
| 零依赖部署 | ✅ | ❌ 需账号 | ⚠️ 需 LSP | ✅ |

**核心差异化**: 唯一一个同时具备 **语义搜索 + LSP 精确导航 + 代码图遍历 + 多信号融合 + 上下文压缩** 的开源方案。

---

## 项目结构

```
OpenACE/
├── Cargo.toml                    # Rust workspace
├── pyproject.toml                # Python package (maturin/PyO3)
├── crates/
│   ├── oc-parser/                # tree-sitter 多语言解析
│   │   ├── src/
│   │   │   ├── lib.rs
│   │   │   ├── languages/        # 各语言的 visitor
│   │   │   │   ├── python.rs
│   │   │   │   ├── typescript.rs
│   │   │   │   ├── rust.rs
│   │   │   │   ├── go.rs
│   │   │   │   └── java.rs
│   │   │   ├── symbol.rs         # CodeSymbol, SymbolKind
│   │   │   └── relation.rs       # CodeRelation, RelationKind
│   │   └── Cargo.toml
│   ├── oc-storage/               # SQLite + HNSW + Tantivy
│   │   ├── src/
│   │   │   ├── lib.rs
│   │   │   ├── graph.rs          # SQLite 图存储 + CTE 遍历
│   │   │   ├── vector.rs         # USearch HNSW 封装
│   │   │   ├── fulltext.rs       # Tantivy 全文索引
│   │   │   └── schema.rs         # DDL + migrations
│   │   └── Cargo.toml
│   ├── oc-indexer/               # 索引管线 (全量 + 增量)
│   │   ├── src/
│   │   │   ├── lib.rs
│   │   │   ├── pipeline.rs       # 全量索引管线
│   │   │   ├── incremental.rs    # 增量更新
│   │   │   └── watcher.rs        # notify-rs 文件监听
│   │   └── Cargo.toml
│   ├── oc-retrieval/             # 多信号检索引擎
│   │   ├── src/
│   │   │   ├── lib.rs
│   │   │   ├── engine.rs         # 统一检索接口
│   │   │   ├── vector_search.rs
│   │   │   ├── text_search.rs
│   │   │   ├── graph_search.rs
│   │   │   └── fusion.rs         # RRF 多信号融合
│   │   └── Cargo.toml
│   └── oc-python/                # PyO3 绑定
│       ├── src/lib.rs
│       └── Cargo.toml
├── python/
│   └── openace/
│       ├── __init__.py           # 导出 Engine
│       ├── engine.py             # 高级 SDK API
│       ├── embedding/
│       │   ├── __init__.py
│       │   ├── base.py           # EmbeddingBackend Protocol
│       │   ├── local_small.py    # AllMiniLM-L6 ONNX
│       │   ├── local_code.py     # Jina Code ONNX
│       │   ├── openai.py         # OpenAI API
│       │   └── voyage.py         # Voyage API
│       ├── lsp/
│       │   ├── __init__.py
│       │   ├── manager.py        # LSP 服务器生命周期管理
│       │   ├── client.py         # LSP JSON-RPC 客户端
│       │   └── servers.py        # 各语言 LSP 配置
│       ├── context/
│       │   ├── __init__.py
│       │   └── builder.py        # 上下文构建 + 压缩
│       ├── mcp/
│       │   ├── __init__.py
│       │   └── server.py         # MCP Server (stdio + HTTP)
│       └── cli/
│           ├── __init__.py
│           └── main.py           # CLI: init, index, search, serve
├── tests/
│   ├── rust/                     # Rust 单元测试
│   └── python/                   # Python 集成测试
├── benchmarks/                   # 性能基准测试
└── docs/                         # 文档
```

---

## 总成本估算

| Phase | 人力 | 周期 | 累计成本 (按 $200/hr) |
|-------|------|------|-----------------------|
| Phase 1: MVP | 2 人 | 8 周 | ~$128K |
| Phase 2: LSP + 增量 | 2 人 | 6 周 | ~$224K |
| Phase 3: 高级检索 | 2-3 人 | 6 周 | ~$344K |
| Phase 4: 规模化 | 3 人 | 8 周 | ~$536K |
| **总计** | | **28 周** | **~$536K** |

---

## 验证方案

### Phase 1 验证
1. 在一个中型开源项目（如 FastAPI, ~200 文件）上运行全量索引
2. 手动构造 20 个查询（10 个语义 + 10 个符号），评估 Recall@10
3. 将 MCP Server 接入 Claude Code，执行 5 个真实编码任务

### Phase 2 验证
1. 对比 LSP vs tree-sitter only 的引用查找准确率
2. 修改文件后测量增量更新延迟
3. 在 Claude Code 中执行 find_references，与 Serena 对比

### Phase 3 验证
1. 在 CodeSearchNet 子集上测量 Recall@10
2. 构造 10 个 SWE-bench 级任务，测量 build_context 输出质量
3. 对比不同嵌入模型（AllMiniLM vs Jina Code vs OpenAI）的检索质量

### Phase 4 验证
1. 在 Linux kernel / Chromium 级大仓库上测试索引时间和内存
2. 72 小时连续运行稳定性测试
3. 社区 beta 测试反馈
