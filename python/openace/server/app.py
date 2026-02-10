"""MCP Server implementation for OpenACE."""

from __future__ import annotations

import asyncio
import json
from typing import Any

from openace.engine import Engine
from openace.exceptions import OpenACEError


def create_server(engine: Engine):
    """Create an MCP server with OpenACE tools.

    Args:
        engine: An initialized OpenACE Engine instance.

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

    @server.list_tools()
    async def list_tools() -> list[Tool]:
        return [
            Tool(
                name="semantic_search",
                description="Search for code symbols by semantic query. "
                "Returns ranked results with relevance scores.",
                inputSchema={
                    "type": "object",
                    "properties": {
                        "query": {
                            "type": "string",
                            "description": "Search query text",
                        },
                        "limit": {
                            "type": "integer",
                            "description": "Maximum number of results (default: 10)",
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

    results = await asyncio.to_thread(engine.search, query, limit=limit)

    if not results:
        return [TextContent(type="text", text="No results found.")]

    lines = []
    for i, r in enumerate(results, 1):
        lines.append(
            f"{i}. {r.name} ({r.kind})\n"
            f"   File: {r.file_path}:{r.line_range[0]}-{r.line_range[1]}\n"
            f"   Qualified: {r.qualified_name}\n"
            f"   Score: {r.score:.4f} | Signals: {', '.join(r.match_signals)}"
        )
        if r.related_symbols:
            related_names = [rs.name for rs in r.related_symbols[:5]]
            lines.append(f"   Related: {', '.join(related_names)}")

    return [TextContent(type="text", text="\n\n".join(lines))]


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
