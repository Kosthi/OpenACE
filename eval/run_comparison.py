#!/usr/bin/env python3
"""Side-by-side comparison of OpenACE vs ace-tool on SWE-bench Lite.

Runs both engines on the same instances with identical queries, then
outputs per-instance and aggregate comparison metrics.

Usage:
    uv run python eval/run_comparison.py \
      --subset lite --split test --slice "0:20" \
      --embedding siliconflow --reranker siliconflow \
      --embedding-base-url "https://router.tumuer.me/v1" \
      --embedding-api-key "$OPENACE_API_KEY" \
      --reranker-base-url "https://router.tumuer.me/v1" \
      --reranker-api-key "$OPENACE_API_KEY" \
      -o eval/output_comparison
"""

from __future__ import annotations

import argparse
import asyncio
import json
import logging
import sys
import time
from dataclasses import asdict, dataclass, field
from pathlib import Path
from typing import Optional

# Ensure project root is on sys.path
_PROJECT_ROOT = str(Path(__file__).resolve().parent.parent)
if _PROJECT_ROOT not in sys.path:
    sys.path.insert(0, _PROJECT_ROOT)

from eval.ace_tool_eval import parse_ace_tool_output
from eval.swebench.context_retrieval import _dedupe_by_symbol_id, generate_queries
from eval.swebench.dataset import load_dataset
from eval.swebench.repo_manager import clone_or_reuse, strip_non_source_files
from eval.swebench.retrieval_eval import (
    GoldInfo,
    _first_rank,
    _mrr,
    _recall_at_k,
    extract_gold_from_patch,
)
from openace.search_utils import _aggregate_by_file, _apply_file_score_gap

logger = logging.getLogger(__name__)


# ---------------------------------------------------------------------------
# Data classes
# ---------------------------------------------------------------------------


@dataclass
class EngineResult:
    """Retrieval results from a single engine on a single instance."""

    retrieved_files: list[str]
    retrieved_functions: list[str]
    elapsed_secs: float
    # OpenACE-specific timing breakdown (None for ace-tool)
    index_time_secs: Optional[float] = None
    search_time_secs: Optional[float] = None
    error: Optional[str] = None


@dataclass
class ComparisonRow:
    """Per-instance comparison between OpenACE and ace-tool."""

    instance_id: str
    query: str
    gold_files: list[str]
    gold_functions: list[str]

    # OpenACE metrics
    oa_retrieved_files: list[str] = field(default_factory=list)
    oa_file_recall_at_1: float = 0.0
    oa_file_recall_at_5: float = 0.0
    oa_file_recall_at_10: float = 0.0
    oa_file_recall_at_20: float = 0.0
    oa_file_mrr: float = 0.0
    oa_first_rank: int = 0
    oa_index_time: float = 0.0
    oa_search_time: float = 0.0
    oa_error: Optional[str] = None

    # ace-tool metrics
    at_retrieved_files: list[str] = field(default_factory=list)
    at_file_recall_at_1: float = 0.0
    at_file_recall_at_5: float = 0.0
    at_file_recall_at_10: float = 0.0
    at_file_recall_at_20: float = 0.0
    at_file_mrr: float = 0.0
    at_first_rank: int = 0
    at_elapsed: float = 0.0
    at_error: Optional[str] = None


# ---------------------------------------------------------------------------
# OpenACE retrieval
# ---------------------------------------------------------------------------


def run_openace(
    project_root: str,
    query: str,
    *,
    embedding_backend: Optional[str] = None,
    reranker_backend: Optional[str] = None,
    embedding_kwargs: Optional[dict] = None,
    reranker_kwargs: Optional[dict] = None,
    search_limit: int = 20,
    query_mode: str = "multi",
    problem_statement: str = "",
) -> EngineResult:
    """Index repo with OpenACE and retrieve files.

    Uses the same MCP-style aggregation as retrieval_eval.py for consistency.
    """
    from openace.engine import Engine

    embedding_provider = None
    reranker = None

    if embedding_backend is not None:
        from openace.embedding.factory import create_provider
        embedding_provider = create_provider(
            embedding_backend, **(embedding_kwargs or {}),
        )

    if reranker_backend is not None:
        from openace.reranking.factory import create_reranker
        reranker = create_reranker(
            reranker_backend, **(reranker_kwargs or {}),
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
        "OpenACE indexed %d files, %d symbols in %.1fs",
        report.files_indexed, report.total_symbols, index_time,
    )

    # Generate queries
    if query_mode == "single":
        queries = [query]
    else:
        queries = generate_queries(problem_statement)

    t1 = time.monotonic()

    all_results = []
    for q in queries:
        try:
            pool_size = min(search_limit * 5, 200)
            results = engine.search(q, limit=pool_size, dedupe_by_file=False)
            all_results.extend(results)
        except Exception:
            logger.warning("OpenACE search failed for query: %s", q, exc_info=True)

    unique_results = _dedupe_by_symbol_id(all_results)
    search_time = time.monotonic() - t1

    # MCP-style aggregation
    file_groups = _aggregate_by_file(unique_results)
    file_groups = _apply_file_score_gap(file_groups)
    file_groups = file_groups[:search_limit]

    retrieved_files = [g["file_path"] for g in file_groups]

    retrieved_functions: list[str] = []
    seen_funcs: set[str] = set()
    for group in file_groups:
        for r in group["symbols"]:
            if r.kind in ("function", "method") and r.name not in seen_funcs:
                seen_funcs.add(r.name)
                retrieved_functions.append(r.name)

    return EngineResult(
        retrieved_files=retrieved_files,
        retrieved_functions=retrieved_functions,
        elapsed_secs=round(index_time + search_time, 2),
        index_time_secs=round(index_time, 2),
        search_time_secs=round(search_time, 4),
    )


# ---------------------------------------------------------------------------
# ace-tool retrieval (via MCP stdio client)
# ---------------------------------------------------------------------------


async def _call_ace_search(
    project_path: str,
    query: str,
    *,
    ace_command: str = "ace-tool",
    ace_args: Optional[list[str]] = None,
) -> str:
    """Call ace-tool search_context via MCP stdio client."""
    from mcp import ClientSession
    from mcp.client.stdio import StdioServerParameters, stdio_client

    server = StdioServerParameters(
        command=ace_command,
        args=ace_args or [],
    )
    async with stdio_client(server) as (read, write):
        async with ClientSession(read_stream=read, write_stream=write) as session:
            await session.initialize()
            result = await session.call_tool(
                "search_context",
                {"project_root_path": project_path, "query": query},
            )
            return result.content[0].text if result.content else ""


def run_ace_tool(
    project_path: str,
    query: str,
    *,
    ace_command: str = "ace-tool",
    ace_args: Optional[list[str]] = None,
) -> EngineResult:
    """Run ace-tool search and parse results."""
    t0 = time.monotonic()
    try:
        raw_output = asyncio.run(
            _call_ace_search(
                project_path, query,
                ace_command=ace_command, ace_args=ace_args,
            )
        )
    except Exception as exc:
        elapsed = time.monotonic() - t0
        logger.error("ace-tool failed: %s", exc, exc_info=True)
        return EngineResult(
            retrieved_files=[],
            retrieved_functions=[],
            elapsed_secs=round(elapsed, 2),
            error=str(exc),
        )
    elapsed = time.monotonic() - t0

    retrieved_files, retrieved_functions = parse_ace_tool_output(raw_output)
    return EngineResult(
        retrieved_files=retrieved_files,
        retrieved_functions=retrieved_functions,
        elapsed_secs=round(elapsed, 2),
    )


# ---------------------------------------------------------------------------
# Comparison logic
# ---------------------------------------------------------------------------


def compare_instance(
    instance,
    project_root: str,
    gold: GoldInfo,
    *,
    embedding_backend: Optional[str] = None,
    reranker_backend: Optional[str] = None,
    embedding_kwargs: Optional[dict] = None,
    reranker_kwargs: Optional[dict] = None,
    search_limit: int = 20,
    query_mode: str = "multi",
    ace_command: str = "ace-tool",
    ace_args: Optional[list[str]] = None,
) -> ComparisonRow:
    """Run both engines on a single instance and compute comparison metrics."""
    query = instance.problem_statement[:500]

    row = ComparisonRow(
        instance_id=instance.instance_id,
        query=query[:100] + "..." if len(query) > 100 else query,
        gold_files=gold.files,
        gold_functions=gold.functions,
    )

    # --- OpenACE ---
    try:
        oa = run_openace(
            project_root,
            query,
            embedding_backend=embedding_backend,
            reranker_backend=reranker_backend,
            embedding_kwargs=embedding_kwargs,
            reranker_kwargs=reranker_kwargs,
            search_limit=search_limit,
            query_mode=query_mode,
            problem_statement=instance.problem_statement,
        )
        row.oa_retrieved_files = oa.retrieved_files
        row.oa_file_recall_at_1 = _recall_at_k(gold.files, oa.retrieved_files, 1)
        row.oa_file_recall_at_5 = _recall_at_k(gold.files, oa.retrieved_files, 5)
        row.oa_file_recall_at_10 = _recall_at_k(gold.files, oa.retrieved_files, 10)
        row.oa_file_recall_at_20 = _recall_at_k(gold.files, oa.retrieved_files, 20)
        row.oa_file_mrr = _mrr(gold.files, oa.retrieved_files)
        row.oa_first_rank = _first_rank(gold.files, oa.retrieved_files)
        row.oa_index_time = oa.index_time_secs or 0.0
        row.oa_search_time = oa.search_time_secs or 0.0
        row.oa_error = oa.error
    except Exception as exc:
        logger.error("OpenACE failed for %s: %s", instance.instance_id, exc, exc_info=True)
        row.oa_error = str(exc)

    # --- ace-tool ---
    try:
        at = run_ace_tool(
            project_root, query,
            ace_command=ace_command, ace_args=ace_args,
        )
        row.at_retrieved_files = at.retrieved_files
        row.at_file_recall_at_1 = _recall_at_k(gold.files, at.retrieved_files, 1)
        row.at_file_recall_at_5 = _recall_at_k(gold.files, at.retrieved_files, 5)
        row.at_file_recall_at_10 = _recall_at_k(gold.files, at.retrieved_files, 10)
        row.at_file_recall_at_20 = _recall_at_k(gold.files, at.retrieved_files, 20)
        row.at_file_mrr = _mrr(gold.files, at.retrieved_files)
        row.at_first_rank = _first_rank(gold.files, at.retrieved_files)
        row.at_elapsed = at.elapsed_secs
        row.at_error = at.error
    except Exception as exc:
        logger.error("ace-tool failed for %s: %s", instance.instance_id, exc, exc_info=True)
        row.at_error = str(exc)

    return row


# ---------------------------------------------------------------------------
# Aggregate reporting
# ---------------------------------------------------------------------------


def _aggregate(rows: list[ComparisonRow]) -> dict:
    """Compute aggregate metrics for both engines."""
    n = len(rows)
    if n == 0:
        return {}

    def _avg(getter) -> float:
        return sum(getter(r) for r in rows) / n

    oa_found = sum(1 for r in rows if r.oa_first_rank > 0)
    at_found = sum(1 for r in rows if r.at_first_rank > 0)
    oa_ranks = [r.oa_first_rank for r in rows if r.oa_first_rank > 0]
    at_ranks = [r.at_first_rank for r in rows if r.at_first_rank > 0]

    return {
        "num_instances": n,
        "openace": {
            "file_recall_at_1": round(_avg(lambda r: r.oa_file_recall_at_1) * 100, 1),
            "file_recall_at_5": round(_avg(lambda r: r.oa_file_recall_at_5) * 100, 1),
            "file_recall_at_10": round(_avg(lambda r: r.oa_file_recall_at_10) * 100, 1),
            "file_recall_at_20": round(_avg(lambda r: r.oa_file_recall_at_20) * 100, 1),
            "file_mrr": round(_avg(lambda r: r.oa_file_mrr), 3),
            "pct_found": round(oa_found / n * 100, 1),
            "avg_rank": round(sum(oa_ranks) / len(oa_ranks), 2) if oa_ranks else 0.0,
            "avg_index_time": round(_avg(lambda r: r.oa_index_time), 2),
            "avg_search_time": round(_avg(lambda r: r.oa_search_time), 4),
            "errors": sum(1 for r in rows if r.oa_error),
        },
        "ace_tool": {
            "file_recall_at_1": round(_avg(lambda r: r.at_file_recall_at_1) * 100, 1),
            "file_recall_at_5": round(_avg(lambda r: r.at_file_recall_at_5) * 100, 1),
            "file_recall_at_10": round(_avg(lambda r: r.at_file_recall_at_10) * 100, 1),
            "file_recall_at_20": round(_avg(lambda r: r.at_file_recall_at_20) * 100, 1),
            "file_mrr": round(_avg(lambda r: r.at_file_mrr), 3),
            "pct_found": round(at_found / n * 100, 1),
            "avg_rank": round(sum(at_ranks) / len(at_ranks), 2) if at_ranks else 0.0,
            "avg_elapsed": round(_avg(lambda r: r.at_elapsed), 2),
            "errors": sum(1 for r in rows if r.at_error),
        },
    }


def _print_comparison_table(agg: dict) -> None:
    """Print the aggregate comparison table to stdout."""
    oa = agg["openace"]
    at = agg["ace_tool"]
    n = agg["num_instances"]

    print(f"\n{'='*60}")
    print(f"  Aggregate Comparison ({n} instances)")
    print(f"{'='*60}")
    print(f"{'Metric':<20s} {'OpenACE':>10s} {'ace-tool':>10s} {'delta':>10s}")
    print(f"{'-'*20} {'-'*10} {'-'*10} {'-'*10}")

    metrics = [
        ("File R@1", "file_recall_at_1", "%"),
        ("File R@5", "file_recall_at_5", "%"),
        ("File R@10", "file_recall_at_10", "%"),
        ("File R@20", "file_recall_at_20", "%"),
        ("File MRR", "file_mrr", ""),
        ("Found%", "pct_found", "%"),
        ("Avg Rank", "avg_rank", ""),
    ]

    for label, key, unit in metrics:
        oa_val = oa[key]
        at_val = at[key]
        delta = oa_val - at_val

        if unit == "%":
            oa_str = f"{oa_val:.1f}%"
            at_str = f"{at_val:.1f}%"
            delta_str = f"{delta:+.1f}pp"
        else:
            oa_str = f"{oa_val:.3f}"
            at_str = f"{at_val:.3f}"
            delta_str = f"{delta:+.3f}"

        print(f"{label:<20s} {oa_str:>10s} {at_str:>10s} {delta_str:>10s}")

    # Timing row
    oa_time = f"{oa['avg_index_time']:.1f}+{oa['avg_search_time']:.2f}s"
    at_time = f"{at['avg_elapsed']:.1f}s"
    print(f"{'Avg Time':<20s} {oa_time:>10s} {at_time:>10s} {'':>10s}")

    if oa["errors"] or at["errors"]:
        print(f"{'Errors':<20s} {oa['errors']:>10d} {at['errors']:>10d}")

    print()


# ---------------------------------------------------------------------------
# Main entry point
# ---------------------------------------------------------------------------


def run_comparison(
    *,
    subset: str = "lite",
    split: str = "test",
    slice_spec: str = "",
    instance_ids: Optional[list[str]] = None,
    output_dir: str = "eval/output_comparison",
    repos_dir: str = "eval/repos",
    embedding: Optional[str] = None,
    reranker: Optional[str] = None,
    search_limit: int = 20,
    embedding_base_url: Optional[str] = None,
    embedding_api_key: Optional[str] = None,
    reranker_base_url: Optional[str] = None,
    reranker_api_key: Optional[str] = None,
    query_mode: str = "multi",
    ace_command: str = "ace-tool",
    ace_base_url: Optional[str] = None,
    ace_token: Optional[str] = None,
) -> dict:
    """Run side-by-side comparison of OpenACE and ace-tool."""
    embedding_kwargs: dict = {}
    if embedding_base_url:
        embedding_kwargs["base_url"] = embedding_base_url
    if embedding_api_key:
        embedding_kwargs["api_key"] = embedding_api_key

    reranker_kwargs: dict = {}
    if reranker_base_url:
        reranker_kwargs["base_url"] = reranker_base_url
    if reranker_api_key:
        reranker_kwargs["api_key"] = reranker_api_key

    # Build ace-tool args
    ace_args: list[str] = []
    if ace_base_url:
        ace_args.extend(["--base-url", ace_base_url])
    if ace_token:
        ace_args.extend(["--token", ace_token])

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
    if instance_ids:
        id_set = set(instance_ids)
        instances = [inst for inst in instances if inst.instance_id in id_set]

    logger.info(
        "Loaded %d instances from %s/%s", len(instances), dataset_name, split,
    )

    out_path = Path(output_dir)
    out_path.mkdir(parents=True, exist_ok=True)

    # Resume from checkpoint if it exists
    rows: list[ComparisonRow] = []
    done_ids: set[str] = set()
    checkpoint_file = out_path / "comparison_results.json"
    if checkpoint_file.exists():
        try:
            prev = json.loads(checkpoint_file.read_text())
            for inst_data in prev.get("instances", []):
                rows.append(ComparisonRow(**inst_data))
                done_ids.add(inst_data["instance_id"])
            if done_ids:
                logger.info("Resumed %d instances from checkpoint", len(done_ids))
        except Exception as exc:
            logger.warning("Failed to load checkpoint, starting fresh: %s", exc)
            rows = []
            done_ids = set()

    for i, inst in enumerate(instances):
        if inst.instance_id in done_ids:
            logger.info("Skipping %s (already in checkpoint)", inst.instance_id)
            continue

        gold = extract_gold_from_patch(inst.patch)
        if not gold.files:
            logger.warning(
                "No gold files from patch for %s, skipping", inst.instance_id,
            )
            continue

        print(f"\n[{i + 1}/{len(instances)}] {inst.instance_id}")
        print(f"  Gold files: {gold.files}")

        try:
            repo_path = clone_or_reuse(inst.repo, inst.base_commit, repos_dir)
        except Exception as exc:
            logger.error("Repo clone failed for %s: %s", inst.instance_id, exc)
            continue

        strip_non_source_files(repo_path)

        row = compare_instance(
            inst,
            str(repo_path),
            gold,
            embedding_backend=embedding,
            reranker_backend=reranker,
            embedding_kwargs=embedding_kwargs or None,
            reranker_kwargs=reranker_kwargs or None,
            search_limit=search_limit,
            query_mode=query_mode,
            ace_command=ace_command,
            ace_args=ace_args or None,
        )
        rows.append(row)

        # Print per-instance results
        _print_instance(row)

        # Save checkpoint after each instance
        _save_results(out_path, rows)

    # Aggregate and print
    agg = _aggregate(rows)
    _print_comparison_table(agg)

    # Save final results
    _save_results(out_path, rows, agg)

    return agg


def _print_instance(row: ComparisonRow) -> None:
    """Print per-instance comparison line."""
    oa_status = "ERROR" if row.oa_error else (
        f"rank={row.oa_first_rank} R@5={row.oa_file_recall_at_5:.0%} "
        f"MRR={row.oa_file_mrr:.3f}  ({row.oa_index_time:.1f}s idx + {row.oa_search_time:.2f}s search)"
    )
    at_status = "ERROR" if row.at_error else (
        f"rank={row.at_first_rank} R@5={row.at_file_recall_at_5:.0%} "
        f"MRR={row.at_file_mrr:.3f}  ({row.at_elapsed:.1f}s)"
    )

    print(f"  OpenACE:  {oa_status}")
    print(f"  ace-tool: {at_status}")


def _save_results(
    out_path: Path,
    rows: list[ComparisonRow],
    agg: Optional[dict] = None,
) -> None:
    """Save results to JSON."""
    data = {
        "instances": [asdict(r) for r in rows],
    }
    if agg is not None:
        data["aggregate"] = agg

    results_file = out_path / "comparison_results.json"
    results_file.write_text(json.dumps(data, indent=2, ensure_ascii=False))


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Side-by-side comparison of OpenACE vs ace-tool on SWE-bench",
    )
    parser.add_argument(
        "--subset", default="lite",
        help="SWE-bench subset: lite, verified, full (default: lite)",
    )
    parser.add_argument(
        "--split", default="test",
        help="Dataset split (default: test)",
    )
    parser.add_argument(
        "--slice", dest="slice_spec", default="",
        help="Instance slice, e.g. '0:20' for first 20",
    )
    parser.add_argument(
        "--instance-ids", default=None,
        help="Comma-separated instance IDs to evaluate (overrides --slice)",
    )
    parser.add_argument(
        "--output-dir", "-o", default="eval/output_comparison",
        help="Output directory (default: eval/output_comparison)",
    )
    parser.add_argument(
        "--repos-dir", default="eval/repos",
        help="Repos cache directory (default: eval/repos)",
    )
    parser.add_argument(
        "--embedding", default=None,
        help="OpenACE embedding backend (e.g. siliconflow)",
    )
    parser.add_argument(
        "--embedding-base-url", default=None,
        help="Override embedding API base URL",
    )
    parser.add_argument(
        "--embedding-api-key", default=None,
        help="Override embedding API key",
    )
    parser.add_argument(
        "--reranker", default=None,
        help="OpenACE reranker backend (e.g. siliconflow)",
    )
    parser.add_argument(
        "--reranker-base-url", default=None,
        help="Override reranker API base URL",
    )
    parser.add_argument(
        "--reranker-api-key", default=None,
        help="Override reranker API key",
    )
    parser.add_argument(
        "--limit", type=int, default=20,
        help="Search result limit (default: 20)",
    )
    parser.add_argument(
        "--query-mode", default="multi",
        choices=["multi", "single"],
        help="Query mode: 'multi' or 'single' (default: multi)",
    )
    parser.add_argument(
        "--ace-tool-command", default="ace-tool",
        help="ace-tool command (default: ace-tool)",
    )
    parser.add_argument(
        "--ace-tool-base-url", default=None,
        help="ace-tool server base URL (required for ace-tool)",
    )
    parser.add_argument(
        "--ace-tool-token", default=None,
        help="ace-tool auth token (required for ace-tool)",
    )
    parser.add_argument(
        "--verbose", "-v", action="store_true",
        help="Enable debug logging",
    )

    args = parser.parse_args()

    logging.basicConfig(
        level=logging.DEBUG if args.verbose else logging.INFO,
        format="%(asctime)s %(levelname)-8s %(name)s — %(message)s",
    )

    run_comparison(
        subset=args.subset,
        split=args.split,
        slice_spec=args.slice_spec,
        instance_ids=args.instance_ids.split(",") if args.instance_ids else None,
        output_dir=args.output_dir,
        repos_dir=args.repos_dir,
        embedding=args.embedding,
        reranker=args.reranker,
        search_limit=args.limit,
        embedding_base_url=args.embedding_base_url,
        embedding_api_key=args.embedding_api_key,
        reranker_base_url=args.reranker_base_url,
        reranker_api_key=args.reranker_api_key,
        query_mode=args.query_mode,
        ace_command=args.ace_tool_command,
        ace_base_url=args.ace_tool_base_url,
        ace_token=args.ace_tool_token,
    )


if __name__ == "__main__":
    main()
