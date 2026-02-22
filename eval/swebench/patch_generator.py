"""Patch generation using LLM with prompt templates."""

from __future__ import annotations

import logging
import re
from pathlib import Path

from eval.swebench.llm_client import LLMClient

logger = logging.getLogger(__name__)

_PROMPT_TEMPLATE_PATH = Path(__file__).parent / "prompts" / "patch_generation.txt"

_DEFAULT_SYSTEM = (
    "You are an expert software engineer. Your task is to fix bugs in code "
    "repositories based on issue descriptions. Generate a patch in unified diff "
    "format. Output ONLY the patch content."
)


def generate_patch(
    client: LLMClient,
    problem_statement: str,
    context: str,
    *,
    hints_text: str = "",
) -> str:
    """Generate a patch using the LLM.

    Args:
        client: LLM client to use.
        problem_statement: The issue description.
        context: Retrieved code context (empty for baseline).
        hints_text: Optional hint text from the dataset.

    Returns:
        Generated patch as a unified diff string.
    """
    prompt = _build_prompt(problem_statement, context, hints_text)

    logger.info(
        "Generating patch (prompt length: %d chars, context: %s)",
        len(prompt),
        "yes" if context else "no",
    )

    raw_output = client.generate(prompt, system=_DEFAULT_SYSTEM)

    patch = extract_diff(raw_output)
    return patch


def _build_prompt(
    problem_statement: str,
    context: str,
    hints_text: str,
) -> str:
    """Build the full prompt from template or inline."""
    # Try loading external template
    if _PROMPT_TEMPLATE_PATH.exists():
        template = _PROMPT_TEMPLATE_PATH.read_text()
    else:
        template = _INLINE_TEMPLATE

    prompt = template.replace("{problem_statement}", problem_statement)
    prompt = prompt.replace("{hints_text}", hints_text or "None provided.")

    if context:
        prompt = prompt.replace("{context_section}", _CONTEXT_BLOCK.format(context=context))
    else:
        prompt = prompt.replace("{context_section}", "")

    return prompt


_CONTEXT_BLOCK = """\

## Relevant Code Context
{context}
"""

_INLINE_TEMPLATE = """\
Fix the bug described in the issue below.

## Issue Description
{problem_statement}

## Hints
{hints_text}
{context_section}
## Instructions
1. Analyze the issue and the relevant code.
2. Generate a patch in unified diff format that fixes the issue.
3. The patch should start with `diff --git` lines.
4. Output ONLY the patch, no explanation before or after.
"""


def extract_diff(raw_output: str) -> str:
    """Extract a unified diff from LLM output.

    Handles common LLM output patterns:
    - Direct diff output
    - Diff wrapped in ```diff ... ``` code blocks
    - Multiple code blocks (takes the longest one)
    """
    # Try extracting from fenced code blocks first
    code_blocks = re.findall(
        r'```(?:diff)?\s*\n(.*?)```',
        raw_output,
        re.DOTALL,
    )
    if code_blocks:
        # Pick the longest block that looks like a diff
        diff_blocks = [b for b in code_blocks if _looks_like_diff(b)]
        if diff_blocks:
            return max(diff_blocks, key=len).strip()
        # Fall back to the longest block overall
        return max(code_blocks, key=len).strip()

    # Try finding diff content directly (starts with diff --git or --- )
    lines = raw_output.split("\n")
    diff_start = None
    for i, line in enumerate(lines):
        if line.startswith("diff --git") or (
            line.startswith("--- ") and i + 1 < len(lines) and lines[i + 1].startswith("+++ ")
        ):
            diff_start = i
            break

    if diff_start is not None:
        return "\n".join(lines[diff_start:]).strip()

    # Last resort: return the raw output
    return raw_output.strip()


def _looks_like_diff(text: str) -> bool:
    """Heuristic check if text looks like a unified diff."""
    return (
        "diff --git" in text
        or ("--- " in text and "+++ " in text)
        or ("@@" in text and ("+" in text or "-" in text))
    )
