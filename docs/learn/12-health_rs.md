# Chapter 12: health.rs - Health Checking

> "A stitch in time saves nine." — English Proverb

## 12.1 What Does This File Do? (In Simple Terms)

The `health.rs` file implements a **fast health check** system that determines whether the database is ready to use. Unlike `doctor.rs` (which runs comprehensive diagnostics), `health.rs` is optimized for speed—completing in under 100 microseconds. This makes it suitable for automatic initialization during search operations.

### The Doctor's Triage Analogy

When you visit a hospital emergency room:

| ER Triage | This health.rs |
|-----------|----------------|
| Quick vital signs check | Fast database validation |
| Determine if treatment needed | Return health status |
| Flag critical issues | Separate Missing from Corrupted |
| Decide next steps | Healthy → use it, Missing → init |

The health module doesn't diagnose problems—it just answers: "Can I search this database right now, or do I need to initialize it first?"

---

## 12.2 Why Fast Health Checks Matter

See `health.rs:45-58`:

```rust
/// Check database health (optimized for <100μs completion).
///
/// This is NOT a full diagnostic—it's a quick "can I search?" check.
/// For comprehensive diagnostics, use `Doctor` instead.
///
/// # Performance
/// Uses Connection::immediate_transaction for fast validation
/// without full integrity check overhead.
#[tracing::instrument(level = "trace", skip(db_path))]
pub fn check_health(db_path: &Path) -> DatabaseHealth {
    // Fast-path: check if file exists (most common case)
    if !db_path.exists() {
        return DatabaseHealth::Missing;
    }
```

### Performance Comparison

| Check Type | Duration | Use Case |
|------------|----------|----------|
| Health check | < 100 μs | Before every search |
| Doctor check | ~10-50 ms | Manual diagnostics |
| Full integrity | ~100+ ms | Post-mortem analysis |

The health check is 100x faster than a full diagnostic because it only verifies the minimum necessary to answer: "Can I search now?"

---

## 12.3 The DatabaseHealth Enum

See `health.rs:60-87`:

```rust
/// Represents the health status of a database.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DatabaseHealth {
    /// Database exists, schema complete, has indexed content
    Healthy,

    /// Schema exists but no files indexed yet
    Empty,

    /// No database file found
    Missing,

    /// Cannot open database file (permissions, locked, corruption)
    Unreadable,

    /// Database exists but has wrong application_id
    WrongApplicationId,

    /// Database incomplete (missing tables, triggers, or indexes)
    SchemaInvalid,

    /// FTS5 integrity check failed (disk corruption)
    Corrupted,
}
```

### Health States Explained

| State | Meaning | User Action |
|-------|---------|-------------|
| `Healthy` | Ready to search | None needed |
| `Empty` | DB exists, no files | Index your project |
| `Missing` | No database | Run `init` |
| `Unreadable` | Permission issue | Check file permissions |
| `WrongApplicationId` | Wrong DB type | Delete and reinit |
| `SchemaInvalid` | Incomplete setup | Delete and reinit |
| `Corrupted` | Disk corruption | Delete and reindex |

### The Healthy vs Empty Distinction

```rust
// health.rs:89-106

/// Determines if a database is ready for searching.
///
/// A database is "healthy" if it exists, is readable, has correct
/// application_id, complete schema, AND has at least one indexed file.
/// An "empty" database has correct schema but zero files.
fn determine_readiness(db: &Connection) -> DatabaseHealth {
    // Check file count
    let count: i64 = db
        .query_row("SELECT COUNT(*) FROM files", [], |row| row.get(0))
        .unwrap_or(0);

    if count == 0 {
        DatabaseHealth::Empty
    } else {
        DatabaseHealth::Healthy
    }
}
```

This distinction matters because:
- An empty database might be mid-initialization
- A healthy database definitely has searchable content

---

## 12.4 Project Root Detection

See `health.rs:187-220`:

```rust
/// Represents a detected project root with metadata.
#[derive(Debug, Clone)]
pub struct ProjectRoot {
    /// Path to the project root directory
    pub path: PathBuf,

    /// How the root was detected
    pub method: DetectionMethod,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DetectionMethod {
    /// Found via existing database file
    ExistingDatabase,

    /// Found via .git directory
    GitRepository,

    /// Default fallback (current directory)
    Default,
}

/// Find the project root by searching upward from a starting directory.
///
/// Searches for:
/// 1. .ffts-index.db file (highest priority)
/// 2. .git directory (indicates Git project)
/// 3. Falls back to starting directory
///
/// Stops at filesystem root or when found.
#[tracing::instrument(level = "trace", fields(method))]
pub fn find_project_root(start_dir: &Path) -> ProjectRoot {
    let mut git_root: Option<PathBuf> = None;

    for ancestor in start_dir.ancestors() {
        // Check for database first (highest priority)
        let db_path = ancestor.join(DB_NAME);
        if is_valid_ffts_database(&db_path) {
            return ProjectRoot {
                path: ancestor.to_path_buf(),
                method: DetectionMethod::ExistingDatabase,
            };
        }

        // Track git root as backup
        if git_root.is_none() && ancestor.join(".git").exists() {
            tracing::trace!(path = %ancestor.display(), "Found git repository");
            git_root = Some(ancestor.to_path_buf());
        }

        // Stop if we hit filesystem root
        if ancestor.parent().is_none() {
            break;
        }
    }

    // Return git root or fallback to start directory
    ProjectRoot {
        path: git_root.unwrap_or_else(|| start_dir.to_path_buf()),
        method: git_root
            .map(|_| DetectionMethod::GitRepository)
            .unwrap_or(DetectionMethod::Default),
    }
}
```

### Why Detect Project Root?

The function handles multiple scenarios:

| Scenario | Starting Dir | Found | Project Root |
|----------|--------------|-------|--------------|
| Inside project | `src/subdir/` | `.ffts-index.db` in parent | Project root |
| Git repo, no DB | `src/subdir/` | `.git` in parent | Project root |
| Fresh project | Any directory | Nothing found | Starting directory |
| Submodule | Nested directory | `.git` in parent | Parent git root |

---

## 12.5 Validating ffts-grep Databases

See `health.rs:222-285`:

```rust
/// Verify a file is a valid ffts-grep database.
///
/// Performs lightweight validation:
/// 1. File exists and is readable
/// 2. Has correct application_id (0xA17E_6D42)
/// 3. Has expected schema (files table exists)
///
/// Does NOT perform full integrity check (too slow for detection).
///
/// # Arguments
/// * `db_path` - Path to potential database file
///
/// # Returns
/// true if file appears to be a valid ffts-grep database
#[tracing::instrument(level = "trace", skip(db_path))]
pub fn is_valid_ffts_database(db_path: &Path) -> bool {
    // Check file existence first (fastest check)
    if !db_path.exists() {
        tracing::trace!(path = %db_path.display(), "Database file does not exist");
        return false;
    }

    // Open with READONLY to avoid locking issues
    match Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READONLY) {
        Ok(conn) => {
            // Verify application_id (magic number)
            let app_id: i32 = conn
                .pragma_query_value(None, "application_id", |row| row.get(0))
                .unwrap_or(0);

            if app_id != 0xA17E_6D42 {
                tracing::trace!(path = %db_path.display(), expected_app_id = 0xA17E_6D42, actual_app_id = app_id,
                    "Application ID mismatch");
                return false;
            }

            // Verify files table exists
            let has_files = conn
                .pragma_query_value(None, "table_files", |row| row.get::<_, Option<String>>(0).is_some())
                .unwrap_or(false);

            if !has_files {
                tracing::trace!(path = %db_path.display(), "Missing files table");
                return false;
            }

            tracing::trace!(path = %db_path.display(), "Valid ffts-grep database");
            true
        }
        Err(e) => {
            tracing::trace!(path = %db_path.display(), error = %e, "Failed to open database");
            false
        }
    }
}
```

### What Validates vs. What Doesn't

This function is intentionally **lightweight**:

| Check | Included | Reason |
|-------|----------|--------|
| File exists | Yes | Fast, common case |
| Application ID | Yes | Identifies our DB type |
| Files table exists | Yes | Schema minimum |
| FTS5 integrity | No | Too slow for detection |
| Row count | No | Too slow for detection |
| Trigger existence | No | Too slow for detection |

This is the **Time-Of-Check Time-Of-Use (TOCTOU)** pattern: we accept a small window where state might change, because the alternative (full validation) would be too slow.

---

## 12.6 Auto-Init: Automatic Project Setup

See `health.rs:302-444`:

```rust
/// Automatically initialize project if needed during search.
///
/// This function:
/// 1. Checks if database exists
/// 2. If not, creates it (with gitignore entries)
/// 3. Indexes all files
///
/// Uses unique temp files per process+thread to handle
/// concurrent initialization attempts safely.
///
/// # Returns
/// Ok(()) if initialization succeeded or wasn't needed
/// Err(...) if initialization failed
///
/// # Race Condition Handling
/// Multiple processes may call this simultaneously.
/// The winner creates the database; losers clean up and retry.
#[tracing::instrument(level = "info", skip(start_dir), fields(path = %start_dir.display()))]
pub fn auto_init(start_dir: &Path) -> Result<AutoInitResult> {
    let project_root = find_project_root(start_dir);
    let db_path = project_root.path.join(DB_NAME);

    // Fast-path: if DB exists and is healthy, we're done
    match check_health(&db_path) {
        DatabaseHealth::Healthy => {
            tracing::info!(path = %db_path.display(), "Database already healthy");
            return Ok(AutoInitResult {
                status: AutoInitStatus::AlreadyComplete,
                project_root: project_root.path,
            });
        }
        DatabaseHealth::Empty => {
            tracing::info!(path = %db_path.display(), "Database empty, indexing files");
            // Continue to indexing below
        }
        DatabaseHealth::Missing => {
            tracing::info!(path = %db_path.display(), "Database missing, initializing");
            // Continue to initialization below
        }
        other => {
            // Corrupted, wrong type, etc. - need to reinit
            tracing::warn!(status = ?other, "Database needs reinitialization");
        }
    }

    // ... initialization logic with race-safe temp files
}
```

### The Auto-Init Flow

```
                    ┌──────────────────────┐
                    │  User runs search    │
                    └──────────┬───────────┘
                               │
                    ┌──────────▼───────────┐
                    │ check_health()       │
                    └──────────┬───────────┘
                               │
              ┌────────────────┼────────────────┐
              │                │                │
        ┌─────▼─────┐    ┌─────▼─────┐    ┌─────▼─────┐
        │  Healthy  │    │   Empty   │    │  Missing  │
        └─────┬─────┘    └─────┬─────┘    └─────┬─────┘
              │                │                │
              ▼                ▼                ▼
        Use existing    Index files      Init + Index
        database        into empty DB    new database
```

### Why Auto-Init Matters

Without auto-init, users would see:

```bash
$ ffts-grep search "main"
Error: No database found. Run 'ffts-grep init' first.

$ ffts-grep init
...wait for init...

$ ffts-grep search "main"
...finally search...
```

With auto-init, they just search:

```bash
$ ffts-grep search "main"
[auto-init] Creating database...
[auto-init] Indexing files...
src/main.rs
```

This is a **quality of life** feature that reduces friction.

---

## 12.7 Race Condition Handling

See `health.rs:350-420`:

```rust
/// Initialize with race-safe temp file pattern.
///
/// Creates database in a unique temp file, indexes, then
/// atomically renames to the final location.
///
/// This handles the case where multiple processes try to
/// initialize simultaneously:
///
/// Process A (winner):
///   1. Creates temp file (.ffts-index.db.init.PID.TID.random)
///   2. Builds complete index
///   3. Atomic rename to .ffts-index.db
///
/// Process B (loser):
///   1. Creates its own temp file
///   2. Builds index
///   3. Atomic rename (fails because A already renamed)
///   4. Retries using existing DB from winner
fn initialize_race_safe(project_root: &Path) -> Result<AutoInitResult> {
    // Generate unique temp file name
    let temp_file = generate_unique_temp_name();

    // Open database at temp location
    let temp_path = project_root.join(&temp_file);
    let db = Database::open(&temp_path, PragmaConfig::default())?;
    db.init_schema()?;

    // Index files
    let mut indexer = Indexer::new(
        project_root,
        db,
        IndexerConfig::default(),
    );
    let stats = indexer.index_directory()?;

    // Close connection before rename (required on some platforms)
    drop(indexer);

    // Atomic rename
    let final_path = project_root.join(DB_NAME);
    match atomic_replace(&temp_path, &final_path) {
        Ok(()) => {
            tracing::info!(path = %final_path.display(), "Successfully initialized");
            Ok(AutoInitResult {
                status: AutoInitStatus::Initialized { stats },
                project_root: project_root.to_path_buf(),
            })
        }
        Err(e) => {
            // Race condition: another process won
            tracing::info!(error = %e, "Lost race to initialize, using winner's database");

            // Clean up our temp file
            let _ = fs::remove_file(&temp_path);

            // The winner's database is now available
            // Verify it's healthy
            match check_health(&final_path) {
                DatabaseHealth::Healthy => {
                    Ok(AutoInitResult {
                        status: AutoInitStatus::AlreadyComplete,
                        project_root: project_root.to_path_buf(),
                    })
                }
                other => {
                    Err(IndexerError::AutoInit {
                        source: Box::new(std::io::Error::new(
                            std::io::ErrorKind::Other,
                            format!("Race loser detected but winner's DB is unhealthy: {:?}", other),
                        )),
                    })
                }
            }
        }
    }
}
```

### Race Condition Timeline

| Time | Process A | Process B |
|------|-----------|-----------|
| T1 | Check: DB missing | Check: DB missing |
| T2 | Create temp.A | Create temp.B |
| T3 | Index files... | Index files... |
| T4 | Rename: temp.A → DB | - |
| T5 | Complete | Rename: temp.B → DB FAILS |
| T6 | - | Detect race, use A's DB |

### Unique Temp File Generation

```rust
/// Generate a unique temporary filename for race-safe initialization.
///
/// Format: .ffts-index.db.init.{PID}.{TID}.{RANDOM}
/// Using process ID, thread ID, and random suffix ensures
/// uniqueness across concurrent processes and threads.
fn generate_unique_temp_name() -> String {
    let pid = std::process::id();
    let tid = std::thread::current().id().as_u64();
    let random: u64 = rand::thread_rng().gen();

    format!("{DB_NAME}.init.{pid}.{tid}.{random:016x}")
}
```

This prevents:
- Two processes using the same temp file
- Thread collisions within a process
- Leftover temp files from crashed processes

---

## 12.8 Backup and Restore

See `health.rs:489-560`:

```rust
/// Backup the current database to a timestamped file.
///
/// Creates a backup in the same directory with timestamp:
/// .ffts-index.db.backup.YYYYMMDD.HHMMSS
///
/// # Safety
/// Performs checkpoint before backup to ensure all WAL
/// changes are in the main file.
///
/// # Errors
/// Returns error if:
/// - Database is locked
/// - Cannot checkpoint
/// - Cannot read source file
/// - Cannot write backup file
pub fn backup_database(project_dir: &Path) -> Result<BackupResult> {
    let db_path = project_dir.join(DB_NAME);
    let backup_path = generate_backup_path(project_dir);

    // Ensure all changes are in main file
    let db = Database::open(&db_path, PragmaConfig::default())?;
    db.conn().pragma_update(None, "wal_checkpoint", "TRUNCATE")?;

    // Copy database file
    std::fs::copy(&db_path, &backup_path)?;

    Ok(BackupResult {
        backup_path,
        size_bytes: std::fs::metadata(&backup_path)?.len(),
    })
}

/// Generate backup filename with timestamp.
fn generate_backup_path(project_dir: &Path) -> PathBuf {
    let now = chrono::Local::now();
    let timestamp = now.format("%Y%m%d.%H%M%S").to_string();
    project_dir.join(format!("{DB_NAME}.backup.{timestamp}"))
}
```

### When to Backup

| Situation | Backup? | Reason |
|-----------|---------|--------|
| Before first index | No | Nothing to lose |
| After successful index | Optional | Safety net |
| Before reindex | Yes | Can't undo reindex |
| Corruption detected | Maybe | May copy corruption |

---

## 12.9 The AutoInitResult Struct

See `health.rs:118-143`:

```rust
/// Result of an auto-initialization operation.
#[derive(Debug, Clone)]
pub struct AutoInitResult {
    /// The result status of initialization
    pub status: AutoInitStatus,

    /// Path to the project root (where database was created/found)
    pub project_root: PathBuf,
}

/// Possible outcomes of auto-initialization.
#[derive(Debug, Clone, PartialEq)]
pub enum AutoInitStatus {
    /// Database already existed and was healthy
    AlreadyComplete,

    /// Successfully created new database and indexed files
    Initialized { stats: IndexStats },

    /// Database existed but was unhealthy, replaced with new one
    Replaced { stats: IndexStats },
}
```

This structure communicates exactly what happened during initialization, enabling the UI or CLI to display appropriate messages.

---

## 12.10 TOCTOU: Acceptable Trade-offs

See `health.rs:260-280`:

```rust
/// Note on TOCTOU (Time-Of-Check Time-Of-Use):
///
/// This function accepts a small window where state may change
/// between checking and using. This is ACCEPTABLE because:
///
/// 1. The check is for user convenience, not security
/// 2. If DB is deleted after our check, we'll report the error
/// 3. Full validation would make auto-init too slow
/// 4. Race losers clean up automatically
///
/// For security-critical validation (e.g., doctor checks),
/// use more thorough validation with locking.
///
/// See: https://en.wikipedia.org/wiki/Time-of-check_to_time-of-use
```

### Why TOCTOU Is Acceptable Here

| Factor | Analysis |
|--------|----------|
| **Security impact** | None. This is search, not authentication. |
| **Data loss risk** | Low. We only read from the database. |
| **Performance cost** | Full validation would be 100x slower. |
| **Failure mode** | If DB disappears, we report error and exit. |

Compare to **doctor.rs** which doesn't use TOCTOU—it performs thorough validation because the purpose is finding problems, not quick convenience.

---

## 12.11 Integration: Health in the CLI

See `health.rs:572-620` and `main.rs:432-553`:

```rust
/// Health-based auto-init for search command.
///
/// This is the integration point between health checking
/// and the search command in main.rs.
pub async fn health_aware_search(
    cli: &Cli,
    project_dir: &Path,
) -> Result<SearchResult> {
    let db_path = project_dir.join(DB_NAME);

    // Fast health check before search
    match check_health(&db_path) {
        DatabaseHealth::Healthy => {
            // Proceed with search immediately
        }
        DatabaseHealth::Empty | DatabaseHealth::Missing => {
            // Auto-init if needed (user will see progress)
            println!("[auto-init] Initializing database...");
            auto_init(project_dir)?;
            println!("[auto-init] Done. Searching...");
        }
        other => {
            // Other states require user attention
            return Err(IndexerError::HealthCheck {
                status: format!("{:?}", other),
                suggestion: "Run 'ffts-grep doctor' for details",
            });
        }
    }

    // ... proceed with search
}
```

The CLI integration ensures users never see confusing "database not found" errors—they just get automatic initialization.

---

## 12.12 Chapter Summary

| Concept | What We Learned |
|---------|----------------|
| Health check | Fast validation (<100μs) vs. full diagnostics |
| DatabaseHealth states | 7 states from Healthy to Corrupted |
| Project root detection | Finds DB or git root, then defaults |
| Validation lightweight | Only checks app_id and table existence |
| Auto-init | Transparent initialization during search |
| Race conditions | Multiple processes handled safely |
| Unique temp files | PID + TID + random for safety |
| TOCTOU | Acceptable for convenience features |
| Backup/restore | Timestamp-based snapshots |

---

## 12.13 Exercises

### Exercise 12.1: Test Health States

Create databases in various states and check their health:

```bash
# Missing database
rm -f .ffts-index.db
ffts-grep doctor

# Empty database
ffts-grep init --force
sqlite3 .ffts-index.db "DELETE FROM files"
ffts-grep doctor

# Corrupted database (careful!)
cp .ffts-index.db corrupted.db
ffts-grep doctor corrupted.db
```

**Deliverable:** Show the health status output for each state.

### Exercise 12.2: Project Root Detection

Create this directory structure:

```
/tmp/
  myproject/
    .git/
    src/
      code.rs
```

Run from `/tmp/myproject/src/` and observe where it finds the root:

```bash
ffts-grep doctor --project-dir /tmp/myproject/src/
```

**Deliverable:** Explain which method was used (Database, Git, or Default).

### Exercise 12.3: Race Condition Test

Simulate concurrent initialization:

```bash
# Terminal 1
ffts-grep init

# Terminal 2 (run simultaneously)
ffts-grep init
```

**Deliverable:** Show both outputs. Did one detect the race?

### Exercise 12.4: Backup and Restore

Test the backup functionality:

```bash
# Create some content
ffts-grep init
ffts-grep index

# Backup
ffts-grep doctor --backup

# Check backup file exists
ls -la .ffts-index.db.backup.*

# Restore from backup
ffts-grep doctor --restore .ffts-index.db.backup.*
```

**Deliverable:** Show the backup file and verify restoration works.

### Exercise 12.5: Design a Health Check

Design a health check for a different scenario (e.g., a web server's database).

**Deliverable:** Write pseudocode for the health check function.

---

## 12.14 Self-Correction Exercise

Review the health checking code and identify potential improvements:

1. **What if the database is locked by another process?**
   - Current behavior: `Unreadable`
   - Better approach: Retry with timeout?

2. **What if the database is huge (>1GB)?**
   - Current behavior: `check_health` might timeout
   - Better approach: Skip row count check?

3. **What if we're on a read-only filesystem?**
   - Current behavior: Fails on init
   - Better approach: Detect and report early?

**Deliverable:** For each issue, propose a code change and explain the trade-off.

---

**Next Chapter**: [Chapter 13: testing.md - Testing the Application](13-testing.md)
