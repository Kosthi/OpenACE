"""Result analysis and comparison across experiment conditions."""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from pathlib import Path


@dataclass
class ConditionResult:
    """Aggregated results for a single experiment condition."""

    condition_id: str
    total_instances: int = 0
    submitted: int = 0
    resolved: int = 0
    resolved_ids: list[str] = field(default_factory=list)

    @property
    def resolution_rate(self) -> float:
        if self.total_instances == 0:
            return 0.0
        return self.resolved / self.total_instances

    @property
    def submit_rate(self) -> float:
        if self.total_instances == 0:
            return 0.0
        return self.submitted / self.total_instances


def load_swebench_results(results_dir: str | Path) -> dict:
    """Load SWE-bench evaluation results JSON.

    SWE-bench outputs a results JSON file after running its Docker-based
    test harness. This function loads it.
    """
    results_dir = Path(results_dir)
    # SWE-bench typically outputs to a results.json file
    for candidate in [
        results_dir / "results.json",
        results_dir / "eval_results.json",
    ]:
        if candidate.exists():
            return json.loads(candidate.read_text())
    raise FileNotFoundError(
        f"No results JSON found in {results_dir}. "
        f"Expected results.json or eval_results.json."
    )


def analyze_condition(
    condition_id: str,
    results: dict,
    predictions_path: str | Path,
) -> ConditionResult:
    """Analyze results for a single experiment condition.

    Args:
        condition_id: The experiment condition identifier.
        results: Parsed SWE-bench evaluation results dict.
        predictions_path: Path to the predictions JSONL file.

    Returns:
        ConditionResult with aggregated metrics.
    """
    # Count submitted predictions
    submitted = 0
    predictions_path = Path(predictions_path)
    if predictions_path.exists():
        with open(predictions_path) as f:
            for line in f:
                if line.strip():
                    submitted += 1

    # Extract resolved instance IDs from SWE-bench results
    resolved_ids = results.get("resolved", [])
    if not isinstance(resolved_ids, list):
        resolved_ids = []

    total = results.get("total_instances", submitted)

    return ConditionResult(
        condition_id=condition_id,
        total_instances=total,
        submitted=submitted,
        resolved=len(resolved_ids),
        resolved_ids=resolved_ids,
    )


def compare_conditions(
    results: list[ConditionResult],
) -> str:
    """Generate a Markdown comparison table across conditions.

    Args:
        results: List of ConditionResult for each experiment condition.

    Returns:
        Markdown formatted comparison report.
    """
    lines: list[str] = []
    lines.append("# SWE-bench Lite Evaluation Results\n")
    lines.append("## Summary\n")
    lines.append("| Condition | Submitted | Resolved | Resolution Rate |")
    lines.append("|-----------|-----------|----------|-----------------|")
    for r in results:
        lines.append(
            f"| {r.condition_id} | {r.submitted} | {r.resolved} | "
            f"{r.resolution_rate:.1%} |"
        )

    # Find baseline for delta computation
    baseline = next((r for r in results if r.condition_id == "baseline"), None)

    if baseline is not None and baseline.resolved > 0:
        lines.append("\n## Delta vs Baseline\n")
        lines.append("| Condition | Baseline | Enhanced | Delta | Relative |")
        lines.append("|-----------|----------|----------|-------|----------|")
        for r in results:
            if r.condition_id == "baseline":
                continue
            delta = r.resolved - baseline.resolved
            relative = (
                f"+{delta / baseline.resolved:.0%}"
                if baseline.resolved > 0
                else "N/A"
            )
            lines.append(
                f"| {r.condition_id} | {baseline.resolved} | {r.resolved} | "
                f"{delta:+d} | {relative} |"
            )

    # Per-instance analysis: which instances flipped from fail to pass
    if baseline is not None:
        baseline_set = set(baseline.resolved_ids)
        lines.append("\n## Per-Instance Analysis\n")
        for r in results:
            if r.condition_id == "baseline":
                continue
            enhanced_set = set(r.resolved_ids)
            gained = sorted(enhanced_set - baseline_set)
            lost = sorted(baseline_set - enhanced_set)
            if gained:
                lines.append(
                    f"### {r.condition_id}: Gained ({len(gained)} instances)\n"
                )
                for inst_id in gained:
                    lines.append(f"- {inst_id}")
                lines.append("")
            if lost:
                lines.append(
                    f"### {r.condition_id}: Lost ({len(lost)} instances)\n"
                )
                for inst_id in lost:
                    lines.append(f"- {inst_id}")
                lines.append("")

    return "\n".join(lines)


def generate_report(
    output_dir: str | Path,
    condition_ids: list[str] | None = None,
) -> str:
    """Generate a full comparison report from evaluation output directory.

    Expects each condition to have:
    - ``output_dir/{condition_id}/predictions.jsonl``
    - ``output_dir/{condition_id}/results.json`` (from SWE-bench harness)

    Args:
        output_dir: Root output directory.
        condition_ids: Specific conditions to compare. If None, auto-detect.

    Returns:
        Markdown report string.
    """
    output_dir = Path(output_dir)

    if condition_ids is None:
        condition_ids = sorted(
            d.name
            for d in output_dir.iterdir()
            if d.is_dir() and (d / "predictions.jsonl").exists()
        )

    all_results: list[ConditionResult] = []
    for cid in condition_ids:
        cond_dir = output_dir / cid
        predictions_path = cond_dir / "predictions.jsonl"

        try:
            swe_results = load_swebench_results(cond_dir)
        except FileNotFoundError:
            # Results not yet available — show submitted count only
            swe_results = {}

        cr = analyze_condition(cid, swe_results, predictions_path)
        all_results.append(cr)

    report = compare_conditions(all_results)

    report_path = output_dir / "report.md"
    report_path.write_text(report)

    return report
