"""Cross-encoder reranker using local ONNX Runtime inference."""

from __future__ import annotations

import os
import threading
from dataclasses import replace
from pathlib import Path
from typing import Optional

from openace.types import SearchResult


class CrossEncoderReranker:
    """Rerank search results using a cross-encoder model via ONNX Runtime.

    Uses cross-encoder/ms-marco-MiniLM-L-6-v2 by default. The model is lazily
    downloaded on first use to ~/.cache/openace/models/.

    Requires: pip install openace[rerank-local]
    """

    MODEL_NAME = "cross-encoder/ms-marco-MiniLM-L-6-v2"

    def __init__(
        self,
        *,
        model_name: str = "cross-encoder/ms-marco-MiniLM-L-6-v2",
        cache_dir: Optional[str] = None,
        batch_size: int = 32,
    ):
        self._model_name = model_name
        self._cache_dir = Path(cache_dir or os.path.expanduser("~/.cache/openace/models"))
        self._batch_size = batch_size
        self._session = None
        self._tokenizer = None
        self._lock = threading.Lock()

    def _load_model(self):
        """Lazily load the ONNX model and tokenizer."""
        if self._session is not None:
            return

        with self._lock:
            if self._session is not None:
                return

            try:
                import numpy as np  # noqa: F401 — verify availability
                import onnxruntime as ort
                from tokenizers import Tokenizer
            except ImportError:
                raise ImportError(
                    "Cross-encoder reranking requires onnxruntime, tokenizers, and numpy. "
                    "Install with: pip install openace[rerank-local]"
                )

            # Derive directory name from model (e.g. "ms-marco-MiniLM-L-6-v2")
            model_dir_name = self._model_name.split("/")[-1]
            model_dir = self._cache_dir / model_dir_name
            model_path = model_dir / "model.onnx"
            tokenizer_path = model_dir / "tokenizer.json"

            if not model_path.exists() or not tokenizer_path.exists():
                self._download_model(model_dir)

            self._tokenizer = Tokenizer.from_file(str(tokenizer_path))
            self._tokenizer.enable_truncation(max_length=512)
            self._tokenizer.enable_padding(length=512)

            sess_options = ort.SessionOptions()
            sess_options.graph_optimization_level = ort.GraphOptimizationLevel.ORT_ENABLE_ALL
            self._session = ort.InferenceSession(str(model_path), sess_options)

    def _download_model(self, model_dir: Path):
        """Download model files from Hugging Face Hub."""
        try:
            from huggingface_hub import hf_hub_download
        except ImportError:
            raise ImportError(
                "Model download requires huggingface_hub. "
                "Install with: pip install huggingface_hub"
            )

        model_dir.mkdir(parents=True, exist_ok=True)

        for filename in ["model.onnx", "tokenizer.json"]:
            hf_hub_download(
                repo_id=self._model_name,
                filename=filename,
                local_dir=str(model_dir),
                local_dir_use_symlinks=False,
            )

    @staticmethod
    def _build_document_text(result: SearchResult) -> str:
        """Build a text representation of a search result for the cross-encoder."""
        return f"{result.qualified_name} ({result.kind}) {result.name}"

    def rerank(
        self, query: str, results: list[SearchResult], *, top_k: int | None = None
    ) -> list[SearchResult]:
        """Rerank search results by relevance to the query.

        Args:
            query: The search query to rerank against.
            results: List of search results to rerank.
            top_k: If provided, return only the top-k results.

        Returns:
            Reranked list of search results with rerank_score set.
        """
        if not results:
            return []

        self._load_model()

        import numpy as np

        pairs = [(query, self._build_document_text(r)) for r in results]

        # Score all pairs in batches
        all_scores: list[float] = []
        for i in range(0, len(pairs), self._batch_size):
            batch = pairs[i : i + self._batch_size]
            batch_scores = self._score_batch(batch)
            all_scores.extend(batch_scores)

        # Attach rerank_score to each result
        scored = [
            replace(result, rerank_score=float(score))
            for result, score in zip(results, all_scores)
        ]

        # Sort by rerank_score descending
        scored.sort(key=lambda r: r.rerank_score, reverse=True)

        if top_k is not None:
            scored = scored[:top_k]

        return scored

    def _score_batch(self, pairs: list[tuple[str, str]]) -> list[float]:
        """Score a batch of (query, document) pairs using the cross-encoder."""
        import numpy as np

        encoded = self._tokenizer.encode_batch(
            [list(pair) for pair in pairs]
        )

        input_ids = np.array([e.ids for e in encoded], dtype=np.int64)
        attention_mask = np.array([e.attention_mask for e in encoded], dtype=np.int64)
        token_type_ids = np.array([e.type_ids for e in encoded], dtype=np.int64)

        outputs = self._session.run(
            None,
            {
                "input_ids": input_ids,
                "attention_mask": attention_mask,
                "token_type_ids": token_type_ids,
            },
        )

        # Cross-encoder outputs logits of shape (batch_size, 1) or (batch_size,)
        logits = outputs[0]
        if logits.ndim == 2:
            logits = logits[:, 0]

        return logits.tolist()
