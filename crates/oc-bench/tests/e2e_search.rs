use oc_bench::fixture::create_five_language_project;
use oc_indexer::{index, IndexConfig};
use oc_retrieval::{RetrievalEngine, SearchQuery};
use oc_storage::manager::StorageManager;
use tempfile::TempDir;

#[test]
fn e2e_full_index_search_all_signals() {
    let tmp = TempDir::new().unwrap();
    create_five_language_project(tmp.path());

    let config = IndexConfig {
        repo_id: "e2e-test".to_string(),
        batch_size: 1000,
    };

    let report = index(tmp.path(), &config).unwrap();

    // All 8 source files across 5 languages should be indexed
    assert!(
        report.files_indexed >= 8,
        "Expected >=8 indexed files, got {}. Failed: {:?}",
        report.files_indexed,
        report.failed_details
    );
    assert_eq!(
        report.files_failed, 0,
        "No files should fail: {:?}",
        report.failed_details
    );
    assert!(report.total_symbols > 0);
    assert!(report.total_relations > 0);

    // Open storage for retrieval
    let storage = StorageManager::open(tmp.path()).unwrap();
    let engine = RetrievalEngine::new(&storage);

    // --- BM25 signal: search for Python class name ---
    let mut query = SearchQuery::new("UserService");
    query.enable_graph_expansion = false;
    let results = engine.search(&query).unwrap();
    assert!(
        !results.is_empty(),
        "BM25 should find UserService"
    );
    let top = &results[0];
    assert_eq!(top.name, "UserService");
    assert!(top.match_signals.contains(&"bm25".to_string()));

    // --- Exact match signal: search for Go struct ---
    let mut query = SearchQuery::new("Router");
    query.enable_graph_expansion = false;
    let results = engine.search(&query).unwrap();
    assert!(
        !results.is_empty(),
        "Exact match should find Router"
    );
    let found = results.iter().any(|r| r.name == "Router");
    assert!(found, "Router should be in results: {:?}", results.iter().map(|r| &r.name).collect::<Vec<_>>());

    // --- Graph expansion signal: search for User class, expect related Admin via Inherits ---
    let mut query = SearchQuery::new("User");
    query.enable_graph_expansion = true;
    query.graph_depth = 2;
    let results = engine.search(&query).unwrap();
    assert!(
        !results.is_empty(),
        "Should find User class with graph expansion"
    );
    // User should be a top result
    let user_result = results.iter().find(|r| r.name == "User");
    assert!(
        user_result.is_some(),
        "User class should be in results: {:?}",
        results.iter().map(|r| &r.name).collect::<Vec<_>>()
    );

    // --- Cross-language search: search for a term that appears across languages ---
    let mut query = SearchQuery::new("process");
    query.enable_graph_expansion = false;
    query.limit = 50;
    let results = engine.search(&query).unwrap();
    assert!(
        !results.is_empty(),
        "Should find 'process' in at least one language"
    );

    // --- Language-filtered search ---
    let mut query = SearchQuery::new("SearchEngine");
    query.enable_graph_expansion = false;
    query.language_filter = Some(oc_core::Language::Rust);
    let results = engine.search(&query).unwrap();
    for r in &results {
        assert!(
            r.file_path.ends_with(".rs"),
            "Language filter to Rust but got file: {}",
            r.file_path
        );
    }

    // --- Verify symbols exist for each language ---
    let graph = storage.graph();
    let py_syms = graph.get_symbols_by_file("src/python/models.py").unwrap();
    assert!(!py_syms.is_empty(), "Python symbols should exist");

    let ts_syms = graph.get_symbols_by_file("src/typescript/server.ts").unwrap();
    assert!(!ts_syms.is_empty(), "TypeScript symbols should exist");

    let rs_syms = graph.get_symbols_by_file("src/rust/engine.rs").unwrap();
    assert!(!rs_syms.is_empty(), "Rust symbols should exist");

    let go_syms = graph.get_symbols_by_file("src/go/handler.go").unwrap();
    assert!(!go_syms.is_empty(), "Go symbols should exist");

    let java_syms = graph.get_symbols_by_file("src/java/Application.java").unwrap();
    assert!(!java_syms.is_empty(), "Java symbols should exist");
}
