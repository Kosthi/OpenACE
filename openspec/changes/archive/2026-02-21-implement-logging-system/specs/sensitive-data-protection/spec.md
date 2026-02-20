## ADDED Requirements

### Requirement: Rust-side sensitive field exclusion
The system SHALL ensure that the following data never appears in tracing output:
- File content (`content` parameter) — enforced via `skip(content)` on all `#[instrument]` macros
- Embedding vectors (`query_vector`, `vectors`) — enforced via `skip(query_vector)`, `skip(vectors)`
- API keys, tokens, secrets — never added as span fields

This is a static code-level guarantee, not a runtime filter.

#### Scenario: File content excluded
- **WHEN** `parse_file(path, content)` is called at TRACE level
- **THEN** `content` does not appear in any span field or event message

#### Scenario: Vector data excluded
- **WHEN** `search(query_vector=vec![0.1, 0.2, ...])` is called at TRACE level
- **THEN** vector values do not appear in any span field

### Requirement: Python-side sensitive field redaction processor
The system SHALL implement a structlog processor `_redact_sensitive_fields` that:
1. Scans all event dict keys (case-insensitive)
2. Replaces values for keys containing: `key`, `secret`, `token`, `password`, `authorization`
3. Replacement value: `"[REDACTED]"`
4. Processor runs early in the pipeline (before rendering)

#### Scenario: API key redaction
- **WHEN** a log event includes `api_key="sk-abc123"`
- **THEN** the rendered output shows `api_key=[REDACTED]`

#### Scenario: Non-sensitive fields preserved
- **WHEN** a log event includes `file_path="/src/main.py"` and `symbol_count=42`
- **THEN** both fields appear in full in the rendered output

#### Scenario: Case-insensitive matching
- **WHEN** a log event includes `API_KEY="sk-abc"` or `ApiKey="sk-abc"`
- **THEN** the value is redacted in both cases

### Requirement: Exception message sanitization
The system SHALL NOT log full stack traces at WARN level or above. Stack traces SHALL only appear at DEBUG level. Exception type and message are always logged.

#### Scenario: WARN-level exception
- **WHEN** an exception is caught and logged at WARN level
- **THEN** output includes exception type and message but NOT the full traceback

#### Scenario: DEBUG-level exception
- **WHEN** an exception is caught and logged at DEBUG level with `exc_info=True`
- **THEN** output includes the full traceback

## PBT Properties

### P13: Redaction completeness
**Invariant**: For any event dict containing a key matching the sensitive patterns, the rendered output never contains the original value.
**Falsification**: Generate random event dicts with keys containing "key", "secret", "token", "password", "authorization" and random values; process through redactor; assert original values absent in output.

### P14: Redaction preserves non-sensitive data
**Invariant**: For any event dict with keys NOT matching sensitive patterns, all values appear unchanged in output.
**Falsification**: Generate random event dicts with safe keys (e.g., "count", "path", "duration"); process through redactor; assert all values preserved.
