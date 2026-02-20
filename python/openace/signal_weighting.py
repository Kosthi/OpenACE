"""Signal weighting for adaptive retrieval fusion."""

from __future__ import annotations

import json
import structlog
import os
import re
import time
import urllib.request
from dataclasses import dataclass
from typing import Optional, Protocol

logger = structlog.get_logger(__name__)


@dataclass(frozen=True)
class SignalWeights:
    """Weights for each retrieval signal in RRF fusion."""

    bm25: float = 1.0
    vector: float = 1.0
    exact: float = 1.0
    chunk_bm25: float = 1.0
    graph: float = 1.0


class SignalWeighter(Protocol):
    """Protocol for signal weight generators."""

    def compute_weights(self, query: str) -> SignalWeights:
        """Compute per-signal weights for a given query.

        Args:
            query: The search query string.

        Returns:
            SignalWeights with appropriate weights for each signal.
        """
        ...


WEIGHTING_PROMPT = """\
You are a code search signal weighter. Given a search query, output JSON weights \
for 5 retrieval signals. Each weight is a float between 1.0 and 3.0.

IMPORTANT: All weights must be >= 1.0. You may BOOST important signals above 1.0 \
but must NEVER reduce any signal below 1.0, because every signal channel provides \
unique recall that should not be suppressed.

Signals:
- bm25: keyword matching in symbol names, signatures, docstrings
- vector: semantic similarity via embeddings
- exact: exact symbol name match
- chunk_bm25: file-level keyword matching in code chunks
- graph: graph traversal from other hits (call graph, inheritance)

Guidelines:
- Symbol name queries (e.g., "parse_xml", "MyClass") -> boost exact to 2.5 and bm25 to 2.0
- Natural language concept queries (e.g., "dependency injection", "error handling") -> boost vector to 2.5 and graph to 2.0
- Architecture queries (e.g., "overall architecture", "data flow") -> boost graph to 2.5 and chunk_bm25 to 2.0
- Chinese/non-ASCII queries -> boost vector to 2.5 and graph to 1.5
- Mixed queries -> mild boosts (1.2-1.5) to the most relevant signals

Output ONLY a JSON object with keys: bm25, vector, exact, chunk_bm25, graph.
No explanation, no markdown fences.

Query: {query}
Weights:"""


class LLMSignalWeighter:
    """Signal weighter using an LLM via OpenAI-compatible chat API.

    Uses stdlib urllib, no extra dependencies required.
    """

    def __init__(
        self,
        *,
        model: str = "Qwen/Qwen3-8B",
        api_key: Optional[str] = None,
        base_url: str = "https://api.siliconflow.cn/v1",
        timeout: float = 10.0,
        max_retries: int = 3,
    ):
        self._model = model
        self._api_key = api_key or os.environ.get("OPENAI_API_KEY")
        self._base_url = base_url.rstrip("/")
        self._timeout = timeout
        self._max_retries = max_retries

    def compute_weights(self, query: str) -> SignalWeights:
        """Compute signal weights using LLM chat completion.

        On failure, returns default weights (all 1.0).
        """
        prompt = WEIGHTING_PROMPT.format(query=query)

        payload = json.dumps({
            "model": self._model,
            "messages": [{"role": "user", "content": prompt}],
            "max_tokens": 120,
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
                return SignalWeights()

            content = body["choices"][0]["message"]["content"].strip()
            # Clean up thinking tags if present
            if "<think>" in content:
                idx = content.find("</think>")
                if idx != -1:
                    content = content[idx + len("</think>"):].strip()

            # Extract JSON object from content
            # Try to find a JSON object even if wrapped in markdown fences
            json_match = re.search(r"\{[^}]+\}", content)
            if json_match:
                weights_dict = json.loads(json_match.group())
            else:
                weights_dict = json.loads(content)

            def _clamp(v: float) -> float:
                return max(1.0, min(3.0, float(v)))

            weights = SignalWeights(
                bm25=_clamp(weights_dict.get("bm25", 1.0)),
                vector=_clamp(weights_dict.get("vector", 1.0)),
                exact=_clamp(weights_dict.get("exact", 1.0)),
                chunk_bm25=_clamp(weights_dict.get("chunk_bm25", 1.0)),
                graph=_clamp(weights_dict.get("graph", 1.0)),
            )
            logger.info("signal weights computed", query=query, weights=str(weights))
            return weights

        except Exception as exc:
            logger.warning("signal weighting failed, using defaults", error=str(exc))
            return SignalWeights()


# Pattern for symbol-like tokens: snake_case, CamelCase, dot.separated
_SYMBOL_PATTERN = re.compile(
    r"^[a-zA-Z_][a-zA-Z0-9_.]*$"
)
_CAMEL_CASE = re.compile(r"[a-z][A-Z]")
_SNAKE_CASE = re.compile(r"[a-z]_[a-z]")
_CJK_RANGE = re.compile(r"[\u4e00-\u9fff\u3400-\u4dbf]")


class RuleBasedSignalWeighter:
    """Zero-latency rule-based signal weighter.

    Heuristic rules to determine query intent and boost relevant signals.
    All weights are >= 1.0 (boost-only, never suppress).
    No API calls required.
    """

    def compute_weights(self, query: str) -> SignalWeights:
        """Compute signal weights using heuristic rules."""
        stripped = query.strip()
        has_cjk = bool(_CJK_RANGE.search(stripped))
        words = stripped.split()
        word_count = len(words)
        is_single_token = word_count == 1

        # Check if the query looks like a symbol name
        is_symbol_like = (
            is_single_token
            and _SYMBOL_PATTERN.match(stripped) is not None
        ) or (
            is_single_token
            and (bool(_SNAKE_CASE.search(stripped)) or bool(_CAMEL_CASE.search(stripped)))
        )

        if is_symbol_like:
            # Symbol name query: boost exact + bm25
            return SignalWeights(
                bm25=2.0,
                vector=1.0,
                exact=2.5,
                chunk_bm25=1.0,
                graph=1.0,
            )

        if has_cjk:
            # CJK query: boost vector + graph (BM25 stays at baseline)
            return SignalWeights(
                bm25=1.0,
                vector=2.5,
                exact=1.0,
                chunk_bm25=1.0,
                graph=1.5,
            )

        if word_count >= 3:
            # Multi-word natural language: boost vector + graph
            return SignalWeights(
                bm25=1.0,
                vector=2.0,
                exact=1.0,
                chunk_bm25=1.2,
                graph=1.5,
            )

        # Default: mild boost to vector for 2-word queries
        return SignalWeights(
            bm25=1.0,
            vector=1.3,
            exact=1.0,
            chunk_bm25=1.0,
            graph=1.0,
        )


def create_signal_weighter(
    backend: str = "siliconflow", **kwargs,
) -> SignalWeighter:
    """Create a signal weighter.

    Args:
        backend: "siliconflow", "openai", or "rule_based".
        **kwargs: Provider-specific arguments.

    Returns:
        A SignalWeighter instance.
    """
    if backend == "siliconflow":
        kwargs.setdefault("base_url", "https://api.siliconflow.cn/v1")
        kwargs.setdefault("model", "Qwen/Qwen3-8B")
        return LLMSignalWeighter(**kwargs)
    elif backend == "openai":
        return LLMSignalWeighter(**kwargs)
    elif backend == "rule_based":
        return RuleBasedSignalWeighter()
    else:
        raise ValueError(
            f"Unknown signal weighting backend: {backend!r}. "
            f"Supported: 'siliconflow', 'openai', 'rule_based'"
        )
