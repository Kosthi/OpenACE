"""File-level summary generation for improved architectural queries."""

from __future__ import annotations

import logging
from typing import TYPE_CHECKING, Protocol, runtime_checkable

if TYPE_CHECKING:
    from openace.types import Symbol

logger = logging.getLogger(__name__)

# Maximum summary text size in characters.
_MAX_SUMMARY_CHARS = 4096

# Maximum number of key signatures to include.
_MAX_KEY_SIGNATURES = 20


@runtime_checkable
class SummaryGenerator(Protocol):
    """Protocol for file summary generators."""

    def generate(self, file_path: str, language: str, symbols: list[Symbol]) -> str:
        """Generate a summary for a single file from its symbols."""
        ...


class RuleBasedSummaryGenerator:
    """Zero-latency rule-based summary generator.

    Constructs a structured text summary from symbol metadata:
    file path, language, class/function listings, and key signatures.
    """

    def generate(self, file_path: str, language: str, symbols: list[Symbol]) -> str:
        if not symbols:
            return f"File: {file_path} ({language})\nEmpty file (no symbols)."

        classes: list[str] = []
        functions: list[str] = []
        methods: list[str] = []
        other: list[str] = []

        for sym in symbols:
            kind = sym.kind.lower()
            if kind in ("class", "struct", "interface", "trait", "enum"):
                classes.append(sym.name)
            elif kind == "function":
                functions.append(sym.name)
            elif kind == "method":
                methods.append(sym.name)
            else:
                other.append(sym.name)

        parts: list[str] = [f"File: {file_path} ({language})"]

        if classes:
            parts.append(f"Classes: {', '.join(classes)}")
        if functions:
            parts.append(f"Functions: {', '.join(functions)}")
        if methods:
            parts.append(f"Methods: {', '.join(methods)}")
        if other:
            parts.append(f"Other: {', '.join(other)}")

        # Key signatures section
        sig_lines: list[str] = []
        sig_budget = _MAX_SUMMARY_CHARS
        count = 0

        # Prioritize classes/structs first, then functions, then methods
        priority_order = []
        for sym in symbols:
            kind = sym.kind.lower()
            if kind in ("class", "struct", "interface", "trait", "enum"):
                priority_order.append((0, sym))
            elif kind == "function":
                priority_order.append((1, sym))
            elif kind == "method":
                priority_order.append((2, sym))
        priority_order.sort(key=lambda x: (x[0], x[1].line_start))

        for _priority, sym in priority_order:
            if count >= _MAX_KEY_SIGNATURES:
                break

            indent = "  " if sym.kind.lower() == "method" else ""
            line = f"{indent}{sym.signature or sym.qualified_name}"
            if sym.doc_comment:
                # Take first line of docstring
                first_line = sym.doc_comment.strip().split("\n")[0].strip()
                if first_line:
                    line += f": {first_line}"

            if sig_budget - len(line) < 0:
                break
            sig_budget -= len(line) + 1
            sig_lines.append(line)
            count += 1

        if sig_lines:
            parts.append("Key signatures:")
            parts.extend(sig_lines)

        return "\n".join(parts)


def generate_file_summaries(
    engine_binding,
    generator: SummaryGenerator,
    *,
    batch_size: int = 50,
) -> int:
    """Generate file-level summaries for all indexed files.

    Args:
        engine_binding: The Rust EngineBinding instance.
        generator: A SummaryGenerator to produce summary text.
        batch_size: Number of summary chunks to upsert per batch.

    Returns:
        Number of file summaries generated.
    """
    from openace._openace import PySummaryChunk
    from openace.engine import _convert_symbol

    files = engine_binding.list_indexed_files()
    if not files:
        return 0

    pending: list = []
    total = 0

    for file_info in files:
        py_syms = engine_binding.get_file_outline(file_info.path)
        symbols = [_convert_symbol(s) for s in py_syms]

        summary_text = generator.generate(
            file_info.path,
            file_info.language,
            symbols,
        )

        if not summary_text:
            continue

        pending.append(PySummaryChunk(
            file_path=file_info.path,
            language=file_info.language,
            content=summary_text,
        ))

        if len(pending) >= batch_size:
            count = engine_binding.upsert_summary_chunks(pending)
            total += count
            pending = []

    if pending:
        count = engine_binding.upsert_summary_chunks(pending)
        total += count

    return total
