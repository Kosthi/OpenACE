"""Unit tests for _extract_identifiers() in openace.engine."""

import pytest

from openace.engine import _extract_identifiers


class TestCamelCase:
    def test_standard_camelcase(self):
        ids = _extract_identifiers("Check the HTMLParser class")
        assert "HTMLParser" in ids

    def test_multi_word_camelcase(self):
        ids = _extract_identifiers("Fix DataProcessor and RuleValidator")
        assert "DataProcessor" in ids
        assert "RuleValidator" in ids

    def test_camelcase_with_digits(self):
        ids = _extract_identifiers("See RuleL031 for details")
        assert "RuleL031" in ids


class TestSnakeCase:
    def test_standard_snake_case(self):
        ids = _extract_identifiers("The process_data function is broken")
        assert "process_data" in ids

    def test_multi_segment(self):
        ids = _extract_identifiers("Call validate_user_input")
        assert "validate_user_input" in ids

    def test_single_word_not_matched(self):
        """Single words without underscores should not be matched as snake_case."""
        ids = _extract_identifiers("The function is broken")
        # No snake_case identifiers in pure English
        snake = [i for i in ids if "_" in i]
        assert len(snake) == 0


class TestDottedRefs:
    def test_dotted_module_class(self):
        ids = _extract_identifiers("Use module.ClassName.method")
        assert "module.ClassName.method" in ids
        assert "method" in ids  # last component extracted

    def test_two_part_dotted(self):
        ids = _extract_identifiers("See os.path for details")
        assert "os.path" in ids
        assert "path" in ids


class TestFilePaths:
    def test_python_file(self):
        ids = _extract_identifiers("Look at L031.py")
        assert "L031" in ids

    def test_nested_path(self):
        ids = _extract_identifiers("Check src/rules/handler.py")
        assert "handler" in ids

    def test_multiple_extensions(self):
        ids = _extract_identifiers("Fix main.ts and utils.rs")
        assert "main" in ids
        assert "utils" in ids


class TestAllCaps:
    def test_constant(self):
        ids = _extract_identifiers("Increase MAX_RETRIES to 5")
        assert "MAX_RETRIES" in ids

    def test_multi_word_constant(self):
        ids = _extract_identifiers("Set DEFAULT_TIMEOUT_MS")
        assert "DEFAULT_TIMEOUT_MS" in ids


class TestEdgeCases:
    def test_empty_string(self):
        assert _extract_identifiers("") == []

    def test_pure_natural_language(self):
        ids = _extract_identifiers("This is a bug where the thing does not work correctly")
        # Should be empty or very minimal — no code identifiers
        assert len(ids) == 0

    def test_deduplication(self):
        ids = _extract_identifiers("FooBar FooBar FooBar")
        assert ids.count("FooBar") == 1

    def test_cap_at_20(self):
        # Generate a string with many identifiers
        many = " ".join(f"func_{i}" for i in range(30))
        ids = _extract_identifiers(many)
        assert len(ids) <= 20

    def test_mixed_identifiers(self):
        text = "Fix HTMLParser.parse_data in module.HTMLParser, check MAX_SIZE and L031.py"
        ids = _extract_identifiers(text)
        assert "HTMLParser" in ids
        assert "parse_data" in ids
        assert "module.HTMLParser" in ids
        assert "MAX_SIZE" in ids
        assert "L031" in ids
