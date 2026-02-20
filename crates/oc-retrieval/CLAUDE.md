[Root](../../CLAUDE.md) > [crates](./) > **oc-retrieval**

# oc-retrieval

## Module Responsibility

Multi-signal retrieval engine that combines BM25 full-text search, vector k-NN search, exact name matching, and graph expansion using Reciprocal Rank Fusion (RRF) scoring.

## Entry Point

- `src/lib.rs` -- re-exports `RetrievalEngine`, `SearchQuery`, `SearchResult`

## Public API

### RetrievalEngine (`src/engine.rs`)
- **`RetrievalEngine::new(storage)`** -- Create engine with reference to `StorageManager`
- **`search(query)`** -- Execute multi-signal search:
  1. **BM25 signal**: Tantivy full-text search with code-aware tokenizer
  2. **Vector signal**: usearch k-NN search (if query vector provided)
  3. **Exact match signal**: SQLite name/qualified_name lookup
  4. **Graph expansion**: k-hop traversal from direct hits (configurable depth, default 2)
  5. **RRF fusion**: Combine all signals via `score += 1/(rank + k)` where k=60
  6. Sort by fused score, truncate to limit, hydrate symbols, attach related_symbols

### SearchQuery (`src/engine.rs`)
- `text` -- search query string
- `limit` -- max results (capped at 100)
- `language_filter` -- optional language restriction
- `file_path_filter` -- optional file path prefix filter
- `enable_graph_expansion` -- toggle graph signal (default: true)
- `graph_depth` -- k-hop depth (default: 2, max: 5)
- `bm25_pool_size` -- BM25 candidate pool (default: 100)
- `exact_match_pool_size` -- exact match pool (default: 50)
- `query_vector` -- optional embedding vector for k-NN
- `vector_pool_size` -- vector candidate pool (default: 50)

### SearchResult (`src/engine.rs`)
- `symbol_id`, `name`, `qualified_name`, `kind`, `file_path`, `line_range`
- `score` -- fused RRF score
- `match_signals` -- list of contributing signals ("bm25", "vector", "exact", "graph")
- `related_symbols` -- graph-expanded neighbor symbols

## Design Notes

- Each signal fails independently (graceful degradation)
- Deduplication by `SymbolId` -- a symbol appearing in multiple signals gets additive RRF scores
- Graph expansion only runs on direct hits, not on graph-discovered symbols
- Results are deterministic for the same query + data (stable sort by score, then by SymbolId)

## Key Dependencies

- `oc-core` (types)
- `oc-storage` (all three backends)

## Tests

Extensive inline tests in `src/engine.rs` covering:
- RRF score properties (monotonicity, positivity, multi-signal > single)
- Deduplication by symbol ID
- Empty results on no data
- Language and file path filtering
- Query defaults and limits
- Vector-only search, multi-signal with vector, graceful degradation on empty vector store
- Integration: multi-signal search with graph expansion
- Score determinism

## Related Files

- `Cargo.toml`
- `src/lib.rs`, `src/engine.rs`, `src/error.rs`

## Changelog

- 2026-02-11: Initial module CLAUDE.md created.
