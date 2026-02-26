"""Integration tests for the OpenACE Engine."""

import pytest

from openace.engine import Engine
from openace.types import IndexReport, SearchResult, Symbol, FunctionContext, CallChainNode
from openace.exceptions import OpenACEError


class TestEngineIndex:
    def test_index_returns_report(self, sample_project):
        engine = Engine(str(sample_project))
        report = engine.index()

        assert isinstance(report, IndexReport)
        assert report.files_indexed >= 2
        assert report.total_symbols > 0
        assert report.total_relations >= 0
        assert report.duration_secs >= 0

    def test_index_finds_symbols(self, sample_project):
        engine = Engine(str(sample_project))
        engine.index()

        # Should find process_data
        symbols = engine.find_symbol("process_data")
        assert len(symbols) > 0
        assert any(s.name == "process_data" for s in symbols)

    def test_index_idempotent(self, sample_project):
        engine = Engine(str(sample_project))
        report1 = engine.index()
        report2 = engine.index()

        # Second index uses incremental path: no files changed → files_indexed=0
        # but total_symbols should remain the same
        assert report2.total_symbols == report1.total_symbols


class TestEngineSearch:
    def test_search_returns_results(self, sample_project):
        engine = Engine(str(sample_project))
        engine.index()

        results = engine.search("process_data")
        assert len(results) > 0
        assert isinstance(results[0], SearchResult)

    def test_search_top_result_is_relevant(self, sample_project):
        engine = Engine(str(sample_project))
        engine.index()

        results = engine.search("process_data", dedupe_by_file=False)
        result_names = [r.name for r in results]
        assert "process_data" in result_names, f"process_data should be in results, got: {result_names}"
        assert results[0].score > 0

    def test_search_respects_limit(self, sample_project):
        engine = Engine(str(sample_project))
        engine.index()

        results = engine.search("process", limit=1)
        assert len(results) <= 1

    def test_search_empty_query_returns_empty(self, sample_project):
        engine = Engine(str(sample_project))
        engine.index()

        results = engine.search("xyznonexistentsymbol123")
        assert len(results) == 0

    def test_search_has_signals(self, sample_project):
        engine = Engine(str(sample_project))
        engine.index()

        results = engine.search("validate")
        assert len(results) > 0
        assert len(results[0].match_signals) > 0


class TestEngineFindSymbol:
    def test_find_by_name(self, sample_project):
        engine = Engine(str(sample_project))
        engine.index()

        symbols = engine.find_symbol("validate")
        assert len(symbols) > 0
        assert all(isinstance(s, Symbol) for s in symbols)
        assert any(s.name == "validate" for s in symbols)

    def test_find_nonexistent(self, sample_project):
        engine = Engine(str(sample_project))
        engine.index()

        symbols = engine.find_symbol("nonexistent_symbol_xyz")
        assert len(symbols) == 0


class TestEngineFileOutline:
    def test_file_outline(self, sample_project):
        engine = Engine(str(sample_project))
        engine.index()

        symbols = engine.get_file_outline("src/main.py")
        assert len(symbols) > 0
        names = [s.name for s in symbols]
        assert "process_data" in names
        assert "validate" in names
        assert "DataProcessor" in names

    def test_file_outline_empty(self, sample_project):
        engine = Engine(str(sample_project))
        engine.index()

        symbols = engine.get_file_outline("nonexistent.py")
        assert len(symbols) == 0


class TestEngineFlush:
    def test_flush_succeeds(self, sample_project):
        engine = Engine(str(sample_project))
        engine.index()
        engine.flush()  # Should not raise


class TestEngineIncrementalIndex:
    def test_incremental_first_run_falls_back_to_full(self, sample_project):
        """First run on a fresh project should fall back to full index."""
        engine = Engine(str(sample_project))
        report = engine.index(incremental=True)

        assert isinstance(report, IndexReport)
        assert report.files_indexed >= 2
        assert report.total_symbols > 0

    def test_incremental_no_changes(self, sample_project):
        """Second run with no changes should index zero files."""
        engine = Engine(str(sample_project))
        report1 = engine.index(incremental=True)

        # Second run — nothing changed
        report2 = engine.index(incremental=True)
        assert report2.files_indexed == 0
        # Total symbols should still be the same (from storage)
        assert report2.total_symbols == report1.total_symbols

    def test_incremental_after_file_modification(self, sample_project):
        """Modifying a file should re-index only that file."""
        engine = Engine(str(sample_project))
        engine.index(incremental=True)

        # Modify a file
        main_py = sample_project / "src" / "main.py"
        content = main_py.read_text()
        main_py.write_text(content + "\ndef new_function():\n    pass\n")

        report = engine.index(incremental=True)
        assert report.files_indexed == 1  # Only the modified file

        # The new function should be findable
        symbols = engine.find_symbol("new_function")
        assert len(symbols) > 0
        assert any(s.name == "new_function" for s in symbols)

    def test_incremental_after_file_deletion(self, sample_project):
        """Deleting a file should remove its symbols."""
        engine = Engine(str(sample_project))
        engine.index(incremental=True)

        # Verify utils.py symbols exist
        symbols = engine.find_symbol("format_output")
        assert len(symbols) > 0

        # Delete utils.py
        (sample_project / "src" / "utils.py").unlink()

        report = engine.index(incremental=True)
        # The deleted file's symbols should be gone
        symbols = engine.find_symbol("format_output")
        assert len(symbols) == 0

    def test_incremental_after_file_addition(self, sample_project):
        """Adding a new file should index it."""
        engine = Engine(str(sample_project))
        engine.index(incremental=True)

        # Add a new file
        (sample_project / "src" / "new_module.py").write_text(
            'def brand_new_func():\n'
            '    """A brand new function."""\n'
            '    return 42\n'
        )

        report = engine.index(incremental=True)
        assert report.files_indexed >= 1

        symbols = engine.find_symbol("brand_new_func")
        assert len(symbols) > 0

    def test_force_full_overrides_incremental(self, sample_project):
        """force_full=True should always do a full reindex."""
        engine = Engine(str(sample_project))
        engine.index(incremental=True)

        # Force full should reindex everything
        report = engine.index(force_full=True)
        assert report.files_indexed >= 2


class TestEngineFunctionContext:
    def test_function_context_returns_result(self, sample_project):
        """get_function_context should return a FunctionContext for a valid symbol."""
        engine = Engine(str(sample_project))
        engine.index()

        symbols = engine.find_symbol("process_data")
        assert len(symbols) > 0

        ctx = engine.get_function_context(symbols[0].id)
        assert isinstance(ctx, FunctionContext)
        assert isinstance(ctx.symbol, CallChainNode)
        assert ctx.symbol.name == "process_data"
        assert ctx.symbol.depth == 0

    def test_function_context_callees(self, sample_project):
        """process_data calls validate, so callees should be non-empty."""
        engine = Engine(str(sample_project))
        engine.index()

        symbols = engine.find_symbol("process_data")
        assert len(symbols) > 0

        ctx = engine.get_function_context(symbols[0].id)
        callee_names = [n.name for n in ctx.callees]
        assert "validate" in callee_names, f"Expected 'validate' in callees, got: {callee_names}"

    def test_function_context_nonexistent_symbol(self, sample_project):
        """get_function_context should raise for a nonexistent symbol ID."""
        engine = Engine(str(sample_project))
        engine.index()

        with pytest.raises(Exception):
            engine.get_function_context("0" * 32)
