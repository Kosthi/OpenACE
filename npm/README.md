# OpenACE

AI-native Contextual Code Engine. Multi-signal semantic code search via MCP for Claude Code, Codex CLI, and any MCP-compatible client.

## Install

```bash
npm install -g openace
```

## Use with Claude Code

Add to your project's `.mcp.json`:

```json
{
  "mcpServers": {
    "openace": {
      "command": "npx",
      "args": ["-y", "openace", "serve", ".", "--embedding", "siliconflow"],
      "env": {
        "OPENACE_EMBEDDING_API_KEY": "your-api-key"
      }
    }
  }
}
```

## Use with Codex CLI

```bash
codex mcp add openace -- npx -y openace serve . --embedding siliconflow
```

## Commands

```bash
# Start MCP server (default)
openace serve /path/to/project --embedding siliconflow

# Index a project
openace index /path/to/project --embedding siliconflow

# Search
openace search "user authentication" -p /path/to/project
```

## MCP Tools

- **semantic_search** — Search code by natural language query with ranked results and snippets
- **find_symbol** — Look up symbols by exact name
- **get_file_outline** — Get structural overview of a file

## How It Works

This npm package is a thin wrapper that delegates to the Python `openace` package via [`uvx`](https://docs.astral.sh/uv/). The Rust+Python engine provides:

- Multi-signal retrieval: BM25 + vector kNN + exact match + graph expansion + AST chunks
- Reciprocal Rank Fusion (RRF) for high-quality ranking
- CJK bigram tokenization for Chinese/Japanese/Korean queries
- Support for Python, TypeScript, JavaScript, Rust, Go, Java

## Prerequisites

- [`uv`](https://docs.astral.sh/uv/) must be installed (`pip install uv` or `curl -LsSf https://astral.sh/uv/install.sh | sh`)

## Custom API Providers

Supports any OpenAI-compatible embedding/reranking API:

| Variable | Description |
|----------|-------------|
| `OPENACE_EMBEDDING_API_KEY` | API key for embedding provider |
| `OPENACE_EMBEDDING_BASE_URL` | Custom base URL for embedding API |
| `OPENACE_RERANKER_API_KEY` | API key for reranker provider |
| `OPENACE_RERANKER_BASE_URL` | Custom base URL for reranker API |

## License

MIT
