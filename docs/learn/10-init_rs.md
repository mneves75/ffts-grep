# Chapter 10: init.rs - Project Initialization

> "Well begun is half done." — Aristotle

## 10.1 What Does This File Do? (In Simple Terms)

The `init.rs` file handles **project initialization**—specifically, it ensures that the `.gitignore` file includes the database files so they don't get committed to version control. Think of it as the **setup crew** that prepares the venue before the main event.

### The Wedding Planner Analogy

Before a wedding:

| Wedding Planner | This init.rs |
|-----------------|--------------|
| Checks the venue is ready | Verifies `.gitignore` |
| Adds items to the checklist | Updates gitignore entries |
| Makes sure nothing is forgotten | Ensures all DB files are ignored |
| Tells you when everything is ready | Reports initialization status |

The init module doesn't create the database (that's the indexer's job)—it just makes sure `.gitignore` is properly configured.

---

## 10.2 Gitignore Entries

See `init.rs:23-25`:

```rust
/// Required gitignore entries for ffts-grep.
#[must_use]
pub const fn gitignore_entries() -> [&'static str; 4] {
    [DB_NAME, DB_SHM_NAME, DB_WAL_NAME, DB_TMP_NAME]
}
```

This returns the 4 files that need to be ignored:
- `.ffts-index.db` — Main database
- `.ffts-index.db-shm` — Shared memory file
- `.ffts-index.db-wal` — Write-Ahead Log
- `.ffts-index.db-tmp` — Temporary file

---

## 10.3 GitignoreResult Enum

See `init.rs:31-39`:

```rust
/// Result of gitignore update operation.
#[derive(Debug, PartialEq, Eq)]
pub enum GitignoreResult {
    /// Created new .gitignore file with N entries
    Created(usize),

    /// Added N entries to existing file
    Updated(usize),

    /// All entries already present
    AlreadyComplete,
}
```

| Variant | Meaning | Example |
|---------|---------|---------|
| `Created(n)` | New file created with n entries | First time running init |
| `Updated(n)` | Added n new entries | Some entries were missing |
| `AlreadyComplete` | Nothing to add | All entries already there |

---

## 10.4 Checking Gitignore

See `init.rs:53-66`:

```rust
/// Check if all required gitignore entries are present.
///
/// Returns a list of missing entries (empty if all present).
#[must_use]
pub fn check_gitignore(project_dir: &Path) -> Vec<&'static str> {
    let gitignore_path = project_dir.join(".gitignore");

    // If no .gitignore, all entries are missing
    let Ok(existing) = fs::read_to_string(&gitignore_path) else {
        return gitignore_entries().to_vec();
    };

    // Parse existing patterns (ignore comments and blanks)
    let existing_patterns: HashSet<&str> =
        existing.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).collect();

    // Find missing entries
    gitignore_entries().iter().filter(|e| !existing_patterns.contains(*e)).copied().collect()
}
```

This function:
1. Reads `.gitignore` (if it exists)
2. Parses each line, removing comments and blanks
3. Returns which entries are missing

---

## 10.5 Updating Gitignore (Atomic!)

See `init.rs:82-144`:

```rust
/// Update .gitignore with required entries (idempotent).
pub fn update_gitignore(project_dir: &Path) -> Result<GitignoreResult> {
    let gitignore_path = project_dir.join(".gitignore");

    // Read existing content
    let existing = fs::read_to_string(&gitignore_path).unwrap_or_default();
    let file_existed = !existing.is_empty() || gitignore_path.exists();

    // Parse existing patterns
    let existing_patterns: HashSet<&str> =
        existing.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).collect();

    // Find missing entries
    let missing: Vec<&str> =
        gitignore_entries().iter().filter(|e| !existing_patterns.contains(*e)).copied().collect();

    // If nothing missing, we're done
    if missing.is_empty() {
        return Ok(GitignoreResult::AlreadyComplete);
    }

    // Build new content
    let mut new_content = existing.clone();

    // Ensure trailing newline
    if !new_content.is_empty() && !new_content.ends_with('\n') {
        new_content.push('\n');
    }

    // Add header comment if not present
    if !existing.contains(GITIGNORE_HEADER) {
        if !new_content.is_empty() {
            new_content.push('\n');
        }
        new_content.push_str(GITIGNORE_HEADER);
        new_content.push('\n');
    }

    // Add missing entries
    for entry in &missing {
        new_content.push_str(entry);
        new_content.push('\n');
    }

    // Atomic write: temp file + rename
    let tmp_path = gitignore_path.with_extension("gitignore.tmp");

    let mut file = fs::File::create(&tmp_path)?;
    file.write_all(new_content.as_bytes())?;
    file.flush()?;
    drop(file);

    // Atomic rename
    fs::rename(&tmp_path, &gitignore_path)?;

    let count = missing.len();
    if file_existed {
        Ok(GitignoreResult::Updated(count))
    } else {
        Ok(GitignoreResult::Created(count))
    }
}
```

### Atomic Update Pattern

```rust
// Write to temp file
let mut file = fs::File::create(&tmp_path)?;
file.write_all(new_content.as_bytes())?;

// Atomic rename
fs::rename(&tmp_path, &gitignore_path)?;
```

This prevents corruption if the write is interrupted.

---

## 10.6 Initialization Result

See `init.rs:42-47`:

```rust
/// Result of init operation.
#[derive(Debug)]
pub struct InitResult {
    pub gitignore: GitignoreResult,
    pub database_created: bool,
    pub files_indexed: usize,
}
```

This combines all initialization results into one struct.

---

## 10.7 Chapter Summary

| Concept | What We Learned |
|---------|-----------------|
| Gitignore entries | 4 files to ignore |
| Idempotent operations | Safe to run multiple times |
| Atomic file writes | Temp file + rename |
| HashSet parsing | Efficient lookup of existing entries |
| Header comments | Documenting auto-generated sections |

---

## Exercises

### Exercise 10.1: Test Gitignore

Run `init` and observe the gitignore changes:

```bash
cat .gitignore
ffts-grep init
cat .gitignore
```

**Deliverable:** Show the before/after of .gitignore.

### Exercise 10.2: Run Init Multiple Times

Run `ffts-grep init` multiple times. What happens?

**Deliverable:** Show the output of each run.

### Exercise 10.3: Manual Gitignore

Manually remove an entry from .gitignore, then run `init`:

```bash
# Remove .ffts-index.db-wal from .gitignore
ffts-grep init
# What happens?
```

**Deliverable:** Explain the idempotent behavior.

### Exercise 10.4: Add a New Ignored File

Add a new temporary file type to ignore.

**Deliverable:** Show the code changes needed.

---

**Next Chapter**: [Chapter 11: doctor.rs - Diagnostics](11-doctor_rs.md)
