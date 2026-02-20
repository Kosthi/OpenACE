[Root](../../CLAUDE.md) > [crates](./) > **oc-indexer**

# oc-indexer

## Module Responsibility

Indexing pipeline for OpenACE: full project indexing, incremental file updates, file scanning, and real-time file watching. Orchestrates parsing and storage.

## Entry Point

- `src/lib.rs` -- re-exports `index`, `scan_files`, `start_watching`, incremental operations, config/report types

## Public API

### Full Indexing (`src/pipeline.rs`)
- **`index(project_path, config)`** -- Full indexing pipeline: scan -> filter -> parallel parse (rayon) -> sequential store -> Tantivy index -> flush. Returns `IndexReport`.
- Pipeline clears existing data before reindex to prevent ghost entries.
- Files > 1 MB are skipped; binary files are detected and skipped.
- Relations are filtered to known source symbols (dangling targets allowed).

### File Scanning (`src/scanner.rs`)
- **`scan_files(project_root)`** -- .gitignore-aware file walker using the `ignore` crate. Skips vendor dirs (`node_modules`, `vendor`, `.venv`, `third_party`), generated files (`.generated.`, `.min.js`, `_pb2.py`, `.pb.go`), hidden dirs, and symlinks.

### Incremental Updates (`src/incremental.rs`)
- **`diff_symbols(old, new)`** -- Computes added/removed/modified/unchanged symbol sets using deterministic IDs and body hashes.
- **`update_file(storage, repo_id, rel_path, content)`** -- Re-parse a single file and apply diff to storage.
- **`delete_file(storage, rel_path)`** -- Remove all symbols/relations for a deleted file.
- **`process_events(storage, repo_id, project_root, events)`** -- Process a batch of `ChangeEvent`s.

### File Watching (`src/watcher.rs`)
- **`start_watching(project_root)`** -- Start a debounced file watcher (notify + crossbeam-channel). Returns `WatcherHandle` with a `Receiver<ChangeEvent>`.
- Events are filtered by language support, vendor/generated patterns.
- `ChangeEvent::Changed(path)` / `ChangeEvent::Removed(path)`

### Configuration (`src/report.rs`)
- **`IndexConfig`** -- `repo_id`, `batch_size` (default 1000), `embedding_dim` (default 384)
- **`IndexReport`** -- Statistics: files scanned/indexed/skipped/failed, symbols, relations, duration
- **`SkipReason`** -- TooLarge, Binary, UnsupportedLanguage, Ignored

## Key Dependencies

- `oc-core`, `oc-parser`, `oc-storage`
- `rayon` (parallel parsing)
- `ignore` (gitignore-aware walking)
- `notify` + `notify-debouncer-mini` (file watching)
- `crossbeam-channel` (event channel)

## Tests

- Inline unit tests in `src/scanner.rs` (empty dir, source files, vendor skip, generated skip, hidden dirs, gitignore)
- Integration tests: `tests/integration_pipeline.rs`, `tests/integration_incremental.rs`

## Related Files

- `Cargo.toml`
- `src/lib.rs`, `src/pipeline.rs`, `src/scanner.rs`, `src/incremental.rs`, `src/watcher.rs`, `src/report.rs`, `src/error.rs`

## Changelog

- 2026-02-11: Initial module CLAUDE.md created.
