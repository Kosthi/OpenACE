use std::path::PathBuf;
use std::time::Duration;

use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;

use oc_indexer::watcher::{start_watching, ChangeEvent, WatcherHandle};

/// Python-facing file watcher binding.
#[pyclass]
pub struct WatcherBinding {
    handle: Option<WatcherHandle>,
    project_root: PathBuf,
}

#[pymethods]
impl WatcherBinding {
    /// Start watching a project directory for source file changes.
    #[staticmethod]
    fn start(project_root: &str) -> PyResult<Self> {
        let path = PathBuf::from(project_root);
        let handle = start_watching(&path)
            .map_err(|e| PyRuntimeError::new_err(format!("failed to start watcher: {e}")))?;

        Ok(Self {
            handle: Some(handle),
            project_root: path,
        })
    }

    /// Poll for pending change events with a timeout.
    ///
    /// Returns a list of `(event_type, path)` tuples where `event_type` is
    /// `"changed"` or `"removed"`.
    fn poll(&self, py: Python<'_>, timeout_ms: u64) -> PyResult<Vec<(String, String)>> {
        let handle = self.handle.as_ref().ok_or_else(|| {
            PyRuntimeError::new_err("watcher has been stopped")
        })?;

        let mut events = Vec::new();

        py.allow_threads(|| {
            let deadline = std::time::Instant::now() + Duration::from_millis(timeout_ms);

            loop {
                match handle.events.try_recv() {
                    Ok(event) => {
                        events.push(change_event_to_tuple(&event));
                        // Drain any additional pending events.
                        while let Ok(event) = handle.events.try_recv() {
                            events.push(change_event_to_tuple(&event));
                        }
                        break;
                    }
                    Err(_) => {
                        if std::time::Instant::now() >= deadline {
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(50));
                    }
                }
            }
        });

        Ok(events)
    }

    /// Stop the watcher and return any remaining buffered events.
    fn stop(&mut self) -> PyResult<Vec<(String, String)>> {
        let handle = self.handle.take().ok_or_else(|| {
            PyRuntimeError::new_err("watcher has already been stopped")
        })?;

        let remaining = handle.stop();
        Ok(remaining.iter().map(change_event_to_tuple).collect())
    }

    /// The project root being watched.
    #[getter]
    fn project_root(&self) -> &str {
        self.project_root.to_str().unwrap_or("")
    }
}

fn change_event_to_tuple(event: &ChangeEvent) -> (String, String) {
    match event {
        ChangeEvent::Changed(p) => ("changed".to_string(), p.to_string_lossy().into_owned()),
        ChangeEvent::Removed(p) => ("removed".to_string(), p.to_string_lossy().into_owned()),
    }
}
