use std::collections::{HashMap, HashSet};

use oc_core::{CodeSymbol, Language, QualifiedName, SymbolId, SymbolKind};
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
            enable_chunk_search: false,
            chunk_bm25_pool_size: 100,
            bm25_weight: 1.0,
            vector_weight: 1.0,
            exact_weight: 1.0,
            chunk_bm25_weight: 1.0,
            graph_weight: 1.0,
        }
    }

    fn effective_limit(&self) -> usize {
        self.limit.min(200)
    }

    fn effective_graph_depth(&self) -> u32 {
        self.graph_depth.min(5)
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
    pub fn search(&self, query: &SearchQuery) -> Result<Vec<SearchResult>, RetrievalError> {
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
            self.expand_graph(query, &mut candidates);
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

        Ok(results)
    }

    /// BM25 signal: query Tantivy, assign RRF scores by rank.
    fn collect_bm25(
        &self,
        query: &SearchQuery,
        candidates: &mut HashMap<SymbolId, ScoredCandidate>,
    ) {
        let hits = self.storage.fulltext().search_bm25(
            &query.text,
            query.bm25_pool_size,
            query.file_path_filter.as_deref(),
            query.language_filter,
        );

        let hits = match hits {
            Ok(h) => h,
            Err(_) => return, // graceful degradation
        };

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
            Err(_) => return, // graceful degradation
        };

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
    fn collect_exact_match(
        &self,
        query: &SearchQuery,
        candidates: &mut HashMap<SymbolId, ScoredCandidate>,
    ) {
        let mut exact_hits: Vec<CodeSymbol> = Vec::new();
        let mut seen: HashSet<SymbolId> = HashSet::new();

        // Name match
        if let Ok(syms) = self.storage.graph().get_symbols_by_name(&query.text) {
            for sym in syms {
                if seen.insert(sym.id) {
                    exact_hits.push(sym);
                }
            }
        }

        // Qualified name match
        if let Ok(syms) = self.storage.graph().get_symbols_by_qualified_name(&query.text) {
            for sym in syms {
                if seen.insert(sym.id) {
                    exact_hits.push(sym);
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
        let hits = self.storage.fulltext().search_bm25_chunks(
            &query.text,
            query.chunk_bm25_pool_size,
            query.file_path_filter.as_deref(),
            query.language_filter,
        );

        let hits = match hits {
            Ok(h) => h,
            Err(_) => return, // graceful degradation
        };

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

        Ok(())
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
        assert!(!q.enable_chunk_search);
        assert_eq!(q.chunk_bm25_pool_size, 100);
        assert!((q.bm25_weight - 1.0).abs() < f64::EPSILON);
        assert!((q.vector_weight - 1.0).abs() < f64::EPSILON);
        assert!((q.exact_weight - 1.0).abs() < f64::EPSILON);
        assert!((q.chunk_bm25_weight - 1.0).abs() < f64::EPSILON);
        assert!((q.graph_weight - 1.0).abs() < f64::EPSILON);
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

        // Graph-expanded results should have the "graph" signal
        let graph_result = results.iter().find(|r| r.symbol_id == sym_b.id).unwrap();
        assert!(graph_result.match_signals.contains(&"graph".to_string()));
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
}
