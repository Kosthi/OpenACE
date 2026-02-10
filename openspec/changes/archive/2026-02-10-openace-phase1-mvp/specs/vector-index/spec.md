## ADDED Requirements

### Requirement: HNSW vector index via usearch
The system SHALL provide an HNSW vector index using the `usearch` crate (v2.21.0) for k-nearest-neighbor search. The index file SHALL be stored at `<project_root>/.openace/vectors.usearch`.

Index configuration:
- Distance metric: cosine similarity
- Construction parameters: M=32, ef_construction=200
- Search parameter: ef_search=100
- Dimension: configurable at creation time (default 384)
- Quantization: f32 (no quantization in Phase 1)

The dimension SHALL be fixed at index creation. If a different dimension is requested on an existing index, the system SHALL return an error indicating that a full vector index rebuild is required.

#### Scenario: Index creation with default parameters
- **WHEN** a new vector index is created with dimension=384
- **THEN** the index is initialized with M=32, ef_construction=200, cosine distance

#### Scenario: Dimension mismatch detection
- **WHEN** a vector of dimension 1536 is added to an index created with dimension 384
- **THEN** the system returns a `StorageError::DimensionMismatch` error

### Requirement: Vector add and remove operations
The system SHALL support adding vectors with a `SymbolId` key and removing vectors by `SymbolId`. Adding a vector with an existing `SymbolId` SHALL update (overwrite) the existing vector. The live vector count SHALL equal the number of unique SymbolIds that have been added minus those removed.

#### Scenario: Add and retrieve vector
- **WHEN** a vector is added with symbol_id=X
- **THEN** a k-NN query with the same vector returns symbol_id=X as the top result

#### Scenario: Remove vector exclusion
- **WHEN** a vector with symbol_id=X is removed
- **THEN** subsequent k-NN queries never return symbol_id=X

#### Scenario: Idempotent add
- **WHEN** the same (symbol_id, vector) pair is added twice
- **THEN** the index contains exactly one entry for that symbol_id

### Requirement: K-NN search performance
The system SHALL achieve k-NN search latency of <10ms for k=10 on an index of 50,000 vectors with dimension 384. Recall SHALL be >90% compared to brute-force exhaustive search.

#### Scenario: Search latency benchmark
- **WHEN** a k-NN query with k=10 is executed on 50,000 vectors of dimension 384
- **THEN** results are returned in <10ms

#### Scenario: Search recall benchmark
- **WHEN** k-NN results are compared to brute-force results
- **THEN** recall (intersection of top-10 sets / 10) is >0.90

### Requirement: Vector index persistence and reload
The system SHALL persist the vector index to disk at `<project_root>/.openace/vectors.usearch`. On restart, the index SHALL be reloaded from disk without requiring re-computation of vectors.

#### Scenario: Persist and reload
- **WHEN** vectors are added, the index is saved, the process restarts, and the index is reloaded
- **THEN** k-NN queries return the same results as before the restart

### Requirement: Graceful degradation on vector index failure
The system SHALL handle vector index failures (corruption, I/O errors) gracefully. When the vector index is unavailable, the retrieval engine SHALL continue operating with BM25 full-text and graph signals only. A warning SHALL be logged indicating vector search is degraded.

#### Scenario: Corrupted vector index
- **WHEN** the vector index file is corrupted and cannot be loaded
- **THEN** the system logs a warning and operates without vector search capability
- **THEN** retrieval results come from BM25 and graph signals only
