[Root](../../CLAUDE.md) > [crates](./) > **oc-bench**

# oc-bench

## Module Responsibility

End-to-end tests and Criterion benchmarks for the OpenACE engine. Provides test fixtures and measures performance of all major subsystems.

## Entry Point

- `src/lib.rs` -- test fixture utilities
- `src/fixture.rs` -- synthetic project generation for benchmarks

## Benchmarks

All benchmarks use Criterion and are located in `benches/`:

| Benchmark | File | What it measures |
|-----------|------|------------------|
| parser_throughput | `benches/parser_throughput.rs` | Files parsed per second |
| graph_khop | `benches/graph_khop.rs` | K-hop graph traversal speed |
| fulltext_bm25 | `benches/fulltext_bm25.rs` | BM25 search latency |
| vector_knn | `benches/vector_knn.rs` | Vector k-NN search latency |
| index_full | `benches/index_full.rs` | Full indexing pipeline speed |
| index_incremental | `benches/index_incremental.rs` | Incremental update speed |

Run benchmarks: `cargo bench -p oc-bench`

## E2E Tests

- `tests/e2e_incremental.rs` -- end-to-end incremental indexing test
- `tests/e2e_search.rs` -- end-to-end search test

## Key Dependencies

- All workspace crates (`oc-core`, `oc-parser`, `oc-storage`, `oc-indexer`, `oc-retrieval`)
- `criterion` (benchmarking)
- `tempfile`, `rand` (test infrastructure)

## Related Files

- `Cargo.toml`
- `src/lib.rs`, `src/fixture.rs`
- `benches/`, `tests/`

## Changelog

- 2026-02-11: Initial module CLAUDE.md created.
