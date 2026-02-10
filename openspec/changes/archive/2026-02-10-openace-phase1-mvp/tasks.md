## 1. Workspace & Core Types (oc-core)

- [x] 1.1 Create root `Cargo.toml` workspace with 5 members: oc-core, oc-parser, oc-storage, oc-indexer, oc-retrieval
- [x] 1.2 Configure workspace-level dependencies: xxhash-rust (xxh3), thiserror, serde, serde_json, rayon
- [x] 1.3 Set `rust-version = "1.85.0"` and commit initial `Cargo.lock`
- [x] 1.4 Create `crates/oc-core/` with `Cargo.toml` depending on xxhash-rust and thiserror
- [x] 1.5 Implement `SymbolKind` enum (Function, Method, Class, Struct, Interface, Trait, Module, Package, Variable, Constant, Enum, TypeAlias) with integer serialization
- [x] 1.6 Implement `RelationKind` enum (Calls, Imports, Inherits, Implements, Uses, Contains) with fixed confidence constants (0.8, 0.9, 0.85, 0.85, 0.7, 0.95)
- [x] 1.7 Implement `Language` enum (Python, TypeScript, JavaScript, Rust, Go, Java) with extension mapping
- [x] 1.8 Implement `SymbolId` newtype (u128) with `XXH3-128(repo_id|path|qualified_name|byte_start|byte_end)` generation
- [x] 1.9 Implement `CodeSymbol` struct with all fields per spec (id, name, qualified_name, kind, language, file_path, byte_range, line_range, signature, doc_comment, body_hash)
- [x] 1.10 Implement `CodeRelation` struct with all fields per spec (source_id, target_id, kind, file_path, line, confidence)
- [x] 1.11 Implement qualified name normalization (Rust `::` → `.`, Go `/` → `.`, identity for others) and language-native rendering
- [x] 1.12 Implement error types: `CoreError` with `thiserror`, including `is_retryable()` classification
- [x] 1.13 Add unit tests for SymbolId determinism, qualified name round-trip, confidence constants

## 2. Code Parser (oc-parser)

- [x] 2.1 Create `crates/oc-parser/` with `Cargo.toml` depending on oc-core, tree-sitter, and all 5 grammar crates (python, typescript, rust, go, java)
- [x] 2.2 Implement `ParserRegistry` mapping file extensions to Language/grammar (`.py`, `.ts`, `.tsx`, `.js`, `.jsx`, `.rs`, `.go`, `.java`)
- [x] 2.3 Implement Python visitor: extract Module, Class, Function, Method, Variable, Constant; compute qualified names via scope chain; extract Calls, Imports, Inherits, Contains relations
- [x] 2.4 Implement TypeScript/JavaScript visitor: extract Module, Class, Function, Method, Interface, TypeAlias, Variable, Constant, Enum; handle both TS and TSX grammars
- [x] 2.5 Implement Rust visitor: extract Module, Struct, Enum, Trait, Function, Method, Constant, TypeAlias; normalize `::` to `.` in qualified names
- [x] 2.6 Implement Go visitor: extract Package, Struct, Interface, Function, Method, Constant, Variable; normalize `/` to `.` in qualified names
- [x] 2.7 Implement Java visitor: extract Package, Class, Interface, Enum, Method, Constant
- [x] 2.8 Implement file size check (skip >1MB) and binary detection (null bytes in first 8KB)
- [x] 2.9 Implement anonymous function/lambda skipping across all languages
- [x] 2.10 Implement re-export handling: create Imports relations only, no alias symbols
- [x] 2.11 Implement body_hash computation (XXH3-128 lower 64 bits of symbol body bytes)
- [x] 2.12 Implement `ParserError` with variants: UnsupportedLanguage, FileTooLarge, InvalidEncoding, ParseFailed
- [x] 2.13 Add robustness: ensure no panics on invalid UTF-8, syntax errors, or corrupted input
- [x] 2.14 Add unit tests: Python class/function extraction, TypeScript interface extraction, Rust struct/impl extraction, Go package extraction, Java class extraction
- [x] 2.15 Add unit tests: qualified name computation for nested scopes, anonymous function skipping, re-export relations, file size limit enforcement

## 3. Graph Storage (oc-storage::graph)

- [x] 3.1 Create `crates/oc-storage/` with `Cargo.toml` depending on oc-core, rusqlite (bundled), tantivy, usearch
- [x] 3.2 Implement SQLite initialization: WAL mode, busy_timeout=5000, synchronous=NORMAL, foreign_keys=ON
- [x] 3.3 Implement schema creation: symbols, relations, files, repositories tables with all indexes per spec
- [x] 3.4 Implement schema version management via `PRAGMA user_version`; purge `.openace/` on mismatch
- [x] 3.5 Implement `GraphStore` with batched insert for symbols (1000 rows/tx bulk, 100 rows/tx incremental)
- [x] 3.6 Implement `GraphStore` with batched insert for relations with ON DELETE CASCADE
- [x] 3.7 Implement file metadata CRUD (insert, update, query by path, query by content_hash)
- [x] 3.8 Implement repository metadata CRUD
- [x] 3.9 Implement recursive CTE k-hop traversal with cycle detection, configurable depth (default 2, max 5), fanout limit (50/node), direction (outgoing/incoming/both)
- [x] 3.10 Implement symbol deletion with cascading relation cleanup
- [x] 3.11 Implement file-based symbol query (all symbols for a given file_path)
- [x] 3.12 Add unit tests: symbol round-trip, relation referential integrity, k-hop traversal with cycles, batch transaction splitting, schema version mismatch

## 4. Vector Index (oc-storage::vector)

- [x] 4.1 Implement `VectorStore` wrapping usearch: init with dimension, M=32, ef_construction=200, cosine distance
- [x] 4.2 Implement add_vector(symbol_id, vector), remove_vector(symbol_id), search_knn(query_vector, k, ef_search=100)
- [x] 4.3 Implement dimension mismatch detection (error on wrong dimension input)
- [x] 4.4 Implement persistence: save to `.openace/vectors.usearch`, reload on startup
- [x] 4.5 Implement graceful degradation: return `StorageError::VectorIndexUnavailable` on corruption, allow system to continue without vectors
- [x] 4.6 Add unit tests: add/remove/search round-trip, dimension mismatch error, persistence reload, idempotent add

## 5. Full-Text Index (oc-storage::fulltext)

- [x] 5.1 Implement `CodeTokenizer` for Tantivy: regex-based camelCase/snake_case/PascalCase splitting with lowercasing and dunder stripping
- [x] 5.2 Implement Tantivy schema: symbol_id (STORED), name (TEXT), qualified_name (TEXT), content (TEXT, 10KB truncation), file_path (STRING), language (STRING)
- [x] 5.3 Implement `FullTextStore`: index initialization at `.openace/tantivy/`
- [x] 5.4 Implement batched commit logic: commit on 500ms timeout OR 500 document threshold (whichever first), forced commit on shutdown
- [x] 5.5 Implement add_document, delete_document (by symbol_id), search_bm25(query, limit, filters)
- [x] 5.6 Implement graceful degradation: return `StorageError::FullTextIndexUnavailable` on corruption
- [x] 5.7 Add unit tests: tokenizer edge cases (HTMLParser, parseXMLStream, __init__, i18n, snake_case), cross-case matching, body truncation, batch commit triggers

## 6. Storage Manager (oc-storage facade)

- [x] 6.1 Implement `StorageManager` as unified facade over GraphStore, VectorStore, FullTextStore
- [x] 6.2 Implement `.openace/` directory initialization (create-or-open semantics)
- [x] 6.3 Implement corrupted state detection and purge-and-rebuild logic
- [x] 6.4 Implement `StorageError` enum aggregating graph, vector, and fulltext errors with `is_retryable()`
- [x] 6.5 Add integration test: full lifecycle — init storage, insert symbols, query graph, search fulltext, close and reopen

## 7. Indexing Pipeline (oc-indexer)

- [x] 7.1 Create `crates/oc-indexer/` with `Cargo.toml` depending on oc-core, oc-parser, oc-storage, ignore, rayon
- [x] 7.2 Implement `FileScanner` using `ignore` crate for .gitignore-aware directory walking with hard-coded skip rules (vendor dirs, generated patterns, hidden dirs, symlinks)
- [x] 7.3 Implement full indexing pipeline: scan → filter → parallel parse (rayon) → sequential store (single writer) → Tantivy index
- [x] 7.4 Implement `IndexReport` generation (files scanned, indexed, skipped, failed, total symbols, total relations, duration)
- [x] 7.5 Implement `IndexerError` with pipeline stage context
- [x] 7.6 Add integration test: index a fixture project with mixed languages, verify symbol counts and search results

## 8. File Watcher (oc-indexer::watcher)

- [x] 8.1 Add `notify` crate dependency to oc-indexer
- [x] 8.2 Implement `FileWatcher` using notify-rs with 300ms debounce, running on background thread
- [x] 8.3 Implement content-hash change detection: compare XXH3-128 of changed file vs stored hash, skip metadata-only changes
- [x] 8.4 Implement watcher filtering: apply same rules as FileScanner (gitignore, binary, size, vendor, symlinks)
- [x] 8.5 Implement `WatcherHandle` with `start_watching()` and `stop_watching()` (graceful shutdown with pending event flush)
- [x] 8.6 Add unit tests: debounce coalescing, metadata-only change ignored, filtered path ignored

## 9. Incremental Update (oc-indexer::incremental)

- [x] 9.1 Implement symbol diff algorithm: compare old symbol IDs (from SQLite) vs new symbol IDs (from parser) to classify Added/Removed/Modified/Unchanged
- [x] 9.2 Implement incremental update pipeline: hash check → re-parse → diff → SQLite update (100 rows/tx) → Tantivy update → files table update
- [x] 9.3 Implement cross-store write ordering: SQLite commit first, then Tantivy buffer, with rollback semantics on SQLite failure
- [x] 9.4 Implement file deletion handling: remove all symbols/relations/Tantivy docs/file metadata for deleted files
- [x] 9.5 Implement watcher → incremental pipeline integration: consume change events from watcher channel, process incrementally
- [x] 9.6 Add integration test: modify fixture file, verify only changed symbols updated; delete file, verify complete cleanup
- [x] 9.7 Add convergence test: compare full re-index result vs incremental updates result for identical final state

## 10. Retrieval Engine (oc-retrieval)

- [x] 10.1 Create `crates/oc-retrieval/` with `Cargo.toml` depending on oc-core, oc-storage
- [x] 10.2 Implement `SearchQuery` struct with all fields per spec (text, limit, language_filter, file_path_filter, enable_graph_expansion, graph_depth, pool sizes)
- [x] 10.3 Implement `SearchResult` struct with all fields per spec (symbol_id, name, qualified_name, kind, file_path, line_range, score, match_signals, related_symbols)
- [x] 10.4 Implement BM25 signal: query Tantivy with code-aware tokenized query, collect top-N (default 100)
- [x] 10.5 Implement exact match signal: query SQLite symbols.name and symbols.qualified_name, collect top-N (default 50)
- [x] 10.6 Implement graph expansion: for each hit, run k-hop traversal (default k=2, max k=5, fanout=50)
- [x] 10.7 Implement RRF fusion: `score = Σ 1/(rank_i + 60)`, deduplicate by symbol_id, combine scores from multiple signals
- [x] 10.8 Implement graceful signal degradation: skip unavailable signals (Tantivy down → BM25 skipped), return empty on total failure
- [x] 10.9 Implement `RetrievalError` with signal-level error context
- [x] 10.10 Add unit tests: RRF score computation, deduplication, signal degradation, language filtering
- [x] 10.11 Add integration test: index fixture project → search → verify ranked results contain expected symbols

## 11. End-to-End Integration & Benchmarks

- [x] 11.1 Create fixture project with 5 languages (Python, TypeScript, Rust, Go, Java) containing known symbols, relations, and cross-references
- [x] 11.2 Add end-to-end test: full index → search → verify results across all signal types
- [x] 11.3 Add end-to-end test: full index → modify file → incremental update → verify consistency
- [x] 11.4 Add benchmark: parser throughput (target >50K symbols/sec)
- [x] 11.5 Add benchmark: SQLite k-hop query (target <50ms for 10K symbols, k=3)
- [x] 11.6 Add benchmark: Tantivy BM25 search (target <50ms for 50K docs)
- [x] 11.7 Add benchmark: usearch k-NN (target <10ms for 50K vectors, k=10)
- [x] 11.8 Add benchmark: full index of 10K files (target <30s)
- [x] 11.9 Add benchmark: incremental single-file update (target <500ms)
- [x] 11.10 Verify `cargo build` succeeds on Rust 1.85.0 with committed Cargo.lock
