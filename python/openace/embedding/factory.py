"""Factory for creating embedding providers."""

from __future__ import annotations

from typing import Any

from openace.embedding.protocol import EmbeddingProvider


def create_provider(backend: str = "local", **kwargs: Any) -> EmbeddingProvider:
    """Create an embedding provider.

    Args:
        backend: Provider type - "local" (ONNX), "openai", "siliconflow", or "voyage".
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
    elif backend == "voyage":
        import os
        from openace.embedding.openai_backend import OpenAIEmbedder
        kwargs.setdefault("base_url", "https://api.voyageai.com/v1")
        kwargs.setdefault("model", "voyage-3-large")
        kwargs.setdefault("dim", 1024)
        kwargs.setdefault("api_key", os.environ.get("VOYAGE_API_KEY"))
        kwargs.setdefault("send_dimensions", False)
        kwargs.setdefault("extra_body", {"output_dimension": kwargs.get("dim", 1024)})
        kwargs.setdefault("batch_size", 128)
        kwargs.setdefault("max_retries", 0)
        kwargs.setdefault("request_delay", 21.0)
        return OpenAIEmbedder(**kwargs)
    elif backend == "voyage-code":
        import os
        from openace.embedding.openai_backend import OpenAIEmbedder
        kwargs.setdefault("base_url", "https://api.voyageai.com/v1")
        kwargs.setdefault("model", "voyage-code-3")
        kwargs.setdefault("dim", 1024)
        kwargs.setdefault("api_key", os.environ.get("VOYAGE_API_KEY"))
        kwargs.setdefault("send_dimensions", False)
        kwargs.setdefault("extra_body", {"output_dimension": kwargs.get("dim", 1024)})
        kwargs.setdefault("batch_size", 128)
        kwargs.setdefault("max_retries", 0)
        kwargs.setdefault("request_delay", 21.0)
        return OpenAIEmbedder(**kwargs)
    else:
        raise ValueError(
            f"Unknown embedding backend: {backend!r}. "
            f"Supported: 'local', 'openai', 'siliconflow', 'voyage', 'voyage-code'"
        )
