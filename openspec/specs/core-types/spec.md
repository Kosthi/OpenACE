## ADDED Requirements

### Requirement: Shared type definitions in oc-core crate
The system SHALL provide a shared `oc-core` crate containing all cross-crate type definitions, ensuring no direct type dependency between `oc-parser` and `oc-storage`.

The crate SHALL define the following types:
- `CodeSymbol`: Represents an extracted code symbol with fields: `id` (SymbolId), `name` (String), `qualified_name` (String), `kind` (SymbolKind), `language` (Language), `file_path` (relative PathBuf), `byte_range` (Range<usize>), `line_range` (Range<u32>), `signature` (Option<String>), `doc_comment` (Option<String>), `body_hash` (u64).
- `CodeRelation`: Represents a relationship between symbols with fields: `source_id` (SymbolId), `target_id` (SymbolId), `kind` (RelationKind), `file_path` (relative PathBuf), `line` (u32), `confidence` (f32).
- `SymbolKind`: Enum with variants: Function, Method, Class, Struct, Interface, Trait, Module, Package, Variable, Constant, Enum, TypeAlias.
- `RelationKind`: Enum with variants: Calls, Imports, Inherits, Implements, Uses, Contains.
- `Language`: Enum with variants: Python, TypeScript, JavaScript, Rust, Go, Java.
- `SymbolId`: Newtype wrapper around `u128` representing an XXH3-128 hash.

#### Scenario: Type independence between parser and storage
- **WHEN** `oc-storage` is compiled
- **THEN** it depends on `oc-core` but NOT on `oc-parser`

#### Scenario: All crates share identical type definitions
- **WHEN** a `CodeSymbol` is created in `oc-parser` and passed to `oc-storage`
- **THEN** no type conversion is required; the same `oc-core::CodeSymbol` struct is used

### Requirement: Deterministic symbol ID generation via XXH3-128
The system SHALL generate symbol IDs using XXH3-128 hash of the concatenation: `repo_id | relative_path | qualified_name | byte_start | byte_end`, using `|` (pipe) as field separator. The hash output SHALL be stored as `u128`.

The canonical input encoding SHALL be:
- `repo_id`: hex-encoded repository identity string
- `relative_path`: forward-slash normalized path relative to project root
- `byte_start` and `byte_end`: decimal ASCII representation of byte offsets

#### Scenario: Deterministic ID generation
- **WHEN** the same source file is parsed twice without changes
- **THEN** all generated symbol IDs are identical

#### Scenario: ID changes on content relocation
- **WHEN** a symbol is moved to a different file path
- **THEN** its symbol ID changes (because `relative_path` changed)

#### Scenario: ID stability across platforms
- **WHEN** the same file is parsed on macOS and Linux
- **THEN** the generated symbol IDs are identical (given same repo_id and normalized path)

### Requirement: File content hashing via XXH3-128
The system SHALL use XXH3-128 to hash raw file bytes (no newline normalization) for change detection. The hash SHALL be stored as `u64` in the `files` table `content_hash` field (using the lower 64 bits of XXH3-128).

#### Scenario: Unchanged file detection
- **WHEN** a file's content hash matches the stored hash
- **THEN** the file is skipped during incremental indexing

#### Scenario: Hash sensitivity
- **WHEN** a single byte in the file changes
- **THEN** the content hash changes

### Requirement: Qualified name normalization
The system SHALL store qualified names using dot-separated segments internally (e.g., `std.collections.HashMap`). The system SHALL accept language-native input forms (`std::collections::HashMap` for Rust, `net/http.Client.Do` for Go) and normalize them to dot-separated canonical form for indexing and search. The system SHALL render results back in language-native form based on the symbol's `language` field.

Normalization rules:
- Rust `::` separator → `.`
- Go `/` package separator → `.`
- Python `.` → `.` (identity)
- TypeScript/JavaScript `.` → `.` (identity)
- Java `.` → `.` (identity)

#### Scenario: Cross-language search
- **WHEN** a user searches for `std.collections.HashMap`
- **THEN** the system matches the Rust symbol originally named `std::collections::HashMap`

#### Scenario: Language-native rendering
- **WHEN** a Rust symbol with internal qualified name `std.collections.HashMap` is returned in search results
- **THEN** the display form shows `std::collections::HashMap`

### Requirement: Typed error hierarchy
The system SHALL define error types using `thiserror` with the following hierarchy:
- `oc-core`: `CoreError` (hash computation failures, type conversion errors)
- `oc-parser`: `ParserError` (parse failures, unsupported language, file too large)
- `oc-storage`: `StorageError` (SQLite errors, Tantivy errors, usearch errors, transaction failures)
- `oc-indexer`: `IndexerError` (pipeline failures, watcher errors, batch failures)
- `oc-retrieval`: `RetrievalError` (query errors, fusion errors, expansion errors)

Each error SHALL carry context: file path (when applicable), operation name, and whether the error is retryable.

#### Scenario: Retryable error classification
- **WHEN** SQLite returns BUSY error
- **THEN** the error is classified as retryable with `is_retryable() == true`

#### Scenario: Non-retryable error classification
- **WHEN** a file exceeds the 1MB size limit
- **THEN** the error is classified as non-retryable with `is_retryable() == false`

### Requirement: Relation confidence constants
The system SHALL assign fixed confidence scores per relation kind for tree-sitter extracted relations:
- `Calls`: 0.8
- `Imports`: 0.9
- `Inherits`: 0.85
- `Implements`: 0.85
- `Uses`: 0.7
- `Contains`: 0.95

LSP-derived relations (future) SHALL use confidence 1.0.

#### Scenario: Confidence assignment during parsing
- **WHEN** a tree-sitter parser extracts a `Calls` relation
- **THEN** the relation's confidence field is set to exactly 0.8

#### Scenario: Confidence values are immutable constants
- **WHEN** any relation is created via tree-sitter extraction
- **THEN** the confidence value matches the fixed constant for its `RelationKind`
