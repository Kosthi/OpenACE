use std::fmt;
use std::ops::Range;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use xxhash_rust::xxh3::{xxh3_128, xxh3_64};

use crate::language::Language;

/// Deterministic chunk identifier, computed as XXH3-128 of
/// `repo_id|relative_path|chunk|byte_start|byte_end`.
///
/// The literal `chunk` in the hash input distinguishes chunk IDs from symbol IDs,
/// ensuring no collision even if a chunk and symbol share the same byte range.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ChunkId(pub u128);

impl ChunkId {
    /// Generate a deterministic chunk ID from its identifying fields.
    pub fn generate(
        repo_id: &str,
        relative_path: &str,
        byte_start: usize,
        byte_end: usize,
    ) -> Self {
        let input = format!(
            "{}|{}|chunk|{}|{}",
            repo_id, relative_path, byte_start, byte_end
        );
        Self(xxh3_128(input.as_bytes()))
    }

    pub fn as_bytes(&self) -> [u8; 16] {
        self.0.to_le_bytes()
    }

    pub fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(u128::from_le_bytes(bytes))
    }
}

impl fmt::Debug for ChunkId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ChunkId({:032x})", self.0)
    }
}

impl fmt::Display for ChunkId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:032x}", self.0)
    }
}

/// An AST-aware code chunk extracted from a source file.
///
/// Chunks split large files into semantically coherent pieces using the
/// cAST algorithm: the tree-sitter AST is traversed and child nodes are
/// greedily grouped into windows that respect a non-whitespace character budget.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeChunk {
    pub id: ChunkId,
    pub language: Language,
    /// Relative to project root, forward-slash normalized.
    pub file_path: PathBuf,
    pub byte_range: Range<usize>,
    /// 0-indexed, end-exclusive.
    pub line_range: Range<u32>,
    /// 0-based index of this chunk within the file.
    pub chunk_index: u32,
    /// Total number of chunks in the file.
    pub total_chunks: u32,
    /// Dot-separated ancestor scope chain (e.g., "MyClass.my_method").
    /// Empty string for top-level chunks.
    pub context_path: String,
    /// Source text of the chunk (capped at 10 KB).
    pub content: String,
    /// XXH3-64 of the chunk content bytes.
    pub content_hash: u64,
}

impl CodeChunk {
    /// Compute a content hash for the given bytes.
    pub fn compute_content_hash(content: &[u8]) -> u64 {
        xxh3_64(content)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_id_deterministic() {
        let id1 = ChunkId::generate("repo1", "src/main.py", 100, 200);
        let id2 = ChunkId::generate("repo1", "src/main.py", 100, 200);
        assert_eq!(id1, id2);
    }

    #[test]
    fn chunk_id_changes_on_path_change() {
        let id1 = ChunkId::generate("repo1", "src/a.py", 0, 50);
        let id2 = ChunkId::generate("repo1", "src/b.py", 0, 50);
        assert_ne!(id1, id2);
    }

    #[test]
    fn chunk_id_changes_on_span_change() {
        let id1 = ChunkId::generate("repo1", "src/a.py", 0, 50);
        let id2 = ChunkId::generate("repo1", "src/a.py", 0, 51);
        assert_ne!(id1, id2);
    }

    #[test]
    fn chunk_id_differs_from_symbol_id() {
        use crate::symbol::SymbolId;
        // Same repo, path, and byte range -- but different ID type
        let chunk_id = ChunkId::generate("repo1", "src/main.py", 0, 100);
        let symbol_id = SymbolId::generate("repo1", "src/main.py", "some_name", 0, 100);
        // The u128 values should differ because chunk uses "chunk" in the hash input
        assert_ne!(chunk_id.0, symbol_id.0);
    }

    #[test]
    fn chunk_id_bytes_round_trip() {
        let id = ChunkId::generate("repo1", "src/main.rs", 10, 20);
        let bytes = id.as_bytes();
        let id2 = ChunkId::from_bytes(bytes);
        assert_eq!(id, id2);
    }

    #[test]
    fn chunk_id_display_hex() {
        let id = ChunkId(0xDEAD_BEEF_CAFE_BABE_1234_5678_9ABC_DEF0);
        let hex = format!("{id}");
        assert_eq!(hex, "deadbeefcafebabe123456789abcdef0");
    }

    #[test]
    fn content_hash_deterministic() {
        let h1 = CodeChunk::compute_content_hash(b"def foo(): pass");
        let h2 = CodeChunk::compute_content_hash(b"def foo(): pass");
        assert_eq!(h1, h2);
    }

    #[test]
    fn content_hash_differs() {
        let h1 = CodeChunk::compute_content_hash(b"def foo(): pass");
        let h2 = CodeChunk::compute_content_hash(b"def bar(): pass");
        assert_ne!(h1, h2);
    }
}
