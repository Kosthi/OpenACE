use std::path::Path;

use oc_core::{CodeRelation, CodeSymbol, Language};

use crate::body_hash::compute_body_hash;
use crate::error::ParserError;
use crate::file_check::{check_file_size, is_binary};
use crate::registry::ParserRegistry;

mod python;
mod typescript;
mod rust_lang;
mod go_lang;
mod java;

/// Output of parsing a single file.
#[derive(Debug)]
pub struct ParseOutput {
    pub symbols: Vec<CodeSymbol>,
    pub relations: Vec<CodeRelation>,
}

/// Parse a single source file, returning extracted symbols and relations.
///
/// # Arguments
/// * `repo_id` - Repository identifier for SymbolId generation.
/// * `file_path` - Path relative to project root (forward-slash normalized).
/// * `content` - Raw UTF-8 source bytes.
/// * `file_size` - Size in bytes (for pre-read size check; content.len() is also checked).
pub fn parse_file(
    repo_id: &str,
    file_path: &str,
    content: &[u8],
    file_size: u64,
) -> Result<ParseOutput, ParserError> {
    // Check both declared file_size and actual content length
    check_file_size(file_path, file_size)?;
    check_file_size(file_path, content.len() as u64)?;

    if is_binary(content) {
        return Err(ParserError::InvalidEncoding {
            path: file_path.to_string(),
        });
    }

    let source = std::str::from_utf8(content).map_err(|_| ParserError::InvalidEncoding {
        path: file_path.to_string(),
    })?;

    let ext = Path::new(file_path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");

    let language = ParserRegistry::language_for_extension(ext).ok_or_else(|| {
        ParserError::UnsupportedLanguage {
            path: file_path.to_string(),
        }
    })?;

    let grammar = ParserRegistry::grammar_for_extension(language, ext);
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&grammar).map_err(|e| {
        ParserError::ParseFailed {
            path: file_path.to_string(),
            reason: format!("failed to set language: {e}"),
        }
    })?;

    let tree = parser.parse(source, None).ok_or_else(|| {
        ParserError::ParseFailed {
            path: file_path.to_string(),
            reason: "tree-sitter returned no tree".to_string(),
        }
    })?;

    let ctx = VisitorContext {
        repo_id,
        file_path,
        source,
        language,
    };

    match language {
        Language::Python => python::extract(&ctx, &tree),
        Language::TypeScript | Language::JavaScript => typescript::extract(&ctx, &tree),
        Language::Rust => rust_lang::extract(&ctx, &tree),
        Language::Go => go_lang::extract(&ctx, &tree),
        Language::Java => java::extract(&ctx, &tree),
    }
}

/// Shared context passed to language visitors.
pub(crate) struct VisitorContext<'a> {
    pub repo_id: &'a str,
    pub file_path: &'a str,
    pub source: &'a str,
    pub language: Language,
}

impl<'a> VisitorContext<'a> {
    /// Extract the text of a tree-sitter node from source.
    pub fn node_text(&self, node: tree_sitter::Node<'_>) -> &str {
        node.utf8_text(self.source.as_bytes()).unwrap_or("")
    }

    /// Compute body_hash for a node's byte range.
    pub fn body_hash(&self, node: tree_sitter::Node<'_>) -> u64 {
        let start = node.start_byte();
        let end = node.end_byte();
        compute_body_hash(&self.source.as_bytes()[start..end])
    }
}
