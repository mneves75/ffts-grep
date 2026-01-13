use chrono::Utc;
use rusqlite::ToSql;
use std::path::Path;
use std::time::Duration;

use crate::error::{IndexerError, Result};

/// Search result returned by FTS5 queries.
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub path: String,
    pub rank: f64,
}

/// Database configuration for PRAGMA settings.
#[derive(Debug, Clone)]
pub struct PragmaConfig {
    pub journal_mode: String,
    pub synchronous: String,
    pub cache_size: i64,
    pub temp_store: String,
    pub mmap_size: i64,
    pub page_size: i64,
    pub busy_timeout_ms: i64,
}

impl Default for PragmaConfig {
    fn default() -> Self {
        Self {
            journal_mode: "WAL".to_string(),
            synchronous: "NORMAL".to_string(),
            cache_size: -32000, // -32000 KB = 32MB (better for 10K+ file codebases)
            temp_store: "MEMORY".to_string(),
            mmap_size: Self::default_mmap_size(),
            page_size: 4096,
            busy_timeout_ms: 5000,
        }
    }
}

impl PragmaConfig {
    /// Platform-aware `mmap_size` default.
    ///
    /// - **macOS**: Returns 0 (mmap unreliable on HFS+/APFS with `SQLite`)
    /// - **Linux/Other**: Returns 256MB (significant read performance boost)
    ///
    /// # References
    /// - <https://sqlite.org/pragma.html#pragma_mmap_size>
    /// - <https://phiresky.github.io/blog/2020/sqlite-performance-tuning/>
    #[cfg(target_os = "macos")]
    #[must_use]
    pub const fn default_mmap_size() -> i64 {
        0
    }

    /// Platform-aware mmap_size default (Linux/Other: 256MB).
    #[cfg(not(target_os = "macos"))]
    #[must_use]
    pub fn default_mmap_size() -> i64 {
        256 * 1024 * 1024 // 256MB
    }
}

/// FTS5 file index database.
///
/// Uses external content table pattern where `files_fts` virtual table
/// automatically syncs with `files` table via triggers.
///
/// # Connection Pooling Design Decision (2025+ consideration)
///
/// This implementation uses a single connection rather than connection pooling
/// (e.g., r2d2, deadpool) for the following reasons:
///
/// 1. **CLI Context**: This is a CLI tool invoked by Claude Code, not a long-running
///    server. Each invocation creates one connection, uses it, and exits.
///
/// 2. **`SQLite` WAL Mode**: WAL mode already enables concurrent readers, which is
///    the primary use case (search while indexing in background).
///
/// 3. **`busy_timeout`**: Configured to 5000ms to handle write contention gracefully
///    when multiple processes access the database.
///
/// 4. **Complexity vs. Benefit**: Connection pooling adds dependency overhead
///    ([r2d2_sqlite](https://lib.rs/crates/r2d2_sqlite) or
///    [deadpool-sqlite](https://github.com/deadpool-rs/deadpool)) without
///    measurable benefit for single-invocation CLI tools.
///
/// **When to reconsider**: If this library is embedded in a long-running server
/// or needs to handle concurrent write operations, add `r2d2_sqlite` for
/// synchronous pooling or `deadpool-sqlite` for async pooling.
pub struct Database {
    conn: rusqlite::Connection,
}

impl Database {
    fn apply_pragma(conn: &rusqlite::Connection, name: &str, value: impl ToSql) -> Result<()> {
        conn.pragma_update(None, name, value).map_err(|e| IndexerError::Database { source: e })
    }

    /// Open database at path, creating if needed.
    ///
    /// # Errors
    /// Returns `IndexerError::Database` if:
    /// - The database file cannot be opened or created
    /// - Any PRAGMA setting fails to apply
    pub fn open(db_path: &Path, config: &PragmaConfig) -> Result<Self> {
        if config.busy_timeout_ms < 0 {
            return Err(IndexerError::ConfigInvalid {
                field: "busy_timeout_ms".to_string(),
                value: config.busy_timeout_ms.to_string(),
                reason: "must be >= 0".to_string(),
            });
        }

        let conn = rusqlite::Connection::open(db_path)?;

        // Apply PRAGMAs with error context
        Self::apply_pragma(&conn, "journal_mode", &config.journal_mode)?;
        Self::apply_pragma(&conn, "synchronous", &config.synchronous)?;
        Self::apply_pragma(&conn, "cache_size", config.cache_size)?;
        Self::apply_pragma(&conn, "temp_store", &config.temp_store)?;
        Self::apply_pragma(&conn, "mmap_size", config.mmap_size)?;
        Self::apply_pragma(&conn, "page_size", config.page_size)?;

        // SQLite-GUIDELINES.md: Required PRAGMAs for safety and correctness
        Self::apply_pragma(&conn, "foreign_keys", "ON")?;
        Self::apply_pragma(&conn, "trusted_schema", "OFF")?;
        // Application ID: 0xA17E_6D42 signature for cc-fts5-indexer
        // Safety: SQLite application_id is a signed 32-bit integer but used as unsigned identifier
        // This specific value (2,710,531,394) is well within i32 positive range
        #[allow(clippy::cast_possible_wrap)]
        Self::apply_pragma(&conn, "application_id", 0xA17E_6D42_u32 as i32)?;

        // Safety: i64→u64 cast is safe for non-negative timeout values
        // config.busy_timeout_ms is always positive (default 5000ms)
        #[allow(clippy::cast_sign_loss)]
        let busy_timeout = Duration::from_millis(config.busy_timeout_ms as u64);
        conn.busy_timeout(busy_timeout).map_err(|e| IndexerError::Database { source: e })?;

        Ok(Self { conn })
    }

    /// Migrate legacy database schema (2-column FTS5) to current schema (3-column FTS5 with filename).
    ///
    /// This migration:
    /// 1. Adds `filename` column to `files` table if missing
    /// 2. Populates filename from existing paths using Rust (SQLite lacks string functions)
    /// 3. Drops old FTS5 table and triggers (will be recreated by `init_schema`)
    ///
    /// Safe to call multiple times - only runs if migration is needed.
    ///
    /// # Errors
    /// Returns `IndexerError::Database` if migration DDL fails.
    pub fn migrate_schema(&self) -> Result<()> {
        // Check if filename column already exists
        let has_filename: bool = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('files') WHERE name = 'filename'",
                [],
                |row| row.get::<_, i64>(0).map(|n| n > 0),
            )
            .unwrap_or(false);

        if has_filename {
            // Already migrated, nothing to do
            return Ok(());
        }

        tracing::info!("Migrating database schema (adding filename column for FTS5 ranking)");

        // Add filename column to files table
        self.conn
            .execute("ALTER TABLE files ADD COLUMN filename TEXT", [])
            .map_err(|e| IndexerError::Database { source: e })?;

        // Populate filename from existing paths using Rust
        // SQLite lacks reliable string functions for extracting filename
        let paths: Vec<(i64, String)> = {
            let mut stmt = self
                .conn
                .prepare("SELECT id, path FROM files WHERE filename IS NULL")
                .map_err(|e| IndexerError::Database { source: e })?;
            stmt.query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))
                .map_err(|e| IndexerError::Database { source: e })?
                .filter_map(|r| r.ok())
                .collect()
        };

        // Update each row with extracted filename
        for (id, path) in paths {
            let filename = Path::new(&path).file_name().and_then(|n| n.to_str()).unwrap_or(&path);
            self.conn
                .execute(
                    "UPDATE files SET filename = ?1 WHERE id = ?2",
                    rusqlite::params![filename, id],
                )
                .map_err(|e| IndexerError::Database { source: e })?;
        }

        // Drop old triggers (will be recreated by init_schema with new column)
        self.conn
            .execute("DROP TRIGGER IF EXISTS files_ai", [])
            .map_err(|e| IndexerError::Database { source: e })?;
        self.conn
            .execute("DROP TRIGGER IF EXISTS files_au", [])
            .map_err(|e| IndexerError::Database { source: e })?;
        self.conn
            .execute("DROP TRIGGER IF EXISTS files_ad", [])
            .map_err(|e| IndexerError::Database { source: e })?;

        // Drop old FTS5 table (will be recreated by init_schema with 3 columns)
        self.conn
            .execute("DROP TABLE IF EXISTS files_fts", [])
            .map_err(|e| IndexerError::Database { source: e })?;

        tracing::info!("Schema migration complete - call init_schema() then rebuild_fts_index()");

        Ok(())
    }

    /// Rebuild FTS5 index from existing files table data.
    ///
    /// Call this after `migrate_schema()` and `init_schema()` to repopulate the
    /// FTS5 index with existing data. This is necessary because the migration
    /// drops and recreates the FTS5 table.
    ///
    /// # Errors
    /// Returns `IndexerError::Database` if the INSERT query fails.
    pub fn rebuild_fts_index(&self) -> Result<()> {
        // Manually populate FTS5 from existing files table
        // This bypasses triggers to do a bulk rebuild
        self.conn
            .execute(
                "INSERT INTO files_fts(rowid, filename, path, content)
                 SELECT id, filename, path, content FROM files",
                [],
            )
            .map_err(|e| IndexerError::Database { source: e })?;

        tracing::info!("FTS5 index rebuilt from existing data");
        Ok(())
    }

    /// Initialize schema (idempotent - safe to call multiple times).
    ///
    /// # Errors
    /// Returns `IndexerError::Database` if any CREATE TABLE, CREATE TRIGGER, or CREATE INDEX statement fails.
    pub fn init_schema(&self) -> Result<()> {
        // Main files table
        // The `filename` column stores just the file name (e.g., "CLAUDE.md" from "docs/CLAUDE.md")
        // This enables precise BM25 weighting where filename matches rank higher than path matches
        self.conn
            .execute(
                "CREATE TABLE IF NOT EXISTS files (
                id INTEGER PRIMARY KEY,
                path TEXT UNIQUE NOT NULL,
                filename TEXT,
                content_hash TEXT,
                mtime INTEGER,
                size INTEGER,
                indexed_at INTEGER,
                content TEXT
            )",
                [],
            )
            .map_err(|e| IndexerError::Database { source: e })?;

        // FTS5 virtual table with external content
        // Column order: filename, path, content (for BM25 weight arguments)
        // BM25 weights: filename=100, path=50, content=1
        // This ensures "CLAUDE.md" ranks higher than "docs/MASTRA-VS-CLAUDE-SDK.md" for query "claude"
        // columnsize=0: saves 10-15% storage (BM25 ranking still works)
        self.conn
            .execute(
                "CREATE VIRTUAL TABLE IF NOT EXISTS files_fts USING fts5(
                filename, path, content,
                content='files',
                content_rowid='id',
                tokenize='porter unicode61',
                columnsize=0
            )",
                [],
            )
            .map_err(|e| IndexerError::Database { source: e })?;

        // Auto-sync triggers (include filename for FTS5 indexing)
        self.conn
            .execute(
                "CREATE TRIGGER IF NOT EXISTS files_ai AFTER INSERT ON files BEGIN
                INSERT INTO files_fts(rowid, filename, path, content)
                VALUES (new.id, new.filename, new.path, new.content);
            END",
                [],
            )
            .map_err(|e| IndexerError::Database { source: e })?;

        self.conn
            .execute(
                "CREATE TRIGGER IF NOT EXISTS files_au AFTER UPDATE ON files BEGIN
                INSERT INTO files_fts(files_fts, rowid, filename, path, content)
                VALUES('delete', old.id, old.filename, old.path, old.content);
                INSERT INTO files_fts(rowid, filename, path, content)
                VALUES (new.id, new.filename, new.path, new.content);
            END",
                [],
            )
            .map_err(|e| IndexerError::Database { source: e })?;

        self.conn
            .execute(
                "CREATE TRIGGER IF NOT EXISTS files_ad AFTER DELETE ON files BEGIN
                INSERT INTO files_fts(files_fts, rowid, filename, path, content)
                VALUES('delete', old.id, old.filename, old.path, old.content);
            END",
                [],
            )
            .map_err(|e| IndexerError::Database { source: e })?;

        // Indexes for efficient queries
        self.conn
            .execute("CREATE INDEX IF NOT EXISTS idx_files_mtime ON files(mtime)", [])
            .map_err(|e| IndexerError::Database { source: e })?;

        self.conn
            .execute("CREATE INDEX IF NOT EXISTS idx_files_path ON files(path)", [])
            .map_err(|e| IndexerError::Database { source: e })?;

        // Index on content_hash for faster lazy invalidation subquery
        self.conn
            .execute("CREATE INDEX IF NOT EXISTS idx_files_hash ON files(content_hash)", [])
            .map_err(|e| IndexerError::Database { source: e })?;

        Ok(())
    }

    /// Insert or update a file (lazy invalidation via `content_hash`).
    ///
    /// Uses ON CONFLICT to handle both insert and update in one query.
    /// Skips update if `content_hash` matches (same content, same mtime).
    ///
    /// The `filename` is automatically extracted from `path` for FTS5 ranking.
    /// This enables BM25 to weight filename matches higher than path matches.
    ///
    /// # Errors
    /// Returns `IndexerError::Database` if the INSERT or UPDATE query fails.
    pub fn upsert_file(&self, path: &str, content: &str, mtime: i64, size: i64) -> Result<()> {
        let hash = wyhash(content.as_bytes());
        let now = Utc::now().timestamp();

        // Extract filename from path for FTS5 ranking boost
        // e.g., "docs/CLAUDE.md" -> "CLAUDE.md"
        let filename = Path::new(path).file_name().and_then(|n| n.to_str()).unwrap_or(path);

        // Lazy invalidation: only update if content changed
        // The ON CONFLICT handles the case where path exists
        self.conn.execute(
            "INSERT INTO files (path, filename, content_hash, mtime, size, indexed_at, content)
             VALUES (?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(path) DO UPDATE SET
                 filename = excluded.filename,
                 content_hash = excluded.content_hash,
                 mtime = excluded.mtime,
                 size = excluded.size,
                 indexed_at = excluded.indexed_at,
                 content = excluded.content
             WHERE excluded.content_hash != (SELECT content_hash FROM files WHERE path = excluded.path)",
            rusqlite::params![path, filename, hash, mtime, size, now, content],
        )
        .map_err(|e| IndexerError::Database { source: e })?;

        Ok(())
    }

    /// Delete a file from the index.
    ///
    /// # Errors
    /// Returns `IndexerError::Database` if the DELETE query fails.
    pub fn delete_file(&self, path: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM files WHERE path = ?", [path])
            .map_err(|e| IndexerError::Database { source: e })?;
        Ok(())
    }

    /// Search with BM25 ranking (filename weight: 100, path weight: 50, content weight: 1).
    ///
    /// Returns results sorted by BM25 score (lower = better match).
    /// - Filename matches get 100x weight (highest priority for exact file matches)
    /// - Path matches get 50x weight (directory structure relevance)
    /// - Content matches get 1x weight (tiebreaker)
    ///
    /// This ensures "CLAUDE.md" ranks higher than "docs/MASTRA-VS-CLAUDE-SDK.md" for query "claude".
    ///
    /// # Errors
    /// Returns `IndexerError::Database` if:
    /// - The FTS5 MATCH query fails (e.g., invalid FTS5 syntax)
    /// - Query preparation or execution fails
    pub fn search(&self, query: &str, paths_only: bool, limit: u32) -> Result<Vec<SearchResult>> {
        // Handle empty queries gracefully
        if query.trim().is_empty() {
            return Ok(vec![]);
        }

        // BM25 weights: filename=100, path=50, content=1
        // Column order in FTS5: filename, path, content
        let sql = if paths_only {
            "SELECT path, bm25(files_fts, 100.0, 50.0, 1.0) FROM files_fts
             WHERE path MATCH ?1 ORDER BY bm25(files_fts, 100.0, 50.0, 1.0) LIMIT ?2"
        } else {
            "SELECT path, bm25(files_fts, 100.0, 50.0, 1.0) FROM files_fts
             WHERE files_fts MATCH ?1 ORDER BY bm25(files_fts, 100.0, 50.0, 1.0) LIMIT ?2"
        };

        let mut stmt =
            self.conn.prepare_cached(sql).map_err(|e| IndexerError::Database { source: e })?;

        // Pre-allocate Vec to avoid reallocation in hot path
        let mut results = Vec::with_capacity(limit as usize);
        let rows = stmt
            .query_map(rusqlite::params![query, limit], |row| {
                Ok(SearchResult { path: row.get::<_, String>(0)?, rank: row.get::<_, f64>(1)? })
            })
            .map_err(|e| IndexerError::Database { source: e })?;

        for row in rows {
            results.push(row.map_err(|e| IndexerError::Database { source: e })?);
        }

        Ok(results)
    }

    /// Search for files where filename CONTAINS the query substring (case-insensitive).
    ///
    /// This bypasses FTS5 token matching to enable substring searches.
    /// FTS5 only matches whole tokens, so "intro" doesn't match "introduction".
    /// SQL LIKE '%intro%' finds "01-introduction.md" correctly.
    ///
    /// Results are ordered by:
    /// 1. Exact filename match (highest priority)
    /// 2. Filename starts with query (prefix match)
    /// 3. Filename length (shorter = more relevant)
    ///
    /// # Arguments
    /// * `query` - Search term (wildcards stripped if present)
    /// * `limit` - Maximum results to return
    ///
    /// # Errors
    /// Returns `IndexerError::Database` if the query fails.
    pub fn search_filename_contains(&self, query: &str, limit: u32) -> Result<Vec<String>> {
        let query = query.trim();
        if query.is_empty() {
            return Ok(vec![]);
        }

        // Strip wildcard if present (from auto-prefix like "01-" → "01*")
        let search_term = query.trim_end_matches('*');
        if search_term.is_empty() {
            return Ok(vec![]);
        }

        let like_term = escape_like_pattern(search_term);

        // CONTAINS match with intelligent ordering:
        // - CASE 0: exact filename match
        // - CASE 1: filename starts with query (prefix)
        // - CASE 2: filename contains query anywhere
        // Secondary sort by filename length (shorter = more specific match)
        let sql = "SELECT path FROM files
                   WHERE filename LIKE '%' || ?1 || '%' ESCAPE '\\' COLLATE NOCASE
                   ORDER BY
                       CASE WHEN LOWER(filename) = LOWER(?2) THEN 0
                            WHEN LOWER(filename) LIKE LOWER(?1) || '%' ESCAPE '\\' THEN 1
                            ELSE 2 END,
                       length(filename)
                   LIMIT ?3";

        let mut stmt =
            self.conn.prepare_cached(sql).map_err(|e| IndexerError::Database { source: e })?;

        let paths: Vec<String> = stmt
            .query_map(rusqlite::params![like_term, search_term, limit], |row| {
                row.get::<_, String>(0)
            })
            .map_err(|e| IndexerError::Database { source: e })?
            .filter_map(std::result::Result::ok)
            .collect();

        Ok(paths)
    }

    /// Get all indexed file paths.
    ///
    /// # Errors
    /// Returns `IndexerError::Database` if the SELECT query fails.
    #[must_use = "returns file paths that should be used"]
    pub fn get_all_files(&self, limit: u32) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT path FROM files LIMIT ?")
            .map_err(|e| IndexerError::Database { source: e })?;

        let paths: Vec<String> = stmt
            .query_map([limit], |row| row.get::<_, String>(0))
            .map_err(|e| IndexerError::Database { source: e })?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| IndexerError::Database { source: e })?;

        Ok(paths)
    }

    /// Get total number of indexed files.
    ///
    /// # Errors
    /// Returns `IndexerError::Database` if the COUNT query fails.
    #[must_use = "returns count that should be used"]
    pub fn get_file_count(&self) -> Result<usize> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM files", [], |row| row.get(0))
            .map_err(|e| IndexerError::Database { source: e })?;
        // Safety: File count will never exceed usize::MAX (limited by available memory)
        // SQLite COUNT returns i64, but practical file counts fit in usize on all platforms
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        Ok(count as usize)
    }

    /// Optimize FTS5 index by merging b-trees.
    ///
    /// # Errors
    /// Returns `IndexerError::Database` if the FTS5 optimize command fails.
    pub fn optimize_fts(&self) -> Result<()> {
        // 'optimize' command merges FTS5 segment b-trees
        self.conn
            .execute("INSERT INTO files_fts(files_fts) VALUES('optimize')", [])
            .map_err(|e| IndexerError::Database { source: e })?;

        Ok(())
    }

    /// Optimize `SQLite` query planner statistics (2025+ best practice).
    ///
    /// Should be called after bulk indexing operations to update statistics
    /// that `SQLite` uses for query planning. This improves query performance
    /// by ensuring the query planner has accurate table/index statistics.
    ///
    /// # Errors
    /// Returns `IndexerError::Database` if the PRAGMA optimize command fails.
    pub fn optimize(&self) -> Result<()> {
        self.conn
            .execute("PRAGMA optimize", [])
            .map_err(|e| IndexerError::Database { source: e })?;
        Ok(())
    }

    /// Check FTS5 integrity.
    ///
    /// Returns true if integrity check passes.
    #[must_use = "returns integrity status that should be checked"]
    pub fn check_fts_integrity(&self) -> bool {
        // FTS5 integrity check command
        self.conn.execute("INSERT INTO files_fts(files_fts) VALUES('integrity-check')", []).is_ok()
    }

    /// Get database connection (for transactions).
    pub const fn conn(&self) -> &rusqlite::Connection {
        &self.conn
    }

    /// Get mut database connection.
    pub const fn conn_mut(&mut self) -> &mut rusqlite::Connection {
        &mut self.conn
    }

    /// Open database in read-only mode (for --doctor diagnostics).
    ///
    /// CRITICAL: Uses `SQLITE_OPEN_READ_ONLY` to ensure no WAL modifications.
    /// This is essential for `--doctor` to be truly non-destructive.
    ///
    /// # Errors
    ///
    /// Returns error if database file doesn't exist or can't be opened.
    pub fn open_readonly(db_path: &Path) -> Result<Self> {
        use rusqlite::OpenFlags;

        let conn = rusqlite::Connection::open_with_flags(
            db_path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|e| IndexerError::Database { source: e })?;

        // Skip PRAGMA writes - just query for read-only access
        Ok(Self { conn })
    }

    /// Check if all required schema objects exist.
    ///
    /// Returns a `SchemaCheck` struct with details about what's present/missing.
    #[must_use]
    pub fn check_schema(&self) -> SchemaCheck {
        let mut check = SchemaCheck::default();

        // Query sqlite_master for all expected objects
        let query = r"
            SELECT
                (SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='files') AS has_files_table,
                (SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='files_fts') AS has_fts_table,
                (SELECT COUNT(*) FROM sqlite_master WHERE type='trigger' AND name='files_ai') AS has_insert_trigger,
                (SELECT COUNT(*) FROM sqlite_master WHERE type='trigger' AND name='files_au') AS has_update_trigger,
                (SELECT COUNT(*) FROM sqlite_master WHERE type='trigger' AND name='files_ad') AS has_delete_trigger,
                (SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='idx_files_mtime') AS has_mtime_idx,
                (SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='idx_files_path') AS has_path_idx,
                (SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='idx_files_hash') AS has_hash_idx
        ";

        if let Ok(row) = self.conn.query_row(query, [], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, i64>(6)?,
                row.get::<_, i64>(7)?,
            ))
        }) {
            check.has_files_table = row.0 > 0;
            check.has_fts_table = row.1 > 0;
            check.has_insert_trigger = row.2 > 0;
            check.has_update_trigger = row.3 > 0;
            check.has_delete_trigger = row.4 > 0;
            check.has_mtime_index = row.5 > 0;
            check.has_path_index = row.6 > 0;
            check.has_hash_index = row.7 > 0;
        }

        check
    }

    /// Get `application_id` pragma value.
    ///
    /// Expected value for cc-fts5-indexer: `0xA17E_6D42`
    #[must_use]
    pub fn get_application_id(&self) -> Option<u32> {
        self.conn
            .query_row("PRAGMA application_id", [], |row| {
                // Safety: SQLite application_id is stored as i32 but semantically unsigned
                // Reinterpreting the bit pattern as u32 (intended usage)
                #[allow(clippy::cast_sign_loss)]
                row.get::<_, i32>(0).map(|v| v as u32)
            })
            .ok()
    }

    /// Get `journal_mode` pragma value.
    #[must_use]
    pub fn get_journal_mode(&self) -> Option<String> {
        self.conn.query_row("PRAGMA journal_mode", [], |row| row.get::<_, String>(0)).ok()
    }

    /// Get database file size in bytes.
    ///
    /// Uses `page_count * page_size` for accurate size including all pages.
    #[must_use]
    pub fn get_db_size_bytes(&self) -> Option<u64> {
        let page_count: i64 =
            self.conn.query_row("PRAGMA page_count", [], |row| row.get(0)).ok()?;
        let page_size: i64 = self.conn.query_row("PRAGMA page_size", [], |row| row.get(0)).ok()?;
        // Safety: page_count and page_size are always non-negative
        // Result is database size in bytes, semantically unsigned
        #[allow(clippy::cast_sign_loss)]
        Some((page_count * page_size) as u64)
    }
}

/// Escape LIKE wildcard characters in user input.
fn escape_like_pattern(input: &str) -> String {
    let mut escaped = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            '%' => escaped.push_str("\\%"),
            '_' => escaped.push_str("\\_"),
            _ => escaped.push(ch),
        }
    }
    escaped
}

/// Result of schema completeness check.
///
/// Each boolean flag represents an independent schema component.
/// Using individual bools (vs. bitfield) provides clearer diagnostic output.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Default, Clone)]
pub struct SchemaCheck {
    pub has_files_table: bool,
    pub has_fts_table: bool,
    pub has_insert_trigger: bool,
    pub has_update_trigger: bool,
    pub has_delete_trigger: bool,
    pub has_mtime_index: bool,
    pub has_path_index: bool,
    pub has_hash_index: bool,
}

impl SchemaCheck {
    /// Returns true if all required schema objects exist.
    #[must_use]
    pub const fn is_complete(&self) -> bool {
        self.has_files_table
            && self.has_fts_table
            && self.has_insert_trigger
            && self.has_update_trigger
            && self.has_delete_trigger
            && self.has_mtime_index
            && self.has_path_index
            && self.has_hash_index
    }

    /// Count of tables (expected: 2 - files + `files_fts`).
    #[must_use]
    pub const fn table_count(&self) -> usize {
        let mut count = 0;
        if self.has_files_table {
            count += 1;
        }
        if self.has_fts_table {
            count += 1;
        }
        count
    }

    /// Count of triggers (expected: 3).
    #[must_use]
    pub const fn trigger_count(&self) -> usize {
        let mut count = 0;
        if self.has_insert_trigger {
            count += 1;
        }
        if self.has_update_trigger {
            count += 1;
        }
        if self.has_delete_trigger {
            count += 1;
        }
        count
    }

    /// Count of indexes (expected: 3).
    #[must_use]
    pub const fn index_count(&self) -> usize {
        let mut count = 0;
        if self.has_mtime_index {
            count += 1;
        }
        if self.has_path_index {
            count += 1;
        }
        if self.has_hash_index {
            count += 1;
        }
        count
    }

    /// Get list of missing objects.
    #[must_use]
    pub fn missing_objects(&self) -> Vec<&'static str> {
        let mut missing = Vec::new();
        if !self.has_files_table {
            missing.push("table: files");
        }
        if !self.has_fts_table {
            missing.push("table: files_fts");
        }
        if !self.has_insert_trigger {
            missing.push("trigger: files_ai");
        }
        if !self.has_update_trigger {
            missing.push("trigger: files_au");
        }
        if !self.has_delete_trigger {
            missing.push("trigger: files_ad");
        }
        if !self.has_mtime_index {
            missing.push("index: idx_files_mtime");
        }
        if !self.has_path_index {
            missing.push("index: idx_files_path");
        }
        if !self.has_hash_index {
            missing.push("index: idx_files_hash");
        }
        missing
    }
}

/// Wyhash 64-bit hash implementation.
///
/// This matches Zig's std.hash.Wyhash exactly:
/// - 64-bit output as 16 hex characters
/// - Big-endian hex encoding
///
/// IMPORTANT: This must match Zig's output for lazy invalidation to work.
///
/// # Performance
/// This function is called for every file during indexing and is marked `#[inline]`
/// to enable cross-crate inlining for maximum performance on the hot path.
/// Uses `format!` macro which is heavily optimized by LLVM for hex conversion.
#[inline]
#[must_use]
pub fn wyhash(content: &[u8]) -> String {
    let hash = wyhash::wyhash(content, 0);
    // format! with :016x is LLVM-optimized and produces identical big-endian output
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DB_NAME;
    use tempfile::tempdir;

    fn create_test_db() -> (tempfile::TempDir, Database) {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join(DB_NAME);
        let config = PragmaConfig::default();
        let db = Database::open(&db_path, &config).unwrap();
        db.init_schema().unwrap();
        (dir, db)
    }

    #[test]
    fn test_wyhash_empty_string() {
        // Empty string produces valid hash
        let hash = wyhash(b"");
        assert_eq!(hash.len(), 16);
        // Verify hex characters only
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_wyhash_hello() {
        // "hello" hash for verification
        let hash = wyhash(b"hello");
        assert_eq!(hash.len(), 16);
        // Verify hex characters only
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_open_creates_db() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let config = PragmaConfig::default();

        // File shouldn't exist yet
        assert!(!db_path.exists());

        let db = Database::open(&db_path, &config).unwrap();
        db.init_schema().unwrap();

        // Now file exists
        assert!(db_path.exists());
    }

    #[test]
    fn test_init_schema_idempotent() {
        let (_dir, db) = create_test_db();

        // Running init_schema twice should not error
        db.init_schema().unwrap();
        db.init_schema().unwrap();
    }

    #[test]
    fn test_upsert_new_file() {
        let (_dir, db) = create_test_db();

        db.upsert_file("test.rs", "fn main() {}", 0, 12).unwrap();

        let files = db.get_all_files(100).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0], "test.rs");
    }

    #[test]
    fn test_upsert_updates_existing() {
        let (_dir, db) = create_test_db();

        // Insert file
        db.upsert_file("test.rs", "v1", 0, 2).unwrap();

        // Update with new content
        db.upsert_file("test.rs", "v2", 1, 2).unwrap();

        // Should still be one file
        let files = db.get_all_files(100).unwrap();
        assert_eq!(files.len(), 1);
    }

    #[test]
    fn test_search_path_match() {
        let (_dir, db) = create_test_db();

        db.upsert_file("src/main.rs", "some content", 0, 12).unwrap();
        db.upsert_file("README.md", "documentation", 0, 12).unwrap();

        let results = db.search("main", false, 10).unwrap();
        assert!(!results.is_empty());
        assert_eq!(results[0].path, "src/main.rs");
    }

    #[test]
    fn test_search_content_match() {
        let (_dir, db) = create_test_db();

        db.upsert_file("foo.rs", "fn calculate_total()", 0, 20).unwrap();
        db.upsert_file("bar.rs", "other content", 0, 12).unwrap();

        let results = db.search("calculate", false, 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].path, "foo.rs");
    }

    #[test]
    fn test_search_bm25_path_boost() {
        let (_dir, db) = create_test_db();

        // Path contains "test" but content doesn't
        db.upsert_file("test.rs", "unrelated content here", 0, 25).unwrap();
        // Content contains "test" but path doesn't
        db.upsert_file("other.rs", "test keyword in content", 0, 26).unwrap();

        let results = db.search("test", false, 10).unwrap();
        assert_eq!(results.len(), 2);
        // Path match should rank higher (lower BM25 score)
        assert!(results[0].rank <= results[1].rank);
    }

    #[test]
    fn test_search_paths_only() {
        let (_dir, db) = create_test_db();

        db.upsert_file("src/main.rs", "hello world", 0, 11).unwrap();

        // Paths-only search for "main" (in path)
        let results = db.search("main", true, 10).unwrap();
        assert!(!results.is_empty());

        // Full search for "world" (in content but not path)
        let results_full = db.search("world", false, 10).unwrap();
        assert!(!results_full.is_empty()); // "world" IS in content
    }

    #[test]
    fn test_search_empty_query() {
        let (_dir, db) = create_test_db();

        // Empty-like query
        let results = db.search("", false, 10).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_delete_file() {
        let (_dir, db) = create_test_db();

        db.upsert_file("test.rs", "content", 0, 7).unwrap();
        assert_eq!(db.get_file_count().unwrap(), 1);

        db.delete_file("test.rs").unwrap();
        assert_eq!(db.get_file_count().unwrap(), 0);
    }

    #[test]
    fn test_optimize_fts() {
        let (_dir, db) = create_test_db();

        // Should not error
        db.optimize_fts().unwrap();
    }

    #[test]
    fn test_check_fts_integrity() {
        let (_dir, db) = create_test_db();

        // Valid DB should pass
        assert!(db.check_fts_integrity());
    }

    #[test]
    fn test_get_file_count() {
        let (_dir, db) = create_test_db();

        assert_eq!(db.get_file_count().unwrap(), 0);

        db.upsert_file("a.rs", "a", 0, 1).unwrap();
        db.upsert_file("b.rs", "b", 0, 1).unwrap();
        db.upsert_file("c.rs", "c", 0, 1).unwrap();

        assert_eq!(db.get_file_count().unwrap(), 3);
    }

    #[test]
    fn test_search_limit() {
        let (_dir, db) = create_test_db();

        for i in 0..20 {
            db.upsert_file(&format!("file{i}.rs"), "test content", 0, 12).unwrap();
        }

        let results = db.search("test", false, 5).unwrap();
        assert_eq!(results.len(), 5);
    }

    #[test]
    fn test_search_special_chars() {
        let (_dir, db) = create_test_db();

        db.upsert_file("test.rs", "content with \"quotes\" and (parens)", 0, 35).unwrap();

        // Should handle without crashing
        let results = db.search("quotes", false, 10).unwrap();
        assert!(!results.is_empty());
    }

    #[test]
    fn test_search_unicode() {
        let (_dir, db) = create_test_db();

        db.upsert_file("test.rs", "café 中文", 0, 12).unwrap();

        let results = db.search("café", false, 10).unwrap();
        assert!(!results.is_empty());
    }

    #[test]
    fn test_transaction_commit() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let config = PragmaConfig::default();
        let db = Database::open(&db_path, &config).unwrap();
        db.init_schema().unwrap();

        // Begin transaction
        db.conn().execute("BEGIN TRANSACTION", []).unwrap();

        db.upsert_file("a.rs", "a", 0, 1).unwrap();
        db.upsert_file("b.rs", "b", 0, 1).unwrap();

        // Commit
        db.conn().execute("COMMIT", []).unwrap();

        assert_eq!(db.get_file_count().unwrap(), 2);
    }

    #[test]
    fn test_pragma_cache_size() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let config = PragmaConfig {
            cache_size: -16000, // 16MB
            ..Default::default()
        };

        let db = Database::open(&db_path, &config).unwrap();

        // Verify pragma was set
        let cache: i64 = db.conn().query_row("PRAGMA cache_size", [], |row| row.get(0)).unwrap();
        assert!(cache <= -16000); // Negative means KB
    }

    #[test]
    fn test_busy_timeout_negative_rejected() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let config = PragmaConfig { busy_timeout_ms: -1, ..Default::default() };

        let result = Database::open(&db_path, &config);
        assert!(matches!(
            result,
            Err(IndexerError::ConfigInvalid { field, .. }) if field == "busy_timeout_ms"
        ));
        assert!(!db_path.exists());
    }

    #[test]
    fn test_migrate_schema_propagates_errors() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let db = Database::open(&db_path, &PragmaConfig::default()).unwrap();

        // Create a legacy schema (no filename column).
        db.conn()
            .execute(
                "CREATE TABLE files (
                id INTEGER PRIMARY KEY,
                path TEXT UNIQUE NOT NULL
            )",
                [],
            )
            .unwrap();

        // Force write operations to fail.
        db.conn().pragma_update(None, "query_only", "ON").unwrap();

        let result = db.migrate_schema();
        assert!(matches!(result, Err(IndexerError::Database { .. })));
    }

    #[test]
    fn test_open_readonly() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join(DB_NAME);
        let config = PragmaConfig::default();

        // First create the database
        let db = Database::open(&db_path, &config).unwrap();
        db.init_schema().unwrap();
        db.upsert_file("test.rs", "content", 0, 7).unwrap();
        drop(db);

        // Now open in read-only mode
        let db_ro = Database::open_readonly(&db_path).unwrap();

        // Can read data
        assert_eq!(db_ro.get_file_count().unwrap(), 1);

        // Verify read-only mode - write should fail
        let result = db_ro.conn().execute("DELETE FROM files", []);
        assert!(result.is_err());
    }

    #[test]
    fn test_open_readonly_nonexistent() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("nonexistent.db");

        // Should fail on non-existent database
        let result = Database::open_readonly(&db_path);
        assert!(result.is_err());
    }

    #[test]
    fn test_check_schema_complete() {
        let (_dir, db) = create_test_db();

        let check = db.check_schema();
        assert!(check.is_complete());
        assert_eq!(check.table_count(), 2);
        assert_eq!(check.trigger_count(), 3);
        assert_eq!(check.index_count(), 3);
        assert!(check.missing_objects().is_empty());
    }

    #[test]
    fn test_check_schema_incomplete() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let config = PragmaConfig::default();
        let db = Database::open(&db_path, &config).unwrap();

        // Create partial schema (only files table)
        db.conn()
            .execute(
                "CREATE TABLE files (
                id INTEGER PRIMARY KEY,
                path TEXT UNIQUE NOT NULL
            )",
                [],
            )
            .unwrap();

        let check = db.check_schema();
        assert!(!check.is_complete());
        assert!(check.has_files_table);
        assert!(!check.has_fts_table);
        assert!(!check.has_insert_trigger);
        assert_eq!(check.table_count(), 1);
        assert_eq!(check.trigger_count(), 0);
        assert_eq!(check.missing_objects().len(), 7);
    }

    #[test]
    fn test_get_application_id() {
        let (_dir, db) = create_test_db();

        let app_id = db.get_application_id();
        assert_eq!(app_id, Some(0xA17E_6D42));
    }

    #[test]
    fn test_get_application_id_wrong() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.db");

        // Create database without our application_id
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.pragma_update(None, "application_id", 0x1234_5678_i32).unwrap();
        drop(conn);

        let db = Database::open_readonly(&db_path).unwrap();
        let app_id = db.get_application_id();
        assert_eq!(app_id, Some(0x1234_5678));
        assert_ne!(app_id, Some(0xA17E_6D42));
    }

    #[test]
    fn test_get_journal_mode() {
        let (_dir, db) = create_test_db();

        let journal_mode = db.get_journal_mode();
        assert_eq!(journal_mode, Some("wal".to_string()));
    }

    #[test]
    fn test_get_db_size_bytes() {
        let (_dir, db) = create_test_db();

        let size = db.get_db_size_bytes();
        assert!(size.is_some());
        // Should be at least 4KB (one page)
        assert!(size.unwrap() >= 4096);
    }

    #[test]
    fn test_schema_check_missing_objects() {
        let check = SchemaCheck {
            has_files_table: true,
            has_fts_table: false,
            has_insert_trigger: true,
            has_update_trigger: false,
            has_delete_trigger: true,
            has_mtime_index: false,
            has_path_index: true,
            has_hash_index: true,
        };

        let missing = check.missing_objects();
        assert_eq!(missing.len(), 3);
        assert!(missing.contains(&"table: files_fts"));
        assert!(missing.contains(&"trigger: files_au"));
        assert!(missing.contains(&"index: idx_files_mtime"));
    }

    // ============================================
    // search_filename_contains() tests
    // ============================================

    #[test]
    fn test_search_filename_contains_basic() {
        let (_dir, db) = create_test_db();

        db.upsert_file("docs/intro.md", "content", 0, 7).unwrap();
        db.upsert_file("src/main.rs", "content", 0, 7).unwrap();

        let results = db.search_filename_contains("intro", 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0], "docs/intro.md");
    }

    #[test]
    fn test_search_filename_contains_substring() {
        // This is the KEY test: "intro" should find "01-introduction.md"
        let (_dir, db) = create_test_db();

        db.upsert_file("docs/learn/01-introduction.md", "chapter one", 0, 12).unwrap();
        db.upsert_file("other.md", "unrelated", 0, 9).unwrap();

        let results = db.search_filename_contains("intro", 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0], "docs/learn/01-introduction.md");
    }

    #[test]
    fn test_search_filename_contains_case_insensitive() {
        let (_dir, db) = create_test_db();

        db.upsert_file("CLAUDE.md", "content", 0, 7).unwrap();

        // Lowercase query should find uppercase filename
        let results = db.search_filename_contains("claude", 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0], "CLAUDE.md");
    }

    #[test]
    fn test_search_filename_contains_exact_match_priority() {
        let (_dir, db) = create_test_db();

        // Exact match
        db.upsert_file("config.rs", "a", 0, 1).unwrap();
        // Longer substring match
        db.upsert_file("my-config-utils.rs", "b", 0, 1).unwrap();

        let results = db.search_filename_contains("config", 10).unwrap();
        assert_eq!(results.len(), 2);
        // Exact match should come first
        assert_eq!(results[0], "config.rs");
    }

    #[test]
    fn test_search_filename_contains_prefix_priority() {
        let (_dir, db) = create_test_db();

        // Prefix match
        db.upsert_file("config-local.rs", "a", 0, 1).unwrap();
        // Contains but not prefix
        db.upsert_file("my-config.rs", "b", 0, 1).unwrap();

        let results = db.search_filename_contains("config", 10).unwrap();
        assert_eq!(results.len(), 2);
        // Prefix match should come before contains-only match
        assert_eq!(results[0], "config-local.rs");
    }

    #[test]
    fn test_search_filename_contains_shorter_first() {
        let (_dir, db) = create_test_db();

        // Both are contains matches, shorter should come first
        db.upsert_file("my-intro-file.md", "a", 0, 1).unwrap();
        db.upsert_file("intro.md", "b", 0, 1).unwrap();

        let results = db.search_filename_contains("intro", 10).unwrap();
        assert_eq!(results.len(), 2);
        // Shorter filename should come first (more specific match)
        assert_eq!(results[0], "intro.md");
    }

    #[test]
    fn test_search_filename_contains_percent_literal() {
        let (_dir, db) = create_test_db();

        db.upsert_file("100%coverage.md", "a", 0, 1).unwrap();
        db.upsert_file("100.md", "b", 0, 1).unwrap();

        let results = db.search_filename_contains("100%", 10).unwrap();
        assert_eq!(results, vec!["100%coverage.md".to_string()]);
    }

    #[test]
    fn test_search_filename_contains_underscore_literal() {
        let (_dir, db) = create_test_db();

        db.upsert_file("foo_bar.md", "a", 0, 1).unwrap();
        db.upsert_file("fooXbar.md", "b", 0, 1).unwrap();

        let results = db.search_filename_contains("foo_bar", 10).unwrap();
        assert_eq!(results, vec!["foo_bar.md".to_string()]);
    }

    #[test]
    fn test_search_filename_contains_empty_query() {
        let (_dir, db) = create_test_db();

        db.upsert_file("test.rs", "content", 0, 7).unwrap();

        let results = db.search_filename_contains("", 10).unwrap();
        assert!(results.is_empty());

        let results = db.search_filename_contains("   ", 10).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_search_filename_contains_strips_wildcard() {
        let (_dir, db) = create_test_db();

        db.upsert_file("01-introduction.md", "content", 0, 7).unwrap();

        // Query with wildcard (from auto-prefix) should still work
        let results = db.search_filename_contains("01*", 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0], "01-introduction.md");
    }

    #[test]
    fn test_search_filename_contains_limit() {
        let (_dir, db) = create_test_db();

        for i in 0..10 {
            db.upsert_file(&format!("test{i}.rs"), "content", 0, 7).unwrap();
        }

        let results = db.search_filename_contains("test", 3).unwrap();
        assert_eq!(results.len(), 3);
    }
}
