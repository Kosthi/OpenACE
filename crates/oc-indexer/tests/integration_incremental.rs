use std::fs;
use std::path::Path;

use oc_indexer::{
    incremental_delete, index, process_events, update_file, ChangeEvent, IndexConfig,
};
use oc_storage::manager::StorageManager;
use tempfile::TempDir;

fn create_fixture(root: &Path) {
    let src = root.join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(
        src.join("main.py"),
        r#"
class UserService:
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

    fs::write(
        src.join("utils.py"),
        r#"
def format_name(name: str) -> str:
    return name.strip().title()

def validate_input(data: dict) -> bool:
    return "name" in data
"#,
    )
    .unwrap();
}

#[test]
fn incremental_update_modified_file() {
    let tmp = TempDir::new().unwrap();
    create_fixture(tmp.path());

    let config = IndexConfig {
        repo_id: "test-repo".to_string(),
        batch_size: 1000,
        embedding_dim: 384,
    };

    // Full index first
    let report = index(tmp.path(), &config).unwrap();
    assert_eq!(report.files_indexed, 2);

    // Open storage and record initial symbol state
    let mut storage = StorageManager::open(tmp.path()).unwrap();
    let initial_symbols = storage.graph().get_symbols_by_file("src/main.py").unwrap();
    let initial_count = initial_symbols.len();
    assert!(initial_count > 0);

    // utils.py should be untouched
    let utils_before = storage.graph().get_symbols_by_file("src/utils.py").unwrap();

    // Modify main.py: add a new function, remove process_request
    fs::write(
        tmp.path().join("src/main.py"),
        r#"
class UserService:
    def create_user(self, name: str) -> dict:
        return {"name": name}

    def delete_user(self, user_id: int) -> bool:
        return True

def new_handler(req):
    return UserService().create_user(req)
"#,
    )
    .unwrap();

    // Incremental update
    let inc_report = update_file(tmp.path(), "src/main.py", "test-repo", &mut storage).unwrap();
    assert!(!inc_report.skipped_unchanged_hash);

    // Verify symbols changed for main.py
    let updated_symbols = storage.graph().get_symbols_by_file("src/main.py").unwrap();
    let updated_names: Vec<&str> = updated_symbols.iter().map(|s| s.name.as_str()).collect();
    assert!(
        updated_names.contains(&"new_handler"),
        "new_handler should be added: {:?}",
        updated_names
    );
    assert!(
        !updated_names.contains(&"process_request"),
        "process_request should be removed: {:?}",
        updated_names
    );

    // utils.py should be completely unaffected
    let utils_after = storage.graph().get_symbols_by_file("src/utils.py").unwrap();
    assert_eq!(
        utils_before.len(),
        utils_after.len(),
        "utils.py should be untouched"
    );
    let before_ids: Vec<_> = utils_before.iter().map(|s| s.id).collect();
    let after_ids: Vec<_> = utils_after.iter().map(|s| s.id).collect();
    assert_eq!(before_ids, after_ids, "utils.py symbol IDs should be identical");
}

#[test]
fn incremental_update_unchanged_file_skipped() {
    let tmp = TempDir::new().unwrap();
    create_fixture(tmp.path());

    let config = IndexConfig {
        repo_id: "test-repo".to_string(),
        batch_size: 1000,
        embedding_dim: 384,
    };
    index(tmp.path(), &config).unwrap();

    let mut storage = StorageManager::open(tmp.path()).unwrap();

    // Update the same file without changing content
    let report = update_file(tmp.path(), "src/main.py", "test-repo", &mut storage).unwrap();
    assert!(
        report.skipped_unchanged_hash,
        "should skip when content hash matches"
    );
    assert_eq!(report.added, 0);
    assert_eq!(report.removed, 0);
    assert_eq!(report.modified, 0);
}

#[test]
fn incremental_delete_file_cleanup() {
    let tmp = TempDir::new().unwrap();
    create_fixture(tmp.path());

    let config = IndexConfig {
        repo_id: "test-repo".to_string(),
        batch_size: 1000,
        embedding_dim: 384,
    };
    index(tmp.path(), &config).unwrap();

    let mut storage = StorageManager::open(tmp.path()).unwrap();

    // Verify symbols exist
    let before = storage.graph().get_symbols_by_file("src/utils.py").unwrap();
    assert!(!before.is_empty());

    // Verify file metadata exists
    assert!(storage.graph().get_file("src/utils.py").unwrap().is_some());

    // Delete the file
    let report = incremental_delete("src/utils.py", &mut storage).unwrap();
    assert!(report.removed > 0);

    // All symbols should be gone
    let after = storage.graph().get_symbols_by_file("src/utils.py").unwrap();
    assert!(after.is_empty(), "all symbols should be removed");

    // File metadata should be gone
    assert!(
        storage.graph().get_file("src/utils.py").unwrap().is_none(),
        "file metadata should be removed"
    );

    // Verify fulltext also cleaned up (search should not find utils.py symbols)
    storage.fulltext_mut().commit().unwrap();
    let hits = storage
        .fulltext()
        .search_bm25("format_name", 10, None, None)
        .unwrap();
    assert!(
        hits.is_empty(),
        "fulltext should not return deleted symbols"
    );
}

#[test]
fn incremental_process_events_batch() {
    let tmp = TempDir::new().unwrap();
    create_fixture(tmp.path());

    let config = IndexConfig {
        repo_id: "test-repo".to_string(),
        batch_size: 1000,
        embedding_dim: 384,
    };
    index(tmp.path(), &config).unwrap();

    // Modify one file, remove the other on disk
    fs::write(
        tmp.path().join("src/main.py"),
        r#"
def only_one_function():
    pass
"#,
    )
    .unwrap();
    fs::remove_file(tmp.path().join("src/utils.py")).unwrap();

    let events = vec![
        ChangeEvent::Changed("src/main.py".into()),
        ChangeEvent::Removed("src/utils.py".into()),
    ];

    let mut storage = StorageManager::open(tmp.path()).unwrap();
    let reports = process_events(tmp.path(), &events, "test-repo", &mut storage);

    // Both should succeed
    for r in &reports {
        assert!(r.is_ok(), "event processing failed: {:?}", r);
    }

    // main.py should have new symbols
    let main_syms = storage.graph().get_symbols_by_file("src/main.py").unwrap();
    let names: Vec<&str> = main_syms.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"only_one_function"));

    // utils.py should be fully cleaned up
    let utils_syms = storage.graph().get_symbols_by_file("src/utils.py").unwrap();
    assert!(utils_syms.is_empty());
}

#[test]
fn convergence_incremental_vs_full_reindex() {
    let tmp = TempDir::new().unwrap();
    create_fixture(tmp.path());

    let config = IndexConfig {
        repo_id: "test-repo".to_string(),
        batch_size: 1000,
        embedding_dim: 384,
    };

    // Full index
    index(tmp.path(), &config).unwrap();

    // Apply a series of incremental changes
    let mut storage = StorageManager::open(tmp.path()).unwrap();

    // Change 1: modify main.py
    fs::write(
        tmp.path().join("src/main.py"),
        r#"
class UserService:
    def create_user(self, name: str) -> dict:
        return {"name": name}

def helper():
    return 42
"#,
    )
    .unwrap();
    update_file(tmp.path(), "src/main.py", "test-repo", &mut storage).unwrap();

    // Change 2: modify again
    fs::write(
        tmp.path().join("src/main.py"),
        r#"
def final_function(x):
    return x * 2

def another_one():
    pass
"#,
    )
    .unwrap();
    update_file(tmp.path(), "src/main.py", "test-repo", &mut storage).unwrap();

    // Change 3: modify utils.py
    fs::write(
        tmp.path().join("src/utils.py"),
        r#"
def new_util(val):
    return str(val)
"#,
    )
    .unwrap();
    update_file(tmp.path(), "src/utils.py", "test-repo", &mut storage).unwrap();

    storage.flush().unwrap();

    // Collect incremental state
    let inc_main = storage.graph().get_symbols_by_file("src/main.py").unwrap();
    let inc_utils = storage.graph().get_symbols_by_file("src/utils.py").unwrap();
    let inc_main_meta = storage.graph().get_file("src/main.py").unwrap().unwrap();
    let inc_utils_meta = storage.graph().get_file("src/utils.py").unwrap().unwrap();

    drop(storage);

    // Remove .openace and do a fresh full re-index on the same file state
    fs::remove_dir_all(tmp.path().join(".openace")).unwrap();
    let fresh_report = index(tmp.path(), &config).unwrap();
    assert_eq!(fresh_report.files_indexed, 2);

    let fresh_storage = StorageManager::open(tmp.path()).unwrap();
    let fresh_main = fresh_storage
        .graph()
        .get_symbols_by_file("src/main.py")
        .unwrap();
    let fresh_utils = fresh_storage
        .graph()
        .get_symbols_by_file("src/utils.py")
        .unwrap();
    let fresh_main_meta = fresh_storage.graph().get_file("src/main.py").unwrap().unwrap();
    let fresh_utils_meta = fresh_storage.graph().get_file("src/utils.py").unwrap().unwrap();

    // Symbol sets should match
    let mut inc_main_ids: Vec<_> = inc_main.iter().map(|s| s.id.0).collect();
    let mut fresh_main_ids: Vec<_> = fresh_main.iter().map(|s| s.id.0).collect();
    inc_main_ids.sort();
    fresh_main_ids.sort();
    assert_eq!(
        inc_main_ids, fresh_main_ids,
        "main.py symbol IDs should match between incremental and full"
    );

    let mut inc_utils_ids: Vec<_> = inc_utils.iter().map(|s| s.id.0).collect();
    let mut fresh_utils_ids: Vec<_> = fresh_utils.iter().map(|s| s.id.0).collect();
    inc_utils_ids.sort();
    fresh_utils_ids.sort();
    assert_eq!(
        inc_utils_ids, fresh_utils_ids,
        "utils.py symbol IDs should match between incremental and full"
    );

    // Content hashes should match
    assert_eq!(
        inc_main_meta.content_hash, fresh_main_meta.content_hash,
        "main.py content hash should match"
    );
    assert_eq!(
        inc_utils_meta.content_hash, fresh_utils_meta.content_hash,
        "utils.py content hash should match"
    );

    // Symbol counts should match
    assert_eq!(
        inc_main_meta.symbol_count, fresh_main_meta.symbol_count,
        "main.py symbol count should match"
    );
    assert_eq!(
        inc_utils_meta.symbol_count, fresh_utils_meta.symbol_count,
        "utils.py symbol count should match"
    );
}
