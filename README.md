# OpenACE

AI-native Contextual Code Engine. Rust core with Python bindings, exposing a CLI, Python SDK, and MCP server for Claude Code / Codex CLI.

OpenACE combines multi-signal retrieval (BM25 full-text + vector kNN + exact match + graph expansion + AST chunk search) with Reciprocal Rank Fusion to deliver high-quality code search across Python, TypeScript, JavaScript, Rust, Go, and Java.

## Quick Start

```bash
pip install "openace[mcp,openai]"

# Index a project
openace index /path/to/project --embedding siliconflow

# Search
openace search "user authentication" -p /path/to/project --embedding siliconflow

# Start MCP server
openace serve /path/to/project --embedding siliconflow
```

## Use with Claude Code

Add to your project's `.mcp.json`:

```json
{
  "mcpServers": {
    "openace": {
      "command": "openace",
      "args": ["serve", ".", "--embedding", "siliconflow"],
      "env": {
        "OPENACE_EMBEDDING_API_KEY": "your-api-key"
      }
    }
  }
}
```

Or use `uvx` for zero-install (once published to PyPI):

```json
{
  "mcpServers": {
    "openace": {
      "command": "uvx",
      "args": ["openace", "serve", ".", "--embedding", "siliconflow"],
      "env": {
        "OPENACE_EMBEDDING_API_KEY": "your-api-key"
      }
    }
  }
}
```

## Use with Codex CLI

```bash
codex mcp add openace -- uvx openace serve . --embedding siliconflow
```

## Custom API Providers

OpenACE supports any OpenAI-compatible embedding/reranking API. Configure via CLI flags or environment variables:

```json
{
  "mcpServers": {
    "openace": {
      "command": "openace",
      "args": [
        "serve", ".",
        "--embedding", "openai",
        "--reranker", "siliconflow"
      ],
      "env": {
        "OPENACE_EMBEDDING_API_KEY": "your-embedding-key",
        "OPENACE_EMBEDDING_BASE_URL": "https://api.your-provider.com/v1",
        "OPENACE_RERANKER_API_KEY": "your-reranker-key",
        "OPENACE_RERANKER_BASE_URL": "https://api.your-reranker.com/v1"
      }
    }
  }
}
```

### Environment Variables

| Variable | Description |
|----------|-------------|
| `OPENACE_EMBEDDING_API_KEY` | API key for embedding provider |
| `OPENACE_EMBEDDING_BASE_URL` | Custom base URL for embedding API |
| `OPENACE_EMBEDDING_DIM` | Override embedding vector dimension |
| `OPENACE_RERANKER_API_KEY` | API key for reranker provider |
| `OPENACE_RERANKER_BASE_URL` | Custom base URL for reranker API |
| `OPENACE_EMBEDDING` | Default embedding provider for `serve` |
| `OPENACE_RERANKER` | Default reranker for `serve` |

### Embedding Providers

| Name | Model | Requires |
|------|-------|----------|
| `siliconflow` | Qwen/Qwen3-Embedding-8B (1024-dim) | `openace[openai]` + API key |
| `openai` | text-embedding-3-small | `openace[openai]` + API key |
| `local` | all-MiniLM-L6-v2 (384-dim) | `openace[onnx]` |
| `none` | BM25 only, no vector search | (default) |

### Reranker Providers

| Name | Backend | Requires |
|------|---------|----------|
| `auto` | Matches embedding provider | (default) |
| `siliconflow` | Qwen/Qwen3-Reranker-8B | `openace[openai]` + API key |
| `cohere` | Cohere Rerank | `openace[rerank-cohere]` + API key |
| `cross_encoder` | Local cross-encoder model | `openace[rerank-local]` |
| `rule_based` | Heuristic (no API needed) | (built-in) |

## MCP Tools

### semantic_search

Search for code by natural language query. Returns file-level results with code snippets, match signals, and file outlines.

### find_symbol

Look up symbols by exact name. Returns definitions with file path, line range, and signature.

### get_file_outline

Get all symbols defined in a file. Returns a structural overview with symbol kinds, names, and line ranges.

## Python SDK

```python
from openace import Engine

engine = Engine("/path/to/project")
engine.index()

# Semantic search
results = engine.search("parse XML", limit=10)
for r in results:
    print(f"{r.file_path}:{r.line_range[0]} {r.name} ({r.kind})")

# Exact lookup
symbols = engine.find_symbol("MyClass")

# File outline
outline = engine.get_file_outline("src/main.py")
```

## Installation

```bash
# Default install (includes MCP server, OpenAI/SiliconFlow embedding & reranking)
pip install openace

# With local ONNX embedding (no API key needed)
pip install "openace[onnx]"

# With Cohere reranker
pip install "openace[rerank-cohere]"

# Development
pip install -e ".[dev]"
```

## Architecture

```
Source files --> Scanner --> Parser (tree-sitter) --> Indexer --> Storage
                                                                   |
                                                     SQLite (graph/relations)
                                                     Tantivy (BM25 full-text)
                                                     usearch (vector kNN)
                                                                   |
Query --> CLI/SDK/MCP --> Retrieval (BM25 + vector + exact + graph + chunk)
                              |
                          RRF Fusion --> Reranker --> Results
```

- **Rust core** handles all performance-critical operations (parsing, indexing, storage, retrieval)
- **Python layer** provides the SDK, CLI, MCP server, and pluggable embedding/reranking providers
- **tree-sitter** for multi-language AST parsing
- **Triple-backend storage**: SQLite for graph/relations, Tantivy for BM25, usearch for HNSW vector search
- **CJK bigram tokenization** in BM25 for Chinese/Japanese/Korean query support

## Supported Languages

Python, TypeScript, JavaScript, Rust, Go, Java.

## Building from Source

Requires Rust >= 1.85.0 and Python >= 3.10.

```bash
git clone https://github.com/Kosthi/OpenACE.git
cd OpenACE
pip install maturin
maturin develop --release
pip install -e ".[dev]"
```

## License

MIT
