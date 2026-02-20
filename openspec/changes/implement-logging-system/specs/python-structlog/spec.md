## ADDED Requirements

### Requirement: structlog as core Python dependency
The system SHALL add `structlog>=24.1.0` to `pyproject.toml` `[project.dependencies]`. This is a core (non-optional) dependency.

#### Scenario: Installation includes structlog
- **WHEN** `pip install openace` is run
- **THEN** structlog is installed as a dependency

### Requirement: Dedicated logging configuration module
The system SHALL create `python/openace/logging.py` containing:
- `configure_logging(level: str = "WARNING", fmt: str = "pretty")` function
- structlog processor pipeline: contextvars merge, log level, logger name, timestamper (ISO), sensitive field redaction, exception formatting, renderer
- Renderer selection: `ConsoleRenderer` for pretty (with color auto-detection), `JSONRenderer` for json
- Output to stderr only (via `structlog.PrintLoggerFactory(file=sys.stderr)`)

The `configure_logging()` function SHALL be called from `openace/__init__.py` at import time with defaults from `OPENACE_LOG_LEVEL` and `OPENACE_LOG_FORMAT` environment variables.

#### Scenario: Default configuration
- **WHEN** `import openace` is executed without env vars set
- **THEN** structlog is configured with WARNING level, pretty format, stderr output

#### Scenario: JSON format via env var
- **WHEN** `OPENACE_LOG_FORMAT=json` is set and `import openace` is executed
- **THEN** all subsequent structlog output is newline-delimited JSON to stderr

#### Scenario: Reconfiguration from CLI
- **WHEN** `configure_logging(level="DEBUG", fmt="pretty")` is called after import
- **THEN** the log level and format are updated for all subsequent events

### Requirement: Replace all print() and logging.getLogger() with structlog
The system SHALL replace the following patterns across all Python modules:
1. `print(..., file=sys.stderr)` in `server/app.py` → `logger.info()` / `logger.warning()`
2. `print()` in `embedding/openai_backend.py` → `logger.warning()` for retries
3. `logging.getLogger(__name__)` in `engine.py`, `query_expansion.py`, `signal_weighting.py`, `summary.py` → `structlog.get_logger()`
4. `logger.info()` / `logger.warning()` calls updated to use keyword arguments for structured fields

Each module SHALL define `logger = structlog.get_logger()` at module level.

#### Scenario: MCP server uses structlog
- **WHEN** MCP server starts background indexing
- **THEN** status messages appear as structured log events on stderr (not raw print)

#### Scenario: Embedding retry logging
- **WHEN** OpenAI embedding encounters a rate limit
- **THEN** a WARN event is emitted via structlog with `retry_attempt` and `wait_seconds` fields

#### Scenario: No print() calls remain
- **WHEN** `grep -rn "print(" python/openace/` is run (excluding test files and __pycache__)
- **THEN** zero matches are found

### Requirement: CLI verbosity flags
The system SHALL add `--verbose` / `-v` (repeatable up to 2) and `--quiet` / `-q` flags to the Click `main` group.

Level mapping:
- `--quiet`: Python=ERROR, Rust=error
- (default): Python=WARNING, Rust=warn
- `-v`: Python=INFO, Rust=info
- `-vv`: Python=DEBUG, Rust=debug

The CLI SHALL set `OPENACE_LOG_LEVEL` environment variable before Engine construction and call `configure_logging()` with the appropriate level.

#### Scenario: Verbose indexing
- **WHEN** `openace index /path -v` is run
- **THEN** INFO-level structured logs appear showing index lifecycle events

#### Scenario: Quiet mode
- **WHEN** `openace search "query" -p /path -q` is run
- **THEN** only ERROR-level logs appear (if any)

#### Scenario: Double verbose
- **WHEN** `openace index /path -vv` is run
- **THEN** DEBUG-level logs appear including per-file parse events and per-signal search details

### Requirement: MCP server stderr-only logging
The system SHALL ensure MCP server logs exclusively to stderr. No log output SHALL appear on stdout. All `click.echo(..., err=True)` calls in the serve command SHALL use structlog instead.

#### Scenario: MCP protocol integrity
- **WHEN** MCP server is running and handling tool calls
- **THEN** stdout contains only valid MCP JSON-RPC messages
- **AND** stderr contains all log output

### Requirement: structlog thread-safety for concurrent embedding
The system SHALL use `structlog.contextvars` for context propagation in async contexts (MCP server's `asyncio.to_thread()` calls). For ThreadPoolExecutor-based embedding batches, each thread SHALL bind its own logger context (batch_offset, batch_size).

#### Scenario: Concurrent embedding logging
- **WHEN** 4 embedding batches run concurrently via ThreadPoolExecutor
- **THEN** each batch's log events include correct `batch_offset` and `batch_size` fields
- **AND** no context leakage between threads

## PBT Properties

### P7: All print() calls eliminated
**Invariant**: After migration, `grep -rn "print(" python/openace/` returns zero matches (excluding comments and string literals).
**Falsification**: Run grep on the openace package; assert empty result.

### P8: structlog output format consistency
**Invariant**: With `OPENACE_LOG_FORMAT=json`, every log line is valid JSON.
**Falsification**: Capture 100 log events; parse each as JSON; assert all succeed.

### P9: CLI level mapping correctness
**Invariant**: For each verbosity flag combination, the effective Rust and Python log levels match the specification table.
**Falsification**: For each flag combo (-q, default, -v, -vv), check `OPENACE_LOG_LEVEL` env var and structlog effective level.
