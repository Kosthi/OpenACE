pub mod error;
pub mod incremental;
pub mod pipeline;
pub mod report;
pub mod resolver;
pub mod scanner;
pub mod watcher;

pub use incremental::{
    delete_file as incremental_delete, diff_symbols, process_events, update_file,
    IncrementalReport, SymbolDiff,
};
pub use oc_parser::{ChunkConfig, chunk_file};
pub use pipeline::{index, index_incremental, IncrementalIndexResult};
pub use report::{IndexConfig, IndexReport, SkipReason};
pub use resolver::ResolutionStats;
pub use scanner::scan_files;
pub use watcher::{start_watching, ChangeEvent, WatcherHandle};
