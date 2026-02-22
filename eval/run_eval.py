#!/usr/bin/env python3
"""CLI entry point for the SWE-bench evaluation framework."""

from __future__ import annotations

import argparse
import logging
import sys
from pathlib import Path

# Ensure the project root is on sys.path so `eval.swebench.*` imports work
# when running as `python eval/run_eval.py`.
_PROJECT_ROOT = str(Path(__file__).resolve().parent.parent)
if _PROJECT_ROOT not in sys.path:
    sys.path.insert(0, _PROJECT_ROOT)


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Run SWE-bench Lite evaluation with OpenACE context enhancement",
    )
    subparsers = parser.add_subparsers(dest="command", required=True)

    # --- run ---
    run_parser = subparsers.add_parser("run", help="Run evaluation")
    run_parser.add_argument(
        "--config", required=True,
        help="Path to YAML config file",
    )
    run_parser.add_argument(
        "--max-instances", type=int, default=None,
        help="Override max_instances from config",
    )
    run_parser.add_argument(
        "--conditions", type=str, default=None,
        help="Comma-separated condition IDs to run (overrides config)",
    )
    run_parser.add_argument(
        "--verbose", "-v", action="store_true",
        help="Enable debug logging",
    )

    # --- mini (agentic via mini-SWE-agent) ---
    mini_parser = subparsers.add_parser(
        "mini", help="Run agentic evaluation via mini-SWE-agent",
    )
    mini_parser.add_argument(
        "--model", "-m", required=True,
        help="LiteLLM model name (e.g. anthropic/claude-sonnet-4-5-20250929)",
    )
    mini_parser.add_argument(
        "--conditions", default="baseline,openace",
        help="Comma-separated condition IDs (default: baseline,openace)",
    )
    mini_parser.add_argument(
        "--subset", default="lite",
        help="SWE-bench subset: lite, verified, full (default: lite)",
    )
    mini_parser.add_argument(
        "--split", default="test",
        help="Dataset split (default: test)",
    )
    mini_parser.add_argument(
        "--slice", dest="slice_spec", default="",
        help="Instance slice, e.g. '0:20' for first 20",
    )
    mini_parser.add_argument(
        "--output-dir", "-o", default="eval/output",
        help="Output directory (default: eval/output)",
    )
    mini_parser.add_argument(
        "--repos-dir", default="eval/repos",
        help="Repos cache directory (default: eval/repos)",
    )
    mini_parser.add_argument(
        "--embedding", default=None,
        help="OpenACE embedding backend for openace condition (default: None = BM25 only)",
    )
    mini_parser.add_argument(
        "--reranker", default=None,
        help="OpenACE reranker backend for openace condition",
    )
    mini_parser.add_argument(
        "--step-limit", type=int, default=50,
        help="Max agent steps per instance (default: 50)",
    )
    mini_parser.add_argument(
        "--cost-limit", type=float, default=3.0,
        help="Max cost per instance in USD (default: 3.0)",
    )
    mini_parser.add_argument(
        "--api-base", default=None,
        help="Custom API base URL for OpenAI-compatible services",
    )
    mini_parser.add_argument(
        "--api-key", default=None,
        help="API key (overrides OPENAI_API_KEY env var)",
    )
    mini_parser.add_argument(
        "--verbose", "-v", action="store_true",
        help="Enable debug logging",
    )

    # --- retrieval (offline retrieval quality eval) ---
    ret_parser = subparsers.add_parser(
        "retrieval", help="Evaluate retrieval quality against gold patches",
    )
    ret_parser.add_argument(
        "--conditions", default="bm25-only",
        help="Comma-separated condition IDs (default: bm25-only)",
    )
    ret_parser.add_argument(
        "--subset", default="lite",
        help="SWE-bench subset: lite, verified, full (default: lite)",
    )
    ret_parser.add_argument(
        "--split", default="dev",
        help="Dataset split (default: dev)",
    )
    ret_parser.add_argument(
        "--slice", dest="slice_spec", default="",
        help="Instance slice, e.g. '0:10' for first 10",
    )
    ret_parser.add_argument(
        "--output-dir", "-o", default="eval/output_dev_retrieval",
        help="Output directory (default: eval/output_dev_retrieval)",
    )
    ret_parser.add_argument(
        "--repos-dir", default="eval/repos",
        help="Repos cache directory (default: eval/repos)",
    )
    ret_parser.add_argument(
        "--embedding", default=None,
        help="Override embedding backend for non-baseline conditions",
    )
    ret_parser.add_argument(
        "--embedding-base-url", default=None,
        help="Override embedding API base URL",
    )
    ret_parser.add_argument(
        "--embedding-api-key", default=None,
        help="Override embedding API key",
    )
    ret_parser.add_argument(
        "--reranker", default=None,
        help="Override reranker backend for non-baseline conditions",
    )
    ret_parser.add_argument(
        "--reranker-base-url", default=None,
        help="Override reranker API base URL",
    )
    ret_parser.add_argument(
        "--reranker-api-key", default=None,
        help="Override reranker API key",
    )
    ret_parser.add_argument(
        "--limit", type=int, default=20,
        help="Search result limit per query (default: 20)",
    )
    ret_parser.add_argument(
        "--query-mode", default="multi",
        choices=["multi", "single"],
        help="Query mode: 'multi' generates multiple sub-queries, "
             "'single' uses problem_statement[:500] as one query (default: multi)",
    )
    ret_parser.add_argument(
        "--verbose", "-v", action="store_true",
        help="Enable debug logging",
    )

    # --- analyze ---
    analyze_parser = subparsers.add_parser("analyze", help="Analyze results")
    analyze_parser.add_argument(
        "--output-dir", required=True,
        help="Path to evaluation output directory",
    )
    analyze_parser.add_argument(
        "--conditions", type=str, default=None,
        help="Comma-separated condition IDs to compare",
    )

    args = parser.parse_args()

    logging.basicConfig(
        level=logging.DEBUG if getattr(args, "verbose", False) else logging.INFO,
        format="%(asctime)s %(levelname)-8s %(name)s — %(message)s",
    )

    if args.command == "run":
        _cmd_run(args)
    elif args.command == "mini":
        _cmd_mini(args)
    elif args.command == "retrieval":
        _cmd_retrieval(args)
    elif args.command == "analyze":
        _cmd_analyze(args)


def _cmd_run(args) -> None:
    from eval.swebench.config import load_config_from_yaml
    from eval.swebench.runner import run_evaluation

    config = load_config_from_yaml(args.config)

    # Apply CLI overrides
    if args.max_instances is not None:
        # Reconstruct with override (frozen dataclass — rebuild)
        config = type(config)(
            dataset=config.dataset,
            conditions=config.conditions,
            llm=config.llm,
            repos_dir=config.repos_dir,
            output_dir=config.output_dir,
            max_instances=args.max_instances,
            instance_ids=config.instance_ids,
        )

    if args.conditions is not None:
        selected = set(args.conditions.split(","))
        filtered = [c for c in config.conditions if c.condition_id in selected]
        if not filtered:
            print(f"Error: no matching conditions found for: {args.conditions}")
            sys.exit(1)
        config = type(config)(
            dataset=config.dataset,
            conditions=filtered,
            llm=config.llm,
            repos_dir=config.repos_dir,
            output_dir=config.output_dir,
            max_instances=config.max_instances,
            instance_ids=config.instance_ids,
        )

    run_evaluation(config)


def _cmd_mini(args) -> None:
    from eval.swebench.mini_runner import run_mini_swebench

    run_mini_swebench(
        conditions=args.conditions.split(","),
        model_name=args.model,
        subset=args.subset,
        split=args.split,
        slice_spec=args.slice_spec,
        output_dir=args.output_dir,
        repos_dir=args.repos_dir,
        openace_embedding=args.embedding,
        openace_reranker=args.reranker,
        step_limit=args.step_limit,
        cost_limit=args.cost_limit,
        api_base=args.api_base,
        api_key=args.api_key,
    )


def _cmd_retrieval(args) -> None:
    from eval.swebench.retrieval_eval import run_retrieval_eval

    run_retrieval_eval(
        conditions=args.conditions.split(","),
        subset=args.subset,
        split=args.split,
        slice_spec=args.slice_spec,
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
    )


def _cmd_analyze(args) -> None:
    from eval.swebench.analyze import generate_report

    condition_ids = None
    if args.conditions:
        condition_ids = args.conditions.split(",")

    report = generate_report(args.output_dir, condition_ids)
    print(report)


if __name__ == "__main__":
    main()
