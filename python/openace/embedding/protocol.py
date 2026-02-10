"""Embedding provider protocol definition."""

from __future__ import annotations

from typing import TYPE_CHECKING, Protocol, runtime_checkable

if TYPE_CHECKING:
    import numpy as np


@runtime_checkable
class EmbeddingProvider(Protocol):
    """Protocol for embedding providers."""

    @property
    def dimension(self) -> int:
        """The dimension of embedding vectors."""
        ...

    def embed(self, texts: list[str]) -> "np.ndarray":
        """Embed texts into vectors.

        Args:
            texts: List of text strings to embed.

        Returns:
            numpy array of shape (len(texts), dimension), dtype float32.
        """
        ...
