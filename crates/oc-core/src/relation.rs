use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::symbol::SymbolId;

/// Kinds of relationships between code symbols.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum RelationKind {
    Calls = 0,
    Imports = 1,
    Inherits = 2,
    Implements = 3,
    Uses = 4,
    Contains = 5,
}

impl RelationKind {
    /// Fixed confidence score for tree-sitter extracted relations.
    pub fn default_confidence(self) -> f32 {
        match self {
            Self::Calls => 0.8,
            Self::Imports => 0.9,
            Self::Inherits => 0.85,
            Self::Implements => 0.85,
            Self::Uses => 0.7,
            Self::Contains => 0.95,
        }
    }

    pub fn from_ordinal(n: u8) -> Option<Self> {
        match n {
            0 => Some(Self::Calls),
            1 => Some(Self::Imports),
            2 => Some(Self::Inherits),
            3 => Some(Self::Implements),
            4 => Some(Self::Uses),
            5 => Some(Self::Contains),
            _ => None,
        }
    }

    pub fn ordinal(self) -> u8 {
        self as u8
    }
}

/// A relationship between two code symbols.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeRelation {
    pub source_id: SymbolId,
    pub target_id: SymbolId,
    pub kind: RelationKind,
    /// File where the relation was observed.
    pub file_path: PathBuf,
    /// 0-indexed line number.
    pub line: u32,
    pub confidence: f32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confidence_constants() {
        assert!((RelationKind::Calls.default_confidence() - 0.8).abs() < f32::EPSILON);
        assert!((RelationKind::Imports.default_confidence() - 0.9).abs() < f32::EPSILON);
        assert!((RelationKind::Inherits.default_confidence() - 0.85).abs() < f32::EPSILON);
        assert!((RelationKind::Implements.default_confidence() - 0.85).abs() < f32::EPSILON);
        assert!((RelationKind::Uses.default_confidence() - 0.7).abs() < f32::EPSILON);
        assert!((RelationKind::Contains.default_confidence() - 0.95).abs() < f32::EPSILON);
    }

    #[test]
    fn relation_kind_ordinal_round_trip() {
        for n in 0..=5u8 {
            let kind = RelationKind::from_ordinal(n).unwrap();
            assert_eq!(kind.ordinal(), n);
        }
        assert!(RelationKind::from_ordinal(6).is_none());
    }
}
