use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crossbeam_channel::Receiver;
use notify::RecursiveMode;
use notify_debouncer_mini::{new_debouncer, DebounceEventResult, Debouncer};

use oc_parser::ParserRegistry;

use crate::error::IndexerError;
use crate::scanner::{GENERATED_PATTERNS, VENDOR_DIRS};

/// A change event emitted by the watcher after path filtering.
///
/// Note: these events are based on filesystem notification only. The consumer
/// must call `should_reindex` to check content hashes and skip metadata-only
/// changes before performing actual re-indexing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChangeEvent {
    /// File was created or modified with new content.
    Changed(PathBuf),
    /// File was deleted.
    Removed(PathBuf),
}

/// Handle to a running file watcher. Dropping it will stop the watcher.
pub struct WatcherHandle {
    /// The notify debouncer (stops on drop).
    _debouncer: Debouncer<notify::RecommendedWatcher>,
    /// Receiver for filtered change events.
    pub events: Receiver<ChangeEvent>,
    /// Project root for path resolution.
    project_root: PathBuf,
}

impl WatcherHandle {
    /// Stop watching and flush pending events.
    ///
    /// Returns all events that were still buffered in the channel.
    pub fn stop(self) -> Vec<ChangeEvent> {
        // Drop the debouncer to stop the watcher and its background thread.
        drop(self._debouncer);
        // Brief wait to allow the debouncer thread to flush pending events.
        std::thread::sleep(Duration::from_millis(50));
        // Drain any remaining events from the channel.
        let mut remaining = Vec::new();
        while let Ok(ev) = self.events.try_recv() {
            remaining.push(ev);
        }
        remaining
    }

    /// The project root this watcher monitors.
    pub fn project_root(&self) -> &Path {
        &self.project_root
    }
}

/// Start watching a project directory for source file changes.
///
/// Uses notify-rs with 300ms debounce. Change events are filtered by path
/// eligibility: vendor dirs, generated file patterns, hidden dirs, symlinks,
/// and unsupported extensions are rejected. Note that `.gitignore` rules are
/// NOT applied at the watcher level (OS-level watchers don't support this);
/// the consumer pipeline should apply gitignore filtering if needed.
///
/// Content-hash checking against stored hashes requires an external consumer
/// to perform the comparison since the watcher does not hold storage references.
/// The consumer should call `should_reindex` to skip metadata-only changes.
pub fn start_watching(project_path: &Path) -> Result<WatcherHandle, IndexerError> {
    let project_root = project_path
        .canonicalize()
        .map_err(|e| IndexerError::Watcher(format!("cannot canonicalize path: {e}")))?;

    let (tx, rx) = crossbeam_channel::bounded::<ChangeEvent>(4096);
    let root = Arc::new(project_root.clone());

    let root_clone = Arc::clone(&root);
    let tx_clone = tx.clone();

    let mut debouncer = new_debouncer(
        Duration::from_millis(300),
        move |result: DebounceEventResult| {
            let events = match result {
                Ok(events) => events,
                Err(_) => return,
            };

            for event in events {
                let path = &event.path;

                // Skip symlinks (scanner also skips them)
                if path.symlink_metadata().map_or(false, |m| m.file_type().is_symlink()) {
                    continue;
                }

                // Compute relative path from project root
                let rel_path = match path.strip_prefix(root_clone.as_ref()) {
                    Ok(r) => r,
                    Err(_) => continue,
                };

                // Apply filtering rules
                if !is_watchable_path(rel_path) {
                    continue;
                }

                let change = if path.exists() {
                    ChangeEvent::Changed(rel_path.to_path_buf())
                } else {
                    ChangeEvent::Removed(rel_path.to_path_buf())
                };

                // Best-effort send; if receiver is dropped, silently discard.
                let _ = tx_clone.send(change);
            }
        },
    )
    .map_err(|e| IndexerError::Watcher(format!("failed to create debouncer: {e}")))?;

    debouncer
        .watcher()
        .watch(&project_root, RecursiveMode::Recursive)
        .map_err(|e| IndexerError::Watcher(format!("failed to start watching: {e}")))?;

    Ok(WatcherHandle {
        _debouncer: debouncer,
        events: rx,
        project_root,
    })
}

/// Check if a file's content hash differs from the stored hash, indicating
/// it should be re-indexed. Returns `true` if re-indexing is needed.
///
/// `stored_hash` is the XXH3-64 hash from the `files` table.
/// `current_content` is the raw file bytes to hash and compare.
pub fn should_reindex(current_content: &[u8], stored_hash: u64) -> bool {
    let current_hash = xxhash_rust::xxh3::xxh3_64(current_content);
    current_hash != stored_hash
}

/// Check whether a relative path passes the watcher filter rules.
///
/// This applies the same rules as the scanner:
/// - Skip vendor directories
/// - Skip generated file patterns
/// - Skip hidden directories/files
/// - Only accept files with supported language extensions
fn is_watchable_path(rel_path: &Path) -> bool {
    // Check each component for vendor dirs and hidden dirs
    for component in rel_path.components() {
        if let std::path::Component::Normal(name) = component {
            let name_str = name.to_string_lossy();
            // Skip hidden directories/files (starting with '.')
            if name_str.starts_with('.') {
                return false;
            }
            // Skip vendor directories
            if VENDOR_DIRS.contains(&name_str.as_ref()) {
                return false;
            }
        }
    }

    // Skip generated file patterns
    if let Some(file_name) = rel_path.file_name() {
        let name = file_name.to_string_lossy();
        for pattern in GENERATED_PATTERNS {
            if name.contains(pattern) {
                return false;
            }
        }
    }

    // Only accept files with supported language extensions
    let ext = rel_path
        .extension()
        .map(|e| e.to_string_lossy().to_string())
        .unwrap_or_default();

    if ext.is_empty() {
        return false;
    }

    ParserRegistry::language_for_extension(&ext).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn is_watchable_accepts_source_files() {
        assert!(is_watchable_path(Path::new("src/main.py")));
        assert!(is_watchable_path(Path::new("lib.rs")));
        assert!(is_watchable_path(Path::new("index.ts")));
        assert!(is_watchable_path(Path::new("App.tsx")));
        assert!(is_watchable_path(Path::new("main.go")));
        assert!(is_watchable_path(Path::new("Main.java")));
    }

    #[test]
    fn is_watchable_rejects_vendor_dirs() {
        assert!(!is_watchable_path(Path::new("node_modules/dep/index.js")));
        assert!(!is_watchable_path(Path::new("vendor/lib/main.go")));
        assert!(!is_watchable_path(Path::new(".venv/lib/site.py")));
    }

    #[test]
    fn is_watchable_rejects_hidden_dirs() {
        assert!(!is_watchable_path(Path::new(".git/config")));
        assert!(!is_watchable_path(Path::new(".secret/key.py")));
    }

    #[test]
    fn is_watchable_rejects_generated_files() {
        assert!(!is_watchable_path(Path::new("schema.generated.ts")));
        assert!(!is_watchable_path(Path::new("bundle.min.js")));
        assert!(!is_watchable_path(Path::new("proto_pb2.py")));
        assert!(!is_watchable_path(Path::new("api.pb.go")));
    }

    #[test]
    fn is_watchable_rejects_unsupported_extensions() {
        assert!(!is_watchable_path(Path::new("readme.md")));
        assert!(!is_watchable_path(Path::new("data.json")));
        assert!(!is_watchable_path(Path::new("image.png")));
        assert!(!is_watchable_path(Path::new("Makefile")));
    }

    #[test]
    fn should_reindex_detects_content_change() {
        let content_v1 = b"fn main() { println!(\"hello\"); }";
        let hash_v1 = xxhash_rust::xxh3::xxh3_64(content_v1);

        // Same content — no re-index needed
        assert!(!should_reindex(content_v1, hash_v1));

        // Different content — re-index needed
        let content_v2 = b"fn main() { println!(\"world\"); }";
        assert!(should_reindex(content_v2, hash_v1));
    }

    #[test]
    fn should_reindex_metadata_only_change_ignored() {
        let content = b"x = 42\n";
        let stored_hash = xxhash_rust::xxh3::xxh3_64(content);

        // Content is identical even if the file was "touched"
        assert!(!should_reindex(content, stored_hash));
    }

    #[test]
    fn debounce_coalescing() {
        let tmp = tempfile::TempDir::new().unwrap();
        let src = tmp.path().join("src");
        fs::create_dir_all(&src).unwrap();
        let file = src.join("main.py");
        fs::write(&file, "x = 1").unwrap();

        let handle = start_watching(tmp.path()).unwrap();

        // Give the watcher time to start
        thread::sleep(Duration::from_millis(200));

        // Write to the same file 5 times in rapid succession
        for i in 0..5 {
            fs::write(&file, format!("x = {i}")).unwrap();
            thread::sleep(Duration::from_millis(10));
        }

        // Wait for debounce window + processing time
        thread::sleep(Duration::from_millis(800));

        // Collect all events
        let mut events = Vec::new();
        while let Ok(ev) = handle.events.try_recv() {
            events.push(ev);
        }

        // Should have coalesced into a small number of events (ideally 1-2, not 5)
        assert!(!events.is_empty(), "should have received at least one event");
        assert!(
            events.len() <= 3,
            "expected debounce coalescing, got {} events",
            events.len()
        );

        // All events should reference main.py
        for ev in &events {
            match ev {
                ChangeEvent::Changed(p) => {
                    assert!(p.to_string_lossy().contains("main.py"));
                }
                ChangeEvent::Removed(p) => {
                    assert!(p.to_string_lossy().contains("main.py"));
                }
            }
        }

        handle.stop();
    }

    #[test]
    fn filtered_path_ignored() {
        let tmp = tempfile::TempDir::new().unwrap();
        let nm = tmp.path().join("node_modules").join("dep");
        fs::create_dir_all(&nm).unwrap();
        let src = tmp.path().join("src");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("main.py"), "x = 1").unwrap();
        fs::write(nm.join("index.js"), "module.exports = {}").unwrap();

        let handle = start_watching(tmp.path()).unwrap();
        thread::sleep(Duration::from_millis(200));

        // Modify file in vendor dir — should be filtered
        fs::write(nm.join("index.js"), "module.exports = {v: 2}").unwrap();
        // Modify a non-source file — should be filtered
        fs::write(tmp.path().join("readme.md"), "# Hello").unwrap();
        // Modify a source file — should pass
        fs::write(src.join("main.py"), "x = 2").unwrap();

        thread::sleep(Duration::from_millis(800));

        let mut events = Vec::new();
        while let Ok(ev) = handle.events.try_recv() {
            events.push(ev);
        }

        // Only main.py should produce events
        for ev in &events {
            match ev {
                ChangeEvent::Changed(p) | ChangeEvent::Removed(p) => {
                    let p_str = p.to_string_lossy();
                    assert!(
                        !p_str.contains("node_modules"),
                        "vendor path should be filtered: {p_str}"
                    );
                    assert!(
                        !p_str.contains("readme.md"),
                        "non-source file should be filtered: {p_str}"
                    );
                }
            }
        }

        // Should have at least one event for main.py
        assert!(
            events.iter().any(|e| match e {
                ChangeEvent::Changed(p) => p.to_string_lossy().contains("main.py"),
                _ => false,
            }),
            "should have received an event for main.py"
        );

        handle.stop();
    }

    #[test]
    fn stop_drains_pending_events() {
        let tmp = tempfile::TempDir::new().unwrap();
        fs::write(tmp.path().join("app.py"), "x = 1").unwrap();

        let handle = start_watching(tmp.path()).unwrap();
        thread::sleep(Duration::from_millis(200));

        // Write and immediately stop
        fs::write(tmp.path().join("app.py"), "x = 2").unwrap();
        thread::sleep(Duration::from_millis(500));

        let remaining = handle.stop();
        // We don't assert specific count, just that stop() doesn't panic
        // and returns a vec (may be empty if events were already consumed)
        let _ = remaining;
    }
}
