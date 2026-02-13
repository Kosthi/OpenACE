mod chunk;
mod error;
mod language;
mod qualified_name;
mod relation;
mod symbol;

pub use chunk::{ChunkId, CodeChunk};
pub use error::CoreError;
pub use language::Language;
pub use qualified_name::QualifiedName;
pub use relation::{CodeRelation, RelationKind};
pub use symbol::{CodeSymbol, SymbolId, SymbolKind};

/// Truncate a string to at most `max_bytes` bytes on a valid UTF-8 boundary.
///
/// Returns a sub-slice that is always valid UTF-8 and at most `max_bytes` long.
pub fn truncate_utf8_bytes(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_utf8_bytes_ascii() {
        assert_eq!(truncate_utf8_bytes("hello", 3), "hel");
        assert_eq!(truncate_utf8_bytes("hello", 100), "hello");
        assert_eq!(truncate_utf8_bytes("hello", 0), "");
    }

    #[test]
    fn truncate_utf8_bytes_multibyte() {
        // 'é' is 2 bytes in UTF-8
        assert_eq!(truncate_utf8_bytes("café", 4), "caf");
        assert_eq!(truncate_utf8_bytes("café", 5), "café");
        // '日' is 3 bytes
        assert_eq!(truncate_utf8_bytes("日本語", 3), "日");
        assert_eq!(truncate_utf8_bytes("日本語", 5), "日");
        assert_eq!(truncate_utf8_bytes("日本語", 6), "日本");
    }

    #[test]
    fn truncate_utf8_bytes_emoji() {
        // '🦀' is 4 bytes
        assert_eq!(truncate_utf8_bytes("🦀rust", 3), "");
        assert_eq!(truncate_utf8_bytes("🦀rust", 4), "🦀");
        assert_eq!(truncate_utf8_bytes("🦀rust", 5), "🦀r");
    }
}
