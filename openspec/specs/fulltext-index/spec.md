## ADDED Requirements

### Requirement: Tantivy full-text index with code-aware tokenizer
The system SHALL provide a full-text search index using Tantivy (v0.25.0) stored at `<project_root>/.openace/tantivy/`. The index SHALL use a custom code-aware tokenizer that splits identifiers on camelCase, PascalCase, snake_case, and SCREAMING_SNAKE_CASE boundaries.

Tokenizer splitting rules:
- camelCase boundaries: `parseXMLStream` → `parse`, `XML`, `Stream`
- PascalCase boundaries: `HTMLParser` → `HTML`, `Parser`
- snake_case underscores: `user_service` → `user`, `service`
- Leading/trailing underscores stripped: `__init__` → `init`
- Numbers separated: `i18n` → `i`, `18`, `n`
- All tokens lowercased for case-insensitive matching

#### Scenario: camelCase search matching
- **WHEN** a symbol named `getUserById` is indexed
- **THEN** searching for `user` returns this symbol

#### Scenario: Cross-case matching
- **WHEN** symbols named `UserService`, `user_service`, and `userService` are indexed
- **THEN** searching for `user_service` matches all three

#### Scenario: Acronym handling
- **WHEN** a symbol named `HTMLParser` is indexed
- **THEN** searching for `html` matches it and searching for `parser` matches it

#### Scenario: Dunder stripping
- **WHEN** a Python symbol named `__init__` is indexed
- **THEN** searching for `init` matches it

### Requirement: Full-text index schema
The system SHALL index the following fields per symbol:
- `symbol_id` (STORED, not indexed): SymbolId for joining back to SQLite
- `name` (TEXT, tokenized with code-aware tokenizer): symbol name
- `qualified_name` (TEXT, tokenized with code-aware tokenizer): dot-separated qualified name
- `content` (TEXT, tokenized with code-aware tokenizer): symbol body text, head-truncated to 10,240 bytes (10KB)
- `file_path` (STRING, exact match): relative file path for filtering
- `language` (STRING, exact match): language name for filtering

#### Scenario: Body truncation at 10KB
- **WHEN** a symbol has a body of 50KB
- **THEN** only the first 10,240 bytes are indexed in the `content` field

#### Scenario: Exact file path filtering
- **WHEN** a search includes a file_path filter for `src/auth.py`
- **THEN** only symbols from that exact file are returned

### Requirement: Batched Tantivy commits
The system SHALL batch Tantivy document additions and commit based on the FIRST threshold reached:
- Time threshold: 500 milliseconds since last commit
- Count threshold: 500 documents since last commit
- Shutdown: forced commit on graceful shutdown

The system SHALL NOT commit per-document. After commit, a new reader SHALL be opened to make committed documents searchable.

#### Scenario: Time-based commit trigger
- **WHEN** 100 documents are added and 500ms elapses
- **THEN** a commit occurs (time threshold reached before count threshold)

#### Scenario: Count-based commit trigger
- **WHEN** 500 documents are added in 200ms
- **THEN** a commit occurs (count threshold reached before time threshold)

#### Scenario: Shutdown commit
- **WHEN** the system shuts down with 50 uncommitted documents
- **THEN** a final commit is performed before shutdown

### Requirement: BM25 search performance
The system SHALL achieve BM25 search latency of <50ms for queries against an index of 50,000 documents.

#### Scenario: Search latency benchmark
- **WHEN** a BM25 query is executed on 50,000 indexed symbols
- **THEN** results are returned in <50ms

### Requirement: Graceful degradation on full-text index failure
The system SHALL handle Tantivy index failures gracefully. When the full-text index is unavailable, the retrieval engine SHALL continue operating with graph traversal and exact symbol-name matching via SQLite LIKE queries. A warning SHALL be logged.

#### Scenario: Corrupted Tantivy index
- **WHEN** the Tantivy index directory is corrupted
- **THEN** the system logs a warning and falls back to SQLite LIKE queries for text search
