pub mod chunker;
pub mod error;
mod body_hash;
mod file_check;
mod registry;
mod visitor;

pub use chunker::{chunk_file, ChunkConfig};
pub use file_check::{check_file_size, is_binary};
pub use registry::ParserRegistry;
pub use visitor::{parse_file, parse_file_with_tree, ParseOutput, ParseOutputWithTree};
