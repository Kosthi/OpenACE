## ADDED Requirements

### Requirement: Full indexing pipeline
The system SHALL provide a full indexing pipeline that processes all source files in a project:

1. **Scan**: Walk project directory recursively
2. **Filter**: Skip files matching .gitignore patterns, binary files, files >1MB, vendor/generated directories
3. **Parse**: Parse files in parallel using rayon thread pool
4. **Store**: Write symbols and relations to SQLite in batched transactions (1000 rows/tx)
5. **Index**: Add symbol documents to Tantivy full-text index with batched commits

Vector embedding is deferred to the Python layer — the pipeline SHALL prepare symbol data for later embedding via the vector store API.

The pipeline SHALL be exposed as a sync (blocking) public API: `fn index(project_path: &Path, config: &IndexConfig) -> Result<IndexReport>`.

#### Scenario: Full index of multi-language project
- **WHEN** `index()` is called on a project with 10,000 source files across 5 languages
- **THEN** all eligible files are parsed, symbols/relations stored in SQLite, and full-text index built

#### Scenario: Performance target
- **WHEN** 10,000 files are indexed
- **THEN** total indexing time is <30 seconds (parsing + SQLite + Tantivy, excluding embeddings)

#### Scenario: Throughput target
- **WHEN** parsing throughput is measured during full indexing
- **THEN** the parser achieves >50,000 symbols/second

### Requirement: File filtering rules
The system SHALL filter files during scanning using the following rules:
- Respect `.gitignore` patterns (using the `ignore` crate)
- Skip binary files (detected by null bytes in first 8KB)
- Skip files >1MB
- Skip vendor directories: `vendor/`, `node_modules/`, `third_party/`, `.venv/`, `venv/`
- Skip generated file patterns: `*.generated.*`, `*.min.js`, `*.min.css`, `*_pb2.py`, `*.pb.go`
- Skip hidden directories (starting with `.`) except `.openace/` is the storage location, not a source
- Follow only regular files (skip symlinks to prevent infinite loops)

#### Scenario: Gitignore respect
- **WHEN** `.gitignore` contains `build/`
- **THEN** all files under `build/` are skipped

#### Scenario: Binary file detection
- **WHEN** a file contains null bytes in its first 8KB
- **THEN** the file is skipped as binary

#### Scenario: Symlink skipping
- **WHEN** a symlink points to a directory
- **THEN** the symlink is not followed

### Requirement: Parallel parsing with rayon
The system SHALL use a rayon thread pool for parallel file parsing. The thread pool size SHALL default to the number of available CPU cores. Parsed results SHALL be collected and written to storage sequentially (single writer model).

File processing order SHALL NOT affect the final index state — the same set of files always produces the same set of symbols and relations regardless of processing order.

#### Scenario: Order-independent indexing
- **WHEN** the same project is indexed twice with different file processing orders
- **THEN** the resulting symbol sets and relation sets are identical

#### Scenario: Parallel scaling
- **WHEN** parsing is measured on a machine with N cores
- **THEN** throughput scales approximately linearly with N (within 80% efficiency)

### Requirement: Index report generation
The system SHALL return an `IndexReport` after full indexing containing:
- `total_files_scanned`: number of files found
- `files_indexed`: number of files successfully parsed and stored
- `files_skipped`: number of files skipped (with reasons: too large, binary, ignored, unsupported language)
- `files_failed`: number of files that failed to parse (with error details)
- `total_symbols`: number of symbols stored
- `total_relations`: number of relations stored
- `duration`: total indexing time

#### Scenario: Report accuracy
- **WHEN** a project with 100 files (80 parseable, 10 binary, 5 too large, 5 unsupported) is indexed
- **THEN** the report shows files_indexed=80, files_skipped=20 with appropriate reasons

### Requirement: Storage directory initialization
The system SHALL create the `.openace/` directory structure on first run:
- `.openace/db.sqlite` — SQLite database
- `.openace/tantivy/` — Tantivy index directory
- `.openace/vectors.usearch` — Vector index file (created when first vector is added)

If `.openace/` exists but fails integrity checks (wrong schema version, corrupted SQLite), the system SHALL purge the entire directory and re-initialize.

#### Scenario: First run initialization
- **WHEN** no `.openace/` directory exists
- **THEN** the directory is created with all sub-components initialized

#### Scenario: Corrupted state recovery
- **WHEN** `.openace/db.sqlite` is corrupted
- **THEN** the entire `.openace/` directory is deleted and re-created
