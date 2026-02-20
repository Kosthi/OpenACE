[Root](../../CLAUDE.md) > [crates](./) > **oc-storage**

# oc-storage

## Module Responsibility

Triple-backend storage layer providing a unified `StorageManager` facade over SQLite (graph/relations), Tantivy (full-text search), and usearch (vector k-NN search).

## Entry Point

- `src/lib.rs` -- re-exports all public modules
- `src/manager.rs` -- `StorageManager` is the primary interface

## Public API

### StorageManager (`src/manager.rs`)
- **`StorageManager::open(project_root)`** -- Open/create `.openace/` directory with auto-detected vector dimension
- **`StorageManager::open_with_dimension(project_root, dim)`** -- Open with explicit vector dimension
- **`graph()` / `graph_mut()`** -- Access the `GraphStore`
- **`fulltext()` / `fulltext_mut()`** -- Access the `FullTextStore`
- **`vector()` / `vector_mut()`** -- Access the `VectorStore`
- **`flush()`** -- Persist Tantivy commits and save vector index to disk
- Auto-recovery: detects schema mismatch, SQLite corruption, unusable indexes; purges and rebuilds `.openace/`
- Dimension detection from `.openace/meta.json`; defaults to 384

### GraphStore (`src/graph.rs`)
- SQLite-backed symbol and relation storage with schema versioning (currently v2)
- `insert_symbols()`, `insert_relations()`, `get_symbol()`, `get_symbols_by_file()`, `get_symbols_by_name()`, `get_symbols_by_qualified_name()`
- `traverse_khop()` -- k-hop graph traversal (outgoing, incoming, or both directions)
- `upsert_file()` -- file metadata tracking (content hash, language, size, symbol count)
- `list_symbols()`, `count_symbols()` -- pagination for embedding backfill

### FullTextStore (`src/fulltext.rs`)
- Tantivy-backed BM25 full-text search
- Custom `CodeTokenizer` that splits on camelCase, PascalCase, snake_case, and digit boundaries
- `add_document()`, `search_bm25()`, `commit()`, `clear()`
- Supports language and file-path-prefix filtering
- Auto-batches commits every 500 documents or 500ms

### VectorStore (`src/vector.rs`)
- usearch HNSW index (cosine distance, M=32, ef_construction=200, ef_search=100)
- Surrogate key mapping: `SymbolId` (u128) <-> u64 usearch keys via sidecar `.keys` file
- `add_vector()`, `search_knn()`, `save()`, `dimension()`

## Storage Layout

```
<project>/.openace/
  db.sqlite              -- SQLite: symbols, relations, files, repositories tables
  tantivy/               -- Tantivy full-text index directory
  vectors.usearch        -- usearch HNSW vector index
  vectors.usearch.keys   -- SymbolId <-> u64 key mapping sidecar
  meta.json              -- {"embedding_dim": 384}
```

## Key Dependencies

- `oc-core` (shared types)
- `rusqlite` (bundled SQLite)
- `tantivy` (full-text search)
- `usearch` (HNSW vector index)
- `crossbeam-channel` (batch commit timing)

## Tests

- Extensive inline unit tests in each source file
- `src/manager.rs` contains a full lifecycle integration test covering open, populate, flush, reopen, and corruption recovery

## Related Files

- `Cargo.toml`
- `src/lib.rs`, `src/manager.rs`, `src/graph.rs`, `src/fulltext.rs`, `src/vector.rs`, `src/error.rs`

## Changelog

- 2026-02-11: Initial module CLAUDE.md created.
