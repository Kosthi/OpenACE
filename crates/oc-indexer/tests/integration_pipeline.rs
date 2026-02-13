use oc_indexer::{index, IndexConfig, SkipReason};
use std::fs;
use tempfile::TempDir;

fn create_fixture_project(root: &std::path::Path) {
    // Python file
    let py_dir = root.join("src");
    fs::create_dir_all(&py_dir).unwrap();
    fs::write(
        py_dir.join("main.py"),
        r#"
class UserService:
    """Manages user operations."""

    def create_user(self, name: str) -> dict:
        return {"name": name}

    def delete_user(self, user_id: int) -> bool:
        return True

def process_request(data):
    svc = UserService()
    return svc.create_user(data["name"])
"#,
    )
    .unwrap();

    // TypeScript file
    fs::write(
        py_dir.join("app.ts"),
        r#"
interface Config {
    host: string;
    port: number;
}

class Server {
    private config: Config;

    constructor(config: Config) {
        this.config = config;
    }

    start(): void {
        console.log(`Listening on ${this.config.host}:${this.config.port}`);
    }
}

function createServer(config: Config): Server {
    return new Server(config);
}
"#,
    )
    .unwrap();

    // Rust file
    fs::write(
        py_dir.join("lib.rs"),
        r#"
pub struct Engine {
    name: String,
}

impl Engine {
    pub fn new(name: &str) -> Self {
        Self { name: name.to_string() }
    }

    pub fn run(&self) -> bool {
        !self.name.is_empty()
    }
}

pub fn initialize() -> Engine {
    Engine::new("default")
}
"#,
    )
    .unwrap();

    // Go file
    fs::write(
        py_dir.join("handler.go"),
        r#"
package main

type Handler struct {
    Name string
}

func NewHandler(name string) *Handler {
    return &Handler{Name: name}
}

func (h *Handler) Handle() string {
    return h.Name
}
"#,
    )
    .unwrap();

    // Java file
    fs::write(
        py_dir.join("App.java"),
        r#"
package com.example;

public class App {
    private String name;

    public App(String name) {
        this.name = name;
    }

    public String getName() {
        return this.name;
    }

    public static void main(String[] args) {
        App app = new App("test");
        System.out.println(app.getName());
    }
}
"#,
    )
    .unwrap();

    // Binary file (should be skipped)
    let mut binary = vec![0u8; 100];
    binary[0] = 0xFF;
    binary[1] = 0xD8; // JPEG-like header
    binary[10] = 0x00; // null byte
    fs::write(py_dir.join("image.dat"), &binary).unwrap();

    // Unsupported language file
    fs::write(py_dir.join("style.css"), "body { color: red; }").unwrap();

    // Generated file (should be skipped by scanner)
    fs::write(py_dir.join("schema.generated.ts"), "export interface Schema {}").unwrap();

    // Vendor dir (should be skipped by scanner)
    let vendor = root.join("node_modules").join("dep");
    fs::create_dir_all(&vendor).unwrap();
    fs::write(vendor.join("index.js"), "module.exports = {}").unwrap();
}

#[test]
fn integration_index_mixed_language_project() {
    let tmp = TempDir::new().unwrap();
    create_fixture_project(tmp.path());

    let config = IndexConfig {
        repo_id: "test-repo".to_string(),
        batch_size: 1000,
        embedding_dim: 384,
        ..Default::default()
    };

    let report = index(tmp.path(), &config).unwrap();

    // 5 language files should be indexed (Python, TS, Rust, Go, Java)
    assert_eq!(
        report.files_indexed, 5,
        "Expected 5 indexed files, got {}. Failed: {:?}",
        report.files_indexed, report.failed_details
    );

    // Binary file should be skipped
    let binary_skipped = report
        .files_skipped
        .get(&SkipReason::Binary)
        .copied()
        .unwrap_or(0);
    assert!(binary_skipped >= 1, "Expected at least 1 binary skip");

    // Unsupported language (CSS) should be skipped
    let unsupported_skipped = report
        .files_skipped
        .get(&SkipReason::UnsupportedLanguage)
        .copied()
        .unwrap_or(0);
    assert!(
        unsupported_skipped >= 1,
        "Expected at least 1 unsupported language skip"
    );

    // Should have extracted symbols from all 5 files
    assert!(
        report.total_symbols > 0,
        "Expected symbols, got 0"
    );

    // Should have extracted some relations (calls, contains, etc.)
    assert!(
        report.total_relations > 0,
        "Expected relations, got 0"
    );

    // No files should have failed
    assert_eq!(
        report.files_failed, 0,
        "Expected 0 failures, got {}: {:?}",
        report.files_failed, report.failed_details
    );

    // Verify the .openace directory was created
    assert!(tmp.path().join(".openace").exists());
    assert!(tmp.path().join(".openace").join("db.sqlite").exists());
    assert!(tmp.path().join(".openace").join("tantivy").exists());
}

#[test]
fn integration_index_search_results() {
    let tmp = TempDir::new().unwrap();
    create_fixture_project(tmp.path());

    let config = IndexConfig {
        repo_id: "test-repo".to_string(),
        batch_size: 1000,
        embedding_dim: 384,
        ..Default::default()
    };

    let _report = index(tmp.path(), &config).unwrap();

    // Open storage and verify we can search
    let storage = oc_storage::manager::StorageManager::open(tmp.path()).unwrap();

    // Search for Python symbols by file
    let py_symbols = storage.graph().get_symbols_by_file("src/main.py").unwrap();
    assert!(
        !py_symbols.is_empty(),
        "Expected Python symbols in src/main.py"
    );

    // Search for Rust symbols by file
    let rs_symbols = storage.graph().get_symbols_by_file("src/lib.rs").unwrap();
    assert!(
        !rs_symbols.is_empty(),
        "Expected Rust symbols in src/lib.rs"
    );

    // BM25 search for "UserService"
    let hits = storage
        .fulltext()
        .search_bm25("UserService", 10, None, None)
        .unwrap();
    assert!(
        !hits.is_empty(),
        "Expected fulltext hit for 'UserService'"
    );
}

#[test]
fn integration_empty_project() {
    let tmp = TempDir::new().unwrap();

    let config = IndexConfig {
        repo_id: "empty".to_string(),
        batch_size: 1000,
        embedding_dim: 384,
        ..Default::default()
    };

    let report = index(tmp.path(), &config).unwrap();
    assert_eq!(report.files_indexed, 0);
    assert_eq!(report.total_symbols, 0);
    assert_eq!(report.total_relations, 0);
}
