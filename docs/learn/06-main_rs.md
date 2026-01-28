# Chapter 6: main.rs - The Entry Point

> "The beginning is the most important part of the work." — Plato

## 6.1 What Does This File Do? (In Simple Terms)

The `main.rs` file is the **orchestrator** of the application. Think of it as the conductor of an orchestra—each musician (module) plays their part, but the conductor makes sure everything happens at the right time and in the right order.

### The Restaurant Kitchen Analogy

When you order food at a restaurant:

| Role | Real World | This Application |
|------|------------|------------------|
| Host | Takes your order | `cli.rs` parses args |
| Conductor | Tells each station what to do | `main.rs` dispatches |
| Chef | Cooks the food | `indexer.rs`, `search.rs` |
| Waiter | Brings food to you | Output formatting |
| Manager | Handles problems | Error handling |

The conductor doesn't cook—but without the conductor, chaos ensues!

---

## 6.2 The Main Function: Entry Point

See `main.rs:24-210`:

```rust
fn main() -> std::process::ExitCode {
    let cli = Cli::parse();

    if !cli.quiet {
        tracing_subscriber::fmt()
            .with_target(false)
            .with_level(true)
            .with_writer(std::io::stderr)
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
            )
            .init();
    }

    let project_dir = match cli.project_dir() {
        Ok(path) => path,
        Err(e) => {
            tracing::error!(error = %e, "Failed to resolve project directory");
            return ExitCode::NoInput.into();
        }
    };

    if !project_dir.exists() {
        tracing::error!(path = %project_dir.display(), "Project directory does not exist");
        return ExitCode::IoErr.into();
    }

    if !project_dir.is_dir() {
        tracing::error!(path = %project_dir.display(), "Project path is not a directory");
        return ExitCode::IoErr.into();
    }

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
            if *benchmark {
                if cli.refresh {
                    tracing::warn!("--refresh ignored in benchmark mode");
                }
                return run_benchmark(&project_dir, &pragma_config, cli.quiet);
            }

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
                SearchOptions {
                    config: SearchConfig {
                        paths_only: *paths,
                        format: output_format,
                        max_results: DEFAULT_MAX_RESULTS,
                    },
                    refresh: cli.refresh,
                    no_auto_init: *no_auto_init,
                    quiet: cli.quiet,
                },
            );
        }
        None => {
            if !query_is_empty(&cli.query) {
                return run_search(
                    &project_dir,
                    &pragma_config,
                    indexer_config(),
                    &cli.query,
                    SearchOptions {
                        config: SearchConfig {
                            paths_only: false,
                            format: OutputFormat::Plain,
                            max_results: DEFAULT_MAX_RESULTS,
                        },
                        refresh: cli.refresh,
                        no_auto_init: false,
                        quiet: cli.quiet,
                    },
                );
            }

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
                                SearchOptions {
                                    config: SearchConfig {
                                        paths_only: false,
                                        format: OutputFormat::Plain,
                                        max_results: DEFAULT_MAX_RESULTS,
                                    },
                                    refresh,
                                    no_auto_init: false,
                                    quiet: cli.quiet,
                                },
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

    ExitCode::Ok.into()
}
```

Helper used by main to treat whitespace-only input as empty:

```rust
fn query_is_empty(parts: &[String]) -> bool {
    parts.iter().all(|part| part.trim().is_empty())
}
```

Search options bundle the remaining flags to keep the call sites readable:

```rust
struct SearchOptions {
    config: SearchConfig,
    refresh: bool,
    no_auto_init: bool,
    quiet: bool,
}

const DEFAULT_MAX_RESULTS: u32 = 50;
```

### Key Observations

1. **Parse first** — Get user input before doing anything
2. **Initialize logging** — Capture what's about to happen
3. **Dispatch** — Route to appropriate handler
4. **No command = implicit search or Claude Code mode** — Non-empty queries run search; otherwise stdin JSON is parsed

---

## 6.3 Running Indexing

See `main.rs:200-310`:

```rust
fn run_indexing(
    project_dir: &Path,
    config: &PragmaConfig,
    indexer_config: IndexerConfig,
    force_reindex: bool,
    _quiet: bool,
) -> std::process::ExitCode {
    let db_path = project_dir.join(DB_NAME);

    if force_reindex {
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
        match index_incremental(project_dir, &db_path, config, indexer_config) {
            Ok(stats) => log_index_stats(&stats, "Indexing complete"),
            Err(e) => {
                tracing::error!(error = %e, "Indexing failed");
                return map_index_error(e);
            }
        }
    }

    ExitCode::Ok.into()
}
```


### Two Indexing Modes

| Mode | When to Use | Behavior |
|------|-------------|----------|
| **Incremental** | Normal use | Skip unchanged files |
| **Reindex** | Corruption suspected | Rebuild everything |

---

## 6.4 Running Searches

See `main.rs:350-520`:

```rust
fn run_search(
    project_dir: &Path,
    config: &PragmaConfig,
    indexer_config: IndexerConfig,
    query: &[String],
    options: SearchOptions,
) -> std::process::ExitCode {
    let SearchOptions { config: search_config, refresh, no_auto_init, quiet } = options;
    let db_path = project_dir.join(DB_NAME);
    let query_str = query.join(" ");
    let mut already_indexed = false;

    let health = health::check_health_fast(project_dir);

    match health {
        DatabaseHealth::Healthy => {}

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
            )
            {
                tracing::error!(error = %e, "Reinit failed");
                return ExitCode::Software.into();
            }
            already_indexed = true;
        }

        DatabaseHealth::Missing | DatabaseHealth::Empty => {
            tracing::error!("Database not initialized. Run: ffts-grep init");
            return ExitCode::DataErr.into();
        }

        DatabaseHealth::SchemaInvalid | DatabaseHealth::Corrupted => {
            tracing::error!("Database corrupted. Run: ffts-grep init --force");
            return ExitCode::DataErr.into();
        }

        DatabaseHealth::WrongApplicationId => {
            tracing::error!(
                "Database {} belongs to different application. Remove manually or use different directory.",
                DB_NAME
            );
            return ExitCode::DataErr.into();
        }

        DatabaseHealth::Unreadable => {
            tracing::error!("Cannot read database - check file permissions");
            return ExitCode::NoPerm.into();
        }

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
        tracing::error!(error = %e, "Failed to initialize schema");
        return ExitCode::Software.into();
    }

    let mut searcher = Searcher::new(&mut db, search_config);

    match searcher.search(&query_str) {
        Ok(results) => {
            if let Err(e) = searcher.format_results(&results, &mut std::io::stdout()) {
                tracing::error!(error = %e, "Failed to output search results");
                return ExitCode::Software.into();
            }
        }
        Err(e) => {
            tracing::error!(error = %e, query = %query_str, "Search query failed");
            return ExitCode::DataErr.into();
        }
    }

    ExitCode::Ok.into()
}
```

### Health-Based Auto-Init

The search command handles 7 health states:

| Health State | Action |
|--------------|--------|
| `Healthy` | Search normally |
| `Empty` | Auto-init with indexing |
| `Missing` | Create database, index |
| `SchemaInvalid` | Backup and reinit |
| `Corrupted` | Backup and reinit |
| `WrongApplicationId` | Error |
| `Unreadable` | Error |

When `--no-auto-init` is set, `Missing`, `Empty`, `SchemaInvalid`, and `Corrupted`
return `ExitCode::DataErr` with an actionable message instead of auto-repairing.

---

## 6.5 Running Diagnostics

See `main.rs:422-454`:

```rust
fn run_doctor(project_dir: &Path, verbose: bool, format: OutputFormat) -> std::process::ExitCode {
    let mut doctor = Doctor::new(project_dir, verbose);
    let summary = doctor.run();

    if let Err(e) = doctor.output(&mut std::io::stdout(), format, &summary) {
        tracing::error!(error = %e, "Failed to output doctor results");
        return ExitCode::Software.into();
    }

    if summary.has_errors() {
        ExitCode::DataErr.into()
    } else if summary.has_warnings() {
        ExitCode::Software.into()
    } else {
        ExitCode::Ok.into()
    }
}
```

---

## 6.6 Running Initialization

See `main.rs:455-640`:

```rust
fn run_init(
    project_dir: &Path,
    config: &PragmaConfig,
    indexer_config: IndexerConfig,
    gitignore_only: bool,
    force: bool,
    quiet: bool,
) -> std::process::ExitCode {
    let gitignore_result = match init::update_gitignore(project_dir) {
        Ok(result) => result,
        Err(e) => {
            tracing::error!(error = %e, path = %project_dir.display(), "Failed to update .gitignore");
            return ExitCode::IoErr.into();
        }
    };

    if gitignore_only {
        let result = InitResult {
            gitignore: gitignore_result,
            database_created: false,
            files_indexed: 0,
        };
        let _ = init::output_init_result(&mut std::io::stderr(), &result, quiet);
        return ExitCode::Ok.into();
    }

    let db_path = project_dir.join(DB_NAME);
    let db_exists = db_path.exists();

    if db_exists && !force {
        let db = match Database::open(&db_path, config) {
            Ok(db) => db,
            Err(e) => {
                tracing::error!(error = %e, db_path = %db_path.display(), "Failed to open existing database");
                return ExitCode::IoErr.into();
            }
        };
        let file_count = db.get_file_count().unwrap_or(0);
        let result = InitResult {
            gitignore: gitignore_result,
            database_created: false,
            files_indexed: file_count,
        };
        let _ = init::output_init_result(&mut std::io::stderr(), &result, quiet);
        return ExitCode::Ok.into();
    }

    if force && db_exists {
        let shm_filename = format!("{DB_NAME}{DB_SHM_SUFFIX}");
        let wal_filename = format!("{DB_NAME}{DB_WAL_SUFFIX}");
        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_file(project_dir.join(&shm_filename));
        let _ = fs::remove_file(project_dir.join(&wal_filename));
    }

    let db = match Database::open(&db_path, config) {
        Ok(db) => db,
        Err(e) => {
            tracing::error!(error = %e, db_path = %db_path.display(), "Failed to create database");
            return ExitCode::IoErr.into();
        }
    };

    if let Err(e) = db.init_schema() {
        tracing::error!(error = %e, "Failed to initialize schema");
        return ExitCode::Software.into();
    }

    let mut indexer = Indexer::new(project_dir, db, indexer_config);
    let files_indexed = match indexer.index_directory() {
        Ok(stats) => stats.files_indexed as usize,
        Err(e) => {
            tracing::error!(error = %e, "Indexing failed during initialization");
            return ExitCode::Software.into();
        }
    };

    let result = InitResult { gitignore: gitignore_result, database_created: true, files_indexed };
    let _ = init::output_init_result(&mut std::io::stderr(), &result, quiet);

    ExitCode::Ok.into()
}
```

---

## 6.7 Platform-Specific Code

See `indexer.rs:41-72` and `init.rs:24-58`:

```rust
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
```

### Why Platform-Specific Code?

| Platform | System Call | Reason |
|----------|-------------|--------|
| Windows | `MoveFileExW` | No native atomic rename; need flags |
| Unix | `fs::rename` | POSIX guarantees atomic rename |

The Unix version is simpler because POSIX guarantees that `rename()` is atomic.

Note: `init.rs` uses a sibling `atomic_replace` helper with the same
MoveFileExW fallback to safely replace `.gitignore` on Windows.

---

## 6.8 The Orchestrator Pattern

The main function follows the **Orchestrator Pattern**:

```
┌─────────────────────────────────────┐
│           main.rs                   │
│   ┌─────────────────────────────┐   │
│   │ Parse args → Dispatch       │   │
│   └─────────────────────────────┘   │
│            │                         │
│   ┌────────┼────────┬─────────┐      │
│   ▼        ▼        ▼         ▼      │
│ run_   run_    run_     run_   │
│index   search  doctor   init   │
│   │        │        │         │      │
│   ▼        ▼        ▼         ▼      │
│ indexer   search   doctor   init    │
│    │        │        │         │      │
│   db      db       db        db      │
└─────────────────────────────────────┘
```

### Benefits of the Orchestrator Pattern

1. **Separation of concerns** — Each handler does one thing
2. **Testability** — Can test each handler independently
3. **Maintainability** — Easy to modify one command without affecting others
4. **Readability** — Clear what happens for each command

---

## 6.9 Claude Code Integration

See `main.rs:150-230`:

```rust
// No args - try reading JSON from stdin (Claude Code integration)
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
                    false,
                    cli.quiet,
                );
            }
        }
    }
}
```

This enables integration with Claude Code's file suggestion feature:

```json
{"query": "main function implementation"}
```

---

## 6.10 Chapter Summary

| Concept | What We Learned |
|---------|-----------------|
| Orchestrator | main.rs coordinates everything |
| Dispatch | match statement routes to handlers |
| Health states | 7 states with different actions |
| Platform code | Windows vs Unix differences |
| Auto-init | Search creates database if needed |
| Integration | Claude Code stdin support |

---

## Exercises

### Exercise 6.1: Trace the Flow

When you run `ffts-grep search "main"`, trace through main.rs:

1. What does `Cli::parse()` return?
2. What condition matches in the `match` statement?
3. What function is called?
4. What does that function do?

**Deliverable:** Write a step-by-step trace.

### Exercise 6.2: Add a New Command

Add a `version` command that prints version info.

**Deliverable:** Show the changes needed in main.rs.

### Exercise 6.3: Platform Differences

Why does Windows need `MoveFileExW` while Unix just needs `rename()`?

**Deliverable:** Research and explain the OS differences.

### Exercise 6.4: Health States

Create a decision tree showing what happens for each health state during search.

**Deliverable:** Draw or describe the flow.

---

**Next Chapter**: [Chapter 7: db.rs - The Database Layer](07-db_rs.md)
