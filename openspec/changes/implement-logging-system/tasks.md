## 1. Rust Tracing Infrastructure

- [x] 1.1 Add workspace tracing dependencies to root `Cargo.toml`:
  - `tracing = { version = "0.1", default-features = false, features = ["std", "attributes"] }`
  - `tracing-subscriber = { version = "0.3", default-features = false, features = ["std", "fmt", "ansi", "registry", "env-filter", "json"] }`
  - `tracing-log = { version = "0.2", default-features = false, features = ["std", "log-tracer"] }`
- [x] 1.2 Add `tracing.workspace = true` to `[dependencies]` in: `oc-core/Cargo.toml`, `oc-parser/Cargo.toml`, `oc-storage/Cargo.toml`, `oc-indexer/Cargo.toml`, `oc-retrieval/Cargo.toml`
- [x] 1.3 Add `tracing.workspace = true`, `tracing-subscriber.workspace = true`, `tracing-log.workspace = true` to `oc-python/Cargo.toml` `[dependencies]`
- [x] 1.4 Implement `init_tracing()` function in `crates/oc-python/src/lib.rs`:
  - Use `std::sync::OnceLock<()>` for idempotent initialization
  - Read `OPENACE_LOG_LEVEL` env var, default `"warn"`, parse with `EnvFilter::try_new()`, fall back to `"warn"` on parse error (print warning to stderr)
  - Read `OPENACE_LOG_FORMAT` env var: `"json"` → JSON layer, else → `fmt::Layer` with pretty + ANSI
  - Initialize `tracing_log::LogTracer::init()` for log crate compatibility
  - Build subscriber with `tracing_subscriber::registry().with(filter).with(fmt_layer)`
  - Call `try_init()` — swallow error silently
  - Force writer to `std::io::stderr`
- [x] 1.5 Call `init_tracing()` at the start of `EngineBinding::new()` in `crates/oc-python/src/engine.rs`
- [x] 1.6 Verify: `cargo build` compiles all crates; `cargo test` passes

## 2. oc-indexer Instrumentation

- [x] 2.1 Add `#[instrument(skip(config))]` to `index()` in `crates/oc-indexer/src/pipeline.rs`. Add `tracing::info!(files = scan_result.files.len(), "index started")` at start. Add `tracing::info!(files = report.files_indexed, symbols = report.total_symbols, duration_secs = %format!("{:.2}", report.duration_secs), "index completed")` at end.
- [x] 2.2 Add `#[instrument(skip_all)]` to `scan_files()` in `crates/oc-indexer/src/scanner.rs`. Add `tracing::debug!(file_count = files.len(), "scan completed")`.
- [x] 2.3 Add `#[instrument(skip(storage, chunk_config))]` to `update_file()` in `crates/oc-indexer/src/incremental.rs`. Add `tracing::debug!(path = %rel_path, "updating file")`.
- [x] 2.4 Add `#[instrument(skip(storage))]` to `delete_file()` in `crates/oc-indexer/src/incremental.rs`. Add `tracing::debug!(path = %rel_path, "deleting file")`.
- [x] 2.5 Add `#[instrument(skip(storage))]` to `process_events()` in `crates/oc-indexer/src/incremental.rs`. Add `tracing::debug!(event_count = events.len(), "processing events")`.
- [x] 2.6 Replace `eprintln!("warning: chunk fulltext index failed: {e}")` in `pipeline.rs` with `tracing::warn!(error = %e, "chunk fulltext index failed")`.
- [x] 2.7 Add rayon span propagation in `index()`: capture `let parent_span = tracing::Span::current();` before `par_iter()`. Inside each file task: `let _guard = tracing::debug_span!(parent: &parent_span, "parse_file", path = %rel_path.display()).entered();`
- [x] 2.8 Verify: `cargo test -p oc-indexer` passes

## 3. oc-parser Instrumentation

- [x] 3.1 Add `#[instrument(skip(content), fields(language, symbol_count))]` to `parse_file_with_tree()` in `crates/oc-parser/src/visitor.rs`. Record language and symbol_count via `tracing::Span::current().record()` after parsing.
- [x] 3.2 Add `#[instrument(skip(content), fields(language, symbol_count))]` to `parse_file()` in `crates/oc-parser/src/visitor.rs`.
- [x] 3.3 Add `tracing::warn!(path = %path, size = size, reason = "too_large", "file skipped")` in `check_file_size()` when file exceeds limit.
- [x] 3.4 Add `tracing::warn!(path = %file_path, reason = "binary", "file skipped")` in `parse_file_with_tree()` when binary content detected.
- [x] 3.5 Verify: `cargo test -p oc-parser` passes

## 4. oc-storage Instrumentation

- [x] 4.1 Add `#[instrument]` to `StorageManager::open_with_dimension()` in `crates/oc-storage/src/manager.rs`. Add `tracing::info!(project_root = %project_root.display(), "opening storage")`. Add `tracing::warn!(corruption_type = %e, "storage corruption detected, rebuilding")` on corruption detection. Add `tracing::info!("storage rebuilt successfully")` after recovery.
- [x] 4.2 Add `tracing::debug!(count = symbols.len(), "inserting symbols")` in `GraphStore::insert_symbols()` in `crates/oc-storage/src/graph.rs`.
- [x] 4.3 Add `#[instrument(skip(self), fields(result_count))]` to `FullTextStore::search_bm25()` in `crates/oc-storage/src/fulltext.rs`. Record result_count after search.
- [x] 4.4 Add `#[instrument(skip(self, query), fields(result_count))]` to `VectorStore::search_knn()` in `crates/oc-storage/src/vector.rs`. Record result_count after search.
- [x] 4.5 Verify: `cargo test -p oc-storage` passes

## 5. oc-retrieval Instrumentation

- [x] 5.1 Add `#[instrument(skip(self), fields(result_count))]` to `RetrievalEngine::search()` in `crates/oc-retrieval/src/engine.rs`. Record `query` (truncated to 100 chars), `limit`.
- [x] 5.2 Add `tracing::debug!(signal = "bm25", count = hits.len(), "signal collected")` after BM25 collection. Repeat for vector, exact, graph, chunk signals.
- [x] 5.3 Replace silent graceful-degradation failures with `tracing::warn!(signal = "bm25", error = %e, "signal failed, skipping")` for each signal.
- [x] 5.4 Add `tracing::info!(fused_count = results.len(), "search completed")` at end.
- [x] 5.5 Verify: `cargo test -p oc-retrieval` passes

## 6. oc-python Trace-ID Support

- [x] 6.1 Add `trace_id: Option<String>` parameter to `EngineBinding::index_full()` in `crates/oc-python/src/engine.rs`. Create span: `let _span = tracing::info_span!("engine.index_full", trace_id = %trace_id.as_deref().unwrap_or("")).entered();`
- [x] 6.2 Add `trace_id: Option<String>` parameter to `EngineBinding::search()`. Create span with trace_id field.
- [x] 6.3 Add `trace_id: Option<String>` parameter to `EngineBinding::find_symbol()`. Create span with trace_id field.
- [x] 6.4 Add `trace_id: Option<String>` parameter to `EngineBinding::get_file_outline()`. Create span with trace_id field.
- [x] 6.5 Verify: `maturin develop` succeeds

## 7. Python structlog Configuration

- [x] 7.1 Add `structlog>=24.1.0` to `pyproject.toml` `[project.dependencies]`
- [x] 7.2 Create `python/openace/logging.py` with configure_logging() and get_logger()
- [x] 7.3 Update `python/openace/__init__.py`: import and call `configure_logging()` at module load time

## 8. Python Layer Logging Migration

- [x] 8.1 Update `python/openace/engine.py`: structlog logger, trace_id via contextvars, keyword arguments
- [x] 8.2 Update `python/openace/server/app.py`: structlog logger, replace print() calls
- [x] 8.3 Update `python/openace/embedding/openai_backend.py`: N/A (no logging.getLogger found)
- [x] 8.4 Update `python/openace/query_expansion.py`: structlog migration, keyword arguments
- [x] 8.5 Update `python/openace/signal_weighting.py`: structlog migration, keyword arguments
- [x] 8.6 Update `python/openace/summary.py`: structlog migration
- [x] 8.7 Update `python/openace/reranking/api_reranker.py`: N/A (no logging.getLogger found)
- [x] 8.8 Update `python/openace/embedding/local.py`: N/A (no logging.getLogger found)

## 9. CLI Verbosity Flags

- [x] 9.1 Add `--verbose` / `-v` and `--quiet` / `-q` flags to `@click.group() main` in `python/openace/cli.py`.
- [x] 9.2 Map flags to levels: `quiet→ERROR`, `default→WARNING`, `verbose→DEBUG`
- [x] 9.3 Set `os.environ["OPENACE_LOG_LEVEL"]` based on flag mapping before Engine construction
- [x] 9.4 Call `configure_logging(level=mapped_level)` in the main group callback
- [x] 9.5 N/A (merged into 9.1-9.4)

## 10. Testing & Validation

- [x] 10.1 Run `cargo test` — all Rust tests pass (270+ tests)
- [x] 10.2 Run `maturin develop` — Python extension builds successfully
- [x] 10.3 Run `pytest tests/` — all 64 Python tests pass
- [x] 10.4 Manual verification: `OPENACE_LOG_LEVEL=info openace index .` produces structured lifecycle logs on stderr
- [x] 10.5 Manual verification: `OPENACE_LOG_FORMAT=json OPENACE_LOG_LEVEL=debug openace search "test" -p .` produces JSON logs on stderr
- [x] 10.6 Manual verification: `openace serve . --embedding none` produces zero log output on stdout
- [x] 10.7 Run `cargo bench -p oc-bench -- bench_index_full` — compare with pre-change baseline, verify <5% regression
- [x] 10.8 Verify sensitive data: run with `OPENACE_LOG_LEVEL=trace` and grep output for any API key patterns — expect zero matches
