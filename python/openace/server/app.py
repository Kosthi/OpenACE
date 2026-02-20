"""MCP Server implementation for OpenACE."""

from __future__ import annotations

import asyncio
import sys
from typing import Any

import structlog

from openace.engine import Engine, _is_test_file, _LOW_VALUE_KINDS
from openace.exceptions import OpenACEError
from openace.types import SearchResult

logger = structlog.get_logger(__name__)


_GRAPH_ONLY = {"graph"}


def _effective_score(r: SearchResult) -> float:
    """Return the best available score for a result, with graph-only penalty."""
    base = r.rerank_score if r.rerank_score is not None else r.score
    if set(r.match_signals) == _GRAPH_ONLY:
        base *= 0.7
    return base


def _aggregate_by_file(results: list[SearchResult]) -> list[dict]:
    """Group search results by file path into file-level groups.

    Returns list of dicts sorted by (tier, -effective_score):
        file_path, symbols, effective_score, tier, all_signals
    """
    groups: dict[str, list[SearchResult]] = {}
    for r in results:
        groups.setdefault(r.file_path, []).append(r)

    file_groups = []
    for file_path, symbols in groups.items():
        symbols.sort(key=_effective_score, reverse=True)
        best_score = _effective_score(symbols[0])
        all_signals: set[str] = set()
        for s in symbols:
            all_signals.update(s.match_signals)

        all_low_value = all(s.kind in _LOW_VALUE_KINDS for s in symbols)
        if all_low_value:
            tier = 2
        elif _is_test_file(file_path):
            tier = 1
        else:
            tier = 0

        file_groups.append({
            "file_path": file_path,
            "symbols": symbols,
            "effective_score": best_score,
            "tier": tier,
            "all_signals": all_signals,
        })

    file_groups.sort(key=lambda g: (g["tier"], -g["effective_score"]))
    return file_groups


def _apply_file_score_gap(groups: list[dict], min_results: int = 3) -> list[dict]:
    """Apply 0.6-ratio score-gap cutoff on file groups."""
    if len(groups) <= min_results:
        return groups
    cut_idx = len(groups)
    for idx in range(min_results, len(groups)):
        prev = groups[idx - 1]["effective_score"]
        cur = groups[idx]["effective_score"]
        if prev > 0 and cur / prev < 0.6:
            cut_idx = idx
            break
    return groups[:max(cut_idx, min_results)]


_MAX_SNIPPET_SYMBOLS = 3
_MAX_SNIPPET_LINES = 15
_NO_SNIPPET_KINDS = {"module", "constant", "variable"}


def _format_file_group(
    rank: int,
    group: dict,
    file_outlines: dict[str, list],
) -> str:
    """Format a single file group as text for MCP output.

    Shows full snippets for the top _MAX_SNIPPET_SYMBOLS actionable symbols
    (functions, classes, methods) per file.  Module/constant/variable symbols
    are always listed compactly.  Remaining matched symbols appear in a
    compact "Also matched" line.
    """
    file_path = group["file_path"]
    symbols = group["symbols"]
    best_score = group["effective_score"]
    all_signals = group["all_signals"]

    header = (
        f"{rank}. {file_path}\n"
        f"   Score: {best_score:.4f} | Signals: {', '.join(sorted(all_signals))} "
        f"| Matched: {len(symbols)} symbol{'s' if len(symbols) != 1 else ''}"
    )

    parts = [header]

    matched_names = {s.name for s in symbols}

    # Split into snippet-worthy symbols vs compact-only
    snippet_symbols = []
    compact_symbols = []
    for r in symbols:
        if r.kind in _NO_SNIPPET_KINDS or len(snippet_symbols) >= _MAX_SNIPPET_SYMBOLS:
            compact_symbols.append(r)
        else:
            snippet_symbols.append(r)

    for si, r in enumerate(snippet_symbols, 1):
        sig_str = ", ".join(r.match_signals) if r.match_signals else "graph"
        score_val = _effective_score(r)
        sym_header = (
            f"   [{si}] {r.name} ({r.kind}) "
            f"L{r.line_range[0]}-{r.line_range[1]} "
            f"[{sig_str}] score={score_val:.4f}"
        )
        parts.append(sym_header)

        if r.snippet:
            snippet_lines = r.snippet.splitlines()[:_MAX_SNIPPET_LINES]
            line_start = r.line_range[0]
            formatted = "\n".join(
                f"       {line_start + j:>4}\t{line}"
                for j, line in enumerate(snippet_lines)
            )
            if len(r.snippet.splitlines()) > _MAX_SNIPPET_LINES:
                formatted += "\n       ..."
            parts.append(formatted)

    # Remaining matched symbols listed compactly
    if compact_symbols:
        compact = [
            f"{r.name} ({r.kind}, L{r.line_range[0]}-{r.line_range[1]})"
            for r in compact_symbols
        ]
        parts.append(f"   Also matched: {', '.join(compact)}")

    # "Also in this file" sidebar: outline symbols not in matched set
    outline = file_outlines.get(file_path, [])
    siblings = [
        s for s in outline
        if s.name not in matched_names and s.kind not in ("constant", "variable")
    ]
    if siblings:
        items = [
            f"{s.name} ({s.kind}, L{s.line_start}-{s.line_end})"
            for s in siblings[:8]
        ]
        suffix = f" +{len(siblings) - 8} more" if len(siblings) > 8 else ""
        parts.append(f"   Also in this file: {', '.join(items)}{suffix}")

    return "\n".join(parts)


def create_server(engine: Engine, *, auto_index: bool = False):
    """Create an MCP server with OpenACE tools.

    Args:
        engine: An initialized OpenACE Engine instance.
        auto_index: If True, run indexing as a background task after
            the MCP handshake completes.  Tool calls will wait until
            indexing finishes before executing.

    Returns:
        An MCP Server instance ready to be run.
    """
    try:
        from mcp.server import Server
        from mcp.server.stdio import stdio_server
        from mcp.types import TextContent, Tool
    except ImportError:
        raise ImportError(
            "MCP server requires the mcp package. "
            "Install with: pip install openace[mcp]"
        )

    server = Server("openace")

    # Event signalling that the index is ready for queries.
    _index_ready = asyncio.Event()
    if not auto_index:
        # Already indexed before server start — mark ready immediately.
        _index_ready.set()

    async def _background_index():
        """Run indexing in a background thread, then signal readiness.

        Skips re-indexing when a valid index already exists.
        Re-indexes if chunk_enabled but no chunks are present yet.
        """
        try:
            existing_symbols = await asyncio.to_thread(engine._core.count_symbols)
            existing_chunks = await asyncio.to_thread(engine._core.count_chunks)
            needs_index = existing_symbols == 0
            needs_chunk_rebuild = (
                engine._chunk_enabled and existing_symbols > 0 and existing_chunks == 0
            )
            if needs_index or needs_chunk_rebuild:
                reason = "no index" if needs_index else "building chunks"
                logger.info("indexing project", reason=reason)
                report = await asyncio.to_thread(engine.index)
                logger.info(
                    "indexing complete",
                    files=report.files_indexed,
                    symbols=report.total_symbols,
                    chunks=report.total_chunks,
                    duration_secs=f"{report.duration_secs:.2f}",
                )
            else:
                logger.info(
                    "index exists, skipping re-index",
                    symbols=existing_symbols,
                    chunks=existing_chunks,
                )
        except Exception as exc:
            logger.error("background indexing failed", error=str(exc))
        finally:
            _index_ready.set()

    original_run = server.run

    async def _run_with_background_index(read_stream, write_stream, init_options):
        if auto_index:
            asyncio.create_task(_background_index())
        await original_run(read_stream, write_stream, init_options)

    server.run = _run_with_background_index

    @server.list_tools()
    async def list_tools() -> list[Tool]:
        return [
            Tool(
                name="semantic_search",
                description="Search for code symbols (functions, classes, methods) by semantic query. "
                "Returns ranked results with code snippets and relevance scores. "
                "Query format: natural language description of the code behavior you're looking for, "
                "followed by keywords for precision. Keep the user's original terms (any language) — "
                "many codebases have comments in non-English languages that are valuable search signals. "
                "Example: user asks '识别框的定位方法' → query "
                "'识别框的定位方法 — functions that detect, locate, and extract bounding boxes "
                "in document layout analysis and OCR detection pipeline. "
                "Keywords: detection box localization bounding_box parse predict filter merge sort bbox postprocess'.",
                inputSchema={
                    "type": "object",
                    "properties": {
                        "query": {
                            "type": "string",
                            "description": "Natural language description of the code you are looking for, "
                            "optionally followed by keywords. Include the user's original terms "
                            "(any language) for matching code comments. "
                            "Recommended format: original terms + English description + Keywords: ...",
                        },
                        "limit": {
                            "type": "integer",
                            "description": "Maximum number of files to return (default: 10)",
                            "default": 10,
                        },
                    },
                    "required": ["query"],
                },
            ),
            Tool(
                name="find_symbol",
                description="Find code symbols by exact name match. "
                "Searches both short names and qualified names.",
                inputSchema={
                    "type": "object",
                    "properties": {
                        "name": {
                            "type": "string",
                            "description": "Symbol name or qualified name",
                        },
                    },
                    "required": ["name"],
                },
            ),
            Tool(
                name="get_file_outline",
                description="Get all code symbols defined in a file. "
                "Returns functions, classes, methods, etc.",
                inputSchema={
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Relative file path within the project",
                        },
                    },
                    "required": ["path"],
                },
            ),
        ]

    @server.call_tool()
    async def call_tool(name: str, arguments: dict[str, Any]) -> list[TextContent]:
        # Wait for background indexing to finish before handling any tool call.
        await _index_ready.wait()
        try:
            if name == "semantic_search":
                return await _handle_search(engine, arguments)
            elif name == "find_symbol":
                return await _handle_find_symbol(engine, arguments)
            elif name == "get_file_outline":
                return await _handle_file_outline(engine, arguments)
            else:
                return [TextContent(type="text", text=f"Unknown tool: {name}")]
        except OpenACEError as e:
            return [TextContent(type="text", text=f"Error: {e}")]
        except Exception as e:
            return [TextContent(type="text", text=f"Internal error: {e}")]

    return server


async def _handle_search(engine: Engine, args: dict) -> list:
    from mcp.types import TextContent

    query = args["query"]
    limit = args.get("limit", 10)

    # Request expanded pool with all symbols (no file dedup) so we can
    # aggregate multiple symbols per file in the presentation layer.
    pool_size = min(limit * 5, 200)
    results = await asyncio.to_thread(
        engine.search, query, limit=pool_size, dedupe_by_file=False,
    )

    if not results:
        return [TextContent(type="text", text="No results found.")]

    # Aggregate into file-level groups
    file_groups = _aggregate_by_file(results)
    file_groups = _apply_file_score_gap(file_groups)
    file_groups = file_groups[:limit]

    # Pre-fetch file outlines for context expansion.
    file_outlines: dict[str, list] = {}
    for group in file_groups:
        fp = group["file_path"]
        try:
            syms = await asyncio.to_thread(engine.get_file_outline, fp)
            file_outlines[fp] = syms
        except Exception:
            pass

    # Format each file group
    formatted = []
    for rank, group in enumerate(file_groups, 1):
        formatted.append(_format_file_group(rank, group, file_outlines))

    return [TextContent(type="text", text="\n\n".join(formatted))]


async def _handle_find_symbol(engine: Engine, args: dict) -> list:
    from mcp.types import TextContent

    name = args["name"]

    symbols = await asyncio.to_thread(engine.find_symbol, name)

    if not symbols:
        return [TextContent(type="text", text=f"No symbols found matching '{name}'.")]

    lines = []
    for sym in symbols:
        lines.append(
            f"- {sym.name} ({sym.kind}, {sym.language})\n"
            f"  File: {sym.file_path}:{sym.line_start}-{sym.line_end}\n"
            f"  Qualified: {sym.qualified_name}"
        )
        if sym.signature:
            lines.append(f"  Signature: {sym.signature}")

    return [TextContent(type="text", text="\n\n".join(lines))]


async def _handle_file_outline(engine: Engine, args: dict) -> list:
    from mcp.types import TextContent

    path = args["path"]

    symbols = await asyncio.to_thread(engine.get_file_outline, path)

    if not symbols:
        return [TextContent(type="text", text=f"No symbols found in '{path}'.")]

    lines = [f"File: {path}\n"]
    for sym in symbols:
        indent = "  " if sym.kind in ("method",) else ""
        sig = f" - {sym.signature}" if sym.signature else ""
        lines.append(
            f"{indent}{sym.kind}: {sym.name} "
            f"(L{sym.line_start}-{sym.line_end}){sig}"
        )

    return [TextContent(type="text", text="\n".join(lines))]
