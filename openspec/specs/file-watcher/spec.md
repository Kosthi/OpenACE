## ADDED Requirements

### Requirement: File system watcher via notify-rs
The system SHALL monitor the project directory for file changes using the `notify` crate. The watcher SHALL detect file creation, modification, deletion, and rename events for source files matching the same filtering rules used during full indexing.

The watcher SHALL use a debounce interval of 300 milliseconds to coalesce rapid bursts of events (e.g., editor save sequences, branch switches).

#### Scenario: File modification detection
- **WHEN** a source file is modified and saved
- **THEN** the change is detected within 1 second

#### Scenario: Debounce coalescing
- **WHEN** a file is saved 5 times in rapid succession within 300ms
- **THEN** only a single change event is emitted for that file

#### Scenario: Branch switch handling
- **WHEN** a git branch switch modifies 100 files within 1 second
- **THEN** changes are coalesced and processed as a batch after the debounce window

### Requirement: Content-hash based change detection
The system SHALL compare the XXH3-128 content hash of a changed file against the stored hash in the `files` table. Only files whose hash has actually changed SHALL be queued for re-indexing. Metadata-only changes (e.g., touch command) SHALL NOT trigger re-indexing.

#### Scenario: Metadata-only change ignored
- **WHEN** a file's modification timestamp changes but content is identical
- **THEN** the file is NOT queued for re-indexing

#### Scenario: Content change detected
- **WHEN** a file's content hash differs from the stored hash
- **THEN** the file IS queued for re-indexing

### Requirement: Watcher respects file filtering rules
The system SHALL apply the same filtering rules as the full indexing pipeline:
- Ignore .gitignore patterns
- Ignore binary files
- Ignore files >1MB
- Ignore vendor/generated directories
- Ignore symlinks
- Only watch files with supported language extensions

#### Scenario: Ignored directory changes
- **WHEN** a file in `node_modules/` is modified
- **THEN** no change event is emitted

#### Scenario: New source file creation
- **WHEN** a new `.py` file is created in a watched directory
- **THEN** a creation event is emitted and the file is queued for indexing

### Requirement: Watcher lifecycle management
The system SHALL expose a sync API for watcher control:
- `fn start_watching(project_path: &Path) -> Result<WatcherHandle>`
- `fn stop_watching(handle: WatcherHandle) -> Result<()>`

The watcher SHALL run on a background thread. `stop_watching` SHALL perform a graceful shutdown: stop accepting new events, flush any pending events to the indexing queue, and join the background thread.

#### Scenario: Graceful shutdown
- **WHEN** `stop_watching` is called while events are pending
- **THEN** pending events are flushed before the watcher stops

#### Scenario: Watcher restarts cleanly
- **WHEN** the watcher is stopped and then started again
- **THEN** file monitoring resumes without missing changes that occurred during downtime (the next full scan detects drift)
