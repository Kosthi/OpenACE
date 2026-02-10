use criterion::{criterion_group, criterion_main, Criterion};
use oc_core::SymbolId;
use oc_storage::vector::VectorStore;

/// Benchmark: usearch k-NN (target <10ms for 50K vectors, k=10)
fn bench_vector_knn(c: &mut Criterion) {
    let dimension = 384;
    let num_vectors = 50_000;

    let mut store = VectorStore::new(dimension).unwrap();

    // Insert 50K random vectors (deterministic seed via simple formula)
    for i in 0..num_vectors {
        let id = SymbolId(i as u128 + 1);
        let vector: Vec<f32> = (0..dimension)
            .map(|d| {
                // Deterministic pseudo-random: sin-based dispersion
                let val = ((i * 7 + d * 13) as f32).sin();
                val
            })
            .collect();
        store.add_vector(id, &vector).unwrap();
    }

    // Query vector
    let query: Vec<f32> = (0..dimension)
        .map(|d| ((42 * 7 + d * 13) as f32).sin())
        .collect();

    let mut group = c.benchmark_group("vector_knn");

    group.bench_function("knn_k10_50k_384d", |b| {
        b.iter(|| {
            let _ = store.search_knn(&query, 10);
        });
    });

    group.bench_function("knn_k50_50k_384d", |b| {
        b.iter(|| {
            let _ = store.search_knn(&query, 50);
        });
    });

    group.finish();
}

criterion_group!(benches, bench_vector_knn);
criterion_main!(benches);
