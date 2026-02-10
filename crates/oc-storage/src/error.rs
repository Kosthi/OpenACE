/// Storage errors.
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("tantivy error: {0}")]
    Tantivy(#[from] tantivy::TantivyError),

    #[error("vector index unavailable: {reason}")]
    VectorIndexUnavailable { reason: String },

    #[error("full-text index unavailable: {reason}")]
    FullTextIndexUnavailable { reason: String },

    #[error("dimension mismatch: expected {expected}, got {actual}")]
    DimensionMismatch { expected: usize, actual: usize },

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("transaction failed: {reason}")]
    TransactionFailed { reason: String },

    #[error("schema version mismatch: expected {expected}, found {actual}")]
    SchemaMismatch { expected: u32, actual: u32 },
}

impl StorageError {
    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::Sqlite(e) if is_sqlite_busy(e))
    }
}

fn is_sqlite_busy(err: &rusqlite::Error) -> bool {
    matches!(
        err,
        rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error {
                code: rusqlite::ErrorCode::DatabaseBusy,
                ..
            },
            _
        )
    )
}
