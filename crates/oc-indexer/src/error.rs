/// Indexer errors.
#[derive(Debug, thiserror::Error)]
pub enum IndexerError {
    #[error("parser error: {0}")]
    Parser(#[from] oc_parser::error::ParserError),

    #[error("storage error: {0}")]
    Storage(#[from] oc_storage::error::StorageError),

    #[error("watcher error: {0}")]
    Watcher(String),

    #[error("pipeline failed at stage '{stage}': {reason}")]
    PipelineFailed { stage: String, reason: String },

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

impl IndexerError {
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::Storage(e) => e.is_retryable(),
            _ => false,
        }
    }
}
