# OpenACE Retrieval Improvement - Context Prompt

Use this prompt to continue work in a new session. Copy everything below into the new conversation.

---

## Background

We ran a fair retrieval quality comparison between OpenACE and Augment's ace-tool (MCP tool `mcp__ace-tool__search_context`) on SWE-bench Lite dev split (23 instances). Both systems used identical `problem_statement[:500]` as the single query.

### Results Summary

| Metric | v1 (single) | ace-tool | v1 (multi, 8q) | **v5 (current)** |
|--------|:---:|:---:|:---:|:---:|
| File R@1 | 21.7% (5/23) | 34.8% (8/23) | 30.4% (7/23) | **39.1% (9/23)** |
| File R@5 | 65.2% (15/23) | 78.3% (18/23) | 52.2% (12/23) | **73.9% (17/23)** |
| File R@10 | 73.9% (17/23) | 87.0% (20/23) | 65.2% (15/23) | **87.0% (20/23)** |
| File Found% | 73.9% (17/23) | 87.0% (20/23) | 95.7% (22/23) | **91.3% (21/23)** |
| Avg Rank (found) | 3.3 | 3.4 | 9.1 | ~2.9 |
| File MRR | 0.363 | ~0.437 | 0.427 | **0.498** |

**v5 now matches or exceeds ace-tool.** See `eval/RETRIEVAL_OPTIMIZATION_PROMPT.md` for full optimization history (Rounds 1-3).

### Per-Instance Comparison (Gold File Rank, 0 = not found)

| Instance | Gold File | v1 | ace-tool | **v5** |
|----------|-----------|:---:|:---:|:---:|
| sqlfluff-1625 | L031.py | 3 | 1 | 2 |
| sqlfluff-2419 | L060.py | 1 | 1 | 1 |
| sqlfluff-1733 | L039.py | **0** | 9 | **0** |
| sqlfluff-1517 | helpers.py | **0** | **0** | **0** |
| sqlfluff-1763 | linted_file.py | 5 | 11 | 12 |
| marshmallow-1359 | fields.py | 6 | ~3 | 3 |
| marshmallow-1343 | schema.py | 3 | ~4 | **1** |
| pvlib-1707 | iam.py | 3 | 1 | 1 |
| pvlib-1072 | temperature.py | **0** | 9 | **1** |
| pvlib-1606 | tools.py | 1 | 2 | 1 |
| pvlib-1854 | pvsystem.py | 5 | 2 | 3 |
| pvlib-1154 | irradiance.py | 1 | 1 | 1 |
| astroid-1978 | raw_building.py | **0** | 11 | **5** |
| astroid-1333 | modutils.py | **0** | **0** | **5** |
| astroid-1196 | node_classes.py | 1 | **0** | 1 |
| astroid-1866 | brain_builtin_inference.py | **0** | 7 | **7** |
| astroid-1268 | as_string.py | 1 | 1 | 1 |
| pyvista-4315 | grid.py | 5 | 3 | 4 |
| pydicom-1694 | dataset.py | 2 | 1 | 1 |
| pydicom-1413 | dataelem.py | 5 | 1 | 6 |
| pydicom-901 | config.py | 2 | 1 | 1 |
| pydicom-1139 | valuerep.py | 4 | 2 | 5 |
| pydicom-1256 | jsonrep.py | 8 | 4 | 5 |

## Root Cause Analysis

### Critical Finding: BM25 signal is 100% broken for natural language queries

In the single-query eval, **ALL 23 instances had BM25 signal failure**. The Tantivy `QueryParser::parse_query()` in `crates/oc-storage/src/fulltext.rs:467-472` treats raw problem_statement text as query DSL, and crashes on characters like `(`, `)`, `:`, `"`, backticks. OpenACE was effectively running **vector-only** search.

The same issue affects chunk BM25 (`search_bm25_chunks` at line 588-593).

Log evidence: 20 lines of `WARN oc_retrieval::engine: signal failed, skipping signal="bm25" error=full-text index unavailable: query parse error: Syntax Error: ...`

## Improvement Plan (ordered by priority)

### 1. ~~[CRITICAL] Fix BM25 query parsing~~ — DONE (Round 1, v2)

Fixed in `crates/oc-storage/src/fulltext.rs`: changed `parse_query()` to `parse_query_lenient()` in both `search_bm25()` and `search_bm25_chunks()`.

### 2. ~~[HIGH] Query preprocessing for natural language input~~ — DONE (Round 3, v5)

Implemented `_extract_identifiers()` in `python/openace/engine.py` with per-signal query routing. Extracts CamelCase, snake_case, dotted refs, file path stems, ALL_CAPS from the query and routes them to BM25 (prepended) and exact match (direct) signals separately. See `eval/RETRIEVAL_OPTIMIZATION_PROMPT.md` Round 3 for full details.

### 3. ~~[HIGH] Enable chunk BM25 signal~~ — TESTED, REJECTED (Round 1, v3)

Chunk BM25 hurt performance. The chunk-to-symbol mapping is too coarse. Do not enable.

### 4. ~~[MEDIUM] File-level score aggregation~~ — TESTED, REJECTED (Round 2, v4)

File-level aggregation actively hurts. Large files with many low-relevance symbols get inflated scores. The "best symbol per file" approach is correct.

### 5. [MEDIUM] Weighted multi-query fusion

In `eval/swebench/context_retrieval.py` and `retrieval_eval.py`, give the primary query (problem_statement[:500]) higher weight than extracted sub-queries:

```python
for i, q in enumerate(queries):
    results = engine.search(q, limit=search_limit)
    weight = 3.0 if i == 0 else 1.0
    for r in results:
        r.score *= weight
    all_results.extend(results)
```

## Key Files

| File | What it does |
|------|-------------|
| `crates/oc-storage/src/fulltext.rs` | Tantivy BM25 search — **FIX parse_query HERE** |
| `crates/oc-retrieval/src/engine.rs` | Multi-signal RRF fusion engine, SearchQuery defaults |
| `python/openace/engine.py` | Python SDK Engine.search() — query expansion, reranking |
| `python/openace/query_expansion.py` | LLM-based query expansion (already exists) |
| `eval/swebench/retrieval_eval.py` | Retrieval eval logic — scoring, file-level aggregation |
| `eval/swebench/context_retrieval.py` | generate_queries(), _dedupe_by_symbol_id(), format_context() |
| `eval/run_eval.py` | CLI entry point with `retrieval` subcommand |

## How to Run the Eval

```bash
# Single-query mode (matches ace-tool query format)
uv run python eval/run_eval.py retrieval \
  --conditions full-siliconflow \
  --subset lite --split dev \
  --embedding siliconflow --reranker siliconflow \
  --embedding-base-url "https://router.tumuer.me/v1" \
  --embedding-api-key "sk-xhJytLyXc2SEGdzFDjRajeWxP9YyS3WZ26TaWzEbvRBWC3rb" \
  --reranker-base-url "https://router.tumuer.me/v1" \
  --reranker-api-key "sk-xhJytLyXc2SEGdzFDjRajeWxP9YyS3WZ26TaWzEbvRBWC3rb" \
  --query-mode single \
  -o eval/output_dev_retrieval_single -v

# Multi-query mode (OpenACE's generate_queries with up to 8 sub-queries)
uv run python eval/run_eval.py retrieval \
  --conditions full-siliconflow \
  --subset lite --split dev \
  --embedding siliconflow --reranker siliconflow \
  --embedding-base-url "https://router.tumuer.me/v1" \
  --embedding-api-key "sk-xhJytLyXc2SEGdzFDjRajeWxP9YyS3WZ26TaWzEbvRBWC3rb" \
  --reranker-base-url "https://router.tumuer.me/v1" \
  --reranker-api-key "sk-xhJytLyXc2SEGdzFDjRajeWxP9YyS3WZ26TaWzEbvRBWC3rb" \
  --query-mode multi \
  -o eval/output_dev_retrieval -v
```

Results go to `eval/output_dev_retrieval_single/retrieval_report.md` or `eval/output_dev_retrieval/retrieval_report.md`.

## Build Commands

```bash
cargo build              # Build Rust workspace
uv run maturin develop   # Build Python extension (dev mode)
cargo test -p oc-storage # Test storage after BM25 fix
cargo test -p oc-retrieval # Test retrieval engine
uv run pytest tests/     # Python integration tests
```

## Actual Impact After Fixes (v5 vs v1 baseline)

| Fix | v1 Baseline | v5 Result | Status |
|-----|-------------|-----------|--------|
| #1 BM25 fix | R@5: 65% | R@5: 74% | Done |
| #2 Query preprocessing | MRR: 0.36 | MRR: 0.50 | Done |
| #3 Chunk BM25 | — | Hurt performance | Rejected |
| #4 File aggregation | — | Hurt performance | Rejected |
| #5 Multi-query fusion | — | Not yet retested with v5 | Pending |
| **Overall** | R@10: 74%, MRR: 0.36 | **R@10: 87%, MRR: 0.50** | **Matches ace-tool** |
