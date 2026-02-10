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
/// mapping using counter-based surrogate keys. Each SymbolId is assigned a
/// monotonically increasing u64 key, eliminating hash collision risk.
/// The mapping is persisted as a sidecar file alongside the usearch index.
pub struct VectorStore {
    index: Index,
    dimension: usize,
    /// Maps usearch u64 key -> full SymbolId.
    key_to_id: HashMap<u64, SymbolId>,
    /// Maps full SymbolId -> usearch u64 key (reverse lookup).
    id_to_key: HashMap<SymbolId, u64>,
    /// Next surrogate key to allocate.
    next_key: u64,
}

impl VectorStore {
    /// Create a new in-memory vector index with the given dimension.
    pub fn new(dimension: usize) -> Result<Self, StorageError> {
        let index = create_index(dimension)?;
        Ok(Self {
            index,
            dimension,
            key_to_id: HashMap::new(),
            id_to_key: HashMap::new(),
            next_key: 0,
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
            let (key_to_id, next_key) = load_key_map(path)?;
            let id_to_key: HashMap<SymbolId, u64> =
                key_to_id.iter().map(|(&k, &v)| (v, k)).collect();
            Ok(Self {
                index,
                dimension,
                key_to_id,
                id_to_key,
                next_key,
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
        let key = *self.id_to_key.entry(symbol_id).or_insert_with(|| {
            let k = self.next_key;
            self.next_key += 1;
            k
        });
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
        let key = match self.id_to_key.remove(&symbol_id) {
            Some(k) => k,
            None => return Ok(false),
        };
        if self.index.contains(key) {
            self.index.remove(key).map_err(|e| {
                StorageError::VectorIndexUnavailable {
                    reason: format!("remove failed: {e}"),
                }
            })?;
        }
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
        save_key_map(path, &self.key_to_id, self.next_key)?;
        Ok(())
    }
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
/// Format v2: [next_key: u64] [count: u64] [key: u64, id_lo: u64, id_hi: u64] * count
fn save_key_map(
    index_path: &Path,
    map: &HashMap<u64, SymbolId>,
    next_key: u64,
) -> Result<(), StorageError> {
    use std::io::Write;
    let path = key_map_path(index_path);
    let mut buf = Vec::with_capacity(16 + map.len() * 24);
    buf.extend_from_slice(&next_key.to_le_bytes());
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
/// Supports both v1 (no next_key prefix) and v2 (with next_key prefix) formats.
fn load_key_map(index_path: &Path) -> Result<(HashMap<u64, SymbolId>, u64), StorageError> {
    let path = key_map_path(index_path);
    if !path.exists() {
        return Ok((HashMap::new(), 0));
    }
    let data = std::fs::read(&path)?;
    if data.len() < 8 {
        return Err(StorageError::VectorIndexUnavailable {
            reason: "keymap file too short".to_string(),
        });
    }

    // Detect format: v2 has next_key prefix (16 bytes header), v1 has count-only (8 bytes header).
    // Try v2 first: read next_key and count, check if file size matches.
    if data.len() >= 16 {
        let next_key = u64::from_le_bytes(data[0..8].try_into().unwrap());
        let count = u64::from_le_bytes(data[8..16].try_into().unwrap()) as usize;
        if data.len() == 16 + count * 24 {
            // v2 format
            let mut map = HashMap::with_capacity(count);
            for i in 0..count {
                let offset = 16 + i * 24;
                let key = u64::from_le_bytes(data[offset..offset + 8].try_into().unwrap());
                let lo = u64::from_le_bytes(data[offset + 8..offset + 16].try_into().unwrap());
                let hi = u64::from_le_bytes(data[offset + 16..offset + 24].try_into().unwrap());
                let sym_id = SymbolId((hi as u128) << 64 | lo as u128);
                map.insert(key, sym_id);
            }
            return Ok((map, next_key));
        }
    }

    // Fall back to v1 format: [count: u64] [key: u64, id_lo: u64, id_hi: u64] * count
    let count = u64::from_le_bytes(data[0..8].try_into().unwrap()) as usize;
    if data.len() != 8 + count * 24 {
        return Err(StorageError::VectorIndexUnavailable {
            reason: "keymap file size mismatch".to_string(),
        });
    }
    let mut map = HashMap::with_capacity(count);
    let mut max_key: u64 = 0;
    for i in 0..count {
        let offset = 8 + i * 24;
        let key = u64::from_le_bytes(data[offset..offset + 8].try_into().unwrap());
        let lo = u64::from_le_bytes(data[offset + 8..offset + 16].try_into().unwrap());
        let hi = u64::from_le_bytes(data[offset + 16..offset + 24].try_into().unwrap());
        let sym_id = SymbolId((hi as u128) << 64 | lo as u128);
        map.insert(key, sym_id);
        if key >= max_key {
            max_key = key + 1;
        }
    }
    Ok((map, max_key))
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
    fn test_no_collision_different_upper_bits() {
        // Two SymbolIds that share the same lower 64 bits but differ in upper 64.
        // With the old truncation approach, these would collide.
        let mut store = VectorStore::new(4).unwrap();
        let id1 = SymbolId(0x0000_0000_0000_0001_FFFF_FFFF_FFFF_FFFF);
        let id2 = SymbolId(0x0000_0000_0000_0002_FFFF_FFFF_FFFF_FFFF);

        store.add_vector(id1, &[1.0, 0.0, 0.0, 0.0]).unwrap();
        store.add_vector(id2, &[0.0, 1.0, 0.0, 0.0]).unwrap();

        // Both should exist independently
        assert_eq!(store.len(), 2);

        let results = store.search_knn(&[1.0, 0.0, 0.0, 0.0], 2).unwrap();
        assert_eq!(results.len(), 2);
        let ids: Vec<SymbolId> = results.iter().map(|h| h.symbol_id).collect();
        assert!(ids.contains(&id1));
        assert!(ids.contains(&id2));
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
    fn test_surrogate_key_persistence() {
        // Verify that next_key is persisted correctly across save/load cycles
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vectors.usearch");
        let id1 = SymbolId(100);
        let id2 = SymbolId(200);

        // Create, add, save
        {
            let mut store = VectorStore::new(4).unwrap();
            store.add_vector(id1, &[1.0, 0.0, 0.0, 0.0]).unwrap();
            store.save(&path).unwrap();
            assert_eq!(store.next_key, 1);
        }

        // Reload, add more, save again
        {
            let mut store = VectorStore::open(&path, 4).unwrap();
            assert_eq!(store.next_key, 1);
            store.add_vector(id2, &[0.0, 1.0, 0.0, 0.0]).unwrap();
            assert_eq!(store.next_key, 2);
            store.save(&path).unwrap();
        }

        // Final reload: both entries present, next_key is 2
        {
            let store = VectorStore::open(&path, 4).unwrap();
            assert_eq!(store.len(), 2);
            assert_eq!(store.next_key, 2);
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
