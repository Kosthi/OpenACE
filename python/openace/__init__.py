"""OpenACE - AI-native Contextual Code Engine."""

__version__ = "0.1.5"


def __getattr__(name):
    """Lazy imports to avoid importing Rust extension at module load."""
    if name == "Engine":
        from openace.engine import Engine
        return Engine
    if name in ("Symbol", "SearchResult", "IndexReport", "Relation"):
        from openace import types
        return getattr(types, name)
    if name in ("OpenACEError", "IndexingError", "SearchError", "StorageError"):
        from openace import exceptions
        return getattr(exceptions, name)
    raise AttributeError(f"module 'openace' has no attribute {name!r}")


__all__ = [
    "Engine",
    "IndexReport",
    "SearchResult",
    "Symbol",
    "Relation",
    "OpenACEError",
    "IndexingError",
    "SearchError",
    "StorageError",
]
