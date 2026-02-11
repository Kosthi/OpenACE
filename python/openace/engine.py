"""OpenACE Engine - high-level Python SDK."""

from __future__ import annotations

import logging
import os
from pathlib import Path
from typing import TYPE_CHECKING, Optional

from openace.exceptions import IndexingError, OpenACEError, SearchError
from openace.types import IndexReport, SearchResult, Symbol

logger = logging.getLogger(__name__)

if TYPE_CHECKING:
    from openace.embedding.protocol import EmbeddingProvider
    from openace.reranking.protocol import Reranker


def _convert_symbol(py_sym) -> Symbol:
    """Convert a PySymbol from the Rust extension to a Python Symbol."""
    return Symbol(
        id=py_sym.id,
        name=py_sym.name,
        qualified_name=py_sym.qualified_name,
        kind=py_sym.kind,
        language=py_sym.language,
        file_path=py_sym.file_path,
        line_start=py_sym.line_start,
        line_end=py_sym.line_end,
        signature=py_sym.signature,
        doc_comment=py_sym.doc_comment,
    )


def _convert_search_result(py_result) -> SearchResult:
    """Convert a PySearchResult from the Rust extension to a Python SearchResult."""
    return SearchResult(
        symbol_id=py_result.symbol_id,
        name=py_result.name,
        qualified_name=py_result.qualified_name,
        kind=py_result.kind,
        file_path=py_result.file_path,
        line_range=py_result.line_range,
        score=py_result.score,
        match_signals=list(py_result.match_signals),
        related_symbols=[_convert_search_result(r) for r in py_result.related_symbols],
    )


def _convert_index_report(py_report) -> IndexReport:
    """Convert a PyIndexReport from the Rust extension."""
    return IndexReport(
        total_files_scanned=py_report.total_files_scanned,
        files_indexed=py_report.files_indexed,
        files_skipped=py_report.files_skipped,
        files_failed=py_report.files_failed,
        total_symbols=py_report.total_symbols,
        total_relations=py_report.total_relations,
        duration_secs=py_report.duration_secs,
    )


class Engine:
    """OpenACE code intelligence engine.

    High-level interface combining Rust-powered indexing/search with
    optional Python embedding providers.

    Usage:
        from openace import Engine
        engine = Engine("/path/to/project")
        report = engine.index()
        results = engine.search("parse XML")
    """

    def __init__(
        self,
        project_root: str | os.PathLike,
        *,
        embedding_provider: Optional[EmbeddingProvider] = None,
        embedding_dim: Optional[int] = None,
        reranker: Optional[Reranker] = None,
        rerank_pool_size: int = 50,
    ):
        """Initialize the engine.

        Args:
            project_root: Path to the project directory.
            embedding_provider: Optional embedding provider for vector search.
            embedding_dim: Dimension of embedding vectors. If None, auto-detected
                from existing index metadata or defaults to 384.
            reranker: Optional reranker for two-stage search.
            rerank_pool_size: Number of candidates to retrieve before reranking.
        """
        from openace._openace import EngineBinding

        self._project_root = str(Path(project_root).resolve())
        self._embedding_provider = embedding_provider
        self._embedding_dim = embedding_dim
        self._reranker = reranker
        self._rerank_pool_size = min(rerank_pool_size, 100)
        if reranker is not None and rerank_pool_size > 100:
            logger.warning(
                "rerank_pool_size=%d exceeds Rust upper bound of 100, capped to 100",
                rerank_pool_size,
            )
        self._core = EngineBinding(self._project_root, embedding_dim)

    @property
    def project_root(self) -> str:
        """The absolute path to the project root."""
        return self._project_root

    def index(self, *, incremental: bool = True) -> IndexReport:
        """Run indexing on the project.

        Args:
            incremental: Currently ignored (always full index).

        Returns:
            IndexReport with statistics about the indexing run.
        """
        try:
            py_report = self._core.index_full(self._project_root)
            report = _convert_index_report(py_report)
        except Exception as e:
            raise IndexingError(f"indexing failed: {e}") from e

        # Auto-embed if provider is set
        if self._embedding_provider is not None:
            self.embed_all()

        return report

    def _validate_path(self, path: str) -> None:
        """Validate that a relative path stays within the project root."""
        resolved = (Path(self._project_root) / path).resolve()
        if not str(resolved).startswith(self._project_root):
            raise SearchError(f"path outside project root: {path}")

    def search(
        self,
        query: str,
        *,
        limit: int = 10,
        language: Optional[str] = None,
        file_path: Optional[str] = None,
        dedupe_by_file: bool = True,
    ) -> list[SearchResult]:
        """Search for symbols using multi-signal retrieval.

        Args:
            query: Search text.
            limit: Maximum number of results.
            language: Optional language filter (e.g., "python").
            file_path: Optional file path prefix filter.
            dedupe_by_file: If True, keep only the highest-scoring symbol
                per file so that results cover more distinct files.

        Returns:
            List of SearchResult sorted by relevance score.
        """
        try:
            if file_path is not None:
                self._validate_path(file_path)

            if limit <= 0:
                return []

            query_vector = None
            if self._embedding_provider is not None:
                vectors = self._embedding_provider.embed([query])
                query_vector = vectors[0].tolist()

            # Stage 1: retrieval with expanded pool
            # Expand when reranker or file-dedup is active so we have
            # enough candidates after filtering.
            if self._reranker is not None or dedupe_by_file:
                retrieval_limit = max(limit * 5, self._rerank_pool_size)
                retrieval_limit = min(retrieval_limit, 100)  # Rust upper bound
            else:
                retrieval_limit = limit

            py_results = self._core.search(
                query,
                query_vector,
                retrieval_limit,
                language,
                file_path,
            )
            results = [_convert_search_result(r) for r in py_results]

            # Stage 2: rerank if reranker is configured
            if self._reranker is not None:
                try:
                    results = self._reranker.rerank(
                        query, results, top_k=retrieval_limit,
                    )
                except Exception as e:
                    logger.warning(
                        "Reranker failed (%s), falling back to original ranking",
                        type(e).__name__,
                    )

            # Stage 3: file-level dedup — keep highest-scoring symbol per file
            if dedupe_by_file:
                seen_files: set[str] = set()
                deduped: list[SearchResult] = []
                for r in results:
                    if r.file_path not in seen_files:
                        seen_files.add(r.file_path)
                        deduped.append(r)
                results = deduped

            return results[:limit]
        except OpenACEError:
            raise
        except Exception as e:
            raise SearchError(f"search failed: {e}") from e

    def find_symbol(self, name: str) -> list[Symbol]:
        """Find symbols by exact name match.

        Args:
            name: Symbol name or qualified name to search for.

        Returns:
            List of matching Symbol objects.
        """
        try:
            py_syms = self._core.find_symbol(name)
            return [_convert_symbol(s) for s in py_syms]
        except Exception as e:
            raise SearchError(f"find_symbol failed: {e}") from e

    def get_file_outline(self, path: str) -> list[Symbol]:
        """Get all symbols defined in a file.

        Args:
            path: Relative file path within the project.

        Returns:
            List of Symbol objects in the file.
        """
        try:
            self._validate_path(path)
            py_syms = self._core.get_file_outline(path)
            return [_convert_symbol(s) for s in py_syms]
        except Exception as e:
            raise SearchError(f"get_file_outline failed: {e}") from e

    def embed_all(self) -> int:
        """Compute and store embeddings for all indexed symbols.

        Iterates all symbols in batches, computes embeddings via the
        configured provider, and stores vectors in the index.

        Returns:
            Number of symbols embedded.
        """
        if self._embedding_provider is None:
            raise OpenACEError("no embedding provider configured")

        batch_size = 100
        offset = 0
        total_embedded = 0

        while True:
            symbols = self._core.list_symbols_for_embedding(batch_size, offset)
            if not symbols:
                break

            # Build text for embedding: qualified_name + signature + doc_comment
            texts = []
            ids = []
            for sym in symbols:
                parts = [sym.qualified_name]
                if sym.signature:
                    parts.append(sym.signature)
                if sym.doc_comment:
                    parts.append(sym.doc_comment)
                texts.append(" ".join(parts))
                ids.append(sym.id)

            # Compute embeddings
            vectors = self._embedding_provider.embed(texts)
            vector_lists = [v.tolist() for v in vectors]

            # Store in vector index
            self._core.add_vectors(ids, vector_lists)

            total_embedded += len(symbols)
            offset += batch_size

        self._core.flush()
        return total_embedded

    def flush(self) -> None:
        """Persist all storage backends to disk."""
        self._core.flush()
