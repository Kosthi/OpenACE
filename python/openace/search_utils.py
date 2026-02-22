"""Shared search-result aggregation utilities.

Used by both the MCP server (``server/app.py``) and the evaluation
harness (``eval/swebench/retrieval_eval.py``) to ensure identical
file-level grouping, scoring, and score-gap truncation logic.
"""

from __future__ import annotations

from openace.engine import _is_test_file, _LOW_VALUE_KINDS
from openace.types import SearchResult

_GRAPH_ONLY = {"graph"}


def _effective_score(r: SearchResult) -> float:
    """Return the best available score for a result, with graph-only penalty."""
    base = r.rerank_score if r.rerank_score is not None else r.score
    if set(r.match_signals) == _GRAPH_ONLY:
        base *= 0.7
    return base


def _aggregate_by_file(results: list[SearchResult]) -> list[dict]:
    """Group search results by file path into file-level groups.

    Returns list of dicts sorted by (tier, -effective_score):
        file_path, symbols, effective_score, tier, all_signals
    """
    groups: dict[str, list[SearchResult]] = {}
    for r in results:
        groups.setdefault(r.file_path, []).append(r)

    file_groups = []
    for file_path, symbols in groups.items():
        symbols.sort(key=_effective_score, reverse=True)
        best_score = _effective_score(symbols[0])
        all_signals: set[str] = set()
        for s in symbols:
            all_signals.update(s.match_signals)

        all_low_value = all(s.kind in _LOW_VALUE_KINDS for s in symbols)
        if all_low_value:
            tier = 2
        elif _is_test_file(file_path):
            tier = 1
        else:
            tier = 0

        file_groups.append({
            "file_path": file_path,
            "symbols": symbols,
            "effective_score": best_score,
            "tier": tier,
            "all_signals": all_signals,
        })

    file_groups.sort(key=lambda g: (g["tier"], -g["effective_score"]))
    return file_groups


def _apply_file_score_gap(groups: list[dict], min_results: int = 3) -> list[dict]:
    """Apply 0.6-ratio score-gap cutoff on file groups."""
    if len(groups) <= min_results:
        return groups
    cut_idx = len(groups)
    for idx in range(min_results, len(groups)):
        prev = groups[idx - 1]["effective_score"]
        cur = groups[idx]["effective_score"]
        if prev > 0 and cur / prev < 0.6:
            cut_idx = idx
            break
    return groups[:max(cut_idx, min_results)]
