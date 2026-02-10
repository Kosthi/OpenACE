use criterion::{criterion_group, criterion_main, Criterion};
use oc_bench::fixture::create_scaled_project;
use oc_indexer::{index, IndexConfig};
use tempfile::TempDir;

/// Benchmark: full index of 10K files (target <30s)
///
/// We use 2000 files per language × 5 languages = 10K files.
fn bench_index_full(c: &mut Criterion) {
    let tmp = TempDir::new().unwrap();
    create_scaled_project(tmp.path(), 2000);

    let config = IndexConfig {
        repo_id: "bench-repo".to_string(),
        batch_size: 1000,
    };

    let mut group = c.benchmark_group("index_full");
    // This is a heavy benchmark; limit iterations
    group.sample_size(10);
    group.warm_up_time(std::time::Duration::from_secs(1));
    group.measurement_time(std::time::Duration::from_secs(120));
    group.bench_function("full_index_10k_files", |b| {
        b.iter_with_setup(
            || {
                // Clean up .openace before each iteration
                let openace = tmp.path().join(".openace");
                if openace.exists() {
                    std::fs::remove_dir_all(&openace).unwrap();
                }
            },
            |_| {
                let report = index(tmp.path(), &config).unwrap();
                assert!(report.files_indexed > 0);
                report
            },
        );
    });
    group.finish();
}

criterion_group!(benches, bench_index_full);
criterion_main!(benches);
