"""Query expansion via LLM for improved code search recall."""

from __future__ import annotations

import json
import logging
import os
import time
import urllib.request
from typing import Optional, Protocol

logger = logging.getLogger(__name__)

EXPANSION_PROMPT = """\
You are a code search query expander. Given a natural language search query about code, \
generate additional search terms that would help find the relevant source code.

Include:
- Likely function/class/variable names (e.g., "parse_xml", "XMLParser")
- Common abbreviations and acronyms used in code (e.g., "mfd" for "math formula detection")
- Related technical terms and synonyms
- File path segments that might contain the code (e.g., "utils", "models")

Return ONLY a single line of space-separated terms. No explanation, no numbering, no punctuation.

Query: {query}
Terms:"""


class QueryExpander(Protocol):
    """Protocol for query expanders."""

    def expand(self, query: str) -> str:
        """Expand a search query with additional terms.

        Args:
            query: Original search query.

        Returns:
            Expanded query string (original + additional terms).
        """
        ...


class LLMQueryExpander:
    """Query expander using an LLM via OpenAI-compatible chat API.

    Uses stdlib urllib, no extra dependencies required.
    """

    def __init__(
        self,
        *,
        model: str = "Qwen/Qwen3-8B",
        api_key: Optional[str] = None,
        base_url: str = "https://api.siliconflow.cn/v1",
        timeout: float = 15.0,
        max_tokens: int = 80,
        max_terms: int = 30,
        max_retries: int = 3,
    ):
        self._model = model
        self._api_key = api_key or os.environ.get("OPENAI_API_KEY")
        self._base_url = base_url.rstrip("/")
        self._timeout = timeout
        self._max_tokens = max_tokens
        self._max_terms = max_terms
        self._max_retries = max_retries

    def expand(self, query: str) -> str:
        """Expand query using LLM chat completion.

        Returns the original query concatenated with LLM-generated terms.
        On failure, returns the original query unchanged.
        """
        prompt = EXPANSION_PROMPT.format(query=query)

        payload = json.dumps({
            "model": self._model,
            "messages": [{"role": "user", "content": prompt}],
            "max_tokens": self._max_tokens,
            "temperature": 0.0,
        }).encode("utf-8")

        url = f"{self._base_url}/chat/completions"

        try:
            body = None
            for attempt in range(self._max_retries):
                req = urllib.request.Request(
                    url,
                    data=payload,
                    headers={
                        "Content-Type": "application/json",
                        "Authorization": f"Bearer {self._api_key}",
                        "User-Agent": "OpenACE/0.1.0",
                    },
                    method="POST",
                )
                try:
                    with urllib.request.urlopen(req, timeout=self._timeout) as resp:
                        body = json.loads(resp.read().decode("utf-8"))
                    break
                except urllib.error.HTTPError as exc:
                    if exc.code in (403, 500, 502, 503, 504) and attempt < self._max_retries - 1:
                        wait = min(3 * (2 ** attempt), 15)
                        time.sleep(wait)
                        continue
                    raise

            if body is None:
                return query

            content = body["choices"][0]["message"]["content"].strip()
            # Clean up: remove any thinking tags if present (some models wrap in <think>)
            if "<think>" in content:
                # Extract text after </think> tag
                idx = content.find("</think>")
                if idx != -1:
                    content = content[idx + len("</think>"):].strip()

            # Take only the first line to avoid multi-line responses
            first_line = content.split("\n")[0].strip()

            if first_line:
                # Limit to max_terms to avoid diluting BM25 signal
                terms = first_line.split()[:self._max_terms]
                expanded = f"{query} {' '.join(terms)}"
                logger.info("Query expanded: %r -> %r", query, expanded)
                return expanded
        except Exception as exc:
            logger.warning("Query expansion failed (%s), using original query", exc)

        return query


def create_query_expander(
    backend: str = "siliconflow", **kwargs
) -> QueryExpander:
    """Create a query expander.

    Args:
        backend: "siliconflow" or "openai".
        **kwargs: Provider-specific arguments.

    Returns:
        A QueryExpander instance.
    """
    if backend == "siliconflow":
        kwargs.setdefault("base_url", "https://api.siliconflow.cn/v1")
        kwargs.setdefault("model", "Qwen/Qwen3-8B")
        return LLMQueryExpander(**kwargs)
    elif backend == "openai":
        return LLMQueryExpander(**kwargs)
    else:
        raise ValueError(
            f"Unknown query expansion backend: {backend!r}. "
            f"Supported: 'siliconflow', 'openai'"
        )
