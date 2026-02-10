use std::path::Path;

use oc_core::{CodeRelation, CodeSymbol, Language, RelationKind, SymbolId, SymbolKind};
use rusqlite::{params, Connection};
use xxhash_rust::xxh3::xxh3_128;

use crate::error::StorageError;

/// Current schema version. Increment when schema changes.
const SCHEMA_VERSION: u32 = 1;

/// Direction for graph traversal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraversalDirection {
    Outgoing,
    Incoming,
    Both,
}

/// A symbol discovered during k-hop traversal.
#[derive(Debug, Clone)]
pub struct TraversalHit {
    pub symbol_id: SymbolId,
    pub depth: u32,
    pub relation_kind: RelationKind,
}

/// File metadata stored in the `files` table.
#[derive(Debug, Clone)]
pub struct FileMetadata {
    pub path: String,
    pub content_hash: u64,
    pub language: Language,
    pub size_bytes: u64,
    pub symbol_count: u32,
    pub last_indexed: String,
    pub last_modified: String,
}

/// Repository metadata stored in the `repositories` table.
#[derive(Debug, Clone)]
pub struct RepoMetadata {
    pub id: String,
    pub path: String,
    pub name: String,
    pub created_at: String,
}

/// SQLite-backed graph storage for symbols and relations.
pub struct GraphStore {
    conn: Connection,
}

impl GraphStore {
    /// Open or create a graph store at the given SQLite database path.
    ///
    /// If the schema version doesn't match, returns `Err` so the caller
    /// can purge `.openace/` and retry.
    pub fn open(db_path: &Path) -> Result<Self, StorageError> {
        let conn = Connection::open(db_path)?;
        configure_pragmas(&conn)?;

        let stored_version = get_user_version(&conn)?;
        if stored_version != 0 && stored_version != SCHEMA_VERSION {
            return Err(StorageError::SchemaMismatch {
                expected: SCHEMA_VERSION,
                actual: stored_version,
            });
        }

        create_schema(&conn)?;
        set_user_version(&conn, SCHEMA_VERSION)?;

        Ok(Self { conn })
    }

    /// Open an in-memory graph store (for testing).
    pub fn open_in_memory() -> Result<Self, StorageError> {
        let conn = Connection::open_in_memory()?;
        configure_pragmas(&conn)?;
        create_schema(&conn)?;
        set_user_version(&conn, SCHEMA_VERSION)?;
        Ok(Self { conn })
    }

    // -- Symbol CRUD --

    /// Insert symbols in batched transactions.
    /// `batch_size`: 1000 for bulk, 100 for incremental.
    pub fn insert_symbols(
        &mut self,
        symbols: &[CodeSymbol],
        batch_size: usize,
    ) -> Result<(), StorageError> {
        let now = now_rfc3339();
        for chunk in symbols.chunks(batch_size) {
            let tx = self.conn.transaction()?;
            {
                let mut stmt = tx.prepare_cached(
                    "INSERT OR REPLACE INTO symbols \
                     (id, name, qualified_name, kind, language, file_path, \
                      line_start, line_end, byte_start, byte_end, \
                      signature, doc_comment, body_hash, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
                )?;
                for sym in chunk {
                    stmt.execute(params![
                        sym.id.as_bytes().as_slice(),
                        sym.name,
                        sym.qualified_name,
                        sym.kind.ordinal() as i64,
                        sym.language.ordinal() as i64,
                        sym.file_path.to_string_lossy().as_ref(),
                        sym.line_range.start as i64,
                        sym.line_range.end as i64,
                        sym.byte_range.start as i64,
                        sym.byte_range.end as i64,
                        sym.signature.as_deref(),
                        sym.doc_comment.as_deref(),
                        sym.body_hash as i64,
                        &now,
                        &now,
                    ])?;
                }
            }
            tx.commit()?;
        }
        Ok(())
    }

    /// Update existing symbols in batched transactions (no FK cascade).
    ///
    /// Unlike `insert_symbols` which uses `INSERT OR REPLACE` (triggering
    /// `ON DELETE CASCADE`), this uses `UPDATE` to preserve cross-file
    /// relations pointing to the modified symbols.
    pub fn update_symbols(
        &mut self,
        symbols: &[CodeSymbol],
        batch_size: usize,
    ) -> Result<(), StorageError> {
        let now = now_rfc3339();
        for chunk in symbols.chunks(batch_size) {
            let tx = self.conn.transaction()?;
            {
                let mut stmt = tx.prepare_cached(
                    "UPDATE symbols SET \
                     name = ?2, qualified_name = ?3, kind = ?4, language = ?5, \
                     file_path = ?6, line_start = ?7, line_end = ?8, \
                     byte_start = ?9, byte_end = ?10, \
                     signature = ?11, doc_comment = ?12, body_hash = ?13, updated_at = ?14 \
                     WHERE id = ?1",
                )?;
                for sym in chunk {
                    stmt.execute(params![
                        sym.id.as_bytes().as_slice(),
                        sym.name,
                        sym.qualified_name,
                        sym.kind.ordinal() as i64,
                        sym.language.ordinal() as i64,
                        sym.file_path.to_string_lossy().as_ref(),
                        sym.line_range.start as i64,
                        sym.line_range.end as i64,
                        sym.byte_range.start as i64,
                        sym.byte_range.end as i64,
                        sym.signature.as_deref(),
                        sym.doc_comment.as_deref(),
                        sym.body_hash as i64,
                        &now,
                    ])?;
                }
            }
            tx.commit()?;
        }
        Ok(())
    }

    /// Query a symbol by its ID.
    pub fn get_symbol(&self, id: SymbolId) -> Result<Option<CodeSymbol>, StorageError> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT id, name, qualified_name, kind, language, file_path, \
             line_start, line_end, byte_start, byte_end, \
             signature, doc_comment, body_hash \
             FROM symbols WHERE id = ?1",
        )?;
        let mut rows = stmt.query(params![id.as_bytes().as_slice()])?;
        match rows.next()? {
            Some(row) => Ok(Some(row_to_symbol(row)?)),
            None => Ok(None),
        }
    }

    /// Query all symbols for a given file path.
    pub fn get_symbols_by_file(&self, file_path: &str) -> Result<Vec<CodeSymbol>, StorageError> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT id, name, qualified_name, kind, language, file_path, \
             line_start, line_end, byte_start, byte_end, \
             signature, doc_comment, body_hash \
             FROM symbols WHERE file_path = ?1",
        )?;
        let mut rows = stmt.query(params![file_path])?;
        let mut results = Vec::new();
        while let Some(row) = rows.next()? {
            results.push(row_to_symbol(row)?);
        }
        Ok(results)
    }

    /// Query symbols by name (exact match).
    pub fn get_symbols_by_name(&self, name: &str) -> Result<Vec<CodeSymbol>, StorageError> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT id, name, qualified_name, kind, language, file_path, \
             line_start, line_end, byte_start, byte_end, \
             signature, doc_comment, body_hash \
             FROM symbols WHERE name = ?1",
        )?;
        let mut rows = stmt.query(params![name])?;
        let mut results = Vec::new();
        while let Some(row) = rows.next()? {
            results.push(row_to_symbol(row)?);
        }
        Ok(results)
    }

    /// Query symbols by qualified name (exact match).
    pub fn get_symbols_by_qualified_name(
        &self,
        qualified_name: &str,
    ) -> Result<Vec<CodeSymbol>, StorageError> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT id, name, qualified_name, kind, language, file_path, \
             line_start, line_end, byte_start, byte_end, \
             signature, doc_comment, body_hash \
             FROM symbols WHERE qualified_name = ?1",
        )?;
        let mut rows = stmt.query(params![qualified_name])?;
        let mut results = Vec::new();
        while let Some(row) = rows.next()? {
            results.push(row_to_symbol(row)?);
        }
        Ok(results)
    }

    /// List all symbols with pagination, ordered by ID for deterministic iteration.
    pub fn list_symbols(&self, limit: usize, offset: usize) -> Result<Vec<CodeSymbol>, StorageError> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT id, name, qualified_name, kind, language, file_path, \
             line_start, line_end, byte_start, byte_end, \
             signature, doc_comment, body_hash \
             FROM symbols ORDER BY id LIMIT ?1 OFFSET ?2",
        )?;
        let mut rows = stmt.query(params![limit as i64, offset as i64])?;
        let mut results = Vec::new();
        while let Some(row) = rows.next()? {
            results.push(row_to_symbol(row)?);
        }
        Ok(results)
    }

    /// Count total number of symbols in the store.
    pub fn count_symbols(&self) -> Result<usize, StorageError> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM symbols",
            [],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    /// Delete a symbol by ID. Relations are cascaded via ON DELETE CASCADE.
    pub fn delete_symbol(&mut self, id: SymbolId) -> Result<bool, StorageError> {
        let affected = self.conn.execute(
            "DELETE FROM symbols WHERE id = ?1",
            params![id.as_bytes().as_slice()],
        )?;
        Ok(affected > 0)
    }

    /// Delete all relations that reference a given file path (in the relation's file_path column).
    pub fn delete_relations_by_file(&mut self, file_path: &str) -> Result<usize, StorageError> {
        let affected = self.conn.execute(
            "DELETE FROM relations WHERE file_path = ?1",
            params![file_path],
        )?;
        Ok(affected)
    }

    /// Delete all symbols (and cascading relations) for a file path.
    pub fn delete_symbols_by_file(&mut self, file_path: &str) -> Result<usize, StorageError> {
        let affected = self.conn.execute(
            "DELETE FROM symbols WHERE file_path = ?1",
            params![file_path],
        )?;
        Ok(affected)
    }

    // -- Relation CRUD --

    /// Insert relations in batched transactions.
    pub fn insert_relations(
        &mut self,
        relations: &[CodeRelation],
        batch_size: usize,
    ) -> Result<(), StorageError> {
        for chunk in relations.chunks(batch_size) {
            let tx = self.conn.transaction()?;
            {
                let mut stmt = tx.prepare_cached(
                    "INSERT OR REPLACE INTO relations \
                     (id, source_id, target_id, kind, file_path, line, confidence) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                )?;
                for rel in chunk {
                    let rel_id = compute_relation_id(rel);
                    stmt.execute(params![
                        rel_id.as_slice(),
                        rel.source_id.as_bytes().as_slice(),
                        rel.target_id.as_bytes().as_slice(),
                        rel.kind.ordinal() as i64,
                        rel.file_path.to_string_lossy().as_ref(),
                        rel.line as i64,
                        rel.confidence as f64,
                    ])?;
                }
            }
            tx.commit()?;
        }
        Ok(())
    }

    /// K-hop graph traversal with cycle detection.
    ///
    /// Uses iterative BFS in Rust (not recursive CTE) for reliable cycle
    /// detection and per-node fanout limiting.
    pub fn traverse_khop(
        &self,
        start: SymbolId,
        max_depth: u32,
        max_fanout: u32,
        direction: TraversalDirection,
    ) -> Result<Vec<TraversalHit>, StorageError> {
        let max_depth = max_depth.min(5);
        let mut visited = std::collections::HashSet::new();
        visited.insert(start.0);

        let mut frontier: Vec<SymbolId> = vec![start];
        let mut results = Vec::new();

        for depth in 1..=max_depth {
            if frontier.is_empty() {
                break;
            }
            let mut next_frontier = Vec::new();

            for sym_id in &frontier {
                let neighbors =
                    self.get_neighbors(*sym_id, direction, max_fanout)?;
                for (neighbor_id, rel_kind) in neighbors {
                    if visited.insert(neighbor_id.0) {
                        results.push(TraversalHit {
                            symbol_id: neighbor_id,
                            depth,
                            relation_kind: rel_kind,
                        });
                        next_frontier.push(neighbor_id);
                    }
                }
            }

            frontier = next_frontier;
        }

        Ok(results)
    }

    fn get_neighbors(
        &self,
        sym_id: SymbolId,
        direction: TraversalDirection,
        max_fanout: u32,
    ) -> Result<Vec<(SymbolId, RelationKind)>, StorageError> {
        let id_bytes = sym_id.as_bytes();
        let mut results = Vec::new();

        if direction == TraversalDirection::Outgoing || direction == TraversalDirection::Both {
            let mut stmt = self.conn.prepare_cached(
                "SELECT target_id, kind FROM relations WHERE source_id = ?1 LIMIT ?2",
            )?;
            let mut rows = stmt.query(params![id_bytes.as_slice(), max_fanout as i64])?;
            while let Some(row) = rows.next()? {
                if let Some((sid, rk)) = parse_neighbor_row(row)? {
                    results.push((sid, rk));
                }
            }
        }

        if direction == TraversalDirection::Incoming || direction == TraversalDirection::Both {
            let remaining = max_fanout.saturating_sub(results.len() as u32);
            if remaining > 0 {
                let mut stmt = self.conn.prepare_cached(
                    "SELECT source_id, kind FROM relations WHERE target_id = ?1 LIMIT ?2",
                )?;
                let mut rows = stmt.query(params![id_bytes.as_slice(), remaining as i64])?;
                while let Some(row) = rows.next()? {
                    if let Some((sid, rk)) = parse_neighbor_row(row)? {
                        results.push((sid, rk));
                    }
                }
            }
        }

        Ok(results)
    }

    // -- File metadata --

    pub fn upsert_file(&mut self, meta: &FileMetadata) -> Result<(), StorageError> {
        self.conn.execute(
            "INSERT OR REPLACE INTO files \
             (path, content_hash, language, size_bytes, symbol_count, last_indexed, last_modified) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                meta.path,
                meta.content_hash as i64,
                meta.language.ordinal() as i64,
                meta.size_bytes as i64,
                meta.symbol_count as i64,
                meta.last_indexed,
                meta.last_modified,
            ],
        )?;
        Ok(())
    }

    pub fn get_file(&self, path: &str) -> Result<Option<FileMetadata>, StorageError> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT path, content_hash, language, size_bytes, symbol_count, \
             last_indexed, last_modified FROM files WHERE path = ?1",
        )?;
        let mut rows = stmt.query(params![path])?;
        match rows.next()? {
            Some(row) => Ok(Some(row_to_file_metadata(row)?)),
            None => Ok(None),
        }
    }

    pub fn get_file_by_content_hash(
        &self,
        content_hash: u64,
    ) -> Result<Vec<FileMetadata>, StorageError> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT path, content_hash, language, size_bytes, symbol_count, \
             last_indexed, last_modified FROM files WHERE content_hash = ?1",
        )?;
        let mut rows = stmt.query(params![content_hash as i64])?;
        let mut results = Vec::new();
        while let Some(row) = rows.next()? {
            results.push(row_to_file_metadata(row)?);
        }
        Ok(results)
    }

    pub fn delete_file(&mut self, path: &str) -> Result<bool, StorageError> {
        let affected = self.conn.execute("DELETE FROM files WHERE path = ?1", params![path])?;
        Ok(affected > 0)
    }

    // -- Repository metadata --

    pub fn upsert_repo(&mut self, meta: &RepoMetadata) -> Result<(), StorageError> {
        self.conn.execute(
            "INSERT OR REPLACE INTO repositories (id, path, name, created_at) \
             VALUES (?1, ?2, ?3, ?4)",
            params![meta.id, meta.path, meta.name, meta.created_at],
        )?;
        Ok(())
    }

    pub fn get_repo(&self, id: &str) -> Result<Option<RepoMetadata>, StorageError> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT id, path, name, created_at FROM repositories WHERE id = ?1",
        )?;
        let mut rows = stmt.query(params![id])?;
        match rows.next()? {
            Some(row) => Ok(Some(RepoMetadata {
                id: row.get(0)?,
                path: row.get(1)?,
                name: row.get(2)?,
                created_at: row.get(3)?,
            })),
            None => Ok(None),
        }
    }

    /// Expose the raw connection for advanced usage (e.g., testing).
    #[doc(hidden)]
    pub fn connection(&self) -> &Connection {
        &self.conn
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn configure_pragmas(conn: &Connection) -> Result<(), StorageError> {
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;\
         PRAGMA busy_timeout = 5000;\
         PRAGMA synchronous = NORMAL;\
         PRAGMA foreign_keys = ON;",
    )?;
    Ok(())
}

fn get_user_version(conn: &Connection) -> Result<u32, StorageError> {
    let v: u32 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    Ok(v)
}

fn set_user_version(conn: &Connection, version: u32) -> Result<(), StorageError> {
    conn.pragma_update(None, "user_version", version)?;
    Ok(())
}

fn create_schema(conn: &Connection) -> Result<(), StorageError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS symbols (
            id          BLOB PRIMARY KEY,
            name        TEXT NOT NULL,
            qualified_name TEXT NOT NULL,
            kind        INTEGER NOT NULL,
            language    INTEGER NOT NULL,
            file_path   TEXT NOT NULL,
            line_start  INTEGER NOT NULL,
            line_end    INTEGER NOT NULL,
            byte_start  INTEGER NOT NULL,
            byte_end    INTEGER NOT NULL,
            signature   TEXT,
            doc_comment TEXT,
            body_hash   INTEGER NOT NULL,
            created_at  TEXT NOT NULL,
            updated_at  TEXT NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_symbols_file ON symbols(file_path);
        CREATE INDEX IF NOT EXISTS idx_symbols_name ON symbols(name);
        CREATE INDEX IF NOT EXISTS idx_symbols_qualified ON symbols(qualified_name);
        CREATE INDEX IF NOT EXISTS idx_symbols_kind ON symbols(kind);

        CREATE TABLE IF NOT EXISTS relations (
            id          BLOB PRIMARY KEY,
            source_id   BLOB NOT NULL REFERENCES symbols(id) ON DELETE CASCADE,
            target_id   BLOB NOT NULL REFERENCES symbols(id) ON DELETE CASCADE,
            kind        INTEGER NOT NULL,
            file_path   TEXT NOT NULL,
            line        INTEGER NOT NULL,
            confidence  REAL NOT NULL,
            UNIQUE(source_id, target_id, kind, file_path, line)
        );

        CREATE INDEX IF NOT EXISTS idx_relations_source ON relations(source_id);
        CREATE INDEX IF NOT EXISTS idx_relations_target ON relations(target_id);
        CREATE INDEX IF NOT EXISTS idx_relations_kind ON relations(kind);

        CREATE TABLE IF NOT EXISTS files (
            path          TEXT PRIMARY KEY,
            content_hash  INTEGER NOT NULL,
            language      INTEGER NOT NULL,
            size_bytes    INTEGER NOT NULL,
            symbol_count  INTEGER NOT NULL,
            last_indexed  TEXT NOT NULL,
            last_modified TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS repositories (
            id          TEXT PRIMARY KEY,
            path        TEXT NOT NULL,
            name        TEXT NOT NULL,
            created_at  TEXT NOT NULL
        );",
    )?;
    Ok(())
}

fn now_rfc3339() -> String {
    // We avoid pulling in chrono/time crates. Use a simple UTC timestamp.
    // Format: 2024-01-15T10:30:00Z
    let duration = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = duration.as_secs();
    // Compute from epoch seconds
    let days = secs / 86400;
    let time_of_day = secs % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;

    // Days to Y-M-D (simplified leap year calculation from epoch 1970-01-01)
    let (year, month, day) = days_to_ymd(days);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        year, month, day, hours, minutes, seconds
    )
}

fn days_to_ymd(mut days: u64) -> (u64, u64, u64) {
    // Civil days from epoch algorithm
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

fn compute_relation_id(rel: &CodeRelation) -> [u8; 16] {
    let input = format!(
        "{}|{}|{}|{}|{}",
        rel.source_id,
        rel.target_id,
        rel.kind.ordinal(),
        rel.file_path.to_string_lossy(),
        rel.line
    );
    xxh3_128(input.as_bytes()).to_le_bytes()
}

fn parse_neighbor_row(
    row: &rusqlite::Row<'_>,
) -> Result<Option<(SymbolId, RelationKind)>, StorageError> {
    let blob: Vec<u8> = row.get(0)?;
    if blob.len() != 16 {
        return Ok(None);
    }
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&blob);
    let kind_ord: i64 = row.get(1)?;
    match RelationKind::from_ordinal(kind_ord as u8) {
        Some(rk) => Ok(Some((SymbolId::from_bytes(bytes), rk))),
        None => Ok(None),
    }
}

fn row_to_symbol(row: &rusqlite::Row<'_>) -> Result<CodeSymbol, StorageError> {
    let id_blob: Vec<u8> = row.get(0)?;
    if id_blob.len() != 16 {
        return Err(StorageError::TransactionFailed {
            reason: format!("invalid symbol id length: {}", id_blob.len()),
        });
    }
    let mut id_bytes = [0u8; 16];
    id_bytes.copy_from_slice(&id_blob);

    let kind_ord: i64 = row.get(3)?;
    let lang_ord: i64 = row.get(4)?;
    let file_path_str: String = row.get(5)?;
    let line_start: i64 = row.get(6)?;
    let line_end: i64 = row.get(7)?;
    let byte_start: i64 = row.get(8)?;
    let byte_end: i64 = row.get(9)?;
    let body_hash: i64 = row.get(12)?;

    let kind = SymbolKind::from_ordinal(kind_ord as u8).ok_or_else(|| {
        StorageError::TransactionFailed {
            reason: format!("invalid symbol kind ordinal: {}", kind_ord),
        }
    })?;
    let language = Language::from_ordinal(lang_ord as u8).ok_or_else(|| {
        StorageError::TransactionFailed {
            reason: format!("invalid language ordinal: {}", lang_ord),
        }
    })?;

    Ok(CodeSymbol {
        id: SymbolId::from_bytes(id_bytes),
        name: row.get(1)?,
        qualified_name: row.get(2)?,
        kind,
        language,
        file_path: file_path_str.into(),
        byte_range: (byte_start as usize)..(byte_end as usize),
        line_range: (line_start as u32)..(line_end as u32),
        signature: row.get(10)?,
        doc_comment: row.get(11)?,
        body_hash: body_hash as u64,
    })
}

fn row_to_file_metadata(row: &rusqlite::Row<'_>) -> Result<FileMetadata, StorageError> {
    let content_hash: i64 = row.get(1)?;
    let lang_ord: i64 = row.get(2)?;
    let size: i64 = row.get(3)?;
    let sym_count: i64 = row.get(4)?;

    let language = Language::from_ordinal(lang_ord as u8).ok_or_else(|| {
        StorageError::TransactionFailed {
            reason: format!("invalid language ordinal: {}", lang_ord),
        }
    })?;

    Ok(FileMetadata {
        path: row.get(0)?,
        content_hash: content_hash as u64,
        language,
        size_bytes: size as u64,
        symbol_count: sym_count as u32,
        last_indexed: row.get(5)?,
        last_modified: row.get(6)?,
    })
}


#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn make_symbol(name: &str, file: &str, byte_start: usize, byte_end: usize) -> CodeSymbol {
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
            body_hash: 12345,
        }
    }

    fn make_relation(
        source: &CodeSymbol,
        target: &CodeSymbol,
        kind: RelationKind,
    ) -> CodeRelation {
        CodeRelation {
            source_id: source.id,
            target_id: target.id,
            kind,
            file_path: source.file_path.clone(),
            line: 5,
            confidence: kind.default_confidence(),
        }
    }

    #[test]
    fn symbol_round_trip() {
        let mut store = GraphStore::open_in_memory().unwrap();
        let sym = make_symbol("module.my_func", "src/main.py", 0, 100);
        store.insert_symbols(&[sym.clone()], 1000).unwrap();

        let loaded = store.get_symbol(sym.id).unwrap().unwrap();
        assert_eq!(loaded.id, sym.id);
        assert_eq!(loaded.name, sym.name);
        assert_eq!(loaded.qualified_name, sym.qualified_name);
        assert_eq!(loaded.kind, sym.kind);
        assert_eq!(loaded.language, sym.language);
        assert_eq!(loaded.file_path, sym.file_path);
        assert_eq!(loaded.byte_range, sym.byte_range);
        assert_eq!(loaded.line_range, sym.line_range);
        assert_eq!(loaded.signature, sym.signature);
        assert_eq!(loaded.doc_comment, sym.doc_comment);
        assert_eq!(loaded.body_hash, sym.body_hash);
    }

    #[test]
    fn symbol_query_by_file() {
        let mut store = GraphStore::open_in_memory().unwrap();
        let s1 = make_symbol("a.foo", "src/a.py", 0, 50);
        let s2 = make_symbol("a.bar", "src/a.py", 60, 120);
        let s3 = make_symbol("b.baz", "src/b.py", 0, 80);
        store.insert_symbols(&[s1, s2, s3], 1000).unwrap();

        let a_symbols = store.get_symbols_by_file("src/a.py").unwrap();
        assert_eq!(a_symbols.len(), 2);

        let b_symbols = store.get_symbols_by_file("src/b.py").unwrap();
        assert_eq!(b_symbols.len(), 1);
    }

    #[test]
    fn relation_referential_integrity() {
        let mut store = GraphStore::open_in_memory().unwrap();
        let s1 = make_symbol("a.foo", "src/a.py", 0, 50);
        let s2 = make_symbol("a.bar", "src/a.py", 60, 120);
        store.insert_symbols(&[s1.clone(), s2.clone()], 1000).unwrap();

        let rel = make_relation(&s1, &s2, RelationKind::Calls);
        store.insert_relations(&[rel], 1000).unwrap();

        // Delete s1 → relation should be cascaded
        store.delete_symbol(s1.id).unwrap();

        // Verify no orphan relations
        let count: i64 = store
            .connection()
            .query_row("SELECT COUNT(*) FROM relations", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn khop_traversal_with_cycle() {
        let mut store = GraphStore::open_in_memory().unwrap();
        let a = make_symbol("a", "src/a.py", 0, 10);
        let b = make_symbol("b", "src/a.py", 20, 30);
        let c = make_symbol("c", "src/a.py", 40, 50);
        store
            .insert_symbols(&[a.clone(), b.clone(), c.clone()], 1000)
            .unwrap();

        // A→B→C→A (cycle)
        let rels = vec![
            make_relation(&a, &b, RelationKind::Calls),
            make_relation(&b, &c, RelationKind::Calls),
            make_relation(&c, &a, RelationKind::Calls),
        ];
        store.insert_relations(&rels, 1000).unwrap();

        let hits = store
            .traverse_khop(a.id, 3, 50, TraversalDirection::Outgoing)
            .unwrap();

        // Should find B and C but not revisit A
        let ids: Vec<u128> = hits.iter().map(|h| h.symbol_id.0).collect();
        assert!(ids.contains(&b.id.0));
        assert!(ids.contains(&c.id.0));
        assert!(!ids.contains(&a.id.0));
    }

    #[test]
    fn batch_transaction_splitting() {
        let mut store = GraphStore::open_in_memory().unwrap();
        let symbols: Vec<CodeSymbol> = (0..250)
            .map(|i| make_symbol(&format!("sym_{}", i), "src/a.py", i * 100, (i + 1) * 100))
            .collect();

        // Batch size 100 → should split into 3 transactions (100, 100, 50)
        store.insert_symbols(&symbols, 100).unwrap();

        let count: i64 = store
            .connection()
            .query_row("SELECT COUNT(*) FROM symbols", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 250);
    }

    #[test]
    fn schema_version_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.sqlite");

        // Create initial store
        {
            let _store = GraphStore::open(&db_path).unwrap();
        }

        // Manually set version to something wrong
        {
            let conn = Connection::open(&db_path).unwrap();
            conn.pragma_update(None, "user_version", 999u32).unwrap();
        }

        // Reopen should fail with SchemaMismatch
        let result = GraphStore::open(&db_path);
        assert!(matches!(result, Err(StorageError::SchemaMismatch { .. })));
    }

    #[test]
    fn file_metadata_crud() {
        let mut store = GraphStore::open_in_memory().unwrap();
        let meta = FileMetadata {
            path: "src/main.py".to_string(),
            content_hash: 0xDEADBEEF,
            language: Language::Python,
            size_bytes: 1024,
            symbol_count: 5,
            last_indexed: "2025-01-01T00:00:00Z".to_string(),
            last_modified: "2025-01-01T00:00:00Z".to_string(),
        };
        store.upsert_file(&meta).unwrap();

        let loaded = store.get_file("src/main.py").unwrap().unwrap();
        assert_eq!(loaded.content_hash, 0xDEADBEEF);
        assert_eq!(loaded.symbol_count, 5);

        let by_hash = store.get_file_by_content_hash(0xDEADBEEF).unwrap();
        assert_eq!(by_hash.len(), 1);

        store.delete_file("src/main.py").unwrap();
        assert!(store.get_file("src/main.py").unwrap().is_none());
    }

    #[test]
    fn repo_metadata_crud() {
        let mut store = GraphStore::open_in_memory().unwrap();
        let meta = RepoMetadata {
            id: "abc123".to_string(),
            path: "/home/user/project".to_string(),
            name: "project".to_string(),
            created_at: "2025-01-01T00:00:00Z".to_string(),
        };
        store.upsert_repo(&meta).unwrap();

        let loaded = store.get_repo("abc123").unwrap().unwrap();
        assert_eq!(loaded.path, "/home/user/project");
        assert_eq!(loaded.name, "project");
    }

    #[test]
    fn symbol_deletion_cascades_relations() {
        let mut store = GraphStore::open_in_memory().unwrap();
        let s1 = make_symbol("x.foo", "src/x.py", 0, 50);
        let s2 = make_symbol("x.bar", "src/x.py", 60, 120);
        let s3 = make_symbol("x.baz", "src/x.py", 130, 200);
        store
            .insert_symbols(&[s1.clone(), s2.clone(), s3.clone()], 1000)
            .unwrap();

        let rels = vec![
            make_relation(&s1, &s2, RelationKind::Calls),
            make_relation(&s2, &s3, RelationKind::Calls),
            make_relation(&s1, &s3, RelationKind::Contains),
        ];
        store.insert_relations(&rels, 1000).unwrap();

        // Delete s1 → 2 relations involving s1 should be removed
        store.delete_symbol(s1.id).unwrap();

        let count: i64 = store
            .connection()
            .query_row("SELECT COUNT(*) FROM relations", [], |r| r.get(0))
            .unwrap();
        // Only s2→s3 should remain
        assert_eq!(count, 1);
    }

    #[test]
    fn file_based_symbol_deletion() {
        let mut store = GraphStore::open_in_memory().unwrap();
        let s1 = make_symbol("a.one", "src/a.py", 0, 50);
        let s2 = make_symbol("a.two", "src/a.py", 60, 120);
        let s3 = make_symbol("b.three", "src/b.py", 0, 80);
        store
            .insert_symbols(&[s1, s2, s3], 1000)
            .unwrap();

        let deleted = store.delete_symbols_by_file("src/a.py").unwrap();
        assert_eq!(deleted, 2);

        let remaining: i64 = store
            .connection()
            .query_row("SELECT COUNT(*) FROM symbols", [], |r| r.get(0))
            .unwrap();
        assert_eq!(remaining, 1);
    }

    #[test]
    fn khop_fanout_limit() {
        let mut store = GraphStore::open_in_memory().unwrap();

        // Create a hub with many spokes
        let hub = make_symbol("hub", "src/hub.py", 0, 10);
        let mut all_symbols = vec![hub.clone()];
        let mut all_rels = Vec::new();

        for i in 0..100 {
            let spoke = make_symbol(
                &format!("spoke_{}", i),
                "src/hub.py",
                (i + 1) * 100,
                (i + 2) * 100,
            );
            all_rels.push(make_relation(&hub, &spoke, RelationKind::Calls));
            all_symbols.push(spoke);
        }

        store.insert_symbols(&all_symbols, 1000).unwrap();
        store.insert_relations(&all_rels, 1000).unwrap();

        // Fanout limit = 10
        let hits = store
            .traverse_khop(hub.id, 1, 10, TraversalDirection::Outgoing)
            .unwrap();
        assert!(hits.len() <= 10);
    }

    #[test]
    fn list_symbols_pagination() {
        let mut store = GraphStore::open_in_memory().unwrap();
        // Insert 10 symbols
        let symbols: Vec<CodeSymbol> = (0..10)
            .map(|i| make_symbol(&format!("sym_{}", i), "src/a.py", i * 100, (i + 1) * 100))
            .collect();
        store.insert_symbols(&symbols, 1000).unwrap();

        // Page 1: first 3
        let page1 = store.list_symbols(3, 0).unwrap();
        assert_eq!(page1.len(), 3);

        // Page 2: next 3
        let page2 = store.list_symbols(3, 3).unwrap();
        assert_eq!(page2.len(), 3);

        // No overlap between pages
        let page1_ids: Vec<_> = page1.iter().map(|s| s.id).collect();
        let page2_ids: Vec<_> = page2.iter().map(|s| s.id).collect();
        for id in &page1_ids {
            assert!(!page2_ids.contains(id));
        }

        // All 10
        let all = store.list_symbols(100, 0).unwrap();
        assert_eq!(all.len(), 10);

        // Beyond end
        let empty = store.list_symbols(10, 100).unwrap();
        assert!(empty.is_empty());
    }

    #[test]
    fn count_symbols_accuracy() {
        let mut store = GraphStore::open_in_memory().unwrap();
        assert_eq!(store.count_symbols().unwrap(), 0);

        let symbols: Vec<CodeSymbol> = (0..5)
            .map(|i| make_symbol(&format!("sym_{}", i), "src/a.py", i * 100, (i + 1) * 100))
            .collect();
        store.insert_symbols(&symbols, 1000).unwrap();
        assert_eq!(store.count_symbols().unwrap(), 5);

        // Delete one
        store.delete_symbol(symbols[0].id).unwrap();
        assert_eq!(store.count_symbols().unwrap(), 4);
    }

    #[test]
    fn list_symbols_empty_table() {
        let store = GraphStore::open_in_memory().unwrap();
        let result = store.list_symbols(10, 0).unwrap();
        assert!(result.is_empty());
        assert_eq!(store.count_symbols().unwrap(), 0);
    }
}
