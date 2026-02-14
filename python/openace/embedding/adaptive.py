"""Adaptive concurrency strategy for embedding batches (AIMD)."""

from __future__ import annotations

import threading
import time
from collections import deque
from dataclasses import dataclass, field


@dataclass
class AdaptiveStrategy:
    """AIMD-based adaptive concurrency controller.

    Tracks success/failure of recent batches in a sliding window and adjusts
    the concurrency level:
    - Additive Increase: if success_rate > 0.95, concurrency += 1
    - Multiplicative Decrease: if success_rate < 0.70, concurrency //= 2

    Thread-safe: all mutations go through an internal lock.
    """

    concurrency: int
    max_concurrency: int
    min_concurrency: int = 1
    _window_size: int = 20
    _records: deque = field(default_factory=deque, repr=False)
    _lock: threading.Lock = field(default_factory=threading.Lock, repr=False)

    def record(self, success: bool, latency_ms: float = 0.0) -> None:
        """Record the outcome of a batch and maybe adjust concurrency."""
        with self._lock:
            self._records.append(success)
            if len(self._records) > self._window_size:
                self._records.popleft()
            self._maybe_adjust()

    def success_rate(self) -> float:
        """Return the success rate over the current window."""
        with self._lock:
            if not self._records:
                return 1.0
            return sum(self._records) / len(self._records)

    def current_concurrency(self) -> int:
        """Return the current concurrency level (thread-safe read)."""
        with self._lock:
            return self.concurrency

    def _maybe_adjust(self) -> None:
        """AIMD adjustment (caller must hold _lock)."""
        if len(self._records) < self.min_concurrency:
            return
        rate = sum(self._records) / len(self._records)
        if rate > 0.95 and self.concurrency < self.max_concurrency:
            self.concurrency += 1
        elif rate < 0.70 and self.concurrency > self.min_concurrency:
            self.concurrency = max(self.concurrency // 2, self.min_concurrency)


def make_strategy(provider: object) -> AdaptiveStrategy:
    """Create an AdaptiveStrategy tuned for the given provider type.

    - Local ONNX provider: concurrency=1, max=1 (single session, not thread-safe)
    - API providers (OpenAI, SiliconFlow, etc.): concurrency=2, max=8
    """
    from openace.embedding.local import OnnxEmbedder

    if isinstance(provider, OnnxEmbedder):
        return AdaptiveStrategy(concurrency=1, max_concurrency=1)
    return AdaptiveStrategy(concurrency=2, max_concurrency=8)
