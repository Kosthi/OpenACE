"""OpenACE data types."""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Optional


@dataclass(frozen=True)
class Symbol:
    """A code symbol extracted from a source file."""
    id: str
    name: str
    qualified_name: str
    kind: str
    language: str
    file_path: str
    line_start: int
    line_end: int
    signature: Optional[str] = None
    doc_comment: Optional[str] = None

    def __repr__(self) -> str:
        return f"Symbol(name={self.name!r}, kind={self.kind!r}, file={self.file_path!r})"


@dataclass(frozen=True)
class ChunkInfo:
    """Information about a chunk that boosted a search result."""
    context_path: str
    chunk_score: float

    def __repr__(self) -> str:
        return f"ChunkInfo(context={self.context_path!r}, score={self.chunk_score:.4f})"


@dataclass(frozen=True)
class SearchResult:
    """A search result with relevance score and provenance."""
    symbol_id: str
    name: str
    qualified_name: str
    kind: str
    file_path: str
    line_range: tuple[int, int]
    score: float
    rerank_score: Optional[float] = None
    match_signals: list[str] = field(default_factory=list)
    related_symbols: list[SearchResult] = field(default_factory=list)
    snippet: Optional[str] = None
    chunk_info: Optional[ChunkInfo] = None

    def __repr__(self) -> str:
        return (
            f"SearchResult(name={self.name!r}, score={self.score:.4f}, "
            f"signals={self.match_signals})"
        )


@dataclass(frozen=True)
class IndexReport:
    """Report from an indexing run."""
    total_files_scanned: int
    files_indexed: int
    files_skipped: int
    files_failed: int
    total_symbols: int
    total_relations: int
    duration_secs: float
    total_chunks: int = 0
    relations_resolved: int = 0
    relations_unresolved: int = 0

    def __repr__(self) -> str:
        return (
            f"IndexReport(files={self.files_indexed}, symbols={self.total_symbols}, "
            f"relations={self.total_relations}, resolved={self.relations_resolved}, "
            f"chunks={self.total_chunks}, duration={self.duration_secs:.2f}s)"
        )


@dataclass(frozen=True)
class Relation:
    """A relationship between two code symbols."""
    source_id: str
    target_id: str
    kind: str
    file_path: str
    line: int
    confidence: float


@dataclass(frozen=True)
class IncrementalIndexReport:
    """Report from an incremental indexing run."""
    total_files_scanned: int
    files_indexed: int
    files_unchanged: int
    files_deleted: int
    files_skipped: int
    files_failed: int
    total_symbols: int
    total_relations: int
    total_chunks: int
    duration_secs: float
    changed_symbol_ids: list[str] = field(default_factory=list)
    removed_symbol_ids: list[str] = field(default_factory=list)
    fell_back_to_full: bool = False

    def __repr__(self) -> str:
        return (
            f"IncrementalIndexReport(indexed={self.files_indexed}, "
            f"unchanged={self.files_unchanged}, deleted={self.files_deleted}, "
            f"changed_symbols={len(self.changed_symbol_ids)}, "
            f"fell_back={self.fell_back_to_full})"
        )


@dataclass(frozen=True)
class CallChainNode:
    """A node in a call chain traversal result."""
    symbol_id: str
    name: str
    qualified_name: str
    kind: str
    file_path: str
    line_range: tuple[int, int]
    depth: int
    signature: Optional[str] = None
    doc_comment: Optional[str] = None
    snippet: Optional[str] = None

    def __repr__(self) -> str:
        return (
            f"CallChainNode(name={self.name!r}, kind={self.kind!r}, "
            f"depth={self.depth}, file={self.file_path!r})"
        )


@dataclass(frozen=True)
class FunctionContext:
    """Structured function context: callers, callees, and hierarchy around a symbol."""
    symbol: CallChainNode
    callers: list[CallChainNode] = field(default_factory=list)
    callees: list[CallChainNode] = field(default_factory=list)
    hierarchy: list[CallChainNode] = field(default_factory=list)

    def __repr__(self) -> str:
        return (
            f"FunctionContext(symbol={self.symbol.name!r}, "
            f"callers={len(self.callers)}, callees={len(self.callees)}, "
            f"hierarchy={len(self.hierarchy)})"
        )
