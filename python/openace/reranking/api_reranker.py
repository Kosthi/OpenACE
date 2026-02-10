"""HTTP-based reranker for standard rerank API endpoints.

Works with providers that implement the Cohere-compatible rerank API format,
such as SiliconFlow, Jina, and others.
"""

from __future__ import annotations

import json
import os
import urllib.request
from dataclasses import replace
from typing import Optional

from openace.types import SearchResult


class APIReranker:
    """Reranker using a standard HTTP rerank API (Cohere-compatible format).

    Compatible with SiliconFlow, Jina, and other providers that expose
    a ``/rerank`` endpoint accepting ``{model, query, documents, top_n}``.

    No extra dependencies required (uses stdlib urllib).
    """

    def __init__(
        self,
        *,
        model: str,
        api_key: Optional[str] = None,
        base_url: str,
        timeout: float = 30.0,
        max_results: int = 100,
    ):
        self._model = model
        self._api_key = api_key or os.environ.get("OPENAI_API_KEY")
        self._base_url = base_url.rstrip("/")
        self._timeout = timeout
        self._max_results = max_results

    @staticmethod
    def _build_document_text(result: SearchResult) -> str:
        return f"{result.qualified_name} ({result.kind}) {result.name}"

    def rerank(
        self, query: str, results: list[SearchResult], *, top_k: int | None = None
    ) -> list[SearchResult]:
        if not results:
            return []
        requested = top_k if top_k is not None else len(results)
        effective_top_k = min(requested, self._max_results, len(results))
        if effective_top_k <= 0:
            return []

        documents = [self._build_document_text(r) for r in results]

        payload = json.dumps({
            "model": self._model,
            "query": query,
            "documents": documents,
            "top_n": effective_top_k,
        }).encode("utf-8")

        url = f"{self._base_url}/rerank"
        req = urllib.request.Request(
            url,
            data=payload,
            headers={
                "Content-Type": "application/json",
                "Authorization": f"Bearer {self._api_key}",
            },
            method="POST",
        )

        try:
            with urllib.request.urlopen(req, timeout=self._timeout) as resp:
                body = json.loads(resp.read().decode("utf-8"))
        except Exception as exc:
            raise RuntimeError(f"Rerank API request failed: {exc}") from exc

        api_results = body.get("results", [])

        scored: list[SearchResult] = []
        for item in api_results:
            idx = item["index"]
            score = item["relevance_score"]
            scored.append(replace(results[idx], rerank_score=float(score)))
        scored.sort(key=lambda r: r.rerank_score, reverse=True)
        return scored[:effective_top_k]
