## ADDED Requirements

### Requirement: SQLite graph storage with WAL mode
The system SHALL store symbols, relations, files, and repository metadata in SQLite using WAL (Write-Ahead Logging) mode. The database file SHALL be located at `<project_root>/.openace/db.sqlite`.

SQLite configuration:
- `PRAGMA journal_mode = WAL`
- `PRAGMA busy_timeout = 5000` (5 seconds)
- `PRAGMA synchronous = NORMAL`
- `PRAGMA foreign_keys = ON`

Connection model: single writer connection + read connection pool. All writes SHALL go through a single writer to avoid contention.

#### Scenario: WAL mode activation
- **WHEN** the database is opened
- **THEN** `PRAGMA journal_mode` returns `wal`

#### Scenario: Busy timeout prevents immediate failure
- **WHEN** two connections attempt concurrent writes
- **THEN** the second connection waits up to 5000ms before returning BUSY error

### Requirement: Database schema for symbols table
The system SHALL create a `symbols` table with the following schema:

```sql
CREATE TABLE symbols (
    id          BLOB PRIMARY KEY,    -- XXH3-128 as 16-byte BLOB
    name        TEXT NOT NULL,
    qualified_name TEXT NOT NULL,
    kind        INTEGER NOT NULL,    -- SymbolKind enum ordinal
    language    INTEGER NOT NULL,    -- Language enum ordinal
    file_path   TEXT NOT NULL,       -- relative to project root, forward-slash normalized
    line_start  INTEGER NOT NULL,    -- 0-indexed
    line_end    INTEGER NOT NULL,    -- 0-indexed, exclusive
    byte_start  INTEGER NOT NULL,
    byte_end    INTEGER NOT NULL,
    signature   TEXT,
    doc_comment TEXT,
    body_hash   INTEGER NOT NULL,    -- XXH3-128 lower 64 bits
    created_at  TEXT NOT NULL,       -- RFC 3339 UTC
    updated_at  TEXT NOT NULL        -- RFC 3339 UTC
);
```

Indexes:
- `idx_symbols_file ON symbols(file_path)`
- `idx_symbols_name ON symbols(name)`
- `idx_symbols_qualified ON symbols(qualified_name)`
- `idx_symbols_kind ON symbols(kind)`

#### Scenario: Symbol round-trip integrity
- **WHEN** a `CodeSymbol` is inserted and then queried by ID
- **THEN** all fields match the original values exactly

#### Scenario: File-based symbol lookup
- **WHEN** symbols are queried by `file_path`
- **THEN** all symbols belonging to that file are returned in O(log n) time via index

### Requirement: Database schema for relations table
The system SHALL create a `relations` table with the following schema:

```sql
CREATE TABLE relations (
    id          BLOB PRIMARY KEY,    -- XXH3-128 hash of (source_id|target_id|kind|file_path|line)
    source_id   BLOB NOT NULL REFERENCES symbols(id) ON DELETE CASCADE,
    target_id   BLOB NOT NULL REFERENCES symbols(id) ON DELETE CASCADE,
    kind        INTEGER NOT NULL,    -- RelationKind enum ordinal
    file_path   TEXT NOT NULL,
    line        INTEGER NOT NULL,    -- 0-indexed
    confidence  REAL NOT NULL,
    UNIQUE(source_id, target_id, kind, file_path, line)
);
```

Indexes:
- `idx_relations_source ON relations(source_id)`
- `idx_relations_target ON relations(target_id)`
- `idx_relations_kind ON relations(kind)`

Foreign key `ON DELETE CASCADE` SHALL ensure that deleting a symbol automatically removes all its relations (no orphans).

#### Scenario: Referential integrity on symbol deletion
- **WHEN** a symbol is deleted from the `symbols` table
- **THEN** all relations where the symbol is `source_id` or `target_id` are automatically deleted

#### Scenario: No orphan relations
- **WHEN** the database is queried for relations with non-existent source or target
- **THEN** zero results are returned

### Requirement: Database schema for files table
The system SHALL create a `files` table:

```sql
CREATE TABLE files (
    path          TEXT PRIMARY KEY,   -- relative to project root
    content_hash  INTEGER NOT NULL,   -- XXH3-128 lower 64 bits
    language      INTEGER NOT NULL,
    size_bytes    INTEGER NOT NULL,
    symbol_count  INTEGER NOT NULL,
    last_indexed  TEXT NOT NULL,      -- RFC 3339 UTC
    last_modified TEXT NOT NULL       -- RFC 3339 UTC
);
```

#### Scenario: File change detection via content hash
- **WHEN** a file's `content_hash` matches the stored value
- **THEN** the file is identified as unchanged

### Requirement: Database schema for repositories table
The system SHALL create a `repositories` table:

```sql
CREATE TABLE repositories (
    id          TEXT PRIMARY KEY,    -- SHA-256 of absolute project root path
    path        TEXT NOT NULL,
    name        TEXT NOT NULL,
    created_at  TEXT NOT NULL        -- RFC 3339 UTC
);
```

#### Scenario: Repository identity
- **WHEN** the same project is opened from the same absolute path
- **THEN** the repository ID is identical

### Requirement: Recursive CTE graph traversal with cycle detection
The system SHALL support k-hop graph traversal using SQLite recursive CTEs. The traversal SHALL:
- Accept parameters: `start_symbol_id`, `max_depth` (default 2, max 5), `max_fanout` (default 50 per node), `direction` (outgoing/incoming/both)
- Detect and break cycles using path tracking
- Limit results to `max_fanout` per expansion level

#### Scenario: K-hop query with k=3 on 10K symbols
- **WHEN** a k-hop query with depth=3 is executed on a graph with 10,000 symbols and 50,000 relations
- **THEN** results are returned in <50ms with no duplicate symbols

#### Scenario: Cycle detection in circular dependencies
- **WHEN** symbols A→B→C→A form a cycle
- **THEN** traversal from A returns {A, B, C} without infinite recursion

#### Scenario: Fanout limiting
- **WHEN** a symbol has 200 outgoing relations and fanout=50
- **THEN** only 50 neighbors are expanded at that level

#### Scenario: Depth cap enforcement
- **WHEN** max_depth=5 is requested
- **THEN** the traversal stops at depth 5 regardless of remaining reachable nodes

### Requirement: Batched transaction writes
The system SHALL batch SQLite writes into transactions of at most 1000 rows during full indexing and at most 100 rows during incremental updates. Each transaction SHALL be committed before starting the next batch.

#### Scenario: Bulk indexing transaction batching
- **WHEN** 5000 symbols are inserted during full indexing
- **THEN** exactly 5 transactions of 1000 rows each are committed

#### Scenario: Incremental update transaction batching
- **WHEN** 250 symbols are updated during incremental indexing
- **THEN** 3 transactions are committed (100, 100, 50 rows)

### Requirement: Schema version management
The system SHALL store the schema version using SQLite `PRAGMA user_version`. On startup, if the stored version does not match the expected version, the system SHALL delete the entire `.openace/` directory and trigger a full re-index.

#### Scenario: Schema version mismatch
- **WHEN** the database is opened and `user_version` is 1 but expected version is 2
- **THEN** the `.openace/` directory is deleted and a fresh database is created

#### Scenario: First run initialization
- **WHEN** no `.openace/` directory exists
- **THEN** the directory is created with `db.sqlite`, and `user_version` is set to the current schema version
