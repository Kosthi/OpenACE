"""Factory for creating reranking providers."""

from __future__ import annotations

from typing import Any

from openace.reranking.protocol import Reranker


def create_reranker(backend: str = "rule_based", **kwargs: Any) -> Reranker:
    """Create a reranker.

    Args:
        backend: Reranker type - "rule_based", "cross_encoder", "cohere", or "openai".
        **kwargs: Provider-specific arguments.

    Returns:
        A Reranker instance.

    Raises:
        ValueError: If backend is not recognized.
    """
    if backend == "rule_based":
        from openace.reranking.rule_based import RuleBasedReranker

        return RuleBasedReranker(**kwargs)
    elif backend == "cross_encoder":
        from openace.reranking.cross_encoder import CrossEncoderReranker

        return CrossEncoderReranker(**kwargs)
    elif backend == "cohere":
        from openace.reranking.llm_backend import LLMReranker

        return LLMReranker(provider="cohere", **kwargs)
    elif backend == "openai":
        from openace.reranking.llm_backend import LLMReranker

        return LLMReranker(provider="openai", **kwargs)
    else:
        raise ValueError(
            f"Unknown reranker backend: {backend!r}. "
            f"Supported: 'rule_based', 'cross_encoder', 'cohere', 'openai'"
        )
