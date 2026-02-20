// 引入标准库的 Path 类型，用于从文件路径中提取扩展名
use std::path::Path;

// 从核心库 oc_core 引入：
//   CodeRelation - 代码关系（如函数调用、继承等）
//   CodeSymbol   - 代码符号（如函数、类、方法等）
//   Language     - 支持的编程语言枚举
use oc_core::{CodeRelation, CodeSymbol, Language};

// 引入本 crate 内的 body_hash 模块，用于计算代码片段的哈希值（增量变更检测）
use crate::body_hash::compute_body_hash;
// 引入本 crate 内的错误类型，统一的解析错误枚举
use crate::error::ParserError;
// 引入文件前置检查工具：check_file_size 检查文件是否过大，is_binary 检查是否为二进制文件
use crate::file_check::{check_file_size, is_binary};
// 引入解析器注册表，用于根据文件扩展名查找对应的语言和 tree-sitter grammar
use crate::registry::ParserRegistry;

// 声明各语言专用的 visitor 子模块，每个模块实现各自语言的符号/关系提取逻辑
mod go_lang; // Go 语言 visitor
mod java;
mod python; // Python 语言 visitor
mod rust_lang; // Rust 语言 visitor
mod typescript; // TypeScript / JavaScript 语言 visitor // Java 语言 visitor

/// 单个文件解析后的输出结果。
#[derive(Debug)]
pub struct ParseOutput {
    /// 从源文件中提取到的所有代码符号（函数、类、方法、变量等）
    pub symbols: Vec<CodeSymbol>,
    /// 从源文件中提取到的所有代码关系（调用、继承、导入等）
    pub relations: Vec<CodeRelation>,
}

/// Output from parsing a file that also retains the tree-sitter AST tree.
///
/// Used when both symbol extraction and chunking need the same parse tree,
/// avoiding a redundant re-parse.
pub struct ParseOutputWithTree {
    /// Extracted symbols and relations.
    pub output: ParseOutput,
    /// The source code as a UTF-8 string.
    pub source: String,
    /// The tree-sitter AST.
    pub tree: tree_sitter::Tree,
    /// The detected language.
    pub language: Language,
}

/// Parse a single source file, returning symbols, relations, and the tree-sitter AST.
///
/// This is the extended version of `parse_file()` that also returns the AST tree
/// and source string, enabling downstream consumers (e.g., the chunker) to reuse
/// the parse result without re-parsing.
#[tracing::instrument(skip(content), fields(language, symbol_count))]
pub fn parse_file_with_tree(
    repo_id: &str,
    file_path: &str,
    content: &[u8],
    file_size: u64,
) -> Result<ParseOutputWithTree, ParserError> {
    check_file_size(file_path, file_size)?;
    check_file_size(file_path, content.len() as u64)?;

    if is_binary(content) {
        tracing::warn!(path = %file_path, reason = "binary", "file skipped");
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
    parser
        .set_language(&grammar)
        .map_err(|e| ParserError::ParseFailed {
            path: file_path.to_string(),
            reason: format!("failed to set language: {e}"),
        })?;

    let tree = parser
        .parse(source, None)
        .ok_or_else(|| ParserError::ParseFailed {
            path: file_path.to_string(),
            reason: "tree-sitter returned no tree".to_string(),
        })?;

    let ctx = VisitorContext {
        repo_id,
        file_path,
        source,
        language,
    };

    let output = match language {
        Language::Python => python::extract(&ctx, &tree),
        Language::TypeScript | Language::JavaScript => typescript::extract(&ctx, &tree),
        Language::Rust => rust_lang::extract(&ctx, &tree),
        Language::Go => go_lang::extract(&ctx, &tree),
        Language::Java => java::extract(&ctx, &tree),
    }?;

    let span = tracing::Span::current();
    span.record("language", tracing::field::debug(&language));
    span.record("symbol_count", output.symbols.len());

    Ok(ParseOutputWithTree {
        output,
        source: source.to_string(),
        tree,
        language,
    })
}

/// Parse a single source file, returning extracted symbols and relations.
///
/// This is the standard entry point. If you also need the tree-sitter AST
/// (e.g., for chunking), use `parse_file_with_tree()` instead.
#[tracing::instrument(skip(content), fields(language, symbol_count))]
pub fn parse_file(
    repo_id: &str,
    file_path: &str,
    content: &[u8],
    file_size: u64,
) -> Result<ParseOutput, ParserError> {
    parse_file_with_tree(repo_id, file_path, content, file_size).map(|r| r.output)
}

/// 各语言 visitor 共享的上下文结构体。
/// 包含解析过程中需要的所有公共信息，避免每个 visitor 重复传参。
pub(crate) struct VisitorContext<'a> {
    /// 仓库标识符，用于为提取到的符号生成全局唯一 ID
    pub repo_id: &'a str,
    /// 文件路径（相对于项目根目录），用于填充符号的位置信息
    pub file_path: &'a str,
    /// 源代码文本的引用，visitor 通过它从语法树节点获取对应的代码文本
    pub source: &'a str,
    /// 源文件的编程语言类型
    pub language: Language,
}

impl<'a> VisitorContext<'a> {
    /// 从源代码中提取指定 tree-sitter 节点对应的文本内容。
    ///
    /// # 参数
    /// * `node` - tree-sitter 语法树中的一个节点
    ///
    /// # 返回
    /// 该节点在源代码中对应的文本切片；如果提取失败则返回空字符串。
    pub fn node_text(&self, node: tree_sitter::Node<'_>) -> &str {
        // utf8_text 根据节点的字节范围从源代码中截取文本
        // unwrap_or("") 保证即使截取失败也不会 panic
        node.utf8_text(self.source.as_bytes()).unwrap_or("")
    }

    /// 计算指定 tree-sitter 节点对应代码片段的哈希值。
    /// 用于增量分析：通过比较哈希值判断代码块是否发生变更。
    ///
    /// # 参数
    /// * `node` - tree-sitter 语法树中的一个节点
    ///
    /// # 返回
    /// 该节点对应字节区间的 64 位哈希值。
    pub fn body_hash(&self, node: tree_sitter::Node<'_>) -> u64 {
        // 获取节点在源代码中的起始字节偏移
        let start = node.start_byte();
        // 获取节点在源代码中的结束字节偏移
        let end = node.end_byte();
        // 对 [start, end) 字节区间的内容计算哈希值
        compute_body_hash(&self.source.as_bytes()[start..end])
    }
}
