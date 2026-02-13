use criterion::{criterion_group, criterion_main, Criterion};
use oc_bench::fixture::create_scaled_project;
use oc_indexer::{index, update_file, IndexConfig};
use oc_storage::manager::StorageManager;
use std::fs;
use tempfile::TempDir;

/// Benchmark: incremental single-file update (target <500ms)
fn bench_index_incremental(c: &mut Criterion) {
    let tmp = TempDir::new().unwrap();
    // Create a medium project (200 files per language = 1K files)
    create_scaled_project(tmp.path(), 200);

    let config = IndexConfig {
        repo_id: "bench-repo".to_string(),
        batch_size: 1000,
        embedding_dim: 384,
        ..Default::default()
    };

    // Full index first
    index(tmp.path(), &config).unwrap();

    let mut group = c.benchmark_group("index_incremental");
    group.bench_function("incremental_single_file_update", |b| {
        let mut counter = 0u64;
        b.iter(|| {
            counter += 1;
            // Modify a single file with slightly different content each iteration
            let new_content = format!(
                r#"
class Service0:
    """Modified service iteration {counter}."""

    def __init__(self, name: str):
        self.name = name

    def process(self, data: dict) -> dict:
        return {{"name": self.name, "iteration": {counter}}}

    def validate(self, input_val: str) -> bool:
        return len(input_val) > {counter}

def new_function_{counter}() -> int:
    return {counter}
"#
            );
            fs::write(
                tmp.path().join("src/python/mod_0.py"),
                &new_content,
            )
            .unwrap();

            let mut storage = StorageManager::open(tmp.path()).unwrap();
            let _ = update_file(
                tmp.path(),
                "src/python/mod_0.py",
                "bench-repo",
                &mut storage,
                None,
            );
        });
    });
    group.finish();
}

criterion_group!(benches, bench_index_incremental);
criterion_main!(benches);
