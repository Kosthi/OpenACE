"""Retrieval quality evaluation for OpenACE against SWE-bench gold patches.

Measures whether OpenACE surfaces the correct files/functions for a given
bug description, without requiring any LLM or Docker.
"""

from __future__ import annotations

import json
import logging
import re
import time
from dataclasses import asdict, dataclass, field
from pathlib import Path
from typing import Optional

from eval.swebench.config import ExperimentCondition
from eval.swebench.context_retrieval import _dedupe_by_symbol_id, generate_queries
from eval.swebench.dataset import SWEInstance, load_dataset
from eval.swebench.repo_manager import clone_or_reuse

logger = logging.getLogger(__name__)


# ---------------------------------------------------------------------------
# Data classes
# ---------------------------------------------------------------------------


@dataclass
class GoldInfo:
    """Ground-truth files and functions extracted from a unified diff patch."""

    files: list[str]
    functions: list[str]


@dataclass
class InstanceMetrics:
    """Per-instance retrieval evaluation metrics."""

    instance_id: str
    gold_files: list[str]
    gold_functions: list[str]
    retrieved_files: list[str]
    retrieved_functions: list[str]
    file_recall_at_1: float
    file_recall_at_5: float
    file_recall_at_10: float
    file_recall_at_20: float
    function_recall_at_5: float
    function_recall_at_10: float
    file_mrr: float
    function_mrr: float
    first_gold_file_rank: int  # 0 = not found
    num_queries: int
    index_time_secs: float
    search_time_secs: float


# ---------------------------------------------------------------------------
# Patch parsing
# ---------------------------------------------------------------------------

_DIFF_FILE_RE = re.compile(r"^diff --git a/(.+?) b/(.+?)$", re.MULTILINE)
_HUNK_HEADER_RE = re.compile(
    r"^@@ .+? @@\s*(?:.*\s)?(\w[\w.]*)\s*\(", re.MULTILINE,
)


def extract_gold_from_patch(patch: str) -> GoldInfo:
    """Extract gold file paths and function names from a unified diff.

    Files come from ``diff --git a/... b/...`` headers (the ``b/`` side).
    Function names come from ``@@ ... @@ <context>`` hunk headers — the
    standard unified-diff context line often contains the enclosing function.
    """
    files: list[str] = []
    seen_files: set[str] = set()
    for m in _DIFF_FILE_RE.finditer(patch):
        path = m.group(2)
        if path not in seen_files:
            seen_files.add(path)
            files.append(path)

    functions: list[str] = []
    seen_funcs: set[str] = set()
    for m in _HUNK_HEADER_RE.finditer(patch):
        name = m.group(1)
        # Skip common noise tokens
        if name.lower() in {"class", "def", "if", "for", "while", "return"}:
            continue
        if name not in seen_funcs:
            seen_funcs.add(name)
            functions.append(name)

    return GoldInfo(files=files, functions=functions)


# ---------------------------------------------------------------------------
# Metric computation
# ---------------------------------------------------------------------------


def _recall_at_k(gold: list[str], retrieved: list[str], k: int) -> float:
    """Fraction of gold items found in the top-k retrieved items."""
    if not gold:
        return 1.0  # vacuously true
    top_k = set(retrieved[:k])
    hits = sum(1 for g in gold if g in top_k)
    return hits / len(gold)


def _mrr(gold: list[str], retrieved: list[str]) -> float:
    """Mean Reciprocal Rank: 1/rank of the first gold hit, or 0."""
    gold_set = set(gold)
    for i, item in enumerate(retrieved, start=1):
        if item in gold_set:
            return 1.0 / i
    return 0.0


def _first_rank(gold: list[str], retrieved: list[str]) -> int:
    """Rank (1-based) of the first gold hit, or 0 if not found."""
    gold_set = set(gold)
    for i, item in enumerate(retrieved, start=1):
        if item in gold_set:
            return i
    return 0


# ---------------------------------------------------------------------------
# Single-instance evaluation
# ---------------------------------------------------------------------------


def evaluate_retrieval_single(
    instance: SWEInstance,
    project_root: str,
    condition: ExperimentCondition,
    gold: GoldInfo,
    search_limit: int = 20,
    embedding_kwargs: Optional[dict] = None,
    reranker_kwargs: Optional[dict] = None,
    query_mode: str = "multi",
) -> InstanceMetrics:
    """Index a repo, run queries, and score retrieval against gold.

    Args:
        instance: SWE-bench task instance.
        project_root: Absolute path to the checked-out repo.
        condition: Experiment condition controlling backends.
        gold: Ground-truth file/function info from the patch.
        search_limit: Maximum results per query.
        embedding_kwargs: Extra kwargs passed to ``create_provider()``.
        reranker_kwargs: Extra kwargs passed to ``create_reranker()``.
        query_mode: ``"multi"`` uses generate_queries() (multiple sub-queries),
            ``"single"`` uses problem_statement[:500] as a single query.

    Returns:
        InstanceMetrics with recall/MRR numbers.
    """
    from openace.engine import Engine

    embedding_provider = None
    reranker = None

    if condition.embedding_backend is not None:
        from openace.embedding.factory import create_provider
        embedding_provider = create_provider(
            condition.embedding_backend, **(embedding_kwargs or {}),
        )

    if condition.reranker_backend is not None:
        from openace.reranking.factory import create_reranker
        reranker = create_reranker(
            condition.reranker_backend, **(reranker_kwargs or {}),
        )

    engine = Engine(
        project_root,
        embedding_provider=embedding_provider,
        embedding_dim=embedding_provider.dimension if embedding_provider else None,
        reranker=reranker,
    )

    # Index
    t0 = time.monotonic()
    report = engine.index()
    index_time = time.monotonic() - t0
    logger.info(
        "Indexed %d files, %d symbols in %.1fs",
        report.files_indexed, report.total_symbols, index_time,
    )

    # Generate queries based on mode
    if query_mode == "single":
        queries = [instance.problem_statement[:500]]
    else:
        queries = generate_queries(instance.problem_statement)
    t1 = time.monotonic()

    all_results = []
    for q in queries:
        try:
            results = engine.search(q, limit=search_limit)
            all_results.extend(results)
        except Exception:
            logger.warning("Search failed for query: %s", q, exc_info=True)

    unique_results = _dedupe_by_symbol_id(all_results)
    search_time = time.monotonic() - t1

    # Extract retrieved file paths (unique, ordered by first appearance at
    # highest score)
    retrieved_files: list[str] = []
    seen_files: set[str] = set()
    for r in unique_results:
        fp = r.file_path
        if fp not in seen_files:
            seen_files.add(fp)
            retrieved_files.append(fp)

    # Extract retrieved function/method names
    retrieved_functions: list[str] = []
    seen_funcs: set[str] = set()
    for r in unique_results:
        if r.kind in ("function", "method"):
            name = r.name
            if name not in seen_funcs:
                seen_funcs.add(name)
                retrieved_functions.append(name)

    return InstanceMetrics(
        instance_id=instance.instance_id,
        gold_files=gold.files,
        gold_functions=gold.functions,
        retrieved_files=retrieved_files,
        retrieved_functions=retrieved_functions,
        file_recall_at_1=_recall_at_k(gold.files, retrieved_files, 1),
        file_recall_at_5=_recall_at_k(gold.files, retrieved_files, 5),
        file_recall_at_10=_recall_at_k(gold.files, retrieved_files, 10),
        file_recall_at_20=_recall_at_k(gold.files, retrieved_files, 20),
        function_recall_at_5=_recall_at_k(gold.functions, retrieved_functions, 5),
        function_recall_at_10=_recall_at_k(gold.functions, retrieved_functions, 10),
        file_mrr=_mrr(gold.files, retrieved_files),
        function_mrr=_mrr(gold.functions, retrieved_functions),
        first_gold_file_rank=_first_rank(gold.files, retrieved_files),
        num_queries=len(queries),
        index_time_secs=round(index_time, 2),
        search_time_secs=round(search_time, 4),
    )


# ---------------------------------------------------------------------------
# Main evaluation loop
# ---------------------------------------------------------------------------


def run_retrieval_eval(
    conditions: list[str],
    *,
    subset: str = "lite",
    split: str = "dev",
    slice_spec: str = "",
    output_dir: str = "eval/output_dev_retrieval",
    repos_dir: str = "eval/repos",
    embedding: Optional[str] = None,
    reranker: Optional[str] = None,
    search_limit: int = 20,
    embedding_base_url: Optional[str] = None,
    embedding_api_key: Optional[str] = None,
    reranker_base_url: Optional[str] = None,
    reranker_api_key: Optional[str] = None,
    query_mode: str = "multi",
) -> dict:
    """Run retrieval evaluation across conditions and instances.

    Args:
        conditions: List of condition IDs to evaluate.
        subset: SWE-bench subset (lite, verified, full).
        split: Dataset split (dev, test).
        slice_spec: Optional slice like "0:10".
        output_dir: Where to write results.
        repos_dir: Cache dir for cloned repos.
        embedding: Override embedding backend for non-baseline conditions.
        reranker: Override reranker backend for non-baseline conditions.
        search_limit: Max search results per query.
        embedding_base_url: Override embedding API base URL.
        embedding_api_key: Override embedding API key.
        reranker_base_url: Override reranker API base URL.
        reranker_api_key: Override reranker API key.
        query_mode: ``"multi"`` (default) or ``"single"`` — controls query
            generation strategy.

    Returns:
        Aggregated results dict.
    """
    # Build embedding kwargs from explicit overrides
    embedding_kwargs: dict = {}
    if embedding_base_url:
        embedding_kwargs["base_url"] = embedding_base_url
    if embedding_api_key:
        embedding_kwargs["api_key"] = embedding_api_key

    # Build reranker kwargs from explicit overrides
    reranker_kwargs: dict = {}
    if reranker_base_url:
        reranker_kwargs["base_url"] = reranker_base_url
    if reranker_api_key:
        reranker_kwargs["api_key"] = reranker_api_key

    dataset_name = {
        "lite": "princeton-nlp/SWE-bench_Lite",
        "verified": "princeton-nlp/SWE-bench_Verified",
        "full": "princeton-nlp/SWE-bench",
    }.get(subset, subset)

    instances = load_dataset(dataset_name, split=split)
    if slice_spec:
        parts = slice_spec.split(":")
        start = int(parts[0]) if parts[0] else 0
        end = int(parts[1]) if len(parts) > 1 and parts[1] else len(instances)
        instances = instances[start:end]

    logger.info(
        "Loaded %d instances from %s/%s", len(instances), dataset_name, split,
    )

    # Build condition objects
    condition_map = _build_conditions(conditions, embedding, reranker, search_limit)

    all_results: dict[str, dict] = {}

    for cond_id, condition in condition_map.items():
        logger.info("=== Condition: %s ===", cond_id)
        cond_output = Path(output_dir) / cond_id
        cond_output.mkdir(parents=True, exist_ok=True)

        instance_metrics: list[InstanceMetrics] = []

        for i, inst in enumerate(instances):
            logger.info(
                "[%d/%d] %s", i + 1, len(instances), inst.instance_id,
            )

            gold = extract_gold_from_patch(inst.patch)
            if not gold.files:
                logger.warning(
                    "No gold files extracted from patch for %s, skipping",
                    inst.instance_id,
                )
                continue

            try:
                repo_path = clone_or_reuse(inst.repo, inst.base_commit, repos_dir)
                metrics = evaluate_retrieval_single(
                    inst, str(repo_path), condition, gold, search_limit,
                    embedding_kwargs=embedding_kwargs or None,
                    reranker_kwargs=reranker_kwargs or None,
                    query_mode=query_mode,
                )
                instance_metrics.append(metrics)

                logger.info(
                    "  File R@1=%.2f R@5=%.2f R@10=%.2f MRR=%.3f | rank=%d",
                    metrics.file_recall_at_1,
                    metrics.file_recall_at_5,
                    metrics.file_recall_at_10,
                    metrics.file_mrr,
                    metrics.first_gold_file_rank,
                )
            except Exception:
                logger.error(
                    "Failed to evaluate %s", inst.instance_id, exc_info=True,
                )

        # Aggregate and save
        agg = _aggregate_metrics(instance_metrics)
        agg["condition_id"] = cond_id
        agg["num_instances"] = len(instance_metrics)
        agg["instances"] = [asdict(m) for m in instance_metrics]

        results_path = cond_output / "retrieval_results.json"
        results_path.write_text(json.dumps(agg, indent=2))
        logger.info("Results saved to %s", results_path)

        all_results[cond_id] = agg

    # Print summary
    report = _format_report(all_results)
    print(report)

    report_path = Path(output_dir) / "retrieval_report.md"
    report_path.parent.mkdir(parents=True, exist_ok=True)
    report_path.write_text(report)
    logger.info("Report saved to %s", report_path)

    return all_results


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _build_conditions(
    condition_ids: list[str],
    embedding: Optional[str],
    reranker: Optional[str],
    search_limit: int,
) -> dict[str, ExperimentCondition]:
    """Build ExperimentCondition objects from IDs with optional overrides."""
    from eval.swebench.config import ALL_CONDITIONS

    predefined = {c.condition_id: c for c in ALL_CONDITIONS}
    result: dict[str, ExperimentCondition] = {}

    for cid in condition_ids:
        if cid in predefined:
            c = predefined[cid]
            # Apply CLI overrides for non-baseline conditions
            if not c.is_baseline and (embedding is not None or reranker is not None):
                c = ExperimentCondition(
                    condition_id=c.condition_id,
                    embedding_backend=embedding if embedding is not None else c.embedding_backend,
                    reranker_backend=reranker if reranker is not None else c.reranker_backend,
                    search_limit=search_limit,
                )
            result[cid] = c
        else:
            # Create ad-hoc condition from ID
            result[cid] = ExperimentCondition(
                condition_id=cid,
                embedding_backend=embedding,
                reranker_backend=reranker,
                search_limit=search_limit,
            )

    return result


def _aggregate_metrics(metrics: list[InstanceMetrics]) -> dict:
    """Compute aggregate statistics across instances."""
    if not metrics:
        return {
            "file_recall_at_1": 0.0,
            "file_recall_at_5": 0.0,
            "file_recall_at_10": 0.0,
            "file_recall_at_20": 0.0,
            "function_recall_at_5": 0.0,
            "function_recall_at_10": 0.0,
            "file_mrr": 0.0,
            "function_mrr": 0.0,
            "avg_first_gold_file_rank": 0.0,
            "pct_gold_file_found": 0.0,
            "avg_index_time_secs": 0.0,
            "avg_search_time_secs": 0.0,
        }

    n = len(metrics)

    def _avg(field: str) -> float:
        return sum(getattr(m, field) for m in metrics) / n

    found = sum(1 for m in metrics if m.first_gold_file_rank > 0)
    ranks_of_found = [m.first_gold_file_rank for m in metrics if m.first_gold_file_rank > 0]

    return {
        "file_recall_at_1": round(_avg("file_recall_at_1"), 4),
        "file_recall_at_5": round(_avg("file_recall_at_5"), 4),
        "file_recall_at_10": round(_avg("file_recall_at_10"), 4),
        "file_recall_at_20": round(_avg("file_recall_at_20"), 4),
        "function_recall_at_5": round(_avg("function_recall_at_5"), 4),
        "function_recall_at_10": round(_avg("function_recall_at_10"), 4),
        "file_mrr": round(_avg("file_mrr"), 4),
        "function_mrr": round(_avg("function_mrr"), 4),
        "avg_first_gold_file_rank": round(
            sum(ranks_of_found) / len(ranks_of_found) if ranks_of_found else 0.0, 2,
        ),
        "pct_gold_file_found": round(found / n * 100, 1),
        "avg_index_time_secs": round(_avg("index_time_secs"), 2),
        "avg_search_time_secs": round(_avg("search_time_secs"), 4),
    }


def _format_report(all_results: dict[str, dict]) -> str:
    """Format aggregated results into a markdown report."""
    lines: list[str] = []
    lines.append("# SWE-bench Retrieval Evaluation Results\n")

    # Summary table
    lines.append("## Summary\n")
    lines.append(
        "| Condition | N | File R@1 | File R@5 | File R@10 | File R@20 "
        "| File MRR | Func R@5 | Func R@10 | Func MRR | Found% | Avg Rank |"
    )
    lines.append(
        "|-----------|---|----------|----------|-----------|-----------|"
        "----------|----------|-----------|----------|--------|----------|"
    )

    for cid, agg in all_results.items():
        lines.append(
            f"| {cid} "
            f"| {agg['num_instances']} "
            f"| {agg['file_recall_at_1']:.2%} "
            f"| {agg['file_recall_at_5']:.2%} "
            f"| {agg['file_recall_at_10']:.2%} "
            f"| {agg['file_recall_at_20']:.2%} "
            f"| {agg['file_mrr']:.3f} "
            f"| {agg['function_recall_at_5']:.2%} "
            f"| {agg['function_recall_at_10']:.2%} "
            f"| {agg['function_mrr']:.3f} "
            f"| {agg['pct_gold_file_found']:.0f}% "
            f"| {agg['avg_first_gold_file_rank']:.1f} |"
        )

    lines.append("")

    # Per-instance details for each condition
    for cid, agg in all_results.items():
        lines.append(f"## Per-Instance: {cid}\n")
        lines.append(
            "| Instance | Gold Files | R@1 | R@5 | R@10 | MRR | Rank | Queries |"
        )
        lines.append(
            "|----------|------------|-----|-----|------|-----|------|---------|"
        )

        for inst in agg.get("instances", []):
            gold_str = ", ".join(Path(f).name for f in inst["gold_files"])
            lines.append(
                f"| {inst['instance_id']} "
                f"| {gold_str} "
                f"| {inst['file_recall_at_1']:.0%} "
                f"| {inst['file_recall_at_5']:.0%} "
                f"| {inst['file_recall_at_10']:.0%} "
                f"| {inst['file_mrr']:.3f} "
                f"| {inst['first_gold_file_rank']} "
                f"| {inst['num_queries']} |"
            )

        lines.append("")

    return "\n".join(lines)
