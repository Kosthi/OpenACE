"""Comprehensive tests for the reranking module."""

import dataclasses

import pytest

from openace.engine import Engine
from openace.reranking import Reranker, create_reranker
from openace.reranking.cross_encoder import CrossEncoderReranker
from openace.reranking.llm_backend import LLMReranker
from openace.reranking.rule_based import RuleBasedReranker
from openace.types import SearchResult


# ---------------------------------------------------------------------------
# Helper
# ---------------------------------------------------------------------------


def _make_result(
    name: str = "foo",
    kind: str = "function",
    score: float = 1.0,
    signals: list[str] | None = None,
) -> SearchResult:
    return SearchResult(
        symbol_id=f"id-{name}",
        name=name,
        qualified_name=f"mod.{name}",
        kind=kind,
        file_path="src/main.py",
        line_range=(1, 10),
        score=score,
        match_signals=signals or [],
    )


# ---------------------------------------------------------------------------
# 1. Protocol conformance
# ---------------------------------------------------------------------------


class TestRerankProtocol:
    def test_rule_based_satisfies_protocol(self):
        assert isinstance(RuleBasedReranker(), Reranker)

    def test_cross_encoder_satisfies_protocol(self):
        assert isinstance(CrossEncoderReranker(), Reranker)

    def test_llm_satisfies_protocol(self):
        assert isinstance(LLMReranker(provider="cohere", api_key="test"), Reranker)


# ---------------------------------------------------------------------------
# 2. RuleBasedReranker
# ---------------------------------------------------------------------------


class TestRuleBasedReranker:
    def test_empty_input(self):
        reranker = RuleBasedReranker()
        assert reranker.rerank("query", []) == []

    def test_preserves_original_score(self):
        reranker = RuleBasedReranker()
        results = [_make_result(score=3.14)]
        reranked = reranker.rerank("query", results)
        assert reranked[0].score == 3.14

    def test_rerank_score_is_set(self):
        reranker = RuleBasedReranker()
        results = [_make_result(), _make_result(name="bar")]
        reranked = reranker.rerank("query", results)
        for r in reranked:
            assert r.rerank_score is not None

    def test_kind_weighting(self):
        reranker = RuleBasedReranker()
        func_result = _make_result(name="a", kind="function", score=1.0)
        var_result = _make_result(name="b", kind="variable", score=1.0)
        reranked = reranker.rerank("query", [func_result, var_result])
        func_out = next(r for r in reranked if r.name == "a")
        var_out = next(r for r in reranked if r.name == "b")
        assert func_out.rerank_score > var_out.rerank_score

    def test_exact_match_bonus(self):
        reranker = RuleBasedReranker()
        match = _make_result(name="query_handler", kind="function", score=1.0)
        no_match = _make_result(name="other", kind="function", score=1.0)
        reranked = reranker.rerank("query", [match, no_match])
        match_out = next(r for r in reranked if r.name == "query_handler")
        no_match_out = next(r for r in reranked if r.name == "other")
        assert match_out.rerank_score > no_match_out.rerank_score

    def test_signal_bonus(self):
        reranker = RuleBasedReranker()
        many_signals = _make_result(
            name="a", kind="function", score=1.0, signals=["name_match", "type_match"]
        )
        few_signals = _make_result(name="b", kind="function", score=1.0, signals=[])
        reranked = reranker.rerank("query", [many_signals, few_signals])
        many_out = next(r for r in reranked if r.name == "a")
        few_out = next(r for r in reranked if r.name == "b")
        assert many_out.rerank_score > few_out.rerank_score

    def test_top_k_truncation(self):
        reranker = RuleBasedReranker()
        results = [_make_result(name=f"r{i}") for i in range(5)]
        reranked = reranker.rerank("query", results, top_k=2)
        assert len(reranked) == 2

    def test_custom_kind_weights(self):
        custom_weights = {"variable": 10.0, "function": 0.0}
        reranker = RuleBasedReranker(kind_weights=custom_weights)
        func_result = _make_result(name="a", kind="function", score=1.0)
        var_result = _make_result(name="b", kind="variable", score=1.0)
        reranked = reranker.rerank("query", [func_result, var_result])
        var_out = next(r for r in reranked if r.name == "b")
        func_out = next(r for r in reranked if r.name == "a")
        assert var_out.rerank_score > func_out.rerank_score


# ---------------------------------------------------------------------------
# 3. Factory
# ---------------------------------------------------------------------------


class TestFactory:
    def test_create_rule_based(self):
        reranker = create_reranker("rule_based")
        assert isinstance(reranker, RuleBasedReranker)

    def test_create_cross_encoder(self):
        reranker = create_reranker("cross_encoder")
        assert isinstance(reranker, CrossEncoderReranker)

    def test_create_cohere(self):
        reranker = create_reranker("cohere", api_key="test")
        assert isinstance(reranker, LLMReranker)

    def test_create_openai(self):
        reranker = create_reranker("openai", api_key="test")
        assert isinstance(reranker, LLMReranker)

    def test_unknown_backend(self):
        with pytest.raises(ValueError):
            create_reranker("unknown")


# ---------------------------------------------------------------------------
# 4. Engine + reranker integration
# ---------------------------------------------------------------------------


class MockReranker:
    def __init__(self):
        self.called = False
        self.received_results_count = 0
        self.received_top_k = None

    def rerank(self, query, results, *, top_k=None):
        self.called = True
        self.received_results_count = len(results)
        self.received_top_k = top_k
        return results[:top_k] if top_k else results


class FailingReranker:
    def rerank(self, query, results, *, top_k=None):
        raise RuntimeError("reranker failed")


class TestEngineRerankerIntegration:
    def test_engine_accepts_reranker(self, sample_project):
        engine = Engine(str(sample_project), reranker=RuleBasedReranker())
        assert engine._reranker is not None

    def test_engine_search_with_reranker_mock(self, sample_project):
        mock = MockReranker()
        engine = Engine(str(sample_project), reranker=mock)
        engine.index()
        engine.search("process_data")
        assert mock.called is True

    def test_engine_fail_open(self, sample_project):
        engine = Engine(str(sample_project), reranker=FailingReranker())
        engine.index()
        results = engine.search("process_data")
        # fail-open: search should not raise and should return results
        assert isinstance(results, list)

    def test_retrieval_limit_calculation(self, sample_project):
        mock = MockReranker()
        engine = Engine(str(sample_project), reranker=mock, rerank_pool_size=50)
        engine.index()
        engine.search("process_data", limit=5)
        # Engine passes the full retrieval pool size to the reranker so it
        # can score all candidates before file-level dedup and final limit.
        assert mock.received_top_k == 50
        # The reranker should receive results from the expanded retrieval pool
        # (up to what's available in the index), not just `limit` results.
        # We also verify the engine computed the pool correctly by checking
        # the internal attribute used for the retrieval limit.
        assert engine._rerank_pool_size == 50


# ---------------------------------------------------------------------------
# 5. SearchResult rerank_score
# ---------------------------------------------------------------------------


class TestSearchResultRerankScore:
    def test_rerank_score_default_none(self):
        result = _make_result()
        assert result.rerank_score is None

    def test_rerank_score_can_be_set(self):
        result = _make_result()
        updated = dataclasses.replace(result, rerank_score=0.99)
        assert updated.rerank_score == 0.99
