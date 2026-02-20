## ADDED Requirements

### Requirement: Workspace tracing dependencies
The system SHALL add `tracing`, `tracing-subscriber`, and `tracing-log` as workspace-level dependencies in root `Cargo.toml`. Only `oc-python` SHALL depend on `tracing-subscriber` and `tracing-log` (subscriber initialization). All other crates SHALL depend on `tracing` only (for macros).

#### Scenario: Workspace compiles with tracing
- **WHEN** `cargo build` is run after adding tracing dependencies
- **THEN** all 7 crates compile without errors
- **AND** no log output appears (no subscriber initialized at build time)

#### Scenario: Per-crate dependency minimality
- **WHEN** `oc-core`, `oc-parser`, `oc-storage`, `oc-indexer`, `oc-retrieval` Cargo.toml are inspected
- **THEN** they list only `tracing.workspace = true` (not `tracing-subscriber`)
- **AND** `oc-python` Cargo.toml lists `tracing.workspace = true`, `tracing-subscriber.workspace = true`, `tracing-log.workspace = true`

### Requirement: Idempotent tracing subscriber initialization
The system SHALL initialize the global tracing subscriber exactly once via `OnceLock` in `oc-python`. Initialization SHALL be called at the start of `EngineBinding::new()`. If a subscriber is already set (by a host application), initialization SHALL silently succeed without error.

#### Scenario: First initialization
- **WHEN** `EngineBinding::new()` is called for the first time
- **THEN** a tracing subscriber is registered with the configured format and level filter
- **AND** subsequent tracing events are captured

#### Scenario: Repeated initialization (embedded SDK)
- **WHEN** `EngineBinding::new()` is called after a host application has already set a global subscriber
- **THEN** no panic occurs
- **AND** the host application's subscriber remains active

#### Scenario: Concurrent initialization
- **WHEN** two threads call `EngineBinding::new()` simultaneously
- **THEN** exactly one subscriber initialization occurs
- **AND** no race condition or panic

### Requirement: Dual output format (pretty / JSON)
The system SHALL support two output formats controlled by `OPENACE_LOG_FORMAT` environment variable:
- `"pretty"` (default): Human-readable colored output with timestamps, level, target, and message
- `"json"`: Newline-delimited JSON with fields: `timestamp`, `level`, `target`, `message`, `span`, and all structured fields

Both formats SHALL write to stderr only, never stdout.

#### Scenario: Default pretty format
- **WHEN** `OPENACE_LOG_FORMAT` is not set and a WARN event is emitted
- **THEN** output is a human-readable line on stderr with ANSI colors (if stderr is a TTY)

#### Scenario: JSON format
- **WHEN** `OPENACE_LOG_FORMAT=json` and a WARN event is emitted
- **THEN** output is a single JSON line on stderr with keys: `timestamp`, `level`, `target`, `fields`, `message`
- **AND** the JSON is valid (parseable by any JSON parser)

#### Scenario: No stdout contamination
- **WHEN** any tracing event is emitted at any level
- **THEN** no bytes are written to stdout

### Requirement: Log level control via environment variable
The system SHALL read `OPENACE_LOG_LEVEL` environment variable for the tracing filter. Default is `"warn"`. The filter SHALL support per-crate overrides (e.g., `"warn,oc_indexer=info,oc_retrieval=debug"`). Invalid filter strings SHALL fall back to `"warn"` with a stderr warning.

#### Scenario: Default level
- **WHEN** `OPENACE_LOG_LEVEL` is not set
- **THEN** only WARN and ERROR events are emitted

#### Scenario: Per-crate override
- **WHEN** `OPENACE_LOG_LEVEL=warn,oc_indexer=info`
- **THEN** oc-indexer INFO events are emitted
- **AND** other crates only emit WARN and above

#### Scenario: Invalid filter string
- **WHEN** `OPENACE_LOG_LEVEL=not_a_valid_level`
- **THEN** filter falls back to `"warn"`
- **AND** a single warning is printed to stderr about the invalid filter

### Requirement: tracing-log compatibility bridge
The system SHALL initialize `tracing_log::LogTracer` during subscriber setup to capture events from any library that uses the `log` crate and route them through the tracing subscriber.

#### Scenario: log crate events captured
- **WHEN** a dependency emits a `log::warn!("message")` event
- **THEN** it appears in the tracing output with the same level and target

## PBT Properties

### P1: Idempotent initialization
**Invariant**: Calling `init_tracing()` N times (N >= 1) results in exactly one subscriber being registered.
**Falsification**: Call `init_tracing()` from 10 concurrent threads; assert no panic and exactly one subscriber active.

### P2: Format determinism
**Invariant**: Given the same event and same `OPENACE_LOG_FORMAT`, the output format is always the same type (pretty or JSON).
**Falsification**: Emit 100 events with `OPENACE_LOG_FORMAT=json`; assert every line is valid JSON.

### P3: No stdout writes
**Invariant**: For any tracing event at any level, zero bytes are written to stdout.
**Falsification**: Redirect stdout to a buffer; emit events at all 5 levels; assert buffer is empty.
