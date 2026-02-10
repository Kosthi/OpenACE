pub mod error;
mod body_hash;
mod file_check;
mod registry;
mod visitor;

pub use file_check::{check_file_size, is_binary};
pub use registry::ParserRegistry;
pub use visitor::{parse_file, ParseOutput};
