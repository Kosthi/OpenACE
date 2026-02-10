/// Parser errors.
#[derive(Debug, thiserror::Error)]
pub enum ParserError {
    #[error("unsupported language for file: {path}")]
    UnsupportedLanguage { path: String },

    #[error("file too large ({size} bytes, max {max}): {path}")]
    FileTooLarge { path: String, size: u64, max: u64 },

    #[error("invalid encoding (non-UTF-8): {path}")]
    InvalidEncoding { path: String },

    #[error("parse failed for {path}: {reason}")]
    ParseFailed { path: String, reason: String },
}

impl ParserError {
    pub fn is_retryable(&self) -> bool {
        false
    }
}
