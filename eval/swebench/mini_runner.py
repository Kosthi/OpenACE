"""mini-SWE-agent based runner for SWE-bench evaluation with OpenACE context injection.

This module provides an agentic evaluation approach: instead of single-shot
patch generation, the LLM iteratively explores the codebase via bash commands.
For the OpenACE condition, pre-retrieved context is injected into the agent's
initial prompt, giving it a head start on locating relevant code.
"""

from __future__ import annotations

import json
import logging
import time
import traceback
from pathlib import Path
from typing import Optional

from eval.swebench.config import ExperimentCondition
from eval.swebench.context_retrieval import retrieve_context
from eval.swebench.repo_manager import clone_or_reuse

logger = logging.getLogger(__name__)


def run_mini_swebench(
    *,
    conditions: list[str],
    model_name: str,
    subset: str = "lite",
    split: str = "test",
    slice_spec: str = "",
    output_dir: str = "eval/output",
    repos_dir: str = "eval/repos",
    openace_embedding: Optional[str] = None,
    openace_reranker: Optional[str] = None,
    step_limit: int = 50,
    cost_limit: float = 3.0,
    api_base: Optional[str] = None,
    api_key: Optional[str] = None,
) -> None:
    """Run mini-SWE-agent on SWE-bench instances for given conditions.

    Args:
        conditions: List of condition IDs to run (e.g. ["baseline", "openace"]).
        model_name: LiteLLM model name (e.g. "anthropic/claude-sonnet-4-5-20250929").
        subset: SWE-bench subset ("lite", "verified", "full").
        split: Dataset split ("test", "dev").
        slice_spec: Instance slice (e.g. "0:20" for first 20).
        output_dir: Root output directory.
        repos_dir: Directory for caching cloned repos.
        openace_embedding: Embedding backend for OpenACE conditions (None = BM25 only).
        openace_reranker: Reranker backend for OpenACE conditions.
        step_limit: Max agent steps per instance.
        cost_limit: Max cost per instance in USD.
        api_base: Custom API base URL for OpenAI-compatible services.
        api_key: API key (overrides OPENAI_API_KEY env var).
    """
    from datasets import load_dataset as hf_load

    from minisweagent.config import builtin_config_dir, get_config_from_spec
    from minisweagent.utils.serialize import recursive_merge

    # Load SWE-bench dataset
    dataset_map = {
        "lite": "princeton-nlp/SWE-bench_Lite",
        "verified": "princeton-nlp/SWE-bench_Verified",
        "full": "princeton-nlp/SWE-Bench",
    }
    dataset_path = dataset_map.get(subset, subset)
    logger.info("Loading dataset %s split=%s ...", dataset_path, split)
    instances = list(hf_load(dataset_path, split=split))

    # Apply slice
    if slice_spec:
        parts = [int(x) if x else None for x in slice_spec.split(":")]
        instances = instances[slice(*parts)]
    logger.info("Running on %d instances", len(instances))

    # Load base mini-SWE-agent config
    base_config_path = builtin_config_dir / "benchmarks" / "swebench.yaml"
    base_config = get_config_from_spec(str(base_config_path))

    output_root = Path(output_dir)

    for condition_id in conditions:
        cond_output = output_root / condition_id
        cond_output.mkdir(parents=True, exist_ok=True)

        # Load checkpoint (already completed instances)
        preds_path = cond_output / "preds.json"
        existing = _load_preds(preds_path)
        remaining = [i for i in instances if i["instance_id"] not in existing]
        logger.info(
            "Condition %r: %d done, %d remaining",
            condition_id, len(existing), len(remaining),
        )

        for idx, instance in enumerate(remaining):
            iid = instance["instance_id"]
            logger.info(
                "[%s] %s (%d/%d)",
                condition_id, iid, idx + 1, len(remaining),
            )

            t0 = time.monotonic()
            try:
                _process_one(
                    instance=instance,
                    condition_id=condition_id,
                    model_name=model_name,
                    base_config=base_config,
                    cond_output=cond_output,
                    repos_dir=repos_dir,
                    openace_embedding=openace_embedding,
                    openace_reranker=openace_reranker,
                    step_limit=step_limit,
                    cost_limit=cost_limit,
                    api_base=api_base,
                    api_key=api_key,
                )
                elapsed = time.monotonic() - t0
                logger.info("[%s] %s done (%.1fs)", condition_id, iid, elapsed)
            except Exception:
                logger.error("[%s] %s FAILED", condition_id, iid, exc_info=True)


def _process_one(
    *,
    instance: dict,
    condition_id: str,
    model_name: str,
    base_config: dict,
    cond_output: Path,
    repos_dir: str,
    openace_embedding: Optional[str],
    openace_reranker: Optional[str],
    step_limit: int,
    cost_limit: float,
    api_base: Optional[str] = None,
    api_key: Optional[str] = None,
) -> None:
    """Process a single (condition, instance) pair."""
    import copy

    from minisweagent.agents.default import DefaultAgent
    from minisweagent.models import get_model
    from minisweagent.utils.serialize import recursive_merge

    iid = instance["instance_id"]
    task = instance["problem_statement"]

    # Build config with overrides
    # LitellmModelConfig only has model_name and model_kwargs fields;
    # api_base and api_key must go into model_kwargs so they get unpacked
    # as kwargs to litellm.completion().
    config = copy.deepcopy(base_config)
    model_overrides: dict = {
        "model_name": model_name,
        # Custom/unmapped models won't have cost info in litellm's registry;
        # ignore cost calculation errors instead of crashing.
        "cost_tracking": "ignore_errors",
    }
    extra_kwargs: dict = {}
    if api_base:
        extra_kwargs["api_base"] = api_base
    if api_key:
        extra_kwargs["api_key"] = api_key
    if extra_kwargs:
        model_overrides["model_kwargs"] = extra_kwargs
    config = recursive_merge(config, {
        "model": model_overrides,
        "agent": {"step_limit": step_limit, "cost_limit": cost_limit},
    })

    # Set up environment
    env = _get_sb_environment(config, instance)

    # Pre-retrieve OpenACE context if not baseline
    openace_context = ""
    if condition_id != "baseline":
        openace_context = _retrieve_openace_context(
            instance, repos_dir, openace_embedding, openace_reranker,
        )

    # Inject OpenACE template into instance_template
    if openace_context:
        config["agent"]["instance_template"] = _inject_context_section(
            config["agent"]["instance_template"],
        )

    # Create model and agent
    model = get_model(config=config.get("model", {}))

    traj_dir = cond_output / iid
    traj_dir.mkdir(parents=True, exist_ok=True)
    traj_path = traj_dir / f"{iid}.traj.json"

    agent = DefaultAgent(
        model,
        env,
        output_path=traj_path,
        **config.get("agent", {}),
    )

    # Run agent — pass openace_context as a Jinja2 template variable
    try:
        info = agent.run(task, openace_context=openace_context)
        submission = info.get("submission", "")
        exit_status = info.get("exit_status", "")
    except Exception as e:
        logger.error("Agent error for %s: %s", iid, e)
        submission = ""
        exit_status = type(e).__name__

    # Save prediction
    _update_preds(cond_output / "preds.json", iid, model_name, submission)


def _get_sb_environment(config: dict, instance: dict):
    """Create a SWE-bench Docker environment for the instance."""
    import copy

    from minisweagent.environments import get_environment

    env_config = copy.deepcopy(config.get("environment", {}))
    env_config.setdefault("environment_class", "docker")

    image_name = _get_docker_image(instance)
    env_class = env_config.get("environment_class", "docker")
    if env_class in ("docker", "swerex_modal"):
        env_config["image"] = image_name
    elif env_class in ("singularity", "contree"):
        env_config["image"] = "docker://" + image_name

    env = get_environment(env_config)

    # Execute startup command if configured
    from jinja2 import StrictUndefined, Template
    startup = config.get("run", {}).get("env_startup_command")
    if startup:
        rendered = Template(startup, undefined=StrictUndefined).render(**instance)
        out = env.execute({"command": rendered})
        if out["returncode"] != 0:
            raise RuntimeError(f"Startup command failed: {out}")

    return env


def _get_docker_image(instance: dict) -> str:
    """Get the SWE-bench Docker image name for an instance."""
    image = instance.get("image_name") or instance.get("docker_image")
    if image is None:
        iid = instance["instance_id"]
        safe_id = iid.replace("__", "_1776_")
        image = f"docker.io/swebench/sweb.eval.x86_64.{safe_id}:latest".lower()
    return image


def _retrieve_openace_context(
    instance: dict,
    repos_dir: str,
    embedding_backend: Optional[str],
    reranker_backend: Optional[str],
) -> str:
    """Clone the repo locally and retrieve context using OpenACE."""
    condition = ExperimentCondition(
        condition_id="openace",
        embedding_backend=embedding_backend,
        reranker_backend=reranker_backend,
    )

    try:
        repo_path = clone_or_reuse(
            instance["repo"],
            instance["base_commit"],
            repos_dir,
        )
        context = retrieve_context(
            str(repo_path),
            instance["problem_statement"],
            condition,
        )
        logger.info(
            "OpenACE context for %s: %d chars",
            instance["instance_id"], len(context),
        )
        return context
    except Exception:
        logger.warning(
            "OpenACE retrieval failed for %s, continuing without context",
            instance["instance_id"],
            exc_info=True,
        )
        return ""


_CONTEXT_INJECTION = """

{% if openace_context %}
<openace_context>
The following code context was retrieved using semantic code search and may be
relevant to the issue. Use this as a starting point for your investigation, but
verify by reading the actual files.

{{ openace_context }}
</openace_context>
{% endif %}
"""


def _inject_context_section(instance_template: str) -> str:
    """Insert the OpenACE context block before the <instructions> tag."""
    marker = "<instructions>"
    if marker in instance_template:
        return instance_template.replace(marker, _CONTEXT_INJECTION + marker, 1)
    # Fallback: append at the end
    return instance_template + _CONTEXT_INJECTION


# ---------------------------------------------------------------------------
# Predictions file (mini-SWE-agent compatible JSON format)
# ---------------------------------------------------------------------------

import threading

_PREDS_LOCK = threading.Lock()


def _load_preds(path: Path) -> dict:
    """Load existing predictions from preds.json."""
    if not path.exists():
        return {}
    try:
        return json.loads(path.read_text())
    except (json.JSONDecodeError, OSError):
        return {}


def _update_preds(path: Path, instance_id: str, model_name: str, patch: str) -> None:
    """Thread-safe update of preds.json."""
    with _PREDS_LOCK:
        data = _load_preds(path)
        data[instance_id] = {
            "instance_id": instance_id,
            "model_name_or_path": model_name,
            "model_patch": patch or "",
        }
        path.write_text(json.dumps(data, indent=2))
