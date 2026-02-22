"""OpenACE context retrieval for SWE-bench instances."""

from __future__ import annotations

import logging
import re
from pathlib import Path

from eval.swebench.config import ExperimentCondition

logger = logging.getLogger(__name__)


def retrieve_context(
    project_root: str,
    problem_statement: str,
    condition: ExperimentCondition,
) -> str:
    """Index a repository with OpenACE and retrieve relevant code context.

    Args:
        project_root: Absolute path to the checked-out repo.
        problem_statement: The issue/bug description text.
        condition: Experiment condition controlling embedding/reranker backends.

    Returns:
        Formatted context string ready for LLM prompt injection.
        Returns empty string for baseline condition.
    """
    if condition.is_baseline:
        return ""

    from openace.engine import Engine

    embedding_provider = None
    reranker = None

    if condition.embedding_backend is not None:
        from openace.embedding.factory import create_provider
        embedding_provider = create_provider(condition.embedding_backend)

    if condition.reranker_backend is not None:
        from openace.reranking.factory import create_reranker
        reranker = create_reranker(condition.reranker_backend)

    engine = Engine(
        project_root,
        embedding_provider=embedding_provider,
        reranker=reranker,
    )

    logger.info("Indexing %s ...", project_root)
    report = engine.index()
    logger.info(
        "Indexed %d files, %d symbols in %.1fs",
        report.files_indexed, report.total_symbols, report.duration_secs,
    )

    queries = generate_queries(problem_statement)

    all_results = []
    for q in queries:
        try:
            results = engine.search(
                q,
                limit=condition.search_limit,
                dedupe_by_file=condition.dedupe_by_file,
            )
            all_results.extend(results)
        except Exception:
            logger.warning("Search failed for query: %s", q, exc_info=True)

    unique_results = _dedupe_by_symbol_id(all_results)

    return format_context(unique_results, project_root)


def generate_queries(problem_statement: str) -> list[str]:
    """Generate search queries from a problem statement.

    Strategy:
    1. The full problem statement (truncated) as the primary semantic query.
    2. Extracted code references (file paths, class/function names) as
       secondary exact-match queries.
    """
    queries: list[str] = []
    seen: set[str] = set()

    def _add(q: str) -> None:
        q = q.strip()
        if q and q not in seen:
            seen.add(q)
            queries.append(q)

    # Primary query: first 500 chars of problem statement for semantic search
    _add(problem_statement[:500])

    # Extract file paths mentioned in the problem statement
    file_paths = re.findall(r'[\w/]+\.(?:py|js|ts|rs|go|java)\b', problem_statement)
    for fp in file_paths[:3]:
        _add(fp)

    # Extract potential class/function names (CamelCase or snake_case identifiers)
    # Require at least one internal uppercase letter to filter common English words
    camel = re.findall(r'\b([A-Z][a-z]+(?:[A-Z][a-zA-Z0-9]*)+)\b', problem_statement)
    for ident in camel[:5]:
        _add(ident)

    # Dotted references like module.ClassName.method
    dotted = re.findall(r'\b([\w]+(?:\.[\w]+){1,})\b', problem_statement)
    for ref in dotted[:5]:
        _add(ref)

    return queries[:8]  # cap at 8 queries


def format_context(results: list, project_root: str) -> str:
    """Format search results into a readable context block for LLM consumption.

    Args:
        results: Deduplicated and sorted SearchResult list.
        project_root: Project root for making paths relative.

    Returns:
        Formatted markdown-style context string.
    """
    if not results:
        return ""

    root = Path(project_root)
    sections: list[str] = []

    for r in results[:20]:  # cap at 20 results to stay within context window
        # Make path relative to project root
        rel_path = r.file_path

        signals = ", ".join(r.match_signals) if r.match_signals else "unknown"
        header = f"### File: {rel_path} (score: {r.score:.2f}, signals: {signals})"

        body_parts = [header]
        if r.snippet:
            line_start, line_end = r.line_range
            body_parts.append(f"```\n# Lines {line_start}-{line_end}\n{r.snippet}\n```")
        else:
            line_start, line_end = r.line_range
            body_parts.append(
                f"Symbol: `{r.qualified_name}` ({r.kind}, lines {line_start}-{line_end})"
            )

        sections.append("\n".join(body_parts))

    return "## Relevant Code Context\n\n" + "\n\n".join(sections)


def _dedupe_by_symbol_id(results: list) -> list:
    """Remove duplicate results by symbol_id, keeping the highest-scoring."""
    seen: dict[str, object] = {}
    for r in results:
        existing = seen.get(r.symbol_id)
        if existing is None or r.score > existing.score:
            seen[r.symbol_id] = r

    return sorted(seen.values(), key=lambda r: r.score, reverse=True)
