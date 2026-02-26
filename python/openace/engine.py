"""OpenACE Engine - high-level Python SDK."""

from __future__ import annotations

import os
import re
import uuid
from pathlib import Path
from typing import TYPE_CHECKING, Optional

import structlog

from openace.exceptions import IndexingError, OpenACEError, SearchError
from openace.logging import get_logger
from openace.types import CallChainNode, ChunkInfo, FunctionContext, IncrementalIndexReport, IndexReport, SearchResult, Symbol

logger = get_logger(__name__)

# Path segments that indicate a test file
_TEST_MARKERS = {"test", "tests", "test_", "_test", "spec", "specs", "__tests__"}


# Symbol kinds that are more specific and actionable for search.
_SPECIFIC_KINDS = {"function", "method"}

# Generic dunder methods that provide less context than named methods.
_GENERIC_NAMES = {"__call__", "__init__", "__new__", "__enter__", "__exit__"}

# Low-value symbol kinds that should be demoted in search ranking.
# These rarely answer "how does X work?" questions on their own.
_LOW_VALUE_KINDS = {"constant", "variable", "module"}

# Module init files are usually re-exports, rarely implementation.
_INIT_FILE = "__init__.py"

# Maximum number of extracted identifiers to pass to exact match.
_MAX_IDENTIFIERS = 20

# Precompiled patterns for identifier extraction.
_RE_CAMELCASE = re.compile(r'\b([A-Z][a-z]+(?:[A-Z][a-zA-Z0-9]*)+)\b')
_RE_ACRONYM_CAMELCASE = re.compile(r'\b([A-Z]{2,}[a-z][a-zA-Z0-9]*)\b')
_RE_SNAKE_CASE = re.compile(r'\b([a-z][a-z0-9]*(?:_[a-z0-9]+)+)\b')
_RE_DOTTED = re.compile(r'\b([\w]+(?:\.[\w]+)+)\b')
_RE_FILE_PATH = re.compile(r'([\w/\\]+\.(?:py|js|ts|rs|go|java))\b')
_RE_ALL_CAPS = re.compile(r'\b([A-Z][A-Z0-9]*(?:_[A-Z0-9]+)+)\b')
# Single PascalCase word with digits (e.g., L031, Rule4) — at least one letter
_RE_PASCAL_SINGLE = re.compile(r'\b([A-Z][a-zA-Z0-9]*\d[a-zA-Z0-9]*)\b')


def _extract_identifiers(text: str) -> list[str]:
    """Extract code identifiers from a natural-language query.

    Returns a deduplicated list of identifiers (capped at _MAX_IDENTIFIERS)
    suitable for exact-match symbol lookup. Zero-cost on pure natural
    language input that contains no code references.
    """
    seen: set[str] = set()
    result: list[str] = []

    def _add(ident: str) -> None:
        if ident and ident not in seen and len(result) < _MAX_IDENTIFIERS:
            seen.add(ident)
            result.append(ident)

    # CamelCase with at least one internal uppercase: DataProcessor, RuleValidator
    for m in _RE_CAMELCASE.findall(text):
        _add(m)

    # Acronym-prefixed CamelCase: HTMLParser, XMLReader, JSONParser
    for m in _RE_ACRONYM_CAMELCASE.findall(text):
        _add(m)

    # PascalCase with digits: L031, Rule4
    for m in _RE_PASCAL_SINGLE.findall(text):
        _add(m)

    # snake_case: process_data, validate_input (require at least one underscore)
    for m in _RE_SNAKE_CASE.findall(text):
        _add(m)

    # Dotted references: module.Class.method — also extract last component
    for m in _RE_DOTTED.findall(text):
        _add(m)
        last = m.rsplit(".", 1)[-1]
        _add(last)

    # File path stems: L031 from L031.py, handler from handler.py
    for m in _RE_FILE_PATH.findall(text):
        stem = Path(m).stem
        _add(stem)

    # ALL_CAPS constants: MAX_RETRIES, DEFAULT_TIMEOUT
    for m in _RE_ALL_CAPS.findall(text):
        _add(m)

    return result


def _is_test_file(file_path: str) -> bool:
    """Heuristic: return True if the file path looks like a test file."""
    parts = file_path.replace("\\", "/").lower().split("/")
    for part in parts:
        if part in _TEST_MARKERS or part.startswith("test_") or part.endswith("_test.py"):
            return True
    return False

if TYPE_CHECKING:
    from openace.embedding.protocol import EmbeddingProvider
    from openace.query_expansion import QueryExpander
    from openace.reranking.protocol import Reranker
    from openace.signal_weighting import SignalWeighter


def _convert_symbol(py_sym) -> Symbol:
    """Convert a PySymbol from the Rust extension to a Python Symbol."""
    return Symbol(
        id=py_sym.id,
        name=py_sym.name,
        qualified_name=py_sym.qualified_name,
        kind=py_sym.kind,
        language=py_sym.language,
        file_path=py_sym.file_path,
        line_start=py_sym.line_start,
        line_end=py_sym.line_end,
        signature=py_sym.signature,
        doc_comment=py_sym.doc_comment,
    )


def _convert_search_result(py_result) -> SearchResult:
    """Convert a PySearchResult from the Rust extension to a Python SearchResult."""
    chunk_info = None
    if hasattr(py_result, 'chunk_info') and py_result.chunk_info is not None:
        ci = py_result.chunk_info
        chunk_info = ChunkInfo(
            context_path=ci.context_path,
            chunk_score=ci.chunk_score,
        )
    return SearchResult(
        symbol_id=py_result.symbol_id,
        name=py_result.name,
        qualified_name=py_result.qualified_name,
        kind=py_result.kind,
        file_path=py_result.file_path,
        line_range=py_result.line_range,
        score=py_result.score,
        match_signals=list(py_result.match_signals),
        related_symbols=[_convert_search_result(r) for r in py_result.related_symbols],
        snippet=py_result.snippet,
        chunk_info=chunk_info,
    )


def _convert_index_report(py_report) -> IndexReport:
    """Convert a PyIndexReport from the Rust extension."""
    return IndexReport(
        total_files_scanned=py_report.total_files_scanned,
        files_indexed=py_report.files_indexed,
        files_skipped=py_report.files_skipped,
        files_failed=py_report.files_failed,
        total_symbols=py_report.total_symbols,
        total_relations=py_report.total_relations,
        duration_secs=py_report.duration_secs,
        total_chunks=getattr(py_report, 'total_chunks', 0),
        relations_resolved=getattr(py_report, 'relations_resolved', 0),
        relations_unresolved=getattr(py_report, 'relations_unresolved', 0),
    )


def _convert_call_chain_node(py_node) -> CallChainNode:
    """Convert a PyCallChainNode from the Rust extension to a Python CallChainNode."""
    return CallChainNode(
        symbol_id=py_node.symbol_id,
        name=py_node.name,
        qualified_name=py_node.qualified_name,
        kind=py_node.kind,
        file_path=py_node.file_path,
        line_range=py_node.line_range,
        depth=py_node.depth,
        signature=py_node.signature,
        doc_comment=py_node.doc_comment,
        snippet=py_node.snippet,
    )


def _convert_function_context(py_ctx) -> FunctionContext:
    """Convert a PyFunctionContext from the Rust extension to a Python FunctionContext."""
    return FunctionContext(
        symbol=_convert_call_chain_node(py_ctx.symbol),
        callers=[_convert_call_chain_node(n) for n in py_ctx.callers],
        callees=[_convert_call_chain_node(n) for n in py_ctx.callees],
        hierarchy=[_convert_call_chain_node(n) for n in py_ctx.hierarchy],
    )


class Engine:
    """OpenACE Contextual Code Engine.

    High-level interface combining Rust-powered indexing/search with
    optional Python embedding providers.

    Usage:
        from openace import Engine
        engine = Engine("/path/to/project")
        report = engine.index()
        results = engine.search("parse XML")
    """

    def __init__(
        self,
        project_root: str | os.PathLike,
        *,
        embedding_provider: Optional[EmbeddingProvider] = None,
        embedding_dim: Optional[int] = None,
        reranker: Optional[Reranker] = None,
        rerank_pool_size: int = 50,
        query_expander: Optional[QueryExpander] = None,
        signal_weighter: Optional[SignalWeighter] = None,
        chunk_enabled: bool = False,
        summary_enabled: bool = False,
    ):
        """Initialize the engine.

        Args:
            project_root: Path to the project directory.
            embedding_provider: Optional embedding provider for vector search.
            embedding_dim: Dimension of embedding vectors. If None, auto-detected
                from existing index metadata or defaults to 384.
            reranker: Optional reranker for two-stage search.
            rerank_pool_size: Number of candidates to retrieve before reranking.
            query_expander: Optional query expander for improved recall.
            signal_weighter: Optional signal weighter for adaptive RRF fusion.
            chunk_enabled: Enable AST chunk-level indexing and search.
            summary_enabled: Enable file-level summary indexing.
        """
        from openace._openace import EngineBinding

        self._project_root = str(Path(project_root).resolve())
        self._embedding_provider = embedding_provider
        self._embedding_dim = embedding_dim
        self._reranker = reranker
        self._rerank_pool_size = min(rerank_pool_size, 200)
        self._query_expander = query_expander
        self._signal_weighter = signal_weighter
        self._chunk_enabled = chunk_enabled
        self._summary_enabled = summary_enabled
        if summary_enabled:
            self._chunk_enabled = True
        if reranker is not None and rerank_pool_size > 200:
            logger.warning(
                "rerank_pool_size=%d exceeds Rust upper bound of 200, capped to 200",
                rerank_pool_size,
            )
        self._core = EngineBinding(self._project_root, embedding_dim)

    @property
    def project_root(self) -> str:
        """The absolute path to the project root."""
        return self._project_root

    def index(self, *, incremental: bool = True, force_full: bool = False,
              trace_id: Optional[str] = None) -> IndexReport:
        """Run indexing on the project.

        Args:
            incremental: If True (default), use incremental indexing which
                only re-parses and re-embeds changed files.
            force_full: If True, always do a full reindex (clears all data).
            trace_id: Optional trace ID for correlation.

        Returns:
            IndexReport with statistics about the indexing run.
        """
        trace_id = trace_id or uuid.uuid4().hex[:16]
        with structlog.contextvars.bound_contextvars(trace_id=trace_id):
            if force_full or not incremental:
                return self._index_full(trace_id=trace_id)
            else:
                return self._index_incremental(trace_id=trace_id)

    def _index_full(self, *, trace_id: str) -> IndexReport:
        """Run full indexing pipeline (clears all data first)."""
        try:
            py_report = self._core.index_full(
                self._project_root, self._chunk_enabled, trace_id=trace_id
            )
            report = _convert_index_report(py_report)
        except Exception as e:
            raise IndexingError(f"indexing failed: {e}") from e

        # Auto-embed if provider is set
        if self._embedding_provider is not None:
            self.embed_all(trace_id=trace_id)

        # Generate file-level summaries
        self._generate_summaries()

        return report

    def _index_incremental(self, *, trace_id: str) -> IndexReport:
        """Run incremental indexing pipeline (only process changed files)."""
        try:
            py_result = self._core.index_incremental(
                self._project_root, self._chunk_enabled, trace_id=trace_id
            )
        except Exception as e:
            raise IndexingError(f"incremental indexing failed: {e}") from e

        report = IndexReport(
            total_files_scanned=py_result.total_files_scanned,
            files_indexed=py_result.files_indexed,
            files_skipped=py_result.files_skipped,
            files_failed=py_result.files_failed,
            total_symbols=py_result.total_symbols,
            total_relations=py_result.total_relations,
            duration_secs=py_result.duration_secs,
            total_chunks=py_result.total_chunks,
            relations_resolved=getattr(py_result, 'relations_resolved', 0),
            relations_unresolved=getattr(py_result, 'relations_unresolved', 0),
        )

        # Selective embedding based on what changed
        if self._embedding_provider is not None:
            if py_result.fell_back_to_full:
                logger.info("first run, embedding all symbols")
                self.embed_all(trace_id=trace_id)
            elif py_result.changed_symbol_ids:
                changed_count = len(py_result.changed_symbol_ids)
                logger.info("embedding changed symbols", count=changed_count)
                self._embed_changed(
                    list(py_result.changed_symbol_ids), trace_id=trace_id
                )
            else:
                logger.info("no symbols changed, skipping embedding")

        # Generate file-level summaries
        self._generate_summaries()

        return report

    def _generate_summaries(self) -> None:
        """Generate file-level summaries if enabled."""
        if not self._summary_enabled:
            return
        try:
            from openace.summary import RuleBasedSummaryGenerator, generate_file_summaries
            generator = RuleBasedSummaryGenerator()
            summary_count = generate_file_summaries(self._core, generator)
            logger.info("generated file summaries", count=summary_count)
        except Exception as e:
            logger.warning(
                "summary generation failed",
                exception_type=type(e).__name__,
                error=str(e),
            )

    def _embed_changed(
        self, symbol_ids: list[str], *, trace_id: Optional[str] = None
    ) -> int:
        """Embed only the specified symbols by ID.

        Args:
            symbol_ids: List of hex symbol ID strings to embed.
            trace_id: Optional trace ID for correlation.

        Returns:
            Number of symbols embedded.
        """
        if self._embedding_provider is None:
            return 0

        batch_size = 100
        total = 0
        for i in range(0, len(symbol_ids), batch_size):
            batch_ids = symbol_ids[i : i + batch_size]
            symbols = self._core.get_symbols_by_ids(batch_ids)
            if not symbols:
                continue

            texts = []
            ids = []
            for sym in symbols:
                parts = [sym.qualified_name]
                if sym.signature:
                    parts.append(sym.signature)
                if sym.doc_comment:
                    parts.append(sym.doc_comment)
                if sym.body_text:
                    parts.append(sym.body_text[:32768])
                texts.append(" ".join(parts))
                ids.append(sym.id)

            vectors = self._embedding_provider.embed(texts)
            vector_lists = [v.tolist() for v in vectors]
            self._core.add_vectors(ids, vector_lists)
            total += len(symbols)

        self._core.flush()
        logger.info("embedded changed symbols", total=total)
        return total

    def _validate_path(self, path: str) -> None:
        """Validate that a relative path stays within the project root."""
        resolved = (Path(self._project_root) / path).resolve()
        if not str(resolved).startswith(self._project_root):
            raise SearchError(f"path outside project root: {path}")

    def search(
        self,
        query: str,
        *,
        limit: int = 10,
        language: Optional[str] = None,
        file_path: Optional[str] = None,
        dedupe_by_file: bool = True,
        trace_id: Optional[str] = None,
    ) -> list[SearchResult]:
        """Search for symbols using multi-signal retrieval.

        Args:
            query: Search text.
            limit: Maximum number of results.
            language: Optional language filter (e.g., "python").
            file_path: Optional file path prefix filter.
            dedupe_by_file: If True, keep only the highest-scoring symbol
                per file so that results cover more distinct files.
            trace_id: Optional trace ID for correlation.

        Returns:
            List of SearchResult sorted by relevance score.
        """
        trace_id = trace_id or uuid.uuid4().hex[:16]
        with structlog.contextvars.bound_contextvars(trace_id=trace_id):
            try:
                if file_path is not None:
                    self._validate_path(file_path)

                if limit <= 0:
                    return []

                # Stage 0: query expansion for better BM25 recall
                search_query = query
                if self._query_expander is not None:
                    try:
                        search_query = self._query_expander.expand(query)
                    except Exception as e:
                        logger.warning(
                            "query expansion failed, using original query",
                            exception_type=type(e).__name__,
                            error=str(e),
                        )

                query_vector = None
                if self._embedding_provider is not None:
                    vectors = self._embedding_provider.embed([query])
                    query_vector = vectors[0].tolist()

                # Stage 0.1: extract identifiers from the ORIGINAL query
                # for exact-match and BM25 boosting.
                extracted_ids = _extract_identifiers(query)
                bm25_text = (
                    " ".join(extracted_ids) + " " + search_query
                    if extracted_ids
                    else None
                )
                exact_queries = extracted_ids if extracted_ids else None

                # Stage 0.5: signal weight generation
                from openace.signal_weighting import SignalWeights

                weights = SignalWeights()
                if self._signal_weighter is not None:
                    try:
                        weights = self._signal_weighter.compute_weights(query)
                    except Exception as e:
                        logger.warning(
                            "signal weighting failed, using defaults",
                            exception_type=type(e).__name__,
                            error=str(e),
                        )

                # Stage 1: retrieval with expanded pool
                # Expand when reranker or file-dedup is active so we have
                # enough candidates after filtering.
                if self._reranker is not None or dedupe_by_file:
                    retrieval_limit = max(limit * 5, self._rerank_pool_size)
                    retrieval_limit = min(retrieval_limit, 200)  # Rust upper bound
                else:
                    retrieval_limit = limit

                py_results = self._core.search(
                    search_query,
                    query_vector,
                    retrieval_limit,
                    language,
                    file_path,
                    self._chunk_enabled,
                    bm25_weight=weights.bm25,
                    vector_weight=weights.vector,
                    exact_weight=weights.exact,
                    chunk_bm25_weight=weights.chunk_bm25,
                    graph_weight=weights.graph,
                    bm25_text=bm25_text,
                    exact_queries=exact_queries,
                    trace_id=trace_id,
                )
                results = [_convert_search_result(r) for r in py_results]
                retrieval_count = len(results)

                # Stage 2: rerank if reranker is configured
                if self._reranker is not None:
                    try:
                        results = self._reranker.rerank(
                            query, results, top_k=retrieval_limit,
                        )
                    except Exception as e:
                        logger.warning(
                            "reranker failed, falling back to original ranking",
                            exception_type=type(e).__name__,
                            error=str(e),
                        )

                # Stage 3: file-level dedup — keep best symbol per file.
                # Prefer methods/functions over classes/modules/constants since
                # they are more specific and actionable for understanding code.
                if dedupe_by_file:
                    best_per_file: dict[str, SearchResult] = {}
                    for r in results:
                        prev = best_per_file.get(r.file_path)
                        if prev is None:
                            best_per_file[r.file_path] = r
                        elif r.kind in _SPECIFIC_KINDS and prev.kind not in _SPECIFIC_KINDS:
                            # Replace generic symbol with a more specific one
                            best_per_file[r.file_path] = r
                        elif (
                            r.kind in _SPECIFIC_KINDS
                            and prev.name in _GENERIC_NAMES
                            and r.name not in _GENERIC_NAMES
                        ):
                            # Prefer named methods over dunder methods
                            best_per_file[r.file_path] = r

                    # Separate into tiers:
                    #   1. source files (direct + graph-expanded, sorted together)
                    #   2. test files
                    #   3. low-value kinds (constants, variables, modules)
                    # Graph-only results get a mild score penalty so direct
                    # matches rank higher, but they stay in the main tier
                    # to preserve domain breadth.
                    _GRAPH_ONLY = {"graph"}
                    source_results: list[SearchResult] = []
                    test_results: list[SearchResult] = []
                    lowval_results: list[SearchResult] = []
                    for r in best_per_file.values():
                        if r.kind in _LOW_VALUE_KINDS:
                            lowval_results.append(r)
                        elif r.file_path.endswith(_INIT_FILE):
                            lowval_results.append(r)
                        elif _is_test_file(r.file_path):
                            test_results.append(r)
                        else:
                            source_results.append(r)
                    # Sort key: prefer rerank_score (from reranker) over
                    # the raw RRF score so reranker judgements are respected.
                    # Graph-only results get a 30% penalty to rank below
                    # direct matches at similar scores.
                    def _sort_key(r: SearchResult) -> float:
                        base = r.rerank_score if r.rerank_score is not None else r.score
                        if set(r.match_signals) == _GRAPH_ONLY:
                            base *= 0.7
                        elif len(r.match_signals) == 1:
                            base *= 0.85  # demote single-signal noise
                        return base

                    source_results.sort(key=_sort_key, reverse=True)
                    test_results.sort(key=_sort_key, reverse=True)
                    lowval_results.sort(key=_sort_key, reverse=True)
                    results = (
                        source_results
                        + test_results
                        + lowval_results
                    )

                deduped_count = len(results)

                # Stage 4: score-gap cutoff — detect a significant score
                # drop between consecutive results and cut there.  This
                # trims the tail of weakly-matching noise.
                _MIN_RESULTS = min(limit, 5)
                if len(results) > _MIN_RESULTS:
                    def _eff_score(r: SearchResult) -> float:
                        return r.rerank_score if r.rerank_score is not None else r.score

                    cut_idx = len(results)
                    for idx in range(_MIN_RESULTS, len(results)):
                        prev = _eff_score(results[idx - 1])
                        cur = _eff_score(results[idx])
                        if prev > 0 and cur / prev < 0.6:
                            cut_idx = idx
                            break
                    results = results[:max(cut_idx, _MIN_RESULTS)]

                final_count = min(len(results), limit)
                logger.info(
                    "search pipeline done",
                    retrieval_count=retrieval_count,
                    deduped_count=deduped_count,
                    returned_count=final_count,
                )
                return results[:limit]
            except OpenACEError:
                raise
            except Exception as e:
                raise SearchError(f"search failed: {e}") from e

    def find_symbol(self, name: str, *, trace_id: Optional[str] = None) -> list[Symbol]:
        """Find symbols by exact name match.

        Args:
            name: Symbol name or qualified name to search for.
            trace_id: Optional trace ID for correlation.

        Returns:
            List of matching Symbol objects.
        """
        trace_id = trace_id or uuid.uuid4().hex[:16]
        with structlog.contextvars.bound_contextvars(trace_id=trace_id):
            try:
                py_syms = self._core.find_symbol(name, trace_id=trace_id)
                return [_convert_symbol(s) for s in py_syms]
            except Exception as e:
                raise SearchError(f"find_symbol failed: {e}") from e

    def get_file_outline(self, path: str, *, trace_id: Optional[str] = None) -> list[Symbol]:
        """Get all symbols defined in a file.

        Args:
            path: Relative file path within the project.
            trace_id: Optional trace ID for correlation.

        Returns:
            List of Symbol objects in the file.
        """
        trace_id = trace_id or uuid.uuid4().hex[:16]
        with structlog.contextvars.bound_contextvars(trace_id=trace_id):
            try:
                self._validate_path(path)
                py_syms = self._core.get_file_outline(path, trace_id=trace_id)
                return [_convert_symbol(s) for s in py_syms]
            except Exception as e:
                raise SearchError(f"get_file_outline failed: {e}") from e

    def get_function_context(
        self,
        symbol_id: str,
        *,
        max_depth: int = 3,
        max_fanout: int = 50,
        trace_id: Optional[str] = None,
    ) -> FunctionContext:
        """Get structured function context for a symbol.

        Returns callers, callees, and hierarchy (parent class/module + siblings)
        around the given symbol, obtained by traversing the code graph.

        Args:
            symbol_id: Hex symbol ID string.
            max_depth: Maximum traversal depth (default 3, max 5).
            max_fanout: Maximum neighbors per node (default 50, max 200).
            trace_id: Optional trace ID for correlation.

        Returns:
            FunctionContext with the root symbol and its callers, callees,
            and hierarchy neighbors.
        """
        trace_id = trace_id or uuid.uuid4().hex[:16]
        with structlog.contextvars.bound_contextvars(trace_id=trace_id):
            try:
                py_ctx = self._core.get_function_context(
                    symbol_id, max_depth, max_fanout, trace_id=trace_id
                )
                return _convert_function_context(py_ctx)
            except Exception as e:
                raise SearchError(f"get_function_context failed: {e}") from e

    def embed_all(self, *, trace_id: Optional[str] = None) -> int:
        """Compute and store embeddings for all indexed symbols.

        Uses adaptive concurrent embedding: multiple batches are sent
        in parallel via a ThreadPoolExecutor with AIMD-based concurrency
        control.  Batches are submitted dynamically (no pre-computed
        count), so this is safe during incremental indexing.

        Returns:
            Number of symbols embedded.
        """
        if self._embedding_provider is None:
            raise OpenACEError("no embedding provider configured")

        return self._concurrent_embed(self._embed_batch)

    def embed_all_chunks(self) -> int:
        """Compute and store embeddings for all indexed chunks.

        Uses adaptive concurrent embedding: multiple batches are sent
        in parallel via a ThreadPoolExecutor with AIMD-based concurrency
        control.  Batches are submitted dynamically (no pre-computed
        count), so this is safe during incremental indexing.

        Returns:
            Number of chunks embedded.
        """
        if self._embedding_provider is None:
            raise OpenACEError("no embedding provider configured")

        return self._concurrent_embed(self._embed_chunk_batch)

    def _embed_batch(self, offset: int, limit: int) -> int:
        """Embed a single batch of symbols.

        Returns the number of symbols embedded in this batch.
        """
        symbols = self._core.list_symbols_for_embedding(limit, offset)
        if not symbols:
            return 0

        texts = []
        ids = []
        for sym in symbols:
            parts = [sym.qualified_name]
            if sym.signature:
                parts.append(sym.signature)
            if sym.doc_comment:
                parts.append(sym.doc_comment)
            if sym.body_text:
                parts.append(sym.body_text[:32768])
            texts.append(" ".join(parts))
            ids.append(sym.id)

        vectors = self._embedding_provider.embed(texts)
        vector_lists = [v.tolist() for v in vectors]
        self._core.add_vectors(ids, vector_lists)
        return len(symbols)

    def _embed_chunk_batch(self, offset: int, limit: int) -> int:
        """Embed a single batch of chunks.

        Returns the number of chunks embedded in this batch.
        """
        chunks = self._core.list_chunks_for_embedding(limit, offset)
        if not chunks:
            return 0

        texts = []
        ids = []
        for chunk in chunks:
            text = f"file: {chunk.file_path}\ncontext: {chunk.context_path}\n{chunk.content[:32768]}"
            texts.append(text)
            ids.append(chunk.id)

        vectors = self._embedding_provider.embed(texts)
        vector_lists = [v.tolist() for v in vectors]
        self._core.add_vectors(ids, vector_lists)
        return len(chunks)

    def _concurrent_embed(
        self,
        batch_fn,
    ) -> int:
        """Run embedding batches concurrently with adaptive concurrency.

        Batches are submitted dynamically with increasing offsets.
        When a batch returns 0 items, no further batches are submitted.
        This makes embedding safe during incremental indexing — no
        pre-computed count that could go stale.

        Args:
            batch_fn: Callable(offset, limit) -> int that embeds one batch.

        Returns:
            Total number of items embedded.
        """
        from concurrent.futures import ThreadPoolExecutor, as_completed

        from openace.embedding.adaptive import make_strategy

        strategy = make_strategy(self._embedding_provider)
        batch_size = 100
        total_embedded = 0
        failed = 0
        next_offset = 0
        exhausted = False

        with ThreadPoolExecutor(max_workers=strategy.max_concurrency) as pool:
            futures: dict = {}

            # Initial fill
            for _ in range(strategy.current_concurrency()):
                fut = pool.submit(batch_fn, next_offset, batch_size)
                futures[fut] = next_offset
                next_offset += batch_size

            while futures:
                done = next(as_completed(futures))
                offset = futures.pop(done)

                try:
                    count = done.result()
                    total_embedded += count
                    strategy.record(True)
                    if count == 0:
                        exhausted = True
                except Exception:
                    failed += 1
                    strategy.record(False)
                    logger.warning(
                        "embedding batch failed", offset=offset, exc_info=True,
                    )

                # Refill up to current concurrency
                while not exhausted and len(futures) < strategy.current_concurrency():
                    fut = pool.submit(batch_fn, next_offset, batch_size)
                    futures[fut] = next_offset
                    next_offset += batch_size

        if failed:
            logger.warning("embedding completed with failures", failed_batches=failed)

        self._core.flush()
        return total_embedded

    def flush(self) -> None:
        """Persist all storage backends to disk."""
        self._core.flush()
