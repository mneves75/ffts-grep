//! ffts-indexer - Fast full-text search file indexer using `SQLite` FTS5
//!
//! This library provides the core functionality for indexing files
//! in a directory and searching them using `SQLite` FTS5.
//!
//! # Example
//!
//! ```rust
//! use ffts_indexer::{Database, Indexer, IndexerConfig, PragmaConfig, DB_NAME};
//! use std::path::Path;
//! use std::time::{SystemTime, UNIX_EPOCH};
//!
//! let unique = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
//! let root = std::env::temp_dir().join(format!("ffts-indexer-doctest-{unique}"));
//! std::fs::create_dir_all(&root)?;
//! std::fs::write(root.join("main.rs"), "fn main() {}")?;
//!
//! let db_path = root.join(DB_NAME);
//! let db = Database::open(&db_path, &PragmaConfig::default())?;
//! db.init_schema()?;
//!
//! let config = IndexerConfig::default();
//! let mut indexer = Indexer::new(Path::new(&root), db, config);
//! indexer.index_directory()?;
//!
//! drop(indexer);
//! let _ = std::fs::remove_dir_all(&root);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

/// Default database filename.
pub const DB_NAME: &str = ".ffts-index.db";

/// WAL mode shm file suffix.
pub const DB_SHM_SUFFIX: &str = "-shm";

/// WAL mode wal file suffix.
pub const DB_WAL_SUFFIX: &str = "-wal";

/// Temporary file suffix during reindex.
pub const DB_TMP_SUFFIX: &str = ".tmp";

/// WAL mode shm file name.
pub const DB_SHM_NAME: &str = ".ffts-index.db-shm";

/// WAL mode wal file name.
pub const DB_WAL_NAME: &str = ".ffts-index.db-wal";

/// Temporary file name during reindex.
pub const DB_TMP_NAME: &str = ".ffts-index.db.tmp";

/// Temporary file glob for gitignore entries (covers suffix variants).
pub const DB_TMP_GLOB: &str = ".ffts-index.db.tmp*";

pub mod cli;
pub mod db;
pub mod doctor;
pub mod error;
pub mod health;
pub mod indexer;
pub mod init;
pub mod search;

pub use cli::OutputFormat;
pub use db::{Database, PragmaConfig, SchemaCheck, SearchResult};
pub use doctor::{
    CheckResult, Doctor, DoctorOutput, DoctorSummary, EXPECTED_APPLICATION_ID, Severity,
};
pub use error::{ExitCode, IndexerError, Result};
pub use health::{
    DatabaseHealth, DetectionMethod, ProjectRoot, auto_init, auto_init_with_config,
    backup_and_reinit, backup_and_reinit_with_config, check_health_fast, find_project_root,
};
pub use indexer::{IndexStats, Indexer, IndexerConfig};
pub use init::{GitignoreResult, InitResult, check_gitignore, gitignore_entries, update_gitignore};
pub use search::{SearchConfig, Searcher};
