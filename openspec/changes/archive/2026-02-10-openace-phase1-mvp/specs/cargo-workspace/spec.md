## ADDED Requirements

### Requirement: Rust workspace with 5 crates
The system SHALL be organized as a Cargo workspace with 5 crates:
- `oc-core`: Shared types, error definitions, and utility functions
- `oc-parser`: Tree-sitter multi-language AST parser
- `oc-storage`: SQLite graph + usearch HNSW vectors + Tantivy full-text
- `oc-indexer`: Indexing pipeline, file watcher, incremental updates
- `oc-retrieval`: Multi-signal retrieval engine with RRF fusion

The dependency graph SHALL be:
- `oc-core`: no workspace dependencies
- `oc-parser` → `oc-core`
- `oc-storage` → `oc-core`
- `oc-indexer` → `oc-core`, `oc-parser`, `oc-storage`
- `oc-retrieval` → `oc-core`, `oc-storage`

The dependency graph SHALL be acyclic. `oc-storage` SHALL NOT depend on `oc-parser`. `oc-retrieval` SHALL NOT depend on `oc-parser` or `oc-indexer`.

#### Scenario: Clean build from fresh clone
- **WHEN** `cargo build` is run on a fresh clone with Rust 1.85.0+
- **THEN** all 5 crates compile without errors

#### Scenario: Dependency graph is acyclic and layer-safe
- **WHEN** `cargo metadata` is inspected
- **THEN** no circular dependencies exist and forbidden reverse edges are absent

### Requirement: MSRV compatibility at Rust 1.85.0
The system SHALL compile with Rust 1.85.0 as the Minimum Supported Rust Version (MSRV). The workspace `Cargo.toml` SHALL declare `rust-version = "1.85.0"`. The `Cargo.lock` file SHALL be committed to the repository to pin transitive dependency versions that are compatible with this MSRV.

#### Scenario: MSRV build succeeds
- **WHEN** `cargo +1.85.0 check --locked` is run
- **THEN** the build succeeds without errors

#### Scenario: Transitive dependency compatibility
- **WHEN** a new dependency is added
- **THEN** `cargo +1.85.0 check --locked` MUST still pass before the change is accepted

### Requirement: Shared workspace dependency versions
The system SHALL use workspace-level dependency declarations in the root `Cargo.toml` for all shared dependencies. Individual crate `Cargo.toml` files SHALL reference workspace dependencies using `dep.workspace = true`.

Key pinned versions:
- `tantivy = "0.25.0"`
- `usearch = "2.21.0"`
- `rusqlite` with `bundled` feature
- `tree-sitter = "0.26.x"` (compatible with all grammar crates)
- `xxhash-rust` with `xxh3` feature
- `thiserror` for error types
- `rayon` for parallel parsing
- `serde` + `serde_json` for serialization

#### Scenario: No version conflicts across crates
- **WHEN** `cargo tree --duplicates` is run
- **THEN** no duplicate major versions of critical dependencies exist
