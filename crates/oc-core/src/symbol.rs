use std::fmt;
use std::ops::Range;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use xxhash_rust::xxh3::xxh3_128;

use crate::language::Language;

/// Deterministic symbol identifier, computed as XXH3-128 of
/// `repo_id|relative_path|qualified_name|byte_start|byte_end`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SymbolId(pub u128);

impl SymbolId {
    /// Generate a deterministic symbol ID from its identifying fields.
    pub fn generate(
        repo_id: &str,
        relative_path: &str,
        qualified_name: &str,
        byte_start: usize,
        byte_end: usize,
    ) -> Self {
        let input = format!(
            "{}|{}|{}|{}|{}",
            repo_id, relative_path, qualified_name, byte_start, byte_end
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

impl fmt::Debug for SymbolId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SymbolId({:032x})", self.0)
    }
}

impl fmt::Display for SymbolId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:032x}", self.0)
    }
}

/// Kinds of code symbols that can be extracted from source files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum SymbolKind {
    Function = 0,
    Method = 1,
    Class = 2,
    Struct = 3,
    Interface = 4,
    Trait = 5,
    Module = 6,
    Package = 7,
    Variable = 8,
    Constant = 9,
    Enum = 10,
    TypeAlias = 11,
}

impl SymbolKind {
    pub fn from_ordinal(n: u8) -> Option<Self> {
        match n {
            0 => Some(Self::Function),
            1 => Some(Self::Method),
            2 => Some(Self::Class),
            3 => Some(Self::Struct),
            4 => Some(Self::Interface),
            5 => Some(Self::Trait),
            6 => Some(Self::Module),
            7 => Some(Self::Package),
            8 => Some(Self::Variable),
            9 => Some(Self::Constant),
            10 => Some(Self::Enum),
            11 => Some(Self::TypeAlias),
            _ => None,
        }
    }

    pub fn ordinal(self) -> u8 {
        self as u8
    }
}

/// A code symbol extracted from a source file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeSymbol {
    pub id: SymbolId,
    pub name: String,
    /// Dot-separated canonical qualified name (e.g., "module.Class.method").
    pub qualified_name: String,
    pub kind: SymbolKind,
    pub language: Language,
    /// Relative to project root, forward-slash normalized.
    pub file_path: PathBuf,
    pub byte_range: Range<usize>,
    /// 0-indexed, end-exclusive.
    pub line_range: Range<u32>,
    pub signature: Option<String>,
    pub doc_comment: Option<String>,
    /// XXH3-128 lower 64 bits of the symbol body bytes.
    pub body_hash: u64,
    /// Optional source text of the symbol body (truncated to 10 KB).
    pub body_text: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbol_id_deterministic() {
        let id1 = SymbolId::generate("repo1", "src/main.py", "main.MyClass.run", 100, 200);
        let id2 = SymbolId::generate("repo1", "src/main.py", "main.MyClass.run", 100, 200);
        assert_eq!(id1, id2);
    }

    #[test]
    fn symbol_id_changes_on_path_change() {
        let id1 = SymbolId::generate("repo1", "src/a.py", "a.Foo", 0, 50);
        let id2 = SymbolId::generate("repo1", "src/b.py", "a.Foo", 0, 50);
        assert_ne!(id1, id2);
    }

    #[test]
    fn symbol_id_changes_on_span_change() {
        let id1 = SymbolId::generate("repo1", "src/a.py", "a.Foo", 0, 50);
        let id2 = SymbolId::generate("repo1", "src/a.py", "a.Foo", 0, 51);
        assert_ne!(id1, id2);
    }

    #[test]
    fn symbol_id_bytes_round_trip() {
        let id = SymbolId::generate("repo1", "src/main.rs", "main.foo", 10, 20);
        let bytes = id.as_bytes();
        let id2 = SymbolId::from_bytes(bytes);
        assert_eq!(id, id2);
    }

    #[test]
    fn symbol_kind_ordinal_round_trip() {
        for n in 0..=11u8 {
            let kind = SymbolKind::from_ordinal(n).unwrap();
            assert_eq!(kind.ordinal(), n);
        }
        assert!(SymbolKind::from_ordinal(12).is_none());
    }
}
