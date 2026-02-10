"""Integration tests for the OpenACE Engine."""

import pytest

from openace.engine import Engine
from openace.types import IndexReport, SearchResult, Symbol
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

        # Second index should produce similar results
        assert report2.files_indexed == report1.files_indexed
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

        results = engine.search("process_data")
        assert results[0].name == "process_data"
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
