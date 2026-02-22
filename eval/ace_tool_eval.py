"""Evaluate ace-tool (Augment context engine) against SWE-bench gold patches.

Usage:
    # Step 1: Generate manifest (already done)
    # Step 2: Call ace-tool for each instance and save results
    # Step 3: Compute metrics
    uv run python eval/ace_tool_eval.py compute-metrics eval/ace_tool_results.json
"""

from __future__ import annotations

import json
import re
import sys
from dataclasses import dataclass
from pathlib import Path


@dataclass
class AceToolInstanceResult:
    instance_id: str
    gold_files: list[str]
    gold_functions: list[str]
    retrieved_files: list[str]
    retrieved_functions: list[str]


def parse_ace_tool_output(raw_output: str) -> tuple[list[str], list[str]]:
    """Parse ace-tool output to extract ordered file paths and function names.

    ace-tool returns snippets like:
        Path: src/foo/bar.py
        ...
        <code>
        ...
        Path: src/foo/baz.py#chunk2of3
        ...

    Returns:
        (ordered_file_paths, ordered_function_names)
    """
    file_paths: list[str] = []
    seen_files: set[str] = set()
    function_names: list[str] = []
    seen_funcs: set[str] = set()

    # Extract file paths in order (deduplicate, keep first occurrence)
    for m in re.finditer(r'^Path:\s+(.+?)(?:#chunk\d+of\d+)?$', raw_output, re.MULTILINE):
        path = m.group(1).strip()
        if path not in seen_files:
            seen_files.add(path)
            file_paths.append(path)

    # Extract function/method definitions from code snippets
    # Look for Python function definitions: def func_name(
    for m in re.finditer(r'^\s*(?:async\s+)?def\s+(\w+)\s*\(', raw_output, re.MULTILINE):
        name = m.group(1)
        if name not in seen_funcs and name not in {'__init__', '__str__', '__repr__'}:
            seen_funcs.add(name)
            function_names.append(name)

    # Also look for class definitions
    for m in re.finditer(r'^\s*class\s+(\w+)\s*[:(]', raw_output, re.MULTILINE):
        name = m.group(1)
        if name not in seen_funcs:
            seen_funcs.add(name)
            function_names.append(name)

    return file_paths, function_names


def _recall_at_k(gold: list[str], retrieved: list[str], k: int) -> float:
    """Fraction of gold items found in the top-k retrieved items."""
    if not gold:
        return 1.0
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


def _match_gold_files(gold_files: list[str], retrieved_files: list[str]) -> list[str]:
    """Match retrieved files against gold files by basename.

    ace-tool returns relative paths. Gold files from SWE-bench are also relative.
    We match by full path first, then by basename as fallback.
    """
    matched: list[str] = []
    gold_basenames = {Path(f).name: f for f in gold_files}

    for rf in retrieved_files:
        # Try full path match first
        if rf in gold_files:
            matched.append(rf)
            continue
        # Try basename match
        basename = Path(rf).name
        if basename in gold_basenames:
            matched.append(gold_basenames[basename])
        else:
            matched.append(rf)  # keep original for non-gold files

    return matched


def _match_gold_funcs(gold_functions: list[str], retrieved_functions: list[str]) -> list[str]:
    """Match retrieved function names against gold functions."""
    # Simple name matching - gold functions from hunk headers are just names
    return retrieved_functions


def compute_instance_metrics(result: AceToolInstanceResult) -> dict:
    """Compute all metrics for a single instance."""
    # Normalize retrieved files to match gold format
    retrieved_files_normalized = _match_gold_files(
        result.gold_files, result.retrieved_files
    )

    return {
        "instance_id": result.instance_id,
        "gold_files": result.gold_files,
        "gold_functions": result.gold_functions,
        "retrieved_files": result.retrieved_files,
        "retrieved_functions": result.retrieved_functions,
        "file_recall_at_1": _recall_at_k(result.gold_files, retrieved_files_normalized, 1),
        "file_recall_at_5": _recall_at_k(result.gold_files, retrieved_files_normalized, 5),
        "file_recall_at_10": _recall_at_k(result.gold_files, retrieved_files_normalized, 10),
        "file_recall_at_20": _recall_at_k(result.gold_files, retrieved_files_normalized, 20),
        "function_recall_at_5": _recall_at_k(result.gold_functions, result.retrieved_functions, 5),
        "function_recall_at_10": _recall_at_k(result.gold_functions, result.retrieved_functions, 10),
        "file_mrr": _mrr(result.gold_files, retrieved_files_normalized),
        "function_mrr": _mrr(result.gold_functions, result.retrieved_functions),
        "first_gold_file_rank": _first_rank(result.gold_files, retrieved_files_normalized),
    }


def compute_aggregate_metrics(instance_metrics: list[dict]) -> dict:
    """Compute aggregate metrics across all instances."""
    n = len(instance_metrics)
    if n == 0:
        return {}

    def _avg(field: str) -> float:
        return sum(m[field] for m in instance_metrics) / n

    found = sum(1 for m in instance_metrics if m["first_gold_file_rank"] > 0)
    ranks_of_found = [m["first_gold_file_rank"] for m in instance_metrics if m["first_gold_file_rank"] > 0]

    return {
        "num_instances": n,
        "file_recall_at_1": round(_avg("file_recall_at_1") * 100, 1),
        "file_recall_at_5": round(_avg("file_recall_at_5") * 100, 1),
        "file_recall_at_10": round(_avg("file_recall_at_10") * 100, 1),
        "file_recall_at_20": round(_avg("file_recall_at_20") * 100, 1),
        "function_recall_at_5": round(_avg("function_recall_at_5") * 100, 1),
        "function_recall_at_10": round(_avg("function_recall_at_10") * 100, 1),
        "file_mrr": round(_avg("file_mrr"), 3),
        "function_mrr": round(_avg("function_mrr"), 3),
        "pct_gold_file_found": round(found / n * 100, 1),
        "avg_first_gold_file_rank": round(
            sum(ranks_of_found) / len(ranks_of_found) if ranks_of_found else 0.0, 2
        ),
    }


def main():
    if len(sys.argv) < 3:
        print("Usage: python eval/ace_tool_eval.py compute-metrics <results.json>")
        sys.exit(1)

    command = sys.argv[1]
    results_file = sys.argv[2]

    if command == "compute-metrics":
        with open(results_file) as f:
            data = json.load(f)

        instance_metrics = []
        for entry in data["instances"]:
            result = AceToolInstanceResult(
                instance_id=entry["instance_id"],
                gold_files=entry["gold_files"],
                gold_functions=entry["gold_functions"],
                retrieved_files=entry["retrieved_files"],
                retrieved_functions=entry["retrieved_functions"],
            )
            metrics = compute_instance_metrics(result)
            instance_metrics.append(metrics)

            gf_short = [Path(f).name for f in result.gold_files]
            print(
                f"{metrics['instance_id']:40s} | "
                f"rank={metrics['first_gold_file_rank']:2d} | "
                f"R@1={metrics['file_recall_at_1']:.0%} | "
                f"R@5={metrics['file_recall_at_5']:.0%} | "
                f"R@10={metrics['file_recall_at_10']:.0%} | "
                f"R@20={metrics['file_recall_at_20']:.0%} | "
                f"MRR={metrics['file_mrr']:.3f} | "
                f"gold={gf_short}"
            )

        agg = compute_aggregate_metrics(instance_metrics)
        print("\n=== Aggregate ===")
        for k, v in agg.items():
            print(f"  {k}: {v}")

        # Save full results
        output = {
            "aggregate": agg,
            "instances": instance_metrics,
        }
        out_path = Path(results_file).with_suffix(".metrics.json")
        with open(out_path, "w") as f:
            json.dump(output, f, indent=2)
        print(f"\nMetrics saved to {out_path}")

    else:
        print(f"Unknown command: {command}")
        sys.exit(1)


if __name__ == "__main__":
    main()
