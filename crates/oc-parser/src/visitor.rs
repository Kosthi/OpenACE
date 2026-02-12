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

/// 解析单个源代码文件，返回提取到的符号和关系。
///
/// # 参数
/// * `repo_id`   - 仓库标识符，用于生成全局唯一的 SymbolId。
/// * `file_path` - 相对于项目根目录的文件路径（使用正斜杠分隔）。
/// * `content`   - 文件的原始 UTF-8 字节内容。
/// * `file_size` - 文件大小（字节），用于预读阶段的大小检查；实际内容长度也会被校验。
pub fn parse_file(
    repo_id: &str,   // 仓库 ID，例如 "github.com/user/repo"
    file_path: &str, // 文件路径，例如 "src/main.rs"
    content: &[u8],  // 文件的原始字节内容
    file_size: u64,  // 文件系统报告的文件大小
) -> Result<ParseOutput, ParserError> {
    // 第一次检查：校验文件系统报告的 file_size 是否超出允许的最大值
    check_file_size(file_path, file_size)?;
    // 第二次检查：校验实际读入内容的长度是否超出允许的最大值
    // (两者可能不一致，例如文件在读取过程中被修改)
    check_file_size(file_path, content.len() as u64)?;

    // 检测文件内容是否为二进制（通过检查是否包含 NULL 字节等特征）
    if is_binary(content) {
        // 二进制文件无法作为源代码解析，返回编码错误
        return Err(ParserError::InvalidEncoding {
            path: file_path.to_string(),
        });
    }

    // 尝试将原始字节转换为 UTF-8 字符串
    // 如果转换失败（非法 UTF-8 序列），返回编码错误
    let source = std::str::from_utf8(content).map_err(|_| ParserError::InvalidEncoding {
        path: file_path.to_string(),
    })?;

    // 从文件路径中提取扩展名（如 "rs"、"py"、"ts"）
    // Path::new 创建路径对象 → .extension() 取扩展名 → .to_str() 转为字符串
    // 如果没有扩展名，则默认为空字符串
    let ext = Path::new(file_path)
        .extension() // 获取 OsStr 类型的扩展名，可能为 None
        .and_then(|e| e.to_str()) // 转换为 &str，非 UTF-8 则为 None
        .unwrap_or(""); // 无扩展名时使用空字符串

    // 根据扩展名查询对应的编程语言枚举值
    // 如果扩展名不被支持（未注册），返回 UnsupportedLanguage 错误
    let language = ParserRegistry::language_for_extension(ext).ok_or_else(|| {
        ParserError::UnsupportedLanguage {
            path: file_path.to_string(),
        }
    })?;

    // 根据语言和扩展名获取对应的 tree-sitter grammar（语法定义）
    let grammar = ParserRegistry::grammar_for_extension(language, ext);
    // 创建一个新的 tree-sitter 解析器实例
    let mut parser = tree_sitter::Parser::new();
    // 为解析器设置目标语言的 grammar
    // 如果语言版本不兼容等原因导致设置失败，返回 ParseFailed 错误
    parser
        .set_language(&grammar)
        .map_err(|e| ParserError::ParseFailed {
            path: file_path.to_string(),
            reason: format!("failed to set language: {e}"),
        })?;

    // 使用 tree-sitter 解析源代码，生成语法树（AST）
    // 第二个参数 None 表示不使用增量解析（没有旧的语法树可复用）
    // 如果解析失败（返回 None），则报告 ParseFailed 错误
    let tree = parser
        .parse(source, None)
        .ok_or_else(|| ParserError::ParseFailed {
            path: file_path.to_string(),
            reason: "tree-sitter returned no tree".to_string(),
        })?;

    // 构造 VisitorContext，将解析所需的上下文信息打包传递给各语言的 visitor
    let ctx = VisitorContext {
        repo_id,   // 仓库标识符
        file_path, // 文件路径
        source,    // 源代码文本
        language,  // 编程语言类型
    };

    // 根据检测到的编程语言，分发到对应的语言 visitor 模块进行符号和关系提取
    match language {
        // Python 文件 → 交给 python 模块处理
        Language::Python => python::extract(&ctx, &tree),
        // TypeScript 或 JavaScript 文件 → 交给 typescript 模块处理（二者共用同一个 visitor）
        Language::TypeScript | Language::JavaScript => typescript::extract(&ctx, &tree),
        // Rust 文件 → 交给 rust_lang 模块处理
        Language::Rust => rust_lang::extract(&ctx, &tree),
        // Go 文件 → 交给 go_lang 模块处理
        Language::Go => go_lang::extract(&ctx, &tree),
        // Java 文件 → 交给 java 模块处理
        Language::Java => java::extract(&ctx, &tree),
    }
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
