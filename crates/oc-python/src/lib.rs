use std::sync::OnceLock;

use pyo3::prelude::*;

mod engine;
mod types;
mod watcher;

use engine::EngineBinding;
use types::{PyChunkData, PyChunkInfo, PyFileInfo, PyIndexReport, PyRelation, PySearchResult, PySummaryChunk, PySymbol};
use watcher::WatcherBinding;

/// Initialize the global tracing subscriber (idempotent).
///
/// Reads `OPENACE_LOG_LEVEL` (default "warn") and `OPENACE_LOG_FORMAT`
/// ("json" or "pretty"/default) from environment variables.
/// All output goes to stderr.
pub fn init_tracing() {
    static INIT: OnceLock<()> = OnceLock::new();
    INIT.get_or_init(|| {
        use tracing_subscriber::{fmt, prelude::*, EnvFilter};

        let level_str = std::env::var("OPENACE_LOG_LEVEL").unwrap_or_else(|_| "warn".to_string());
        let filter = EnvFilter::try_new(&level_str).unwrap_or_else(|_| {
            eprintln!("openace: invalid OPENACE_LOG_LEVEL={level_str:?}, falling back to \"warn\"");
            EnvFilter::new("warn")
        });

        let format = std::env::var("OPENACE_LOG_FORMAT").unwrap_or_default();
        let is_json = format.eq_ignore_ascii_case("json");

        // Initialize log-crate bridge so libraries using `log` also emit tracing events
        let _ = tracing_log::LogTracer::init();

        if is_json {
            let layer = fmt::layer()
                .json()
                .with_writer(std::io::stderr)
                .with_target(true)
                .with_thread_ids(false);
            let _ = tracing_subscriber::registry()
                .with(filter)
                .with(layer)
                .try_init();
        } else {
            let layer = fmt::layer()
                .with_writer(std::io::stderr)
                .with_target(true)
                .with_thread_ids(false)
                .with_ansi(true);
            let _ = tracing_subscriber::registry()
                .with(filter)
                .with(layer)
                .try_init();
        }
    });
}

#[pymodule(name = "_openace")]
fn openace_module(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PySymbol>()?;
    m.add_class::<PySearchResult>()?;
    m.add_class::<PyIndexReport>()?;
    m.add_class::<PyRelation>()?;
    m.add_class::<PyChunkInfo>()?;
    m.add_class::<PyChunkData>()?;
    m.add_class::<PyFileInfo>()?;
    m.add_class::<PySummaryChunk>()?;
    m.add_class::<EngineBinding>()?;
    m.add_class::<WatcherBinding>()?;
    Ok(())
}
