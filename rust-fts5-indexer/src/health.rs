//! Database health checking and project root detection.
//!
//! This module provides fast health checks (<100us) for auto-init functionality.
//! Unlike `doctor.rs` (user diagnostics), this is optimized for programmatic use.
//!
//! # Key Features
//!
//! - **Project root detection**: Walks up from CWD to find `.ffts-index.db` or `.git`
//! - **Fast health checks**: Sub-100us checks for hot path usage
//! - **Atomic auto-init**: Race-condition-safe database initialization
//! - **Corruption recovery**: Backup and reinitialize corrupted databases
//!
//! # Concurrency & TOCTOU Considerations
//!
//! The health checking functions provide a **snapshot** of database state. Between
//! calling [`check_health_fast`] and acting on the result, the state may change
//! (classic TOCTOU - Time-Of-Check-Time-Of-Use).
//!
//! This is acceptable for the auto-init use case because:
//! - If another process initializes between check and action, `auto_init` detects
//!   the existing database and skips the rename (no data loss)
//! - Concurrent auto-inits produce functionally identical databases
//! - The alternative (holding locks during health checks) would significantly
//!   impact performance for the common case
//!
//! For strict consistency requirements, use proper database locking or
//! the `doctor` module which performs comprehensive validation.
//!
//! # Example
//!
//! ```rust,no_run
//! use ffts_indexer::health::{find_project_root, check_health_fast, auto_init, DatabaseHealth};
//! use ffts_indexer::db::PragmaConfig;
//! use std::path::Path;
//!
//! let root = find_project_root(Path::new("."));
//! let health = check_health_fast(&root.path);
//!
//! if health.needs_init() {
//!     auto_init(&root.path, &PragmaConfig::default(), false)?;
//! }
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use std::fs;
use std::path::{Path, PathBuf};

use crate::constants::EXPECTED_APPLICATION_ID;
use crate::db::{Database, PragmaConfig};
use crate::error::Result;
use crate::fs_utils::{sync_file, sync_parent_dir};
use crate::indexer::{IndexStats, Indexer, IndexerConfig};
use crate::{DB_NAME, DB_TMP_SUFFIX, init};

/// Database health status for auto-init decisions.
///
/// Variants are ordered by severity - higher variants indicate worse states.
/// Uses `#[non_exhaustive]` to allow future extension without breaking changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DatabaseHealth {
    /// Database exists, schema complete, has indexed content.
    Healthy,

    /// Schema exists but contains zero indexed files.
    /// Action: Run indexing only (skip schema init).
    Empty,

    /// No database file found at expected location.
    /// Action: Run full init + index.
    Missing,

    /// Database file exists but cannot be opened (permissions, locked, etc.).
    /// Action: Check permissions or wait for lock release.
    Unreadable,

    /// Database opens but has wrong `application_id` (not ours).
    /// Action: **NEVER auto-delete** - require manual resolution.
    WrongApplicationId,

    /// Database opens but schema is incomplete (missing tables/triggers/indexes).
    /// Action: Backup, delete, and reinitialize.
    SchemaInvalid,

    /// FTS5 integrity check failed or other corruption detected.
    /// Action: Backup, delete, and reinitialize.
    Corrupted,
}

impl DatabaseHealth {
    /// Returns true if the database can be used for searching.
    #[must_use]
    pub const fn is_usable(&self) -> bool {
        matches!(self, Self::Healthy)
    }

    /// Returns true if auto-init should be attempted.
    #[must_use]
    pub const fn needs_init(&self) -> bool {
        matches!(self, Self::Missing | Self::Empty)
    }

    /// Returns true if database needs backup + reinit.
    #[must_use]
    pub const fn needs_reinit(&self) -> bool {
        matches!(self, Self::SchemaInvalid | Self::Corrupted)
    }

    /// Returns true if this is an unrecoverable state requiring user action.
    #[must_use]
    pub const fn is_unrecoverable(&self) -> bool {
        matches!(self, Self::WrongApplicationId | Self::Unreadable)
    }
}

/// Method used to detect project root.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetectionMethod {
    /// Found existing `.ffts-index.db` file (highest priority).
    ExistingDatabase,

    /// Found `.git` directory (fallback).
    GitRepository,

    /// Used provided/current directory as-is.
    Fallback,
}

/// Project root detection result.
#[derive(Debug, Clone)]
pub struct ProjectRoot {
    /// The detected project root path.
    pub path: PathBuf,

    /// How the root was detected.
    pub method: DetectionMethod,
}

/// Check if a database file is a valid ffts-grep database.
///
/// Returns `true` only if all conditions are met:
/// 1. File exists
/// 2. File is a readable SQLite database
/// 3. File has our `application_id` (`EXPECTED_APPLICATION_ID`)
///
/// This prevents corrupt, empty, or foreign databases from being
/// used as project root markers.
///
/// # Performance
///
/// Opens database read-only, checks single PRAGMA. Target: <1ms.
fn is_valid_ffts_database(db_path: &Path) -> bool {
    if !db_path.exists() {
        return false;
    }

    let Ok(db) = Database::open_readonly(db_path) else {
        return false;
    };

    matches!(db.get_application_id(), Some(id) if id == EXPECTED_APPLICATION_ID)
}

/// Find project root using SINGLE-PASS algorithm.
///
/// Walks up the directory tree from `start_dir` looking for markers
/// that indicate a project root.
///
/// # Priority Order
///
/// 1. Directory containing existing `.ffts-index.db` (highest priority)
/// 2. Directory containing `.git`
/// 3. The original `start_dir` (fallback)
///
/// # Performance
///
/// Uses `Path::ancestors()` for efficient traversal with single allocation.
/// Target: <100us for typical directory depths.
///
/// # Arguments
///
/// * `start_dir` - Directory to start searching from (typically CWD)
///
/// # Returns
///
/// `ProjectRoot` containing the detected path and detection method.
#[must_use]
pub fn find_project_root(start_dir: &Path) -> ProjectRoot {
    let mut git_root: Option<PathBuf> = None;

    // Single pass: check both markers, valid database takes priority
    for ancestor in start_dir.ancestors() {
        let db_path = ancestor.join(DB_NAME);

        // Only use database as marker if it's OUR valid database
        // (prevents corrupt/empty/foreign databases from hijacking project root)
        if is_valid_ffts_database(&db_path) {
            return ProjectRoot {
                path: ancestor.to_path_buf(),
                method: DetectionMethod::ExistingDatabase,
            };
        }

        // Remember first .git found as fallback (nearest ancestor wins)
        if git_root.is_none() && ancestor.join(".git").exists() {
            git_root = Some(ancestor.to_path_buf());
        }
    }

    // Return git root if found, otherwise fallback to start_dir
    match git_root {
        Some(path) => ProjectRoot { path, method: DetectionMethod::GitRepository },
        None => ProjectRoot { path: start_dir.to_path_buf(), method: DetectionMethod::Fallback },
    }
}

/// Fast health check (skips FTS5 integrity for <100us performance).
///
/// Performs quick validation of database state without expensive integrity checks.
/// Use this for hot paths where speed matters more than comprehensive validation.
///
/// # Check Order (fail-fast)
///
/// 1. Database file exists
/// 2. Database readable (open with `SQLITE_OPEN_READ_ONLY`)
/// 3. Application ID matches `EXPECTED_APPLICATION_ID`
/// 4. Schema complete (tables, triggers, indexes)
/// 5. File count > 0
///
/// Note: Does NOT run FTS5 integrity check (that requires write access).
///
/// # Arguments
///
/// * `project_dir` - Directory containing the database
///
/// # Returns
///
/// `DatabaseHealth` indicating current state.
#[must_use]
pub fn check_health_fast(project_dir: &Path) -> DatabaseHealth {
    let db_path = project_dir.join(DB_NAME);

    // Check 1: File exists
    if !db_path.exists() {
        return DatabaseHealth::Missing;
    }

    // Check 2: Can open read-only
    let db = match Database::open_readonly(&db_path) {
        Ok(db) => db,
        Err(_) => return DatabaseHealth::Unreadable,
    };

    // Check 3: Application ID matches
    match db.get_application_id() {
        Some(id) if id == EXPECTED_APPLICATION_ID => {}
        Some(_) => return DatabaseHealth::WrongApplicationId,
        None => return DatabaseHealth::Corrupted,
    }

    // Check 4: Schema complete
    if !db.check_schema().is_complete() {
        return DatabaseHealth::SchemaInvalid;
    }

    // Check 5: Has content
    match db.get_file_count() {
        Ok(0) => DatabaseHealth::Empty,
        Ok(_) => DatabaseHealth::Healthy,
        Err(_) => DatabaseHealth::Corrupted,
    }
}

/// Auto-initialize database with atomic pattern to prevent race conditions.
///
/// Uses `.tmp` file + atomic rename to handle concurrent init attempts safely.
/// If another process wins the race, the loser's temp file is cleaned up.
///
/// # Atomicity
///
/// 1. Creates database at `{DB_NAME}{DB_TMP_SUFFIX}.{unique_suffix}`
/// 2. Initializes schema and indexes files
/// 3. Checkpoints WAL to main file
/// 4. Atomically renames to `{DB_NAME}`
/// 5. If rename fails (race lost), cleans up temp file
///
/// # Arguments
///
/// * `project_dir` - Directory to initialize
/// * `config` - SQLite PRAGMA configuration
/// * `quiet` - Suppress progress logging
///
/// # Errors
///
/// Returns `IndexerError` if database creation or indexing fails.
/// Race condition (another process won) is handled gracefully, not as error.
pub fn auto_init(project_dir: &Path, config: &PragmaConfig, quiet: bool) -> Result<IndexStats> {
    auto_init_with_config(project_dir, config, IndexerConfig::default(), quiet)
}

/// Auto-initialize database with explicit indexer configuration.
///
/// Use this to override indexing defaults (e.g., symlink handling).
pub fn auto_init_with_config(
    project_dir: &Path,
    config: &PragmaConfig,
    indexer_config: IndexerConfig,
    quiet: bool,
) -> Result<IndexStats> {
    // Update gitignore first (idempotent operation)
    let _ = init::update_gitignore(project_dir);

    let db_path = project_dir.join(DB_NAME);

    // Use unique temp file per process+thread to prevent concurrent overwrites
    // Format: .ffts-index.db.tmp.{pid}_{thread_id_hash}
    // Using hash of thread ID to get a clean numeric suffix (ThreadId Debug format has parentheses)
    let thread_id_hash = {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        std::thread::current().id().hash(&mut hasher);
        hasher.finish()
    };
    let unique_suffix = format!("{}_{:x}", std::process::id(), thread_id_hash);
    let tmp_path = project_dir.join(format!("{DB_NAME}{DB_TMP_SUFFIX}.{unique_suffix}"));

    // Clean up any stale temp file from previous failed attempt (same process/thread)
    let _ = fs::remove_file(&tmp_path);
    let _ =
        fs::remove_file(project_dir.join(format!("{DB_NAME}{DB_TMP_SUFFIX}.{unique_suffix}-shm")));
    let _ =
        fs::remove_file(project_dir.join(format!("{DB_NAME}{DB_TMP_SUFFIX}.{unique_suffix}-wal")));

    // Create database in temp location (atomic pattern)
    let db = Database::open(&tmp_path, config)?;
    db.init_schema()?;

    // Index files
    let mut indexer = Indexer::new(project_dir, db, indexer_config);
    let stats = indexer.index_directory()?;

    if !quiet {
        tracing::info!(
            files = stats.files_indexed,
            skipped = stats.files_skipped,
            "Initialized database"
        );
    }

    // WAL checkpoint: flush all data to main file before rename
    // This ensures the renamed file is self-contained (no orphaned WAL files)
    //
    // CRITICAL: If checkpoint fails completely, we must NOT proceed because:
    // - Data may still be in WAL file (tmp.db-wal)
    // - Renaming main file would orphan the WAL (SQLite looks for final.db-wal)
    // - Then cleanup would DELETE the orphaned WAL, causing DATA LOSS
    //
    // Checkpoint result columns:
    // - busy: 0 = no blocking, >0 = checkpoint was blocked
    // - log: frames in WAL before checkpoint
    // - checkpointed: frames moved to database file
    let checkpoint_result: std::result::Result<(i64, i64, i64), _> =
        indexer.db().conn().query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        });

    // Close the database connection before rename to release locks
    drop(indexer);

    // Determine if checkpoint was successful enough to proceed
    let checkpoint_ok = match checkpoint_result {
        Ok((0, log, checkpointed)) if log == checkpointed => {
            // Perfect: not blocked, all frames checkpointed
            tracing::debug!(log, checkpointed, "WAL checkpoint complete");
            true
        }
        Ok((busy, log, checkpointed)) => {
            // Partial or blocked checkpoint
            if log == checkpointed {
                // All data was checkpointed despite being busy
                tracing::debug!(busy, log, checkpointed, "WAL checkpoint complete (was busy)");
                true
            } else {
                // Incomplete: some frames not checkpointed
                tracing::warn!(
                    busy,
                    log,
                    checkpointed,
                    "WAL checkpoint incomplete - {} frames remain",
                    log - checkpointed
                );
                false
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "WAL checkpoint query failed");
            false
        }
    };

    // Helper to clean up temp files on failure/skip
    let cleanup_temp = || {
        let _ = fs::remove_file(&tmp_path);
        let _ = fs::remove_file(
            project_dir.join(format!("{DB_NAME}{DB_TMP_SUFFIX}.{unique_suffix}-shm")),
        );
        let _ = fs::remove_file(
            project_dir.join(format!("{DB_NAME}{DB_TMP_SUFFIX}.{unique_suffix}-wal")),
        );
    };

    if !checkpoint_ok {
        // Checkpoint failed - don't rename (would cause data loss)
        // Keep temp files for potential recovery, but don't fail the function
        // because we might be in a race and another process succeeded
        if db_path.exists() {
            // Another process created a database - use theirs
            cleanup_temp();
            tracing::debug!("Checkpoint failed but database exists, using existing");
        } else {
            // No database exists and we can't safely create one
            // This is a real failure - clean up and return error
            cleanup_temp();
            return Err(crate::error::IndexerError::Database {
                source: rusqlite::Error::SqliteFailure(
                    rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_BUSY),
                    Some("WAL checkpoint failed, cannot safely create database".to_string()),
                ),
            });
        }
    } else {
        // Checkpoint succeeded - safe to rename
        // Race condition handling:
        // - On Unix: rename() atomically REPLACES target if exists (POSIX semantics)
        // - On Windows: rename() fails if target exists
        //
        // We check if target exists first to avoid unnecessary overwrites.
        if db_path.exists() {
            // Another process already created the database - use theirs
            cleanup_temp();
            tracing::debug!("Database already exists, using existing");
        } else {
            if let Err(err) = sync_file(&tmp_path) {
                cleanup_temp();
                return Err(crate::error::IndexerError::Io { source: err });
            }
            if let Err(e) = fs::rename(&tmp_path, &db_path) {
                // Rename failed (Windows: target appeared between check and rename)
                cleanup_temp();
                tracing::debug!(error = %e, "Rename failed, using existing database");
            } else {
                // Rename succeeded - only clean up WAL files (main file was renamed)
                let _ = fs::remove_file(
                    project_dir.join(format!("{DB_NAME}{DB_TMP_SUFFIX}.{unique_suffix}-shm")),
                );
                let _ = fs::remove_file(
                    project_dir.join(format!("{DB_NAME}{DB_TMP_SUFFIX}.{unique_suffix}-wal")),
                );
                sync_parent_dir(&db_path)
                    .map_err(|e| crate::error::IndexerError::Io { source: e })?;
            }
        }
    }

    Ok(stats)
}

/// Backup corrupted database and reinitialize.
///
/// Creates a timestamped backup of the corrupted database before removal,
/// then performs fresh initialization.
///
/// # Backup Format
///
/// Backup filename: `.ffts-index.db.backup.{unix_timestamp}`
///
/// # Arguments
///
/// * `project_dir` - Directory containing corrupted database
/// * `config` - SQLite PRAGMA configuration
/// * `quiet` - Suppress progress logging
///
/// # Errors
///
/// Returns `IndexerError` if reinitialization fails.
/// Backup failure is logged but not fatal (we still try to reinit).
pub fn backup_and_reinit(
    project_dir: &Path,
    config: &PragmaConfig,
    quiet: bool,
) -> Result<IndexStats> {
    backup_and_reinit_with_config(project_dir, config, IndexerConfig::default(), quiet)
}

/// Backup corrupted database and reinitialize with explicit indexer configuration.
pub fn backup_and_reinit_with_config(
    project_dir: &Path,
    config: &PragmaConfig,
    indexer_config: IndexerConfig,
    quiet: bool,
) -> Result<IndexStats> {
    let db_path = project_dir.join(DB_NAME);

    // Create timestamped backup filename
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let backup_path = project_dir.join(format!("{DB_NAME}.backup.{timestamp}"));

    // Attempt backup via rename (atomic move)
    if let Err(e) = fs::rename(&db_path, &backup_path) {
        tracing::warn!(error = %e, "Backup failed, removing corrupted database");
        let _ = fs::remove_file(&db_path);
    } else {
        if !quiet {
            tracing::info!(path = %backup_path.display(), "Created backup of corrupted database");
        }
        if let Err(e) = sync_parent_dir(&backup_path) {
            tracing::warn!(error = %e, "Failed to sync backup directory");
        }
    }

    // Clean up WAL/SHM files from corrupted database
    let _ = fs::remove_file(project_dir.join(format!("{DB_NAME}-shm")));
    let _ = fs::remove_file(project_dir.join(format!("{DB_NAME}-wal")));

    // Perform fresh initialization
    auto_init_with_config(project_dir, config, indexer_config, quiet)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::APPLICATION_ID_I32;
    use tempfile::tempdir;

    // === find_project_root tests ===

    #[test]
    fn test_find_root_with_existing_database() {
        let root = tempdir().unwrap();
        let subdir = root.path().join("src/lib");
        fs::create_dir_all(&subdir).unwrap();

        // Create database in root
        let db = Database::open(&root.path().join(DB_NAME), &PragmaConfig::default()).unwrap();
        db.init_schema().unwrap();
        drop(db);

        let found = find_project_root(&subdir);
        assert_eq!(found.path, root.path());
        assert_eq!(found.method, DetectionMethod::ExistingDatabase);
    }

    #[test]
    fn test_find_root_with_git_directory() {
        let root = tempdir().unwrap();
        let subdir = root.path().join("src/lib");
        fs::create_dir_all(&subdir).unwrap();
        fs::create_dir(root.path().join(".git")).unwrap();

        let found = find_project_root(&subdir);
        assert_eq!(found.path, root.path());
        assert_eq!(found.method, DetectionMethod::GitRepository);
    }

    #[test]
    fn test_find_root_database_priority_over_git() {
        let root = tempdir().unwrap();
        let subdir = root.path().join("nested");
        fs::create_dir_all(&subdir).unwrap();

        // Create both .git and database in root
        fs::create_dir(root.path().join(".git")).unwrap();
        let db = Database::open(&root.path().join(DB_NAME), &PragmaConfig::default()).unwrap();
        db.init_schema().unwrap();
        drop(db);

        let found = find_project_root(&subdir);
        // Database should take priority
        assert_eq!(found.method, DetectionMethod::ExistingDatabase);
    }

    #[test]
    fn test_find_root_fallback_to_start() {
        let dir = tempdir().unwrap();
        // No .git, no database

        let found = find_project_root(dir.path());
        assert_eq!(found.path, dir.path());
        assert_eq!(found.method, DetectionMethod::Fallback);
    }

    #[test]
    fn test_find_root_nested_git_finds_nearest() {
        let outer = tempdir().unwrap();
        let inner = outer.path().join("inner");
        let deep = inner.join("deep");
        fs::create_dir_all(&deep).unwrap();

        // Create .git in both outer and inner
        fs::create_dir(outer.path().join(".git")).unwrap();
        fs::create_dir(inner.join(".git")).unwrap();

        let found = find_project_root(&deep);
        // Should find inner (nearest ancestor with .git)
        assert_eq!(found.path, inner);
        assert_eq!(found.method, DetectionMethod::GitRepository);
    }

    #[test]
    fn test_find_root_ignores_corrupt_database() {
        let root = tempdir().unwrap();
        let subdir = root.path().join("project");
        fs::create_dir_all(&subdir).unwrap();

        // Create corrupt 0-byte database in root (the actual bug scenario)
        fs::write(root.path().join(DB_NAME), b"").unwrap();

        // Create .git in project subdirectory
        fs::create_dir(subdir.join(".git")).unwrap();

        // Should find .git, NOT the corrupt database
        let found = find_project_root(&subdir);
        assert_eq!(found.method, DetectionMethod::GitRepository);
        assert_eq!(found.path, subdir);
    }

    #[test]
    fn test_find_root_ignores_foreign_database() {
        let root = tempdir().unwrap();
        let subdir = root.path().join("project");
        fs::create_dir_all(&subdir).unwrap();

        // Create SQLite database with different application_id
        let conn = rusqlite::Connection::open(root.path().join(DB_NAME)).unwrap();
        conn.pragma_update(None, "application_id", 0x1234_5678_i32).unwrap();
        drop(conn);

        // Create .git in project subdirectory
        fs::create_dir(subdir.join(".git")).unwrap();

        // Should find .git, NOT the foreign database
        let found = find_project_root(&subdir);
        assert_eq!(found.method, DetectionMethod::GitRepository);
        assert_eq!(found.path, subdir);
    }

    // === DatabaseHealth tests ===

    #[test]
    fn test_health_missing_database() {
        let dir = tempdir().unwrap();
        assert_eq!(check_health_fast(dir.path()), DatabaseHealth::Missing);
        assert!(DatabaseHealth::Missing.needs_init());
    }

    #[test]
    fn test_health_empty_database() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join(DB_NAME);

        let db = Database::open(&db_path, &PragmaConfig::default()).unwrap();
        db.init_schema().unwrap();
        drop(db);

        assert_eq!(check_health_fast(dir.path()), DatabaseHealth::Empty);
        assert!(DatabaseHealth::Empty.needs_init());
    }

    #[test]
    fn test_health_healthy_database() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join(DB_NAME);

        let db = Database::open(&db_path, &PragmaConfig::default()).unwrap();
        db.init_schema().unwrap();
        db.upsert_file("test.rs", "fn main() {}", 0, 12).unwrap();
        drop(db);

        let health = check_health_fast(dir.path());
        assert_eq!(health, DatabaseHealth::Healthy);
        assert!(health.is_usable());
        assert!(!health.needs_init());
    }

    #[test]
    fn test_health_wrong_application_id() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join(DB_NAME);

        // Create database with wrong application ID
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.pragma_update(None, "application_id", 0x1234_5678_i32).unwrap();
        // Create minimal schema so it doesn't fail on schema check
        conn.execute("CREATE TABLE files (path TEXT PRIMARY KEY, content TEXT)", []).unwrap();
        drop(conn);

        let health = check_health_fast(dir.path());
        assert_eq!(health, DatabaseHealth::WrongApplicationId);
        assert!(health.is_unrecoverable());
    }

    #[test]
    fn test_health_corrupted_file() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join(DB_NAME);

        // Write garbage to database file
        // rusqlite can open this file, but queries will fail
        // Result: get_application_id() returns None → Corrupted
        fs::write(&db_path, b"not a sqlite database").unwrap();

        let health = check_health_fast(dir.path());
        // Garbage file is either Unreadable (if rusqlite rejects it) or Corrupted (if queries fail)
        assert!(
            matches!(health, DatabaseHealth::Unreadable | DatabaseHealth::Corrupted),
            "Expected Unreadable or Corrupted, got {health:?}"
        );
    }

    #[test]
    fn test_health_incomplete_schema() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join(DB_NAME);

        // Create database with correct app ID but missing schema
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.pragma_update(None, "application_id", APPLICATION_ID_I32).unwrap();
        // Create only files table, missing FTS, triggers, indexes
        conn.execute("CREATE TABLE files (path TEXT PRIMARY KEY, content TEXT)", []).unwrap();
        drop(conn);

        let health = check_health_fast(dir.path());
        assert_eq!(health, DatabaseHealth::SchemaInvalid);
        assert!(health.needs_reinit());
    }

    // === DatabaseHealth enum tests ===

    #[test]
    fn test_health_enum_methods() {
        assert!(DatabaseHealth::Healthy.is_usable());
        assert!(!DatabaseHealth::Missing.is_usable());
        assert!(!DatabaseHealth::Corrupted.is_usable());

        assert!(DatabaseHealth::Missing.needs_init());
        assert!(DatabaseHealth::Empty.needs_init());
        assert!(!DatabaseHealth::Healthy.needs_init());

        assert!(DatabaseHealth::SchemaInvalid.needs_reinit());
        assert!(DatabaseHealth::Corrupted.needs_reinit());
        assert!(!DatabaseHealth::Missing.needs_reinit());

        assert!(DatabaseHealth::WrongApplicationId.is_unrecoverable());
        assert!(DatabaseHealth::Unreadable.is_unrecoverable());
        assert!(!DatabaseHealth::Corrupted.is_unrecoverable());
    }

    // === auto_init tests ===

    #[test]
    fn test_auto_init_creates_database() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("test.rs"), "fn main() {}").unwrap();

        let stats = auto_init(dir.path(), &PragmaConfig::default(), true).unwrap();
        assert!(stats.files_indexed > 0);
        assert!(dir.path().join(DB_NAME).exists());

        // Verify database is healthy
        assert_eq!(check_health_fast(dir.path()), DatabaseHealth::Healthy);
    }

    #[test]
    fn test_auto_init_race_condition_safe() {
        use std::sync::Arc;
        use std::thread;

        let dir = tempdir().unwrap();
        fs::write(dir.path().join("test.rs"), "content").unwrap();

        let dir_path = Arc::new(dir.path().to_path_buf());
        let config = Arc::new(PragmaConfig::default());

        // Spawn multiple threads to init simultaneously
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let dir = Arc::clone(&dir_path);
                let cfg = Arc::clone(&config);
                thread::spawn(move || auto_init(&dir, &cfg, true))
            })
            .collect();

        // Collect results - at least one must succeed
        let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        let successes = results.iter().filter(|r| r.is_ok()).count();

        // At least one thread must succeed (race winner)
        // Others may succeed or fail depending on timing
        assert!(
            successes >= 1,
            "Expected at least 1 success, got {successes} out of {} threads",
            results.len()
        );

        // CRITICAL: Database should exist and be healthy regardless of individual thread outcomes
        assert_eq!(check_health_fast(&dir_path), DatabaseHealth::Healthy);
    }

    #[test]
    fn test_auto_init_updates_gitignore() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("test.rs"), "content").unwrap();
        fs::write(dir.path().join(".gitignore"), "node_modules/\n").unwrap();

        let _ = auto_init(dir.path(), &PragmaConfig::default(), true).unwrap();

        let content = fs::read_to_string(dir.path().join(".gitignore")).unwrap();
        assert!(content.contains("node_modules/"));
        assert!(content.contains(DB_NAME));
    }

    #[test]
    fn test_auto_init_empty_directory() {
        // Test auto_init on directory with no indexable files
        let dir = tempdir().unwrap();
        // Only create hidden/ignored files
        fs::write(dir.path().join(".gitignore"), "*.log\n").unwrap();

        let stats = auto_init(dir.path(), &PragmaConfig::default(), true).unwrap();

        // Should succeed with 0 files indexed (only .gitignore)
        // The gitignore itself gets indexed since it's a text file
        assert!(dir.path().join(DB_NAME).exists());
        // Health should be Healthy (has at least .gitignore) or Empty (if gitignore excluded)
        let health = check_health_fast(dir.path());
        assert!(
            health == DatabaseHealth::Healthy || health == DatabaseHealth::Empty,
            "Expected Healthy or Empty, got {health:?} with {} files indexed",
            stats.files_indexed
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_auto_init_with_config_follows_symlinks() {
        use std::os::unix::fs::symlink;

        let dir = tempdir().unwrap();
        let real_dir = dir.path().join("real");
        fs::create_dir_all(&real_dir).unwrap();
        fs::write(real_dir.join("inner.rs"), "fn inner() {}").unwrap();
        let link_dir = dir.path().join("linkdir");
        symlink(&real_dir, &link_dir).unwrap();

        let indexer_config = IndexerConfig { follow_symlinks: true, ..Default::default() };
        let stats =
            auto_init_with_config(dir.path(), &PragmaConfig::default(), indexer_config, true)
                .unwrap();

        assert!(stats.files_indexed >= 1);
        assert!(dir.path().join(DB_NAME).exists());

        let db = Database::open(&dir.path().join(DB_NAME), &PragmaConfig::default()).unwrap();
        let files = db.get_all_files(10).unwrap();
        assert!(files.contains(&"linkdir/inner.rs".to_string()));
    }

    #[test]
    fn test_auto_init_skips_if_database_exists() {
        // Test that auto_init doesn't overwrite existing database
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("test.rs"), "original content").unwrap();

        // First init
        let _ = auto_init(dir.path(), &PragmaConfig::default(), true).unwrap();

        // Add new file
        fs::write(dir.path().join("new.rs"), "new content").unwrap();

        // Second init should skip because database exists
        let stats2 = auto_init(dir.path(), &PragmaConfig::default(), true).unwrap();

        // The function creates a temp database and indexes, but then
        // sees the existing database and skips the rename
        // So stats2 will have the new file count, but the actual
        // database on disk is unchanged (still has only test.rs)

        // Verify original database is unchanged by checking file count
        let db = Database::open(&dir.path().join(DB_NAME), &PragmaConfig::default()).unwrap();
        let count = db.get_file_count().unwrap();
        // Original database had 1 file (test.rs), new temp had 2
        // After skip, should still have 1
        assert_eq!(count, 1, "Database should not be overwritten when exists");
        // Stats from temp database show what WOULD have been indexed
        assert!(stats2.files_indexed >= 1);
    }

    // === backup_and_reinit tests ===

    #[test]
    fn test_backup_creates_timestamped_file() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join(DB_NAME), b"corrupted").unwrap();
        fs::write(dir.path().join("test.rs"), "content").unwrap();

        let indexer_config = IndexerConfig::default();
        let _ = backup_and_reinit_with_config(
            dir.path(),
            &PragmaConfig::default(),
            indexer_config,
            true,
        )
        .unwrap();

        // Backup should exist
        let backups: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains("backup"))
            .collect();
        assert!(!backups.is_empty());

        // Database should be healthy
        assert_eq!(check_health_fast(dir.path()), DatabaseHealth::Healthy);
    }

    #[test]
    fn test_backup_cleans_wal_files() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join(DB_NAME), b"corrupted").unwrap();
        fs::write(dir.path().join(format!("{DB_NAME}-shm")), b"shm").unwrap();
        fs::write(dir.path().join(format!("{DB_NAME}-wal")), b"wal").unwrap();
        fs::write(dir.path().join("test.rs"), "content").unwrap();

        let _ = backup_and_reinit(dir.path(), &PragmaConfig::default(), true).unwrap();

        // Old WAL files should be cleaned up
        assert!(!dir.path().join(format!("{DB_NAME}-shm")).exists());
        assert!(!dir.path().join(format!("{DB_NAME}-wal")).exists());
    }

    // === Performance tests ===

    #[test]
    fn test_health_check_fast_performance() {
        let dir = tempdir().unwrap();
        let db = Database::open(&dir.path().join(DB_NAME), &PragmaConfig::default()).unwrap();
        db.init_schema().unwrap();
        db.upsert_file("test.rs", "content", 0, 7).unwrap();
        drop(db);

        let start = std::time::Instant::now();
        for _ in 0..100 {
            let _ = check_health_fast(dir.path());
        }
        let elapsed = start.elapsed();

        // 100 checks should complete in <500ms (conservative for CI variability)
        // Production target: <100μs each = <10ms for 100 iterations
        assert!(elapsed.as_millis() < 500, "Health check too slow: {elapsed:?} for 100 iterations");
    }

    #[test]
    fn test_find_project_root_performance() {
        let dir = tempdir().unwrap();
        let deep = dir.path().join("a/b/c/d/e/f/g/h/i/j");
        fs::create_dir_all(&deep).unwrap();
        fs::create_dir(dir.path().join(".git")).unwrap();

        let start = std::time::Instant::now();
        for _ in 0..1000 {
            let _ = find_project_root(&deep);
        }
        let elapsed = start.elapsed();

        // 1000 lookups should complete in <500ms (conservative for CI variability)
        // Production target: <100μs each = <100ms for 1000 iterations
        assert!(
            elapsed.as_millis() < 500,
            "Project root detection too slow: {elapsed:?} for 1000 iterations"
        );
    }
}
