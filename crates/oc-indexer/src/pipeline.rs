use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::time::Instant;

use rayon::prelude::*;
use tracing;

use oc_core::{CodeChunk, CodeSymbol};
use oc_parser::{chunk_file, is_binary, parse_file_with_tree, ParserRegistry};
use oc_storage::graph::FileMetadata;
use oc_storage::manager::StorageManager;

use crate::error::IndexerError;
use crate::report::{IndexConfig, IndexReport, SkipReason};
use crate::scanner::scan_files;

/// Outcome of attempting to parse a single file.
enum FileOutcome {
    Parsed {
        rel_path: String,
        symbols: Vec<CodeSymbol>,
        relations: Vec<oc_core::CodeRelation>,
        chunks: Vec<CodeChunk>,
        content_hash: u64,
        file_size: u64,
        language: oc_core::Language,
        source_bytes: Vec<u8>,
    },
    Skipped(SkipReason),
    Failed(String, String),
}

/// Run a full indexing pipeline on a project directory.
///
/// Pipeline: scan → filter → parallel parse (rayon) → sequential store → Tantivy index.
///
/// Returns an `IndexReport` with statistics about the indexing run.
#[tracing::instrument(skip(config))]
pub fn index(project_path: &Path, config: &IndexConfig) -> Result<IndexReport, IndexerError> {
    let start = Instant::now();

    // 1. Scan for files
    let scan_result = scan_files(project_path);
    let total_files_scanned = scan_result.files.len();
    tracing::info!(files = total_files_scanned, "index started");

    // 2. Open storage
    let mut storage = StorageManager::open_with_dimension(project_path, config.embedding_dim)?;

    // 3. Clear existing data for a clean full reindex.
    // This prevents ghost entries from deleted files and Tantivy index bloat.
    storage
        .graph_mut()
        .clear()
        .map_err(|e| IndexerError::PipelineFailed {
            stage: "clear_graph".to_string(),
            reason: e.to_string(),
        })?;
    storage
        .fulltext_mut()
        .clear()
        .map_err(|e| IndexerError::PipelineFailed {
            stage: "clear_fulltext".to_string(),
            reason: e.to_string(),
        })?;

    // 4. Parallel parse
    let chunk_enabled = config.chunk_enabled;
    let chunk_config = config.chunk_config.clone();
    let parent_span = tracing::Span::current();
    let outcomes: Vec<FileOutcome> = scan_result
        .files
        .par_iter()
        .map(|rel_path| {
            let _guard = tracing::debug_span!(parent: &parent_span, "parse_file", path = %rel_path.display()).entered();
            let rel_str = normalize_path(rel_path);
            let abs_path = project_path.join(rel_path);

            // Read file metadata
            let metadata = match fs::metadata(&abs_path) {
                Ok(m) => m,
                Err(e) => return FileOutcome::Failed(rel_str, e.to_string()),
            };

            let file_size = metadata.len();

            // Size check
            if file_size > 1_048_576 {
                return FileOutcome::Skipped(SkipReason::TooLarge);
            }

            // Read file content
            let content = match fs::read(&abs_path) {
                Ok(c) => c,
                Err(e) => return FileOutcome::Failed(rel_str, e.to_string()),
            };

            // Binary check
            if is_binary(&content) {
                return FileOutcome::Skipped(SkipReason::Binary);
            }

            // Check language support
            let lang = match ParserRegistry::language_for_extension(
                &extension_from_path(rel_path),
            ) {
                Some(l) => l,
                None => return FileOutcome::Skipped(SkipReason::UnsupportedLanguage),
            };

            // Parse (with tree for optional chunking)
            match parse_file_with_tree(&config.repo_id, &rel_str, &content, file_size) {
                Ok(result) => {
                    let content_hash = xxhash_rust::xxh3::xxh3_64(&content);

                    // Conditionally chunk
                    let chunks = if chunk_enabled {
                        chunk_file(
                            &config.repo_id,
                            &rel_str,
                            &result.source,
                            &result.tree,
                            result.language,
                            &chunk_config,
                        )
                    } else {
                        Vec::new()
                    };

                    FileOutcome::Parsed {
                        rel_path: rel_str,
                        symbols: result.output.symbols,
                        relations: result.output.relations,
                        chunks,
                        content_hash,
                        file_size,
                        language: lang,
                        source_bytes: content,
                    }
                }
                Err(e) => {
                    use oc_parser::error::ParserError;
                    match &e {
                        ParserError::FileTooLarge { .. } => FileOutcome::Skipped(SkipReason::TooLarge),
                        ParserError::InvalidEncoding { .. } => FileOutcome::Skipped(SkipReason::Binary),
                        ParserError::UnsupportedLanguage { .. } => {
                            FileOutcome::Skipped(SkipReason::UnsupportedLanguage)
                        }
                        ParserError::ParseFailed { .. } => FileOutcome::Failed(rel_str, e.to_string()),
                    }
                }
            }
        })
        .collect();

    // 5. Sequential store
    let mut files_indexed = 0usize;
    let mut files_skipped: HashMap<SkipReason, usize> = HashMap::new();
    let mut files_failed = 0usize;
    let mut failed_details: Vec<(String, String)> = Vec::new();
    let mut total_symbols = 0usize;
    let mut total_chunks = 0usize;

    let mut all_symbols: Vec<CodeSymbol> = Vec::new();
    let mut all_relations: Vec<oc_core::CodeRelation> = Vec::new();
    let mut all_body_map: HashMap<oc_core::SymbolId, String> = HashMap::new();
    let mut all_chunks: Vec<CodeChunk> = Vec::new();
    let mut file_metas: Vec<FileMetadata> = Vec::new();

    let now = chrono_like_now();

    for outcome in outcomes {
        match outcome {
            FileOutcome::Parsed {
                rel_path,
                symbols,
                relations,
                chunks,
                content_hash,
                file_size,
                language,
                source_bytes,
            } => {
                let sym_count = symbols.len();
                total_symbols += sym_count;
                total_chunks += chunks.len();
                files_indexed += 1;

                // Build body text map from source bytes and populate body_text field
                let mut symbols = symbols;
                for sym in &mut symbols {
                    let start = sym.byte_range.start;
                    let end = sym.byte_range.end.min(source_bytes.len());
                    if start < end {
                        let body = String::from_utf8_lossy(&source_bytes[start..end]);
                        let capped = oc_core::truncate_utf8_bytes(&body, 10240);
                        let capped_str = capped.to_string();
                        sym.body_text = Some(capped_str.clone());
                        all_body_map.insert(sym.id, capped_str);
                    }
                }

                file_metas.push(FileMetadata {
                    path: rel_path,
                    content_hash,
                    language,
                    size_bytes: file_size,
                    symbol_count: sym_count as u32,
                    last_indexed: now.clone(),
                    last_modified: now.clone(),
                });

                all_symbols.extend(symbols);
                all_relations.extend(relations);
                all_chunks.extend(chunks);
            }
            FileOutcome::Skipped(reason) => {
                *files_skipped.entry(reason).or_insert(0) += 1;
            }
            FileOutcome::Failed(path, reason) => {
                files_failed += 1;
                failed_details.push((path, reason));
            }
        }
    }

    // Build set of known symbol IDs for relation filtering
    let known_ids: HashSet<oc_core::SymbolId> = all_symbols.iter().map(|s| s.id).collect();

    // Filter relations to only those whose source is a known symbol.
    // Target may be unresolved (cross-file or external) — we allow dangling
    // target references since the FK constraint on target_id has been removed.
    let valid_relations: Vec<_> = all_relations
        .iter()
        .filter(|r| known_ids.contains(&r.source_id))
        .collect();
    let valid_relation_count = valid_relations.len();

    // Batch insert symbols into SQLite
    if !all_symbols.is_empty() {
        storage
            .graph_mut()
            .insert_symbols(&all_symbols, config.batch_size)
            .map_err(|e| IndexerError::PipelineFailed {
                stage: "store_symbols".to_string(),
                reason: e.to_string(),
            })?;
    }

    // Batch insert relations into SQLite (only valid ones)
    if !valid_relations.is_empty() {
        // Clone into owned vec for the insert API
        let owned: Vec<oc_core::CodeRelation> = valid_relations.into_iter().cloned().collect();
        storage
            .graph_mut()
            .insert_relations(&owned, config.batch_size)
            .map_err(|e| IndexerError::PipelineFailed {
                stage: "store_relations".to_string(),
                reason: e.to_string(),
            })?;
    }

    // Insert file metadata
    for meta in &file_metas {
        storage
            .graph_mut()
            .upsert_file(meta)
            .map_err(|e| IndexerError::PipelineFailed {
                stage: "store_file_metadata".to_string(),
                reason: e.to_string(),
            })?;
    }

    // Index symbols into Tantivy fulltext
    for sym in &all_symbols {
        let body = all_body_map.get(&sym.id).map(|s| s.as_str());
        storage
            .fulltext_mut()
            .add_document(sym, body)
            .map_err(|e| IndexerError::PipelineFailed {
                stage: "fulltext_index".to_string(),
                reason: e.to_string(),
            })?;
    }

    // Index chunks (if enabled)
    if config.chunk_enabled && !all_chunks.is_empty() {
        // Batch insert chunks into SQLite
        storage
            .graph_mut()
            .insert_chunks(&all_chunks, config.batch_size)
            .map_err(|e| IndexerError::PipelineFailed {
                stage: "store_chunks".to_string(),
                reason: e.to_string(),
            })?;

        // Index chunks into Tantivy fulltext
        for chunk in &all_chunks {
            if let Err(e) = storage.fulltext_mut().add_chunk_document(chunk) {
                tracing::warn!(error = %e, "chunk fulltext index failed");
            }
        }
    }

    // Commit Tantivy and flush
    storage.flush().map_err(|e| IndexerError::PipelineFailed {
        stage: "flush".to_string(),
        reason: e.to_string(),
    })?;

    let duration = start.elapsed();

    tracing::info!(
        files = files_indexed,
        symbols = total_symbols,
        duration_secs = %format!("{:.2}", duration.as_secs_f64()),
        "index completed"
    );

    Ok(IndexReport {
        total_files_scanned,
        files_indexed,
        files_skipped,
        files_failed,
        failed_details,
        total_symbols,
        total_relations: valid_relation_count,
        total_chunks,
        duration,
    })
}

/// Extract file extension from a path.
fn extension_from_path(p: &Path) -> String {
    p.extension()
        .map(|e| e.to_string_lossy().to_string())
        .unwrap_or_default()
}

/// Normalize a path to forward-slash format.
fn normalize_path(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

/// Simple timestamp string (RFC 3339-ish) without pulling in chrono.
fn chrono_like_now() -> String {
    use std::time::SystemTime;
    let dur = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}Z", dur.as_secs())
}
