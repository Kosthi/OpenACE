use pyo3::prelude::*;

mod engine;
mod types;
mod watcher;

use engine::EngineBinding;
use types::{PyChunkData, PyChunkInfo, PyFileInfo, PyIndexReport, PyRelation, PySearchResult, PySummaryChunk, PySymbol};
use watcher::WatcherBinding;

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
