## Context

OpenACE is a greenfield Rust project building the core layer of an open-source Context Engine SDK. The repository currently contains only planning documents — no source code exists. This design covers the Rust core: AST parsing, triple-backend storage (SQLite + Tantivy + usearch), indexing pipeline, and multi-signal retrieval engine. The PyO3 bridge and Python layer are deferred to a subsequent change.

The architecture was validated through multi-model analysis (Codex backend perspective + Gemini integration perspective), with all ambiguities resolved through structured constraint elimination.

## Goals / Non-Goals

**Goals:**
- Establish a 5-crate Cargo workspace with clean dependency layering
- Deliver a working parse → index → search pipeline for 5 languages
- Achieve all performance targets (50K symbols/sec parse, <50ms graph query, <50ms BM25, <10ms k-NN, <30s full index of 10K files, <500ms incremental update)
- Provide a sync public API surface designed for clean PyO3 binding in Phase 2

**Non-Goals:**
- PyO3 bridge, Python bindings, Python SDK, MCP server
- Embedding generation (vector population deferred to Python layer)
- LSP integration (Phase 2)
- Context builder / output formatting (Phase 3)
- Multi-repository support (Phase 4)
- CLI tool
- Async public API

## Decisions

### D1: 5-Crate Workspace with Shared Core Types

**Decision**: Split into `oc-core`, `oc-parser`, `oc-storage`, `oc-indexer`, `oc-retrieval`.

**Rationale**: The original 4-crate plan had `oc-storage` depending on `oc-parser` for type definitions, creating tight coupling between the storage and parsing layers. Both Codex and Gemini flagged this as a maintainability risk. Introducing `oc-core` for shared types (`CodeSymbol`, `CodeRelation`, enums, errors) eliminates this coupling.

**Dependency graph** (acyclic, layer-safe):
```
oc-core          (no deps)
oc-parser      → oc-core
oc-storage     → oc-core
oc-indexer     → oc-core, oc-parser, oc-storage
oc-retrieval   → oc-core, oc-storage
```

Forbidden edges: `oc-storage → oc-parser`, `oc-retrieval → oc-parser`, `oc-retrieval → oc-indexer`.

**Alternatives considered**:
- 4 crates with types in `oc-parser`: Rejected — couples storage to parser
- Monolithic single crate: Rejected — prevents independent compilation and testing

### D2: Deterministic Symbol IDs via XXH3-128

**Decision**: Symbol IDs are `XXH3-128(repo_id | relative_path | qualified_name | byte_start | byte_end)` stored as `u128`. Field separator is `|` (pipe character).

**Rationale**: Codex recommended deterministic IDs over random UUIDs for stable incremental diffing. With content-addressable IDs, the diff between old and new symbol sets is a simple set operation (old_ids - new_ids = deleted, new_ids - old_ids = added, intersection with different body_hash = modified). Random UUIDs would require qualified_name+span matching logic.

**Alternatives considered**:
- Random UUID v4: Rejected — requires explicit matching logic for incremental diff
- SHA-256: Rejected — slower than XXH3 with no benefit (not cryptographic context)
- XXH3-64: Rejected — higher collision probability at scale

### D3: Sync Public API with Internal Rayon Parallelism

**Decision**: All public API functions are blocking (sync). Internal parallelism uses `rayon` for CPU-bound parsing. No `tokio` runtime in the public API.

**Rationale**: Gemini flagged that async APIs create significant friction for PyO3 binding (requiring `pyo3-asyncio`). Since the primary consumer will be Python via PyO3, a sync API is the pragmatic choice. Rayon handles the parallelism need (file parsing) without exposing async complexity.

**Concurrency model**:
- **Parsing**: `rayon::par_iter` over files → CPU-bound parallel parsing
- **Storage writes**: Sequential through single-writer model
- **File watcher**: Background thread (not async), communicates via `crossbeam-channel`

**Alternatives considered**:
- Tokio async throughout: Rejected — complicates PyO3 bridge significantly
- Sync + tokio for watcher only: Rejected — mixed runtime complexity for minimal gain

### D4: SQLite as Source of Truth with Ordered Multi-Store Writes

**Decision**: SQLite is the authoritative store. Write order: SQLite COMMIT first → Tantivy buffer → usearch update → periodic flush. If SQLite fails, downstream stores are not touched.

**Rationale**: Both models converged on this approach. There is no distributed transaction manager across SQLite, Tantivy, and usearch. Making SQLite the source of truth means crash recovery is simple: on startup, verify SQLite state against Tantivy/usearch and rebuild derived indexes if inconsistent.

**Recovery strategy**:
- On startup: check `user_version` pragma → if mismatch, purge `.openace/` and re-index
- On crash between SQLite commit and Tantivy flush: set consistency flag, re-sync on next startup
- Periodic consistency check: compare SQLite symbol count vs Tantivy doc count

### D5: Batched Writes with Conservative Defaults

**Decision**:
- SQLite: 1000 rows/transaction (bulk), 100 rows/transaction (incremental)
- Tantivy: commit every 500ms or 500 documents (whichever first)
- File watcher: 300ms debounce interval
- SQLite busy_timeout: 5000ms

**Rationale**: Both models flagged per-file Tantivy commits as a critical performance risk (segment accumulation). Conservative batch sizes balance memory usage and throughput. These can be tuned later based on benchmarks.

### D6: Code-Aware Tantivy Tokenizer

**Decision**: Custom `CodeTokenizer` implementing Tantivy's `Tokenizer` trait. Splitting regex: `[A-Z]?[a-z]+|[A-Z]+(?=[A-Z][a-z]|\d|\b)|[A-Z]+|[0-9]+`. All tokens lowercased. Underscores and dunder patterns (`__x__`) stripped to yield inner name.

**Rationale**: Gemini provided detailed edge case analysis. Standard Tantivy tokenizers break on code identifiers. The regex-based approach handles camelCase, PascalCase, snake_case, and acronyms correctly.

**Key edge cases handled**:
- `HTMLParser` → `html`, `parser`
- `parseXMLStream` → `parse`, `xml`, `stream`
- `user_service` → `user`, `service`
- `__init__` → `init`
- `i18n` → `i`, `18`, `n`

### D7: Qualified Name Normalization (Dot-Separated Canonical Form)

**Decision**: Internal canonical form uses dot-separated segments. Language-native forms are accepted on input and restored on output.

**Normalization map**: Rust `::` → `.`, Go `/` → `.`, Python/TS/JS/Java `.` → `.` (identity).

**Rationale**: A universal internal form enables cross-language search without language-specific query logic. The display layer restores native form using the symbol's `language` field.

### D8: On-Disk Storage Layout

**Decision**: All data stored under `<project_root>/.openace/`:
```
.openace/
├── db.sqlite          # SQLite database (WAL mode)
├── tantivy/           # Tantivy index directory
└── vectors.usearch    # usearch HNSW index file
```

**Rationale**: Project-local storage keeps data co-located with the codebase. No global state, no user-level config required. Standard `.` prefix for hidden directory convention.

### D9: Schema Migration via Purge-and-Rebuild

**Decision**: Store schema version in SQLite `PRAGMA user_version`. On version mismatch, delete entire `.openace/` directory and trigger full re-index.

**Rationale**: Phase 1 greenfield — no existing users to migrate. Incremental migration infrastructure adds complexity with zero benefit until there's a deployed user base. Full re-index is fast (<30s for 10K files).

### D10: File Filtering Strategy

**Decision**: Use the `ignore` crate (same engine as ripgrep) for .gitignore-aware file walking. Additional hard-coded skip rules:
- Binary detection: null bytes in first 8KB
- Size limit: >1MB
- Vendor dirs: `vendor/`, `node_modules/`, `third_party/`, `.venv/`, `venv/`
- Generated patterns: `*.generated.*`, `*.min.js`, `*.min.css`, `*_pb2.py`, `*.pb.go`
- Symlinks: not followed
- Hidden dirs: skipped (except `.openace/` which is storage, not source)

### D11: Encoding and Path Normalization

**Decision**:
- File encoding: UTF-8 only. Non-UTF-8 files are skipped with `ParserError::InvalidEncoding`
- File paths: stored relative to project root, forward-slash normalized on all platforms
- Line numbers: 0-indexed (LSP/tree-sitter compatible)
- Byte ranges: UTF-8 byte offsets (Rust native)
- Timestamps: RFC 3339 (ISO 8601) UTC strings

## Risks / Trade-offs

### R1: MSRV Drift via Transitive Dependencies
**Risk**: Codex empirically tested and found that fresh `Cargo.lock` resolution on Rust 1.85.0 fails due to transitive deps (`darling`, `time`) requiring newer Rust.
**Mitigation**: Commit `Cargo.lock` to repository. Pin problematic transitive crates. Add CI job: `cargo +1.85.0 check --locked`. Block merges that break MSRV.

### R2: usearch C++ Build Portability
**Risk**: usearch wraps C++ core requiring C++ compiler at build time. May fail on minimal environments.
**Mitigation**: Document build prerequisites (C++ compiler, CMake). Ensure static linking for future maturin/PyO3 wheels. Decision was made to NOT add a pure-Rust fallback — accept the C++ dependency.

### R3: Tantivy Segment Accumulation
**Risk**: Without proper merge policy, many small commits create many small segments degrading search performance.
**Mitigation**: Batched commits (500ms/500 docs). Configure Tantivy's `LogMergePolicy` with appropriate floor/ceiling segment sizes. Monitor segment count in IndexReport.

### R4: SQLite Write Contention Under Load
**Risk**: Single-writer model means the watcher + manual re-index can contend.
**Mitigation**: Single writer channel — all writes go through one `crossbeam` sender. WAL mode + `busy_timeout=5000ms`. Bounded write queue with backpressure.

### R5: Cross-Store Inconsistency After Crash
**Risk**: Process crash between SQLite commit and Tantivy flush leaves stores diverged.
**Mitigation**: SQLite is source of truth. On startup, compare symbol count in SQLite vs Tantivy doc count. If mismatch, trigger targeted re-sync (not full re-index). Consistency flag in SQLite marks "dirty" state.

### R6: tree-sitter Grammar Version Conflicts
**Risk**: Different grammar crates may pull different `tree-sitter` core versions.
**Mitigation**: Pin all grammar crates to versions compatible with a single `tree-sitter` core version. Use workspace dependency declarations. Test with `cargo tree --duplicates`.

### R7: Large Monorepo Performance at Scale
**Risk**: File scanning and parallel parsing may hit OS limits (file descriptors, memory) on very large repos.
**Mitigation**: Phase 1 targets 10K files. Scale testing deferred to Phase 4. Rayon pool size defaults to CPU count. Memory bounded by batch sizes.

## Open Questions

None — all ambiguities were resolved through multi-model analysis and structured constraint elimination during the planning phase.
