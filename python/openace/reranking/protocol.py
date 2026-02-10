"""Reranker protocol definition."""

from __future__ import annotations

from typing import Protocol, runtime_checkable

from openace.types import SearchResult


@runtime_checkable
class Reranker(Protocol):
    """Protocol for reranking providers."""

    def rerank(
        self, query: str, results: list[SearchResult], *, top_k: int | None = None
    ) -> list[SearchResult]:
        """Rerank search results by relevance to the query.

        Args:
            query: The search query to rerank against.
            results: List of search results to rerank.
            top_k: If provided, return only the top-k results.

        Returns:
            Reranked list of search results, scored by relevance.
        """
        ...
