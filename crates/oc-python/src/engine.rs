use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;

use oc_core::{Language, SymbolId};
use oc_indexer::IndexConfig;
use oc_retrieval::engine::{RetrievalEngine, SearchQuery};
use oc_storage::manager::StorageManager;

use crate::types::{PyChunkData, PyFileInfo, PyIndexReport, PySearchResult, PySummaryChunk, PySymbol};

/// Parse a 32-hex-char string into a SymbolId.
fn parse_symbol_id(hex: &str) -> PyResult<SymbolId> {
    u128::from_str_radix(hex, 16)
        .map(SymbolId)
        .map_err(|e| PyRuntimeError::new_err(format!("invalid symbol ID hex: {e}")))
}

/// Parse a language name string into a Language enum variant.
fn parse_language(name: &str) -> Option<Language> {
    match name.to_lowercase().as_str() {
        "python" => Some(Language::Python),
        "typescript" => Some(Language::TypeScript),
        "javascript" => Some(Language::JavaScript),
        "rust" => Some(Language::Rust),
        "go" => Some(Language::Go),
        "java" => Some(Language::Java),
        _ => None,
    }
}

/// Acquire the mutex and return a guard, failing if poisoned.
fn lock_inner(
    inner: &Mutex<Option<StorageManager>>,
) -> PyResult<MutexGuard<'_, Option<StorageManager>>> {
    inner
        .lock()
        .map_err(|e| PyRuntimeError::new_err(format!("lock poisoned: {e}")))
}

/// Core engine binding wrapping the Rust StorageManager for Python access.
///
/// All heavy operations release the GIL via `py.allow_threads()`.
/// The StorageManager is wrapped in `Arc<Mutex<Option<>>>` so that indexing
/// can temporarily drop it (releasing the Tantivy lock) before re-opening.
#[pyclass]
pub struct EngineBinding {
    inner: Arc<Mutex<Option<StorageManager>>>,
    project_root: PathBuf,
    embedding_dim: usize,
    repo_id: String,
}

#[pymethods]
impl EngineBinding {
    #[new]
    #[pyo3(signature = (project_root, embedding_dim=None))]
    fn new(project_root: &str, embedding_dim: Option<usize>) -> PyResult<Self> {
        crate::init_tracing();

        let path = PathBuf::from(project_root);

        let mgr = match embedding_dim {
            Some(dim) => StorageManager::open_with_dimension(&path, dim),
            None => StorageManager::open(&path),
        }
        .map_err(|e| PyRuntimeError::new_err(format!("failed to open storage: {e}")))?;

        let dim = embedding_dim.unwrap_or_else(|| mgr.vector().dimension());

        let repo_id = project_root.to_string();

        Ok(Self {
            inner: Arc::new(Mutex::new(Some(mgr))),
            project_root: path,
            embedding_dim: dim,
            repo_id,
        })
    }

    /// Run full indexing pipeline on the project.
    ///
    /// Temporarily drops the StorageManager to release the Tantivy lock,
    /// runs the indexer (which opens its own), then re-opens for queries.
    #[pyo3(signature = (repo_root, chunk_enabled=false, trace_id=None))]
    fn index_full(&self, py: Python<'_>, repo_root: &str, chunk_enabled: bool, trace_id: Option<String>) -> PyResult<PyIndexReport> {
        let path = PathBuf::from(repo_root);
        let repo_id = self.repo_id.clone();
        let project_root = self.project_root.clone();
        let dim = self.embedding_dim;
        let inner = Arc::clone(&self.inner);

        py.allow_threads(move || {
            let _span = tracing::info_span!(
                "engine.index_full",
                trace_id = %trace_id.as_deref().unwrap_or("")
            ).entered();
            // Drop the existing StorageManager to release the Tantivy lock
            // before the indexer opens its own.
            {
                let mut locked = lock_inner(&inner)?;
                *locked = None;
            }

            let config = IndexConfig {
                repo_id,
                batch_size: 1000,
                embedding_dim: dim,
                chunk_enabled,
                ..Default::default()
            };
            let result = oc_indexer::index(&path, &config)
                .map(PyIndexReport::from)
                .map_err(|e| PyRuntimeError::new_err(format!("indexing failed: {e}")));

            // Re-open StorageManager so queries use fresh data
            let fresh_mgr = StorageManager::open_with_dimension(&project_root, dim)
                .map_err(|e| PyRuntimeError::new_err(format!("failed to reopen storage: {e}")))?;
            {
                let mut locked = lock_inner(&inner)?;
                *locked = Some(fresh_mgr);
            }

            result
        })
    }

    /// Search for symbols using multi-signal retrieval.
    #[pyo3(signature = (
        text,
        query_vector=None, limit=None, language=None, file_path=None,
        enable_chunk_search=true,
        bm25_weight=1.0, vector_weight=1.0, exact_weight=1.0,
        chunk_bm25_weight=1.0, graph_weight=1.0,
        bm25_pool_size=None, vector_pool_size=None, graph_depth=None,
        bm25_text=None, exact_queries=None,
        trace_id=None,
    ))]
    #[allow(clippy::too_many_arguments)]
    fn search(
        &self,
        py: Python<'_>,
        text: &str,
        query_vector: Option<Vec<f32>>,
        limit: Option<usize>,
        language: Option<&str>,
        file_path: Option<&str>,
        enable_chunk_search: bool,
        bm25_weight: f64,
        vector_weight: f64,
        exact_weight: f64,
        chunk_bm25_weight: f64,
        graph_weight: f64,
        bm25_pool_size: Option<usize>,
        vector_pool_size: Option<usize>,
        graph_depth: Option<u32>,
        bm25_text: Option<String>,
        exact_queries: Option<Vec<String>>,
        trace_id: Option<String>,
    ) -> PyResult<Vec<PySearchResult>> {
        let text = text.to_string();
        let lang = language.and_then(parse_language);
        let fp = file_path.map(String::from);

        let inner = Arc::clone(&self.inner);

        py.allow_threads(move || {
            let _span = tracing::info_span!(
                "engine.search",
                trace_id = %trace_id.as_deref().unwrap_or("")
            ).entered();
            let locked = lock_inner(&inner)?;
            let mgr = locked
                .as_ref()
                .ok_or_else(|| PyRuntimeError::new_err("storage unavailable (indexing in progress)"))?;

            let mut query = SearchQuery::new(&text);
            if let Some(lim) = limit {
                query.limit = lim;
            }
            query.language_filter = lang;
            query.file_path_filter = fp;
            query.query_vector = query_vector;
            query.enable_chunk_search = enable_chunk_search;
            query.bm25_weight = bm25_weight;
            query.vector_weight = vector_weight;
            query.exact_weight = exact_weight;
            query.chunk_bm25_weight = chunk_bm25_weight;
            query.graph_weight = graph_weight;
            if let Some(pool) = bm25_pool_size {
                query.bm25_pool_size = pool;
            }
            if let Some(pool) = vector_pool_size {
                query.vector_pool_size = pool;
            }
            if let Some(depth) = graph_depth {
                query.graph_depth = depth;
            }
            query.bm25_text = bm25_text;
            if let Some(eq) = exact_queries {
                query.exact_queries = eq;
            }

            let engine = RetrievalEngine::new(mgr);
            let results = engine
                .search(&query)
                .map_err(|e| PyRuntimeError::new_err(format!("search failed: {e}")))?;

            Ok(results.into_iter().map(PySearchResult::from).collect())
        })
    }

    /// Find symbols by name (exact match on both name and qualified_name).
    #[pyo3(signature = (name, trace_id=None))]
    fn find_symbol(&self, py: Python<'_>, name: &str, trace_id: Option<String>) -> PyResult<Vec<PySymbol>> {
        let name = name.to_string();
        let inner = Arc::clone(&self.inner);

        py.allow_threads(move || {
            let _span = tracing::info_span!(
                "engine.find_symbol",
                trace_id = %trace_id.as_deref().unwrap_or("")
            ).entered();
            let locked = lock_inner(&inner)?;
            let mgr = locked
                .as_ref()
                .ok_or_else(|| PyRuntimeError::new_err("storage unavailable (indexing in progress)"))?;

            let mut results = Vec::new();
            let mut seen = std::collections::HashSet::new();

            if let Ok(syms) = mgr.graph().get_symbols_by_name(&name) {
                for sym in syms {
                    if seen.insert(sym.id) {
                        results.push(PySymbol::from(sym));
                    }
                }
            }

            if let Ok(syms) = mgr.graph().get_symbols_by_qualified_name(&name) {
                for sym in syms {
                    if seen.insert(sym.id) {
                        results.push(PySymbol::from(sym));
                    }
                }
            }

            Ok(results)
        })
    }

    /// Get all symbols in a file (file outline).
    #[pyo3(signature = (path, trace_id=None))]
    fn get_file_outline(&self, py: Python<'_>, path: &str, trace_id: Option<String>) -> PyResult<Vec<PySymbol>> {
        let path = path.to_string();
        let inner = Arc::clone(&self.inner);

        py.allow_threads(move || {
            let _span = tracing::info_span!(
                "engine.get_file_outline",
                trace_id = %trace_id.as_deref().unwrap_or("")
            ).entered();
            let locked = lock_inner(&inner)?;
            let mgr = locked
                .as_ref()
                .ok_or_else(|| PyRuntimeError::new_err("storage unavailable (indexing in progress)"))?;

            let syms = mgr.graph().get_symbols_by_file(&path).map_err(|e| {
                PyRuntimeError::new_err(format!("get_file_outline failed: {e}"))
            })?;

            Ok(syms.into_iter().map(PySymbol::from).collect())
        })
    }

    /// Add embedding vectors for symbols.
    ///
    /// `ids` and `vectors` must have the same length.
    /// Each id is a 32-hex-character symbol ID string.
    fn add_vectors(
        &self,
        py: Python<'_>,
        ids: Vec<String>,
        vectors: Vec<Vec<f32>>,
    ) -> PyResult<()> {
        if ids.len() != vectors.len() {
            return Err(PyRuntimeError::new_err(
                "ids and vectors must have the same length",
            ));
        }

        let inner = Arc::clone(&self.inner);

        py.allow_threads(move || {
            let mut locked = lock_inner(&inner)?;
            let mgr = locked
                .as_mut()
                .ok_or_else(|| PyRuntimeError::new_err("storage unavailable (indexing in progress)"))?;

            for (hex, vec) in ids.iter().zip(vectors.iter()) {
                let sym_id = parse_symbol_id(hex)?;
                mgr.vector_mut().add_vector(sym_id, vec).map_err(|e| {
                    PyRuntimeError::new_err(format!("add_vector failed: {e}"))
                })?;
            }

            Ok(())
        })
    }

    /// List symbols with pagination for embedding backfill.
    fn list_symbols_for_embedding(
        &self,
        py: Python<'_>,
        limit: usize,
        offset: usize,
    ) -> PyResult<Vec<PySymbol>> {
        let inner = Arc::clone(&self.inner);

        py.allow_threads(move || {
            let locked = lock_inner(&inner)?;
            let mgr = locked
                .as_ref()
                .ok_or_else(|| PyRuntimeError::new_err("storage unavailable (indexing in progress)"))?;

            let syms = mgr.graph().list_symbols(limit, offset).map_err(|e| {
                PyRuntimeError::new_err(format!("list_symbols failed: {e}"))
            })?;

            Ok(syms.into_iter().map(PySymbol::from).collect())
        })
    }

    /// Count total number of symbols in the store.
    fn count_symbols(&self, py: Python<'_>) -> PyResult<usize> {
        let inner = Arc::clone(&self.inner);

        py.allow_threads(move || {
            let locked = lock_inner(&inner)?;
            let mgr = locked
                .as_ref()
                .ok_or_else(|| PyRuntimeError::new_err("storage unavailable (indexing in progress)"))?;

            mgr.graph()
                .count_symbols()
                .map_err(|e| PyRuntimeError::new_err(format!("count_symbols failed: {e}")))
        })
    }

    /// Count total number of chunks in the store.
    fn count_chunks(&self, py: Python<'_>) -> PyResult<usize> {
        let inner = Arc::clone(&self.inner);

        py.allow_threads(move || {
            let locked = lock_inner(&inner)?;
            let mgr = locked
                .as_ref()
                .ok_or_else(|| PyRuntimeError::new_err("storage unavailable (indexing in progress)"))?;

            mgr.graph()
                .count_chunks()
                .map_err(|e| PyRuntimeError::new_err(format!("count_chunks failed: {e}")))
        })
    }

    /// List chunks with pagination for embedding backfill.
    fn list_chunks_for_embedding(
        &self,
        py: Python<'_>,
        limit: usize,
        offset: usize,
    ) -> PyResult<Vec<PyChunkData>> {
        let inner = Arc::clone(&self.inner);

        py.allow_threads(move || {
            let locked = lock_inner(&inner)?;
            let mgr = locked
                .as_ref()
                .ok_or_else(|| PyRuntimeError::new_err("storage unavailable (indexing in progress)"))?;

            let chunks = mgr.graph().list_chunks(limit, offset).map_err(|e| {
                PyRuntimeError::new_err(format!("list_chunks failed: {e}"))
            })?;

            Ok(chunks
                .into_iter()
                .map(|c| PyChunkData {
                    id: format!("{}", c.id),
                    file_path: c.file_path.to_string_lossy().into_owned(),
                    context_path: c.context_path,
                    content: c.content,
                })
                .collect())
        })
    }

    /// List all indexed files with metadata for summary generation.
    fn list_indexed_files(&self, py: Python<'_>) -> PyResult<Vec<PyFileInfo>> {
        let inner = Arc::clone(&self.inner);

        py.allow_threads(move || {
            let locked = lock_inner(&inner)?;
            let mgr = locked
                .as_ref()
                .ok_or_else(|| PyRuntimeError::new_err("storage unavailable (indexing in progress)"))?;

            let files = mgr.graph().list_files().map_err(|e| {
                PyRuntimeError::new_err(format!("list_files failed: {e}"))
            })?;

            Ok(files
                .into_iter()
                .map(|f| PyFileInfo {
                    path: f.path,
                    language: f.language.name().to_string(),
                    symbol_count: f.symbol_count,
                })
                .collect())
        })
    }

    /// Insert summary chunks for files.
    ///
    /// For each chunk, deletes any existing summary for the file, then inserts
    /// the new summary into SQLite and Tantivy.
    fn upsert_summary_chunks(
        &self,
        py: Python<'_>,
        chunks: Vec<PySummaryChunk>,
    ) -> PyResult<usize> {
        let repo_id = self.repo_id.clone();
        let inner = Arc::clone(&self.inner);

        py.allow_threads(move || {
            let mut locked = lock_inner(&inner)?;
            let mgr = locked
                .as_mut()
                .ok_or_else(|| PyRuntimeError::new_err("storage unavailable (indexing in progress)"))?;

            let mut count = 0usize;
            for sc in &chunks {
                let lang = parse_language(&sc.language).unwrap_or(Language::Python);

                // Delete old summary chunks for this file (SQLite + Tantivy)
                let old_chunks = mgr.graph().get_chunks_by_file(&sc.file_path).map_err(|e| {
                    PyRuntimeError::new_err(format!("get_chunks_by_file failed: {e}"))
                })?;
                for old in &old_chunks {
                    if old.context_path == "__file_summary__" {
                        mgr.fulltext_mut().delete_chunk_document(old.id).map_err(|e| {
                            PyRuntimeError::new_err(format!("delete_chunk_document failed: {e}"))
                        })?;
                    }
                }
                mgr.graph_mut().delete_summary_chunks_by_file(&sc.file_path).map_err(|e| {
                    PyRuntimeError::new_err(format!("delete_summary_chunks failed: {e}"))
                })?;

                // Build the CodeChunk
                let content_hash = oc_core::CodeChunk::compute_content_hash(sc.content.as_bytes());
                let chunk = oc_core::CodeChunk {
                    id: oc_core::ChunkId::generate(&repo_id, &sc.file_path, 0, 0),
                    language: lang,
                    file_path: sc.file_path.clone().into(),
                    byte_range: 0..0,
                    line_range: 0..0,
                    chunk_index: 0,
                    total_chunks: 1,
                    context_path: "__file_summary__".to_string(),
                    content: sc.content.clone(),
                    content_hash,
                };

                // Insert into SQLite
                mgr.graph_mut().insert_chunks(&[chunk.clone()], 1000).map_err(|e| {
                    PyRuntimeError::new_err(format!("insert_chunks failed: {e}"))
                })?;

                // Insert into Tantivy
                mgr.fulltext_mut().add_chunk_document(&chunk).map_err(|e| {
                    PyRuntimeError::new_err(format!("add_chunk_document failed: {e}"))
                })?;

                count += 1;
            }

            // Commit Tantivy
            mgr.fulltext_mut().commit().map_err(|e| {
                PyRuntimeError::new_err(format!("fulltext commit failed: {e}"))
            })?;

            Ok(count)
        })
    }

    /// Delete summary chunks for given file paths.
    fn delete_summary_chunks(
        &self,
        py: Python<'_>,
        file_paths: Vec<String>,
    ) -> PyResult<usize> {
        let inner = Arc::clone(&self.inner);

        py.allow_threads(move || {
            let mut locked = lock_inner(&inner)?;
            let mgr = locked
                .as_mut()
                .ok_or_else(|| PyRuntimeError::new_err("storage unavailable (indexing in progress)"))?;

            let mut total = 0usize;
            for fp in &file_paths {
                // Delete from Tantivy first
                let old_chunks = mgr.graph().get_chunks_by_file(fp).map_err(|e| {
                    PyRuntimeError::new_err(format!("get_chunks_by_file failed: {e}"))
                })?;
                for old in &old_chunks {
                    if old.context_path == "__file_summary__" {
                        mgr.fulltext_mut().delete_chunk_document(old.id).map_err(|e| {
                            PyRuntimeError::new_err(format!("delete_chunk_document failed: {e}"))
                        })?;
                    }
                }
                let deleted = mgr.graph_mut().delete_summary_chunks_by_file(fp).map_err(|e| {
                    PyRuntimeError::new_err(format!("delete_summary_chunks failed: {e}"))
                })?;
                total += deleted;
            }

            mgr.fulltext_mut().commit().map_err(|e| {
                PyRuntimeError::new_err(format!("fulltext commit failed: {e}"))
            })?;

            Ok(total)
        })
    }

    /// Flush all storage backends to disk.
    fn flush(&self, py: Python<'_>) -> PyResult<()> {
        let inner = Arc::clone(&self.inner);

        py.allow_threads(move || {
            let mut locked = lock_inner(&inner)?;
            let mgr = locked
                .as_mut()
                .ok_or_else(|| PyRuntimeError::new_err("storage unavailable (indexing in progress)"))?;

            mgr.flush()
                .map_err(|e| PyRuntimeError::new_err(format!("flush failed: {e}")))
        })
    }
}
