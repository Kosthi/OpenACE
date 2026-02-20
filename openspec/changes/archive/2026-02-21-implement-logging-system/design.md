## Context

OpenACE is a Rust+Python hybrid monorepo with virtually no observability infrastructure. The Rust core (7 crates, ~15K LoC) has 2 `eprintln!` calls total; the Python layer has inconsistent logging across 4 of ~15 modules with raw `print()` elsewhere. This design covers adding a unified structured logging system across both layers, with cross-language trace-id correlation, dual output format (human/JSON), and sensitive data protection.

The design was validated through multi-model analysis (Codex for Rust/backend, Gemini for Python/integration), with all ambiguities resolved through user decisions.

## Goals / Non-Goals

**Goals:**
- Add `tracing` ecosystem to all 7 Rust crates with structured spans and events
- Add `structlog` to the Python layer, replacing all `print()` and stdlib `logging` calls
- Support dual output format (pretty dev / JSON prod) via `OPENACE_LOG_FORMAT` env var
- Propagate trace-id from Python to Rust for cross-language log correlation
- Protect sensitive data (API keys, source code content) from appearing in logs
- Maintain performance: <5% indexing regression, <2% search latency regression

**Non-Goals:**
- Distributed tracing (OpenTelemetry, Jaeger, Zipkin) — deferred to future change
- Log aggregation infrastructure (ELK, Datadog setup) — user responsibility
- Metrics/counters (Prometheus) — separate concern
- Async logging (tracing-appender non-blocking writer) — premature; default stderr is sufficient
- Compile-time level filtering (release_max_level_*) — user chose runtime flexibility

## Decisions

### D1: tracing Ecosystem for Rust (Not log Crate)

**Decision**: Use `tracing = "0.1"` with `tracing-subscriber = "0.3"` (features: `fmt`, `json`, `ansi`, `env-filter`, `registry`). Add `tracing-log = "0.2"` for `log` crate compatibility bridge.

**Rationale**: `tracing` provides structured spans with `#[instrument]`, zero-cost disabled events, and layered subscriber architecture. The `log` crate lacks span concept and structured fields. Codex confirmed version compatibility and feature set.

**Workspace dependencies** (added to root `Cargo.toml`):
```toml
tracing = { version = "0.1", default-features = false, features = ["std", "attributes"] }
tracing-subscriber = { version = "0.3", default-features = false, features = ["std", "fmt", "ansi", "registry", "env-filter", "json"] }
tracing-log = { version = "0.2", default-features = false, features = ["std", "log-tracer"] }
```

**Per-crate usage**: Each crate that needs logging adds `tracing.workspace = true` to `[dependencies]`. Only `oc-python` depends on `tracing-subscriber` and `tracing-log` (it owns initialization).

**Alternatives considered**:
- `log` + `env_logger`: Rejected — no span/structured-field support
- `slog`: Rejected — less ecosystem momentum, no `#[instrument]` equivalent

### D2: Subscriber Initialization in oc-python via OnceLock

**Decision**: `oc-python` owns global tracing subscriber initialization. Use `std::sync::OnceLock` for idempotent one-time init. Call `init_tracing()` at the start of `EngineBinding::new()`. Use `tracing_subscriber::registry().with(fmt_layer).with(filter).try_init()` — swallow "already set" errors silently.

**Rationale**: Codex identified that `set_global_default` panics on second call. `try_init()` returns `Err` if already initialized, which is the correct behavior when OpenACE is embedded in a host application that already has tracing. `OnceLock` ensures the init function body runs at most once even under concurrent calls.

**Configuration via environment variables**:
- `OPENACE_LOG_LEVEL`: Filter string, default `"warn"`. Supports per-crate overrides like `"warn,oc_indexer=info,oc_retrieval=debug"`.
- `OPENACE_LOG_FORMAT`: `"pretty"` (default) or `"json"`.

**Alternatives considered**:
- Init in Python layer via FFI call: Rejected — tracing subscriber must be set before any Rust tracing calls
- Per-crate subscriber: Rejected — tracing uses a single global subscriber

### D3: Selective #[instrument] Placement Strategy

**Decision**: Instrument only pipeline-level and public API functions. Never instrument leaf functions or hot inner loops.

**Instrumented functions** (with `#[instrument]`):
| Crate | Function | Skip Fields |
|-------|----------|-------------|
| oc-indexer | `index()` | `skip(config)` |
| oc-indexer | `scan_files()` | `skip_all` |
| oc-indexer | `update_file()` | `skip(storage, chunk_config)` |
| oc-indexer | `delete_file()` | `skip(storage)` |
| oc-indexer | `process_events()` | `skip(storage)` |
| oc-parser | `parse_file()` | `skip(content)` |
| oc-parser | `parse_file_with_tree()` | `skip(content)` |
| oc-storage | `StorageManager::open_with_dimension()` | none |
| oc-storage | `GraphStore::insert_symbols()` | `skip(symbols)` |
| oc-storage | `FullTextStore::search_bm25()` | none |
| oc-storage | `VectorStore::search_knn()` | `skip(query_vector)` |
| oc-retrieval | `RetrievalEngine::search()` | `skip(self)` |
| oc-python | `EngineBinding::index_full()` | `skip(self)` |
| oc-python | `EngineBinding::search()` | `skip(self, query_vector)` |

**NOT instrumented** (hot paths):
- `SymbolId::generate()` (~1M calls/index)
- Tree-sitter visitor inner traversals (per-node walks)
- `FullTextStore::add_document()` hot loop (per-document)
- `VectorStore::add_vector()` (per-symbol)
- All `oc-core` functions (leaf-level, high frequency)

**Rationale**: Codex measured that `#[instrument]` has ~50ns overhead per call due to span creation. At 1M calls, that's 50ms — acceptable for pipeline functions but not for per-symbol/per-node operations.

### D4: Rayon Span Propagation via Manual Parent Pattern

**Decision**: Capture `tracing::Span::current()` before the rayon `par_iter()` call. Inside each rayon task, create a child span with explicit `parent: &parent_span`.

**Pattern**:
```rust
let parent_span = tracing::Span::current();
files.par_iter().for_each(|file| {
    let file_span = tracing::debug_span!(parent: &parent_span, "parse_file", path = %file.display());
    let _guard = file_span.enter();
    // ... parse file ...
});
```

**Rationale**: `tracing-subscriber` does not automatically propagate spans into rayon worker threads (they use a work-stealing thread pool with no async context). Codex confirmed this requires manual propagation. The overhead is one span creation per file (thousands, not millions) — acceptable.

**Alternatives considered**:
- `tracing::Instrument` trait: Rejected — designed for async futures, not rayon sync tasks
- Thread-local span storage: Rejected — rayon work-stealing invalidates thread-local assumptions

### D5: structlog as Core Python Dependency

**Decision**: Add `structlog >= 24.1.0` as a core dependency in `pyproject.toml` `[project.dependencies]`. Create `python/openace/logging.py` for configuration. Initialize in `openace/__init__.py`.

**structlog configuration**:
```python
# Processor pipeline
processors = [
    structlog.contextvars.merge_contextvars,   # async-safe context
    structlog.stdlib.add_log_level,
    structlog.stdlib.add_logger_name,
    structlog.processors.TimeStamper(fmt="iso"),
    _redact_sensitive_fields,                  # custom: redact API keys
    structlog.processors.StackInfoRenderer(),
    structlog.processors.format_exc_info,
]

# Renderer: switch on OPENACE_LOG_FORMAT
if os.environ.get("OPENACE_LOG_FORMAT") == "json":
    renderer = structlog.processors.JSONRenderer()
else:
    renderer = structlog.dev.ConsoleRenderer(colors=sys.stderr.isatty())
```

**Module-level usage**: `logger = structlog.get_logger()` in every module. Bind context per operation: `logger.bind(trace_id=tid, operation="search")`.

**Alternatives considered**:
- stdlib logging alone: Rejected — user chose structlog for better structured output
- structlog as optional: Rejected — user chose core dependency for simplicity

### D6: Trace-ID Generation and Propagation

**Decision**: Python `Engine` generates a UUID4 string per public method call. Passed to Rust `EngineBinding` methods as `trace_id: Option<String>` parameter. Recorded in tracing spans as a string field.

**Flow**:
```
Python Engine.search(query)
  → trace_id = uuid.uuid4().hex[:16]  # 16-char hex
  → structlog.bind(trace_id=trace_id)
  → self._core.search(..., trace_id=trace_id)
    → Rust: info_span!("engine.search", trace_id = %trace_id)
```

**Design decisions**:
- 16-char hex (64-bit) — sufficient uniqueness for single-process correlation, compact in logs
- Not a full UUID4 string — shorter, cheaper to pass and format
- `Option<String>` on Rust side — backward compatible, no trace-id if Python doesn't provide one
- `structlog.contextvars` for async propagation in MCP server's `asyncio.to_thread()` calls

**Alternatives considered**:
- Full UUID4 (32 hex chars): Rejected — unnecessarily long for single-process use
- OpenTelemetry trace context: Rejected — premature; deferred to future distributed tracing change
- Thread-local storage: Rejected — doesn't work across `py.allow_threads()` boundary

### D7: Sensitive Data Redaction Strategy

**Decision**: Two-layer protection.

**Rust layer**: Use `skip(content)` on all `#[instrument]` macros that handle file content. Never add `api_key`, `token`, `secret`, or `authorization` as span fields. This is enforced by code review — no runtime overhead.

**Python layer**: Custom structlog processor that scans event dict keys for patterns matching `*key*`, `*secret*`, `*token*`, `*password*`, `*authorization*` (case-insensitive) and replaces values with `"[REDACTED]"`. Applied early in the processor pipeline (before rendering).

```python
def _redact_sensitive_fields(logger, method_name, event_dict):
    SENSITIVE_PATTERNS = {"key", "secret", "token", "password", "authorization"}
    for k in list(event_dict.keys()):
        if any(p in k.lower() for p in SENSITIVE_PATTERNS):
            event_dict[k] = "[REDACTED]"
    return event_dict
```

**What is NOT logged** (enforced by instrumentation placement):
- File content (Rust: `skip(content)`)
- Embedding vectors (Rust: `skip(query_vector)`, `skip(vectors)`)
- API response bodies
- Source code snippets from search results

**What IS logged** (safe metadata):
- File paths, symbol names, qualified names
- Counts (files, symbols, results)
- Durations (parse time, search time, embedding time)
- Error types and messages (but not stack traces at WARN level)
- Log levels, module names, timestamps

### D8: CLI Verbosity Control

**Decision**: Add `--verbose` / `-v` and `--quiet` / `-q` flags to the Click `main` group. These control both Python structlog level and Rust `OPENACE_LOG_LEVEL`.

| Flag | Python Level | Rust Level |
|------|-------------|------------|
| `--quiet` | ERROR | error |
| (default) | WARNING | warn |
| `-v` | INFO | info |
| `-vv` | DEBUG | debug |

**Implementation**: Set `OPENACE_LOG_LEVEL` environment variable before `Engine` construction so that Rust subscriber picks it up during `EngineBinding::new()`.

### D9: MCP Server Logging Safety

**Decision**: MCP server communicates via stdin/stdout. ALL log output MUST go to stderr. This is already the default for both `tracing-subscriber` (writes to stderr) and structlog (configured with `sys.stderr`). Replace all existing `print(file=sys.stderr)` with structlog calls; remove any `print()` calls that go to stdout.

**Verification**: Add an integration test that captures stdout during MCP tool calls and asserts it contains only valid MCP JSON-RPC messages, zero log lines.

## Dependency Summary

### Rust (Cargo.toml workspace)
```toml
tracing = { version = "0.1", default-features = false, features = ["std", "attributes"] }
tracing-subscriber = { version = "0.3", default-features = false, features = ["std", "fmt", "ansi", "registry", "env-filter", "json"] }
tracing-log = { version = "0.2", default-features = false, features = ["std", "log-tracer"] }
```

### Python (pyproject.toml)
```toml
dependencies = [
    "click",
    "structlog>=24.1.0",
    # ... existing deps ...
]
```
