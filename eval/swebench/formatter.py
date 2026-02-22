"""Format predictions into SWE-bench compatible JSONL files."""

from __future__ import annotations

import json
from pathlib import Path

from eval.swebench.config import ExperimentCondition, LLMConfig


def format_model_name(condition: ExperimentCondition, llm: LLMConfig) -> str:
    """Encode experiment condition + LLM model into a model_name_or_path string.

    Examples:
        "openace-baseline+gpt-4o"
        "openace-full-pipeline+claude-sonnet-4-20250514"
    """
    return f"openace-{condition.condition_id}+{llm.model}"


def save_prediction(
    condition: ExperimentCondition,
    llm: LLMConfig,
    instance_id: str,
    patch: str,
    output_dir: str | Path,
) -> None:
    """Append a single prediction to the condition's JSONL file.

    Each line is a JSON object:
        {"instance_id": "...", "model_name_or_path": "...", "model_patch": "..."}
    """
    output_dir = Path(output_dir) / condition.condition_id
    output_dir.mkdir(parents=True, exist_ok=True)

    predictions_path = output_dir / "predictions.jsonl"
    record = {
        "instance_id": instance_id,
        "model_name_or_path": format_model_name(condition, llm),
        "model_patch": patch,
    }

    with open(predictions_path, "a") as f:
        f.write(json.dumps(record, ensure_ascii=False) + "\n")


def load_predictions(path: str | Path) -> list[dict]:
    """Load predictions from a JSONL file."""
    results = []
    with open(path) as f:
        for line in f:
            line = line.strip()
            if line:
                results.append(json.loads(line))
    return results
