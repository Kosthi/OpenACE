"""Local ONNX Runtime embedding provider."""

from __future__ import annotations

import os
from pathlib import Path
from typing import Optional


class OnnxEmbedder:
    """Local embedding using ONNX Runtime with all-MiniLM-L6-v2 (384-dim).

    Model is lazily downloaded on first use to ~/.cache/openace/models/.
    Requires: pip install openace[onnx]
    """

    MODEL_NAME = "sentence-transformers/all-MiniLM-L6-v2"
    _DIMENSION = 384

    def __init__(self, *, cache_dir: Optional[str] = None, batch_size: int = 32):
        self._cache_dir = Path(cache_dir or os.path.expanduser("~/.cache/openace/models"))
        self._batch_size = batch_size
        self._session = None
        self._tokenizer = None

    @property
    def dimension(self) -> int:
        return self._DIMENSION

    def _load_model(self):
        """Lazily load the ONNX model and tokenizer."""
        if self._session is not None:
            return

        try:
            import onnxruntime as ort
            from tokenizers import Tokenizer
        except ImportError:
            raise ImportError(
                "ONNX embedding requires onnxruntime and tokenizers. "
                "Install with: pip install openace[onnx]"
            )

        model_dir = self._cache_dir / "all-MiniLM-L6-v2"
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
                repo_id=self.MODEL_NAME,
                filename=filename,
                local_dir=str(model_dir),
                local_dir_use_symlinks=False,
            )

    def embed(self, texts: list[str]) -> "numpy.ndarray":
        """Embed texts using local ONNX model.

        Args:
            texts: List of text strings to embed.

        Returns:
            numpy array of shape (len(texts), 384), dtype float32.
        """
        import numpy as np

        self._load_model()

        all_embeddings = []
        for i in range(0, len(texts), self._batch_size):
            batch = texts[i : i + self._batch_size]
            batch_embeddings = self._embed_batch(batch)
            all_embeddings.append(batch_embeddings)

        if not all_embeddings:
            return np.zeros((0, self._DIMENSION), dtype=np.float32)

        return np.vstack(all_embeddings)

    def _embed_batch(self, texts: list[str]) -> "numpy.ndarray":
        """Embed a single batch."""
        import numpy as np

        encoded = self._tokenizer.encode_batch(texts)

        input_ids = np.array([e.ids for e in encoded], dtype=np.int64)
        attention_mask = np.array([e.attention_mask for e in encoded], dtype=np.int64)
        token_type_ids = np.zeros_like(input_ids)

        outputs = self._session.run(
            None,
            {
                "input_ids": input_ids,
                "attention_mask": attention_mask,
                "token_type_ids": token_type_ids,
            },
        )

        # Mean pooling over token embeddings
        token_embeddings = outputs[0]  # (batch, seq_len, hidden_dim)
        mask_expanded = attention_mask[:, :, np.newaxis].astype(np.float32)
        sum_embeddings = np.sum(token_embeddings * mask_expanded, axis=1)
        sum_mask = np.clip(mask_expanded.sum(axis=1), a_min=1e-9, a_max=None)
        mean_embeddings = sum_embeddings / sum_mask

        # L2 normalize
        norms = np.linalg.norm(mean_embeddings, axis=1, keepdims=True)
        norms = np.clip(norms, a_min=1e-9, a_max=None)
        normalized = mean_embeddings / norms

        return normalized.astype(np.float32)
