use std::collections::HashMap;
use std::path::Path;

use oc_core::SymbolId;
use usearch::{Index, IndexOptions, MetricKind, ScalarKind};

use crate::error::StorageError;

/// A single search hit from k-NN search.
#[derive(Debug, Clone)]
pub struct VectorHit {
    pub symbol_id: SymbolId,
    pub distance: f32,
}

/// HNSW vector index backed by usearch.
///
/// Configuration: cosine distance, M=32, ef_construction=200, ef_search=100.
/// Dimension is fixed at creation time.
///
/// Since usearch keys are u64 but SymbolId is u128, we maintain a bidirectional
/// mapping (lower-64-bit key <-> full SymbolId). The mapping is persisted as a
/// sidecar file alongside the usearch index.
pub struct VectorStore {
    index: Index,
    dimension: usize,
    /// Maps usearch u64 key -> full SymbolId.
    key_to_id: HashMap<u64, SymbolId>,
}

impl VectorStore {
    /// Create a new in-memory vector index with the given dimension.
    pub fn new(dimension: usize) -> Result<Self, StorageError> {
        let index = create_index(dimension)?;
        Ok(Self {
            index,
            dimension,
            key_to_id: HashMap::new(),
        })
    }

    /// Open an existing vector index from disk, or create a new one if the file doesn't exist.
    pub fn open(path: &Path, dimension: usize) -> Result<Self, StorageError> {
        if path.exists() {
            let index = create_index(dimension)?;
            index.load(path.to_str().unwrap_or("")).map_err(|e| {
                StorageError::VectorIndexUnavailable {
                    reason: format!("failed to load vector index: {e}"),
                }
            })?;
            let loaded_dim = index.dimensions();
            if loaded_dim != dimension {
                return Err(StorageError::DimensionMismatch {
                    expected: dimension,
                    actual: loaded_dim,
                });
            }
            let key_to_id = load_key_map(path)?;
            Ok(Self {
                index,
                dimension,
                key_to_id,
            })
        } else {
            Self::new(dimension)
        }
    }

    /// The fixed dimension of vectors in this index.
    pub fn dimension(&self) -> usize {
        self.dimension
    }

    /// Number of vectors currently in the index.
    pub fn len(&self) -> usize {
        self.key_to_id.len()
    }

    /// Whether the index is empty.
    pub fn is_empty(&self) -> bool {
        self.key_to_id.is_empty()
    }

    /// Add a vector for the given symbol. Overwrites if the symbol already exists.
    pub fn add_vector(
        &mut self,
        symbol_id: SymbolId,
        vector: &[f32],
    ) -> Result<(), StorageError> {
        if vector.len() != self.dimension {
            return Err(StorageError::DimensionMismatch {
                expected: self.dimension,
                actual: vector.len(),
            });
        }
        let key = symbol_id_to_key(symbol_id);
        // Remove existing entry first to ensure idempotent add (single entry per key).
        if self.index.contains(key) {
            let _ = self.index.remove(key);
        }
        // Ensure capacity before adding.
        if self.index.size() >= self.index.capacity() {
            let new_cap = (self.index.capacity() + 1).max(64) * 2;
            self.index.reserve(new_cap).map_err(|e| {
                StorageError::VectorIndexUnavailable {
                    reason: format!("reserve failed: {e}"),
                }
            })?;
        }
        self.index.add(key, vector).map_err(|e| {
            StorageError::VectorIndexUnavailable {
                reason: format!("add failed: {e}"),
            }
        })?;
        self.key_to_id.insert(key, symbol_id);
        Ok(())
    }

    /// Remove the vector for the given symbol. Returns true if it existed.
    pub fn remove_vector(&mut self, symbol_id: SymbolId) -> Result<bool, StorageError> {
        let key = symbol_id_to_key(symbol_id);
        if !self.index.contains(key) {
            return Ok(false);
        }
        self.index.remove(key).map_err(|e| {
            StorageError::VectorIndexUnavailable {
                reason: format!("remove failed: {e}"),
            }
        })?;
        self.key_to_id.remove(&key);
        Ok(true)
    }

    /// Search for the k nearest neighbors of the query vector.
    pub fn search_knn(
        &self,
        query: &[f32],
        k: usize,
    ) -> Result<Vec<VectorHit>, StorageError> {
        if query.len() != self.dimension {
            return Err(StorageError::DimensionMismatch {
                expected: self.dimension,
                actual: query.len(),
            });
        }
        if self.index.size() == 0 {
            return Ok(Vec::new());
        }
        let matches = self.index.search(query, k).map_err(|e| {
            StorageError::VectorIndexUnavailable {
                reason: format!("search failed: {e}"),
            }
        })?;
        let hits = matches
            .keys
            .iter()
            .zip(matches.distances.iter())
            .filter_map(|(&key, &distance)| {
                self.key_to_id.get(&key).map(|&symbol_id| VectorHit {
                    symbol_id,
                    distance,
                })
            })
            .collect();
        Ok(hits)
    }

    /// Persist the index and key map to disk.
    pub fn save(&self, path: &Path) -> Result<(), StorageError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        self.index
            .save(path.to_str().unwrap_or(""))
            .map_err(|e| StorageError::VectorIndexUnavailable {
                reason: format!("save failed: {e}"),
            })?;
        save_key_map(path, &self.key_to_id)?;
        Ok(())
    }
}

/// Map a SymbolId (u128) to a usearch Key (u64) using the lower 64 bits.
fn symbol_id_to_key(id: SymbolId) -> u64 {
    id.0 as u64
}

fn create_index(dimension: usize) -> Result<Index, StorageError> {
    let mut options = IndexOptions::default();
    options.dimensions = dimension;
    options.metric = MetricKind::Cos;
    options.quantization = ScalarKind::F32;
    options.connectivity = 32; // M=32
    options.expansion_add = 200; // ef_construction=200
    options.expansion_search = 100; // ef_search=100

    Index::new(&options).map_err(|e| StorageError::VectorIndexUnavailable {
        reason: format!("failed to create index: {e}"),
    })
}

/// Sidecar file path for the key-to-SymbolId mapping.
fn key_map_path(index_path: &Path) -> std::path::PathBuf {
    index_path.with_extension("keymap")
}

/// Persist the u64->SymbolId mapping as a flat binary file.
/// Format: [count: u64] [key: u64, id_lo: u64, id_hi: u64] * count
fn save_key_map(index_path: &Path, map: &HashMap<u64, SymbolId>) -> Result<(), StorageError> {
    use std::io::Write;
    let path = key_map_path(index_path);
    let mut buf = Vec::with_capacity(8 + map.len() * 24);
    buf.extend_from_slice(&(map.len() as u64).to_le_bytes());
    for (&key, &sym_id) in map {
        buf.extend_from_slice(&key.to_le_bytes());
        buf.extend_from_slice(&(sym_id.0 as u64).to_le_bytes());
        buf.extend_from_slice(&((sym_id.0 >> 64) as u64).to_le_bytes());
    }
    let mut f = std::fs::File::create(&path)?;
    f.write_all(&buf)?;
    Ok(())
}

/// Load the u64->SymbolId mapping from the sidecar file.
fn load_key_map(index_path: &Path) -> Result<HashMap<u64, SymbolId>, StorageError> {
    let path = key_map_path(index_path);
    if !path.exists() {
        return Ok(HashMap::new());
    }
    let data = std::fs::read(&path)?;
    if data.len() < 8 {
        return Err(StorageError::VectorIndexUnavailable {
            reason: "keymap file too short".to_string(),
        });
    }
    let count = u64::from_le_bytes(data[0..8].try_into().unwrap()) as usize;
    if data.len() != 8 + count * 24 {
        return Err(StorageError::VectorIndexUnavailable {
            reason: "keymap file size mismatch".to_string(),
        });
    }
    let mut map = HashMap::with_capacity(count);
    for i in 0..count {
        let offset = 8 + i * 24;
        let key = u64::from_le_bytes(data[offset..offset + 8].try_into().unwrap());
        let lo = u64::from_le_bytes(data[offset + 8..offset + 16].try_into().unwrap());
        let hi = u64::from_le_bytes(data[offset + 16..offset + 24].try_into().unwrap());
        let sym_id = SymbolId((hi as u128) << 64 | lo as u128);
        map.insert(key, sym_id);
    }
    Ok(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_symbol_id(n: u128) -> SymbolId {
        SymbolId(n)
    }

    #[test]
    fn test_add_and_search_round_trip() {
        let mut store = VectorStore::new(4).unwrap();
        let id1 = make_symbol_id(1);
        let id2 = make_symbol_id(2);

        store.add_vector(id1, &[1.0, 0.0, 0.0, 0.0]).unwrap();
        store.add_vector(id2, &[0.0, 1.0, 0.0, 0.0]).unwrap();

        let results = store.search_knn(&[1.0, 0.0, 0.0, 0.0], 2).unwrap();
        assert!(!results.is_empty());
        assert_eq!(results[0].symbol_id, id1);
    }

    #[test]
    fn test_full_128bit_symbol_id_round_trip() {
        let mut store = VectorStore::new(4).unwrap();
        // Use a SymbolId with significant upper 64 bits.
        let id = SymbolId(0xDEAD_BEEF_CAFE_BABE_1234_5678_9ABC_DEF0);

        store.add_vector(id, &[1.0, 0.0, 0.0, 0.0]).unwrap();
        let results = store.search_knn(&[1.0, 0.0, 0.0, 0.0], 1).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].symbol_id, id);
    }

    #[test]
    fn test_remove_vector_exclusion() {
        let mut store = VectorStore::new(4).unwrap();
        let id1 = make_symbol_id(10);
        let id2 = make_symbol_id(20);

        store.add_vector(id1, &[1.0, 0.0, 0.0, 0.0]).unwrap();
        store.add_vector(id2, &[0.0, 1.0, 0.0, 0.0]).unwrap();

        assert!(store.remove_vector(id1).unwrap());
        let results = store.search_knn(&[1.0, 0.0, 0.0, 0.0], 10).unwrap();
        for hit in &results {
            assert_ne!(hit.symbol_id, id1);
        }
    }

    #[test]
    fn test_dimension_mismatch_error() {
        let mut store = VectorStore::new(4).unwrap();
        let id = make_symbol_id(1);

        let err = store.add_vector(id, &[1.0, 2.0]).unwrap_err();
        assert!(matches!(
            err,
            StorageError::DimensionMismatch {
                expected: 4,
                actual: 2
            }
        ));

        // search with wrong dimension
        store.add_vector(id, &[1.0, 0.0, 0.0, 0.0]).unwrap();
        let err = store.search_knn(&[1.0, 2.0], 1).unwrap_err();
        assert!(matches!(
            err,
            StorageError::DimensionMismatch {
                expected: 4,
                actual: 2
            }
        ));
    }

    #[test]
    fn test_idempotent_add() {
        let mut store = VectorStore::new(4).unwrap();
        let id = make_symbol_id(42);
        let vec1 = [1.0, 0.0, 0.0, 0.0];

        store.add_vector(id, &vec1).unwrap();
        store.add_vector(id, &vec1).unwrap();

        // Should contain exactly one entry for this key
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn test_persistence_reload() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vectors.usearch");
        let id1 = SymbolId(0xAAAA_BBBB_CCCC_DDDD_1111_2222_3333_4444);
        let id2 = SymbolId(0xFFFF_EEEE_DDDD_CCCC_5555_6666_7777_8888);

        // Create, add, save
        {
            let mut store = VectorStore::new(4).unwrap();
            store.add_vector(id1, &[1.0, 0.0, 0.0, 0.0]).unwrap();
            store.add_vector(id2, &[0.0, 1.0, 0.0, 0.0]).unwrap();
            store.save(&path).unwrap();
        }

        // Reload and verify full 128-bit IDs survive the round-trip
        {
            let store = VectorStore::open(&path, 4).unwrap();
            assert_eq!(store.len(), 2);
            let results = store.search_knn(&[1.0, 0.0, 0.0, 0.0], 1).unwrap();
            assert_eq!(results.len(), 1);
            assert_eq!(results[0].symbol_id, id1);
        }
    }

    #[test]
    fn test_corrupted_file_returns_unavailable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vectors.usearch");
        std::fs::write(&path, b"not a valid usearch file").unwrap();

        let result = VectorStore::open(&path, 4);
        assert!(matches!(
            result,
            Err(StorageError::VectorIndexUnavailable { .. })
        ));
    }

    #[test]
    fn test_search_empty_index() {
        let store = VectorStore::new(4).unwrap();
        let results = store.search_knn(&[1.0, 0.0, 0.0, 0.0], 10).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_remove_nonexistent() {
        let mut store = VectorStore::new(4).unwrap();
        let removed = store.remove_vector(make_symbol_id(999)).unwrap();
        assert!(!removed);
    }
}
