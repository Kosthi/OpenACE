"""Experiment configuration dataclasses."""

from __future__ import annotations

from dataclasses import dataclass, field
from pathlib import Path
from typing import Optional


@dataclass(frozen=True)
class ExperimentCondition:
    """A single experiment condition defining OpenACE retrieval settings."""

    condition_id: str
    embedding_backend: Optional[str] = None
    reranker_backend: Optional[str] = None
    search_limit: int = 20

    @property
    def is_baseline(self) -> bool:
        return self.condition_id == "baseline"


@dataclass(frozen=True)
class LLMConfig:
    """LLM provider configuration."""

    provider: str  # "openai" | "anthropic"
    model: str
    api_key_env: str  # environment variable name holding the API key
    base_url: Optional[str] = None  # custom API endpoint (e.g. vLLM, Azure, LiteLLM)
    temperature: float = 0.0
    max_tokens: int = 4096


@dataclass(frozen=True)
class EvalConfig:
    """Top-level evaluation configuration."""

    conditions: list[ExperimentCondition]
    llm: LLMConfig
    dataset: str = "princeton-nlp/SWE-bench_Lite"
    repos_dir: str = "eval/repos"
    output_dir: str = "eval/output"
    max_instances: Optional[int] = None
    instance_ids: Optional[list[str]] = None


# ---------------------------------------------------------------------------
# Predefined experiment conditions
# ---------------------------------------------------------------------------

BASELINE = ExperimentCondition(condition_id="baseline")

BM25_ONLY = ExperimentCondition(
    condition_id="bm25-only",
    embedding_backend=None,
    reranker_backend=None,
)

BM25_VECTOR = ExperimentCondition(
    condition_id="bm25+vector",
    embedding_backend="local",
    reranker_backend=None,
)

FULL_PIPELINE = ExperimentCondition(
    condition_id="full-pipeline",
    embedding_backend="local",
    reranker_backend="rule_based",
)

FULL_SILICONFLOW = ExperimentCondition(
    condition_id="full-siliconflow",
    embedding_backend="siliconflow",
    reranker_backend="siliconflow",
)

ALL_CONDITIONS = [BASELINE, BM25_ONLY, BM25_VECTOR, FULL_PIPELINE, FULL_SILICONFLOW]


def load_config_from_yaml(path: str | Path) -> EvalConfig:
    """Load an EvalConfig from a YAML file.

    Expected YAML structure::

        dataset: princeton-nlp/SWE-bench_Lite
        repos_dir: eval/repos
        output_dir: eval/output
        max_instances: 10           # optional
        instance_ids: [...]         # optional

        llm:
          provider: openai
          model: gpt-4o
          api_key_env: OPENAI_API_KEY
          base_url: https://custom-api.example.com/v1  # optional
          temperature: 0.0
          max_tokens: 4096

        conditions:
          - condition_id: baseline
          - condition_id: bm25-only
          - condition_id: full-pipeline
            embedding_backend: local
            reranker_backend: rule_based
            search_limit: 20
    """
    import yaml

    raw = yaml.safe_load(Path(path).read_text())

    llm_raw = raw["llm"]
    llm = LLMConfig(
        provider=llm_raw["provider"],
        model=llm_raw["model"],
        api_key_env=llm_raw["api_key_env"],
        base_url=llm_raw.get("base_url"),
        temperature=llm_raw.get("temperature", 0.0),
        max_tokens=llm_raw.get("max_tokens", 4096),
    )

    conditions = []
    for c in raw["conditions"]:
        conditions.append(
            ExperimentCondition(
                condition_id=c["condition_id"],
                embedding_backend=c.get("embedding_backend"),
                reranker_backend=c.get("reranker_backend"),
                search_limit=c.get("search_limit", 20),
            )
        )

    return EvalConfig(
        dataset=raw.get("dataset", "princeton-nlp/SWE-bench_Lite"),
        conditions=conditions,
        llm=llm,
        repos_dir=raw.get("repos_dir", "eval/repos"),
        output_dir=raw.get("output_dir", "eval/output"),
        max_instances=raw.get("max_instances"),
        instance_ids=raw.get("instance_ids"),
    )
