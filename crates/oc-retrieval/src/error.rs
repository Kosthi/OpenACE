/// Retrieval errors.
#[derive(Debug, thiserror::Error)]
pub enum RetrievalError {
    #[error("storage error: {0}")]
    Storage(#[from] oc_storage::error::StorageError),

    #[error("query error: {reason}")]
    QueryFailed { reason: String },

    #[error("fusion error: {reason}")]
    FusionFailed { reason: String },

    #[error("graph expansion failed: {reason}")]
    ExpansionFailed { reason: String },
}

impl RetrievalError {
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::Storage(e) => e.is_retryable(),
            _ => false,
        }
    }
}
