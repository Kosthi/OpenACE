"""SWE-bench Lite dataset loader."""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Optional


@dataclass(frozen=True)
class SWEInstance:
    """A single SWE-bench task instance."""

    instance_id: str
    repo: str              # e.g. "scikit-learn/scikit-learn"
    base_commit: str
    problem_statement: str
    hints_text: str
    patch: str             # golden patch (for analysis only)
    fail_to_pass: list[str] = field(default_factory=list)
    pass_to_pass: list[str] = field(default_factory=list)


def load_dataset(
    dataset_name: str = "princeton-nlp/SWE-bench_Lite",
    *,
    split: str = "test",
    max_instances: Optional[int] = None,
    instance_ids: Optional[list[str]] = None,
) -> list[SWEInstance]:
    """Load SWE-bench Lite from HuggingFace and convert to SWEInstance list.

    Args:
        dataset_name: HuggingFace dataset identifier.
        split: Dataset split to load.
        max_instances: If set, limit to the first N instances.
        instance_ids: If set, only load these specific instance IDs.

    Returns:
        List of SWEInstance objects.
    """
    from datasets import load_dataset as hf_load

    ds = hf_load(dataset_name, split=split)

    instances: list[SWEInstance] = []
    for row in ds:
        inst = SWEInstance(
            instance_id=row["instance_id"],
            repo=row["repo"],
            base_commit=row["base_commit"],
            problem_statement=row["problem_statement"],
            hints_text=row.get("hints_text", ""),
            patch=row.get("patch", ""),
            fail_to_pass=_parse_test_list(row.get("FAIL_TO_PASS", "[]")),
            pass_to_pass=_parse_test_list(row.get("PASS_TO_PASS", "[]")),
        )
        instances.append(inst)

    if instance_ids is not None:
        id_set = set(instance_ids)
        instances = [i for i in instances if i.instance_id in id_set]

    if max_instances is not None:
        instances = instances[:max_instances]

    return instances


def _parse_test_list(raw: str | list) -> list[str]:
    """Parse a test list field which may be a JSON string or already a list."""
    if isinstance(raw, list):
        return raw
    import json
    try:
        parsed = json.loads(raw)
        if isinstance(parsed, list):
            return parsed
    except (json.JSONDecodeError, TypeError):
        pass
    return []
