use std::fs;

use oc_bench::fixture::create_five_language_project;
use oc_indexer::{index, update_file, IndexConfig};
use oc_retrieval::{RetrievalEngine, SearchQuery};
use oc_storage::manager::StorageManager;
use tempfile::TempDir;

#[test]
fn e2e_incremental_update_consistency() {
    let tmp = TempDir::new().unwrap();
    create_five_language_project(tmp.path());

    let config = IndexConfig {
        repo_id: "e2e-inc".to_string(),
        batch_size: 1000,
    };

    // Full index
    let report = index(tmp.path(), &config).unwrap();
    assert!(report.files_indexed >= 8);

    let mut storage = StorageManager::open(tmp.path()).unwrap();

    // Record initial state for an unmodified file
    let go_syms_before = storage
        .graph()
        .get_symbols_by_file("src/go/handler.go")
        .unwrap();
    let go_ids_before: Vec<_> = go_syms_before.iter().map(|s| s.id).collect();

    // Modify Python service file: remove process_batch, add audit_user
    fs::write(
        tmp.path().join("src/python/service.py"),
        r#"
from models import User, UserRepository

class UserService:
    """Business logic for user operations."""

    def __init__(self):
        self.repo = UserRepository()

    def create_user(self, name: str, email: str) -> User:
        user = User(name, email)
        self.repo.save(user)
        return user

    def audit_user(self, email: str) -> dict:
        user = self.get_user(email)
        return {"email": email, "name": user.name}

BATCH_SIZE = 100
"#,
    )
    .unwrap();

    // Incremental update
    let inc_report = update_file(
        tmp.path(),
        "src/python/service.py",
        "e2e-inc",
        &mut storage,
    )
    .unwrap();
    assert!(!inc_report.skipped_unchanged_hash);

    // Verify: audit_user should exist, process_batch should be gone
    let py_syms = storage
        .graph()
        .get_symbols_by_file("src/python/service.py")
        .unwrap();
    let names: Vec<&str> = py_syms.iter().map(|s| s.name.as_str()).collect();
    assert!(
        names.contains(&"audit_user"),
        "audit_user should be added: {:?}",
        names
    );
    assert!(
        !names.contains(&"process_batch"),
        "process_batch should be removed: {:?}",
        names
    );

    // Verify: Go file should be completely untouched
    let go_syms_after = storage
        .graph()
        .get_symbols_by_file("src/go/handler.go")
        .unwrap();
    let go_ids_after: Vec<_> = go_syms_after.iter().map(|s| s.id).collect();
    assert_eq!(
        go_ids_before, go_ids_after,
        "Go symbols should be identical after Python-only update"
    );

    // Verify: fulltext search reflects the change
    storage.fulltext_mut().commit().unwrap();
    let engine = RetrievalEngine::new(&storage);

    let mut query = SearchQuery::new("audit_user");
    query.enable_graph_expansion = false;
    let results = engine.search(&query).unwrap();
    assert!(
        !results.is_empty(),
        "Fulltext should find newly added audit_user"
    );

    let mut query = SearchQuery::new("process_batch");
    query.enable_graph_expansion = false;
    let results = engine.search(&query).unwrap();
    // process_batch should not appear (or if it does, not from this file)
    let from_service: Vec<_> = results
        .iter()
        .filter(|r| r.file_path == "src/python/service.py" && r.name == "process_batch")
        .collect();
    assert!(
        from_service.is_empty(),
        "process_batch should be removed from fulltext"
    );
}
