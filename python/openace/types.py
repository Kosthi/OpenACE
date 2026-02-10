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

    def __repr__(self) -> str:
        return (
            f"IndexReport(files={self.files_indexed}, symbols={self.total_symbols}, "
            f"relations={self.total_relations}, duration={self.duration_secs:.2f}s)"
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
