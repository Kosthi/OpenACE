## ADDED Requirements

### Requirement: Trace-ID parameter on EngineBinding methods
The system SHALL add an optional `trace_id: Option<String>` parameter to the following `EngineBinding` methods:
- `index_full(repo_root, chunk_enabled, summary_enabled, trace_id=None)`
- `search(text, query_vector, limit, ..., trace_id=None)`
- `find_symbol(name, trace_id=None)`
- `get_file_outline(path, trace_id=None)`

When `trace_id` is `Some(id)`, it SHALL be recorded as a field in the method's top-level tracing span.

#### Scenario: Trace-ID in Rust span
- **WHEN** Python calls `search(text, trace_id="abc123")`
- **THEN** the Rust tracing span for that search includes `trace_id="abc123"`

#### Scenario: No trace-ID (backward compatible)
- **WHEN** Python calls `search(text)` without trace_id
- **THEN** the span has an empty trace_id field (or omits it)
- **AND** no error occurs

### Requirement: Trace-ID generation in Python Engine
The system SHALL generate a 16-character hex trace-id (UUID4 lower 64 bits) in each public `Engine` method: `index()`, `search()`, `find_symbol()`, `get_file_outline()`, `embed_all()`. The trace-id SHALL be:
1. Bound to the structlog context via `structlog.contextvars.bind_contextvars(trace_id=tid)`
2. Passed to `EngineBinding` methods via the `trace_id` parameter
3. Unbound after the method completes (via `unbind_contextvars`)

#### Scenario: Trace-ID appears in both layers
- **WHEN** `Engine.search("query")` is called
- **THEN** Python structlog events include `trace_id` field
- **AND** Rust tracing events include the same `trace_id` value

#### Scenario: Unique trace-ID per call
- **WHEN** `Engine.search("q1")` and `Engine.search("q2")` are called sequentially
- **THEN** each call has a different trace_id

#### Scenario: MCP server trace-ID propagation
- **WHEN** MCP server handles a search tool call via `asyncio.to_thread(engine.search, ...)`
- **THEN** the trace_id is propagated to the background thread via function parameter (not contextvars)

### Requirement: Trace-ID in MCP server responses
The system SHALL include the trace-id in MCP error responses for debugging purposes. When a tool call fails, the error TextContent SHALL include the trace-id.

#### Scenario: Error response with trace-ID
- **WHEN** a search tool call fails with SearchError
- **THEN** the error response includes `[trace_id=abc123] Error: ...` prefix

## PBT Properties

### P10: Trace-ID uniqueness
**Invariant**: For any sequence of N Engine method calls, all N trace-ids are unique.
**Falsification**: Call `Engine.search()` 1000 times; collect trace-ids; assert all unique.

### P11: Trace-ID correlation
**Invariant**: For any Engine method call, the trace-id in Python structlog output matches the trace-id in Rust tracing output.
**Falsification**: Capture both Python and Rust log output for one search call; extract trace-ids; assert equal.

### P12: Trace-ID cleanup
**Invariant**: After an Engine method completes (success or exception), the trace-id is no longer in the structlog contextvars context.
**Falsification**: Call `Engine.search()`; after completion, check `structlog.contextvars.get_contextvars()` for trace_id; assert absent.
