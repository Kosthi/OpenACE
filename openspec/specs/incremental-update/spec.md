## ADDED Requirements

### Requirement: Incremental update pipeline
The system SHALL provide an incremental update pipeline that processes individual file changes:

1. **Detect**: Receive changed file path from watcher or manual trigger
2. **Hash check**: Compare content hash against stored hash; skip if unchanged
3. **Re-parse**: Parse the changed file with tree-sitter
4. **Diff**: Compare old symbols (from SQLite) with new symbols (from parser) by symbol ID
5. **Update SQLite**: Within a transaction (max 100 rows/tx):
   - DELETE symbols that exist in old but not in new
   - INSERT symbols that exist in new but not in old
   - UPDATE symbols that exist in both but have different `body_hash`
   - CASCADE deletes automatically clean up orphan relations
   - INSERT new relations
6. **Update Tantivy**: Delete old documents by symbol_id, add new documents (batched commit)
7. **Update files table**: Update `content_hash`, `symbol_count`, `last_indexed`

#### Scenario: Single file incremental update
- **WHEN** a single source file is modified
- **THEN** only that file is re-parsed and only changed symbols are updated across SQLite and Tantivy

#### Scenario: Incremental latency target
- **WHEN** a single file with 50 symbols is updated
- **THEN** the incremental update completes in <500ms (excluding embedding)

### Requirement: Symbol diff based on deterministic IDs
The system SHALL compute the diff between old and new symbol sets using deterministic symbol IDs (XXH3-128). Since the ID incorporates `qualified_name` and `byte_range`, a symbol that moves within a file gets a new ID (old ID deleted, new ID inserted). A symbol with the same ID but different `body_hash` is an update.

Diff categories:
- **Added**: symbol ID exists in new but not in old → INSERT
- **Removed**: symbol ID exists in old but not in new → DELETE
- **Modified**: symbol ID exists in both but `body_hash` differs → UPDATE
- **Unchanged**: symbol ID exists in both with same `body_hash` → SKIP

#### Scenario: Symbol rename detection
- **WHEN** a function `foo` is renamed to `bar` in a file
- **THEN** the old symbol (with `foo` in qualified name) is deleted and a new symbol (with `bar`) is inserted

#### Scenario: Symbol body modification
- **WHEN** a function's body changes but its name and location stay the same
- **THEN** the symbol's `body_hash` changes, triggering an UPDATE of that symbol

#### Scenario: No-op on unchanged file
- **WHEN** incremental update is triggered on a file whose content hash matches
- **THEN** no SQLite or Tantivy operations are performed

### Requirement: Cross-store consistency during incremental updates
The system SHALL maintain consistency across SQLite and Tantivy during incremental updates. SQLite is the source of truth. The write order SHALL be:

1. SQLite: BEGIN TRANSACTION → delete old → insert new → COMMIT
2. Tantivy: delete old docs → add new docs (batched, committed per Tantivy's batch policy)

If SQLite commit fails, Tantivy operations SHALL NOT proceed. If Tantivy operations fail after SQLite commit, the inconsistency SHALL be logged and resolved on next startup via a consistency check (SQLite symbol IDs vs Tantivy document IDs).

#### Scenario: SQLite failure aborts Tantivy
- **WHEN** the SQLite transaction fails during incremental update
- **THEN** no Tantivy documents are modified

#### Scenario: Tantivy failure after SQLite commit
- **WHEN** SQLite commits successfully but Tantivy update fails
- **THEN** the error is logged and a consistency flag is set for next startup

### Requirement: File deletion handling
The system SHALL handle file deletions in the incremental pipeline:
1. Remove all symbols belonging to the deleted file from SQLite (CASCADE deletes relations)
2. Remove corresponding documents from Tantivy
3. Remove the file entry from the `files` table

#### Scenario: File deletion cleanup
- **WHEN** a source file is deleted
- **THEN** all its symbols, relations, Tantivy documents, and file metadata are removed

### Requirement: Convergence property
The system SHALL guarantee that the final index state depends only on the current file system state, not on the history of edits. Running a full re-index on the current file system SHALL produce the same result as applying any sequence of incremental updates that led to the same file system state.

#### Scenario: Incremental equals full re-index
- **WHEN** a file is modified 10 times incrementally and then compared to a fresh full index of the final state
- **THEN** the symbol sets, relation sets, and Tantivy documents are identical
