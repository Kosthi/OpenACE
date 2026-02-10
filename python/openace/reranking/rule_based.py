"""Rule-based reranker with configurable symbol-kind and signal weights."""

from __future__ import annotations

from dataclasses import replace
from typing import Optional

from openace.reranking.protocol import Reranker
from openace.types import SearchResult

DEFAULT_KIND_WEIGHTS: dict[str, float] = {
    "function": 1.0,
    "method": 1.0,
    "class": 0.9,
    "struct": 0.9,
    "interface": 0.8,
    "trait": 0.8,
    "module": 0.5,
    "variable": 0.3,
    "field": 0.3,
    "constant": 0.4,
}

_DEFAULT_SIGNAL_WEIGHT = 1.0
_EXACT_MATCH_BONUS = 0.5


class RuleBasedReranker:
    """Reranker that combines RRF score with rule-based bonuses.

    Scoring formula per result::

        rerank_score = score + kind_bonus + signal_bonus + exact_match_bonus

    Where:
        - ``score`` is the original retrieval score (kept unchanged).
        - ``kind_bonus`` is looked up from *kind_weights* by the symbol's kind.
        - ``signal_bonus`` is the sum of weights for each signal in *match_signals*.
        - ``exact_match_bonus`` is added when *query* appears (case-insensitive)
          in the result's *name* or *qualified_name*.
    """

    def __init__(
        self,
        *,
        kind_weights: Optional[dict[str, float]] = None,
        signal_weights: Optional[dict[str, float]] = None,
    ) -> None:
        self.kind_weights = kind_weights if kind_weights is not None else dict(DEFAULT_KIND_WEIGHTS)
        self.signal_weights = signal_weights if signal_weights is not None else {}

    # -- Reranker protocol ----------------------------------------------------

    def rerank(
        self,
        query: str,
        results: list[SearchResult],
        *,
        top_k: int | None = None,
    ) -> list[SearchResult]:
        """Rerank *results* using rule-based scoring."""
        if not results:
            return []

        query_lower = query.lower()
        scored: list[SearchResult] = []

        for result in results:
            kind_bonus = self.kind_weights.get(result.kind, 0.0)

            signal_bonus = 0.0
            for signal in result.match_signals:
                signal_bonus += self.signal_weights.get(signal, _DEFAULT_SIGNAL_WEIGHT)

            exact_match_bonus = 0.0
            if query_lower and (
                query_lower in result.name.lower()
                or query_lower in result.qualified_name.lower()
            ):
                exact_match_bonus = _EXACT_MATCH_BONUS

            rerank_score = result.score + kind_bonus + signal_bonus + exact_match_bonus
            scored.append(replace(result, rerank_score=rerank_score))

        scored.sort(key=lambda r: r.rerank_score or 0.0, reverse=True)

        if top_k is not None:
            scored = scored[:top_k]

        return scored
