"""OpenACE command-line interface."""

from __future__ import annotations

import sys
from typing import Optional

import click

EMBEDDING_CHOICES = ["local", "openai", "siliconflow", "voyage", "voyage-code", "none"]
RERANKER_CHOICES = ["auto", "siliconflow", "cohere", "openai", "cross_encoder", "rule_based", "none"]
EXPANSION_CHOICES = ["siliconflow", "openai", "none"]

# Mapping: embedding backend -> default reranker backend
_AUTO_RERANKER = {
    "siliconflow": "siliconflow",
    "openai": "rule_based",
    "local": "rule_based",
    "voyage": "rule_based",
    "voyage-code": "rule_based",
    "none": "none",
}


def _provider_options(f):
    """Add shared provider configuration options to a Click command."""
    options = [
        click.option("--embedding-base-url", default=None,
                     envvar="OPENACE_EMBEDDING_BASE_URL",
                     help="Custom base URL for embedding API."),
        click.option("--embedding-api-key", default=None,
                     envvar="OPENACE_EMBEDDING_API_KEY",
                     help="Custom API key for embedding provider."),
        click.option("--embedding-dim", default=None, type=int,
                     envvar="OPENACE_EMBEDDING_DIM",
                     help="Embedding vector dimension override."),
        click.option("--reranker-base-url", default=None,
                     envvar="OPENACE_RERANKER_BASE_URL",
                     help="Custom base URL for reranker API."),
        click.option("--reranker-api-key", default=None,
                     envvar="OPENACE_RERANKER_API_KEY",
                     help="Custom API key for reranker provider."),
    ]
    for option in reversed(options):
        f = option(f)
    return f


def _build_engine_kwargs(
    embedding: str,
    reranker: str,
    expansion: str = "none",
    embedding_base_url: Optional[str] = None,
    embedding_api_key: Optional[str] = None,
    embedding_dim: Optional[int] = None,
    reranker_base_url: Optional[str] = None,
    reranker_api_key: Optional[str] = None,
):
    """Build Engine constructor kwargs from CLI options."""
    provider = None
    if embedding != "none":
        from openace.embedding.factory import create_provider
        embed_kwargs = {}
        if embedding_api_key:
            embed_kwargs["api_key"] = embedding_api_key
        if embedding_base_url:
            embed_kwargs["base_url"] = embedding_base_url
        if embedding_dim:
            embed_kwargs["dim"] = embedding_dim
        provider = create_provider(embedding, **embed_kwargs)

    # Resolve reranker
    reranker_backend = reranker if reranker != "auto" else _AUTO_RERANKER.get(embedding, "none")
    reranker_obj = None
    if reranker_backend != "none":
        from openace.reranking.factory import create_reranker
        rerank_kwargs = {}
        if reranker_api_key:
            rerank_kwargs["api_key"] = reranker_api_key
        if reranker_base_url:
            rerank_kwargs["base_url"] = reranker_base_url
        reranker_obj = create_reranker(reranker_backend, **rerank_kwargs)

    # Resolve query expander
    expander_obj = None
    if expansion != "none":
        from openace.query_expansion import create_query_expander
        expander_obj = create_query_expander(expansion)

    kwargs = {
        "embedding_provider": provider,
        "reranker": reranker_obj,
        "query_expander": expander_obj,
    }
    if provider is not None:
        kwargs["embedding_dim"] = provider.dimension

    return kwargs


@click.group()
@click.version_option(package_name="openace")
def main():
    """OpenACE - AI-native Contextual Code Engine."""
    pass


@main.command()
@click.argument("path", default=".", type=click.Path(exists=True))
@click.option("--embedding", type=click.Choice(EMBEDDING_CHOICES), default="none",
              help="Embedding provider to use.")
@click.option("--reranker", type=click.Choice(RERANKER_CHOICES), default="auto",
              help="Reranker backend (default: auto, matches embedding).")
@click.option("--chunk/--no-chunk", default=True,
              help="Enable AST chunk-level indexing (default: on).")
@_provider_options
def index(path: str, embedding: str, reranker: str, chunk: bool,
          embedding_base_url, embedding_api_key, embedding_dim,
          reranker_base_url, reranker_api_key):
    """Index a project directory."""
    from pathlib import Path
    from openace.engine import Engine

    project_path = str(Path(path).resolve())
    click.echo(f"Indexing {project_path}...")

    engine_kwargs = _build_engine_kwargs(
        embedding, reranker,
        embedding_base_url=embedding_base_url,
        embedding_api_key=embedding_api_key,
        embedding_dim=embedding_dim,
        reranker_base_url=reranker_base_url,
        reranker_api_key=reranker_api_key,
    )
    engine = Engine(project_path, chunk_enabled=chunk, **engine_kwargs)
    report = engine.index()

    click.echo(f"\nIndexing complete:")
    click.echo(f"  Files scanned:  {report.total_files_scanned}")
    click.echo(f"  Files indexed:  {report.files_indexed}")
    click.echo(f"  Files skipped:  {report.files_skipped}")
    click.echo(f"  Files failed:   {report.files_failed}")
    click.echo(f"  Symbols:        {report.total_symbols}")
    click.echo(f"  Relations:      {report.total_relations}")
    if report.total_chunks > 0:
        click.echo(f"  Chunks:         {report.total_chunks}")
    click.echo(f"  Duration:       {report.duration_secs:.2f}s")


@main.command()
@click.argument("query")
@click.option("--path", "-p", default=".", type=click.Path(exists=True),
              help="Project path.")
@click.option("--embedding", type=click.Choice(EMBEDDING_CHOICES), default="none",
              help="Embedding provider for vector search.")
@click.option("--reranker", type=click.Choice(RERANKER_CHOICES), default="auto",
              help="Reranker backend (default: auto, matches embedding).")
@click.option("--expansion", type=click.Choice(EXPANSION_CHOICES), default="none",
              help="Query expansion backend for improved recall.")
@click.option("--chunk/--no-chunk", default=True,
              help="Enable AST chunk-level search (default: on).")
@click.option("--limit", "-n", default=10, help="Max results.")
@click.option("--language", "-l", default=None, help="Language filter.")
@click.option("--file-path", "-f", default=None, help="File path prefix filter.")
@_provider_options
def search(query: str, path: str, embedding: str, reranker: str, expansion: str,
           chunk: bool, limit: int, language: str, file_path: str,
           embedding_base_url, embedding_api_key, embedding_dim,
           reranker_base_url, reranker_api_key):
    """Search for symbols in an indexed project."""
    from pathlib import Path
    from openace.engine import Engine

    project_path = str(Path(path).resolve())
    engine_kwargs = _build_engine_kwargs(
        embedding, reranker, expansion,
        embedding_base_url=embedding_base_url,
        embedding_api_key=embedding_api_key,
        embedding_dim=embedding_dim,
        reranker_base_url=reranker_base_url,
        reranker_api_key=reranker_api_key,
    )
    engine = Engine(project_path, chunk_enabled=chunk, **engine_kwargs)
    results = engine.search(query, limit=limit, language=language, file_path=file_path)

    if not results:
        click.echo("No results found.")
        return

    for i, r in enumerate(results, 1):
        score_str = f"score={r.score:.4f}"
        if r.rerank_score is not None:
            score_str += f" rerank={r.rerank_score:.4f}"
        click.echo(
            f"{i}. {r.name} ({r.kind}) "
            f"[{', '.join(r.match_signals)}] "
            f"{score_str}"
        )
        click.echo(f"   {r.file_path}:{r.line_range[0]}-{r.line_range[1]}")
        click.echo(f"   {r.qualified_name}")
        if r.snippet:
            lines = r.snippet.splitlines()[:30]
            line_start = r.line_range[0]
            for j, line in enumerate(lines):
                click.echo(f"     {line_start + j:>4}\t{line}")
            if len(r.snippet.splitlines()) > 30:
                click.echo("     ...")
        if r.related_symbols:
            related = ", ".join(rs.name for rs in r.related_symbols[:3])
            click.echo(f"   Related: {related}")
        click.echo()


@main.command()
@click.argument("path", default=".", type=click.Path(exists=True))
@click.option("--embedding", type=click.Choice(EMBEDDING_CHOICES), default="none",
              envvar="OPENACE_EMBEDDING", help="Embedding provider.")
@click.option("--reranker", type=click.Choice(RERANKER_CHOICES), default="auto",
              envvar="OPENACE_RERANKER", help="Reranker backend (default: auto, matches embedding).")
@click.option("--expansion", type=click.Choice(EXPANSION_CHOICES), default="none",
              envvar="OPENACE_EXPANSION", help="Query expansion backend.")
@click.option("--chunk/--no-chunk", default=True,
              help="Enable AST chunk-level indexing and search (default: on).")
@_provider_options
def serve(path: str, embedding: str, reranker: str, expansion: str, chunk: bool,
          embedding_base_url, embedding_api_key, embedding_dim,
          reranker_base_url, reranker_api_key):
    """Start MCP server on stdio."""
    import asyncio
    from pathlib import Path
    from openace.engine import Engine
    from openace.server.app import create_server

    try:
        import mcp.server.stdio  # noqa: F401
    except ImportError:
        raise click.ClickException(
            "MCP server requires the mcp package. Install with: pip install openace[mcp]"
        )

    project_path = str(Path(path).resolve())
    click.echo(f"Starting OpenACE MCP server for {project_path}", err=True)

    try:
        engine_kwargs = _build_engine_kwargs(
            embedding, reranker, expansion,
            embedding_base_url=embedding_base_url,
            embedding_api_key=embedding_api_key,
            embedding_dim=embedding_dim,
            reranker_base_url=reranker_base_url,
            reranker_api_key=reranker_api_key,
        )
        engine = Engine(project_path, chunk_enabled=chunk, **engine_kwargs)
    except click.ClickException:
        raise
    except Exception as e:
        raise click.ClickException(str(e))

    server = create_server(engine, auto_index=True)

    async def run():
        from mcp.server.stdio import stdio_server

        async with stdio_server() as (read_stream, write_stream):
            await server.run(read_stream, write_stream, server.create_initialization_options())

    asyncio.run(run())
