use std::fs;
use std::io::{self, BufRead, IsTerminal};
use std::path::Path;

use clap::Parser;
use serde::Deserialize;

/// JSON input format for Claude Code file suggestion integration.
/// Claude Code sends: {"query": "search term", "refresh": true}
#[derive(Deserialize)]
struct StdinQuery {
    query: String,
    #[serde(default)]
    refresh: bool,
}
use ffts_indexer::{
    DB_NAME, DB_SHM_SUFFIX, DB_WAL_SUFFIX,
    cli::{Cli, Commands, OutputFormat},
    db::{Database, PragmaConfig},
    doctor::Doctor,
    error::{ExitCode, IndexerError},
    health::{self, DatabaseHealth},
    indexer::{IndexStats, Indexer, IndexerConfig, atomic_reindex_with_config},
    init::{self, InitResult},
    search::{SearchConfig, Searcher},
};

fn main() -> std::process::ExitCode {
    // Parse CLI arguments
    let cli = Cli::parse();

    // Initialize structured logging (respects RUST_LOG env var)
    // Default: WARN level (only errors and warnings)
    // Override: RUST_LOG=info or RUST_LOG=debug for verbose output
    // Quiet flag (-q/--quiet) disables all logging output
    if !cli.quiet {
        tracing_subscriber::fmt()
            .with_target(false) // Hide target module (cleaner output)
            .with_level(true) // Show log level
            .with_writer(std::io::stderr) // Write to stderr (preserves stdout for search results)
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
            )
            .init();
    }

    // Resolve project directory
    let project_dir = match cli.project_dir() {
        Ok(path) => path,
        Err(e) => {
            tracing::error!(
                error = %e,
                "Failed to resolve project directory"
            );
            return ExitCode::NoInput.into(); // NOINPUT
        }
    };

    // Verify project directory exists
    if !project_dir.exists() {
        tracing::error!(
            path = %project_dir.display(),
            "Project directory does not exist"
        );
        return ExitCode::IoErr.into(); // IOERR
    }

    if !project_dir.is_dir() {
        tracing::error!(
            path = %project_dir.display(),
            "Project path is not a directory"
        );
        return ExitCode::IoErr.into(); // IOERR
    }

    // Build PRAGMA configuration
    let pragma_config = PragmaConfig {
        journal_mode: "WAL".to_string(),
        synchronous: cli.pragma_synchronous.clone(),
        cache_size: cli.pragma_cache_size,
        temp_store: "MEMORY".to_string(),
        mmap_size: cli.pragma_mmap_size,
        page_size: cli.pragma_page_size,
        busy_timeout_ms: cli.pragma_busy_timeout,
    };
    let indexer_config =
        || IndexerConfig { follow_symlinks: cli.follow_symlinks, ..Default::default() };

    if cli.refresh
        && matches!(
            cli.command,
            Some(Commands::Index { .. } | Commands::Doctor { .. } | Commands::Init { .. })
        )
    {
        tracing::error!("--refresh is only valid for search operations");
        return ExitCode::DataErr.into();
    }

    // Handle subcommands
    match &cli.command {
        Some(Commands::Doctor { verbose, json }) => {
            let format = if *json { OutputFormat::Json } else { OutputFormat::Plain };
            return run_doctor(&project_dir, *verbose, format);
        }
        Some(Commands::Init { gitignore_only, force }) => {
            return run_init(
                &project_dir,
                &pragma_config,
                indexer_config(),
                *gitignore_only,
                *force,
                cli.quiet,
            );
        }
        Some(Commands::Index { reindex }) => {
            return run_indexing(
                &project_dir,
                &pragma_config,
                indexer_config(),
                *reindex,
                cli.quiet,
            );
        }
        Some(Commands::Search { query, paths, format, benchmark, no_auto_init }) => {
            // Run benchmark mode if requested
            if *benchmark {
                if cli.refresh {
                    tracing::warn!("--refresh ignored in benchmark mode");
                }
                return run_benchmark(&project_dir, &pragma_config, cli.quiet);
            }

            // Use subcommand query if provided, fall back to top-level query
            let search_query = if query.is_empty() { &cli.query } else { query };
            if cli.refresh && query_is_empty(search_query) {
                tracing::error!("--refresh requires a search query or stdin JSON");
                return ExitCode::DataErr.into();
            }
            let output_format = format.unwrap_or(OutputFormat::Plain);
            return run_search(
                &project_dir,
                &pragma_config,
                indexer_config(),
                search_query,
                *paths,
                output_format,
                cli.refresh,
                *no_auto_init,
                cli.quiet,
            );
        }
        None => {
            // No subcommand - check for search query (implicit search, auto-init enabled)
            if !query_is_empty(&cli.query) {
                return run_search(
                    &project_dir,
                    &pragma_config,
                    indexer_config(),
                    &cli.query,
                    false,
                    OutputFormat::Plain,
                    cli.refresh,
                    false, // auto-init enabled for implicit search
                    cli.quiet,
                );
            }

            // No args - try reading JSON from stdin (Claude Code integration)
            // Format: {"query": "search term"}
            // Only read stdin if it's not a terminal (i.e., data is being piped)
            let stdin = io::stdin();
            if !stdin.is_terminal() {
                if let Some(Ok(line)) = stdin.lock().lines().next() {
                    if let Ok(input) = serde_json::from_str::<StdinQuery>(&line) {
                        let trimmed_query = input.query.trim();
                        if trimmed_query.is_empty() {
                            if cli.refresh || input.refresh {
                                tracing::error!("--refresh requires a search query or stdin JSON");
                                return ExitCode::DataErr.into();
                            }
                        } else {
                            let query_parts: Vec<String> =
                                trimmed_query.split_whitespace().map(String::from).collect();
                            let refresh = cli.refresh || input.refresh;
                            return run_search(
                                &project_dir,
                                &pragma_config,
                                indexer_config(),
                                &query_parts,
                                false,
                                OutputFormat::Plain,
                                refresh,
                                false, // auto-init enabled for stdin search
                                cli.quiet,
                            );
                        }
                    }
                }
            }

            if cli.refresh {
                tracing::error!("--refresh requires a search query or stdin JSON");
                return ExitCode::DataErr.into();
            }
        }
    }

    ExitCode::Ok.into() // OK
}

fn query_is_empty(parts: &[String]) -> bool {
    parts.iter().all(|part| part.trim().is_empty())
}

/// Run indexing operation (incremental or full reindex).
///
/// This function orchestrates the complete indexing workflow including database
/// initialization, reindex logic, error handling, and logging. Keeping it as a
/// single function maintains clarity of the full operation flow.
#[allow(clippy::too_many_lines)]
fn run_indexing(
    project_dir: &Path,
    config: &PragmaConfig,
    indexer_config: IndexerConfig,
    force_reindex: bool,
    _quiet: bool,
) -> std::process::ExitCode {
    let db_path = project_dir.join(DB_NAME);

    if force_reindex {
        // Atomic reindex with temp file
        tracing::info!("Running atomic reindex");

        match atomic_reindex_with_config(project_dir, config, indexer_config) {
            Ok(stats) => {
                tracing::info!(
                    files = stats.files_indexed,
                    bytes = stats.bytes_indexed,
                    duration_secs = %format!("{:.2}", stats.duration.as_secs_f64()),
                    "Indexing complete"
                );
            }
            Err(e) => {
                tracing::error!(error = %e, "Atomic reindex failed");
                return match e {
                    IndexerError::Io { .. } => ExitCode::IoErr.into(),
                    _ => ExitCode::Software.into(),
                };
            }
        }
    } else {
        // Incremental index
        match index_incremental(project_dir, &db_path, config, indexer_config) {
            Ok(stats) => {
                log_index_stats(&stats, "Indexing complete");
            }
            Err(e) => {
                tracing::error!(error = %e, "Indexing failed");
                return map_index_error(e);
            }
        }
    }

    ExitCode::Ok.into() // OK
}

fn index_incremental(
    project_dir: &Path,
    db_path: &Path,
    config: &PragmaConfig,
    indexer_config: IndexerConfig,
) -> std::result::Result<IndexStats, IndexerError> {
    let db = Database::open(db_path, config)?;
    db.init_schema()?;
    let mut indexer = Indexer::new(project_dir, db, indexer_config);
    indexer.index_directory()
}

fn log_index_stats(stats: &IndexStats, message: &str) {
    tracing::info!(
        files = stats.files_indexed,
        bytes = stats.bytes_indexed,
        duration_secs = %format!("{:.2}", stats.duration.as_secs_f64()),
        "{message}"
    );
}

fn map_index_error(error: IndexerError) -> std::process::ExitCode {
    match error {
        IndexerError::Io { .. } => ExitCode::IoErr.into(),
        _ => ExitCode::Software.into(),
    }
}

/// Run benchmark mode.
fn run_benchmark(
    project_dir: &Path,
    config: &PragmaConfig,
    _quiet: bool,
) -> std::process::ExitCode {
    let db_path = project_dir.join(DB_NAME);

    let db = match Database::open(&db_path, config) {
        Ok(db) => db,
        Err(e) => {
            tracing::error!(
                error = %e,
                db_path = %db_path.display(),
                "Failed to open database"
            );
            return ExitCode::IoErr.into(); // IOERR
        }
    };

    if let Err(e) = db.init_schema() {
        tracing::error!(
            error = %e,
            "Failed to initialize schema"
        );
        return ExitCode::Software.into(); // SOFTWARE
    }

    // Check if database has content
    let file_count = match db.get_file_count() {
        Ok(count) => count,
        Err(e) => {
            tracing::error!(
                error = %e,
                "Failed to get file count"
            );
            return ExitCode::Software.into(); // SOFTWARE
        }
    };

    if file_count == 0 {
        tracing::error!("Database is empty - run --index first");
        return ExitCode::DataErr.into(); // DATAERR
    }

    tracing::info!(file_count, "Benchmarking indexed files");

    // Run sample queries
    let queries = ["main", "test", "use ", "fn ", "struct", "impl"];

    for query in &queries {
        let start = std::time::Instant::now();
        let results = db.search(query, false, 20);
        let elapsed = start.elapsed();

        match results {
            Ok(res) => {
                tracing::info!(
                    query,
                    results = res.len(),
                    duration_ms = %format!("{:.2}", elapsed.as_secs_f64() * 1000.0),
                    "Query benchmark result"
                );
            }
            Err(e) => {
                tracing::error!(
                    query,
                    error = %e,
                    "Query failed"
                );
            }
        }
    }

    ExitCode::Ok.into() // OK
}

/// Run search operation with auto-init support.
///
/// This function checks database health before searching and can automatically
/// initialize or repair the database unless `no_auto_init` is set.
#[allow(clippy::too_many_arguments)]
fn run_search(
    project_dir: &Path,
    config: &PragmaConfig,
    indexer_config: IndexerConfig,
    query: &[String],
    paths_only: bool,
    format: OutputFormat,
    refresh: bool,
    no_auto_init: bool,
    quiet: bool,
) -> std::process::ExitCode {
    let db_path = project_dir.join(DB_NAME);
    let query_str = query.join(" ");
    let mut already_indexed = false;

    // Check health and handle auto-init BEFORE opening database
    let health = health::check_health_fast(project_dir);

    match health {
        DatabaseHealth::Healthy => {
            // Proceed with search
        }

        DatabaseHealth::Missing | DatabaseHealth::Empty if !no_auto_init => {
            if !quiet {
                tracing::info!(
                    health = ?health,
                    path = %project_dir.display(),
                    "Auto-initializing database"
                );
            }
            if let Err(e) =
                health::auto_init_with_config(project_dir, config, indexer_config.clone(), quiet)
            {
                tracing::error!(error = %e, "Auto-init failed");
                return ExitCode::Software.into();
            }
            already_indexed = true;
        }

        DatabaseHealth::SchemaInvalid | DatabaseHealth::Corrupted if !no_auto_init => {
            tracing::warn!(health = ?health, "Database corrupted, reinitializing");
            if let Err(e) = health::backup_and_reinit_with_config(
                project_dir,
                config,
                indexer_config.clone(),
                quiet,
            ) {
                tracing::error!(error = %e, "Reinit failed");
                return ExitCode::Software.into();
            }
            already_indexed = true;
        }

        DatabaseHealth::Missing | DatabaseHealth::Empty => {
            // --no-auto-init specified
            tracing::error!("Database not initialized. Run: ffts-grep init");
            return ExitCode::DataErr.into();
        }

        DatabaseHealth::SchemaInvalid | DatabaseHealth::Corrupted => {
            // --no-auto-init specified
            tracing::error!("Database corrupted. Run: ffts-grep init --force");
            return ExitCode::DataErr.into();
        }

        DatabaseHealth::WrongApplicationId => {
            tracing::error!(
                "Database {} belongs to different application. \
                 Remove manually or use different directory.",
                DB_NAME
            );
            return ExitCode::DataErr.into();
        }

        DatabaseHealth::Unreadable => {
            tracing::error!("Cannot read database - check file permissions");
            return ExitCode::NoPerm.into();
        }

        // Future-proofing: DatabaseHealth is #[non_exhaustive]
        _ => {
            tracing::error!("Unknown database health state");
            return ExitCode::Software.into();
        }
    }

    if refresh && !already_indexed {
        if !quiet {
            tracing::info!("Refreshing index before search");
        }
        match index_incremental(project_dir, &db_path, config, indexer_config) {
            Ok(stats) => log_index_stats(&stats, "Index refresh complete"),
            Err(e) => {
                tracing::error!(error = %e, "Index refresh failed");
                return map_index_error(e);
            }
        }
    }

    // Now open the database for search
    let mut db = match Database::open(&db_path, config) {
        Ok(db) => db,
        Err(e) => {
            tracing::error!(
                error = %e,
                db_path = %db_path.display(),
                "Failed to open database"
            );
            return ExitCode::IoErr.into();
        }
    };

    if let Err(e) = db.init_schema() {
        tracing::error!(
            error = %e,
            "Failed to initialize schema"
        );
        return ExitCode::Software.into();
    }

    let search_config = SearchConfig { paths_only, format, max_results: 50 };

    let mut searcher = Searcher::new(&mut db, search_config);

    match searcher.search(&query_str) {
        Ok(results) => {
            if let Err(e) = searcher.format_results(&results, &mut std::io::stdout()) {
                tracing::error!(
                    error = %e,
                    "Failed to output search results"
                );
                return ExitCode::Software.into();
            }
        }
        Err(e) => {
            tracing::error!(
                error = %e,
                query = %query_str,
                "Search query failed"
            );
            return ExitCode::DataErr.into();
        }
    }

    ExitCode::Ok.into()
}

/// Run doctor diagnostic checks.
fn run_doctor(project_dir: &Path, verbose: bool, format: OutputFormat) -> std::process::ExitCode {
    let mut doctor = Doctor::new(project_dir, verbose);
    let summary = doctor.run();

    // Output results
    if let Err(e) = doctor.output(&mut std::io::stdout(), format, &summary) {
        tracing::error!(
            error = %e,
            "Failed to output doctor results"
        );
        return ExitCode::Software.into();
    }

    // Return appropriate exit code
    if summary.has_errors() {
        ExitCode::DataErr.into() // Database corruption or missing
    } else if summary.has_warnings() {
        ExitCode::Software.into() // Non-critical issues
    } else {
        ExitCode::Ok.into()
    }
}

/// Run project initialization.
///
/// Orchestrates gitignore setup, database creation, and initial indexing.
/// Keeping as single function maintains clarity of initialization flow.
#[allow(clippy::too_many_lines)]
fn run_init(
    project_dir: &Path,
    config: &PragmaConfig,
    indexer_config: IndexerConfig,
    gitignore_only: bool,
    force: bool,
    quiet: bool,
) -> std::process::ExitCode {
    // Update gitignore
    let gitignore_result = match init::update_gitignore(project_dir) {
        Ok(result) => result,
        Err(e) => {
            tracing::error!(
                error = %e,
                path = %project_dir.display(),
                "Failed to update .gitignore"
            );
            return ExitCode::IoErr.into();
        }
    };

    // If gitignore-only mode, skip database creation
    if gitignore_only {
        let result =
            InitResult { gitignore: gitignore_result, database_created: false, files_indexed: 0 };
        if let Err(e) = init::output_init_result(&mut std::io::stderr(), &result, quiet) {
            tracing::error!(
                error = %e,
                "Failed to output init results"
            );
            return ExitCode::Software.into();
        }
        return ExitCode::Ok.into();
    }

    // Check if database exists
    let db_path = project_dir.join(DB_NAME);
    let db_exists = db_path.exists();

    if db_exists && !force {
        // Database already exists, just report file count
        let db = match Database::open(&db_path, config) {
            Ok(db) => db,
            Err(e) => {
                tracing::error!(
                    error = %e,
                    db_path = %db_path.display(),
                    "Failed to open existing database"
                );
                return ExitCode::IoErr.into();
            }
        };

        let file_count = db.get_file_count().unwrap_or(0);

        let result = InitResult {
            gitignore: gitignore_result,
            database_created: false,
            files_indexed: file_count,
        };

        if let Err(e) = init::output_init_result(&mut std::io::stderr(), &result, quiet) {
            tracing::error!(
                error = %e,
                "Failed to output init results"
            );
            return ExitCode::Software.into();
        }

        return ExitCode::Ok.into();
    }

    // Create and index database
    if force && db_exists {
        // Delete existing database files for force mode (must append suffix)
        let shm_filename = format!("{DB_NAME}{DB_SHM_SUFFIX}");
        let wal_filename = format!("{DB_NAME}{DB_WAL_SUFFIX}");
        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_file(project_dir.join(&shm_filename));
        let _ = fs::remove_file(project_dir.join(&wal_filename));
    }

    let db = match Database::open(&db_path, config) {
        Ok(db) => db,
        Err(e) => {
            tracing::error!(
                error = %e,
                db_path = %db_path.display(),
                "Failed to create database"
            );
            return ExitCode::IoErr.into();
        }
    };

    if let Err(e) = db.init_schema() {
        tracing::error!(
            error = %e,
            "Failed to initialize schema"
        );
        return ExitCode::Software.into();
    }

    // Index files
    let mut indexer = Indexer::new(project_dir, db, indexer_config);

    // Safety: files_indexed will never exceed usize::MAX (limited by available memory)
    #[allow(clippy::cast_possible_truncation)]
    let files_indexed = match indexer.index_directory() {
        Ok(stats) => stats.files_indexed as usize,
        Err(e) => {
            tracing::error!(
                error = %e,
                "Indexing failed during initialization"
            );
            return ExitCode::Software.into();
        }
    };

    let result = InitResult { gitignore: gitignore_result, database_created: true, files_indexed };

    if let Err(e) = init::output_init_result(&mut std::io::stderr(), &result, quiet) {
        tracing::error!(
            error = %e,
            "Failed to output init results"
        );
        return ExitCode::Software.into();
    }

    ExitCode::Ok.into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_reindex_cleans_wal_files() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("one.rs"), "fn one() {}").unwrap();

        let shm_path = dir.path().join(format!("{DB_NAME}{DB_SHM_SUFFIX}"));
        let wal_path = dir.path().join(format!("{DB_NAME}{DB_WAL_SUFFIX}"));
        fs::write(&shm_path, "old shm").unwrap();
        fs::write(&wal_path, "old wal").unwrap();

        let exit = run_indexing(
            dir.path(),
            &PragmaConfig::default(),
            IndexerConfig::default(),
            true,
            true,
        );
        assert_eq!(exit, ExitCode::Ok.into());
        assert!(dir.path().join(DB_NAME).exists());
        assert!(!shm_path.exists());
        assert!(!wal_path.exists());
    }
}
