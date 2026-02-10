"""OpenACE command-line interface."""

from __future__ import annotations

import sys

import click


@click.group()
@click.version_option(package_name="openace")
def main():
    """OpenACE - AI-native code intelligence engine."""
    pass


@main.command()
@click.argument("path", default=".", type=click.Path(exists=True))
@click.option("--embedding", type=click.Choice(["local", "openai", "none"]), default="none",
              help="Embedding provider to use.")
def index(path: str, embedding: str):
    """Index a project directory."""
    from pathlib import Path
    from openace.engine import Engine

    project_path = str(Path(path).resolve())
    click.echo(f"Indexing {project_path}...")

    provider = None
    if embedding != "none":
        from openace.embedding.factory import create_provider
        provider = create_provider(embedding)

    engine = Engine(project_path, embedding_provider=provider)
    report = engine.index()

    click.echo(f"\nIndexing complete:")
    click.echo(f"  Files scanned:  {report.total_files_scanned}")
    click.echo(f"  Files indexed:  {report.files_indexed}")
    click.echo(f"  Files skipped:  {report.files_skipped}")
    click.echo(f"  Files failed:   {report.files_failed}")
    click.echo(f"  Symbols:        {report.total_symbols}")
    click.echo(f"  Relations:      {report.total_relations}")
    click.echo(f"  Duration:       {report.duration_secs:.2f}s")


@main.command()
@click.argument("query")
@click.option("--path", "-p", default=".", type=click.Path(exists=True),
              help="Project path.")
@click.option("--limit", "-n", default=10, help="Max results.")
@click.option("--language", "-l", default=None, help="Language filter.")
@click.option("--file-path", "-f", default=None, help="File path prefix filter.")
def search(query: str, path: str, limit: int, language: str, file_path: str):
    """Search for symbols in an indexed project."""
    from pathlib import Path
    from openace.engine import Engine

    project_path = str(Path(path).resolve())
    engine = Engine(project_path)
    results = engine.search(query, limit=limit, language=language, file_path=file_path)

    if not results:
        click.echo("No results found.")
        return

    for i, r in enumerate(results, 1):
        click.echo(
            f"{i}. {r.name} ({r.kind}) "
            f"[{', '.join(r.match_signals)}] "
            f"score={r.score:.4f}"
        )
        click.echo(f"   {r.file_path}:{r.line_range[0]}-{r.line_range[1]}")
        click.echo(f"   {r.qualified_name}")
        if r.related_symbols:
            related = ", ".join(rs.name for rs in r.related_symbols[:3])
            click.echo(f"   Related: {related}")
        click.echo()


@main.command()
@click.argument("path", default=".", type=click.Path(exists=True))
@click.option("--embedding", type=click.Choice(["local", "openai", "none"]), default="none",
              help="Embedding provider.")
def serve(path: str, embedding: str):
    """Start MCP server on stdio."""
    import asyncio
    from pathlib import Path
    from openace.engine import Engine
    from openace.server.app import create_server

    project_path = str(Path(path).resolve())
    click.echo(f"Starting OpenACE MCP server for {project_path}", err=True)

    provider = None
    if embedding != "none":
        from openace.embedding.factory import create_provider
        provider = create_provider(embedding)

    engine = Engine(project_path, embedding_provider=provider)

    # Index on startup
    click.echo("Indexing project...", err=True)
    report = engine.index()
    click.echo(
        f"Indexed {report.files_indexed} files, "
        f"{report.total_symbols} symbols in {report.duration_secs:.2f}s",
        err=True,
    )

    server = create_server(engine)

    async def run():
        try:
            from mcp.server.stdio import stdio_server
        except ImportError:
            click.echo(
                "MCP server requires the mcp package. "
                "Install with: pip install openace[mcp]",
                err=True,
            )
            sys.exit(1)

        async with stdio_server() as (read_stream, write_stream):
            await server.run(read_stream, write_stream, server.create_initialization_options())

    asyncio.run(run())
