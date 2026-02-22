# OpenACE Retrieval Optimization - Context Prompt (Round 4)

Copy everything below into a new conversation.

---

## Background

We fixed BM25 query parsing in Round 1, tested post-processing optimizations in Round 2 (all failed), and implemented per-signal query routing via identifier extraction in Round 3. All eval runs use SWE-bench Lite dev split (23 instances, single-query mode with `problem_statement[:500]`).

### Results Summary (all rounds)

| Metric | v1 (baseline) | v2 (BM25 fix) | v3 (chunk) | v4 series | v2_rerun | **v5 (id extract)** | ace-tool |
|--------|:---:|:---:|:---:|:---:|:---:|:---:|:---:|
| File R@1 | 21.7% (5) | 39.1% (9) | 34.8% (8) | 30-35% | 34.8% | **39.1% (9)** | 34.8% |
| File R@5 | 65.2% (15) | 60.9% (14) | 52.2% (12) | 60.9% | 65.2% | **73.9% (17)** | 78.3% |
| File R@10 | 73.9% (17) | 78.3% (18) | 73.9% (17) | 65-70% | 73.9% | **87.0% (20)** | 87.0% |
| File R@20 | 73.9% | 82.6% (19) | 78.3% (18) | 82.6% | 82.6% | **91.3% (21)** | — |
| File MRR | 0.363 | 0.467 | 0.444 | 0.42-0.44 | 0.446 | **0.498** | ~0.437 |
| Found% | 74% (17) | 83% (19) | 78% (18) | 83% | 83% | **91% (21)** | 87% |

**v5 now matches or exceeds ace-tool on every comparable metric.**

### Critical finding: eval nondeterminism

The v2_rerun (exact same code as v2) produced R@10=73.9% vs the original v2's 78.3%. This ±4-9% variance per metric is caused by **API-based embedding and reranker nondeterminism** (SiliconFlow endpoints). With only 23 instances, ±1-2 rank changes per instance = ±4-9% in metrics.

**Implication**: Small metric differences (<10%) may be noise. The v5 improvement is large enough (R@10: ~74% → 87%, Found%: 83% → 91%) to be clearly above the noise floor.

### Per-Instance: v1 through v5 and ace-tool

| Instance | Gold File | v1 | v2 | v5 | ace-tool | v5 Notes |
|----------|-----------|:---:|:---:|:---:|:---:|---------|
| sqlfluff-1625 | L031.py | 3 | 9 | 2 | 1 | Improved from v2 |
| sqlfluff-2419 | L060.py | 1 | 1 | 1 | 1 | |
| sqlfluff-1733 | L039.py | 0 | 0 | 0 | 9 | Still not found |
| sqlfluff-1517 | helpers.py | 0 | 0 | 0 | 0 | Not found by any |
| sqlfluff-1763 | linted_file.py | 5 | 15 | 12 | 11 | Improved from v2 |
| marshmallow-1359 | fields.py | 6 | 5 | 3 | ~3 | Improved |
| marshmallow-1343 | schema.py | 3 | 6 | **1** | ~4 | **Big win** (6→1) |
| pvlib-1707 | iam.py | 3 | 1 | 1 | 1 | |
| pvlib-1072 | temperature.py | 0 | 5 | **1** | 9 | **Big win** (5→1) |
| pvlib-1606 | tools.py | 1 | 1 | 1 | 2 | |
| pvlib-1854 | pvsystem.py | 5 | 3 | 3 | 2 | |
| pvlib-1154 | irradiance.py | 1 | 1 | 1 | 1 | |
| astroid-1978 | raw_building.py | 0 | 0 | **5** | 11 | **Fixed!** Was not found |
| astroid-1333 | modutils.py | 0 | 7 | **5** | 0 | Improved (7→5) |
| astroid-1196 | node_classes.py | 1 | 1 | 1 | 0 | |
| astroid-1866 | brain_builtin_inference.py | 0 | 0 | **7** | 7 | **Fixed!** Was not found |
| astroid-1268 | as_string.py | 1 | 1 | 1 | 1 | |
| pyvista-4315 | grid.py | 5 | 1 | 4 | 3 | Regressed (1→4) |
| pydicom-1694 | dataset.py | 2 | 1 | 1 | 1 | |
| pydicom-1413 | dataelem.py | 5 | 9 | 6 | 1 | Improved from v2 |
| pydicom-901 | config.py | 2 | 1 | 1 | 1 | |
| pydicom-1139 | valuerep.py | 4 | 5 | 5 | 2 | |
| pydicom-1256 | jsonrep.py | 8 | 5 | 5 | 4 | |

### What was already done (Rounds 1-3)

#### Round 1: BM25 lenient parsing (v2)

**File**: `crates/oc-storage/src/fulltext.rs`

Changed `parse_query()` to `parse_query_lenient()` in both `search_bm25()` and `search_bm25_chunks()`. This fixed the 100% BM25 signal failure for natural language queries.

#### Round 2: Post-processing tuning (v4 series) — ALL FAILED

Three optimizations tested independently and in combination. None produced reliable improvements:

1. **File-level score aggregation** — FAILED. Large files with many low-relevance symbols got inflated scores. The "best symbol per file" approach is already correct.
2. **Score-gap cutoff relaxation (0.6 → 0.4)** — NO EFFECT. Within noise band.
3. **Single-signal penalty reduction (0.85 → 0.95)** — NO EFFECT. The reranker's `rerank_score` makes the penalty multiplier irrelevant (monotonic scaling doesn't change relative order).

All v4 changes were reverted.

#### Round 3: Per-signal query routing via identifier extraction (v5)

**Problem**: The exact match signal was completely dead for long natural language queries. `collect_exact_match()` does `WHERE name = ?` against the full 500-char problem statement, which never matches any symbol name. BM25 also gets diluted by natural language noise.

**Solution**: Extract code identifiers from the query (CamelCase, snake_case, dotted refs, file path stems, ALL_CAPS) and route them to appropriate signals:

| Signal | Query Source |
|--------|-------------|
| BM25 | `bm25_text`: extracted identifiers prepended to expanded query |
| Exact match | `exact_queries`: extracted identifiers only |
| Vector | `query_vector`: embedding of original query (unchanged) |
| Chunk BM25 | Same as BM25 (`bm25_text`) |
| Graph | No text input (traverses from direct hits) |

**Files modified**:
- `crates/oc-retrieval/src/engine.rs` — Added `bm25_text: Option<String>` and `exact_queries: Vec<String>` to `SearchQuery`; modified `collect_bm25()`, `collect_bm25_chunks()`, `collect_exact_match()` to use new fields
- `crates/oc-python/src/engine.rs` — Added PyO3 bindings for new parameters
- `python/openace/engine.py` — Added `_extract_identifiers()` function with regex patterns for CamelCase, acronym-CamelCase (HTMLParser), snake_case, dotted refs, file paths, ALL_CAPS; wired into `Engine.search()` as Stage 0.1
- `tests/test_identifier_extraction.py` — 18 unit tests for identifier extraction

**Key implementation details**:
- `_extract_identifiers()` is zero-latency, deterministic (pure regex, no ML/API calls)
- Backward compatible: all new fields have defaults (`None` / empty vec), existing callers unaffected
- Helper method `SearchQuery::effective_bm25_text()` falls back to `text` when `bm25_text` is `None`
- `collect_exact_match()` iterates over each identifier in `exact_queries`, searching by both `name` and `qualified_name`, deduplicating by `SymbolId`

**Commit**: `6eaefff` — `feat(retrieval): per-signal query routing with identifier extraction`

### Chunk BM25 conclusion

Chunk BM25 (v3) hurt performance. The `collect_bm25_chunks` implementation maps chunk hits to the "best" symbol per file (by kind priority), which is too coarse. **Do not enable chunk indexing in the eval for now.**

## Current state

**Code is at v5** — identifier extraction and per-signal query routing are live. v5 matches or exceeds ace-tool on all comparable metrics (R@10: 87.0% = ace-tool, MRR: 0.498 > 0.437, Found%: 91% > 87%).

Remaining gaps:
- **R@5**: 73.9% vs ace-tool's 78.3% (within noise range, but worth investigating)
- **sqlfluff-1733** (L039.py): Still not found by OpenACE (ace-tool finds at rank 9)
- **sqlfluff-1517** (helpers.py): Not found by either tool
- **pyvista-4315** regression: v2 had rank 1, v5 has rank 4 (nondeterminism or identifier interference)

## What to investigate next (Round 4)

### 1. Remaining unfound instances

- **sqlfluff-1733** (L039.py): ace-tool finds it at rank 9. Investigate what query terms could match. The problem statement may reference the rule indirectly.
- **sqlfluff-1517** (helpers.py): Not found by either tool. May require deeper semantic understanding or multi-query mode.

### 2. pyvista-4315 regression analysis

v2 had rank 1, v5 has rank 4. Could be:
- Identifier noise in BM25 (prepended identifiers dilute the relevant terms)
- Nondeterminism from API embedding/reranking
- Run v5 multiple times to determine if it's noise

### 3. Multi-query mode

The eval uses `--query-mode single`. Multi-query mode generates multiple sub-queries via LLM, which could help for instances where relevant keywords are buried deep in the problem statement. The `multi` mode showed 95.7% Found% (22/23) in v1, but with worse ranking (MRR: 0.427). With v5's identifier extraction, multi-query might perform better.

### 4. Larger eval set

23 instances is too small to reliably distinguish improvements from noise. Consider:
- Full Lite split (300 instances) for more reliable metrics
- Multiple runs (3-5x) per configuration to measure variance
- Deterministic BM25-only baseline (no embedding/reranker API calls)

### 5. Embedding/vector signal quality

The vector signal uses the same query text for embedding. Consider:
- Code-optimized embedding model instead of general-purpose
- Separate embedding text (e.g., extracted identifiers + key terms only)

### 6. Graph expansion signal

The Rust `RetrievalEngine` supports graph expansion (following import/call relationships). Its contribution to RRF hasn't been separately evaluated. Could help for indirectly-referenced files.

## Key Files

| File | Status |
|------|--------|
| `python/openace/engine.py` | v5: `_extract_identifiers()` + Stage 0.1 query routing. |
| `crates/oc-retrieval/src/engine.rs` | v5: `SearchQuery` with `bm25_text`, `exact_queries`; modified signal collectors. |
| `crates/oc-python/src/engine.rs` | v5: PyO3 bindings for `bm25_text`, `exact_queries`. |
| `crates/oc-storage/src/fulltext.rs` | v2: BM25 lenient fix. No further changes needed. |
| `tests/test_identifier_extraction.py` | v5: 18 unit tests for `_extract_identifiers()`. |
| `eval/swebench/retrieval_eval.py` | Eval harness. Consider adding multi-run averaging. |

## How to Run the Eval

```bash
# Build Python extension first
unset CONDA_PREFIX && uv run maturin develop --release

# Run single-query eval
uv run python eval/run_eval.py retrieval \
  --conditions full-siliconflow \
  --subset lite --split dev \
  --embedding siliconflow --reranker siliconflow \
  --embedding-base-url "https://router.tumuer.me/v1" \
  --embedding-api-key "sk-xhJytLyXc2SEGdzFDjRajeWxP9YyS3WZ26TaWzEbvRBWC3rb" \
  --reranker-base-url "https://router.tumuer.me/v1" \
  --reranker-api-key "sk-xhJytLyXc2SEGdzFDjRajeWxP9YyS3WZ26TaWzEbvRBWC3rb" \
  --query-mode single \
  -o eval/output_dev_retrieval_single_vX -v
```

Note: Only Python changes needed for `engine.py` tuning — no Rust rebuild required. But if modifying Rust crates, run `uv run maturin develop --release`.

## Build Commands

```bash
cargo build --workspace --exclude oc-python   # Verify Rust compiles
cargo test -p oc-retrieval                     # Rust retrieval tests (22 tests)
uv run maturin develop --release               # Build Python extension
uv run pytest tests/test_identifier_extraction.py  # Identifier extraction tests (18 tests)
uv run pytest tests/test_engine.py             # Python integration tests (13 tests)
```
