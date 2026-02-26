use std::collections::{HashMap, HashSet};

use oc_core::{CodeSymbol, Language, QualifiedName, RelationKind, SymbolId, SymbolKind};
use oc_storage::graph::TraversalDirection;
use oc_storage::manager::StorageManager;

use crate::error::RetrievalError;

/// RRF smoothing constant (k in `1/(rank + k)`).
const RRF_K: f64 = 60.0;

/// Search query with all configurable parameters.
#[derive(Debug, Clone)]
pub struct SearchQuery {
    pub text: String,
    pub limit: usize,
    pub language_filter: Option<Language>,
    pub file_path_filter: Option<String>,
    pub enable_graph_expansion: bool,
    pub graph_depth: u32,
    pub bm25_pool_size: usize,
    pub exact_match_pool_size: usize,
    pub query_vector: Option<Vec<f32>>,
    pub vector_pool_size: usize,
    /// Enable chunk-level BM25 search as an additional signal.
    pub enable_chunk_search: bool,
    /// Pool size for chunk BM25 retrieval.
    pub chunk_bm25_pool_size: usize,
    /// Weight multiplier for BM25 signal (default 1.0).
    pub bm25_weight: f64,
    /// Weight multiplier for vector k-NN signal (default 1.0).
    pub vector_weight: f64,
    /// Weight multiplier for exact match signal (default 1.0).
    pub exact_weight: f64,
    /// Weight multiplier for chunk BM25 signal (default 1.0).
    pub chunk_bm25_weight: f64,
    /// Weight multiplier for graph expansion signal (default 1.0).
    pub graph_weight: f64,
    /// BM25-specific query text; falls back to `text` if None.
    pub bm25_text: Option<String>,
    /// Identifiers for exact match signal; falls back to `text` if empty.
    pub exact_queries: Vec<String>,
    /// Enable relation-aware graph expansion sub-signals.
    /// When true, graph expansion is split into separate callers (Calls
    /// incoming), callees (Calls outgoing), and hierarchy (Contains)
    /// sub-signals, each scored independently via RRF.
    /// When false, uses the original undirected all-relation-type expansion.
    pub enable_relation_aware_graph: bool,
    /// Weight multiplier for callee graph signal (Calls outgoing).
    pub callee_weight: f64,
    /// Weight multiplier for caller graph signal (Calls incoming).
    pub caller_weight: f64,
    /// Weight multiplier for hierarchy graph signal (Contains).
    pub hierarchy_weight: f64,
}

impl SearchQuery {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            limit: 10,
            language_filter: None,
            file_path_filter: None,
            enable_graph_expansion: true,
            graph_depth: 2,
            bm25_pool_size: 100,
            exact_match_pool_size: 50,
            query_vector: None,
            vector_pool_size: 100,
            enable_chunk_search: true,
            chunk_bm25_pool_size: 100,
            bm25_weight: 1.0,
            vector_weight: 1.0,
            exact_weight: 1.0,
            chunk_bm25_weight: 1.0,
            graph_weight: 1.0,
            bm25_text: None,
            exact_queries: Vec::new(),
            enable_relation_aware_graph: true,
            callee_weight: 1.5,
            caller_weight: 1.2,
            hierarchy_weight: 0.8,
        }
    }

    fn effective_limit(&self) -> usize {
        self.limit.min(200)
    }

    fn effective_graph_depth(&self) -> u32 {
        self.graph_depth.min(5)
    }

    /// Return the BM25-specific text, falling back to the generic `text`.
    pub fn effective_bm25_text(&self) -> &str {
        self.bm25_text.as_deref().unwrap_or(&self.text)
    }
}

/// A single search result with fused score and provenance.
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub symbol_id: SymbolId,
    pub name: String,
    pub qualified_name: String,
    pub kind: SymbolKind,
    pub file_path: String,
    pub line_range: (u32, u32),
    pub score: f64,
    pub match_signals: Vec<String>,
    pub related_symbols: Vec<SearchResult>,
    pub snippet: Option<String>,
    /// If this result was boosted by chunk BM25, contains the context path.
    pub chunk_info: Option<ChunkInfo>,
}

/// Information about a chunk that boosted a search result.
#[derive(Debug, Clone)]
pub struct ChunkInfo {
    pub context_path: String,
    pub chunk_score: f32,
}

/// A node in a call chain traversal result.
#[derive(Debug, Clone)]
pub struct CallChainNode {
    pub symbol_id: SymbolId,
    pub name: String,
    pub qualified_name: String,
    pub kind: SymbolKind,
    pub file_path: String,
    pub line_range: (u32, u32),
    pub depth: u32,
    pub signature: Option<String>,
    pub doc_comment: Option<String>,
    pub snippet: Option<String>,
}

/// Structured function context: callers, callees, and hierarchy around a symbol.
#[derive(Debug, Clone)]
pub struct FunctionContext {
    pub symbol: CallChainNode,
    pub callers: Vec<CallChainNode>,
    pub callees: Vec<CallChainNode>,
    pub hierarchy: Vec<CallChainNode>,
}

/// Accumulator for per-symbol RRF scoring.
#[derive(Debug)]
struct ScoredCandidate {
    symbol_id: SymbolId,
    score: f64,
    signals: Vec<String>,
}

/// Multi-signal retrieval engine combining BM25, exact match, and graph expansion.
pub struct RetrievalEngine<'a> {
    storage: &'a StorageManager,
}

impl<'a> RetrievalEngine<'a> {
    pub fn new(storage: &'a StorageManager) -> Self {
        Self { storage }
    }

    /// Execute a multi-signal search, fuse results via RRF, and return ranked hits.
    #[tracing::instrument(skip(self, query), fields(query, limit, result_count))]
    pub fn search(&self, query: &SearchQuery) -> Result<Vec<SearchResult>, RetrievalError> {
        let query_text = if query.text.len() > 100 { &query.text[..100] } else { &query.text };
        let span = tracing::Span::current();
        span.record("query", query_text);
        span.record("limit", query.limit);
        tracing::debug!("search started");

        let mut candidates: HashMap<SymbolId, ScoredCandidate> = HashMap::new();

        // Signal 1: BM25 full-text search
        self.collect_bm25(query, &mut candidates);

        // Signal 2: Vector k-NN search
        self.collect_vector(query, &mut candidates);

        // Signal 3: Exact symbol name match
        self.collect_exact_match(query, &mut candidates);

        // Signal 4: Chunk BM25 (file-level boost from chunk hits)
        if query.enable_chunk_search {
            self.collect_bm25_chunks(query, &mut candidates);
        }

        // Collect seed IDs (direct signal hits) before graph expansion
        let direct_hit_ids: HashSet<SymbolId> = candidates.keys().copied().collect();

        // Signal 5: Graph expansion (applied to hits from direct signals)
        if query.enable_graph_expansion && !candidates.is_empty() {
            if query.enable_relation_aware_graph {
                self.expand_graph_relation_aware(query, &mut candidates);
            } else {
                self.expand_graph(query, &mut candidates);
            }
        }

        // Fuse, sort, truncate
        let limit = query.effective_limit();
        let mut scored: Vec<ScoredCandidate> = candidates.into_values().collect();
        scored.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.symbol_id.cmp(&b.symbol_id))
        });
        scored.truncate(limit);

        // Hydrate symbols and attach related_symbols for direct hits
        let mut results = Vec::with_capacity(scored.len());
        for candidate in &scored {
            if let Some(result) = self.hydrate_candidate(candidate)? {
                results.push(result);
            }
        }

        // Populate related_symbols: for each direct hit, attach graph-expanded neighbors
        if query.enable_graph_expansion {
            self.attach_related_symbols(&direct_hit_ids, &mut results, query)?;
        }

        tracing::Span::current().record("result_count", results.len());
        tracing::info!(fused_count = results.len(), "search completed");

        Ok(results)
    }

    /// BM25 signal: query Tantivy, assign RRF scores by rank.
    fn collect_bm25(
        &self,
        query: &SearchQuery,
        candidates: &mut HashMap<SymbolId, ScoredCandidate>,
    ) {
        let bm25_text = query.effective_bm25_text();
        let hits = self.storage.fulltext().search_bm25(
            bm25_text,
            query.bm25_pool_size,
            query.file_path_filter.as_deref(),
            query.language_filter,
        );

        let hits = match hits {
            Ok(h) => h,
            Err(e) => {
                tracing::warn!(signal = "bm25", error = %e, "signal failed, skipping");
                return;
            }
        };

        tracing::debug!(signal = "bm25", count = hits.len(), "signal collected");

        for (rank, hit) in hits.iter().enumerate() {
            let rrf_score = 1.0 / (rank as f64 + 1.0 + RRF_K);
            let entry = candidates
                .entry(hit.symbol_id)
                .or_insert_with(|| ScoredCandidate {
                    symbol_id: hit.symbol_id,
                    score: 0.0,
                    signals: Vec::new(),
                });
            entry.score += query.bm25_weight * rrf_score;
            if !entry.signals.contains(&"bm25".to_string()) {
                entry.signals.push("bm25".to_string());
            }
        }
    }

    /// Vector k-NN signal: query the vector store, assign RRF scores by rank.
    fn collect_vector(
        &self,
        query: &SearchQuery,
        candidates: &mut HashMap<SymbolId, ScoredCandidate>,
    ) {
        let query_vec = match &query.query_vector {
            Some(v) => v,
            None => return,
        };

        let hits = match self.storage.vector().search_knn(query_vec, query.vector_pool_size) {
            Ok(h) => h,
            Err(e) => {
                tracing::warn!(signal = "vector", error = %e, "signal failed, skipping");
                return;
            }
        };

        tracing::debug!(signal = "vector", count = hits.len(), "signal collected");

        for (rank, hit) in hits.iter().enumerate() {
            let rrf_score = 1.0 / (rank as f64 + 1.0 + RRF_K);
            let entry = candidates
                .entry(hit.symbol_id)
                .or_insert_with(|| ScoredCandidate {
                    symbol_id: hit.symbol_id,
                    score: 0.0,
                    signals: Vec::new(),
                });
            entry.score += query.vector_weight * rrf_score;
            if !entry.signals.contains(&"vector".to_string()) {
                entry.signals.push("vector".to_string());
            }
        }
    }

    /// Exact match signal: query SQLite name and qualified_name columns.
    /// If `exact_queries` is non-empty, iterate over each identifier;
    /// otherwise fall back to `text` for backward compatibility.
    fn collect_exact_match(
        &self,
        query: &SearchQuery,
        candidates: &mut HashMap<SymbolId, ScoredCandidate>,
    ) {
        let mut exact_hits: Vec<CodeSymbol> = Vec::new();
        let mut seen: HashSet<SymbolId> = HashSet::new();

        let terms: Vec<&str> = if query.exact_queries.is_empty() {
            vec![&query.text]
        } else {
            query.exact_queries.iter().map(|s| s.as_str()).collect()
        };

        for term in &terms {
            // Name match
            match self.storage.graph().get_symbols_by_name(term) {
                Ok(syms) => {
                    for sym in syms {
                        if seen.insert(sym.id) {
                            exact_hits.push(sym);
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(signal = "exact", error = %e, "signal failed on name lookup, skipping");
                }
            }

            // Qualified name match
            match self.storage.graph().get_symbols_by_qualified_name(term) {
                Ok(syms) => {
                    for sym in syms {
                        if seen.insert(sym.id) {
                            exact_hits.push(sym);
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(signal = "exact", error = %e, "signal failed on qualified name lookup, skipping");
                }
            }
        }

        // Apply filters
        exact_hits.retain(|sym| {
            if let Some(lang) = query.language_filter {
                if sym.language != lang {
                    return false;
                }
            }
            if let Some(ref prefix) = query.file_path_filter {
                if !sym.file_path.to_string_lossy().starts_with(prefix.as_str()) {
                    return false;
                }
            }
            true
        });

        exact_hits.truncate(query.exact_match_pool_size);

        tracing::debug!(signal = "exact", count = exact_hits.len(), "signal collected");

        for (rank, sym) in exact_hits.iter().enumerate() {
            let rrf_score = 1.0 / (rank as f64 + 1.0 + RRF_K);
            let entry = candidates
                .entry(sym.id)
                .or_insert_with(|| ScoredCandidate {
                    symbol_id: sym.id,
                    score: 0.0,
                    signals: Vec::new(),
                });
            entry.score += query.exact_weight * rrf_score;
            if !entry.signals.contains(&"exact".to_string()) {
                entry.signals.push("exact".to_string());
            }
        }
    }

    /// Chunk BM25 signal: query Tantivy for chunk documents, map chunk hits
    /// to file-level candidates. Each chunk hit boosts the best symbol in its file.
    fn collect_bm25_chunks(
        &self,
        query: &SearchQuery,
        candidates: &mut HashMap<SymbolId, ScoredCandidate>,
    ) {
        let bm25_text = query.effective_bm25_text();
        let hits = self.storage.fulltext().search_bm25_chunks(
            bm25_text,
            query.chunk_bm25_pool_size,
            query.file_path_filter.as_deref(),
            query.language_filter,
        );

        let hits = match hits {
            Ok(h) => h,
            Err(e) => {
                tracing::warn!(signal = "chunk_bm25", error = %e, "signal failed, skipping");
                return;
            }
        };

        tracing::debug!(signal = "chunk_bm25", count = hits.len(), "signal collected");

        if hits.is_empty() {
            return;
        }

        // Map chunk hits to file paths, tracking the best chunk per file
        let mut file_chunks: HashMap<String, (usize, f32)> = HashMap::new(); // file -> (rank, score)
        for (rank, hit) in hits.iter().enumerate() {
            file_chunks
                .entry(hit.file_path.clone())
                .or_insert((rank, hit.score));
        }

        // For each file with chunk hits, find the best symbol in that file
        // and apply the chunk_bm25 RRF boost
        for (file_path, (rank, _score)) in &file_chunks {
            let symbols = match self.storage.graph().get_symbols_by_file(file_path) {
                Ok(syms) => syms,
                Err(_) => continue,
            };

            if symbols.is_empty() {
                continue;
            }

            // Pick the best symbol: prefer classes/functions, use the first one
            let best = symbols
                .iter()
                .min_by_key(|s| match s.kind {
                    SymbolKind::Class | SymbolKind::Struct | SymbolKind::Interface | SymbolKind::Trait => 0,
                    SymbolKind::Function | SymbolKind::Method => 1,
                    _ => 2,
                })
                .unwrap();

            let rrf_score = 1.0 / (*rank as f64 + 1.0 + RRF_K);
            let entry = candidates
                .entry(best.id)
                .or_insert_with(|| ScoredCandidate {
                    symbol_id: best.id,
                    score: 0.0,
                    signals: Vec::new(),
                });
            entry.score += query.chunk_bm25_weight * rrf_score;
            if !entry.signals.contains(&"chunk_bm25".to_string()) {
                entry.signals.push("chunk_bm25".to_string());
            }
        }
    }

    /// Graph expansion: for each current hit, traverse k-hop neighbors and add
    /// them as candidates with the "graph" signal.
    fn expand_graph(
        &self,
        query: &SearchQuery,
        candidates: &mut HashMap<SymbolId, ScoredCandidate>,
    ) {
        let depth = query.effective_graph_depth();
        let seed_ids: Vec<SymbolId> = candidates.keys().copied().collect();

        // Collect all graph-discovered symbol IDs with their best (smallest) depth.
        let mut graph_hits: HashMap<SymbolId, u32> = HashMap::new();

        for seed in &seed_ids {
            let traversal = self.storage.graph().traverse_khop(
                *seed,
                depth,
                50,
                TraversalDirection::Both,
            );

            let hits = match traversal {
                Ok(h) => h,
                Err(_) => continue, // graceful degradation per seed
            };

            for hit in hits {
                let entry = graph_hits.entry(hit.symbol_id).or_insert(hit.depth);
                if hit.depth < *entry {
                    *entry = hit.depth;
                }
            }
        }

        // Remove seeds themselves (they already have signal scores)
        for seed in &seed_ids {
            graph_hits.remove(seed);
        }

        // Apply filters by looking up symbols
        let mut filtered: Vec<(SymbolId, u32)> = Vec::new();
        for (sid, depth_val) in &graph_hits {
            if let Ok(Some(sym)) = self.storage.graph().get_symbol(*sid) {
                let pass_lang = query
                    .language_filter
                    .map_or(true, |l| sym.language == l);
                let pass_path = query.file_path_filter.as_ref().map_or(true, |prefix| {
                    sym.file_path.to_string_lossy().starts_with(prefix.as_str())
                });
                if pass_lang && pass_path {
                    filtered.push((*sid, *depth_val));
                }
            }
        }

        // Sort by depth (closer neighbors rank higher), then by SymbolId for determinism
        filtered.sort_by(|(sid_a, d_a), (sid_b, d_b)| d_a.cmp(d_b).then_with(|| sid_a.cmp(sid_b)));

        tracing::debug!(signal = "graph", count = filtered.len(), "signal collected");

        for (rank, (sid, _)) in filtered.iter().enumerate() {
            let rrf_score = 1.0 / (rank as f64 + 1.0 + RRF_K);
            let entry = candidates
                .entry(*sid)
                .or_insert_with(|| ScoredCandidate {
                    symbol_id: *sid,
                    score: 0.0,
                    signals: Vec::new(),
                });
            entry.score += query.graph_weight * rrf_score;
            if !entry.signals.contains(&"graph".to_string()) {
                entry.signals.push("graph".to_string());
            }
        }
    }

    /// Relation-aware graph expansion: splits traversal into three directed
    /// sub-signals, each filtered by relation type and scored independently.
    ///
    /// - **callees** (`Calls` outgoing): functions called by the seed symbol
    /// - **callers** (`Calls` incoming): functions that call the seed symbol
    /// - **hierarchy** (`Contains` both): parent/child containment relationships
    ///
    /// Each sub-signal is scored via its own RRF pass with a dedicated weight,
    /// producing finer-grained ranking than the original all-relation expansion.
    fn expand_graph_relation_aware(
        &self,
        query: &SearchQuery,
        candidates: &mut HashMap<SymbolId, ScoredCandidate>,
    ) {
        let depth = query.effective_graph_depth();
        let seed_ids: Vec<SymbolId> = candidates.keys().copied().collect();

        // Sub-signal definitions: (relation kinds, direction, signal name, weight)
        let sub_signals: &[(&[RelationKind], TraversalDirection, &str, f64)] = &[
            (
                &[RelationKind::Calls],
                TraversalDirection::Outgoing,
                "graph_callees",
                query.callee_weight,
            ),
            (
                &[RelationKind::Calls],
                TraversalDirection::Incoming,
                "graph_callers",
                query.caller_weight,
            ),
            (
                &[RelationKind::Contains],
                TraversalDirection::Both,
                "graph_hierarchy",
                query.hierarchy_weight,
            ),
        ];

        for &(relation_kinds, direction, signal_name, weight) in sub_signals {
            if weight <= 0.0 {
                continue;
            }

            let mut graph_hits: HashMap<SymbolId, u32> = HashMap::new();

            for seed in &seed_ids {
                let traversal = self.storage.graph().traverse_khop_filtered(
                    *seed,
                    depth,
                    50,
                    direction,
                    Some(relation_kinds),
                );

                let hits = match traversal {
                    Ok(h) => h,
                    Err(_) => continue,
                };

                for hit in hits {
                    let entry = graph_hits.entry(hit.symbol_id).or_insert(hit.depth);
                    if hit.depth < *entry {
                        *entry = hit.depth;
                    }
                }
            }

            // Remove seeds
            for seed in &seed_ids {
                graph_hits.remove(seed);
            }

            // Filter by language and file path
            let mut filtered: Vec<(SymbolId, u32)> = Vec::new();
            for (sid, depth_val) in &graph_hits {
                if let Ok(Some(sym)) = self.storage.graph().get_symbol(*sid) {
                    let pass_lang = query
                        .language_filter
                        .map_or(true, |l| sym.language == l);
                    let pass_path = query.file_path_filter.as_ref().map_or(true, |prefix| {
                        sym.file_path.to_string_lossy().starts_with(prefix.as_str())
                    });
                    if pass_lang && pass_path {
                        filtered.push((*sid, *depth_val));
                    }
                }
            }

            // Sort by depth then SymbolId
            filtered.sort_by(|(sid_a, d_a), (sid_b, d_b)| {
                d_a.cmp(d_b).then_with(|| sid_a.cmp(sid_b))
            });

            tracing::debug!(
                signal = signal_name,
                count = filtered.len(),
                "relation-aware sub-signal collected"
            );

            for (rank, (sid, _)) in filtered.iter().enumerate() {
                let rrf_score = 1.0 / (rank as f64 + 1.0 + RRF_K);
                let entry = candidates
                    .entry(*sid)
                    .or_insert_with(|| ScoredCandidate {
                        symbol_id: *sid,
                        score: 0.0,
                        signals: Vec::new(),
                    });
                entry.score += weight * rrf_score;
                if !entry.signals.contains(&signal_name.to_string()) {
                    entry.signals.push(signal_name.to_string());
                }
            }
        }
    }

    /// Hydrate a ScoredCandidate into a full SearchResult by fetching the symbol
    /// from the graph store.
    fn hydrate_candidate(
        &self,
        candidate: &ScoredCandidate,
    ) -> Result<Option<SearchResult>, RetrievalError> {
        let sym = self.storage.graph().get_symbol(candidate.symbol_id)?;
        let sym = match sym {
            Some(s) => s,
            None => return Ok(None),
        };

        // Truncate body_text to ~50 lines for snippet
        let snippet = sym.body_text.as_ref().map(|text| {
            let lines: Vec<&str> = text.lines().take(50).collect();
            lines.join("\n")
        });

        Ok(Some(SearchResult {
            symbol_id: sym.id,
            name: sym.name.clone(),
            qualified_name: QualifiedName::to_native(&sym.qualified_name, sym.language),
            kind: sym.kind,
            file_path: sym.file_path.to_string_lossy().into_owned(),
            line_range: (sym.line_range.start, sym.line_range.end),
            score: candidate.score,
            match_signals: candidate.signals.clone(),
            related_symbols: Vec::new(),
            snippet,
            chunk_info: None,
        }))
    }

    /// Convert a CodeSymbol to a SearchResult (for related_symbols).
    fn symbol_to_result(sym: &CodeSymbol, signal: &str) -> SearchResult {
        SearchResult {
            symbol_id: sym.id,
            name: sym.name.clone(),
            qualified_name: QualifiedName::to_native(&sym.qualified_name, sym.language),
            kind: sym.kind,
            file_path: sym.file_path.to_string_lossy().into_owned(),
            line_range: (sym.line_range.start, sym.line_range.end),
            score: 0.0,
            match_signals: vec![signal.to_string()],
            related_symbols: Vec::new(),
            snippet: None,
            chunk_info: None,
        }
    }

    /// Attach graph-expanded neighbors as `related_symbols` on each direct hit.
    /// When relation-aware mode is enabled, related symbols carry typed signal
    /// names (`graph_callers`, `graph_callees`, `graph_hierarchy`) instead of
    /// the generic `graph`.
    fn attach_related_symbols(
        &self,
        direct_hit_ids: &HashSet<SymbolId>,
        results: &mut [SearchResult],
        query: &SearchQuery,
    ) -> Result<(), RetrievalError> {
        let depth = query.effective_graph_depth();

        for result in results.iter_mut() {
            if !direct_hit_ids.contains(&result.symbol_id) {
                continue;
            }

            if query.enable_relation_aware_graph {
                let mut related = Vec::new();
                let mut seen = HashSet::new();

                let sub_signals: &[(&[RelationKind], TraversalDirection, &str)] = &[
                    (&[RelationKind::Calls], TraversalDirection::Outgoing, "graph_callees"),
                    (&[RelationKind::Calls], TraversalDirection::Incoming, "graph_callers"),
                    (&[RelationKind::Contains], TraversalDirection::Both, "graph_hierarchy"),
                ];

                for &(kinds, direction, signal_name) in sub_signals {
                    let traversal = self.storage.graph().traverse_khop_filtered(
                        result.symbol_id,
                        depth,
                        50,
                        direction,
                        Some(kinds),
                    );

                    let hits = match traversal {
                        Ok(h) => h,
                        Err(_) => continue,
                    };

                    for hit in hits {
                        if hit.symbol_id == result.symbol_id || !seen.insert(hit.symbol_id) {
                            continue;
                        }
                        if let Ok(Some(sym)) = self.storage.graph().get_symbol(hit.symbol_id) {
                            related.push(Self::symbol_to_result(&sym, signal_name));
                        }
                    }
                }

                result.related_symbols = related;
            } else {
                let traversal = self.storage.graph().traverse_khop(
                    result.symbol_id,
                    depth,
                    50,
                    TraversalDirection::Both,
                );

                let hits = match traversal {
                    Ok(h) => h,
                    Err(_) => continue,
                };

                let mut related = Vec::new();
                for hit in hits {
                    if hit.symbol_id == result.symbol_id {
                        continue;
                    }
                    if let Ok(Some(sym)) = self.storage.graph().get_symbol(hit.symbol_id) {
                        related.push(Self::symbol_to_result(&sym, "graph"));
                    }
                }
                result.related_symbols = related;
            }
        }

        Ok(())
    }

    // --- Function context (call chain traversal) ---

    /// Get structured function context for a symbol: callers, callees, and hierarchy.
    ///
    /// Performs three graph traversals from the given symbol:
    /// - callers: incoming `Calls` edges (who calls this symbol?)
    /// - callees: outgoing `Calls` edges (what does this symbol call?)
    /// - hierarchy: bidirectional `Contains` edges (parent class/module + siblings)
    pub fn get_function_context(
        &self,
        symbol_id: SymbolId,
        max_depth: u32,
        max_fanout: u32,
    ) -> Result<FunctionContext, RetrievalError> {
        let max_depth = max_depth.min(5);
        let max_fanout = max_fanout.min(200);

        // Look up the root symbol
        let root_sym = self.storage.graph().get_symbol(symbol_id)?
            .ok_or_else(|| RetrievalError::QueryFailed {
                reason: format!("symbol not found: {}", symbol_id),
            })?;

        let root_node = Self::symbol_to_chain_node(&root_sym, 0);

        // Callers: who calls this symbol? (incoming Calls edges)
        let callers = self.traverse_to_chain_nodes(
            symbol_id,
            max_depth,
            max_fanout,
            TraversalDirection::Incoming,
            &[RelationKind::Calls],
        )?;

        // Callees: what does this symbol call? (outgoing Calls edges)
        let callees = self.traverse_to_chain_nodes(
            symbol_id,
            max_depth,
            max_fanout,
            TraversalDirection::Outgoing,
            &[RelationKind::Calls],
        )?;

        // Hierarchy: parent/child containment (bidirectional Contains edges)
        let hierarchy_depth = max_depth.min(2);
        let hierarchy = self.traverse_to_chain_nodes(
            symbol_id,
            hierarchy_depth,
            max_fanout,
            TraversalDirection::Both,
            &[RelationKind::Contains],
        )?;

        Ok(FunctionContext {
            symbol: root_node,
            callers,
            callees,
            hierarchy,
        })
    }

    /// BFS traversal + hydration into `CallChainNode` list, sorted by depth then qualified_name.
    fn traverse_to_chain_nodes(
        &self,
        symbol_id: SymbolId,
        max_depth: u32,
        max_fanout: u32,
        direction: TraversalDirection,
        relation_kinds: &[RelationKind],
    ) -> Result<Vec<CallChainNode>, RetrievalError> {
        let hits = self.storage.graph().traverse_khop_filtered(
            symbol_id,
            max_depth,
            max_fanout,
            direction,
            Some(relation_kinds),
        )?;

        let mut nodes = Vec::with_capacity(hits.len());
        for hit in &hits {
            if let Ok(Some(sym)) = self.storage.graph().get_symbol(hit.symbol_id) {
                nodes.push(Self::symbol_to_chain_node(&sym, hit.depth));
            }
        }

        // Sort by depth ascending, then by qualified_name for determinism
        nodes.sort_by(|a, b| {
            a.depth.cmp(&b.depth)
                .then_with(|| a.qualified_name.cmp(&b.qualified_name))
        });

        Ok(nodes)
    }

    /// Convert a `CodeSymbol` into a `CallChainNode` at the given traversal depth.
    fn symbol_to_chain_node(sym: &CodeSymbol, depth: u32) -> CallChainNode {
        let snippet = sym.body_text.as_ref().map(|text| {
            let lines: Vec<&str> = text.lines().take(50).collect();
            lines.join("\n")
        });

        CallChainNode {
            symbol_id: sym.id,
            name: sym.name.clone(),
            qualified_name: QualifiedName::to_native(&sym.qualified_name, sym.language),
            kind: sym.kind,
            file_path: sym.file_path.to_string_lossy().into_owned(),
            line_range: (sym.line_range.start, sym.line_range.end),
            depth,
            signature: sym.signature.clone(),
            doc_comment: sym.doc_comment.clone(),
            snippet,
        }
    }
}

/// Compute the RRF score for a given 0-based rank.
#[inline]
pub fn rrf_score(rank: usize) -> f64 {
    1.0 / (rank as f64 + 1.0 + RRF_K)
}

#[cfg(test)]
mod tests {
    use super::*;
    use oc_core::{CodeRelation, CodeSymbol, Language, RelationKind, SymbolId, SymbolKind};
    use oc_storage::manager::StorageManager;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn make_symbol(
        name: &str,
        qname: &str,
        file: &str,
        byte_start: usize,
        byte_end: usize,
        language: Language,
    ) -> CodeSymbol {
        CodeSymbol {
            id: SymbolId::generate("test-repo", file, qname, byte_start, byte_end),
            name: name.to_string(),
            qualified_name: qname.to_string(),
            kind: SymbolKind::Function,
            language,
            file_path: PathBuf::from(file),
            byte_range: byte_start..byte_end,
            line_range: 0..10,
            signature: Some(format!("def {name}()")),
            doc_comment: None,
            body_hash: 42,
            body_text: None,
        }
    }

    fn make_class_symbol(
        name: &str,
        qname: &str,
        file: &str,
        byte_start: usize,
        byte_end: usize,
        language: Language,
    ) -> CodeSymbol {
        CodeSymbol {
            id: SymbolId::generate("test-repo", file, qname, byte_start, byte_end),
            name: name.to_string(),
            qualified_name: qname.to_string(),
            kind: SymbolKind::Class,
            language,
            file_path: PathBuf::from(file),
            byte_range: byte_start..byte_end,
            line_range: 0..20,
            signature: Some(format!("class {name}")),
            doc_comment: None,
            body_hash: 42,
            body_text: None,
        }
    }

    fn make_relation(
        source: &CodeSymbol,
        target: &CodeSymbol,
        kind: RelationKind,
    ) -> CodeRelation {
        CodeRelation {
            source_id: source.id,
            target_id: target.id,
            kind,
            file_path: source.file_path.clone(),
            line: 5,
            confidence: kind.default_confidence(),
        }
    }

    fn setup_storage(tmp: &TempDir) -> StorageManager {
        StorageManager::open(tmp.path()).unwrap()
    }

    // --- Unit tests for RRF score computation ---

    #[test]
    fn rrf_score_rank_zero() {
        let score = rrf_score(0);
        let expected = 1.0 / (0.0 + 1.0 + 60.0); // 1/61
        assert!((score - expected).abs() < 1e-10);
    }

    #[test]
    fn rrf_score_monotonically_decreasing() {
        for rank in 0..99 {
            assert!(rrf_score(rank) > rrf_score(rank + 1));
        }
    }

    #[test]
    fn rrf_score_always_positive() {
        for rank in 0..1000 {
            assert!(rrf_score(rank) > 0.0);
        }
    }

    #[test]
    fn rrf_multi_signal_higher_than_single() {
        // A symbol appearing in two signals should score higher than one in a single signal
        let single = rrf_score(0);
        let double = rrf_score(0) + rrf_score(0);
        assert!(double > single);
    }

    // --- Unit tests for deduplication ---

    #[test]
    fn deduplication_by_symbol_id() {
        let tmp = TempDir::new().unwrap();
        let mut mgr = setup_storage(&tmp);

        // Create a symbol that will match both BM25 and exact
        let sym = make_symbol("process_data", "process_data", "src/main.py", 0, 100, Language::Python);
        mgr.graph_mut().insert_symbols(&[sym.clone()], 1000).unwrap();
        mgr.fulltext_mut().add_document(&sym, Some("def process_data(): pass")).unwrap();
        mgr.fulltext_mut().commit().unwrap();

        let engine = RetrievalEngine::new(&mgr);
        let query = SearchQuery::new("process_data");
        let results = engine.search(&query).unwrap();

        // Should appear exactly once despite matching multiple signals
        let count = results.iter().filter(|r| r.symbol_id == sym.id).count();
        assert_eq!(count, 1);

        // Should have both signals
        let result = results.iter().find(|r| r.symbol_id == sym.id).unwrap();
        assert!(result.match_signals.contains(&"bm25".to_string()));
        assert!(result.match_signals.contains(&"exact".to_string()));
    }

    // --- Unit tests for graceful degradation ---

    #[test]
    fn empty_results_on_no_data() {
        let tmp = TempDir::new().unwrap();
        let mgr = setup_storage(&tmp);

        let engine = RetrievalEngine::new(&mgr);
        let query = SearchQuery::new("nonexistent_symbol");
        let results = engine.search(&query).unwrap();
        assert!(results.is_empty());
    }

    // --- Unit tests for language filtering ---

    #[test]
    fn language_filter_restricts_results() {
        let tmp = TempDir::new().unwrap();
        let mut mgr = setup_storage(&tmp);

        let py_sym = make_symbol("process", "process", "src/main.py", 0, 100, Language::Python);
        let rs_sym = make_symbol("process", "process", "src/main.rs", 0, 100, Language::Rust);

        mgr.graph_mut()
            .insert_symbols(&[py_sym.clone(), rs_sym.clone()], 1000)
            .unwrap();
        mgr.fulltext_mut().add_document(&py_sym, Some("def process(): pass")).unwrap();
        mgr.fulltext_mut().add_document(&rs_sym, Some("fn process() {}")).unwrap();
        mgr.fulltext_mut().commit().unwrap();

        let engine = RetrievalEngine::new(&mgr);

        // Filter to Python only
        let mut query = SearchQuery::new("process");
        query.language_filter = Some(Language::Python);
        query.enable_graph_expansion = false;
        let results = engine.search(&query).unwrap();

        assert!(!results.is_empty());
        for r in &results {
            // All results should be from Python files
            assert!(r.file_path.ends_with(".py"), "Expected .py file, got {}", r.file_path);
        }
    }

    // --- Unit tests for file path filtering ---

    #[test]
    fn file_path_prefix_filter() {
        let tmp = TempDir::new().unwrap();
        let mut mgr = setup_storage(&tmp);

        let sym_a = make_symbol("handler", "handler", "src/api/handler.py", 0, 100, Language::Python);
        let sym_b = make_symbol("handler", "handler", "lib/handler.py", 0, 100, Language::Python);

        mgr.graph_mut()
            .insert_symbols(&[sym_a.clone(), sym_b.clone()], 1000)
            .unwrap();
        mgr.fulltext_mut().add_document(&sym_a, Some("def handler(): pass")).unwrap();
        mgr.fulltext_mut().add_document(&sym_b, Some("def handler(): pass")).unwrap();
        mgr.fulltext_mut().commit().unwrap();

        let engine = RetrievalEngine::new(&mgr);

        let mut query = SearchQuery::new("handler");
        query.file_path_filter = Some("src/".to_string());
        query.enable_graph_expansion = false;
        let results = engine.search(&query).unwrap();

        assert!(!results.is_empty());
        for r in &results {
            assert!(r.file_path.starts_with("src/"), "Expected src/ prefix, got {}", r.file_path);
        }
    }

    // --- Unit tests for query defaults and limits ---

    #[test]
    fn search_query_defaults() {
        let q = SearchQuery::new("test");
        assert_eq!(q.text, "test");
        assert_eq!(q.limit, 10);
        assert!(q.language_filter.is_none());
        assert!(q.file_path_filter.is_none());
        assert!(q.enable_graph_expansion);
        assert_eq!(q.graph_depth, 2);
        assert_eq!(q.bm25_pool_size, 100);
        assert_eq!(q.exact_match_pool_size, 50);
        assert!(q.query_vector.is_none());
        assert_eq!(q.vector_pool_size, 100);
        assert!(q.enable_chunk_search);
        assert_eq!(q.chunk_bm25_pool_size, 100);
        assert!((q.bm25_weight - 1.0).abs() < f64::EPSILON);
        assert!((q.vector_weight - 1.0).abs() < f64::EPSILON);
        assert!((q.exact_weight - 1.0).abs() < f64::EPSILON);
        assert!((q.chunk_bm25_weight - 1.0).abs() < f64::EPSILON);
        assert!((q.graph_weight - 1.0).abs() < f64::EPSILON);
        assert!(q.bm25_text.is_none());
        assert!(q.exact_queries.is_empty());
        assert!(q.enable_relation_aware_graph);
        assert!((q.callee_weight - 1.5).abs() < f64::EPSILON);
        assert!((q.caller_weight - 1.2).abs() < f64::EPSILON);
        assert!((q.hierarchy_weight - 0.8).abs() < f64::EPSILON);
    }

    #[test]
    fn limit_capped_at_200() {
        let mut q = SearchQuery::new("test");
        q.limit = 500;
        assert_eq!(q.effective_limit(), 200);
    }

    #[test]
    fn graph_depth_capped_at_5() {
        let mut q = SearchQuery::new("test");
        q.graph_depth = 10;
        assert_eq!(q.effective_graph_depth(), 5);
    }

    // --- Integration test: multi-signal search ---

    #[test]
    fn integration_multi_signal_search() {
        let tmp = TempDir::new().unwrap();
        let mut mgr = setup_storage(&tmp);

        // Create a small "project"
        let sym_a = make_symbol("process_data", "app.process_data", "src/app.py", 0, 100, Language::Python);
        let sym_b = make_symbol("validate_input", "app.validate_input", "src/app.py", 200, 350, Language::Python);
        let sym_c = make_symbol("format_output", "utils.format_output", "src/utils.py", 0, 80, Language::Python);

        // A calls B, A calls C
        let rel_ab = CodeRelation {
            source_id: sym_a.id,
            target_id: sym_b.id,
            kind: RelationKind::Calls,
            file_path: PathBuf::from("src/app.py"),
            line: 5,
            confidence: RelationKind::Calls.default_confidence(),
        };
        let rel_ac = CodeRelation {
            source_id: sym_a.id,
            target_id: sym_c.id,
            kind: RelationKind::Calls,
            file_path: PathBuf::from("src/app.py"),
            line: 6,
            confidence: RelationKind::Calls.default_confidence(),
        };

        mgr.graph_mut()
            .insert_symbols(&[sym_a.clone(), sym_b.clone(), sym_c.clone()], 1000)
            .unwrap();
        mgr.graph_mut()
            .insert_relations(&[rel_ab, rel_ac], 1000)
            .unwrap();

        // Index into fulltext
        mgr.fulltext_mut()
            .add_document(&sym_a, Some("def process_data(): validate_input(); format_output()"))
            .unwrap();
        mgr.fulltext_mut()
            .add_document(&sym_b, Some("def validate_input(): pass"))
            .unwrap();
        mgr.fulltext_mut()
            .add_document(&sym_c, Some("def format_output(): pass"))
            .unwrap();
        mgr.fulltext_mut().commit().unwrap();

        let engine = RetrievalEngine::new(&mgr);

        // Search for "process_data" — should find it via BM25 + exact match
        let query = SearchQuery::new("process_data");
        let results = engine.search(&query).unwrap();

        assert!(!results.is_empty(), "Expected at least one result");

        // process_data should be the top hit (both BM25 and exact match)
        assert_eq!(results[0].symbol_id, sym_a.id);
        assert_eq!(results[0].name, "process_data");
        assert!(results[0].match_signals.contains(&"bm25".to_string()));
        assert!(results[0].match_signals.contains(&"exact".to_string()));

        // Graph-expanded neighbors should appear in results
        let result_ids: Vec<SymbolId> = results.iter().map(|r| r.symbol_id).collect();
        assert!(result_ids.contains(&sym_b.id), "validate_input should appear via graph expansion");
        assert!(result_ids.contains(&sym_c.id), "format_output should appear via graph expansion");

        // Graph-expanded results should have a graph signal (relation-aware by default)
        let graph_result = results.iter().find(|r| r.symbol_id == sym_b.id).unwrap();
        let has_graph_signal = graph_result.match_signals.iter().any(|s| s.starts_with("graph"));
        assert!(has_graph_signal, "expected a graph signal, got: {:?}", graph_result.match_signals);
    }

    // --- Integration test: search with graph expansion disabled ---

    #[test]
    fn integration_no_graph_expansion() {
        let tmp = TempDir::new().unwrap();
        let mut mgr = setup_storage(&tmp);

        let sym_a = make_symbol("main_func", "main_func", "src/main.py", 0, 100, Language::Python);
        let sym_b = make_symbol("helper_func", "helper_func", "src/main.py", 200, 300, Language::Python);

        let rel = CodeRelation {
            source_id: sym_a.id,
            target_id: sym_b.id,
            kind: RelationKind::Calls,
            file_path: PathBuf::from("src/main.py"),
            line: 5,
            confidence: RelationKind::Calls.default_confidence(),
        };

        mgr.graph_mut().insert_symbols(&[sym_a.clone(), sym_b.clone()], 1000).unwrap();
        mgr.graph_mut().insert_relations(&[rel], 1000).unwrap();
        mgr.fulltext_mut().add_document(&sym_a, Some("def main_func(): pass")).unwrap();
        mgr.fulltext_mut().add_document(&sym_b, Some("def helper_func(): pass")).unwrap();
        mgr.fulltext_mut().commit().unwrap();

        let engine = RetrievalEngine::new(&mgr);

        // With graph expansion disabled, helper_func should not appear
        let mut query = SearchQuery::new("main_func");
        query.enable_graph_expansion = false;
        let results = engine.search(&query).unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].symbol_id, sym_a.id);
        assert!(!results[0].match_signals.contains(&"graph".to_string()));
    }

    // --- Score properties ---

    #[test]
    fn score_determinism() {
        let tmp = TempDir::new().unwrap();
        let mut mgr = setup_storage(&tmp);

        let sym = make_symbol("deterministic", "deterministic", "src/main.py", 0, 100, Language::Python);
        mgr.graph_mut().insert_symbols(&[sym.clone()], 1000).unwrap();
        mgr.fulltext_mut().add_document(&sym, Some("def deterministic(): pass")).unwrap();
        mgr.fulltext_mut().commit().unwrap();

        let engine = RetrievalEngine::new(&mgr);

        let q = SearchQuery::new("deterministic");
        let r1 = engine.search(&q).unwrap();
        let r2 = engine.search(&q).unwrap();

        assert_eq!(r1.len(), r2.len());
        for (a, b) in r1.iter().zip(r2.iter()) {
            assert_eq!(a.symbol_id, b.symbol_id);
            assert!((a.score - b.score).abs() < 1e-10);
        }
    }

    // --- Vector signal tests ---

    fn setup_storage_vec(tmp: &TempDir) -> StorageManager {
        StorageManager::open_with_dimension(tmp.path(), 4).unwrap()
    }

    #[test]
    fn vector_only_search() {
        let tmp = TempDir::new().unwrap();
        let mut mgr = setup_storage_vec(&tmp);

        let sym = make_symbol("embed_func", "embed_func", "src/embed.py", 0, 100, Language::Python);
        mgr.graph_mut().insert_symbols(&[sym.clone()], 1000).unwrap();
        mgr.vector_mut().add_vector(sym.id, &[1.0, 0.0, 0.0, 0.0]).unwrap();

        let engine = RetrievalEngine::new(&mgr);

        // Query with vector only, no text that would match BM25/exact
        let mut query = SearchQuery::new("zzz_no_match_zzz");
        query.query_vector = Some(vec![1.0, 0.0, 0.0, 0.0]);
        query.enable_graph_expansion = false;
        let results = engine.search(&query).unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].symbol_id, sym.id);
        assert!(results[0].match_signals.contains(&"vector".to_string()));
        assert!(!results[0].match_signals.contains(&"bm25".to_string()));
        assert!(!results[0].match_signals.contains(&"exact".to_string()));
    }

    #[test]
    fn multi_signal_with_vector() {
        let tmp = TempDir::new().unwrap();
        let mut mgr = setup_storage_vec(&tmp);

        let sym = make_symbol("process_data", "process_data", "src/main.py", 0, 100, Language::Python);
        mgr.graph_mut().insert_symbols(&[sym.clone()], 1000).unwrap();
        mgr.fulltext_mut().add_document(&sym, Some("def process_data(): pass")).unwrap();
        mgr.fulltext_mut().commit().unwrap();
        mgr.vector_mut().add_vector(sym.id, &[1.0, 0.0, 0.0, 0.0]).unwrap();

        let engine = RetrievalEngine::new(&mgr);

        let mut query = SearchQuery::new("process_data");
        query.query_vector = Some(vec![1.0, 0.0, 0.0, 0.0]);
        query.enable_graph_expansion = false;
        let results = engine.search(&query).unwrap();

        assert_eq!(results.len(), 1);
        let r = &results[0];
        assert_eq!(r.symbol_id, sym.id);
        // Should have all three signals
        assert!(r.match_signals.contains(&"bm25".to_string()));
        assert!(r.match_signals.contains(&"exact".to_string()));
        assert!(r.match_signals.contains(&"vector".to_string()));

        // Score should be higher than any single signal alone
        let single_rrf = rrf_score(0);
        assert!(r.score > single_rrf, "Multi-signal score {} should exceed single signal {}", r.score, single_rrf);
    }

    #[test]
    fn vector_graceful_degradation_empty_store() {
        let tmp = TempDir::new().unwrap();
        let mut mgr = setup_storage_vec(&tmp);

        let sym = make_symbol("some_func", "some_func", "src/main.py", 0, 100, Language::Python);
        mgr.graph_mut().insert_symbols(&[sym.clone()], 1000).unwrap();
        mgr.fulltext_mut().add_document(&sym, Some("def some_func(): pass")).unwrap();
        mgr.fulltext_mut().commit().unwrap();
        // Intentionally NOT adding any vectors

        let engine = RetrievalEngine::new(&mgr);

        let mut query = SearchQuery::new("some_func");
        query.query_vector = Some(vec![1.0, 0.0, 0.0, 0.0]);
        query.enable_graph_expansion = false;
        let results = engine.search(&query).unwrap();

        // Should still get BM25/exact results, no panic from empty vector store
        assert!(!results.is_empty());
        assert!(!results[0].match_signals.contains(&"vector".to_string()));
    }

    // --- Tests for bm25_text and exact_queries fields ---

    #[test]
    fn effective_bm25_text_fallback() {
        let q = SearchQuery::new("original text");
        assert_eq!(q.effective_bm25_text(), "original text");
    }

    #[test]
    fn effective_bm25_text_override() {
        let mut q = SearchQuery::new("original text");
        q.bm25_text = Some("override text".to_string());
        assert_eq!(q.effective_bm25_text(), "override text");
    }

    #[test]
    fn bm25_text_override_in_search() {
        let tmp = TempDir::new().unwrap();
        let mut mgr = setup_storage(&tmp);

        // sym_a has "alpha" in its body, sym_b has "beta"
        let sym_a = make_symbol("alpha_func", "alpha_func", "src/alpha.py", 0, 100, Language::Python);
        let sym_b = make_symbol("beta_func", "beta_func", "src/beta.py", 0, 100, Language::Python);

        mgr.graph_mut().insert_symbols(&[sym_a.clone(), sym_b.clone()], 1000).unwrap();
        mgr.fulltext_mut().add_document(&sym_a, Some("def alpha_func(): alpha alpha alpha")).unwrap();
        mgr.fulltext_mut().add_document(&sym_b, Some("def beta_func(): beta beta beta")).unwrap();
        mgr.fulltext_mut().commit().unwrap();

        let engine = RetrievalEngine::new(&mgr);

        // text is "zzz_no_match" but bm25_text overrides to "alpha"
        let mut query = SearchQuery::new("zzz_no_match");
        query.bm25_text = Some("alpha".to_string());
        query.enable_graph_expansion = false;
        let results = engine.search(&query).unwrap();

        // Should find alpha_func via BM25 using the override text
        assert!(!results.is_empty());
        let alpha_hit = results.iter().find(|r| r.name == "alpha_func");
        assert!(alpha_hit.is_some(), "alpha_func should be found via bm25_text override");
        assert!(alpha_hit.unwrap().match_signals.contains(&"bm25".to_string()));
    }

    #[test]
    fn exact_match_with_multiple_queries() {
        let tmp = TempDir::new().unwrap();
        let mut mgr = setup_storage(&tmp);

        let sym_a = make_symbol("FooClass", "module.FooClass", "src/foo.py", 0, 100, Language::Python);
        let sym_b = make_symbol("bar_func", "module.bar_func", "src/bar.py", 0, 100, Language::Python);
        let sym_c = make_symbol("unrelated", "module.unrelated", "src/other.py", 0, 100, Language::Python);

        mgr.graph_mut().insert_symbols(&[sym_a.clone(), sym_b.clone(), sym_c.clone()], 1000).unwrap();
        // No fulltext docs — we only care about the exact match signal
        mgr.fulltext_mut().commit().unwrap();

        let engine = RetrievalEngine::new(&mgr);

        // exact_queries targets two specific identifiers
        let mut query = SearchQuery::new("some long problem statement that won't match anything");
        query.exact_queries = vec!["FooClass".to_string(), "bar_func".to_string()];
        query.enable_graph_expansion = false;
        let results = engine.search(&query).unwrap();

        let ids: Vec<&str> = results.iter().map(|r| r.name.as_str()).collect();
        assert!(ids.contains(&"FooClass"), "FooClass should be found via exact_queries");
        assert!(ids.contains(&"bar_func"), "bar_func should be found via exact_queries");
        assert!(!ids.contains(&"unrelated"), "unrelated should not appear");

        // Both should have the "exact" signal
        for r in &results {
            if r.name == "FooClass" || r.name == "bar_func" {
                assert!(r.match_signals.contains(&"exact".to_string()));
            }
        }
    }

    #[test]
    fn exact_queries_empty_falls_back_to_text() {
        let tmp = TempDir::new().unwrap();
        let mut mgr = setup_storage(&tmp);

        let sym = make_symbol("target_fn", "target_fn", "src/main.py", 0, 100, Language::Python);
        mgr.graph_mut().insert_symbols(&[sym.clone()], 1000).unwrap();
        mgr.fulltext_mut().commit().unwrap();

        let engine = RetrievalEngine::new(&mgr);

        // exact_queries is empty, so text is used for exact match (backward compat)
        let mut query = SearchQuery::new("target_fn");
        query.enable_graph_expansion = false;
        let results = engine.search(&query).unwrap();

        assert!(!results.is_empty());
        assert_eq!(results[0].name, "target_fn");
        assert!(results[0].match_signals.contains(&"exact".to_string()));
    }

    // --- Relation-aware graph expansion tests ---

    /// Build a project graph: A calls B, A calls C, D contains A.
    /// Verify that relation-aware expansion produces typed signals.
    #[test]
    fn relation_aware_graph_separates_callers_callees() {
        let tmp = TempDir::new().unwrap();
        let mut mgr = setup_storage(&tmp);

        let func_a = make_symbol("func_a", "mod.func_a", "src/mod.py", 0, 100, Language::Python);
        let func_b = make_symbol("func_b", "mod.func_b", "src/mod.py", 200, 300, Language::Python);
        let func_c = make_symbol("func_c", "mod.func_c", "src/mod.py", 400, 500, Language::Python);

        // A calls B, A calls C
        let rel_ab = make_relation(&func_a, &func_b, RelationKind::Calls);
        let rel_ac = make_relation(&func_a, &func_c, RelationKind::Calls);

        mgr.graph_mut()
            .insert_symbols(&[func_a.clone(), func_b.clone(), func_c.clone()], 1000)
            .unwrap();
        mgr.graph_mut()
            .insert_relations(&[rel_ab, rel_ac], 1000)
            .unwrap();

        mgr.fulltext_mut()
            .add_document(&func_a, Some("def func_a(): func_b(); func_c()"))
            .unwrap();
        mgr.fulltext_mut()
            .add_document(&func_b, Some("def func_b(): pass"))
            .unwrap();
        mgr.fulltext_mut()
            .add_document(&func_c, Some("def func_c(): pass"))
            .unwrap();
        mgr.fulltext_mut().commit().unwrap();

        let engine = RetrievalEngine::new(&mgr);

        // Search for func_a with relation-aware graph enabled
        let mut query = SearchQuery::new("func_a");
        query.enable_relation_aware_graph = true;
        let results = engine.search(&query).unwrap();

        // func_a should be top hit
        assert!(!results.is_empty());
        assert_eq!(results[0].name, "func_a");

        // func_b and func_c should appear as graph_callees (A calls B, A calls C)
        let result_b = results.iter().find(|r| r.name == "func_b");
        assert!(result_b.is_some(), "func_b should appear via graph_callees");
        assert!(
            result_b.unwrap().match_signals.contains(&"graph_callees".to_string()),
            "func_b signals: {:?}",
            result_b.unwrap().match_signals
        );

        let result_c = results.iter().find(|r| r.name == "func_c");
        assert!(result_c.is_some(), "func_c should appear via graph_callees");
        assert!(
            result_c.unwrap().match_signals.contains(&"graph_callees".to_string()),
            "func_c signals: {:?}",
            result_c.unwrap().match_signals
        );
    }

    #[test]
    fn relation_aware_callers_signal() {
        let tmp = TempDir::new().unwrap();
        let mut mgr = setup_storage(&tmp);

        let caller = make_symbol("caller", "mod.caller", "src/mod.py", 0, 100, Language::Python);
        let target = make_symbol("xtarget", "mod.xtarget", "src/mod.py", 200, 300, Language::Python);

        // caller calls target
        let rel = make_relation(&caller, &target, RelationKind::Calls);

        mgr.graph_mut()
            .insert_symbols(&[caller.clone(), target.clone()], 1000)
            .unwrap();
        mgr.graph_mut().insert_relations(&[rel], 1000).unwrap();

        // Use body text that won't match "xtarget" for the caller
        mgr.fulltext_mut()
            .add_document(&target, Some("def xtarget(): pass"))
            .unwrap();
        mgr.fulltext_mut()
            .add_document(&caller, Some("def caller(): invoke_something()"))
            .unwrap();
        mgr.fulltext_mut().commit().unwrap();

        let engine = RetrievalEngine::new(&mgr);

        // Search for xtarget — caller should appear as graph_callers
        let mut query = SearchQuery::new("xtarget");
        query.enable_relation_aware_graph = true;
        let results = engine.search(&query).unwrap();

        let caller_result = results.iter().find(|r| r.name == "caller");
        assert!(
            caller_result.is_some(),
            "caller should appear via graph_callers"
        );
        assert!(
            caller_result.unwrap().match_signals.contains(&"graph_callers".to_string()),
            "caller signals: {:?}",
            caller_result.unwrap().match_signals
        );
    }

    #[test]
    fn relation_aware_hierarchy_signal() {
        let tmp = TempDir::new().unwrap();
        let mut mgr = setup_storage(&tmp);

        let class_sym = make_class_symbol("MyClass", "mod.MyClass", "src/mod.py", 0, 500, Language::Python);
        let method = make_symbol("my_method", "mod.MyClass.my_method", "src/mod.py", 100, 200, Language::Python);

        // MyClass contains my_method
        let rel = make_relation(&class_sym, &method, RelationKind::Contains);

        mgr.graph_mut()
            .insert_symbols(&[class_sym.clone(), method.clone()], 1000)
            .unwrap();
        mgr.graph_mut().insert_relations(&[rel], 1000).unwrap();

        mgr.fulltext_mut()
            .add_document(&method, Some("def my_method(self): pass"))
            .unwrap();
        mgr.fulltext_mut()
            .add_document(&class_sym, Some("class MyClass: pass"))
            .unwrap();
        mgr.fulltext_mut().commit().unwrap();

        let engine = RetrievalEngine::new(&mgr);

        // Search for my_method — MyClass should appear via graph_hierarchy
        let mut query = SearchQuery::new("my_method");
        query.enable_relation_aware_graph = true;
        let results = engine.search(&query).unwrap();

        let class_result = results.iter().find(|r| r.name == "MyClass");
        assert!(
            class_result.is_some(),
            "MyClass should appear via graph_hierarchy"
        );
        assert!(
            class_result.unwrap().match_signals.contains(&"graph_hierarchy".to_string()),
            "MyClass signals: {:?}",
            class_result.unwrap().match_signals
        );
    }

    #[test]
    fn relation_aware_does_not_mix_calls_and_contains() {
        let tmp = TempDir::new().unwrap();
        let mut mgr = setup_storage(&tmp);

        let func_a = make_symbol("func_a", "mod.func_a", "src/mod.py", 0, 100, Language::Python);
        let func_b = make_symbol("func_b", "mod.func_b", "src/mod.py", 200, 300, Language::Python);
        let class_c = make_class_symbol("ClassC", "mod.ClassC", "src/mod.py", 400, 600, Language::Python);

        // A calls B (Calls), C contains A (Contains)
        let rel_call = make_relation(&func_a, &func_b, RelationKind::Calls);
        let rel_contains = make_relation(&class_c, &func_a, RelationKind::Contains);

        mgr.graph_mut()
            .insert_symbols(&[func_a.clone(), func_b.clone(), class_c.clone()], 1000)
            .unwrap();
        mgr.graph_mut()
            .insert_relations(&[rel_call, rel_contains], 1000)
            .unwrap();

        mgr.fulltext_mut()
            .add_document(&func_a, Some("def func_a(): func_b()"))
            .unwrap();
        mgr.fulltext_mut()
            .add_document(&func_b, Some("def func_b(): pass"))
            .unwrap();
        mgr.fulltext_mut()
            .add_document(&class_c, Some("class ClassC: pass"))
            .unwrap();
        mgr.fulltext_mut().commit().unwrap();

        let engine = RetrievalEngine::new(&mgr);

        // Search for func_a — B should be graph_callees, C should be graph_hierarchy
        let mut query = SearchQuery::new("func_a");
        query.enable_relation_aware_graph = true;
        let results = engine.search(&query).unwrap();

        let result_b = results.iter().find(|r| r.name == "func_b");
        assert!(result_b.is_some());
        let b_signals = &result_b.unwrap().match_signals;
        assert!(b_signals.contains(&"graph_callees".to_string()), "func_b signals: {:?}", b_signals);
        assert!(!b_signals.contains(&"graph_hierarchy".to_string()), "func_b should NOT have hierarchy signal");

        let result_c = results.iter().find(|r| r.name == "ClassC");
        assert!(result_c.is_some());
        let c_signals = &result_c.unwrap().match_signals;
        assert!(c_signals.contains(&"graph_hierarchy".to_string()), "ClassC signals: {:?}", c_signals);
        assert!(!c_signals.contains(&"graph_callees".to_string()), "ClassC should NOT have callees signal");
    }

    #[test]
    fn callee_weight_higher_ranks_callees_above_callers() {
        let tmp = TempDir::new().unwrap();
        let mut mgr = setup_storage(&tmp);

        let target = make_symbol("target", "mod.target", "src/mod.py", 0, 100, Language::Python);
        let callee = make_symbol("callee_fn", "mod.callee_fn", "src/mod.py", 200, 300, Language::Python);
        let caller = make_symbol("caller_fn", "mod.caller_fn", "src/mod.py", 400, 500, Language::Python);

        // target calls callee, caller calls target
        let rel_out = make_relation(&target, &callee, RelationKind::Calls);
        let rel_in = make_relation(&caller, &target, RelationKind::Calls);

        mgr.graph_mut()
            .insert_symbols(&[target.clone(), callee.clone(), caller.clone()], 1000)
            .unwrap();
        mgr.graph_mut()
            .insert_relations(&[rel_out, rel_in], 1000)
            .unwrap();

        mgr.fulltext_mut()
            .add_document(&target, Some("def target(): callee_fn()"))
            .unwrap();
        mgr.fulltext_mut()
            .add_document(&callee, Some("def callee_fn(): pass"))
            .unwrap();
        mgr.fulltext_mut()
            .add_document(&caller, Some("def caller_fn(): target()"))
            .unwrap();
        mgr.fulltext_mut().commit().unwrap();

        let engine = RetrievalEngine::new(&mgr);

        // With callee_weight > caller_weight, callee should score higher
        let mut query = SearchQuery::new("target");
        query.enable_relation_aware_graph = true;
        query.callee_weight = 3.0;
        query.caller_weight = 1.0;
        let results = engine.search(&query).unwrap();

        let callee_result = results.iter().find(|r| r.name == "callee_fn").unwrap();
        let caller_result = results.iter().find(|r| r.name == "caller_fn").unwrap();

        assert!(
            callee_result.score > caller_result.score,
            "callee (score={}) should rank above caller (score={}) when callee_weight > caller_weight",
            callee_result.score, caller_result.score
        );
    }

    #[test]
    fn fallback_to_legacy_graph_when_disabled() {
        let tmp = TempDir::new().unwrap();
        let mut mgr = setup_storage(&tmp);

        let func_a = make_symbol("func_a", "mod.func_a", "src/mod.py", 0, 100, Language::Python);
        let func_b = make_symbol("func_b", "mod.func_b", "src/mod.py", 200, 300, Language::Python);

        let rel = make_relation(&func_a, &func_b, RelationKind::Calls);

        mgr.graph_mut()
            .insert_symbols(&[func_a.clone(), func_b.clone()], 1000)
            .unwrap();
        mgr.graph_mut().insert_relations(&[rel], 1000).unwrap();

        mgr.fulltext_mut()
            .add_document(&func_a, Some("def func_a(): func_b()"))
            .unwrap();
        mgr.fulltext_mut()
            .add_document(&func_b, Some("def func_b(): pass"))
            .unwrap();
        mgr.fulltext_mut().commit().unwrap();

        let engine = RetrievalEngine::new(&mgr);

        // With relation-aware disabled, should use the old "graph" signal name
        let mut query = SearchQuery::new("func_a");
        query.enable_relation_aware_graph = false;
        let results = engine.search(&query).unwrap();

        let result_b = results.iter().find(|r| r.name == "func_b");
        assert!(result_b.is_some(), "func_b should still appear via legacy graph expansion");
        assert!(
            result_b.unwrap().match_signals.contains(&"graph".to_string()),
            "should use generic 'graph' signal, got: {:?}",
            result_b.unwrap().match_signals
        );
        // Should NOT have typed signal names
        assert!(
            !result_b.unwrap().match_signals.contains(&"graph_callees".to_string()),
            "legacy mode should not produce typed graph signals"
        );
    }

    #[test]
    fn relation_aware_related_symbols_carry_typed_signals() {
        let tmp = TempDir::new().unwrap();
        let mut mgr = setup_storage(&tmp);

        let func_a = make_symbol("func_a", "mod.func_a", "src/mod.py", 0, 100, Language::Python);
        let func_b = make_symbol("func_b", "mod.func_b", "src/mod.py", 200, 300, Language::Python);
        let class_c = make_class_symbol("ClassC", "mod.ClassC", "src/mod.py", 400, 600, Language::Python);

        let rel_call = make_relation(&func_a, &func_b, RelationKind::Calls);
        let rel_contains = make_relation(&class_c, &func_a, RelationKind::Contains);

        mgr.graph_mut()
            .insert_symbols(&[func_a.clone(), func_b.clone(), class_c.clone()], 1000)
            .unwrap();
        mgr.graph_mut()
            .insert_relations(&[rel_call, rel_contains], 1000)
            .unwrap();

        mgr.fulltext_mut()
            .add_document(&func_a, Some("def func_a(): func_b()"))
            .unwrap();
        mgr.fulltext_mut().commit().unwrap();

        let engine = RetrievalEngine::new(&mgr);

        let mut query = SearchQuery::new("func_a");
        query.enable_relation_aware_graph = true;
        let results = engine.search(&query).unwrap();

        // Check related_symbols on func_a carry typed signal names
        let func_a_result = results.iter().find(|r| r.name == "func_a").unwrap();
        let related_names: Vec<&str> = func_a_result
            .related_symbols
            .iter()
            .map(|r| r.name.as_str())
            .collect();

        assert!(
            related_names.contains(&"func_b"),
            "func_b should be in related_symbols, got: {:?}",
            related_names
        );

        let related_b = func_a_result
            .related_symbols
            .iter()
            .find(|r| r.name == "func_b")
            .unwrap();
        assert!(
            related_b.match_signals.contains(&"graph_callees".to_string()),
            "related func_b should have graph_callees signal, got: {:?}",
            related_b.match_signals
        );
    }

    // --- Function context (call chain traversal) tests ---

    #[test]
    fn function_context_basic() {
        let tmp = TempDir::new().unwrap();
        let mut mgr = setup_storage(&tmp);

        // A -> B -> C call chain
        let sym_a = make_symbol("func_a", "mod.func_a", "src/mod.py", 0, 100, Language::Python);
        let sym_b = make_symbol("func_b", "mod.func_b", "src/mod.py", 200, 300, Language::Python);
        let sym_c = make_symbol("func_c", "mod.func_c", "src/mod.py", 400, 500, Language::Python);

        let rel_ab = make_relation(&sym_a, &sym_b, RelationKind::Calls);
        let rel_bc = make_relation(&sym_b, &sym_c, RelationKind::Calls);

        mgr.graph_mut()
            .insert_symbols(&[sym_a.clone(), sym_b.clone(), sym_c.clone()], 1000)
            .unwrap();
        mgr.graph_mut()
            .insert_relations(&[rel_ab, rel_bc], 1000)
            .unwrap();

        let engine = RetrievalEngine::new(&mgr);
        let ctx = engine.get_function_context(sym_b.id, 3, 50).unwrap();

        // Root symbol should be B
        assert_eq!(ctx.symbol.symbol_id, sym_b.id);
        assert_eq!(ctx.symbol.name, "func_b");
        assert_eq!(ctx.symbol.depth, 0);

        // Callers of B = [A]
        assert_eq!(ctx.callers.len(), 1);
        assert_eq!(ctx.callers[0].symbol_id, sym_a.id);
        assert_eq!(ctx.callers[0].depth, 1);

        // Callees of B = [C]
        assert_eq!(ctx.callees.len(), 1);
        assert_eq!(ctx.callees[0].symbol_id, sym_c.id);
        assert_eq!(ctx.callees[0].depth, 1);
    }

    #[test]
    fn function_context_multi_hop_callers() {
        let tmp = TempDir::new().unwrap();
        let mut mgr = setup_storage(&tmp);

        // X -> Y -> Z call chain; query Z, callers should be [Y@1, X@2]
        let sym_x = make_symbol("func_x", "mod.func_x", "src/mod.py", 0, 100, Language::Python);
        let sym_y = make_symbol("func_y", "mod.func_y", "src/mod.py", 200, 300, Language::Python);
        let sym_z = make_symbol("func_z", "mod.func_z", "src/mod.py", 400, 500, Language::Python);

        let rel_xy = make_relation(&sym_x, &sym_y, RelationKind::Calls);
        let rel_yz = make_relation(&sym_y, &sym_z, RelationKind::Calls);

        mgr.graph_mut()
            .insert_symbols(&[sym_x.clone(), sym_y.clone(), sym_z.clone()], 1000)
            .unwrap();
        mgr.graph_mut()
            .insert_relations(&[rel_xy, rel_yz], 1000)
            .unwrap();

        let engine = RetrievalEngine::new(&mgr);
        let ctx = engine.get_function_context(sym_z.id, 3, 50).unwrap();

        // Callers of Z: Y at depth 1, X at depth 2
        assert_eq!(ctx.callers.len(), 2);
        assert_eq!(ctx.callers[0].depth, 1); // sorted by depth
        assert_eq!(ctx.callers[0].symbol_id, sym_y.id);
        assert_eq!(ctx.callers[1].depth, 2);
        assert_eq!(ctx.callers[1].symbol_id, sym_x.id);

        // Z has no callees
        assert!(ctx.callees.is_empty());
    }

    #[test]
    fn function_context_symbol_not_found() {
        let tmp = TempDir::new().unwrap();
        let mgr = setup_storage(&tmp);

        let engine = RetrievalEngine::new(&mgr);
        let fake_id = SymbolId(999999);
        let result = engine.get_function_context(fake_id, 3, 50);

        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(err_msg.contains("symbol not found"), "error: {}", err_msg);
    }

    #[test]
    fn function_context_with_hierarchy() {
        let tmp = TempDir::new().unwrap();
        let mut mgr = setup_storage(&tmp);

        let class_sym = make_class_symbol("MyClass", "mod.MyClass", "src/mod.py", 0, 500, Language::Python);
        let method_a = make_symbol("method_a", "mod.MyClass.method_a", "src/mod.py", 100, 200, Language::Python);
        let method_b = make_symbol("method_b", "mod.MyClass.method_b", "src/mod.py", 300, 400, Language::Python);

        // MyClass contains method_a and method_b
        let rel_a = make_relation(&class_sym, &method_a, RelationKind::Contains);
        let rel_b = make_relation(&class_sym, &method_b, RelationKind::Contains);

        mgr.graph_mut()
            .insert_symbols(&[class_sym.clone(), method_a.clone(), method_b.clone()], 1000)
            .unwrap();
        mgr.graph_mut()
            .insert_relations(&[rel_a, rel_b], 1000)
            .unwrap();

        let engine = RetrievalEngine::new(&mgr);
        let ctx = engine.get_function_context(method_a.id, 3, 50).unwrap();

        // Hierarchy should include the class and the sibling method
        let hierarchy_ids: Vec<SymbolId> = ctx.hierarchy.iter().map(|n| n.symbol_id).collect();
        assert!(
            hierarchy_ids.contains(&class_sym.id),
            "hierarchy should contain the parent class"
        );
        assert!(
            hierarchy_ids.contains(&method_b.id),
            "hierarchy should contain sibling method_b"
        );
    }

    #[test]
    fn function_context_no_relations() {
        let tmp = TempDir::new().unwrap();
        let mut mgr = setup_storage(&tmp);

        let sym = make_symbol("lonely", "mod.lonely", "src/mod.py", 0, 100, Language::Python);
        mgr.graph_mut().insert_symbols(&[sym.clone()], 1000).unwrap();

        let engine = RetrievalEngine::new(&mgr);
        let ctx = engine.get_function_context(sym.id, 3, 50).unwrap();

        assert_eq!(ctx.symbol.symbol_id, sym.id);
        assert!(ctx.callers.is_empty());
        assert!(ctx.callees.is_empty());
        assert!(ctx.hierarchy.is_empty());
    }
}
