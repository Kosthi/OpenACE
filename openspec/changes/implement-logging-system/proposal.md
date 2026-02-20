# Proposal: Implement Best-Practice Logging System

## Context

### User Need

OpenACE currently has almost no observability: the entire Rust core (7 crates, ~15K lines) contains only 2 `eprintln!` calls, and the Python layer has inconsistent logging — 4 modules use `logging.getLogger()`, while MCP server and embedding providers use raw `print(file=sys.stderr)`. Diagnosing indexing slowness, search quality issues, storage corruption recovery, or API provider failures requires manual debugging with no structured trail.

The goal is to implement a unified, best-practice logging system across both Rust and Python layers with:
- Structured logging (key-value fields, not just messages)
- Dual output format (human-readable dev / JSON production)
- Cross-language trace-id propagation (Python → Rust via PyO3)
- Sensitive data protection (API keys, user code never logged)
- Zero performance regression on hot paths

### Current State

**Rust layer (7 crates):**
- Zero logging framework dependencies (no `tracing`, `log`, or `env_logger` in Cargo.toml)
- 2 `eprintln!` calls total:
  - `crates/oc-indexer/src/pipeline.rs:297` — chunk fulltext index failure warning
  - `crates/oc-bench/benches/index_full.rs:40` — .openace cleanup retry
- Error handling is thorough (`Result<T, E>` everywhere) but errors are silent unless they propagate to Python
- Storage corruption auto-recovery happens silently (no notification)
- Graceful degradation in retrieval engine silences individual signal failures

**Python layer:**
- 4 modules use `logging.getLogger(__name__)`: engine.py, query_expansion.py, signal_weighting.py, summary.py
- MCP server uses `print(file=sys.stderr)` for status messages
- Embedding providers use `print()` for retry/rate-limit notifications
- CLI uses `click.echo()` only — no log level control
- No structured logging library; all messages are human-readable strings
- No request/response tracing for API calls

**Sensitive data exposure risks:**
- API keys stored in instance variables (OPENAI_API_KEY, COHERE_API_KEY, VOYAGE_API_KEY)
- Project file paths included in exception messages
- Code snippets in SearchResult passed through MCP without sanitization

### Discovered Constraints

#### Hard Constraints (Technical Limitations)

- **HC-1: SymbolId hot path** — `SymbolId::generate()` is called ~1M times per full indexing run (XXH3-128 hashing). Any per-call logging overhead is unacceptable. Use `tracing` span-level instrumentation on the indexing pipeline, not individual hash calls.
- **HC-2: Rayon parallel parsing** — `oc-indexer` uses `rayon::par_iter()` for multi-threaded file parsing. `tracing` spans must be propagated to rayon worker threads via `tracing::Span::current()` + `span.enter()` pattern (not automatic in rayon).
- **HC-3: GIL-released Rust operations** — All heavy Rust operations in `oc-python` use `py.allow_threads()`. Logging inside GIL-released blocks works fine with `tracing` (thread-safe), but Python logging context (trace-id) must be passed as a parameter, not read from thread-local.
- **HC-4: MCP server uses stdio** — MCP server communicates via stdin/stdout. All logging MUST go to stderr only. `print()` to stdout would corrupt the MCP protocol stream.
- **HC-5: Arc<Mutex<Option<StorageManager>>>** — StorageManager is wrapped in mutex and temporarily dropped during re-indexing. Logging of storage operations must handle the "storage unavailable" window gracefully.
- **HC-6: Tantivy background commit thread** — FullTextStore has a background thread for auto-committing (500 docs / 500ms threshold). Logging from this thread must be thread-safe and not block the commit.

#### Soft Constraints (Conventions & Preferences)

- **SC-1: tracing ecosystem** — User selected `tracing` + `tracing-subscriber` for Rust (not `log` crate). Supports structured spans, `#[instrument]` macro, and layered subscribers.
- **SC-2: Dual output format** — Development: `tracing_subscriber::fmt` with pretty colors. Production: JSON format via `tracing-subscriber` JSON layer. Controlled by `OPENACE_LOG_FORMAT=json|pretty` env var.
- **SC-3: structlog for Python** — User selected `structlog` (not stdlib logging alone). Provides context binding, processor pipeline, and JSON rendering.
- **SC-4: Basic trace-id propagation** — Python Engine generates a UUID trace-id per operation. Passed to Rust via function parameters. Recorded in tracing spans as a field.
- **SC-5: Log level convention** — ERROR: unrecoverable failures. WARN: degraded behavior (fallbacks, retries). INFO: operation lifecycle (index start/end, search request/response). DEBUG: detailed internal state. TRACE: per-item processing (individual files, symbols).
- **SC-6: Existing error handling preserved** — `thiserror` derive macros and per-crate error enums remain unchanged. Logging augments but does not replace Result-based error propagation.
- **SC-7: Sensitive data never logged** — API keys, authorization headers, user source code content, and file contents must never appear in log output. Only metadata (file paths, symbol counts, durations, error types) are logged.

#### Dependencies (Cross-Module)

- **D-1**: `tracing` and `tracing-subscriber` added as workspace-level dependencies — all 7 Rust crates can use them
- **D-2**: `oc-python` initializes the tracing subscriber (single global initialization point) when `EngineBinding::new()` is called
- **D-3**: Python `structlog` configured in `openace/__init__.py` or a dedicated `openace/logging.py` module
- **D-4**: Trace-id generated in Python, passed to Rust `EngineBinding` methods, recorded in tracing spans
- **D-5**: CLI `--verbose` / `--quiet` flags control both Python structlog level and Rust tracing filter level

#### Risks

- **R-1: tracing subscriber initialization** — `tracing_subscriber::init()` can only be called once globally. If a user embeds OpenACE in a larger Rust application that already has tracing, initialization will conflict. Mitigation: use `try_init()` which returns `Err` instead of panicking; document this in SDK docs.
- **R-2: Performance regression on indexing** — Adding `#[instrument]` to frequently-called functions could measurably slow indexing. Mitigation: only instrument pipeline-level functions (index, parse_file, search), not leaf functions (hash, tree-sitter node traversal). Use `skip_all` to avoid cloning large arguments.
- **R-3: Rayon span propagation overhead** — Entering/exiting spans on each rayon task has non-zero cost. Mitigation: use a single `index_file` span per file, not per-symbol. Benchmark before/after with `cargo bench -p oc-bench`.
- **R-4: structlog dependency size** — structlog pulls in additional dependencies. Mitigation: add as optional dependency in `[project.optional-dependencies]` if size is a concern, or accept as core dependency given its small footprint.
- **R-5: MCP stdout contamination** — Any accidental log output to stdout breaks MCP protocol. Mitigation: configure tracing subscriber with explicit stderr writer; add integration test that verifies no stdout output during MCP operations.

## Requirements

### R1: Rust Workspace Logging Dependencies

**Description**: Add `tracing`, `tracing-subscriber` (with `fmt`, `json`, `env-filter` features) as workspace dependencies. Each crate that needs logging adds `tracing.workspace = true` to its `[dependencies]`.

**Scenario**: Given the workspace Cargo.toml, when `cargo build` is run, then all crates compile with tracing available. The dependency addition does not change any existing behavior — no log output appears until a subscriber is initialized.

### R2: Tracing Subscriber Initialization in oc-python

**Description**: Initialize the global tracing subscriber when `EngineBinding::new()` is called. Support dual-mode formatting: pretty (default) or JSON (when `OPENACE_LOG_FORMAT=json`). Log level controlled by `OPENACE_LOG_LEVEL` env var (default: `warn`). Output always goes to stderr.

**Scenario**: Given `OPENACE_LOG_LEVEL=info` and `OPENACE_LOG_FORMAT=pretty`, when `EngineBinding::new()` is called, then the tracing subscriber is initialized with info-level filter and human-readable colored output to stderr. If `OPENACE_LOG_FORMAT=json`, output is newline-delimited JSON.

### R3: Instrument oc-indexer Pipeline

**Description**: Add `#[instrument]` to pipeline-level functions in oc-indexer: `index()`, `scan_files()`, `process_events()`, `update_file()`, `delete_file()`. Record file count, symbol count, duration, and errors as span fields. Replace the existing `eprintln!` in pipeline.rs with `tracing::warn!`.

**Scenario**: Given a project with 100 files, when `index()` is called with `OPENACE_LOG_LEVEL=info`, then log output shows: index started, files scanned (count), parsing progress, storage writes, index completed (total symbols, duration). At debug level, individual file parse events are visible.

### R4: Instrument oc-parser Entry Points

**Description**: Add `#[instrument(skip(content))]` to `parse_file()` and `parse_file_with_tree()`. Log file path, language, symbol count on completion. Log warnings for skipped files (too large, binary, unsupported language). Never log file content (`skip(content)` ensures content is not captured).

**Scenario**: Given a 2MB file, when parsed, then a WARN event is emitted: "file skipped: too large" with path and size fields. Given a valid Python file, when parsed at DEBUG level, then an event shows file path, detected language, and extracted symbol count.

### R5: Instrument oc-storage Operations

**Description**: Add tracing to `StorageManager::open_with_dimension()` (corruption detection/recovery), `GraphStore::insert_symbols()` (batch size), `FullTextStore::add_document()` / `commit()` (batch count), `VectorStore::add_vector()` / `search_knn()` (result count). Warn on corruption detection and auto-recovery.

**Scenario**: Given a corrupted SQLite database, when `StorageManager::open_with_dimension()` detects corruption, then a WARN event is emitted: "storage corruption detected, purging .openace/ for rebuild" with the corruption type. Previously this was completely silent.

### R6: Instrument oc-retrieval Search

**Description**: Add tracing to `RetrievalEngine::search()`. Create a top-level span with query text (truncated to 100 chars), limit, and filters. Log each signal's result count and timing as DEBUG events. Log the final RRF-fused result count at INFO. Replace graceful-degradation silent failures with `tracing::warn!`.

**Scenario**: Given a search query, when executed with `OPENACE_LOG_LEVEL=debug`, then output shows: BM25 results (count, ms), vector results (count, ms), exact results (count, ms), graph expansion (count, ms), RRF fusion (final count), and any signal failures as warnings.

### R7: Trace-ID Propagation (Python → Rust)

**Description**: Add an optional `trace_id: Option<String>` parameter to key `EngineBinding` methods (`index_full`, `search`, `find_symbol`, `get_file_outline`). When provided, record it as a field in the top-level tracing span. Python `Engine` generates a UUID4 trace-id per public method call and passes it through.

**Scenario**: Given a search call from Python with trace-id "abc-123", when the Rust search executes, then all tracing spans/events for that operation include `trace_id="abc-123"` as a field. This allows correlating Python-side and Rust-side logs.

### R8: Python structlog Configuration

**Description**: Create `python/openace/logging.py` with structlog configuration. Development: colored key-value output to stderr. Production: JSON output when `OPENACE_LOG_FORMAT=json`. Configure at import time via `openace/__init__.py`. Add `structlog` as a core dependency in pyproject.toml.

**Scenario**: Given `OPENACE_LOG_FORMAT=json`, when the Python SDK logs a warning, then output is a single JSON line to stderr with `timestamp`, `level`, `module`, `event`, and any bound context (trace_id, operation).

### R9: Unify Python Layer Logging

**Description**: Replace all `print(file=sys.stderr)` and `print()` calls with `structlog.get_logger()` calls across: `server/app.py`, `embedding/openai_backend.py`, `embedding/local.py`, `reranking/` modules, `cli.py`. Unify the 4 existing `logging.getLogger()` usages to structlog. Add `--verbose` / `--quiet` CLI flags.

**Scenario**: Given `--verbose` on the CLI, when `openace index` runs, then DEBUG-level structured logs appear on stderr showing: each file indexed, embedding batch progress, and timing. Given `--quiet`, only ERROR-level output appears. Default (no flag) shows INFO level.

### R10: Sensitive Data Protection

**Description**: Ensure API keys, authorization headers, and file content are never logged. In Rust: use `skip(content)` on `#[instrument]`, never log `api_key` fields. In Python: configure structlog processor to redact any field matching `*key*`, `*secret*`, `*token*`, `*authorization*` patterns. Document the redaction policy.

**Scenario**: Given a search with an embedding API key configured, when logs are examined at TRACE level, then no API key value appears in any log line. File paths appear but file contents do not.

### R11: Performance Validation

**Description**: Run `cargo bench -p oc-bench` before and after logging changes. Full indexing benchmark (10K files) must not regress more than 5%. Search latency benchmark must not regress more than 2%. If regression exceeds thresholds, remove instrumentation from hot paths.

**Scenario**: Given the oc-bench benchmark suite, when benchmarks are run at the default log level (warn), then indexing throughput is within 5% of the pre-logging baseline, and search latency is within 2%.

## Success Criteria

1. `OPENACE_LOG_LEVEL=info openace index /path/to/project` produces structured lifecycle logs (start, progress, completion) on stderr
2. `OPENACE_LOG_LEVEL=debug openace search "query" -p /path/to/project` produces per-signal timing and result count logs
3. `OPENACE_LOG_FORMAT=json` switches all output (Rust + Python) to newline-delimited JSON
4. Storage corruption auto-recovery emits a visible WARN log (no longer silent)
5. Trace-id appears in both Python and Rust log output for the same operation
6. `grep -ri "api_key\|secret\|token" <log_output>` finds zero actual credential values
7. `cargo bench -p oc-bench` shows <5% indexing regression and <2% search regression
8. MCP server (`openace serve`) produces zero stdout log contamination
