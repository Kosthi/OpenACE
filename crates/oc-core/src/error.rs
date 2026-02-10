use std::path::PathBuf;

/// Errors from oc-core operations.
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("hash computation failed for {path}: {reason}")]
    HashFailed { path: PathBuf, reason: String },

    #[error("invalid ordinal {ordinal} for {type_name}")]
    InvalidOrdinal {
        type_name: &'static str,
        ordinal: u8,
    },

    #[error("type conversion failed: {reason}")]
    ConversionFailed { reason: String },
}

impl CoreError {
    pub fn is_retryable(&self) -> bool {
        false
    }
}
