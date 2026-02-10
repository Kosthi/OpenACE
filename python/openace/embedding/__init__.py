"""Embedding providers for OpenACE."""

from openace.embedding.protocol import EmbeddingProvider
from openace.embedding.factory import create_provider

__all__ = ["EmbeddingProvider", "create_provider"]
