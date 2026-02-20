[Root](../CLAUDE.md) > **python/openace**

# Python SDK (openace)

## Module Responsibility

High-level Python interface to the OpenACE engine. Provides the `Engine` class, CLI, MCP server, pluggable embedding providers, and pluggable rerankers. Wraps the Rust core via PyO3 bindings.

## Entry Points

- **`openace.engine.Engine`** -- Primary SDK class for indexing, searching, and symbol lookup
- **`openace.cli.main`** -- Click CLI group (`openace index`, `openace search`, `openace serve`)
- **`openace.server.app.create_server`** -- MCP server factory

## Public API

### Engine (`engine.py`)
- `Engine(project_root, *, embedding_provider=None, embedding_dim=None, reranker=None, rerank_pool_size=50)`
- `index(incremental=True)` -> `IndexReport` -- Run full indexing; auto-embeds if provider set
- `search(query, *, limit=10, language=None, file_path=None)` -> `list[SearchResult]` -- Two-stage: retrieval (expanded pool if reranker) then rerank. Fail-open on reranker errors.
- `find_symbol(name)` -> `list[Symbol]` -- Exact name match
- `get_file_outline(path)` -> `list[Symbol]` -- File outline
- `embed_all()` -> `int` -- Batch embed all symbols
- `flush()` -- Persist storage

### CLI (`cli.py`)
- `openace index [PATH] [--embedding {local,openai,siliconflow,none}] [--reranker {auto,...}]`
- `openace search QUERY [-p PATH] [--embedding ...] [--reranker ...] [-n LIMIT] [-l LANGUAGE] [-f FILE_PATH]`
- `openace serve [PATH] [--embedding ...] [--reranker ...]` -- MCP server on stdio

### MCP Server (`server/app.py`)
Three tools: `semantic_search`, `find_symbol`, `get_file_outline`. Uses `asyncio.to_thread()` for non-blocking Rust calls.

### Embedding Providers (`embedding/`)
Protocol-based (`EmbeddingProvider`): `embed(texts) -> np.ndarray`, `dimension -> int`
- `OnnxEmbedder` (`local.py`) -- all-MiniLM-L6-v2, 384-dim, lazy model download
- `OpenAIEmbedder` (`openai_backend.py`) -- OpenAI API, also used for SiliconFlow with custom base_url/model
- Factory: `create_provider("local"|"openai"|"siliconflow")`

### Reranking Providers (`reranking/`)
Protocol-based (`Reranker`): `rerank(query, results, *, top_k=None) -> list[SearchResult]`
- `RuleBasedReranker` (`rule_based.py`) -- Kind weighting + signal bonus + exact match bonus
- `CrossEncoderReranker` (`cross_encoder.py`) -- Local cross-encoder model
- `LLMReranker` (`llm_backend.py`) -- Cohere/OpenAI LLM-based reranking
- `APIReranker` (`api_reranker.py`) -- Generic API reranker (SiliconFlow)
- Factory: `create_reranker("rule_based"|"cross_encoder"|"cohere"|"openai"|"siliconflow"|"api")`

### Types (`types.py`)
Frozen dataclasses: `Symbol`, `SearchResult` (with `rerank_score`), `IndexReport`, `Relation`

### Exceptions (`exceptions.py`)
`OpenACEError` -> `IndexingError`, `SearchError`, `StorageError`

## Key Design

- Lazy imports in `__init__.py` to avoid loading Rust extension at module import
- Protocol (structural typing) for provider interfaces -- no inheritance required
- Two-stage search: retrieval with expanded pool -> reranker with top_k
- Fail-open: reranker errors fall back to original ranking with warning
- `rerank_pool_size` capped at 100 (Rust upper bound)

## Tests

- `tests/test_engine.py` -- Engine integration (index, search, find_symbol, file_outline, flush)
- `tests/test_embedding.py` -- Embedding factory, OnnxEmbedder, OpenAIEmbedder
- `tests/test_mcp.py` -- MCP server creation, CLI help commands
- `tests/test_reranking.py` -- Protocol conformance, RuleBasedReranker, factory, Engine integration, mock/failing rerankers

## Related Files

- `python/openace/__init__.py`, `engine.py`, `cli.py`, `types.py`, `exceptions.py`
- `python/openace/server/__init__.py`, `server/app.py`
- `python/openace/embedding/__init__.py`, `embedding/protocol.py`, `embedding/factory.py`, `embedding/local.py`, `embedding/openai_backend.py`
- `python/openace/reranking/__init__.py`, `reranking/protocol.py`, `reranking/factory.py`, `reranking/rule_based.py`, `reranking/cross_encoder.py`, `reranking/llm_backend.py`, `reranking/api_reranker.py`
- `tests/conftest.py`, `tests/test_engine.py`, `tests/test_embedding.py`, `tests/test_mcp.py`, `tests/test_reranking.py`

## Changelog

- 2026-02-11: Initial module CLAUDE.md created.
