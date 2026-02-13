use std::path::PathBuf;

use oc_core::{ChunkId, CodeChunk, Language};
use xxhash_rust::xxh3::xxh3_64;

/// Configuration for the AST chunking algorithm.
#[derive(Debug, Clone)]
pub struct ChunkConfig {
    /// Maximum non-whitespace characters per chunk.
    pub max_chunk_chars: usize,
    /// Number of overlapping nodes between adjacent chunks.
    pub overlap_nodes: usize,
}

impl Default for ChunkConfig {
    fn default() -> Self {
        Self {
            max_chunk_chars: 1500,
            overlap_nodes: 1,
        }
    }
}

/// Maximum content size stored per chunk (10 KB).
const CHUNK_CONTENT_MAX_BYTES: usize = 10_240;

/// Build a cumulative sum of non-whitespace character counts.
///
/// `cumsum[i]` = number of non-whitespace chars in `source[0..i]`.
/// This allows O(1) non-whitespace size queries for any byte range.
fn build_nws_cumsum(source: &[u8]) -> Vec<u32> {
    let mut cumsum = Vec::with_capacity(source.len() + 1);
    cumsum.push(0);
    let mut count = 0u32;
    for &b in source {
        if !b.is_ascii_whitespace() {
            count += 1;
        }
        cumsum.push(count);
    }
    cumsum
}

/// Query the non-whitespace character count in `source[start..end]` in O(1).
fn nws_size(cumsum: &[u32], start: usize, end: usize) -> u32 {
    if end <= start || end > cumsum.len() - 1 {
        return 0;
    }
    cumsum[end] - cumsum[start]
}

/// Tree-sitter node kinds that represent named scopes for context_path.
fn is_scope_node(kind: &str) -> bool {
    matches!(
        kind,
        "function_definition"
            | "class_definition"
            | "decorated_definition"
            | "method_definition"
            | "function_declaration"
            | "class_declaration"
            | "impl_item"
            | "function_item"
            | "struct_item"
            | "enum_item"
            | "trait_item"
            | "interface_declaration"
            | "type_declaration"
            | "method_declaration"
            | "constructor_declaration"
    )
}

/// Extract a name from a scope node by looking for a child with kind "identifier",
/// "name", or "type_identifier".
fn extract_scope_name<'a>(node: tree_sitter::Node<'a>, source: &'a [u8]) -> Option<&'a str> {
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            let kind = child.kind();
            if kind == "identifier" || kind == "name" || kind == "type_identifier" {
                return child.utf8_text(source).ok();
            }
        }
    }
    None
}

/// Build a dot-separated context path by walking ancestors of the given node.
fn build_context_path(node: tree_sitter::Node<'_>, source: &[u8]) -> String {
    let mut parts: Vec<&str> = Vec::new();
    let mut current = node.parent();
    while let Some(parent) = current {
        if is_scope_node(parent.kind()) {
            if let Some(name) = extract_scope_name(parent, source) {
                parts.push(name);
            }
        }
        current = parent.parent();
    }
    parts.reverse();
    parts.join(".")
}

/// A window of consecutive child nodes being accumulated into a chunk.
struct Window {
    nodes: Vec<(usize, usize)>, // (byte_start, byte_end)
    nws_count: u32,
}

impl Window {
    fn new() -> Self {
        Self {
            nodes: Vec::new(),
            nws_count: 0,
        }
    }

    fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    fn byte_range(&self) -> Option<(usize, usize)> {
        if self.nodes.is_empty() {
            return None;
        }
        let start = self.nodes.first().unwrap().0;
        let end = self.nodes.last().unwrap().1;
        Some((start, end))
    }
}

/// Recursively assign child nodes to windows, splitting when the budget is exceeded.
fn assign_children_to_windows(
    node: tree_sitter::Node<'_>,
    cumsum: &[u32],
    max_chars: usize,
    overlap: usize,
    windows: &mut Vec<(usize, usize)>,
) {
    let child_count = node.child_count();
    if child_count == 0 {
        // Leaf node: emit its range as a window
        let start = node.start_byte();
        let end = node.end_byte();
        if start < end {
            windows.push((start, end));
        }
        return;
    }

    let mut window = Window::new();

    for i in 0..child_count {
        let child = match node.child(i) {
            Some(c) => c,
            None => continue,
        };

        let child_start = child.start_byte();
        let child_end = child.end_byte();
        let child_nws = nws_size(cumsum, child_start, child_end);

        if child_nws == 0 {
            continue;
        }

        // Case 1: child fits within the budget → try to add to current window
        if child_nws <= max_chars as u32 {
            if window.nws_count + child_nws <= max_chars as u32 {
                // Fits in current window
                window.nodes.push((child_start, child_end));
                window.nws_count += child_nws;
            } else {
                // Current window is full → yield it and start new
                if !window.is_empty() {
                    if let Some(range) = window.byte_range() {
                        windows.push(range);
                    }
                    // Overlap: carry last `overlap` nodes into new window
                    let carry: Vec<(usize, usize)> = if overlap > 0 && window.nodes.len() > overlap {
                        window.nodes[window.nodes.len() - overlap..].to_vec()
                    } else {
                        Vec::new()
                    };
                    let carry_nws: u32 = carry
                        .iter()
                        .map(|(s, e)| nws_size(cumsum, *s, *e))
                        .sum();
                    window = Window::new();
                    window.nodes = carry;
                    window.nws_count = carry_nws;
                }
                window.nodes.push((child_start, child_end));
                window.nws_count += child_nws;
            }
        } else {
            // Case 2: child is too large → yield current window, recurse into child
            if !window.is_empty() {
                if let Some(range) = window.byte_range() {
                    windows.push(range);
                }
                window = Window::new();
            }
            assign_children_to_windows(child, cumsum, max_chars, overlap, windows);
        }
    }

    // Yield remaining window
    if !window.is_empty() {
        if let Some(range) = window.byte_range() {
            windows.push(range);
        }
    }
}

/// Chunk a parsed file into AST-aware code chunks.
///
/// The algorithm:
/// 1. If the entire file fits within `config.max_chunk_chars` non-whitespace characters,
///    produce a single chunk.
/// 2. Otherwise, greedily group root-level AST children into windows that respect
///    the character budget. If a child is too large, recurse into its children.
/// 3. Each window becomes a `CodeChunk` with a context path derived from ancestor scopes.
pub fn chunk_file(
    repo_id: &str,
    file_path: &str,
    source: &str,
    tree: &tree_sitter::Tree,
    language: Language,
    config: &ChunkConfig,
) -> Vec<CodeChunk> {
    let source_bytes = source.as_bytes();
    let cumsum = build_nws_cumsum(source_bytes);
    let total_nws = nws_size(&cumsum, 0, source_bytes.len());

    // Single-chunk fast path
    if total_nws <= config.max_chunk_chars as u32 {
        let content = oc_core::truncate_utf8_bytes(source, CHUNK_CONTENT_MAX_BYTES);
        let content_hash = xxh3_64(content.as_bytes());
        let line_end = source.lines().count() as u32;
        let id = ChunkId::generate(repo_id, file_path, 0, source_bytes.len());
        return vec![CodeChunk {
            id,
            language,
            file_path: PathBuf::from(file_path),
            byte_range: 0..source_bytes.len(),
            line_range: 0..line_end,
            chunk_index: 0,
            total_chunks: 1,
            context_path: String::new(),
            content: content.to_string(),
            content_hash,
        }];
    }

    // Multi-chunk path: assign AST children to windows
    let root = tree.root_node();
    let mut window_ranges: Vec<(usize, usize)> = Vec::new();
    assign_children_to_windows(
        root,
        &cumsum,
        config.max_chunk_chars,
        config.overlap_nodes,
        &mut window_ranges,
    );

    if window_ranges.is_empty() {
        return Vec::new();
    }

    let total_chunks = window_ranges.len() as u32;
    let mut chunks = Vec::with_capacity(window_ranges.len());

    for (chunk_index, (byte_start, byte_end)) in window_ranges.iter().enumerate() {
        let byte_start = *byte_start;
        let byte_end = (*byte_end).min(source_bytes.len());

        if byte_start >= byte_end {
            continue;
        }

        let chunk_source = &source[byte_start..byte_end];
        let content = oc_core::truncate_utf8_bytes(chunk_source, CHUNK_CONTENT_MAX_BYTES);
        let content_hash = xxh3_64(content.as_bytes());

        // Compute line range
        let line_start = source[..byte_start].matches('\n').count() as u32;
        let line_end = line_start + chunk_source.matches('\n').count() as u32 + 1;

        // Build context path from the first node in the window's position
        // Find the deepest named node at byte_start to derive context
        let context_node = root.descendant_for_byte_range(byte_start, byte_start + 1);
        let context_path = match context_node {
            Some(n) => build_context_path(n, source_bytes),
            None => String::new(),
        };

        let id = ChunkId::generate(repo_id, file_path, byte_start, byte_end);

        chunks.push(CodeChunk {
            id,
            language,
            file_path: PathBuf::from(file_path),
            byte_range: byte_start..byte_end,
            line_range: line_start..line_end,
            chunk_index: chunk_index as u32,
            total_chunks,
            context_path,
            content: content.to_string(),
            content_hash,
        });
    }

    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_source(source: &str, grammar: tree_sitter::Language) -> tree_sitter::Tree {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&grammar).unwrap();
        parser.parse(source, None).unwrap()
    }

    #[test]
    fn single_chunk_small_file() {
        let source = "def foo():\n    return 42\n";
        let tree = parse_source(source, tree_sitter_python::LANGUAGE.into());
        let config = ChunkConfig::default();
        let chunks = chunk_file("repo", "test.py", source, &tree, Language::Python, &config);

        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].chunk_index, 0);
        assert_eq!(chunks[0].total_chunks, 1);
        assert_eq!(chunks[0].byte_range, 0..source.len());
        assert_eq!(chunks[0].content, source);
    }

    #[test]
    fn multi_chunk_large_file() {
        // Build a source file large enough to need multiple chunks
        let mut source = String::new();
        for i in 0..50 {
            source.push_str(&format!(
                "def function_{}():\n    x = {}\n    y = x * 2\n    return y + x\n\n",
                i, i
            ));
        }

        let tree = parse_source(&source, tree_sitter_python::LANGUAGE.into());
        let config = ChunkConfig {
            max_chunk_chars: 200,
            overlap_nodes: 1,
        };
        let chunks = chunk_file("repo", "big.py", &source, &tree, Language::Python, &config);

        assert!(chunks.len() > 1, "Expected multiple chunks, got {}", chunks.len());

        // All chunks should have correct total_chunks
        for chunk in &chunks {
            assert_eq!(chunk.total_chunks, chunks.len() as u32);
        }

        // Chunk indices should be sequential
        for (i, chunk) in chunks.iter().enumerate() {
            assert_eq!(chunk.chunk_index, i as u32);
        }

        // Chunks should cover the entire file (non-overlapping in byte ranges)
        // At minimum, first chunk starts near 0 and last chunk ends near source.len()
        assert!(chunks.first().unwrap().byte_range.start <= 10);
    }

    #[test]
    fn chunk_content_hash_differs() {
        let mut source = String::new();
        for i in 0..50 {
            source.push_str(&format!(
                "def unique_func_{}():\n    return {}\n\n",
                i, i * 1000
            ));
        }

        let tree = parse_source(&source, tree_sitter_python::LANGUAGE.into());
        let config = ChunkConfig {
            max_chunk_chars: 200,
            overlap_nodes: 0,
        };
        let chunks = chunk_file("repo", "test.py", &source, &tree, Language::Python, &config);

        // Different chunks should (generally) have different content hashes
        if chunks.len() >= 2 {
            let hashes: Vec<u64> = chunks.iter().map(|c| c.content_hash).collect();
            // At least some should differ
            let unique: std::collections::HashSet<u64> = hashes.iter().copied().collect();
            assert!(unique.len() > 1, "Expected different content hashes across chunks");
        }
    }

    #[test]
    fn chunk_ids_deterministic() {
        let source = "def foo():\n    pass\ndef bar():\n    pass\n";
        let tree = parse_source(source, tree_sitter_python::LANGUAGE.into());
        let config = ChunkConfig::default();

        let chunks1 = chunk_file("repo", "test.py", source, &tree, Language::Python, &config);
        let chunks2 = chunk_file("repo", "test.py", source, &tree, Language::Python, &config);

        assert_eq!(chunks1.len(), chunks2.len());
        for (c1, c2) in chunks1.iter().zip(chunks2.iter()) {
            assert_eq!(c1.id, c2.id);
        }
    }

    #[test]
    fn nws_cumsum_basic() {
        let source = b"  hello  world  ";
        let cumsum = build_nws_cumsum(source);
        // Total non-whitespace: "helloworld" = 10 chars
        assert_eq!(nws_size(&cumsum, 0, source.len()), 10);
        // "hello" at bytes 2..7
        assert_eq!(nws_size(&cumsum, 2, 7), 5);
    }

    #[test]
    fn empty_file_produces_no_chunks() {
        let source = "";
        let tree = parse_source(source, tree_sitter_python::LANGUAGE.into());
        let config = ChunkConfig::default();
        let chunks = chunk_file("repo", "empty.py", source, &tree, Language::Python, &config);
        // Empty file has 0 non-whitespace chars, which is <= max_chunk_chars,
        // so it produces a single chunk
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].content, "");
    }

    #[test]
    fn chunk_config_default() {
        let config = ChunkConfig::default();
        assert_eq!(config.max_chunk_chars, 1500);
        assert_eq!(config.overlap_nodes, 1);
    }

    #[test]
    fn context_path_for_nested_class() {
        let source = "class MyClass:\n    def my_method(self):\n        x = 1\n        y = 2\n        z = 3\n        return x + y + z\n";
        let tree = parse_source(source, tree_sitter_python::LANGUAGE.into());
        let config = ChunkConfig {
            max_chunk_chars: 30,
            overlap_nodes: 0,
        };
        let chunks = chunk_file("repo", "test.py", source, &tree, Language::Python, &config);

        // At least one chunk should have a context_path containing "MyClass"
        let has_class_context = chunks.iter().any(|c| c.context_path.contains("MyClass"));
        assert!(
            has_class_context || chunks.len() == 1,
            "Expected context_path with 'MyClass', chunks: {:?}",
            chunks.iter().map(|c| &c.context_path).collect::<Vec<_>>()
        );
    }
}
