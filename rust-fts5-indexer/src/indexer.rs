use ignore::{DirEntry, WalkBuilder};
use std::fs;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::db::Database;
use crate::error::{IndexerError, Result};
use crate::fs_utils::{sync_file, sync_parent_dir};
use crate::{DB_NAME, DB_SHM_SUFFIX, DB_TMP_NAME, DB_TMP_SUFFIX, DB_WAL_SUFFIX};
#[cfg(windows)]
use std::os::windows::ffi::OsStrExt;

/// Configuration for the indexer.
#[derive(Debug, Clone)]
pub struct IndexerConfig {
    /// Maximum file size to index (in bytes)
    pub max_file_size: u64,
    /// Files per transaction batch
    pub batch_size: usize,
    /// Follow symlinks (disabled by default)
    pub follow_symlinks: bool,
}

impl Default for IndexerConfig {
    fn default() -> Self {
        Self {
            max_file_size: 1024 * 1024, // 1MB
            batch_size: 500,
            follow_symlinks: false,
        }
    }
}

/// Statistics from an indexing operation.
#[derive(Debug, Default)]
pub struct IndexStats {
    pub files_indexed: u64,
    pub files_skipped: u64,
    pub bytes_indexed: u64,
    pub duration: Duration,
}

#[cfg(windows)]
fn atomic_replace(from: &Path, to: &Path) -> Result<()> {
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };

    let from_wide: Vec<u16> = from.as_os_str().encode_wide().chain(Some(0)).collect();
    let to_wide: Vec<u16> = to.as_os_str().encode_wide().chain(Some(0)).collect();

    let result = unsafe {
        MoveFileExW(
            from_wide.as_ptr(),
            to_wide.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };

    if result == 0 {
        return Err(IndexerError::Io { source: std::io::Error::last_os_error() });
    }

    Ok(())
}

#[cfg(not(windows))]
fn atomic_replace(from: &Path, to: &Path) -> Result<()> {
    fs::rename(from, to).map_err(|e| IndexerError::Io { source: e })
}

/// FTS5 file indexer.
///
/// Uses the `ignore` crate for gitignore-aware directory walking.
pub struct Indexer {
    db: Database,
    root: PathBuf,
    root_canonical: PathBuf,
    config: IndexerConfig,
}

impl Indexer {
    /// Create a new indexer for the given project root.
    pub fn new(root: &Path, db: Database, config: IndexerConfig) -> Self {
        let root_canonical = root.canonicalize().unwrap_or_else(|err| {
            tracing::warn!(
                path = %root.display(),
                error = %err,
                "Failed to canonicalize root; symlink containment checks may be overly strict"
            );
            root.to_path_buf()
        });
        Self { db, root: root.to_path_buf(), root_canonical, config }
    }

    /// Index all files in the project directory (incremental).
    ///
    /// # Errors
    /// Returns `IndexerError` if:
    /// - Database operations fail (see [`Database::upsert_file`](crate::Database::upsert_file))
    /// - File I/O operations fail (reading file content)
    /// - Gitignore parsing fails
    pub fn index_directory(&mut self) -> Result<IndexStats> {
        // Conditional transaction strategy (2025+ best practice)
        const TRANSACTION_THRESHOLD: usize = 50;

        let start = SystemTime::now();

        // Use ignore crate for gitignore-aware walking
        let walk = WalkBuilder::new(&self.root)
            .standard_filters(true) // Respect .gitignore
            .same_file_system(true) // Prevent crossing filesystems
            .follow_links(self.config.follow_symlinks)
            .build();

        let mut stats = IndexStats::default();
        let mut batch_count = 0;
        let mut transaction_started = false;

        for result in walk {
            match result {
                Ok(entry) => {
                    // Process with batch tracking
                    match self.process_entry(&entry, &mut stats) {
                        Ok(needs_commit) => {
                            if needs_commit {
                                batch_count += 1;

                                // Start transaction after hitting threshold
                                if batch_count == TRANSACTION_THRESHOLD && !transaction_started {
                                    self.db
                                        .conn()
                                        .execute("BEGIN IMMEDIATE", [])
                                        .map_err(|e| IndexerError::Database { source: e })?;
                                    transaction_started = true;
                                }

                                // Batched commits for large operations
                                if transaction_started && batch_count >= self.config.batch_size {
                                    self.db
                                        .conn()
                                        .execute("COMMIT", [])
                                        .map_err(|e| IndexerError::Database { source: e })?;
                                    self.db
                                        .conn()
                                        .execute("BEGIN IMMEDIATE", [])
                                        .map_err(|e| IndexerError::Database { source: e })?;
                                    batch_count = TRANSACTION_THRESHOLD; // Reset to threshold, not 0
                                }
                            }
                        }
                        Err(e @ IndexerError::Database { .. }) => {
                            if transaction_started {
                                let _ = self.db.conn().execute("ROLLBACK", []);
                            }
                            return Err(e);
                        }
                        Err(e) => {
                            // Log and continue - single file errors shouldn't fail the index
                            tracing::warn!(
                                path = %entry.path().display(),
                                error = %e,
                                "Failed to index file"
                            );
                            stats.files_skipped += 1;
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        "Directory walk error"
                    );
                }
            }
        }

        // Commit final batch if transaction was started
        if transaction_started {
            self.db
                .conn()
                .execute("COMMIT", [])
                .map_err(|e| IndexerError::Database { source: e })?;
        }

        let pruned = self.db.prune_missing_files(&self.root)?;
        if pruned > 0 {
            tracing::info!(pruned, "Pruned missing files");
        }

        // SQLite-GUIDELINES.md: Run ANALYZE after bulk changes for query optimization
        self.db.conn().execute("ANALYZE", []).ok();

        // 2025+ best practice: PRAGMA optimize updates query planner statistics
        self.db.optimize().ok();

        // 2025+ best practice: FTS5 OPTIMIZE defragments index after >10% row changes
        self.db.optimize_fts().ok();

        stats.duration = start.elapsed().unwrap_or_default();
        Ok(stats)
    }

    /// Process a single directory entry.
    fn process_entry(&self, entry: &DirEntry, stats: &mut IndexStats) -> Result<bool> {
        let path = entry.path();

        // Skip the database file itself
        if Self::is_database_file(path) {
            return Ok(false);
        }

        // Check if it's a symlink (symlink_metadata avoids following links).
        let is_symlink = match fs::symlink_metadata(path) {
            Ok(metadata) => metadata.file_type().is_symlink(),
            Err(e) => {
                tracing::warn!(
                    path = %path.display(),
                    error = %e,
                    "Failed to read symlink metadata"
                );
                stats.files_skipped += 1;
                return Ok(false);
            }
        };

        if is_symlink {
            if !self.config.follow_symlinks {
                stats.files_skipped += 1;
                return Ok(false);
            }

            // Resolve symlink and verify it's within root
            if let Ok(resolved) = fs::canonicalize(path) {
                if !self.is_within_root(&resolved) {
                    tracing::warn!(
                        path = %path.display(),
                        resolved = %resolved.display(),
                        "Skipping symlink that escapes project root"
                    );
                    stats.files_skipped += 1;
                    return Ok(false);
                }
            } else {
                stats.files_skipped += 1;
                return Ok(false);
            }
        }

        // Skip directories (only index files)
        if entry.file_type().is_some_and(|ft| ft.is_dir()) {
            return Ok(false);
        }

        // Get metadata
        let metadata = entry.metadata()?;

        // Skip files larger than max size
        if metadata.len() > self.config.max_file_size {
            stats.files_skipped += 1;
            return Ok(false);
        }

        // Read file content
        let content = match self.read_file_content(path, metadata.len()) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(
                    path = %path.display(),
                    error = %e,
                    "Failed to read file content"
                );
                stats.files_skipped += 1;
                return Ok(false);
            }
        };

        // Upsert into database - use relative path from root
        let rel_path = path.strip_prefix(&self.root).map_err(|_| IndexerError::PathTraversal {
            path: path.to_string_lossy().to_string(),
        })?;

        // Cross-platform mtime using SystemTime (Windows compatible)
        let mtime_secs = metadata
            .modified()
            .map_err(|e| IndexerError::Io { source: e })?
            .duration_since(UNIX_EPOCH)
            .map_err(|e| IndexerError::Io {
                source: std::io::Error::other(format!("Invalid mtime: {e}")),
            })?
            .as_secs();
        let mtime = Self::checked_i64_from_u64(mtime_secs, "mtime")?;

        let size = metadata.len();
        let size_i64 = Self::checked_i64_from_u64(size, "file size")?;
        self.db.upsert_file(&rel_path.to_string_lossy(), &content, mtime, size_i64)?;

        stats.files_indexed += 1;
        stats.bytes_indexed += size;

        Ok(true)
    }

    /// Read file content with UTF-8 validation.
    ///
    /// # Memory Efficiency (2025+ best practice)
    /// - Direct file read (no intermediate `BufReader` buffer) when size is known
    /// - Pre-allocates `Vec<u8>` with known file size to avoid reallocation
    /// - Size limit protects against memory exhaustion
    /// - Explicit UTF-8 validation with `String::from_utf8`
    ///
    /// Note: FTS5 requires full content for indexing, so streaming is not possible.
    /// Memory protection is provided by `max_file_size` limit (default 1MB).
    fn read_file_content(&self, path: &Path, size: u64) -> Result<String> {
        // Check size limit first (fail fast)
        if size > self.config.max_file_size {
            return Err(IndexerError::FileTooLarge { size, max: self.config.max_file_size });
        }

        // Open file and read directly (no BufReader overhead when size is known)
        let file = File::open(path).map_err(|e| IndexerError::Io { source: e })?;

        // Pre-allocate Vec with known capacity to avoid reallocation
        let max_size = self.config.max_file_size;
        let capacity = std::cmp::min(size, max_size);
        // Safety: capacity ≤ max_file_size, which is bounded to sane values for indexing.
        #[allow(clippy::cast_possible_truncation)]
        let mut bytes = Vec::with_capacity(capacity as usize);

        // Read at most max_size + 1 bytes to detect concurrent growth beyond limit.
        let read_limit = max_size.saturating_add(1);
        file.take(read_limit)
            .read_to_end(&mut bytes)
            .map_err(|e| IndexerError::Io { source: e })?;

        if bytes.len() as u64 > max_size {
            return Err(IndexerError::FileTooLarge { size: bytes.len() as u64, max: max_size });
        }

        // Convert to String with explicit UTF-8 validation
        String::from_utf8(bytes)
            .map_err(|_| IndexerError::InvalidUtf8 { path: path.to_string_lossy().to_string() })
    }

    /// Check if a path is safely within the project root.
    ///
    /// # Performance
    /// Called for symlink resolution. Marked `#[inline]` for hot-path optimization.
    #[inline]
    fn is_within_root(&self, path: &Path) -> bool {
        // Path must start with canonical root prefix
        if let Ok(rel_path) = path.strip_prefix(&self.root_canonical) {
            // Ensure no ".." components that could escape
            for component in rel_path.components() {
                if component == std::path::Component::ParentDir {
                    return false;
                }
            }
            return true;
        }

        // Fallback: handle non-canonical paths (e.g., unit tests or callers)
        if let Ok(rel_path) = path.strip_prefix(&self.root) {
            for component in rel_path.components() {
                if component == std::path::Component::ParentDir {
                    return false;
                }
            }
            return true;
        }

        false
    }

    /// Check if path is a database file that should be skipped.
    ///
    /// # Performance
    /// Called for every file during directory walk. Marked `#[inline]` for hot-path optimization.
    #[inline]
    fn is_database_file(path: &Path) -> bool {
        let file_name = match path.file_name().and_then(|name| name.to_str()) {
            Some(name) => name,
            None => return false,
        };

        // Skip auxiliary WAL files
        if file_name.ends_with(DB_SHM_SUFFIX) || file_name.ends_with(DB_WAL_SUFFIX) {
            return true;
        }

        // Skip primary database file
        if file_name == DB_NAME {
            return true;
        }

        // Skip temp files from reindex/auto-init
        if file_name.ends_with(".db.tmp")
            || (file_name.starts_with(DB_NAME) && file_name.contains(DB_TMP_SUFFIX))
        {
            return true;
        }

        // Skip by extension
        if file_name.ends_with(".db")
            || file_name.ends_with(".sqlite")
            || file_name.ends_with(".sqlite3")
        {
            return true;
        }

        false
    }

    /// Force a commit of any pending transactions.
    ///
    /// # Errors
    /// Currently returns `Ok(())` (rusqlite auto-commits after each execute).
    /// This signature exists for API compatibility and future transaction batching.
    pub const fn flush(&self) -> Result<()> {
        // rusqlite auto-commits after each execute
        Ok(())
    }

    /// Get the database instance.
    pub const fn db(&self) -> &Database {
        &self.db
    }

    /// Get mutable database instance.
    pub const fn db_mut(&mut self) -> &mut Database {
        &mut self.db
    }

    /// Get the project root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Get the configuration.
    pub const fn config(&self) -> &IndexerConfig {
        &self.config
    }

    fn checked_i64_from_u64(value: u64, label: &'static str) -> Result<i64> {
        if value > i64::MAX as u64 {
            return Err(IndexerError::Io {
                source: std::io::Error::other(format!("{label} out of range: {value}")),
            });
        }
        Ok(value as i64)
    }
}

/// Atomic reindex - creates a new database and atomically replaces the old one.
///
/// # Errors
/// Returns `IndexerError` if:
/// - Temporary database creation fails
/// - Schema initialization fails
/// - Directory indexing fails (see [`Indexer::index_directory`])
/// - FTS5 optimization fails
/// - File system operations fail (atomic rename, WAL file cleanup)
pub fn atomic_reindex(root: &Path, config: &crate::db::PragmaConfig) -> Result<IndexStats> {
    atomic_reindex_with_config(root, config, IndexerConfig::default())
}

/// Atomic reindex with explicit indexer configuration.
///
/// Use this when you need to override defaults such as symlink handling.
pub fn atomic_reindex_with_config(
    root: &Path,
    config: &crate::db::PragmaConfig,
    indexer_config: IndexerConfig,
) -> Result<IndexStats> {
    let db_path = root.join(DB_NAME);
    let tmp_path = root.join(DB_TMP_NAME);

    // Clean up any existing temp file
    let _ = fs::remove_file(&tmp_path);

    // Create new database in temp location
    let db = Database::open(&tmp_path, config)?;
    db.init_schema()?;

    // Index all files
    let mut indexer = Indexer::new(root, db, indexer_config);
    let stats = indexer.index_directory()?;

    // Ensure WAL contents are checkpointed into the main database file before rename
    indexer
        .db
        .conn()
        .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
            let _busy: i64 = row.get(0)?;
            let _log: i64 = row.get(1)?;
            let _checkpointed: i64 = row.get(2)?;
            Ok(())
        })
        .map_err(|e| IndexerError::Database { source: e })?;

    // Close database before replacing file to avoid WAL/file descriptor issues
    drop(indexer);

    // Ensure data is flushed before renaming.
    // This reduces the risk of ending up with a zero-length or partially written file after a crash.
    sync_file(&tmp_path).map_err(|e| IndexerError::Io { source: e })?;

    // Atomic rename (Windows requires replace strategy)
    atomic_replace(&tmp_path, &db_path)?;

    // Ensure the rename is durable on filesystems that require directory fsync.
    sync_parent_dir(&db_path).map_err(|e| IndexerError::Io { source: e })?;

    // Clean up WAL files from old database (if exists) after rename
    let _ = fs::remove_file(root.join(format!("{DB_NAME}{DB_SHM_SUFFIX}")));
    let _ = fs::remove_file(root.join(format!("{DB_NAME}{DB_WAL_SUFFIX}")));

    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::PragmaConfig;
    use crate::{DB_NAME, DB_SHM_SUFFIX, DB_TMP_NAME, DB_TMP_SUFFIX, DB_WAL_SUFFIX};
    use std::path::Path;
    use tempfile::tempdir;

    #[test]
    fn test_index_empty_dir() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join(DB_NAME);
        let config = IndexerConfig::default();
        let db = Database::open(&db_path, &PragmaConfig::default()).unwrap();
        db.init_schema().unwrap();
        let mut indexer = Indexer::new(dir.path(), db, config);

        let stats = indexer.index_directory().unwrap();
        assert_eq!(stats.files_indexed, 0);
    }

    #[test]
    fn test_atomic_reindex_cleans_up_wal_files() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join(DB_NAME);
        let config = PragmaConfig::default();

        let db = Database::open(&db_path, &config).unwrap();
        db.init_schema().unwrap();
        drop(db);

        let shm_path = dir.path().join(format!("{DB_NAME}{DB_SHM_SUFFIX}"));
        let wal_path = dir.path().join(format!("{DB_NAME}{DB_WAL_SUFFIX}"));
        fs::write(&shm_path, b"fake shm").unwrap();
        fs::write(&wal_path, b"fake wal").unwrap();

        let stats = atomic_reindex(dir.path(), &config).unwrap();
        assert_eq!(stats.files_indexed, 0);
        assert!(db_path.exists());
        assert!(!shm_path.exists());
        assert!(!wal_path.exists());
    }

    #[test]
    fn test_atomic_reindex_skips_temp_database_file() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join(DB_NAME);
        let config = PragmaConfig::default();

        std::fs::write(dir.path().join("main.rs"), "fn main() {}").unwrap();

        let stats = atomic_reindex(dir.path(), &config).unwrap();
        assert_eq!(stats.files_indexed, 1);
        assert!(db_path.exists());

        let db = Database::open(&db_path, &config).unwrap();
        let files = db.get_all_files(10).unwrap();
        assert!(files.contains(&"main.rs".to_string()));

        let legacy_tmp_name = Path::new(DB_NAME)
            .with_extension(DB_TMP_SUFFIX.trim_start_matches('.'))
            .to_string_lossy()
            .to_string();

        assert!(!files.contains(&DB_TMP_NAME.to_string()));
        assert!(!files.contains(&legacy_tmp_name));
    }

    #[cfg(unix)]
    #[test]
    fn test_atomic_reindex_with_config_follows_symlinks() {
        use std::os::unix::fs::symlink;

        let dir = tempdir().unwrap();
        let config = PragmaConfig::default();
        let indexer_config = IndexerConfig { follow_symlinks: true, ..Default::default() };

        let real_dir = dir.path().join("real");
        fs::create_dir_all(&real_dir).unwrap();
        fs::write(real_dir.join("inner.rs"), "fn inner() {}").unwrap();
        let link_dir = dir.path().join("linkdir");
        symlink(&real_dir, &link_dir).unwrap();

        let stats = atomic_reindex_with_config(dir.path(), &config, indexer_config).unwrap();
        assert!(stats.files_indexed >= 1);

        let db = Database::open(&dir.path().join(DB_NAME), &config).unwrap();
        let files = db.get_all_files(10).unwrap();
        assert!(files.contains(&"real/inner.rs".to_string()));
        assert!(files.contains(&"linkdir/inner.rs".to_string()));
    }

    #[test]
    fn test_index_single_file() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join(DB_NAME);
        let config = IndexerConfig::default();
        let db = Database::open(&db_path, &PragmaConfig::default()).unwrap();
        db.init_schema().unwrap();
        let mut indexer = Indexer::new(dir.path(), db, config);

        // Create a test file
        std::fs::write(dir.path().join("test.rs"), "fn main() {}").unwrap();

        let stats = indexer.index_directory().unwrap();
        assert_eq!(stats.files_indexed, 1);
    }

    #[test]
    fn test_prunes_missing_files() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join(DB_NAME);
        let config = IndexerConfig::default();
        let db = Database::open(&db_path, &PragmaConfig::default()).unwrap();
        db.init_schema().unwrap();
        let mut indexer = Indexer::new(dir.path(), db, config);

        let file_path = dir.path().join("gone.rs");
        fs::write(&file_path, "fn gone() {}").unwrap();

        indexer.index_directory().unwrap();
        assert_eq!(indexer.db().get_file_count().unwrap(), 1);

        fs::remove_file(&file_path).unwrap();

        indexer.index_directory().unwrap();
        assert_eq!(indexer.db().get_file_count().unwrap(), 0);
    }

    #[test]
    fn test_index_nested_dirs() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join(DB_NAME);
        let config = IndexerConfig::default();
        let db = Database::open(&db_path, &PragmaConfig::default()).unwrap();
        db.init_schema().unwrap();
        let mut indexer = Indexer::new(dir.path(), db, config);

        // Create nested structure
        std::fs::create_dir_all(dir.path().join("src/lib")).unwrap();
        std::fs::write(dir.path().join("src/lib/utils.rs"), "// utils").unwrap();
        std::fs::write(dir.path().join("main.rs"), "// main").unwrap();

        let stats = indexer.index_directory().unwrap();
        assert_eq!(stats.files_indexed, 2);
    }

    #[test]
    fn test_skips_binary_files() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join(DB_NAME);
        let config = IndexerConfig::default();
        let db = Database::open(&db_path, &PragmaConfig::default()).unwrap();
        db.init_schema().unwrap();
        let mut indexer = Indexer::new(dir.path(), db, config);

        // Create a binary file with invalid UTF-8
        let binary_content = [0x80, 0x81, 0x82, 0xff];
        std::fs::write(dir.path().join("binary.bin"), binary_content).unwrap();

        let stats = indexer.index_directory().unwrap();
        assert_eq!(stats.files_indexed, 0);
        assert_eq!(stats.files_skipped, 1);
    }

    #[test]
    fn test_skips_large_files() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join(DB_NAME);
        let config = IndexerConfig::default();
        let db = Database::open(&db_path, &PragmaConfig::default()).unwrap();
        db.init_schema().unwrap();
        let mut indexer = Indexer::new(dir.path(), db, config);

        // Create a file larger than 1MB
        let large_content = vec![0u8; 1024 * 1024 + 1];
        std::fs::write(dir.path().join("large.bin"), large_content).unwrap();

        let stats = indexer.index_directory().unwrap();
        assert_eq!(stats.files_indexed, 0);
        assert_eq!(stats.files_skipped, 1);
    }

    #[test]
    fn test_index_directory_fails_on_database_error() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join(DB_NAME);
        let db = Database::open(&db_path, &PragmaConfig::default()).unwrap();
        db.init_schema().unwrap();

        // Force database writes to fail.
        db.conn().pragma_update(None, "query_only", "ON").unwrap();

        std::fs::write(dir.path().join("test.rs"), "fn main() {}").unwrap();

        let mut indexer = Indexer::new(dir.path(), db, IndexerConfig::default());
        let result = indexer.index_directory();

        assert!(matches!(result, Err(IndexerError::Database { .. })));
    }

    #[test]
    fn test_respects_gitignore() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join(DB_NAME);
        let config = IndexerConfig::default();
        let db = Database::open(&db_path, &PragmaConfig::default()).unwrap();
        db.init_schema().unwrap();
        let mut indexer = Indexer::new(dir.path(), db, config);

        // Test that .git directory is skipped by standard_filters
        std::fs::create_dir_all(dir.path().join(".git/objects")).unwrap();
        std::fs::write(dir.path().join(".git/config"), "git config").unwrap();
        std::fs::write(dir.path().join("visible.rs"), "// visible").unwrap();

        let stats = indexer.index_directory().unwrap();
        assert_eq!(stats.files_indexed, 1); // Only visible.rs (.git should be skipped)
    }

    #[test]
    fn test_skips_database_file() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join(DB_NAME);
        let config = IndexerConfig::default();
        let db = Database::open(&db_path, &PragmaConfig::default()).unwrap();
        db.init_schema().unwrap();
        let mut indexer = Indexer::new(dir.path(), db, config);

        // Create database-like files
        std::fs::write(dir.path().join("test.rs"), "// test").unwrap();
        std::fs::write(dir.path().join("data.db"), "// db").unwrap();
        std::fs::write(dir.path().join("data.db-shm"), "// shm").unwrap();
        std::fs::write(dir.path().join("data.db-wal"), "// wal").unwrap();

        let stats = indexer.index_directory().unwrap();
        assert_eq!(stats.files_indexed, 1); // Only test.rs
    }

    #[test]
    fn test_skips_temp_database_suffix_variants() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join(DB_NAME);
        let config = IndexerConfig::default();
        let db = Database::open(&db_path, &PragmaConfig::default()).unwrap();
        db.init_schema().unwrap();
        let mut indexer = Indexer::new(dir.path(), db, config);

        std::fs::write(dir.path().join("file.rs"), "// real file").unwrap();
        std::fs::write(dir.path().join(format!("{DB_NAME}{DB_TMP_SUFFIX}.1234")), "temp db")
            .unwrap();

        let stats = indexer.index_directory().unwrap();
        assert_eq!(stats.files_indexed, 1);

        let files = indexer.db().get_all_files(10).unwrap();
        assert!(files.contains(&"file.rs".to_string()));
        assert!(!files.contains(&format!("{DB_NAME}{DB_TMP_SUFFIX}.1234")));
    }

    #[test]
    fn test_is_within_root() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join(DB_NAME);
        let config = IndexerConfig::default();
        let db = Database::open(&db_path, &PragmaConfig::default()).unwrap();
        let indexer = Indexer::new(dir.path(), db, config);

        // Path within root
        let test_file = dir.path().join("test.rs");
        assert!(indexer.is_within_root(&test_file));

        // Path outside root
        let outside = PathBuf::from("/etc/passwd");
        assert!(!indexer.is_within_root(&outside));
    }

    #[cfg(unix)]
    #[test]
    fn test_is_within_root_symlink_root() {
        use std::os::unix::fs::symlink;

        let dir = tempdir().unwrap();
        let real_root = dir.path().join("real");
        fs::create_dir_all(&real_root).unwrap();
        let link_root = dir.path().join("link");
        symlink(&real_root, &link_root).unwrap();

        let db_path = link_root.join(DB_NAME);
        let db = Database::open(&db_path, &PragmaConfig::default()).unwrap();
        db.init_schema().unwrap();
        let indexer = Indexer::new(&link_root, db, IndexerConfig::default());

        let inside = real_root.join("inside.rs");
        fs::write(&inside, "fn main() {}").unwrap();
        let canonical = fs::canonicalize(&inside).unwrap();

        assert!(
            indexer.is_within_root(&canonical),
            "Canonicalized path inside symlinked root should be allowed"
        );
    }

    #[test]
    fn test_read_file_content_rejects_growth_beyond_max() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join(DB_NAME);
        let config = IndexerConfig { max_file_size: 4, ..Default::default() };
        let db = Database::open(&db_path, &PragmaConfig::default()).unwrap();
        db.init_schema().unwrap();
        let indexer = Indexer::new(dir.path(), db, config);

        let file_path = dir.path().join("grow.txt");
        std::fs::write(&file_path, "0123456789").unwrap();

        let result = indexer.read_file_content(&file_path, 4);
        assert!(matches!(result, Err(IndexerError::FileTooLarge { .. })));
    }

    #[test]
    fn test_read_file_content_max_size_no_overflow() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join(DB_NAME);
        let config = IndexerConfig { max_file_size: u64::MAX, ..Default::default() };
        let db = Database::open(&db_path, &PragmaConfig::default()).unwrap();
        db.init_schema().unwrap();
        let indexer = Indexer::new(dir.path(), db, config);

        let file_path = dir.path().join("tiny.txt");
        fs::write(&file_path, "hi").unwrap();

        let content = indexer.read_file_content(&file_path, 2).unwrap();
        assert_eq!(content, "hi");
    }

    #[test]
    fn test_checked_i64_from_u64_ok() {
        let value = Indexer::checked_i64_from_u64(42, "value").unwrap();
        assert_eq!(value, 42);
    }

    #[test]
    fn test_checked_i64_from_u64_overflow() {
        let err = Indexer::checked_i64_from_u64(u64::MAX, "value").unwrap_err();
        let message = err.to_string();
        assert!(message.contains("out of range"));
    }

    #[cfg(unix)]
    #[test]
    fn test_symlink_file_skipped_by_default() {
        use std::os::unix::fs::symlink;

        let dir = tempdir().unwrap();
        let db_path = dir.path().join(DB_NAME);
        let db = Database::open(&db_path, &PragmaConfig::default()).unwrap();
        db.init_schema().unwrap();
        let config = IndexerConfig::default();
        let mut indexer = Indexer::new(dir.path(), db, config);

        let target = dir.path().join("target.rs");
        fs::write(&target, "fn main() {}").unwrap();
        let link = dir.path().join("link.rs");
        symlink(&target, &link).unwrap();

        let stats = indexer.index_directory().unwrap();
        assert_eq!(stats.files_indexed, 1);
        assert_eq!(stats.files_skipped, 1);

        let files = indexer.db().get_all_files(10).unwrap();
        assert!(files.contains(&"target.rs".to_string()));
        assert!(!files.contains(&"link.rs".to_string()));
    }

    #[cfg(unix)]
    #[test]
    fn test_symlink_file_follow_enabled() {
        use std::os::unix::fs::symlink;

        let dir = tempdir().unwrap();
        let db_path = dir.path().join(DB_NAME);
        let db = Database::open(&db_path, &PragmaConfig::default()).unwrap();
        db.init_schema().unwrap();
        let config = IndexerConfig { follow_symlinks: true, ..Default::default() };
        let mut indexer = Indexer::new(dir.path(), db, config);

        let target = dir.path().join("target.rs");
        fs::write(&target, "fn main() {}").unwrap();
        let link = dir.path().join("link.rs");
        symlink(&target, &link).unwrap();

        let stats = indexer.index_directory().unwrap();
        assert_eq!(stats.files_indexed, 2);

        let files = indexer.db().get_all_files(10).unwrap();
        assert!(files.contains(&"target.rs".to_string()));
        assert!(files.contains(&"link.rs".to_string()));
    }

    #[cfg(unix)]
    #[test]
    fn test_symlink_dir_follow_enabled() {
        use std::os::unix::fs::symlink;

        let dir = tempdir().unwrap();
        let db_path = dir.path().join(DB_NAME);
        let db = Database::open(&db_path, &PragmaConfig::default()).unwrap();
        db.init_schema().unwrap();
        let config = IndexerConfig { follow_symlinks: true, ..Default::default() };
        let mut indexer = Indexer::new(dir.path(), db, config);

        let real_dir = dir.path().join("real");
        fs::create_dir_all(&real_dir).unwrap();
        fs::write(real_dir.join("inner.rs"), "fn inner() {}").unwrap();

        let link_dir = dir.path().join("linkdir");
        symlink(&real_dir, &link_dir).unwrap();

        let stats = indexer.index_directory().unwrap();
        assert_eq!(stats.files_indexed, 2);

        let files = indexer.db().get_all_files(10).unwrap();
        assert!(files.contains(&"real/inner.rs".to_string()));
        assert!(files.contains(&"linkdir/inner.rs".to_string()));
    }

    #[cfg(unix)]
    #[test]
    fn test_symlink_escape_skipped() {
        use std::os::unix::fs::symlink;

        let dir = tempdir().unwrap();
        let outside = tempdir().unwrap();
        let external = outside.path().join("external.rs");
        fs::write(&external, "fn external() {}").unwrap();

        let db_path = dir.path().join(DB_NAME);
        let db = Database::open(&db_path, &PragmaConfig::default()).unwrap();
        db.init_schema().unwrap();
        let config = IndexerConfig { follow_symlinks: true, ..Default::default() };
        let mut indexer = Indexer::new(dir.path(), db, config);

        let link = dir.path().join("escape.rs");
        symlink(&external, &link).unwrap();

        let stats = indexer.index_directory().unwrap();
        assert_eq!(stats.files_indexed, 0);
        assert_eq!(stats.files_skipped, 1);

        let files = indexer.db().get_all_files(10).unwrap();
        assert!(files.is_empty());
    }
}
