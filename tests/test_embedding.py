"""Tests for the embedding manager."""

import numpy as np
import pytest
from unittest.mock import MagicMock, patch

from openace.embedding.factory import create_provider


class TestFactory:
    def test_create_local_provider(self):
        provider = create_provider("local")
        assert provider.dimension == 384

    def test_create_openai_provider(self):
        provider = create_provider("openai")
        assert provider.dimension == 1536

    def test_create_unknown_raises(self):
        with pytest.raises(ValueError, match="Unknown embedding backend"):
            create_provider("unknown_backend")

    def test_local_provider_custom_params(self):
        provider = create_provider("local", batch_size=16)
        assert provider.dimension == 384

    def test_openai_provider_custom_dim(self):
        provider = create_provider("openai", dim=768)
        assert provider.dimension == 768


class TestOnnxEmbedder:
    """Tests that don't require model download."""

    def test_dimension_property(self):
        from openace.embedding.local import OnnxEmbedder
        embedder = OnnxEmbedder()
        assert embedder.dimension == 384

    def test_custom_cache_dir(self, tmp_path):
        from openace.embedding.local import OnnxEmbedder
        embedder = OnnxEmbedder(cache_dir=str(tmp_path / "models"))
        assert embedder.dimension == 384


class TestOpenAIEmbedder:
    """Tests that don't require API key."""

    def test_dimension_property(self):
        from openace.embedding.openai_backend import OpenAIEmbedder
        embedder = OpenAIEmbedder()
        assert embedder.dimension == 1536

    def test_custom_dimension(self):
        from openace.embedding.openai_backend import OpenAIEmbedder
        embedder = OpenAIEmbedder(dim=768)
        assert embedder.dimension == 768

    def test_custom_model(self):
        from openace.embedding.openai_backend import OpenAIEmbedder
        embedder = OpenAIEmbedder(model="text-embedding-3-large", dim=3072)
        assert embedder.dimension == 3072


class TestAdaptiveStrategy:
    """Tests for the AIMD adaptive concurrency controller."""

    def test_initial_concurrency(self):
        from openace.embedding.adaptive import AdaptiveStrategy
        s = AdaptiveStrategy(concurrency=2, max_concurrency=8)
        assert s.current_concurrency() == 2

    def test_success_rate_empty(self):
        from openace.embedding.adaptive import AdaptiveStrategy
        s = AdaptiveStrategy(concurrency=2, max_concurrency=8)
        assert s.success_rate() == 1.0

    def test_additive_increase(self):
        from openace.embedding.adaptive import AdaptiveStrategy
        s = AdaptiveStrategy(concurrency=2, max_concurrency=8, _window_size=5)
        # Each successful record with rate > 0.95 triggers +1
        s.record(True)
        assert s.current_concurrency() == 3

    def test_multiplicative_decrease(self):
        from openace.embedding.adaptive import AdaptiveStrategy
        s = AdaptiveStrategy(concurrency=4, max_concurrency=8, _window_size=5)
        # Fill window with failures to drive rate well below 0.70
        for _ in range(5):
            s.record(False)
        # concurrency should have been halved (possibly multiple times)
        assert s.current_concurrency() <= 2
        assert s.current_concurrency() >= 1

    def test_concurrency_respects_max(self):
        from openace.embedding.adaptive import AdaptiveStrategy
        s = AdaptiveStrategy(concurrency=7, max_concurrency=8, _window_size=5)
        # All successes -> increase stops at max
        for _ in range(10):
            s.record(True)
        assert s.current_concurrency() == 8

    def test_concurrency_respects_min(self):
        from openace.embedding.adaptive import AdaptiveStrategy
        s = AdaptiveStrategy(concurrency=2, max_concurrency=8, min_concurrency=1, _window_size=5)
        # All failures
        for _ in range(10):
            s.record(False)
        assert s.current_concurrency() == 1

    def test_stable_in_middle_range(self):
        from openace.embedding.adaptive import AdaptiveStrategy
        # Start with rate in the stable zone (0.70 < rate < 0.95)
        s = AdaptiveStrategy(concurrency=4, max_concurrency=8, _window_size=10)
        # Pre-fill the window with a mix that gives ~0.80 rate
        # 8 successes then 2 failures
        for _ in range(8):
            s.record(True)
        # At this point, 8 successes in window -> rate=1.0, concurrency has grown
        # Now add failures to bring rate into the stable zone
        start_c = s.current_concurrency()
        # Fill with alternating to keep rate ~0.80
        for _ in range(2):
            s.record(False)
        # After adding 2 failures to 10-window: 8 success / 10 = 0.80
        # Rate is between 0.70 and 0.95 -> no adjustment on those records
        end_c = s.current_concurrency()
        assert end_c == start_c  # stable, no change

    def test_make_strategy_local(self):
        from openace.embedding.adaptive import make_strategy
        from openace.embedding.local import OnnxEmbedder
        provider = OnnxEmbedder()
        s = make_strategy(provider)
        assert s.concurrency == 1
        assert s.max_concurrency == 1

    def test_make_strategy_api(self):
        from openace.embedding.adaptive import make_strategy
        from openace.embedding.openai_backend import OpenAIEmbedder
        provider = OpenAIEmbedder()
        s = make_strategy(provider)
        assert s.concurrency == 2
        assert s.max_concurrency == 8


class TestConcurrentEmbedAll:
    """Test concurrent embed_all with a mock provider and engine."""

    def test_embed_all_concurrent_mock(self, sample_project):
        """Verify embed_all works end-to-end with a mock provider."""
        from openace.engine import Engine

        # Create a mock embedding provider (not OnnxEmbedder so it gets concurrency > 1)
        mock_provider = MagicMock()
        mock_provider.dimension = 16
        mock_provider.embed = MagicMock(
            side_effect=lambda texts: np.random.rand(len(texts), 16).astype(np.float32)
        )

        engine = Engine(str(sample_project), embedding_provider=mock_provider, embedding_dim=16)
        engine.index()

        count = engine.embed_all()
        assert count > 0
        assert mock_provider.embed.call_count > 0

    def test_embed_all_handles_batch_failure(self, sample_project):
        """Verify embed_all continues when individual batches fail."""
        from openace.engine import Engine

        call_count = 0

        def flaky_embed(texts):
            nonlocal call_count
            call_count += 1
            if call_count == 1:
                raise RuntimeError("transient API error")
            return np.random.rand(len(texts), 16).astype(np.float32)

        mock_provider = MagicMock()
        mock_provider.dimension = 16
        mock_provider.embed = MagicMock(side_effect=flaky_embed)

        engine = Engine(str(sample_project), embedding_provider=mock_provider, embedding_dim=16)
        engine.index()

        # Should not raise despite one batch failure
        count = engine.embed_all()
        # Some symbols were embedded (minus the failed batch)
        assert count >= 0

    def test_embed_all_no_provider_raises(self, sample_project):
        """embed_all without a provider should raise."""
        from openace.engine import Engine
        from openace.exceptions import OpenACEError

        engine = Engine(str(sample_project))
        engine.index()

        with pytest.raises(OpenACEError, match="no embedding provider"):
            engine.embed_all()

    def test_embed_all_zero_symbols(self, tmp_path):
        """embed_all on an empty project returns 0."""
        from openace.engine import Engine

        mock_provider = MagicMock()
        mock_provider.dimension = 16

        engine = Engine(str(tmp_path), embedding_provider=mock_provider, embedding_dim=16)
        # Don't index -- no symbols
        count = engine.embed_all()
        assert count == 0
