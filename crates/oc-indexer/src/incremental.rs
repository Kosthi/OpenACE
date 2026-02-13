use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use oc_core::{CodeRelation, CodeSymbol, SymbolId};
use oc_parser::{chunk_file, is_binary, parse_file_with_tree, ChunkConfig, ParserRegistry};
use oc_storage::graph::FileMetadata;
use oc_storage::manager::StorageManager;

use crate::error::IndexerError;
use crate::watcher::{should_reindex, ChangeEvent};

/// Batch size for incremental SQLite operations (100 rows/tx per spec).
const INCREMENTAL_BATCH_SIZE: usize = 100;

/// Categories resulting from diffing old vs new symbol sets.
#[derive(Debug)]
pub struct SymbolDiff {
    /// Symbols that exist in new but not in old.
    pub added: Vec<CodeSymbol>,
    /// Symbol IDs that exist in old but not in new.
    pub removed: Vec<SymbolId>,
    /// Symbols that exist in both but have different body_hash.
    pub modified: Vec<CodeSymbol>,
    /// Count of symbols unchanged (same ID and body_hash).
    pub unchanged_count: usize,
}

/// Compute the diff between old symbols (from storage) and new symbols (from parser).
///
/// Classification uses deterministic symbol IDs:
/// - Added: ID in new but not old → INSERT
/// - Removed: ID in old but not new → DELETE
/// - Modified: ID in both but body_hash differs → UPDATE
/// - Unchanged: ID in both with same body_hash → SKIP
pub fn diff_symbols(old_symbols: &[CodeSymbol], new_symbols: &[CodeSymbol]) -> SymbolDiff {
    let old_map: HashMap<SymbolId, u64> = old_symbols
        .iter()
        .map(|s| (s.id, s.body_hash))
        .collect();

    let new_map: HashMap<SymbolId, &CodeSymbol> = new_symbols
        .iter()
        .map(|s| (s.id, s))
        .collect();

    let old_ids: HashSet<SymbolId> = old_map.keys().copied().collect();
    let new_ids: HashSet<SymbolId> = new_map.keys().copied().collect();

    let added: Vec<CodeSymbol> = new_ids
        .difference(&old_ids)
        .map(|id| (*new_map[id]).clone())
        .collect();

    let removed: Vec<SymbolId> = old_ids.difference(&new_ids).copied().collect();

    let mut modified = Vec::new();
    let mut unchanged_count = 0usize;

    for id in old_ids.intersection(&new_ids) {
        let old_hash = old_map[id];
        let new_sym = new_map[id];
        if new_sym.body_hash != old_hash {
            modified.push((*new_sym).clone());
        } else {
            unchanged_count += 1;
        }
    }

    SymbolDiff {
        added,
        removed,
        modified,
        unchanged_count,
    }
}

/// Report for a single incremental file update.
#[derive(Debug)]
pub struct IncrementalReport {
    pub file_path: String,
    pub added: usize,
    pub removed: usize,
    pub modified: usize,
    pub unchanged: usize,
    pub skipped_unchanged_hash: bool,
}

/// Process a single file change incrementally.
///
/// Pipeline: hash check → re-parse → diff → SQLite update (100 rows/tx)
/// → Tantivy update → files table update.
///
/// SQLite is committed first; Tantivy updates happen only after SQLite succeeds.
///
/// If `chunk_config` is Some, chunks will be re-indexed for the file.
pub fn update_file(
    project_path: &Path,
    rel_path: &str,
    repo_id: &str,
    storage: &mut StorageManager,
    chunk_config: Option<&ChunkConfig>,
) -> Result<IncrementalReport, IndexerError> {
    let abs_path = project_path.join(rel_path);

    // Validate that the resolved path stays within the project root.
    // If the file doesn't exist, canonicalize will fail — fall through to the
    // fs::read below which handles NotFound by calling delete_file.
    match abs_path.canonicalize() {
        Ok(canonical) => {
            let canonical_root = project_path.canonicalize().map_err(IndexerError::Io)?;
            if !canonical.starts_with(&canonical_root) {
                return Err(IndexerError::PipelineFailed {
                    stage: "path_validation".into(),
                    reason: format!("path outside project root: {}", rel_path),
                });
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // File doesn't exist — let the read below handle it
        }
        Err(e) => {
            return Err(IndexerError::PipelineFailed {
                stage: "path_validation".into(),
                reason: format!("cannot canonicalize path: {e}"),
            });
        }
    }

    // Read the file; if it was deleted between event and processing, fall back to delete
    let content = match fs::read(&abs_path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return delete_file(rel_path, storage, chunk_config.is_some());
        }
        Err(e) => return Err(IndexerError::Io(e)),
    };
    let file_size = content.len() as u64;

    // Hash check: compare against stored hash
    if let Some(stored_meta) = storage.graph().get_file(rel_path)? {
        if !should_reindex(&content, stored_meta.content_hash) {
            return Ok(IncrementalReport {
                file_path: rel_path.to_string(),
                added: 0,
                removed: 0,
                modified: 0,
                unchanged: 0,
                skipped_unchanged_hash: true,
            });
        }
    }

    // Size check
    if file_size > 1_048_576 {
        return Err(IndexerError::PipelineFailed {
            stage: "incremental_size_check".to_string(),
            reason: format!("file too large: {file_size} bytes"),
        });
    }

    // Binary check
    if is_binary(&content) {
        return Err(IndexerError::PipelineFailed {
            stage: "incremental_binary_check".to_string(),
            reason: "file is binary".to_string(),
        });
    }

    // Determine language
    let ext = Path::new(rel_path)
        .extension()
        .map(|e| e.to_string_lossy().to_string())
        .unwrap_or_default();
    let language = ParserRegistry::language_for_extension(&ext).ok_or_else(|| {
        IndexerError::PipelineFailed {
            stage: "incremental_language_check".to_string(),
            reason: format!("unsupported extension: {ext}"),
        }
    })?;

    // Re-parse
    let parse_result = parse_file_with_tree(repo_id, rel_path, &content, file_size)?;
    let mut new_symbols = parse_result.output.symbols;
    let new_relations = parse_result.output.relations;
    let content_hash = xxhash_rust::xxh3::xxh3_64(&content);

    // Build body text map from source bytes for fulltext indexing,
    // and populate body_text field on each symbol for storage.
    let body_map: HashMap<SymbolId, String> = new_symbols
        .iter_mut()
        .filter_map(|sym| {
            let start = sym.byte_range.start;
            let end = sym.byte_range.end.min(content.len());
            if start < end {
                let body = String::from_utf8_lossy(&content[start..end]);
                let capped = oc_core::truncate_utf8_bytes(&body, 10240);
                let capped_str = capped.to_string();
                sym.body_text = Some(capped_str.clone());
                Some((sym.id, capped_str))
            } else {
                None
            }
        })
        .collect();

    // Get old symbols from SQLite
    let old_symbols = storage.graph().get_symbols_by_file(rel_path)?;

    // Diff
    let diff = diff_symbols(&old_symbols, &new_symbols);

    let report = IncrementalReport {
        file_path: rel_path.to_string(),
        added: diff.added.len(),
        removed: diff.removed.len(),
        modified: diff.modified.len(),
        unchanged: diff.unchanged_count,
        skipped_unchanged_hash: false,
    };

    // Phase 1: SQLite updates (source of truth)
    // Delete removed symbols (CASCADE handles relations)
    for chunk in diff.removed.chunks(INCREMENTAL_BATCH_SIZE) {
        for id in chunk {
            storage.graph_mut().delete_symbol(*id)?;
        }
    }

    // Insert added symbols
    if !diff.added.is_empty() {
        storage
            .graph_mut()
            .insert_symbols(&diff.added, INCREMENTAL_BATCH_SIZE)?;
    }

    // Update modified symbols (UPDATE preserves cross-file FK relations)
    if !diff.modified.is_empty() {
        storage
            .graph_mut()
            .update_symbols(&diff.modified, INCREMENTAL_BATCH_SIZE)?;
    }

    // Delete old relations for this file and re-insert new ones.
    // This is simpler and correct: CASCADE already removed relations for deleted symbols,
    // but we also need to handle relation changes for modified symbols.
    // Delete remaining relations sourced from this file, then insert all new ones.
    delete_relations_for_file(storage, rel_path)?;

    // Validate and insert new relations.
    // Source must exist in the graph store (it was just inserted/updated for
    // this file). Target may be cross-file or unresolved — we allow dangling
    // target references since the FK constraint on target_id has been removed.
    let valid_relations: Vec<&CodeRelation> = new_relations
        .iter()
        .filter(|r| {
            storage.graph().get_symbol(r.source_id).ok().flatten().is_some()
        })
        .collect();

    if !valid_relations.is_empty() {
        let owned: Vec<CodeRelation> = valid_relations.into_iter().cloned().collect();
        storage
            .graph_mut()
            .insert_relations(&owned, INCREMENTAL_BATCH_SIZE)?;
    }

    // Update file metadata
    let now = chrono_like_now();
    storage.graph_mut().upsert_file(&FileMetadata {
        path: rel_path.to_string(),
        content_hash,
        language,
        size_bytes: file_size,
        symbol_count: new_symbols.len() as u32,
        last_indexed: now.clone(),
        last_modified: now,
    })?;

    // Phase 2: Tantivy updates (only after SQLite succeeds)
    // Delete old documents for removed and modified symbols
    for id in &diff.removed {
        storage.fulltext_mut().delete_document(*id)?;
    }
    for sym in &diff.modified {
        storage.fulltext_mut().delete_document(sym.id)?;
    }

    // Add new documents for added and modified symbols
    for sym in &diff.added {
        let body = body_map.get(&sym.id).map(|s| s.as_str());
        storage.fulltext_mut().add_document(sym, body)?;
    }
    for sym in &diff.modified {
        let body = body_map.get(&sym.id).map(|s| s.as_str());
        storage.fulltext_mut().add_document(sym, body)?;
    }

    // Phase 3: Chunk updates (only when chunk_config is provided)
    if let Some(cfg) = chunk_config {
        // Delete old chunks for this file
        let old_chunks = storage.graph().get_chunks_by_file(rel_path)?;
        for c in &old_chunks {
            let _ = storage.fulltext_mut().delete_chunk_document(c.id);
        }
        storage.graph_mut().delete_chunks_by_file(rel_path)?;

        // Re-chunk and insert
        let new_chunks = chunk_file(
            repo_id,
            rel_path,
            &parse_result.source,
            &parse_result.tree,
            parse_result.language,
            cfg,
        );
        if !new_chunks.is_empty() {
            storage
                .graph_mut()
                .insert_chunks(&new_chunks, INCREMENTAL_BATCH_SIZE)?;
            for chunk in &new_chunks {
                let _ = storage.fulltext_mut().add_chunk_document(chunk);
            }
        }
    }

    Ok(report)
}

/// Handle a file deletion: remove all symbols, relations, chunks, Tantivy docs, and file metadata.
///
/// Write ordering: SQLite first, then Tantivy.
pub fn delete_file(
    rel_path: &str,
    storage: &mut StorageManager,
    chunk_enabled: bool,
) -> Result<IncrementalReport, IndexerError> {
    // Get all symbols for this file before deleting
    let old_symbols = storage.graph().get_symbols_by_file(rel_path)?;
    let removed_count = old_symbols.len();

    // Phase 1: SQLite (source of truth)
    // Delete all symbols (CASCADE handles relations)
    storage.graph_mut().delete_symbols_by_file(rel_path)?;
    // Delete file metadata
    storage.graph_mut().delete_file(rel_path)?;

    // Delete chunks if enabled
    let old_chunks = if chunk_enabled {
        let chunks = storage.graph().get_chunks_by_file(rel_path)?;
        storage.graph_mut().delete_chunks_by_file(rel_path)?;
        chunks
    } else {
        Vec::new()
    };

    // Phase 2: Tantivy (only after SQLite succeeds)
    for sym in &old_symbols {
        storage.fulltext_mut().delete_document(sym.id)?;
    }
    for chunk in &old_chunks {
        let _ = storage.fulltext_mut().delete_chunk_document(chunk.id);
    }

    Ok(IncrementalReport {
        file_path: rel_path.to_string(),
        added: 0,
        removed: removed_count,
        modified: 0,
        unchanged: 0,
        skipped_unchanged_hash: false,
    })
}

/// Process a batch of change events from the watcher.
///
/// Each event is processed incrementally. Returns a report per file.
pub fn process_events(
    project_path: &Path,
    events: &[ChangeEvent],
    repo_id: &str,
    storage: &mut StorageManager,
    chunk_config: Option<&ChunkConfig>,
) -> Vec<Result<IncrementalReport, IndexerError>> {
    // Deduplicate events: keep only the latest event per path
    let mut latest: HashMap<String, &ChangeEvent> = HashMap::new();
    for event in events {
        let path = match event {
            ChangeEvent::Changed(p) | ChangeEvent::Removed(p) => {
                p.to_string_lossy().replace('\\', "/")
            }
        };
        latest.insert(path, event);
    }

    latest
        .into_iter()
        .map(|(path, event)| match event {
            ChangeEvent::Changed(_) => {
                update_file(project_path, &path, repo_id, storage, chunk_config)
            }
            ChangeEvent::Removed(_) => delete_file(&path, storage, chunk_config.is_some()),
        })
        .collect()
}

/// Delete all relations that originate from (are sourced in) a given file.
fn delete_relations_for_file(
    storage: &mut StorageManager,
    file_path: &str,
) -> Result<(), IndexerError> {
    storage
        .graph_mut()
        .delete_relations_by_file(file_path)
        .map_err(|e| IndexerError::PipelineFailed {
            stage: "delete_relations".to_string(),
            reason: e.to_string(),
        })?;
    Ok(())
}

fn chrono_like_now() -> String {
    use std::time::SystemTime;
    let dur = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = dur.as_secs();
    let days = secs / 86400;
    let time_of_day = secs % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;
    let (year, month, day) = days_to_ymd(days);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        year, month, day, hours, minutes, seconds
    )
}

fn days_to_ymd(mut days: u64) -> (u64, u64, u64) {
    let mut year = 1970u64;
    loop {
        let days_in_year = if is_leap(year) { 366 } else { 365 };
        if days < days_in_year {
            break;
        }
        days -= days_in_year;
        year += 1;
    }
    let leap = is_leap(year);
    let months: [u64; 12] = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut month = 1u64;
    for &m in &months {
        if days < m {
            break;
        }
        days -= m;
        month += 1;
    }
    (year, month, days + 1)
}

fn is_leap(y: u64) -> bool {
    y % 4 == 0 && (y % 100 != 0 || y % 400 == 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use oc_core::{Language, SymbolKind};
    use std::path::PathBuf;

    fn make_symbol(name: &str, file: &str, byte_start: usize, byte_end: usize, body_hash: u64) -> CodeSymbol {
        CodeSymbol {
            id: SymbolId::generate("test-repo", file, name, byte_start, byte_end),
            name: name.split('.').last().unwrap_or(name).to_string(),
            qualified_name: name.to_string(),
            kind: SymbolKind::Function,
            language: Language::Python,
            file_path: PathBuf::from(file),
            byte_range: byte_start..byte_end,
            line_range: 0..10,
            signature: Some(format!("def {}()", name)),
            doc_comment: None,
            body_hash,
            body_text: None,
        }
    }

    #[test]
    fn diff_detects_added_symbols() {
        let old = vec![];
        let new = vec![make_symbol("foo", "a.py", 0, 50, 100)];
        let diff = diff_symbols(&old, &new);
        assert_eq!(diff.added.len(), 1);
        assert_eq!(diff.removed.len(), 0);
        assert_eq!(diff.modified.len(), 0);
        assert_eq!(diff.unchanged_count, 0);
    }

    #[test]
    fn diff_detects_removed_symbols() {
        let old = vec![make_symbol("foo", "a.py", 0, 50, 100)];
        let new = vec![];
        let diff = diff_symbols(&old, &new);
        assert_eq!(diff.added.len(), 0);
        assert_eq!(diff.removed.len(), 1);
        assert_eq!(diff.modified.len(), 0);
        assert_eq!(diff.unchanged_count, 0);
    }

    #[test]
    fn diff_detects_modified_symbols() {
        let old = vec![make_symbol("foo", "a.py", 0, 50, 100)];
        let new = vec![make_symbol("foo", "a.py", 0, 50, 200)]; // different body_hash
        let diff = diff_symbols(&old, &new);
        assert_eq!(diff.added.len(), 0);
        assert_eq!(diff.removed.len(), 0);
        assert_eq!(diff.modified.len(), 1);
        assert_eq!(diff.unchanged_count, 0);
    }

    #[test]
    fn diff_detects_unchanged_symbols() {
        let old = vec![make_symbol("foo", "a.py", 0, 50, 100)];
        let new = vec![make_symbol("foo", "a.py", 0, 50, 100)]; // same body_hash
        let diff = diff_symbols(&old, &new);
        assert_eq!(diff.added.len(), 0);
        assert_eq!(diff.removed.len(), 0);
        assert_eq!(diff.modified.len(), 0);
        assert_eq!(diff.unchanged_count, 1);
    }

    #[test]
    fn diff_rename_shows_remove_and_add() {
        // Renaming foo→bar changes qualified_name, so SymbolId changes
        let old = vec![make_symbol("foo", "a.py", 0, 50, 100)];
        let new = vec![make_symbol("bar", "a.py", 0, 50, 100)];
        let diff = diff_symbols(&old, &new);
        assert_eq!(diff.added.len(), 1);
        assert_eq!(diff.removed.len(), 1);
        assert_eq!(diff.modified.len(), 0);
        assert_eq!(diff.unchanged_count, 0);
    }

    #[test]
    fn diff_mixed_changes() {
        let old = vec![
            make_symbol("a", "f.py", 0, 10, 1),
            make_symbol("b", "f.py", 20, 30, 2),
            make_symbol("c", "f.py", 40, 50, 3),
        ];
        let new = vec![
            make_symbol("a", "f.py", 0, 10, 1),   // unchanged
            make_symbol("b", "f.py", 20, 30, 99),  // modified (different body_hash)
            make_symbol("d", "f.py", 60, 70, 4),   // added (c removed, d added)
        ];
        let diff = diff_symbols(&old, &new);
        assert_eq!(diff.added.len(), 1);
        assert_eq!(diff.removed.len(), 1);
        assert_eq!(diff.modified.len(), 1);
        assert_eq!(diff.unchanged_count, 1);
    }
}
