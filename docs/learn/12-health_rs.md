# Chapter 12: health.rs - Health Checking

> "A stitch in time saves nine." — English Proverb

## 12.1 What Does This File Do? (In Simple Terms)

The `health.rs` file implements **fast database health checks** and **auto-init** helpers.
Unlike `doctor.rs` (full diagnostics), these checks are optimized for speed so search
can decide quickly whether to proceed, initialize, or bail out.

### The ER Triage Analogy

| ER Triage | health.rs |
|-----------|-----------|
| Quick vitals | Fast database validation |
| Decide treatment | Healthy vs Missing vs Corrupted |
| Escalate critical cases | Wrong app id / unreadable |

---

## 12.2 Fast Health Checks

See `health.rs:233-268`:

```rust
#[must_use]
pub fn check_health_fast(project_dir: &Path) -> DatabaseHealth {
    let db_path = project_dir.join(DB_NAME);

    if !db_path.exists() {
        return DatabaseHealth::Missing;
    }

    let db = match Database::open_readonly(&db_path) {
        Ok(db) => db,
        Err(_) => return DatabaseHealth::Unreadable,
    };

    match db.get_application_id() {
        Some(id) if id == EXPECTED_APPLICATION_ID => {}
        Some(_) => return DatabaseHealth::WrongApplicationId,
        None => return DatabaseHealth::Corrupted,
    }

    if !db.check_schema().is_complete() {
        return DatabaseHealth::SchemaInvalid;
    }

    match db.get_file_count() {
        Ok(0) => DatabaseHealth::Empty,
        Ok(_) => DatabaseHealth::Healthy,
        Err(_) => DatabaseHealth::Corrupted,
    }
}
```

**Why this is fast:**
- Uses `open_readonly` (no WAL writes).
- Checks only essentials (file exists, app id, schema completeness, file count).
- Skips expensive integrity checks.

---

## 12.3 The DatabaseHealth Enum

See `health.rs:69-109`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DatabaseHealth {
    Healthy,
    Empty,
    Missing,
    Unreadable,
    WrongApplicationId,
    SchemaInvalid,
    Corrupted,
}
```

| State | Meaning | Typical Action |
|-------|---------|----------------|
| `Healthy` | Searchable | Proceed |
| `Empty` | No files indexed | Auto-init/index |
| `Missing` | No DB file | Auto-init |
| `Unreadable` | Permission/lock | Fail with NoPerm |
| `WrongApplicationId` | Different app | Fail with DataErr |
| `SchemaInvalid` | Missing tables/triggers | Backup + reinit |
| `Corrupted` | Integrity failure | Backup + reinit |

`#[non_exhaustive]` means callers must handle future variants.

---

## 12.4 Project Root Detection

See `health.rs:139-212`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetectionMethod {
    ExistingDatabase,
    GitRepository,
    Fallback,
}

#[derive(Debug, Clone)]
pub struct ProjectRoot {
    pub path: PathBuf,
    pub method: DetectionMethod,
}

#[must_use]
pub fn find_project_root(start_dir: &Path) -> ProjectRoot {
    let mut git_root: Option<PathBuf> = None;

    for ancestor in start_dir.ancestors() {
        let db_path = ancestor.join(DB_NAME);
        if is_valid_ffts_database(&db_path) {
            return ProjectRoot { path: ancestor.to_path_buf(), method: DetectionMethod::ExistingDatabase };
        }

        if git_root.is_none() && ancestor.join(".git").exists() {
            git_root = Some(ancestor.to_path_buf());
        }
    }

    match git_root {
        Some(path) => ProjectRoot { path, method: DetectionMethod::GitRepository },
        None => ProjectRoot { path: start_dir.to_path_buf(), method: DetectionMethod::Fallback },
    }
}
```

Priority order:
1. Valid ffts-grep DB in an ancestor directory
2. Nearest `.git`
3. The starting directory

---

## 12.5 Auto-Init and Recovery

### Auto-init

See `health.rs:274-418`:

```rust
pub fn auto_init(project_dir: &Path, config: &PragmaConfig, quiet: bool) -> Result<IndexStats> {
    auto_init_with_config(project_dir, config, IndexerConfig::default(), quiet)
}

pub fn auto_init_with_config(
    project_dir: &Path,
    config: &PragmaConfig,
    indexer_config: IndexerConfig,
    quiet: bool,
) -> Result<IndexStats> {
    let _ = init::update_gitignore(project_dir);

    let thread_id_hash = /* hash std::thread::current().id() */;
    let unique_suffix = format!("{}_{:x}", std::process::id(), thread_id_hash);
    let tmp_path = project_dir.join(format!("{DB_NAME}{DB_TMP_SUFFIX}.{unique_suffix}"));

    let db = Database::open(&tmp_path, config)?;
    db.init_schema()?;

    let mut indexer = Indexer::new(project_dir, db, indexer_config);
    let stats = indexer.index_directory()?;

    // WAL checkpoint + atomic rename happen before finalizing
    // (see health.rs for full race-safe handling)

    Ok(stats)
}
```

Key behaviors:
- Uses **unique temp files** per process+thread to avoid collisions.
- Runs a **WAL checkpoint** before rename to prevent data loss.
- If another process wins the race, cleanup is best-effort and **not an error**.

### Backup + Reinit

See `health.rs:427-518`:

```rust
pub fn backup_and_reinit(project_dir: &Path, config: &PragmaConfig, quiet: bool) -> Result<IndexStats> {
    backup_and_reinit_with_config(project_dir, config, IndexerConfig::default(), quiet)
}

pub fn backup_and_reinit_with_config(
    project_dir: &Path,
    config: &PragmaConfig,
    indexer_config: IndexerConfig,
    quiet: bool,
) -> Result<IndexStats> {
    let db_path = project_dir.join(DB_NAME);
    let timestamp = /* UNIX timestamp seconds */;
    let backup_path = project_dir.join(format!("{DB_NAME}.backup.{timestamp}"));
    let _ = fs::rename(&db_path, &backup_path);
    let _ = fs::remove_file(project_dir.join(format!("{DB_NAME}-shm")));
    let _ = fs::remove_file(project_dir.join(format!("{DB_NAME}-wal")));

    auto_init_with_config(project_dir, config, indexer_config, quiet)
}
```

---

## 12.6 TOCTOU and Race Safety

Health checks are **snapshots**. Between `check_health_fast` and `auto_init`, another
process may act. The implementation is safe because:

- **Temp files are unique** per process/thread.
- **Atomic rename** is used for the final DB move.
- If rename fails (race lost), the winner's DB is used.

---

## 12.7 Chapter Summary

| Concept | What We Learned |
|---------|-----------------|
| Fast health checks | `check_health_fast` is the hot-path gate |
| Root detection | Prefer valid DB, then `.git`, then fallback |
| Auto-init | Unique temp + WAL checkpoint + atomic rename |
| Recovery | Backup corrupted DB, then reinitialize |

---

## Exercises

### Exercise 12.1: Health States

Create an empty project directory and run:

```bash
ffts-grep search "test" --no-auto-init
```

**Deliverable:** What `ExitCode` do you get and why?

### Exercise 12.2: Reinit Flow

Corrupt the database and run:

```bash
ffts-grep init --force
```

**Deliverable:** Which files are removed or backed up?
