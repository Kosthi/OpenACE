"""Shared test fixtures for OpenACE integration tests."""

import os
import tempfile
import shutil

import pytest


@pytest.fixture
def sample_project(tmp_path):
    """Create a minimal sample project with source files for testing."""
    src = tmp_path / "src"
    src.mkdir()

    # Python file
    (src / "main.py").write_text(
        'def process_data(items):\n'
        '    """Process a list of items."""\n'
        '    return [validate(x) for x in items]\n'
        '\n'
        'def validate(item):\n'
        '    """Validate a single item."""\n'
        '    if item is None:\n'
        '        raise ValueError("item cannot be None")\n'
        '    return item\n'
        '\n'
        'class DataProcessor:\n'
        '    def __init__(self, config):\n'
        '        self.config = config\n'
        '\n'
        '    def run(self):\n'
        '        return process_data(self.config.get("items", []))\n'
    )

    # Another Python file
    (src / "utils.py").write_text(
        'def format_output(data):\n'
        '    """Format data for display."""\n'
        '    return str(data)\n'
        '\n'
        'def parse_input(raw):\n'
        '    """Parse raw input string."""\n'
        '    return raw.strip().split(",")\n'
    )

    return tmp_path
