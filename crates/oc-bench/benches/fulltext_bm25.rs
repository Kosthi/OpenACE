use criterion::{criterion_group, criterion_main, Criterion};
use oc_core::{CodeSymbol, Language, SymbolId, SymbolKind};
use oc_storage::fulltext::FullTextStore;
use std::path::PathBuf;
use tempfile::TempDir;

fn make_symbol(name: &str, file: &str, byte_start: usize, byte_end: usize) -> CodeSymbol {
    CodeSymbol {
        id: SymbolId::generate("bench-repo", file, name, byte_start, byte_end),
        name: name.to_string(),
        qualified_name: format!("mod.{name}"),
        kind: SymbolKind::Function,
        language: Language::Python,
        file_path: PathBuf::from(file),
        byte_range: byte_start..byte_end,
        line_range: 0..10,
        signature: Some(format!("def {name}()")),
        doc_comment: None,
        body_hash: 42,
        body_text: None,
    }
}

/// Benchmark: Tantivy BM25 search (target <50ms for 50K docs)
fn bench_fulltext_bm25(c: &mut Criterion) {
    let tmp = TempDir::new().unwrap();
    let tantivy_path = tmp.path().join("tantivy");

    let mut store = FullTextStore::open(&tantivy_path).unwrap();

    let num_docs = 50_000;

    // Insert 50K documents
    for i in 0..num_docs {
        let name = format!("function_{i}");
        let file = format!("src/mod_{}.py", i / 100);
        let sym = make_symbol(&name, &file, i * 100, i * 100 + 50);
        let body = format!("def function_{i}(data): return process_and_validate(data, {i})");
        store.add_document(&sym, Some(&body)).unwrap();
    }
    store.commit().unwrap();

    let mut group = c.benchmark_group("fulltext_bm25");

    group.bench_function("bm25_50k_docs_simple", |b| {
        b.iter(|| {
            let _ = store.search_bm25("function_12345", 10, None, None);
        });
    });

    group.bench_function("bm25_50k_docs_multi_term", |b| {
        b.iter(|| {
            let _ = store.search_bm25("process validate data", 10, None, None);
        });
    });

    group.bench_function("bm25_50k_docs_with_filter", |b| {
        b.iter(|| {
            let _ = store.search_bm25("function", 10, Some("src/mod_50"), None);
        });
    });

    group.finish();
}

criterion_group!(benches, bench_fulltext_bm25);
criterion_main!(benches);
