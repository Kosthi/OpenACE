use std::path::{Path, PathBuf};

use ignore::WalkBuilder;

/// Generated file patterns to skip.
pub(crate) const GENERATED_PATTERNS: &[&str] = &[
    ".generated.",
    ".min.js",
    ".min.css",
    "_pb2.py",
    ".pb.go",
];

/// Vendor directories to skip.
pub(crate) const VENDOR_DIRS: &[&str] = &[
    "vendor",
    "node_modules",
    "third_party",
    ".venv",
    "venv",
];

/// Result of scanning a project directory for source files.
pub struct ScanResult {
    /// Paths relative to project root (forward-slash normalized).
    pub files: Vec<PathBuf>,
    /// Total entries seen (including skipped).
    pub total_entries: usize,
}

/// Scan a project directory for indexable source files.
///
/// Uses the `ignore` crate for .gitignore-aware walking.
/// Applies additional filtering: vendor dirs, generated patterns, hidden dirs, symlinks.
pub fn scan_files(project_root: &Path) -> ScanResult {
    let mut files = Vec::new();
    let mut total_entries = 0usize;

    let walker = WalkBuilder::new(project_root)
        .hidden(true) // skip hidden files/dirs
        .git_ignore(true) // respect .gitignore
        .git_global(true)
        .git_exclude(true)
        .follow_links(false) // skip symlinks
        .filter_entry(|entry| {
            // Prune vendor directories at the directory level
            if entry.file_type().map_or(false, |ft| ft.is_dir()) {
                if let Some(name) = entry.file_name().to_str() {
                    return !VENDOR_DIRS.contains(&name);
                }
            }
            true
        })
        .build();

    for entry in walker {
        // Skip entries that produce errors (permission denied, etc.)
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };

        total_entries += 1;

        // Only process regular files
        let ft = match entry.file_type() {
            Some(ft) => ft,
            None => continue,
        };
        if !ft.is_file() {
            continue;
        }

        let path = entry.path();

        // Skip generated file patterns
        if is_generated_file(path) {
            continue;
        }

        // Compute relative path (forward-slash normalized)
        let rel = match path.strip_prefix(project_root) {
            Ok(r) => r.to_path_buf(),
            Err(_) => continue,
        };

        files.push(rel);
    }

    ScanResult {
        files,
        total_entries,
    }
}

/// Check if a filename matches generated file patterns.
fn is_generated_file(path: &Path) -> bool {
    let name = match path.file_name() {
        Some(n) => n.to_string_lossy(),
        None => return false,
    };

    for pattern in GENERATED_PATTERNS {
        if name.contains(pattern) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn scan_empty_dir() {
        let tmp = tempfile::TempDir::new().unwrap();
        let result = scan_files(tmp.path());
        assert!(result.files.is_empty());
    }

    #[test]
    fn scan_finds_source_files() {
        let tmp = tempfile::TempDir::new().unwrap();
        let src = tmp.path().join("src");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("main.py"), "print('hello')").unwrap();
        fs::write(src.join("lib.rs"), "fn main() {}").unwrap();

        let result = scan_files(tmp.path());
        assert_eq!(result.files.len(), 2);
    }

    #[test]
    fn scan_skips_vendor_dirs() {
        let tmp = tempfile::TempDir::new().unwrap();
        fs::write(tmp.path().join("app.py"), "x = 1").unwrap();
        let nm = tmp.path().join("node_modules").join("dep");
        fs::create_dir_all(&nm).unwrap();
        fs::write(nm.join("index.js"), "module.exports = {}").unwrap();

        let result = scan_files(tmp.path());
        assert_eq!(result.files.len(), 1);
        assert!(result.files[0].to_string_lossy().contains("app.py"));
    }

    #[test]
    fn scan_skips_generated_files() {
        let tmp = tempfile::TempDir::new().unwrap();
        fs::write(tmp.path().join("app.py"), "x = 1").unwrap();
        fs::write(tmp.path().join("schema.generated.ts"), "export {}").unwrap();
        fs::write(tmp.path().join("bundle.min.js"), "var x").unwrap();
        fs::write(tmp.path().join("proto_pb2.py"), "# gen").unwrap();
        fs::write(tmp.path().join("api.pb.go"), "package api").unwrap();

        let result = scan_files(tmp.path());
        assert_eq!(result.files.len(), 1);
    }

    #[test]
    fn scan_skips_hidden_dirs() {
        let tmp = tempfile::TempDir::new().unwrap();
        fs::write(tmp.path().join("app.py"), "x = 1").unwrap();
        let hidden = tmp.path().join(".secret");
        fs::create_dir_all(&hidden).unwrap();
        fs::write(hidden.join("key.py"), "KEY = 42").unwrap();

        let result = scan_files(tmp.path());
        assert_eq!(result.files.len(), 1);
    }

    #[test]
    fn scan_respects_gitignore() {
        let tmp = tempfile::TempDir::new().unwrap();
        // Initialize a git repo so .gitignore is respected
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(tmp.path())
            .output()
            .ok();
        fs::write(tmp.path().join(".gitignore"), "build/\n").unwrap();
        fs::write(tmp.path().join("app.py"), "x = 1").unwrap();
        let build = tmp.path().join("build");
        fs::create_dir_all(&build).unwrap();
        fs::write(build.join("output.js"), "var x").unwrap();

        let result = scan_files(tmp.path());
        // Should find app.py and .gitignore, but NOT build/output.js
        let names: Vec<String> = result.files.iter().map(|p| p.to_string_lossy().to_string()).collect();
        assert!(names.iter().any(|n| n.contains("app.py")));
        assert!(!names.iter().any(|n| n.contains("output.js")));
    }
}
