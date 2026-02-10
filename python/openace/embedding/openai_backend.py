"""OpenAI API embedding provider."""

from __future__ import annotations

import os
from typing import Optional


class OpenAIEmbedder:
    """Embedding using OpenAI API.

    Requires: pip install openace[openai]
    Set OPENAI_API_KEY environment variable.
    """

    def __init__(
        self,
        *,
        model: str = "text-embedding-3-small",
        dim: int = 1536,
        api_key: Optional[str] = None,
        batch_size: int = 2048,
    ):
        self._model = model
        self._dimension = dim
        self._api_key = api_key or os.environ.get("OPENAI_API_KEY")
        self._batch_size = batch_size
        self._client = None

    @property
    def dimension(self) -> int:
        return self._dimension

    def _get_client(self):
        if self._client is not None:
            return self._client
        try:
            from openai import OpenAI
        except ImportError:
            raise ImportError(
                "OpenAI embedding requires the openai package. "
                "Install with: pip install openace[openai]"
            )
        if not self._api_key:
            raise ValueError(
                "OpenAI API key required. Set OPENAI_API_KEY environment variable "
                "or pass api_key parameter."
            )
        self._client = OpenAI(api_key=self._api_key)
        return self._client

    def embed(self, texts: list[str]) -> "numpy.ndarray":
        """Embed texts using OpenAI API.

        Args:
            texts: List of text strings to embed.

        Returns:
            numpy array of shape (len(texts), dimension), dtype float32.
        """
        import numpy as np

        client = self._get_client()

        all_embeddings = []
        for i in range(0, len(texts), self._batch_size):
            batch = texts[i : i + self._batch_size]
            response = client.embeddings.create(
                input=batch,
                model=self._model,
                dimensions=self._dimension,
            )
            batch_vecs = [item.embedding for item in response.data]
            all_embeddings.extend(batch_vecs)

        if not all_embeddings:
            return np.zeros((0, self._dimension), dtype=np.float32)

        return np.array(all_embeddings, dtype=np.float32)
