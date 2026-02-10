use criterion::{criterion_group, criterion_main, Criterion};
use oc_core::{
    CodeRelation, CodeSymbol, Language, RelationKind, SymbolId, SymbolKind,
};
use oc_storage::graph::{GraphStore, TraversalDirection};
use std::path::PathBuf;

fn make_symbol(name: &str, file: &str, byte_start: usize, byte_end: usize) -> CodeSymbol {
    CodeSymbol {
        id: SymbolId::generate("bench-repo", file, name, byte_start, byte_end),
        name: name.to_string(),
        qualified_name: name.to_string(),
        kind: SymbolKind::Function,
        language: Language::Python,
        file_path: PathBuf::from(file),
        byte_range: byte_start..byte_end,
        line_range: 0..10,
        signature: Some(format!("def {name}()")),
        doc_comment: None,
        body_hash: 42,
    }
}

/// Benchmark: SQLite k-hop query (target <50ms for 10K symbols, k=3)
fn bench_graph_khop(c: &mut Criterion) {
    // Build a graph of 10K symbols with interconnected relations
    let mut store = GraphStore::open_in_memory().unwrap();
    let num_symbols = 10_000;

    let symbols: Vec<CodeSymbol> = (0..num_symbols)
        .map(|i| {
            make_symbol(
                &format!("func_{i}"),
                &format!("src/mod_{}.py", i / 100),
                i * 100,
                i * 100 + 50,
            )
        })
        .collect();

    store.insert_symbols(&symbols, 1000).unwrap();

    // Create a chain + fan-out pattern: each symbol calls the next 3 symbols
    let mut relations = Vec::new();
    for i in 0..num_symbols {
        for offset in 1..=3 {
            let target = (i + offset) % num_symbols;
            relations.push(CodeRelation {
                source_id: symbols[i].id,
                target_id: symbols[target].id,
                kind: RelationKind::Calls,
                file_path: PathBuf::from(format!("src/mod_{}.py", i / 100)),
                line: (i % 100) as u32,
                confidence: RelationKind::Calls.default_confidence(),
            });
        }
    }
    store.insert_relations(&relations, 1000).unwrap();

    let seed = symbols[0].id;

    let mut group = c.benchmark_group("graph_khop");

    group.bench_function("khop_k2_10k_symbols", |b| {
        b.iter(|| {
            let _ = store.traverse_khop(seed, 2, 50, TraversalDirection::Both);
        });
    });

    group.bench_function("khop_k3_10k_symbols", |b| {
        b.iter(|| {
            let _ = store.traverse_khop(seed, 3, 50, TraversalDirection::Both);
        });
    });

    group.finish();
}

criterion_group!(benches, bench_graph_khop);
criterion_main!(benches);
