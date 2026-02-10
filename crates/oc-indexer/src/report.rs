use std::collections::HashMap;
use std::time::Duration;

/// Configuration for the indexing pipeline.
pub struct IndexConfig {
    /// Repository identifier for SymbolId generation.
    pub repo_id: String,
    /// Batch size for SQLite bulk inserts (default: 1000).
    pub batch_size: usize,
}

impl Default for IndexConfig {
    fn default() -> Self {
        Self {
            repo_id: String::new(),
            batch_size: 1000,
        }
    }
}

/// Reason why a file was skipped during indexing.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SkipReason {
    TooLarge,
    Binary,
    UnsupportedLanguage,
    Ignored,
}

/// Report generated after a full indexing run.
#[derive(Debug)]
pub struct IndexReport {
    pub total_files_scanned: usize,
    pub files_indexed: usize,
    pub files_skipped: HashMap<SkipReason, usize>,
    pub files_failed: usize,
    pub failed_details: Vec<(String, String)>,
    pub total_symbols: usize,
    pub total_relations: usize,
    pub duration: Duration,
}

impl IndexReport {
    pub fn total_skipped(&self) -> usize {
        self.files_skipped.values().sum()
    }
}
