use clap::{Parser, Subcommand, ValueEnum};
use std::path::{Path, PathBuf};

use crate::{
    DB_NAME,
    error::{IndexerError, Result},
    health::find_project_root,
};

#[cfg(target_os = "macos")]
const DEFAULT_MMAP_SIZE: i64 = 0;

#[cfg(not(target_os = "macos"))]
const DEFAULT_MMAP_SIZE: i64 = 256 * 1024 * 1024;

/// Output format for search results.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
pub enum OutputFormat {
    #[default]
    Plain,
    Json,
}

/// CLI arguments for the FTS5 indexer.
#[derive(Parser, Debug)]
#[command(
    name = "ffts-grep",
    version = env!("CARGO_PKG_VERSION"),
    about = "Fast full-text search file indexer using SQLite FTS5",
    long_about = concat!("Fast full-text search file indexer using SQLite FTS5

A high-performance file indexer that provides sub-10ms queries after initial indexing.
Uses SQLite FTS5 with BM25 ranking for relevant search results.

Version: ", env!("CARGO_PKG_VERSION"), "
Repository: https://github.com/mneves75/ffts-grep

SUBCOMMANDS:
  search     Search indexed files (default when query provided)
  index      Index or reindex the project directory
  doctor     Run diagnostic checks on installation health
  init       Initialize project with .gitignore and database

EXIT CODES:
  0   Success
  1   Warnings (non-fatal issues found)
  2   Errors (diagnostic failures)

For more information, see: https://github.com/mneves75/ffts-grep"),
    disable_help_flag = false,
    disable_version_flag = false
)]
pub struct Cli {
    /// Search query (triggers search mode if provided)
    #[arg(index = 1)]
    pub query: Vec<String>,

    #[command(subcommand)]
    pub command: Option<Commands>,

    /// Suppress status messages (for CI/scripting)
    #[arg(long, short = 'q')]
    pub quiet: bool,

    /// Project directory (defaults to current directory)
    #[arg(long, env = "CLAUDE_PROJECT_DIR")]
    pub project_dir: Option<PathBuf>,

    /// Follow symlinks while indexing (disabled by default for safety)
    #[arg(long)]
    pub follow_symlinks: bool,

    /// Refresh index before searching (requires a non-empty query)
    #[arg(long, global = true)]
    pub refresh: bool,

    /// `SQLite` cache size in `KB` (negative) or `pages` (positive)
    #[arg(long, default_value = "-32000", value_parser = validate_cache_size)]
    pub pragma_cache_size: i64,

    /// Memory-mapped I/O size in bytes (0 = disabled on macOS)
    #[arg(long, default_value_t = DEFAULT_MMAP_SIZE, value_parser = validate_mmap_size)]
    pub pragma_mmap_size: i64,

    /// Database page size in bytes (must be power of 2, 512-65536)
    #[arg(long, default_value = "4096", value_parser = validate_page_size)]
    pub pragma_page_size: i64,

    /// Busy timeout in milliseconds (0 = disabled)
    #[arg(long, default_value = "5000", value_parser = validate_busy_timeout)]
    pub pragma_busy_timeout: i64,

    /// `SQLite` synchronous mode (`OFF`, `NORMAL`, `FULL`, `EXTRA`)
    #[arg(long, default_value = "NORMAL", value_parser = validate_synchronous)]
    pub pragma_synchronous: String,
}

/// Subcommands for ffts-grep.
#[derive(Subcommand, Debug, Clone)]
pub enum Commands {
    /// Index or reindex files in the project directory.
    Index {
        /// Force full reindex (atomic replace)
        #[arg(long)]
        reindex: bool,
    },
    /// Run diagnostic checks on installation health.
    Doctor {
        /// Verbose output for diagnostics
        #[arg(long, short = 'v')]
        verbose: bool,
        /// JSON output format
        #[arg(long)]
        json: bool,
    },
    /// Initialize project with .gitignore and database.
    Init {
        /// Only update .gitignore, skip database creation
        #[arg(long)]
        gitignore_only: bool,
        /// Force reinitialization even if already initialized
        #[arg(long)]
        force: bool,
    },
    /// Search indexed files (this is the default when a query is provided).
    Search {
        /// Search query
        #[arg(index = 1)]
        query: Vec<String>,
        /// Search paths only (no content)
        #[arg(long)]
        paths: bool,
        /// Output format
        #[arg(long, value_enum)]
        format: Option<OutputFormat>,
        /// Run performance benchmark
        #[arg(long)]
        benchmark: bool,
        /// Disable auto-initialization on search (fail if no database)
        #[arg(long)]
        no_auto_init: bool,
    },
}

/// Validates `cache_size`: must be positive (`pages`) or `-1000` to `-1000000` (`KB`).
fn validate_cache_size(s: &str) -> std::result::Result<i64, String> {
    let val: i64 = s.parse().map_err(|_| "invalid integer".to_string())?;

    match val {
        v if v > 0 => Ok(v),                              // Pages
        v if (-1_000_000..=-1_000).contains(&v) => Ok(v), // KB range
        _ => Err("must be positive (pages) or -1000 to -1000000 (KB)".to_string()),
    }
}

/// Validates `mmap_size`: must be between `0` and `256MB`.
fn validate_mmap_size(s: &str) -> std::result::Result<i64, String> {
    const MAX_MMAP: i64 = 256 * 1024 * 1024; // 256MB max on Linux

    let val: i64 = s.parse().map_err(|_| "invalid integer".to_string())?;

    if val < 0 {
        return Err("must be >= 0".to_string());
    }

    if val > MAX_MMAP {
        return Err(format!("must be <= {MAX_MMAP} (256MB)"));
    }

    Ok(val)
}

/// Validates `page_size`: must be power of 2 between `512` and `65536`.
fn validate_page_size(s: &str) -> std::result::Result<i64, String> {
    let val: i64 = s.parse().map_err(|_| "invalid integer".to_string())?;

    if !(512..=65536).contains(&val) {
        return Err("must be between 512 and 65536".to_string());
    }

    if (val & (val - 1)) != 0 {
        return Err("must be a power of 2".to_string());
    }

    Ok(val)
}

/// Validates `busy_timeout`: must be non-negative.
fn validate_busy_timeout(s: &str) -> std::result::Result<i64, String> {
    let val: i64 = s.parse().map_err(|_| "invalid integer".to_string())?;

    if val < 0 {
        return Err("must be >= 0".to_string());
    }

    Ok(val)
}

/// Validates synchronous mode: must be OFF, NORMAL, FULL, or EXTRA.
fn validate_synchronous(s: &str) -> std::result::Result<String, String> {
    match s.to_uppercase().as_str() {
        "OFF" | "NORMAL" | "FULL" | "EXTRA" => Ok(s.to_uppercase()),
        _ => Err("must be OFF, NORMAL, FULL, or EXTRA".to_string()),
    }
}

impl Cli {
    /// Get the resolved project directory.
    ///
    /// When no explicit path is provided, uses single-pass project root detection:
    /// 1. Existing `.ffts-index.db` (highest priority)
    /// 2. `.git` repository root
    /// 3. Current working directory (fallback)
    ///
    /// # Errors
    /// Returns `IndexerError::ConfigInvalid` if:
    /// - The `~` home directory expansion fails (home directory cannot be determined)
    /// - The current directory cannot be accessed when no explicit path is provided
    pub fn project_dir(&self) -> Result<PathBuf> {
        match &self.project_dir {
            Some(path) => self.expand_tilde(path),
            None => {
                let cwd = std::env::current_dir().map_err(|e| IndexerError::ConfigInvalid {
                    field: "project_dir".to_string(),
                    value: "current_dir".to_string(),
                    reason: e.to_string(),
                })?;
                Ok(find_project_root(&cwd).path)
            }
        }
    }

    /// Expand tilde (`~`) to home directory in path.
    fn expand_tilde(&self, path: &Path) -> Result<PathBuf> {
        if let Some(stripped) = path.to_str().and_then(|s| s.strip_prefix('~')) {
            let home = dirs::home_dir().ok_or_else(|| IndexerError::ConfigInvalid {
                field: "project_dir".to_string(),
                value: path.to_string_lossy().to_string(),
                reason: "Could not determine home directory".to_string(),
            })?;
            if stripped.is_empty() {
                return Ok(home);
            }
            if stripped.starts_with('/') || stripped.starts_with('\\') {
                return Ok(home.join(&stripped[1..]));
            }
        }
        Ok(path.to_path_buf())
    }

    /// Get the database path for the project.
    ///
    /// # Errors
    /// Returns `IndexerError::ConfigInvalid` if `project_dir()` fails.
    pub fn db_path(&self) -> Result<PathBuf> {
        Ok(self.project_dir()?.join(DB_NAME))
    }

    /// Get the search query as a single string.
    #[must_use]
    pub fn query_string(&self) -> Option<String> {
        if self.query.is_empty() { None } else { Some(self.query.join(" ")) }
    }

    /// Returns true if index subcommand is requested.
    #[must_use]
    pub const fn wants_index(&self) -> bool {
        matches!(self.command, Some(Commands::Index { .. }))
    }

    /// Returns true if reindex flag is set.
    #[must_use]
    pub const fn wants_reindex(&self) -> bool {
        match &self.command {
            Some(Commands::Index { reindex }) => *reindex,
            _ => false,
        }
    }

    /// Returns true if doctor subcommand is requested.
    #[must_use]
    pub const fn wants_doctor(&self) -> bool {
        matches!(self.command, Some(Commands::Doctor { .. }))
    }

    /// Returns true if init subcommand is requested.
    #[must_use]
    pub const fn wants_init(&self) -> bool {
        matches!(self.command, Some(Commands::Init { .. }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::PragmaConfig;
    use serial_test::serial;
    use std::path::PathBuf;

    const BIN_NAME: &str = "ffts-grep";

    #[test]
    fn test_subcommand_index() {
        let cli = Cli::parse_from([BIN_NAME, "index"]);
        assert!(cli.wants_index());
        assert!(!cli.wants_reindex());

        let cli = Cli::parse_from([BIN_NAME, "index", "--reindex"]);
        assert!(cli.wants_index());
        assert!(cli.wants_reindex());
    }

    #[test]
    fn test_subcommand_index_reindex_conflict() {
        // reindex is part of index subcommand, not a conflict
        let cli = Cli::parse_from([BIN_NAME, "index", "--reindex"]);
        assert!(cli.wants_index());
        assert!(cli.wants_reindex());
    }

    #[test]
    fn test_subcommand_doctor() {
        let cli = Cli::parse_from([BIN_NAME, "doctor"]);
        assert!(cli.wants_doctor());
        match &cli.command {
            Some(Commands::Doctor { verbose, json }) => {
                assert!(!*verbose);
                assert!(!*json);
            }
            _ => panic!("Expected Doctor subcommand"),
        }
    }

    #[test]
    fn test_subcommand_doctor_verbose() {
        let cli = Cli::parse_from([BIN_NAME, "doctor", "-v"]);
        assert!(cli.wants_doctor());
        match &cli.command {
            Some(Commands::Doctor { verbose, json }) => {
                assert!(*verbose);
                assert!(!*json);
            }
            _ => panic!("Expected Doctor subcommand"),
        }

        let cli = Cli::parse_from([BIN_NAME, "doctor", "--verbose"]);
        assert!(cli.wants_doctor());
        match &cli.command {
            Some(Commands::Doctor { verbose, .. }) => assert!(*verbose),
            _ => panic!("Expected Doctor subcommand"),
        }
    }

    #[test]
    fn test_subcommand_doctor_json() {
        let cli = Cli::parse_from([BIN_NAME, "doctor", "--json"]);
        assert!(cli.wants_doctor());
        match &cli.command {
            Some(Commands::Doctor { json, .. }) => assert!(*json),
            _ => panic!("Expected Doctor subcommand"),
        }
    }

    #[test]
    fn test_subcommand_init() {
        let cli = Cli::parse_from([BIN_NAME, "init"]);
        assert!(cli.wants_init());
        match &cli.command {
            Some(Commands::Init { gitignore_only, force }) => {
                assert!(!*gitignore_only);
                assert!(!*force);
            }
            _ => panic!("Expected Init subcommand"),
        }
    }

    #[test]
    fn test_subcommand_init_gitignore_only() {
        let cli = Cli::parse_from([BIN_NAME, "init", "--gitignore-only"]);
        assert!(cli.wants_init());
        match &cli.command {
            Some(Commands::Init { gitignore_only, .. }) => assert!(*gitignore_only),
            _ => panic!("Expected Init subcommand"),
        }
    }

    #[test]
    fn test_subcommand_init_force() {
        let cli = Cli::parse_from([BIN_NAME, "init", "--force"]);
        assert!(cli.wants_init());
        match &cli.command {
            Some(Commands::Init { force, .. }) => assert!(*force),
            _ => panic!("Expected Init subcommand"),
        }
    }

    #[test]
    fn test_search_subcommand_benchmark() {
        // Search subcommand is primarily for benchmark mode (no query needed)
        let cli = Cli::parse_from([BIN_NAME, "search", "--benchmark"]);
        assert!(cli.query.is_empty());
        match &cli.command {
            Some(Commands::Search { query, benchmark, paths, format, no_auto_init }) => {
                assert!(query.is_empty());
                assert!(*benchmark);
                assert!(!*paths);
                assert!(format.is_none());
                assert!(!*no_auto_init);
            }
            _ => panic!("Expected Search subcommand"),
        }
    }

    #[test]
    fn test_search_subcommand_with_query() {
        // Search subcommand with explicit query
        let cli = Cli::parse_from([BIN_NAME, "search", "test", "query"]);
        assert!(cli.query.is_empty()); // Top-level query is empty
        match &cli.command {
            Some(Commands::Search { query, benchmark, paths, format, no_auto_init }) => {
                assert_eq!(query, &vec!["test", "query"]);
                assert!(!*benchmark);
                assert!(!*paths);
                assert!(format.is_none());
                assert!(!*no_auto_init);
            }
            _ => panic!("Expected Search subcommand"),
        }
    }

    #[test]
    fn test_search_subcommand_no_auto_init() {
        let cli = Cli::parse_from([BIN_NAME, "search", "--no-auto-init", "test"]);
        match &cli.command {
            Some(Commands::Search { no_auto_init, .. }) => {
                assert!(*no_auto_init);
            }
            _ => panic!("Expected Search subcommand"),
        }
    }

    #[test]
    fn test_implicit_search_with_query() {
        // Main usage: query without subcommand triggers implicit search
        let cli = Cli::parse_from([BIN_NAME, "main"]);
        assert_eq!(cli.query, vec!["main"]);
        assert!(cli.command.is_none());
    }

    #[test]
    fn test_implicit_search_multiword_query() {
        // Multi-word queries work without subcommand
        let cli = Cli::parse_from([BIN_NAME, "main", "function"]);
        assert_eq!(cli.query, vec!["main", "function"]);
        assert!(cli.command.is_none());
    }

    #[test]
    fn test_query_parsing() {
        let cli = Cli::parse_from([BIN_NAME, "main", "function"]);
        assert_eq!(cli.query, vec!["main", "function"]);
    }

    #[test]
    fn test_empty_query() {
        let cli = Cli::parse_from([BIN_NAME]);
        assert!(cli.query.is_empty());
        assert!(cli.command.is_none());
    }

    #[test]
    fn test_query_string_single() {
        let cli = Cli::parse_from([BIN_NAME, "test"]);
        assert_eq!(cli.query_string(), Some("test".to_string()));
    }

    #[test]
    fn test_query_string_multiple() {
        let cli = Cli::parse_from([BIN_NAME, "test", "query", "here"]);
        assert_eq!(cli.query_string(), Some("test query here".to_string()));
    }

    #[test]
    fn test_project_dir_tilde_expansion() {
        let home = dirs::home_dir().expect("home dir should be available for test");

        let cli = Cli::parse_from([BIN_NAME, "--project-dir=~"]);
        assert_eq!(cli.project_dir().unwrap(), home);

        let cli = Cli::parse_from([BIN_NAME, "--project-dir=~/project"]);
        assert_eq!(cli.project_dir().unwrap(), home.join("project"));
    }

    #[test]
    #[serial]
    fn test_project_dir_env_fallback() {
        // SAFETY: This test runs serially to avoid concurrent env var mutation.
        unsafe {
            std::env::set_var("CLAUDE_PROJECT_DIR", "/custom/path");
            let cli = Cli::parse_from([BIN_NAME]);
            assert_eq!(cli.project_dir, Some(PathBuf::from("/custom/path")));
            std::env::remove_var("CLAUDE_PROJECT_DIR");
        }
    }

    #[test]
    fn test_default_mmap_size_matches_platform() {
        let cli = Cli::parse_from([BIN_NAME]);
        assert_eq!(cli.pragma_mmap_size, PragmaConfig::default_mmap_size());
    }

    #[test]
    fn test_wants_index() {
        let cli = Cli::parse_from([BIN_NAME, "index"]);
        assert!(cli.wants_index());

        let cli = Cli::parse_from([BIN_NAME, "index", "--reindex"]);
        assert!(cli.wants_index());

        let cli = Cli::parse_from([BIN_NAME, "doctor"]);
        assert!(!cli.wants_index());
    }

    #[test]
    fn test_wants_reindex() {
        let cli = Cli::parse_from([BIN_NAME, "index"]);
        assert!(!cli.wants_reindex());

        let cli = Cli::parse_from([BIN_NAME, "index", "--reindex"]);
        assert!(cli.wants_reindex());
    }

    #[test]
    fn test_wants_doctor() {
        let cli = Cli::parse_from([BIN_NAME, "doctor"]);
        assert!(cli.wants_doctor());

        let cli = Cli::parse_from([BIN_NAME, "index"]);
        assert!(!cli.wants_doctor());
    }

    #[test]
    fn test_wants_init() {
        let cli = Cli::parse_from([BIN_NAME, "init"]);
        assert!(cli.wants_init());

        let cli = Cli::parse_from([BIN_NAME, "doctor"]);
        assert!(!cli.wants_init());
    }

    #[test]
    fn test_default_values() {
        let cli = Cli::parse_from([BIN_NAME]);
        assert_eq!(cli.pragma_cache_size, -32000); // 32MB default
        assert_eq!(cli.pragma_mmap_size, PragmaConfig::default_mmap_size());
        assert_eq!(cli.pragma_page_size, 4096);
        assert_eq!(cli.pragma_busy_timeout, 5000);
        assert_eq!(cli.pragma_synchronous, "NORMAL");
    }

    #[test]
    fn test_follow_symlinks_flag() {
        let cli = Cli::parse_from([BIN_NAME]);
        assert!(!cli.follow_symlinks);

        let cli = Cli::parse_from([BIN_NAME, "--follow-symlinks"]);
        assert!(cli.follow_symlinks);
    }

    #[test]
    fn test_refresh_flag() {
        let cli = Cli::parse_from([BIN_NAME]);
        assert!(!cli.refresh);

        let cli = Cli::parse_from([BIN_NAME, "--refresh", "query"]);
        assert!(cli.refresh);

        let cli = Cli::parse_from([BIN_NAME, "search", "--refresh", "query"]);
        assert!(cli.refresh);
        match &cli.command {
            Some(Commands::Search { query, .. }) => assert_eq!(query, &vec!["query"]),
            _ => panic!("Expected Search subcommand"),
        }
    }

    #[test]
    fn test_quiet_flag() {
        let cli = Cli::parse_from([BIN_NAME, "--quiet"]);
        assert!(cli.quiet);

        let cli = Cli::parse_from([BIN_NAME, "-q"]);
        assert!(cli.quiet);

        let cli = Cli::parse_from([BIN_NAME]);
        assert!(!cli.quiet);
    }

    #[test]
    fn test_pragma_cache_size_negative() {
        let cli = Cli::parse_from([BIN_NAME, "--pragma-cache-size=-8000"]);
        assert_eq!(cli.pragma_cache_size, -8000);
    }

    #[test]
    fn test_pragma_cache_size_positive() {
        let cli = Cli::parse_from([BIN_NAME, "--pragma-cache-size=2000"]);
        assert_eq!(cli.pragma_cache_size, 2000);
    }

    #[test]
    fn test_pragma_cache_size_invalid() {
        // Use try_parse_from to get Result instead of exiting process
        let result = Cli::try_parse_from([BIN_NAME, "--pragma-cache-size=-100"]);
        assert!(result.is_err());
    }

    #[test]
    fn test_pragma_mmap_size_valid() {
        let cli = Cli::parse_from([BIN_NAME, "--pragma-mmap-size=268435456"]);
        assert_eq!(cli.pragma_mmap_size, 268_435_456);
    }

    #[test]
    fn test_pragma_mmap_size_overflow() {
        let result = Cli::try_parse_from([BIN_NAME, "--pragma-mmap-size=300000000"]);
        assert!(result.is_err());
    }

    #[test]
    fn test_pragma_page_size_power_of_two() {
        let cli = Cli::parse_from([BIN_NAME, "--pragma-page-size=4096"]);
        assert_eq!(cli.pragma_page_size, 4096);
    }

    #[test]
    fn test_pragma_page_size_not_power() {
        let result = Cli::try_parse_from([BIN_NAME, "--pragma-page-size=5000"]);
        assert!(result.is_err());
    }

    #[test]
    fn test_pragma_synchronous_valid_values() {
        let cli = Cli::parse_from([BIN_NAME, "--pragma-synchronous=OFF"]);
        assert_eq!(cli.pragma_synchronous, "OFF");

        let cli = Cli::parse_from([BIN_NAME, "--pragma-synchronous=normal"]);
        assert_eq!(cli.pragma_synchronous, "NORMAL");

        let cli = Cli::parse_from([BIN_NAME, "--pragma-synchronous=FULL"]);
        assert_eq!(cli.pragma_synchronous, "FULL");

        let cli = Cli::parse_from([BIN_NAME, "--pragma-synchronous=extra"]);
        assert_eq!(cli.pragma_synchronous, "EXTRA");
    }

    #[test]
    fn test_pragma_synchronous_invalid() {
        let result = Cli::try_parse_from([BIN_NAME, "--pragma-synchronous=INVALID"]);
        assert!(result.is_err());
    }

    #[test]
    fn test_subcommand_only() {
        // Subcommand without extra args should work
        let cli = Cli::parse_from([BIN_NAME, "doctor"]);
        assert!(cli.wants_doctor());
    }

    #[test]
    fn test_db_name_constant() {
        assert_eq!(DB_NAME, ".ffts-index.db");
    }
}
