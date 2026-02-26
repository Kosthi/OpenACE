use pyo3::prelude::*;

use oc_core::{CodeRelation, CodeSymbol, RelationKind, SymbolKind};
use oc_indexer::{IndexReport, IncrementalIndexResult};
use oc_retrieval::{CallChainNode, ChunkInfo, FunctionContext, SearchResult};

/// Python-compatible symbol representation.
#[pyclass(frozen)]
#[derive(Clone)]
pub struct PySymbol {
    #[pyo3(get)]
    pub id: String,
    #[pyo3(get)]
    pub name: String,
    #[pyo3(get)]
    pub qualified_name: String,
    #[pyo3(get)]
    pub kind: String,
    #[pyo3(get)]
    pub language: String,
    #[pyo3(get)]
    pub file_path: String,
    #[pyo3(get)]
    pub line_start: u32,
    #[pyo3(get)]
    pub line_end: u32,
    #[pyo3(get)]
    pub signature: Option<String>,
    #[pyo3(get)]
    pub doc_comment: Option<String>,
    #[pyo3(get)]
    pub body_text: Option<String>,
}

#[pymethods]
impl PySymbol {
    fn __repr__(&self) -> String {
        format!(
            "Symbol(name={:?}, kind={:?}, file={:?})",
            self.name, self.kind, self.file_path
        )
    }
}

impl From<CodeSymbol> for PySymbol {
    fn from(sym: CodeSymbol) -> Self {
        Self {
            id: format!("{}", sym.id),
            name: sym.name,
            qualified_name: sym.qualified_name,
            kind: kind_to_string(sym.kind),
            language: sym.language.name().to_string(),
            file_path: sym.file_path.to_string_lossy().into_owned(),
            line_start: sym.line_range.start,
            line_end: sym.line_range.end,
            signature: sym.signature,
            doc_comment: sym.doc_comment,
            body_text: sym.body_text,
        }
    }
}

/// Python-compatible search result.
#[pyclass(frozen)]
#[derive(Clone)]
pub struct PySearchResult {
    #[pyo3(get)]
    pub symbol_id: String,
    #[pyo3(get)]
    pub name: String,
    #[pyo3(get)]
    pub qualified_name: String,
    #[pyo3(get)]
    pub kind: String,
    #[pyo3(get)]
    pub file_path: String,
    #[pyo3(get)]
    pub line_range: (u32, u32),
    #[pyo3(get)]
    pub score: f64,
    #[pyo3(get)]
    pub match_signals: Vec<String>,
    #[pyo3(get)]
    pub related_symbols: Vec<PySearchResult>,
    #[pyo3(get)]
    pub snippet: Option<String>,
    #[pyo3(get)]
    pub chunk_info: Option<PyChunkInfo>,
}

#[pymethods]
impl PySearchResult {
    fn __repr__(&self) -> String {
        format!(
            "SearchResult(name={:?}, score={:.4}, signals={:?})",
            self.name, self.score, self.match_signals
        )
    }
}

impl From<SearchResult> for PySearchResult {
    fn from(r: SearchResult) -> Self {
        Self {
            symbol_id: format!("{}", r.symbol_id),
            name: r.name,
            qualified_name: r.qualified_name,
            kind: kind_to_string(r.kind),
            file_path: r.file_path,
            line_range: r.line_range,
            score: r.score,
            match_signals: r.match_signals,
            related_symbols: r.related_symbols.into_iter().map(PySearchResult::from).collect(),
            snippet: r.snippet,
            chunk_info: r.chunk_info.map(PyChunkInfo::from),
        }
    }
}

/// Python-compatible chunk info.
#[pyclass(frozen)]
#[derive(Clone)]
pub struct PyChunkInfo {
    #[pyo3(get)]
    pub context_path: String,
    #[pyo3(get)]
    pub chunk_score: f32,
}

#[pymethods]
impl PyChunkInfo {
    fn __repr__(&self) -> String {
        format!(
            "ChunkInfo(context={:?}, score={:.4})",
            self.context_path, self.chunk_score
        )
    }
}

impl From<ChunkInfo> for PyChunkInfo {
    fn from(c: ChunkInfo) -> Self {
        Self {
            context_path: c.context_path,
            chunk_score: c.chunk_score,
        }
    }
}

/// Python-compatible chunk data for embedding backfill.
#[pyclass(frozen)]
#[derive(Clone)]
pub struct PyChunkData {
    #[pyo3(get)]
    pub id: String,
    #[pyo3(get)]
    pub file_path: String,
    #[pyo3(get)]
    pub context_path: String,
    #[pyo3(get)]
    pub content: String,
}

#[pymethods]
impl PyChunkData {
    fn __repr__(&self) -> String {
        format!(
            "ChunkData(file={:?}, context={:?})",
            self.file_path, self.context_path
        )
    }
}

/// Python-compatible index report.
#[pyclass(frozen)]
#[derive(Clone)]
pub struct PyIndexReport {
    #[pyo3(get)]
    pub total_files_scanned: usize,
    #[pyo3(get)]
    pub files_indexed: usize,
    #[pyo3(get)]
    pub files_skipped: usize,
    #[pyo3(get)]
    pub files_failed: usize,
    #[pyo3(get)]
    pub total_symbols: usize,
    #[pyo3(get)]
    pub total_relations: usize,
    #[pyo3(get)]
    pub relations_resolved: usize,
    #[pyo3(get)]
    pub relations_unresolved: usize,
    #[pyo3(get)]
    pub total_chunks: usize,
    #[pyo3(get)]
    pub duration_secs: f64,
}

#[pymethods]
impl PyIndexReport {
    fn __repr__(&self) -> String {
        format!(
            "IndexReport(files={}, symbols={}, relations={}, resolved={}, chunks={}, duration={:.2}s)",
            self.files_indexed, self.total_symbols, self.total_relations,
            self.relations_resolved, self.total_chunks, self.duration_secs
        )
    }
}

impl From<IndexReport> for PyIndexReport {
    fn from(r: IndexReport) -> Self {
        Self {
            total_files_scanned: r.total_files_scanned,
            files_indexed: r.files_indexed,
            files_skipped: r.total_skipped(),
            files_failed: r.files_failed,
            total_symbols: r.total_symbols,
            total_relations: r.total_relations,
            relations_resolved: r.relations_resolved,
            relations_unresolved: r.relations_unresolved,
            total_chunks: r.total_chunks,
            duration_secs: r.duration.as_secs_f64(),
        }
    }
}

/// Python-compatible incremental index result.
#[pyclass(frozen)]
#[derive(Clone)]
pub struct PyIncrementalIndexResult {
    #[pyo3(get)]
    pub total_files_scanned: usize,
    #[pyo3(get)]
    pub files_indexed: usize,
    #[pyo3(get)]
    pub files_unchanged: usize,
    #[pyo3(get)]
    pub files_deleted: usize,
    #[pyo3(get)]
    pub files_skipped: usize,
    #[pyo3(get)]
    pub files_failed: usize,
    #[pyo3(get)]
    pub total_symbols: usize,
    #[pyo3(get)]
    pub total_relations: usize,
    #[pyo3(get)]
    pub total_chunks: usize,
    #[pyo3(get)]
    pub duration_secs: f64,
    #[pyo3(get)]
    pub changed_symbol_ids: Vec<String>,
    #[pyo3(get)]
    pub removed_symbol_ids: Vec<String>,
    #[pyo3(get)]
    pub fell_back_to_full: bool,
}

#[pymethods]
impl PyIncrementalIndexResult {
    fn __repr__(&self) -> String {
        format!(
            "IncrementalIndexResult(indexed={}, unchanged={}, deleted={}, \
             changed_symbols={}, removed_symbols={}, fell_back={})",
            self.files_indexed, self.files_unchanged, self.files_deleted,
            self.changed_symbol_ids.len(), self.removed_symbol_ids.len(),
            self.fell_back_to_full
        )
    }
}

impl From<IncrementalIndexResult> for PyIncrementalIndexResult {
    fn from(r: IncrementalIndexResult) -> Self {
        Self {
            total_files_scanned: r.report.total_files_scanned,
            files_indexed: r.report.files_indexed,
            files_unchanged: r.files_unchanged,
            files_deleted: r.files_deleted,
            files_skipped: r.report.total_skipped(),
            files_failed: r.report.files_failed,
            total_symbols: r.report.total_symbols,
            total_relations: r.report.total_relations,
            total_chunks: r.report.total_chunks,
            duration_secs: r.report.duration.as_secs_f64(),
            changed_symbol_ids: r
                .changed_symbol_ids
                .iter()
                .map(|id| format!("{}", id))
                .collect(),
            removed_symbol_ids: r
                .removed_symbol_ids
                .iter()
                .map(|id| format!("{}", id))
                .collect(),
            fell_back_to_full: r.fell_back_to_full,
        }
    }
}

/// Python-compatible relation.
#[pyclass(frozen)]
#[derive(Clone)]
pub struct PyRelation {
    #[pyo3(get)]
    pub source_id: String,
    #[pyo3(get)]
    pub target_id: String,
    #[pyo3(get)]
    pub kind: String,
    #[pyo3(get)]
    pub file_path: String,
    #[pyo3(get)]
    pub line: u32,
    #[pyo3(get)]
    pub confidence: f32,
}

#[pymethods]
impl PyRelation {
    fn __repr__(&self) -> String {
        format!(
            "Relation(kind={:?}, {}->{})",
            self.kind, self.source_id, self.target_id
        )
    }
}

impl From<CodeRelation> for PyRelation {
    fn from(r: CodeRelation) -> Self {
        Self {
            source_id: format!("{}", r.source_id),
            target_id: format!("{}", r.target_id),
            kind: relation_kind_to_string(r.kind),
            file_path: r.file_path.to_string_lossy().into_owned(),
            line: r.line,
            confidence: r.confidence,
        }
    }
}

/// Python-compatible file info for summary generation.
#[pyclass(frozen)]
#[derive(Clone)]
pub struct PyFileInfo {
    #[pyo3(get)]
    pub path: String,
    #[pyo3(get)]
    pub language: String,
    #[pyo3(get)]
    pub symbol_count: u32,
}

#[pymethods]
impl PyFileInfo {
    fn __repr__(&self) -> String {
        format!(
            "FileInfo(path={:?}, language={:?}, symbols={})",
            self.path, self.language, self.symbol_count
        )
    }
}

/// Python-compatible summary chunk input for upsert.
#[pyclass(frozen)]
#[derive(Clone)]
pub struct PySummaryChunk {
    #[pyo3(get)]
    pub file_path: String,
    #[pyo3(get)]
    pub language: String,
    #[pyo3(get)]
    pub content: String,
}

#[pymethods]
impl PySummaryChunk {
    #[new]
    fn new(file_path: String, language: String, content: String) -> Self {
        Self {
            file_path,
            language,
            content,
        }
    }

    fn __repr__(&self) -> String {
        format!(
            "SummaryChunk(file={:?}, language={:?})",
            self.file_path, self.language
        )
    }
}

/// Python-compatible call chain node.
#[pyclass(frozen)]
#[derive(Clone)]
pub struct PyCallChainNode {
    #[pyo3(get)]
    pub symbol_id: String,
    #[pyo3(get)]
    pub name: String,
    #[pyo3(get)]
    pub qualified_name: String,
    #[pyo3(get)]
    pub kind: String,
    #[pyo3(get)]
    pub file_path: String,
    #[pyo3(get)]
    pub line_range: (u32, u32),
    #[pyo3(get)]
    pub depth: u32,
    #[pyo3(get)]
    pub signature: Option<String>,
    #[pyo3(get)]
    pub doc_comment: Option<String>,
    #[pyo3(get)]
    pub snippet: Option<String>,
}

#[pymethods]
impl PyCallChainNode {
    fn __repr__(&self) -> String {
        format!(
            "CallChainNode(name={:?}, kind={:?}, depth={}, file={:?})",
            self.name, self.kind, self.depth, self.file_path
        )
    }
}

impl From<CallChainNode> for PyCallChainNode {
    fn from(n: CallChainNode) -> Self {
        Self {
            symbol_id: format!("{}", n.symbol_id),
            name: n.name,
            qualified_name: n.qualified_name,
            kind: kind_to_string(n.kind),
            file_path: n.file_path,
            line_range: n.line_range,
            depth: n.depth,
            signature: n.signature,
            doc_comment: n.doc_comment,
            snippet: n.snippet,
        }
    }
}

/// Python-compatible function context.
#[pyclass(frozen)]
#[derive(Clone)]
pub struct PyFunctionContext {
    #[pyo3(get)]
    pub symbol: PyCallChainNode,
    #[pyo3(get)]
    pub callers: Vec<PyCallChainNode>,
    #[pyo3(get)]
    pub callees: Vec<PyCallChainNode>,
    #[pyo3(get)]
    pub hierarchy: Vec<PyCallChainNode>,
}

#[pymethods]
impl PyFunctionContext {
    fn __repr__(&self) -> String {
        format!(
            "FunctionContext(symbol={:?}, callers={}, callees={}, hierarchy={})",
            self.symbol.name, self.callers.len(), self.callees.len(), self.hierarchy.len()
        )
    }
}

impl From<FunctionContext> for PyFunctionContext {
    fn from(ctx: FunctionContext) -> Self {
        Self {
            symbol: PyCallChainNode::from(ctx.symbol),
            callers: ctx.callers.into_iter().map(PyCallChainNode::from).collect(),
            callees: ctx.callees.into_iter().map(PyCallChainNode::from).collect(),
            hierarchy: ctx.hierarchy.into_iter().map(PyCallChainNode::from).collect(),
        }
    }
}

fn kind_to_string(kind: SymbolKind) -> String {
    match kind {
        SymbolKind::Function => "function",
        SymbolKind::Method => "method",
        SymbolKind::Class => "class",
        SymbolKind::Struct => "struct",
        SymbolKind::Interface => "interface",
        SymbolKind::Trait => "trait",
        SymbolKind::Module => "module",
        SymbolKind::Package => "package",
        SymbolKind::Variable => "variable",
        SymbolKind::Constant => "constant",
        SymbolKind::Enum => "enum",
        SymbolKind::TypeAlias => "type_alias",
    }
    .to_string()
}

fn relation_kind_to_string(kind: RelationKind) -> String {
    match kind {
        RelationKind::Calls => "calls",
        RelationKind::Imports => "imports",
        RelationKind::Inherits => "inherits",
        RelationKind::Implements => "implements",
        RelationKind::Uses => "uses",
        RelationKind::Contains => "contains",
    }
    .to_string()
}
