use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;

use oc_core::{Language, SymbolId};
use oc_indexer::IndexConfig;
use oc_retrieval::engine::{RetrievalEngine, SearchQuery};
use oc_storage::manager::StorageManager;

use crate::types::{PyChunkData, PyIndexReport, PySearchResult, PySymbol};

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

/// Core engine binding wrapping the Rust StorageManager for Python access.
///
/// All heavy operations release the GIL via `py.allow_threads()`.
/// The StorageManager is wrapped in `Arc<Mutex<>>` for safe concurrent access.
#[pyclass]
pub struct EngineBinding {
    inner: Arc<Mutex<StorageManager>>,
    project_root: PathBuf,
    embedding_dim: usize,
    repo_id: String,
}

#[pymethods]
impl EngineBinding {
    #[new]
    #[pyo3(signature = (project_root, embedding_dim=None))]
    fn new(project_root: &str, embedding_dim: Option<usize>) -> PyResult<Self> {
        let path = PathBuf::from(project_root);

        let mgr = match embedding_dim {
            Some(dim) => StorageManager::open_with_dimension(&path, dim),
            None => StorageManager::open(&path),
        }
        .map_err(|e| PyRuntimeError::new_err(format!("failed to open storage: {e}")))?;

        let dim = embedding_dim.unwrap_or_else(|| mgr.vector().dimension());

        let repo_id = project_root.to_string();

        Ok(Self {
            inner: Arc::new(Mutex::new(mgr)),
            project_root: path,
            embedding_dim: dim,
            repo_id,
        })
    }

    /// Run full indexing pipeline on the project.
    ///
    /// After indexing completes, re-opens the StorageManager so that
    /// subsequent queries reflect the newly indexed data.
    #[pyo3(signature = (repo_root, chunk_enabled=false))]
    fn index_full(&self, py: Python<'_>, repo_root: &str, chunk_enabled: bool) -> PyResult<PyIndexReport> {
        let path = PathBuf::from(repo_root);
        let repo_id = self.repo_id.clone();
        let project_root = self.project_root.clone();
        let dim = self.embedding_dim;
        let inner = Arc::clone(&self.inner);

        py.allow_threads(move || {
            let config = IndexConfig {
                repo_id,
                batch_size: 1000,
                embedding_dim: dim,
                chunk_enabled,
                ..Default::default()
            };
            let report = oc_indexer::index(&path, &config)
                .map(PyIndexReport::from)
                .map_err(|e| PyRuntimeError::new_err(format!("indexing failed: {e}")))?;

            // Re-open StorageManager so queries use fresh data
            let fresh_mgr = StorageManager::open_with_dimension(&project_root, dim)
                .map_err(|e| PyRuntimeError::new_err(format!("failed to reopen storage: {e}")))?;
            let mut locked = inner
                .lock()
                .map_err(|e| PyRuntimeError::new_err(format!("lock poisoned: {e}")))?;
            *locked = fresh_mgr;

            Ok(report)
        })
    }

    /// Search for symbols using multi-signal retrieval.
    #[pyo3(signature = (text, query_vector=None, limit=None, language=None, file_path=None, enable_chunk_search=false))]
    fn search(
        &self,
        py: Python<'_>,
        text: &str,
        query_vector: Option<Vec<f32>>,
        limit: Option<usize>,
        language: Option<&str>,
        file_path: Option<&str>,
        enable_chunk_search: bool,
    ) -> PyResult<Vec<PySearchResult>> {
        let text = text.to_string();
        let lang = language.and_then(parse_language);
        let fp = file_path.map(String::from);

        let inner = Arc::clone(&self.inner);

        py.allow_threads(move || {
            let mgr = inner
                .lock()
                .map_err(|e| PyRuntimeError::new_err(format!("lock poisoned: {e}")))?;

            let mut query = SearchQuery::new(&text);
            if let Some(lim) = limit {
                query.limit = lim;
            }
            query.language_filter = lang;
            query.file_path_filter = fp;
            query.query_vector = query_vector;
            query.enable_chunk_search = enable_chunk_search;

            let engine = RetrievalEngine::new(&mgr);
            let results = engine
                .search(&query)
                .map_err(|e| PyRuntimeError::new_err(format!("search failed: {e}")))?;

            Ok(results.into_iter().map(PySearchResult::from).collect())
        })
    }

    /// Find symbols by name (exact match on both name and qualified_name).
    fn find_symbol(&self, py: Python<'_>, name: &str) -> PyResult<Vec<PySymbol>> {
        let name = name.to_string();
        let inner = Arc::clone(&self.inner);

        py.allow_threads(move || {
            let mgr = inner
                .lock()
                .map_err(|e| PyRuntimeError::new_err(format!("lock poisoned: {e}")))?;

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
    fn get_file_outline(&self, py: Python<'_>, path: &str) -> PyResult<Vec<PySymbol>> {
        let path = path.to_string();
        let inner = Arc::clone(&self.inner);

        py.allow_threads(move || {
            let mgr = inner
                .lock()
                .map_err(|e| PyRuntimeError::new_err(format!("lock poisoned: {e}")))?;

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
            let mut mgr = inner
                .lock()
                .map_err(|e| PyRuntimeError::new_err(format!("lock poisoned: {e}")))?;

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
            let mgr = inner
                .lock()
                .map_err(|e| PyRuntimeError::new_err(format!("lock poisoned: {e}")))?;

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
            let mgr = inner
                .lock()
                .map_err(|e| PyRuntimeError::new_err(format!("lock poisoned: {e}")))?;

            mgr.graph()
                .count_symbols()
                .map_err(|e| PyRuntimeError::new_err(format!("count_symbols failed: {e}")))
        })
    }

    /// Count total number of chunks in the store.
    fn count_chunks(&self, py: Python<'_>) -> PyResult<usize> {
        let inner = Arc::clone(&self.inner);

        py.allow_threads(move || {
            let mgr = inner
                .lock()
                .map_err(|e| PyRuntimeError::new_err(format!("lock poisoned: {e}")))?;

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
            let mgr = inner
                .lock()
                .map_err(|e| PyRuntimeError::new_err(format!("lock poisoned: {e}")))?;

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

    /// Flush all storage backends to disk.
    fn flush(&self, py: Python<'_>) -> PyResult<()> {
        let inner = Arc::clone(&self.inner);

        py.allow_threads(move || {
            let mut mgr = inner
                .lock()
                .map_err(|e| PyRuntimeError::new_err(format!("lock poisoned: {e}")))?;

            mgr.flush()
                .map_err(|e| PyRuntimeError::new_err(format!("flush failed: {e}")))
        })
    }
}
