"""Tests for the embedding manager."""

import pytest

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
