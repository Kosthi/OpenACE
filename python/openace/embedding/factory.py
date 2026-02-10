"""Factory for creating embedding providers."""

from __future__ import annotations

from typing import Any

from openace.embedding.protocol import EmbeddingProvider


def create_provider(backend: str = "local", **kwargs: Any) -> EmbeddingProvider:
    """Create an embedding provider.

    Args:
        backend: Provider type - "local" (ONNX), "openai", or "siliconflow".
        **kwargs: Provider-specific arguments.

    Returns:
        An EmbeddingProvider instance.

    Raises:
        ValueError: If backend is not recognized.
    """
    if backend == "local":
        from openace.embedding.local import OnnxEmbedder
        return OnnxEmbedder(**kwargs)
    elif backend == "openai":
        from openace.embedding.openai_backend import OpenAIEmbedder
        return OpenAIEmbedder(**kwargs)
    elif backend == "siliconflow":
        from openace.embedding.openai_backend import OpenAIEmbedder
        kwargs.setdefault("base_url", "https://api.siliconflow.cn/v1")
        kwargs.setdefault("model", "Qwen/Qwen3-Embedding-8B")
        kwargs.setdefault("dim", 1024)
        return OpenAIEmbedder(**kwargs)
    else:
        raise ValueError(
            f"Unknown embedding backend: {backend!r}. "
            f"Supported: 'local', 'openai', 'siliconflow'"
        )
