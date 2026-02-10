# Proposal: OpenACE Phase 1 MVP - Rust Core

## Context

### User Need

Build the foundational Rust core of OpenACE - an open-source Context Engine SDK that combines semantic code search, precise symbol navigation (LSP), code dependency graphs, and multi-signal retrieval. This change covers the Rust core layer only: code parser, storage layer (SQLite graph + HNSW vectors + Tantivy full-text), and basic retrieval engine. The PyO3 bridge and Python layer (MCP/embedding/SDK) will follow in a subsequent change.

### Current State

Greenfield project. Only `openace-implementation-plan.md`, `README.md`, and `LICENSE` exist. No Cargo.toml, no source code, no pyproject.toml.

### Discovered Constraints

#### Hard Constraints (Technical Limitations)

- **HC-1: Tantivy immutable segments** - Tantivy uses an immutable segment model. Document updates require delete-by-term + add-document + commit. Frequent commits degrade performance (each creates a new segment). Batch updates with periodic commits are required. Background segment merging reclaims deleted space. Current version: 0.25.0, MSRV: Rust 1.85.0.
- **HC-2: usearch C++ dependency** - usearch Rust crate (v2.21.0) wraps a C++ core. Requires C++ compiler at build time. Supports on-the-fly deletions, disk-mapped views, SIMD-optimized, f16/i8 support. 10x faster than FAISS.
- **HC-3: tree-sitter grammar distribution** - Each language grammar is a separate crate: `tree-sitter-python`, `tree-sitter-typescript`, `tree-sitter-rust`, `tree-sitter-go`, `tree-sitter-java`. TypeScript crate contains both TypeScript and TSX grammars. Grammars compile C source at build time.
- **HC-4: SQLite recursive CTE depth** - Practical limit of ~20-30 hops for k-hop ego-graph traversal on large graphs. Beyond this, performance degrades exponentially.
- **HC-5: HNSW dimension lock-in** - Vector dimension is fixed at index creation. Switching embedding models (e.g., 384 -> 1536 dimensions) requires full vector index rebuild.
- **HC-6: Rust MSRV alignment** - Tantivy 0.25.0 requires Rust 1.85.0. All crates in the workspace must target this MSRV or higher.

#### Soft Constraints (Conventions & Preferences)

- **SC-1: Cargo workspace** - Multi-crate workspace with shared dependencies. Crates: oc-parser, oc-storage, oc-indexer, oc-retrieval, oc-python (oc-python deferred to next change).
- **SC-2: Code-aware tokenization** - Tantivy default tokenizer splits on whitespace/punctuation. Code requires custom tokenizer that splits camelCase/snake_case/PascalCase identifiers.
- **SC-3: Symbol qualified names** - Use dot-separated qualified names: `module.Class.method`. This is the canonical identifier for cross-reference.
- **SC-4: Confidence scoring convention** - tree-sitter relations: confidence 0.7-0.9. LSP relations: confidence 1.0. Storage must preserve and query by confidence.
- **SC-5: File filtering** - Respect .gitignore patterns during file scanning. Also filter binary files, generated files, and vendor directories.

#### Dependencies (Cross-Module)

- **D-1**: oc-storage depends on oc-parser types (CodeSymbol, CodeRelation, SymbolKind, RelationKind)
- **D-2**: oc-indexer depends on oc-parser (for parsing) and oc-storage (for persistence)
- **D-3**: oc-retrieval depends on oc-storage (for querying all three storage backends)
- **D-4**: oc-python (future) depends on all other crates via PyO3

#### Risks

- **R-1: usearch build complexity** - C++ compilation may fail on some platforms without proper toolchain. Mitigation: document build prerequisites, consider fallback to hnsw_rs.
- **R-2: tree-sitter grammar version conflicts** - Different grammar crates may depend on different tree-sitter core versions. Mitigation: pin all grammar crates to compatible versions.
- **R-3: SQLite concurrent writes** - WAL mode helps but BUSY errors can still occur under high load. Mitigation: use connection pooling with retry logic.
- **R-4: Tantivy segment accumulation** - Without proper merge policy, many small commits create many small segments. Mitigation: implement commit batching and configure merge policy.

## Requirements

### R1: Cargo Workspace Setup

**Description**: Initialize Rust workspace with 4 crates (oc-parser, oc-storage, oc-indexer, oc-retrieval) sharing dependencies.

**Scenario**: Given a fresh clone, when `cargo build` is run, then all 4 crates compile without errors with Rust 1.85.0+.

### R2: Code Parser (oc-parser)

**Description**: Multi-language AST parser using tree-sitter. Extracts CodeSymbol and CodeRelation from source files.

**Scenario**: Given a Python file containing classes and functions, when parsed, then all symbols are extracted with correct qualified_name, kind, line_range, signature, doc_comment, and body_hash. Static relations (calls, imports, inherits) are extracted with confidence 0.7-0.9.

**Languages (Phase 1)**: Python, TypeScript/JavaScript, Rust, Go, Java.

**Data structures**: CodeSymbol, CodeRelation, SymbolKind, RelationKind as defined in implementation plan.

### R3: SQLite Graph Storage (oc-storage::graph)

**Description**: SQLite-backed storage for symbols, relations, and file metadata. Supports recursive CTE graph traversal.

**Scenario**: Given 10,000 symbols with 50,000 relations, when a k-hop query (k=3) is executed from a given symbol, then all reachable symbols within 3 hops are returned in <50ms with cycle detection.

**Schema**: symbols, relations, files, repositories tables as defined in plan.

### R4: Vector Index (oc-storage::vector)

**Description**: HNSW vector index using usearch. Supports add, remove, and k-NN search operations.

**Scenario**: Given 50,000 vectors of dimension 384, when a k-NN query (k=10) is executed, then results are returned in <10ms with >90% recall compared to brute-force.

**Configuration**: M=32, ef_construction=200, ef_search=100, cosine distance.

### R5: Full-Text Index (oc-storage::fulltext)

**Description**: Tantivy full-text index with code-aware tokenizer (camelCase/snake_case splitting).

**Scenario**: Given indexed symbols, when searching for "user_service", then results include symbols named "UserService", "user_service", and "userService" (code-aware tokenization).

**Fields**: name, qualified_name, content, file_path, language.

### R6: Indexing Pipeline (oc-indexer)

**Description**: Full-quantity indexing pipeline: scan files -> filter -> parallel parse -> store symbols/relations -> build fulltext index. Vector embedding is deferred to Python layer.

**Scenario**: Given a project with 10,000 source files across 5 languages, when `index()` is called, then all files are parsed in parallel, symbols and relations are stored in SQLite, and full-text index is built. Performance target: >50K symbols/second parsing throughput.

**Note**: Vector index population is deferred - the pipeline prepares symbol data for later embedding via Python. The storage layer exposes an API to add vectors after the fact.

### R7: Basic Retrieval Engine (oc-retrieval)

**Description**: Multi-signal retrieval combining BM25 full-text + symbol exact match + graph expansion. Vector search signal is available but deferred to Python embedding layer.

**Scenario**: Given a query "authentication logic", when search is executed, then BM25 results and exact symbol matches are fused via RRF (k=60). For each hit, k-hop graph expansion (k=2) provides related symbols. Results include file_path, line_range, relevance_score.

### R8: File Watcher Integration (oc-indexer::watcher)

**Description**: notify-rs based file watcher for detecting changes. Computes content hash diff to determine which files need re-indexing.

**Scenario**: Given a running watcher, when a source file is modified, then the change is detected within 1s, content hash is compared, and if changed, the file is queued for incremental re-parsing.

### R9: Incremental Update (oc-indexer::incremental)

**Description**: Incremental update pipeline: re-parse changed file -> diff old vs new symbols -> update SQLite (delete old + insert new) -> update Tantivy (delete old + add new + commit).

**Scenario**: Given a single file change, when incremental update runs, then only the changed file is re-parsed, old symbols are removed and new symbols are inserted across all storage backends. Target latency: <500ms excluding embedding.

## Success Criteria

1. `cargo build` compiles all 4 crates on macOS and Linux with Rust 1.85.0+
2. `cargo test` passes all unit and integration tests
3. Parse throughput: >50K symbols/second for Python/TypeScript files
4. SQLite k-hop query (k=3): <50ms for 10K symbols
5. Tantivy BM25 search: <50ms for 50K documents
6. usearch k-NN (k=10): <10ms for 50K vectors (384-dim)
7. Full index of 10K files: <30 seconds (parsing + SQLite + Tantivy, excluding embeddings)
8. Incremental single-file update: <500ms (excluding embeddings)
9. File watcher detects changes within 1 second

## Out of Scope (Deferred to Subsequent Changes)

- PyO3 bridge and Python bindings (oc-python crate)
- Python embedding manager (AllMiniLM-L6, OpenAI, Jina Code)
- Python MCP server
- Python SDK (Engine API)
- LSP integration (Phase 2)
- Context Builder (Phase 3)
- Multi-repository support (Phase 4)
- CLI tool
