"""OpenAI API embedding provider."""

from __future__ import annotations

import os
import time
from typing import Optional

import structlog

from openace.logging import get_logger

logger = get_logger(__name__)


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
        base_url: Optional[str] = None,
        batch_size: int = 2048,
        send_dimensions: bool = True,
        extra_body: Optional[dict] = None,
        max_retries: int = 2,
        request_delay: float = 0.0,
    ):
        self._model = model
        self._dimension = dim
        self._api_key = api_key or os.environ.get("OPENAI_API_KEY")
        self._base_url = base_url
        self._batch_size = batch_size
        self._send_dimensions = send_dimensions
        self._extra_body = extra_body
        self._max_retries = max_retries
        self._request_delay = request_delay
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
        kwargs = {
            "api_key": self._api_key,
            "max_retries": self._max_retries,
            "default_headers": {"User-Agent": "OpenACE/0.1.0"},
        }
        if self._base_url:
            kwargs["base_url"] = self._base_url
        self._client = OpenAI(**kwargs)
        return self._client

    def _call_with_retry(self, client, kwargs: dict, max_attempts: int = 5) -> list:
        """Call embeddings API with manual retry for rate limits and transient errors."""
        from openai import APIStatusError, RateLimitError

        for attempt in range(max_attempts):
            try:
                response = client.embeddings.create(**kwargs)
                return [item.embedding for item in response.data]
            except RateLimitError:
                if attempt == max_attempts - 1:
                    raise
                wait = min(30 * (2 ** attempt), 120)
                logger.warning(
                    "rate limited, retrying",
                    wait_seconds=wait,
                    attempt=attempt + 1,
                    max_attempts=max_attempts,
                )
                time.sleep(wait)
            except APIStatusError as exc:
                if exc.status_code in (403, 500, 502, 503, 504):
                    if attempt == max_attempts - 1:
                        raise
                    wait = min(5 * (2 ** attempt), 60)
                    logger.warning(
                        "HTTP error, retrying",
                        status_code=exc.status_code,
                        wait_seconds=wait,
                        attempt=attempt + 1,
                        max_attempts=max_attempts,
                    )
                    time.sleep(wait)
                else:
                    raise
        return []

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
        n_batches = (len(texts) + self._batch_size - 1) // self._batch_size
        for idx, i in enumerate(range(0, len(texts), self._batch_size)):
            if self._request_delay > 0 and idx > 0:
                time.sleep(self._request_delay)
            batch = texts[i : i + self._batch_size]
            kwargs = {"input": batch, "model": self._model}
            if self._send_dimensions:
                kwargs["dimensions"] = self._dimension
            if self._extra_body:
                kwargs["extra_body"] = self._extra_body
            batch_vecs = self._call_with_retry(client, kwargs)
            all_embeddings.extend(batch_vecs)
            if n_batches > 1:
                logger.debug(
                    "embedding batch done",
                    batch=idx + 1,
                    total_batches=n_batches,
                    embedded=len(all_embeddings),
                    total=len(texts),
                )

        if not all_embeddings:
            return np.zeros((0, self._dimension), dtype=np.float32)

        return np.array(all_embeddings, dtype=np.float32)
