"""OpenACE exception hierarchy."""


class OpenACEError(Exception):
    """Base exception for all OpenACE errors."""
    pass


class IndexingError(OpenACEError):
    """Error during indexing operations."""
    pass


class SearchError(OpenACEError):
    """Error during search operations."""
    pass


class StorageError(OpenACEError):
    """Error from the storage layer."""
    pass
