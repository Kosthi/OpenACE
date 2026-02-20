## ADDED Requirements

### Requirement: oc-indexer pipeline instrumentation
The system SHALL add `#[instrument]` to the following functions in oc-indexer:
- `index()` with `skip(config)`: log file_count, symbol_count, duration at INFO on completion
- `scan_files()` with `skip_all`: log scanned file count at DEBUG
- `update_file()` with `skip(storage, chunk_config)`: log file path at DEBUG
- `delete_file()` with `skip(storage)`: log file path at DEBUG
- `process_events()` with `skip(storage)`: log event count at DEBUG

The existing `eprintln!("warning: chunk fulltext index failed: {e}")` in pipeline.rs SHALL be replaced with `tracing::warn!(error = %e, "chunk fulltext index failed")`.

#### Scenario: Index lifecycle logging
- **WHEN** `index()` is called with `OPENACE_LOG_LEVEL=info`
- **THEN** output includes: "index started" event, "index completed" event with fields `files`, `symbols`, `duration_secs`

#### Scenario: File-level debug logging
- **WHEN** `index()` is called with `OPENACE_LOG_LEVEL=debug`
- **THEN** individual file scan and parse events are visible with file paths

#### Scenario: Chunk failure warning
- **WHEN** chunk fulltext indexing fails for a file
- **THEN** a WARN event is emitted with `error` field containing the error message (not eprintln)

### Requirement: oc-parser entry point instrumentation
The system SHALL add `#[instrument(skip(content))]` to `parse_file()` and `parse_file_with_tree()`. Log file path and detected language at TRACE level. Log warning for skipped files (too large, binary, unsupported language). The `content` parameter SHALL never be captured in spans.

#### Scenario: Parse tracing at TRACE level
- **WHEN** a Python file is parsed with `OPENACE_LOG_LEVEL=trace`
- **THEN** a span event shows `file_path`, `language="python"`, `symbol_count` on completion

#### Scenario: Skipped file warning
- **WHEN** a file exceeding 1MB is encountered
- **THEN** a WARN event is emitted: "file skipped" with `path` and `reason="too_large"` fields

#### Scenario: Content never in logs
- **WHEN** any file is parsed at any log level
- **THEN** the file content does not appear in any log line or span field

### Requirement: oc-storage operation instrumentation
The system SHALL add tracing to:
- `StorageManager::open_with_dimension()`: INFO span with `project_root`, WARN on corruption detection + auto-recovery
- `GraphStore::insert_symbols()` with `skip(symbols)`: DEBUG event with `count` field
- `FullTextStore::search_bm25()`: DEBUG span with `query`, `limit`, result `count`
- `VectorStore::search_knn()` with `skip(query_vector)`: DEBUG span with `k`, result `count`

#### Scenario: Corruption recovery visibility
- **WHEN** `StorageManager::open_with_dimension()` detects SQLite corruption
- **THEN** a WARN event is emitted: "storage corruption detected, rebuilding" with `corruption_type` field
- **AND** a subsequent INFO event confirms "storage rebuilt successfully"

#### Scenario: Storage operation timing
- **WHEN** `FullTextStore::search_bm25()` completes at DEBUG level
- **THEN** the span duration is visible in the output

### Requirement: oc-retrieval search instrumentation
The system SHALL add `#[instrument(skip(self))]` to `RetrievalEngine::search()`. Create a top-level INFO span with `query` (truncated to 100 chars), `limit`. Log each signal's result count as DEBUG events: `bm25_count`, `vector_count`, `exact_count`, `graph_count`, `chunk_count`. Log final fused result count at INFO. Replace silent graceful-degradation failures with `tracing::warn!`.

#### Scenario: Search signal visibility
- **WHEN** a search is executed with `OPENACE_LOG_LEVEL=debug`
- **THEN** output shows per-signal result counts: BM25, vector, exact, graph, chunk
- **AND** a final "search completed" event with `total_results` and `fused_count`

#### Scenario: Signal failure warning
- **WHEN** BM25 search fails but vector search succeeds
- **THEN** a WARN event is emitted for the BM25 failure with `signal="bm25"` and `error` fields
- **AND** search still returns results from other signals

### Requirement: Rayon parallel parsing span propagation
The system SHALL propagate the current tracing span into rayon parallel tasks during indexing. The parent span SHALL be captured before `par_iter()` and each file task SHALL create a child span with `parent: &parent_span`.

#### Scenario: Parallel parse log correlation
- **WHEN** files are parsed in parallel with `OPENACE_LOG_LEVEL=debug`
- **THEN** each file's parse events appear within the parent `index` span
- **AND** events from different files are distinguishable by their `path` field

### Requirement: oc-python EngineBinding instrumentation
The system SHALL add `#[instrument(skip(self))]` to `EngineBinding::index_full()` and `EngineBinding::search()`. The `trace_id` parameter (when provided) SHALL be included as a span field.

#### Scenario: Cross-boundary span
- **WHEN** Python calls `EngineBinding::search()` with `trace_id="abc123"`
- **THEN** the Rust span includes `trace_id="abc123"` as a field

## PBT Properties

### P4: Instrumentation does not affect determinism
**Invariant**: For any input project, `index()` produces identical `IndexReport` (same symbol counts, same files indexed) regardless of log level.
**Falsification**: Run `index()` at levels TRACE and ERROR; compare IndexReport fields.

### P5: Content never in log output
**Invariant**: For any file content and any log level, the content string does not appear in tracing output.
**Falsification**: Parse a file with unique sentinel content; capture all tracing output; assert sentinel not found.

### P6: Performance regression bounded
**Invariant**: Full indexing of 10K files with log level `warn` takes no more than 105% of the pre-instrumentation baseline.
**Falsification**: Run `cargo bench -p oc-bench -- bench_index_full` before and after; compare.
