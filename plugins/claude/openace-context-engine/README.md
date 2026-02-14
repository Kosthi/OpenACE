# OpenACE Context Engine Plugin for Claude Code

Bring full codebase context into your AI workflows. OpenACE is an AI-native Contextual Code Engine that delivers semantic code search, symbol lookup, and file outline capabilities across large codebases via MCP. Use it to find code locations, understand implementations, and plan changes with precision. Multi-signal retrieval (BM25 + vector search + exact match + graph expansion) with RRF fusion ensures high-quality recall across Python, TypeScript, JavaScript, Rust, Go, and Java.

## What's Included

This plugin provides:

- **MCP Server** - Connects Claude Code to OpenACE's Contextual Code Engine
- **Skills** - Auto-triggers codebase retrieval when you ask about your code
- **Agents** - A dedicated `context-expert` agent for focused code localization

## Claude Code Setup

Start Claude Code with the plugin directory:

```bash
claude --plugin-dir plugins/claude/openace-context-engine
```

**Prerequisites:**

- `uvx` must be on your PATH (install with `pip install uv` or `pipx install uv`)

**With embedding enabled** (recommended for better semantic search):

```bash
export OPENACE_EMBEDDING=siliconflow
export OPENAI_API_KEY=your-key-here
claude --plugin-dir plugins/claude/openace-context-engine
```

## Codex CLI Setup

Add OpenACE as an MCP server in Codex CLI:

```bash
codex mcp add openace -- uvx openace serve .
```

**With embedding enabled:**

```bash
codex mcp add openace \
  --env OPENACE_EMBEDDING=siliconflow \
  --env OPENAI_API_KEY=$OPENAI_API_KEY \
  -- uvx openace serve .
```

**Or configure via `config.toml`:**

```toml
[mcp_servers.openace]
type = "stdio"
command = "uvx"
args = ["openace", "serve", "."]

[mcp_servers.openace.env]
OPENACE_EMBEDDING = "none"
```

## Available Tools

### semantic_search

Searches your codebase using natural language queries and returns ranked results with relevance scores.

```
Input: "Where is user authentication handled?"
Output: Ranked results with scores, match signals, file paths, line ranges,
        qualified names, and related symbols
```

Key features:
- Multi-signal retrieval: BM25, vector search, exact match, and graph expansion
- Results fused with Reciprocal Rank Fusion (RRF) for high-quality recall
- Retrieves across Python, TypeScript, JavaScript, Rust, Go, and Java
- Reflects the current state of files on disk

### find_symbol

Finds symbols by exact name and returns their definitions.

```
Input: "AuthService"
Output: Matching symbols with file path, line range, and signature
```

Key features:
- Exact name lookup for functions, classes, structs, traits, and more
- Returns full signature and source location
- Fast direct lookup without requiring natural language

### get_file_outline

Returns all symbols defined in a file, giving you a structural overview.

```
Input: "src/auth/service.ts"
Output: List of symbols with kind, name, line range, and signature
```

Key features:
- Shows the complete structure of any indexed file
- Includes symbol kind (function, class, method, etc.), name, line range, and signature
- Useful for understanding a file before diving into specific functions

## Configuration

### Environment Variables

| Variable | Description | Values |
|---|---|---|
| `OPENACE_EMBEDDING` | Embedding provider for vector search | `none` (default), `local`, `openai`, `siliconflow` |
| `OPENACE_RERANKER` | Reranker backend for result reranking | `auto` (default), `rule_based`, `cross_encoder`, `cohere`, `openai`, `siliconflow`, `none` |
| `OPENAI_API_KEY` | API key for OpenAI or SiliconFlow backends | Required when using `openai` or `siliconflow` embedding |
| `COHERE_API_KEY` | API key for Cohere reranker | Required when using `cohere` reranker |

### Auto-Reranker Mapping

When `OPENACE_RERANKER=auto` (the default), the reranker is selected automatically based on the embedding provider:

| `OPENACE_EMBEDDING` | Reranker Used |
|---|---|
| `siliconflow` | `siliconflow` |
| `openai` | `rule_based` |
| `local` | `rule_based` |
| `none` | `none` |

**Priority:** CLI args > environment variables > defaults.

## Usage Examples

The plugin works automatically when you ask about your codebase:

- "Where is the function that handles user authentication?"
- "What tests are there for the login functionality?"
- "How is the database connected to the application?"
- "Where should I add a new API endpoint?"

Use `find_symbol` when you know the exact name:

- "Find the definition of `Engine`"
- "Look up the `StorageManager` class"

Use `get_file_outline` to understand a file's structure:

- "Show me the outline of `python/openace/engine.py`"

Or spawn the context-expert agent when you want focused code localization:

```
spawn context-expert to find all the payment processing code
```

## Development Setup

If the package is not yet available on PyPI, install from source:

```bash
pip install -e ".[mcp]"
```

Then run the server directly:

```bash
openace serve .
```

For Claude Code, modify `.mcp.json` in the plugin directory to use `openace` directly instead of `uvx`:

```json
{
  "mcpServers": {
    "openace": {
      "type": "stdio",
      "command": "openace",
      "args": ["serve", "."],
      "env": {
        "OPENACE_EMBEDDING": "${OPENACE_EMBEDDING}",
        "OPENACE_RERANKER": "${OPENACE_RERANKER}",
        "OPENAI_API_KEY": "${OPENAI_API_KEY}",
        "COHERE_API_KEY": "${COHERE_API_KEY}"
      }
    }
  }
}
```

For Codex CLI, update your `config.toml` accordingly:

```toml
[mcp_servers.openace]
type = "stdio"
command = "openace"
args = ["serve", "."]
```

## Best Practices

- **Be specific**: Detailed queries yield better results from `semantic_search`
- **Use natural language**: Describe what you're looking for conceptually
- **Iterate**: If initial results aren't helpful, try alternative terminology
- **Combine tools**: Use `semantic_search` to locate a file, `get_file_outline` to understand its structure, then `find_symbol` for specific definitions
- **Use exact names when available**: `find_symbol` is faster and more precise when you know the symbol name
- **Combine with other tools**: Use retrieval results to guide further exploration with file viewing

## Known Limitations

- **First-run indexing latency**: The initial index build scales with project size. Subsequent updates are incremental and fast.
- **Package not yet on PyPI**: Use the development setup described above for local installation.
- **Rust toolchain may be required**: If no pre-built wheel is available for your platform, building from source requires a Rust toolchain.
