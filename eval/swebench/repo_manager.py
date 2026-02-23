"""Repository clone and checkout management."""

from __future__ import annotations

import logging
import subprocess
from pathlib import Path

logger = logging.getLogger(__name__)

# Extensions considered source code — everything else gets stripped.
_SOURCE_EXTENSIONS: set[str] = {
    # Python
    ".py", ".pyx", ".pxd",
    # JavaScript / TypeScript
    ".js", ".jsx", ".ts", ".tsx", ".mjs", ".cjs",
    # Rust
    ".rs",
    # Go
    ".go",
    # Java / Kotlin / Scala
    ".java", ".kt", ".kts", ".scala",
    # C / C++
    ".c", ".h", ".cpp", ".hpp", ".cc", ".hh", ".cxx",
    # C#
    ".cs",
    # Ruby
    ".rb",
    # PHP
    ".php",
    # Swift
    ".swift",
    # Shell
    ".sh", ".bash",
    # Perl
    ".pl", ".pm",
    # Lua
    ".lua",
    # Clojure
    ".clj", ".cljs",
    # Elixir
    ".ex", ".exs",
    # Haskell
    ".hs",
    # Erlang
    ".erl",
    # OCaml
    ".ml", ".mli",
    # Julia
    ".jl",
    # Dart
    ".dart",
    # R
    ".r",
}

_STRIP_MARKER = ".source_only_stripped"


def strip_non_source_files(repo_path: Path) -> int:
    """Remove non-source-code files from a cloned repo.

    Keeps only files whose extension (case-insensitive) is in
    ``_SOURCE_EXTENSIONS``.  Skips the ``.git`` directory.
    Writes a marker file so the operation is idempotent across reruns.

    Returns the number of files removed.
    """
    marker = repo_path / _STRIP_MARKER
    if marker.exists():
        logger.debug("Already stripped: %s", repo_path)
        return 0

    removed = 0
    for f in repo_path.rglob("*"):
        if not f.is_file():
            continue
        try:
            rel = f.relative_to(repo_path)
        except ValueError:
            continue
        if rel.parts[0] == ".git":
            continue
        if f.suffix.lower() not in _SOURCE_EXTENSIONS:
            f.unlink()
            removed += 1

    # Clean up empty directories (bottom-up).
    for d in sorted(repo_path.rglob("*"), reverse=True):
        if not d.is_dir():
            continue
        try:
            rel = d.relative_to(repo_path)
        except ValueError:
            continue
        if rel.parts[0] == ".git":
            continue
        try:
            d.rmdir()  # succeeds only when empty
        except OSError:
            pass

    marker.write_text(str(removed))
    logger.info("Stripped %d non-source files from %s", removed, repo_path)
    return removed


def clone_or_reuse(
    repo: str,
    base_commit: str,
    repos_dir: str | Path,
) -> Path:
    """Clone the repository and checkout the specified commit.

    Uses a two-level cache: ``repos_dir/{owner}__{name}/{commit}/``.
    If the directory already exists and HEAD matches, skip clone.

    Args:
        repo: GitHub repo in ``owner/name`` format.
        base_commit: Git commit SHA to checkout.
        repos_dir: Root directory for cached repos.

    Returns:
        Path to the checked-out working directory.
    """
    repos_dir = Path(repos_dir)
    owner, name = repo.split("/")
    dest = repos_dir / f"{owner}__{name}" / base_commit

    if dest.exists() and _head_matches(dest, base_commit):
        logger.info("Reusing cached repo: %s @ %s", repo, base_commit[:10])
        return dest

    dest.mkdir(parents=True, exist_ok=True)

    clone_url = f"https://github.com/{repo}.git"

    if (dest / ".git").exists():
        # Directory exists but HEAD mismatch -- reset to the right commit.
        logger.info("Resetting existing clone to %s", base_commit[:10])
        _run(["git", "fetch", "origin"], cwd=dest)
        _run(["git", "checkout", base_commit], cwd=dest)
        _run(["git", "clean", "-fdx"], cwd=dest)
        return dest

    logger.info("Cloning %s ...", repo)
    _run(["git", "clone", "--quiet", clone_url, str(dest)])
    _run(["git", "checkout", base_commit], cwd=dest)

    return dest


def _head_matches(repo_path: Path, expected_commit: str) -> bool:
    """Check if the repo HEAD matches the expected commit."""
    try:
        result = subprocess.run(
            ["git", "rev-parse", "HEAD"],
            cwd=repo_path,
            capture_output=True,
            text=True,
            check=True,
        )
        return result.stdout.strip() == expected_commit
    except (subprocess.CalledProcessError, FileNotFoundError):
        return False


def _run(
    cmd: list[str],
    cwd: Path | None = None,
    timeout: int = 600,
) -> subprocess.CompletedProcess:
    """Run a git command with logging."""
    logger.debug("Running: %s (cwd=%s)", " ".join(cmd), cwd)
    return subprocess.run(
        cmd,
        cwd=cwd,
        capture_output=True,
        text=True,
        check=True,
        timeout=timeout,
    )
