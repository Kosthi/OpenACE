[Root](../../CLAUDE.md) > [crates](./) > **oc-python**

# oc-python

## Module Responsibility

PyO3 bindings that expose the Rust core engine to Python. Compiles as a `cdylib` native extension module (`_openace`), loaded by the Python `openace` package.

## Entry Point

- `src/lib.rs` -- `#[pymodule]` definition, registers all Python-visible classes

## Public API (Python classes)

### EngineBinding (`src/engine.rs`)
- **`EngineBinding(project_root, embedding_dim=None)`** -- Constructor opens `StorageManager`
- **`index_full(repo_root)`** -- Full indexing, returns `PyIndexReport`. Re-opens storage after indexing.
- **`search(text, query_vector=None, limit=None, language=None, file_path=None)`** -- Multi-signal search
- **`find_symbol(name)`** -- Exact name/qualified_name lookup
- **`get_file_outline(path)`** -- List symbols in a file
- **`add_vectors(ids, vectors)`** -- Store embedding vectors by hex symbol IDs
- **`list_symbols_for_embedding(limit, offset)`** -- Paginated symbol listing for embedding backfill
- **`count_symbols()`** -- Total symbol count
- **`flush()`** -- Persist all storage backends

All methods release the GIL via `py.allow_threads()`. The `StorageManager` is wrapped in `Arc<Mutex<>>`.

### Type Conversions (`src/types.rs`)
- **`PySymbol`** -- Python-compatible symbol with string IDs and kind names
- **`PySearchResult`** -- Python-compatible search result with related_symbols
- **`PyIndexReport`** -- Python-compatible indexing report
- **`PyRelation`** -- Python-compatible relation

### WatcherBinding (`src/watcher.rs`)
- Exposes file-watching functionality to Python

## Build

Built via maturin: `maturin develop` (dev) or `maturin develop --release` (optimized).
Configured in root `pyproject.toml` as `module-name = "openace._openace"`.

## Key Dependencies

- `pyo3` (Python bindings)
- `oc-core`, `oc-storage`, `oc-indexer`, `oc-retrieval`

## Related Files

- `Cargo.toml`
- `src/lib.rs`, `src/engine.rs`, `src/types.rs`, `src/watcher.rs`
- Root `pyproject.toml` (maturin config)

## Changelog

- 2026-02-11: Initial module CLAUDE.md created.
