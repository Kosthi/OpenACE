"""Main evaluation orchestrator with checkpoint/resume support."""

from __future__ import annotations

import json
import logging
import time
from pathlib import Path

from eval.swebench.config import EvalConfig, ExperimentCondition
from eval.swebench.context_retrieval import retrieve_context
from eval.swebench.dataset import SWEInstance, load_dataset
from eval.swebench.formatter import save_prediction
from eval.swebench.llm_client import create_llm_client
from eval.swebench.patch_generator import generate_patch
from eval.swebench.repo_manager import clone_or_reuse

logger = logging.getLogger(__name__)


def run_evaluation(config: EvalConfig) -> None:
    """Run the full evaluation pipeline.

    For each (condition, instance) pair:
    1. Clone/checkout the repository at the correct commit.
    2. Retrieve relevant context using OpenACE (skip for baseline).
    3. Generate a patch using the configured LLM.
    4. Save the prediction to JSONL.
    5. Update the checkpoint.

    Interrupted runs can be resumed — completed pairs are skipped.
    """
    instances = load_dataset(
        config.dataset,
        max_instances=config.max_instances,
        instance_ids=config.instance_ids,
    )
    logger.info("Loaded %d instances from %s", len(instances), config.dataset)

    client = create_llm_client(config.llm)

    output_dir = Path(config.output_dir)
    output_dir.mkdir(parents=True, exist_ok=True)

    total_pairs = len(config.conditions) * len(instances)
    completed_count = 0
    failed_count = 0

    for condition in config.conditions:
        checkpoint = _load_checkpoint(output_dir, condition)
        logger.info(
            "Condition %r: %d already completed, %d remaining",
            condition.condition_id,
            len(checkpoint),
            len(instances) - len(checkpoint),
        )

        for i, instance in enumerate(instances):
            if instance.instance_id in checkpoint:
                completed_count += 1
                continue

            pair_label = (
                f"[{condition.condition_id}] {instance.instance_id} "
                f"({i + 1}/{len(instances)})"
            )

            try:
                t0 = time.monotonic()
                _process_instance(
                    config, client, condition, instance, output_dir,
                )
                elapsed = time.monotonic() - t0
                logger.info("%s — done (%.1fs)", pair_label, elapsed)
                completed_count += 1
            except Exception:
                logger.error(
                    "%s — FAILED", pair_label, exc_info=True,
                )
                failed_count += 1

    logger.info(
        "Evaluation complete: %d/%d succeeded, %d failed",
        completed_count, total_pairs, failed_count,
    )

    # Log token usage if available
    if hasattr(client, "usage"):
        u = client.usage
        logger.info(
            "Token usage — prompt: %d, completion: %d, total: %d",
            u.prompt_tokens, u.completion_tokens, u.total_tokens,
        )


def _process_instance(
    config: EvalConfig,
    client,
    condition: ExperimentCondition,
    instance: SWEInstance,
    output_dir: Path,
) -> None:
    """Process a single (condition, instance) pair."""
    # 1. Prepare repository
    repo_path = clone_or_reuse(
        instance.repo,
        instance.base_commit,
        config.repos_dir,
    )

    # 2. Retrieve context (skip for baseline)
    context = ""
    if not condition.is_baseline:
        context = retrieve_context(
            str(repo_path),
            instance.problem_statement,
            condition,
        )

    # 3. Generate patch
    patch = generate_patch(
        client,
        instance.problem_statement,
        context,
        hints_text=instance.hints_text,
    )

    # 4. Save prediction
    save_prediction(
        condition, config.llm, instance.instance_id, patch, output_dir,
    )

    # 5. Update checkpoint
    _save_checkpoint(output_dir, condition, instance.instance_id)


# ---------------------------------------------------------------------------
# Checkpoint management
# ---------------------------------------------------------------------------

def _checkpoint_path(output_dir: Path, condition: ExperimentCondition) -> Path:
    return output_dir / condition.condition_id / "checkpoint.json"


def _load_checkpoint(
    output_dir: Path,
    condition: ExperimentCondition,
) -> set[str]:
    """Load the set of completed instance IDs for a condition."""
    path = _checkpoint_path(output_dir, condition)
    if not path.exists():
        return set()
    try:
        data = json.loads(path.read_text())
        return set(data.get("completed", []))
    except (json.JSONDecodeError, KeyError):
        return set()


def _save_checkpoint(
    output_dir: Path,
    condition: ExperimentCondition,
    instance_id: str,
) -> None:
    """Mark an instance as completed in the checkpoint file."""
    path = _checkpoint_path(output_dir, condition)
    path.parent.mkdir(parents=True, exist_ok=True)

    completed = _load_checkpoint(output_dir, condition)
    completed.add(instance_id)

    path.write_text(json.dumps({"completed": sorted(completed)}, indent=2))
