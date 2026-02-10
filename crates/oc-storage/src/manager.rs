use std::path::{Path, PathBuf};

use crate::error::StorageError;
use crate::fulltext::FullTextStore;
use crate::graph::GraphStore;
use crate::vector::VectorStore;

/// SQLite errors that indicate a corrupted or incompatible database file.
fn is_sqlite_corruption(err: &rusqlite::Error) -> bool {
    use rusqlite::ErrorCode;
    match err {
        rusqlite::Error::SqliteFailure(e, _) => matches!(
            e.code,
            ErrorCode::DatabaseCorrupt | ErrorCode::NotADatabase
        ),
        _ => false,
    }
}

/// Default vector dimension (placeholder; real dimension comes from the embedding model).
const DEFAULT_VECTOR_DIMENSION: usize = 384;

/// Metadata file name stored inside `.openace/`.
const META_FILE: &str = "meta.json";

/// Unified facade over GraphStore, VectorStore, and FullTextStore.
///
/// Owns the `.openace/` directory and coordinates initialization, corruption
/// recovery, and access to all three storage backends.
pub struct StorageManager {
    graph: GraphStore,
    vector: VectorStore,
    fulltext: FullTextStore,
    root: PathBuf,
}

impl StorageManager {
    /// Open or create the storage directory at `<project_root>/.openace/`.
    ///
    /// If the directory exists but any backend fails integrity checks
    /// (schema version mismatch, corrupted SQLite, unusable indexes), the
    /// entire `.openace/` directory is purged and re-initialized.
    ///
    /// Reads the vector dimension from the metadata file if it exists,
    /// otherwise uses the default (384).
    pub fn open(project_root: &Path) -> Result<Self, StorageError> {
        let dim = Self::detect_dimension(project_root);
        Self::open_with_dimension(project_root, dim)
    }

    /// Open or create with an explicit vector dimension.
    pub fn open_with_dimension(
        project_root: &Path,
        vector_dimension: usize,
    ) -> Result<Self, StorageError> {
        let root = project_root.join(".openace");

        match Self::try_open(&root, vector_dimension) {
            Ok(mgr) => {
                mgr.save_meta(vector_dimension);
                Ok(mgr)
            }
            Err(e) if Self::should_purge(&e) => {
                Self::purge(&root)?;
                let mgr = Self::try_open(&root, vector_dimension)?;
                mgr.save_meta(vector_dimension);
                Ok(mgr)
            }
            Err(e) => Err(e),
        }
    }

    /// Detect the vector dimension from an existing `.openace/meta.json`.
    /// Returns the default dimension if the file doesn't exist or can't be read.
    fn detect_dimension(project_root: &Path) -> usize {
        let meta_path = project_root.join(".openace").join(META_FILE);
        if let Ok(data) = std::fs::read_to_string(&meta_path) {
            // Simple JSON parsing: look for "embedding_dim": <number>
            if let Some(pos) = data.find("\"embedding_dim\"") {
                let rest = &data[pos..];
                if let Some(colon) = rest.find(':') {
                    let after_colon = rest[colon + 1..].trim_start();
                    // Parse the number (stop at comma, brace, or whitespace)
                    let num_str: String = after_colon
                        .chars()
                        .take_while(|c| c.is_ascii_digit())
                        .collect();
                    if let Ok(dim) = num_str.parse::<usize>() {
                        if dim > 0 {
                            return dim;
                        }
                    }
                }
            }
        }
        DEFAULT_VECTOR_DIMENSION
    }

    /// Save vector dimension to `.openace/meta.json`.
    fn save_meta(&self, vector_dimension: usize) {
        let meta_path = self.root.join(META_FILE);
        let content = format!("{{\"embedding_dim\": {}}}\n", vector_dimension);
        let _ = std::fs::write(&meta_path, content);
    }

    /// Attempt to open all three backends from an `.openace/` directory.
    /// Creates the directory structure if it doesn't exist.
    fn try_open(root: &Path, vector_dimension: usize) -> Result<Self, StorageError> {
        std::fs::create_dir_all(root)?;

        let db_path = root.join("db.sqlite");
        let tantivy_path = root.join("tantivy");
        let vector_path = root.join("vectors.usearch");

        let graph = GraphStore::open(&db_path)?;
        let fulltext = FullTextStore::open(&tantivy_path)?;
        let vector = VectorStore::open(&vector_path, vector_dimension)?;

        Ok(Self {
            graph,
            vector,
            fulltext,
            root: root.to_path_buf(),
        })
    }

    /// Decide whether an error warrants purging the entire `.openace/` directory.
    fn should_purge(err: &StorageError) -> bool {
        match err {
            StorageError::SchemaMismatch { .. }
            | StorageError::VectorIndexUnavailable { .. }
            | StorageError::FullTextIndexUnavailable { .. } => true,
            StorageError::Sqlite(e) => is_sqlite_corruption(e),
            StorageError::Tantivy(_) => true,
            _ => false,
        }
    }

    /// Delete the entire `.openace/` directory.
    fn purge(root: &Path) -> Result<(), StorageError> {
        if root.exists() {
            std::fs::remove_dir_all(root)?;
        }
        Ok(())
    }

    /// Borrow the graph store.
    pub fn graph(&self) -> &GraphStore {
        &self.graph
    }

    /// Mutably borrow the graph store.
    pub fn graph_mut(&mut self) -> &mut GraphStore {
        &mut self.graph
    }

    /// Borrow the vector store.
    pub fn vector(&self) -> &VectorStore {
        &self.vector
    }

    /// Mutably borrow the vector store.
    pub fn vector_mut(&mut self) -> &mut VectorStore {
        &mut self.vector
    }

    /// Borrow the full-text store.
    pub fn fulltext(&self) -> &FullTextStore {
        &self.fulltext
    }

    /// Mutably borrow the full-text store.
    pub fn fulltext_mut(&mut self) -> &mut FullTextStore {
        &mut self.fulltext
    }

    /// The `.openace/` directory path.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Persist all backends that require explicit flushing.
    ///
    /// - Commits pending Tantivy documents.
    /// - Saves the vector index to disk.
    pub fn flush(&mut self) -> Result<(), StorageError> {
        self.fulltext.commit()?;
        let vector_path = self.root.join("vectors.usearch");
        self.vector.save(&vector_path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oc_core::{CodeRelation, CodeSymbol, Language, RelationKind, SymbolId, SymbolKind};
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn make_symbol(name: &str, file: &str, byte_start: usize, byte_end: usize) -> CodeSymbol {
        CodeSymbol {
            id: SymbolId::generate("test-repo", file, name, byte_start, byte_end),
            name: name.to_string(),
            qualified_name: name.to_string(),
            kind: SymbolKind::Function,
            language: Language::Python,
            file_path: PathBuf::from(file),
            byte_range: byte_start..byte_end,
            line_range: 0..10,
            signature: Some(format!("def {name}()")),
            doc_comment: None,
            body_hash: 42,
        }
    }

    #[test]
    fn open_creates_directory_structure() {
        let tmp = TempDir::new().unwrap();
        let mgr = StorageManager::open(tmp.path()).unwrap();

        assert!(mgr.root().exists());
        assert!(mgr.root().join("db.sqlite").exists());
        assert!(mgr.root().join("tantivy").exists());
    }

    #[test]
    fn open_idempotent() {
        let tmp = TempDir::new().unwrap();
        let _mgr1 = StorageManager::open(tmp.path()).unwrap();
        drop(_mgr1);
        let _mgr2 = StorageManager::open(tmp.path()).unwrap();
    }

    #[test]
    fn corrupted_sqlite_triggers_purge_and_rebuild() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join(".openace");

        let mgr = StorageManager::open(tmp.path()).unwrap();
        drop(mgr);

        // Corrupt the SQLite db
        std::fs::write(root.join("db.sqlite"), b"not a sqlite database").unwrap();

        // Re-open should detect corruption, purge, and rebuild
        let mgr = StorageManager::open(tmp.path()).unwrap();
        assert!(mgr.root().join("db.sqlite").exists());
    }

    #[test]
    fn flush_persists_state() {
        let tmp = TempDir::new().unwrap();
        let mut mgr = StorageManager::open(tmp.path()).unwrap();
        mgr.flush().unwrap();
    }

    #[test]
    fn full_lifecycle_integration() {
        let tmp = TempDir::new().unwrap();

        let sym_a = make_symbol("process_data", "src/main.py", 0, 100);
        let sym_b = make_symbol("validate_input", "src/main.py", 200, 350);
        let sym_c = make_symbol("format_output", "src/utils.py", 0, 80);

        let relation = CodeRelation {
            source_id: sym_a.id,
            target_id: sym_b.id,
            kind: RelationKind::Calls,
            file_path: PathBuf::from("src/main.py"),
            line: 5,
            confidence: RelationKind::Calls.default_confidence(),
        };

        // Phase 1: open, populate, flush, close
        {
            let mut mgr = StorageManager::open(tmp.path()).unwrap();

            // Insert symbols into graph
            mgr.graph_mut()
                .insert_symbols(&[sym_a.clone(), sym_b.clone(), sym_c.clone()], 1000)
                .unwrap();
            mgr.graph_mut()
                .insert_relations(&[relation], 1000)
                .unwrap();

            // Index into fulltext
            mgr.fulltext_mut()
                .add_document(&sym_a, Some("def process_data(): validate_input()"))
                .unwrap();
            mgr.fulltext_mut()
                .add_document(&sym_b, Some("def validate_input(): pass"))
                .unwrap();
            mgr.fulltext_mut()
                .add_document(&sym_c, Some("def format_output(): pass"))
                .unwrap();

            // Query graph before close
            let fetched = mgr.graph().get_symbol(sym_a.id).unwrap().unwrap();
            assert_eq!(fetched.name, "process_data");

            let file_syms = mgr.graph().get_symbols_by_file("src/main.py").unwrap();
            assert_eq!(file_syms.len(), 2);

            // Query fulltext before close
            mgr.fulltext_mut().commit().unwrap();
            let hits = mgr
                .fulltext()
                .search_bm25("process", 10, None, None)
                .unwrap();
            assert!(!hits.is_empty());
            assert_eq!(hits[0].symbol_id, sym_a.id);

            // K-hop traversal: sym_a calls sym_b
            use crate::graph::TraversalDirection;
            let neighbors = mgr
                .graph()
                .traverse_khop(sym_a.id, 1, 50, TraversalDirection::Outgoing)
                .unwrap();
            assert_eq!(neighbors.len(), 1);
            assert_eq!(neighbors[0].symbol_id, sym_b.id);

            mgr.flush().unwrap();
        }

        // Phase 2: reopen and verify data persisted
        {
            let mgr = StorageManager::open(tmp.path()).unwrap();

            // Graph data survived
            let fetched = mgr.graph().get_symbol(sym_a.id).unwrap().unwrap();
            assert_eq!(fetched.name, "process_data");

            let fetched_c = mgr.graph().get_symbol(sym_c.id).unwrap().unwrap();
            assert_eq!(fetched_c.name, "format_output");

            let file_syms = mgr.graph().get_symbols_by_file("src/main.py").unwrap();
            assert_eq!(file_syms.len(), 2);

            // Fulltext data survived
            let hits = mgr
                .fulltext()
                .search_bm25("validate", 10, None, None)
                .unwrap();
            assert!(!hits.is_empty());
            assert_eq!(hits[0].symbol_id, sym_b.id);

            // Language filter
            let py_hits = mgr
                .fulltext()
                .search_bm25("format", 10, None, Some(Language::Python))
                .unwrap();
            assert!(!py_hits.is_empty());
            assert_eq!(py_hits[0].symbol_id, sym_c.id);
        }
    }
}
