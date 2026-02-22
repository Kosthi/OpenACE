# OpenACE Retrieval Evaluation

SWE-bench Lite dev split, 23 instances, single query mode (`problem_statement[:500]`).

## 综合指标对比

| Metric | ace-tool (重测) | v1 | v2 (BM25 fix) | v5 (id extract) | **v6 (MCP)** | v6 vs ace-tool |
|--------|:---:|:---:|:---:|:---:|:---:|:---:|
| File R@1 | 30.4% | 21.7% | 39.1% | 39.1% | **60.9%** | **+30.5pp** |
| File R@5 | 65.2% | 65.2% | 60.9% | 73.9% | **87.0%** | **+21.8pp** |
| File R@10 | 78.3% | 73.9% | 78.3% | 87.0% | **91.3%** | **+13.0pp** |
| File R@20 | 82.6% | 73.9% | 82.6% | 91.3% | **95.7%** | **+13.1pp** |
| File MRR | 0.443 | 0.363 | 0.467 | 0.498 | **0.720** | **+0.277** |
| Found% | 82.6% | 74% | 83% | 91% | **95.7%** | **+13.1pp** |
| Func R@5 | —† | — | — | 33.3% | **46.4%** | — |
| Func R@10 | —† | — | — | 37.7% | **55.1%** | — |
| Func MRR | —† | — | — | 0.283 | **0.410** | — |

OpenACE v6 在所有文件级指标上全面超越 ace-tool，v6 vs ace-tool 差距在 +13pp 至 +30pp 之间。

† ace-tool 返回代码片段（chunk），非符号级别结果，函数级指标无法公平对比。

## Per-Instance 排名对比 (Gold File Rank, 0 = not found)

| Instance | Gold File | v1 | v2 | v5 | **v6** | ace-tool (重测) |
|----------|-----------|:---:|:---:|:---:|:---:|:---:|
| sqlfluff-1625 | L031.py | 3 | 9 | 2 | **1** | 1 |
| sqlfluff-2419 | L060.py | 1 | 1 | 1 | **1** | 1 |
| sqlfluff-1733 | L039.py | 0 | 0 | 0 | 13 | 0 |
| sqlfluff-1517 | helpers.py | 0 | 0 | 0 | **0** | 0 |
| sqlfluff-1763 | linted_file.py | 5 | 15 | 12 | 7 | 11 |
| marshmallow-1359 | fields.py | 6 | 5 | 3 | **1** | 1 |
| marshmallow-1343 | schema.py | 3 | 6 | 1 | **3** | 5 |
| pvlib-1707 | iam.py | 3 | 1 | 1 | **1** | 5 |
| pvlib-1072 | temperature.py | 0 | 5 | 1 | **3** | 5 |
| pvlib-1606 | tools.py | 1 | 1 | 1 | **1** | 3 |
| pvlib-1854 | pvsystem.py | 5 | 3 | 3 | **1** | 2 |
| pvlib-1154 | irradiance.py | 1 | 1 | 1 | **1** | 1 |
| astroid-1978 | raw_building.py | 0 | 0 | 5 | **1** | 9 |
| astroid-1333 | modutils.py | 0 | 7 | 5 | **3** | 0 |
| astroid-1196 | node_classes.py | 1 | 1 | 1 | **1** | 0 |
| astroid-1866 | brain_builtin_inference.py | 0 | 0 | 7 | **2** | 7 |
| astroid-1268 | as_string.py | 1 | 1 | 1 | **2** | 1 |
| pyvista-4315 | grid.py | 5 | 1 | 4 | **1** | 4 |
| pydicom-1694 | dataset.py | 2 | 1 | 1 | **1** | 2 |
| pydicom-1413 | dataelem.py | 5 | 9 | 6 | **1** | 1 |
| pydicom-901 | config.py | 2 | 1 | 1 | **1** | 1 |
| pydicom-1139 | valuerep.py | 4 | 5 | 5 | **1** | 2 |
| pydicom-1256 | jsonrep.py | 8 | 5 | 5 | **3** | 6 |

## ace-tool vs OpenACE 架构差异

| 维度 | ace-tool | OpenACE |
|------|----------|---------|
| **返回粒度** | 代码片段（chunk） | 符号（function/class/method） |
| **行号定位** | 不精确 | 精确 `line_range` |
| **信号透明** | 黑盒 | `match_signals` 可观测 |
| **检索策略** | 单信号（推测: 语义检索） | 多信号 RRF (BM25 + vector + exact + graph) |
| **查询预处理** | 直接使用原文 | 标识符提取 + 按信号路由 |
| **Reranking** | 无/内置 | 可插拔（rule-based / cross-encoder / LLM / API） |
| **索引要求** | 无需预索引 | 需要 `engine.index()` |
| **图遍历** | 无 | 2-hop graph expansion |
| **确定性** | 依赖 API | 除 embedding/reranker API 外确定 |

## 优化历史

### Round 1 — BM25 lenient parsing (v2)

**问题**: Tantivy `parse_query()` 将自然语言当作查询 DSL，遇到 `(` `)` `:` `"` 等字符崩溃。23 个实例中 BM25 信号 100% 失败，OpenACE 实质上只靠 vector 单信号检索。

**修复**: `crates/oc-storage/src/fulltext.rs` — `parse_query()` → `parse_query_lenient()`，同时修复 `search_bm25()` 和 `search_bm25_chunks()`。

**结果**: File R@10: 73.9% → 78.3%, Found%: 74% → 83%

### Round 2 — Post-processing tuning (v4 series) — 全部失败

三项优化独立测试，均未产生可靠改进，全部回滚：

1. **文件级分数聚合** — 大文件中低相关符号导致分数膨胀，"best symbol per file" 策略已是最优
2. **Score-gap cutoff 放宽 (0.6 → 0.4)** — 在噪声范围内无效果
3. **单信号惩罚减弱 (0.85 → 0.95)** — reranker 的 `rerank_score` 使乘法惩罚无意义

### Round 3 — Per-signal query routing (v5)

**问题**: Exact match 信号对自然语言查询完全失效（`WHERE name = ?` 匹配 500 字符长文本，永远命中不了）。BM25 也被自然语言噪声稀释。

**方案**: 从查询中提取代码标识符（CamelCase, snake_case, dotted refs, file path stems, ALL_CAPS），按信号类型路由：

| 信号 | 查询来源 |
|------|---------|
| BM25 | 提取的标识符 prepend 到扩展查询 |
| Exact match | 仅标识符列表 |
| Vector | 原始查询的 embedding（不变） |
| Graph | 无文本输入（从直接命中扩展） |

**修改文件**: `crates/oc-retrieval/src/engine.rs`（`SearchQuery` 新增 `bm25_text`, `exact_queries`）、`crates/oc-python/src/engine.rs`（PyO3 绑定）、`python/openace/engine.py`（`_extract_identifiers()`）

**结果**: File R@10: ~78% → 87.0%, Found%: 83% → 91%, MRR: 0.467 → 0.498

**Commit**: `6eaefff`

### Round 3.5 — Chunk BM25 (v3) — 失败

Chunk BM25 的 chunk-to-symbol 映射过于粗糙，反而降低性能。不启用 chunk 索引。

### Round 4 — MCP 模式搜索 (v6)

**问题**: eval 使用 `engine.search(q, limit=20)` 走 `dedupe_by_file=True` 路径，每文件只保留 1 个符号。而 MCP server 走 `dedupe_by_file=False` + `_aggregate_by_file()` 路径。eval 未测试用户实际使用的搜索路径。

**方案**:
- 提取 `_aggregate_by_file()`, `_apply_file_score_gap()` 到 `python/openace/search_utils.py` 共享模块
- eval 搜索改为 `dedupe_by_file=False`, `pool_size = min(search_limit * 5, 200)`
- 结果经 `_aggregate_by_file()` 按文件分组 + `_apply_file_score_gap()` 截断
- 删除 `ExperimentCondition.dedupe_by_file` 字段

**结果**: File R@1: 39.1% → 60.9%, File R@5: 73.9% → 87.0%, Func R@10: 37.7% → 55.1%

### 关键发现：eval 非确定性

v2_rerun（与 v2 完全相同的代码）产出 R@10=73.9% vs 原始 v2 的 78.3%。这 ±4-9% 的方差源于 SiliconFlow embedding/reranker API 的非确定性。23 个实例下，每实例 ±1-2 rank 变化 = ±4-9% 指标变化。小于 10% 的差异可能是噪声。

### ace-tool 重测说明

ace-tool 指标通过 `mcp__ace-tool__search_context` 重新测试全部 23 个实例获得（查询使用 `problem_statement[:500]`）。per-instance 排名和聚合指标均基于重测数据计算，使用严格路径匹配（无 basename fallback）。

注意 ace-tool API 存在非确定性：同一查询不同时间调用结果可能不同，per-instance 排名可能有 ±1-2 的波动。详细数据见 `eval/ace_tool_results.json`。

## 胜负模式分析

### ace-tool 曾胜出但 v6 已反超

| 实例 | ace-tool | v5 | **v6** | 分析 |
|------|:---:|:---:|:---:|------|
| pydicom-1413 (dataelem.py) | 1 | 8 | **1** | 扩大 pool 后 reranker 候选更多 |
| pydicom-1139 (valuerep.py) | 2 | 5 | **1** | MCP 聚合让高分符号所在文件排序更准 |
| pydicom-1256 (jsonrep.py) | 6 | 5 | **3** | 反超 ace-tool 的 rank 6 |

### OpenACE 持续胜出（标识符匹配型）

| 实例 | ace-tool | v5 | **v6** | 分析 |
|------|:---:|:---:|:---:|------|
| marshmallow-1343 (schema.py) | 5 | 1 | **3** | 仍优于 ace-tool |
| pvlib-1707 (iam.py) | 5 | 1 | **1** | v6 远优于 ace-tool |
| pvlib-1072 (temperature.py) | 5 | 1 | **3** | v6 优于 ace-tool |
| astroid-1978 (raw_building.py) | 9 | 5 | **1** | v6 大幅提升 |
| astroid-1333 (modutils.py) | 0 | 5 | **3** | ace-tool 完全找不到 |

## 当前不足

1. **sqlfluff-1733 (L039.py)** — 问题描述说 "unnecessary whitespace rule"，没有出现 "L039"。需要理解规则编号与描述的映射关系。v6 rank 13，ace-tool 重测 rank 0（均未找到）。
2. **sqlfluff-1517 (helpers.py)** — 通用文件名，BM25/exact match 无法区分。v6 和 ace-tool 均找不到。
3. **astroid-1196 (node_classes.py)** — ace-tool 返回 `astroid/node_classes.py`（兼容 shim），而非 gold 文件 `astroid/nodes/node_classes.py`。
4. **搜索延迟** — embedding + reranker API 调用增加延迟，ace-tool 响应更快。

## 改进方向

| 方向 | 预期影响 | 难度 | 状态 |
|------|---------|------|------|
| ~~BM25 lenient parsing~~ | R@10 +4pp | 低 | **v2 已完成** |
| ~~Per-signal query routing~~ | R@10 +9pp, Found% +8pp | 中 | **v5 已完成** |
| ~~MCP 模式搜索~~ | R@1 +22pp, R@5 +13pp, Func R@10 +17pp | 低 | **v6 已完成** |
| 代码专用 embedding 模型 | 改善 sqlfluff 系列 | 中 | 待做 |
| Graph expansion 分数传播 | 找到间接关联文件 | 低 | 待做 |
| Function-level reranking | 进一步提升 Func R@10 | 中 | 待做 |
| 扩大评测集 (300 instances) | 更可靠的指标 | 低 | 待做 |
| 多次运行取平均 (3-5x) | 消除 API 非确定性噪声 | 低 | 待做 |

## 如何运行

```bash
# 构建 Python 扩展
unset CONDA_PREFIX && uv run maturin develop --release

# 运行评测 (single query mode)
uv run python eval/run_eval.py retrieval \
  --conditions full-siliconflow \
  --subset lite --split dev \
  --embedding siliconflow --reranker siliconflow \
  --embedding-base-url "https://router.tumuer.me/v1" \
  --embedding-api-key "sk-xhJytLyXc2SEGdzFDjRajeWxP9YyS3WZ26TaWzEbvRBWC3rb" \
  --reranker-base-url "https://router.tumuer.me/v1" \
  --reranker-api-key "sk-xhJytLyXc2SEGdzFDjRajeWxP9YyS3WZ26TaWzEbvRBWC3rb" \
  --query-mode single \
  -o eval/output_dev_retrieval_single_vX -v
```

仅修改 Python 代码无需重新构建 Rust。修改 Rust crates 后需运行 `uv run maturin develop --release`。

## 关键文件

| 文件 | 说明 |
|------|------|
| `python/openace/engine.py` | `Engine.search()`, `_extract_identifiers()` (v5) |
| `python/openace/search_utils.py` | `_aggregate_by_file()`, `_apply_file_score_gap()` (v6，MCP/eval 共享) |
| `python/openace/server/app.py` | MCP server，调用 search_utils |
| `crates/oc-retrieval/src/engine.rs` | `SearchQuery` + 多信号 RRF 融合 |
| `crates/oc-python/src/engine.rs` | PyO3 绑定 |
| `crates/oc-storage/src/fulltext.rs` | Tantivy BM25 (v2 lenient fix) |
| `eval/swebench/retrieval_eval.py` | eval 主逻辑 |
| `eval/swebench/context_retrieval.py` | `generate_queries()`, `_dedupe_by_symbol_id()` |
| `tests/test_identifier_extraction.py` | 标识符提取 18 个单元测试 |
