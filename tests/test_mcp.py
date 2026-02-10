"""Tests for the MCP server."""

import pytest

from openace.engine import Engine


@pytest.fixture
def indexed_engine(sample_project):
    """Create an Engine with an indexed sample project."""
    engine = Engine(str(sample_project))
    engine.index()
    return engine


class TestMCPServer:
    def test_server_creation(self, indexed_engine):
        """Test that the MCP server can be created."""
        pytest.importorskip("mcp")
        from openace.server.app import create_server

        server = create_server(indexed_engine)
        assert server is not None

    def test_server_creation_without_mcp(self, indexed_engine, monkeypatch):
        """Test graceful error when mcp is not installed."""
        import sys
        # Only test if mcp is NOT installed
        try:
            import mcp
            pytest.skip("mcp is installed, cannot test missing import")
        except ImportError:
            with pytest.raises(ImportError, match="mcp"):
                from openace.server.app import create_server
                create_server(indexed_engine)


class TestCLI:
    def test_cli_group_exists(self):
        from openace.cli import main
        assert main is not None

    def test_cli_help(self):
        from click.testing import CliRunner
        from openace.cli import main

        runner = CliRunner()
        result = runner.invoke(main, ["--help"])
        assert result.exit_code == 0
        assert "OpenACE" in result.output

    def test_cli_index_help(self):
        from click.testing import CliRunner
        from openace.cli import main

        runner = CliRunner()
        result = runner.invoke(main, ["index", "--help"])
        assert result.exit_code == 0
        assert "Index" in result.output

    def test_cli_search_help(self):
        from click.testing import CliRunner
        from openace.cli import main

        runner = CliRunner()
        result = runner.invoke(main, ["search", "--help"])
        assert result.exit_code == 0

    def test_cli_serve_help(self):
        from click.testing import CliRunner
        from openace.cli import main

        runner = CliRunner()
        result = runner.invoke(main, ["serve", "--help"])
        assert result.exit_code == 0
