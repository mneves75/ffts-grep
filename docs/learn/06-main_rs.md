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

See `main.rs:57-189`:

```rust
fn main() -> Result<()> {
    // Parse command-line arguments
    let args = Cli::parse();

    // Initialize structured logging with current timestamp
    // Uses RUST_LOG format for compatibility with log aggregators
    tracing_subscribers::fmt::init();

    // Resolve project directory (single-pass detection)
    let project_dir = args.project_dir()?;

    // Dispatch to appropriate command handler
    match args.command {
        Some(Commands::Index { reindex }) => {
            run_indexing(&project_dir, reindex, args.quiet)?;
        }
        Some(Commands::Search { query, paths_only, format, max_results }) => {
            run_search(&project_dir, query, paths_only, format, max_results, args.quiet)?;
        }
        Some(Commands::Doctor { verbose }) => {
            run_doctor(&project_dir, args.format, verbose)?;
        }
        Some(Commands::Init { gitignore_only, force }) => {
            run_init(&project_dir, gitignore_only, force, args.quiet)?;
        }
        // No command = interactive mode (stdin JSON from Claude Code)
        None => {
            // Read query from stdin as JSON: {"query": "search term"}
            if let Some(query) = read_query_from_stdin()? {
                run_search(&project_dir, query, false, None, 15, true)?;
            }
        }
    }

    Ok(())
}
```

### Key Observations

1. **Parse first** — Get user input before doing anything
2. **Initialize logging** — Capture what's about to happen
3. **Dispatch** — Route to appropriate handler
4. **No command = Claude Code mode** — Special integration

---

## 6.3 Running Indexing

See `main.rs:197-349`:

```rust
fn run_indexing(project_dir: &Path, reindex: bool, quiet: bool) -> Result<IndexStats> {
    let config = build_pragma_config(&Cli::parse())?;

    // Handle reindex mode (atomic file replacement)
    if reindex {
        if !quiet {
            info!("Starting atomic reindex...");
        }

        // Create temp database, index all files, then atomically replace
        let stats = indexer::atomic_reindex(project_dir, &config)?;

        if !quiet {
            info!(
                files = stats.files_indexed,
                skipped = stats.files_skipped,
                bytes = stats.bytes_indexed,
                duration = ?stats.duration,
                "Reindex complete"
            );
        }

        return Ok(stats);
    }

    // Incremental index (skip unchanged files)
    if !quiet {
        info!("Starting incremental index...");
    }

    let db = db::Database::open(project_dir.join(DB_NAME), &config)?;
    db.init_schema()?;

    let mut indexer = indexer::Indexer::new(
        project_dir,
        db,
        indexer::IndexerConfig::default(),
    );

    let stats = indexer.index_directory()?;

    if !quiet {
        info!(
            files = stats.files_indexed,
            skipped = stats.files_skipped,
            bytes = stats.bytes_indexed,
            duration = ?stats.duration,
            "Index complete"
        );
    }

    Ok(stats)
}
```

### Two Indexing Modes

| Mode | When to Use | Behavior |
|------|-------------|----------|
| **Incremental** | Normal use | Skip unchanged files |
| **Reindex** | Corruption suspected | Rebuild everything |

---

## 6.4 Running Searches

See `main.rs:432-553`:

```rust
fn run_search(
    project_dir: &Path,
    query: String,
    paths_only: bool,
    format: Option<OutputFormat>,
    max_results: u32,
    quiet: bool,
) -> Result<()> {
    let config = build_pragma_config(&Cli::parse())?;

    // Fast health check with auto-init capability
    // This ensures we can search even if DB doesn't exist yet
    let health = health::check_health_fast(project_dir);

    match health {
        health::DatabaseHealth::Healthy => {
            // Continue with search
        }
        health::DatabaseHealth::Empty => {
            // Database exists but has no files - auto-init
            if !quiet {
                warn!("Database is empty, initializing...");
            }
            let _ = health::auto_init(project_dir, &config, true)?;
        }
        health::DatabaseHealth::Missing => {
            // No database - create one and index
            if !quiet {
                info!("No database found, initializing...");
            }
            let _ = health::auto_init(project_dir, &config, true)?;
        }
        health::DatabaseHealth::SchemaInvalid | health::DatabaseHealth::Corrupted => {
            // Database exists but is broken - reinitialize
            if !quiet {
                warn!("Database is corrupted, reinitializing...");
            }
            let _ = health::backup_and_reinit(project_dir, &config, true)?;
        }
        health::DatabaseHealth::WrongApplicationId => {
            return Err(IndexerError::ForeignDatabase);
        }
        health::DatabaseHealth::Unreadable => {
            return Err(IndexerError::Io {
                source: std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "Cannot read database",
                ),
            });
        }
    };

    // Open database for searching
    let mut db = db::Database::open(project_dir.join(DB_NAME), &config)?;

    // Configure and execute search
    let search_config = search::SearchConfig {
        paths_only: paths_only || format == Some(OutputFormat::Json),
        format: format.unwrap_or(OutputFormat::Plain),
        max_results,
    };

    let mut searcher = search::Searcher::new(&mut db, search_config);

    // Sanitize query and execute search
    let results = searcher.search(&query)?;

    // Output results
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    searcher.format_results(&results, &mut handle)?;

    Ok(())
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

---

## 6.5 Running Diagnostics

See `main.rs:556-577`:

```rust
fn run_doctor(
    project_dir: &Path,
    format: Option<OutputFormat>,
    verbose: bool,
) -> Result<doctor::DoctorSummary> {
    let mut doctor = doctor::Doctor::new(project_dir, verbose);
    let summary = doctor.run();

    let output_format = format.unwrap_or(OutputFormat::Plain);
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    doctor.output(&mut handle, output_format, &summary)?;

    // Exit with appropriate code based on severity
    if summary.has_errors() {
        std::process::exit(doctor::ExitCode::DataErr as i32);
    } else if summary.has_warnings() {
        std::process::exit(doctor::ExitCode::Software as i32);
    }

    Ok(summary)
}
```

---

## 6.6 Running Initialization

See `main.rs:584-713`:

```rust
fn run_init(
    project_dir: &Path,
    gitignore_only: bool,
    force: bool,
    quiet: bool,
) -> Result<init::InitResult> {
    let config = build_pragma_config(&Cli::parse())?;

    let result = init::initialize_project(project_dir, &config, gitignore_only, force)?;

    if !quiet {
        let stdout = std::io::stdout();
        let mut handle = stdout.lock();
        init::output_init_result(&mut handle, &result, quiet)?;
    }

    Ok(result)
}
```

---

## 6.7 Platform-Specific Code

See `main.rs:28-55`:

```rust
#[cfg(windows)]
fn atomic_replace(from: &Path, to: &Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };

    let from_wide: Vec<u16> = from.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    let to_wide: Vec<u16> = to.as_os_str().encode_wide().chain(std::iter::once(0)).collect();

    let result = unsafe {
        MoveFileExW(
            from_wide.as_ptr(),
            to_wide.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };

    if result == 0 {
        return Err(IndexerError::Io {
            source: std::io::Error::last_os_error(),
        });
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

See `main.rs:162-184`:

```rust
// No command = interactive mode (stdin JSON from Claude Code)
// Claude Code sends: {"query": "search term"}
if args.query.is_empty() {
    if let Some(query) = read_query_from_stdin()? {
        run_search(&project_dir, query, false, None, 15, true)?;
    }
    return Ok(());
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
