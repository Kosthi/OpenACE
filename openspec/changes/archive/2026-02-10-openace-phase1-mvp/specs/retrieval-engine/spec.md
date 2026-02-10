## ADDED Requirements

### Requirement: Multi-signal retrieval with RRF fusion
The system SHALL provide a retrieval engine that combines multiple signals using Reciprocal Rank Fusion (RRF):

1. **BM25 full-text**: Query Tantivy for top-100 results
2. **Exact symbol match**: Query SQLite `symbols.name` and `symbols.qualified_name` for top-50 results
3. **Graph expansion**: For each hit from signals 1-2, expand k-hop neighbors (default k=2, max k=5, fanout=50/node)

RRF fusion formula: `score(item) = Σ 1/(rank_i + k)` where `k=60` (smoothing constant).

The RRF k parameter (60) SHALL be a compile-time constant. Signal pool sizes (BM25 top-100, exact-match top-50) SHALL be configurable via `SearchQuery`.

Vector search signal is available via the storage API but actual vector population is deferred to the Python embedding layer.

#### Scenario: Multi-signal search
- **WHEN** a query "authentication logic" is executed
- **THEN** BM25 results and exact symbol matches are fused via RRF, and graph expansion enriches each hit

#### Scenario: RRF score determinism
- **WHEN** the same query is executed twice on the same index
- **THEN** the ranked results and scores are identical

#### Scenario: RRF score monotonicity
- **WHEN** a document's rank improves in any signal
- **THEN** its fused RRF score increases or stays the same

### Requirement: Search query interface
The system SHALL expose a sync public API:
```rust
fn search(query: &SearchQuery) -> Result<Vec<SearchResult>>
```

`SearchQuery` fields:
- `text`: search text (required)
- `limit`: maximum results (default 10, max 100)
- `language_filter`: optional language filter
- `file_path_filter`: optional file path prefix filter
- `enable_graph_expansion`: whether to include graph-expanded results (default true)
- `graph_depth`: k-hop depth for expansion (default 2, max 5)
- `bm25_pool_size`: BM25 candidate pool size (default 100)
- `exact_match_pool_size`: exact match pool size (default 50)

`SearchResult` fields:
- `symbol_id`: SymbolId
- `name`: symbol name
- `qualified_name`: qualified name (in language-native display form)
- `kind`: SymbolKind
- `file_path`: relative file path
- `line_range`: (start, end) 0-indexed lines
- `score`: fused relevance score (f64)
- `match_signals`: which signals matched (bm25, exact, graph)
- `related_symbols`: graph-expanded neighbor symbols (if enabled)

#### Scenario: Search with language filter
- **WHEN** a search for "parse" is executed with `language_filter = Some(Python)`
- **THEN** only Python symbols are returned

#### Scenario: Search with graph expansion disabled
- **WHEN** a search is executed with `enable_graph_expansion = false`
- **THEN** results contain no `related_symbols` and only direct matches are returned

### Requirement: No duplicate results in fused output
The system SHALL deduplicate results across all signals before returning. If the same `SymbolId` appears in multiple signals, it SHALL appear once in the output with the combined RRF score from all contributing signals.

#### Scenario: Deduplication across signals
- **WHEN** symbol X appears at rank 1 in BM25 and rank 3 in exact match
- **THEN** the output contains X exactly once with score = 1/(1+60) + 1/(3+60)

### Requirement: Graceful handling of missing signals
The system SHALL handle cases where one or more signals return no results. If Tantivy is unavailable, BM25 is skipped and results come from exact match + graph only. If all signals fail, the system SHALL return an empty result set (not an error).

#### Scenario: Tantivy unavailable
- **WHEN** BM25 search fails due to corrupted Tantivy index
- **THEN** results come from exact match + graph expansion only

#### Scenario: No results from any signal
- **WHEN** a query matches nothing in any signal
- **THEN** an empty `Vec<SearchResult>` is returned (not an error)
