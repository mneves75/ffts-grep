# Chapter 8: indexer.rs - Directory Walking

> "The journey of a thousand files begins with a single walk." — Adapted from Lao Tzu

## 8.1 What Does This File Do? (In Simple Terms)

The `indexer.rs` file is the **file walker**—it walks through your project directory, reads each file's contents, and stores them in the database. Think of it as a librarian who walks through the stacks, reading each book's title and content to update the card catalog.

### The Bouncer Analogy

The indexer is like a bouncer at a club:

| Bouncer | This Indexer |
|---------|--------------|
| Checks ID at the door | Validates files before indexing |
| Rejects underage | Skips binary files |
| Knows the VIP list | Respects `.gitignore` |
| Doesn't let trouble in | Prevents symlink attacks |

The bouncer decides what gets into the club (indexed) and what stays out (skipped).

---

## 8.2 IndexerConfig: Configuration

See `indexer.rs:14-33`:

```rust
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
            max_file_size: 1024 * 1024, // 1MB default
            batch_size: 500,
            follow_symlinks: false,
        }
    }
}
```

| Setting | Default | Purpose |
|---------|---------|---------|
| `max_file_size` | 1MB | Skip large files |
| `batch_size` | 500 | Transaction batching |
| `follow_symlinks` | false | Include symlinked files (opt-in) |

---

## 8.3 The Indexer Struct

See `indexer.rs:73-86`:

```rust
/// FTS5 file indexer.
///
/// Uses the `ignore` crate for gitignore-aware directory walking.
pub struct Indexer {
    db: Database,
    root: PathBuf,
    config: IndexerConfig,
}

impl Indexer {
    /// Create a new indexer for the given project root.
    pub fn new(root: &Path, db: Database, config: IndexerConfig) -> Self {
        Self {
            db,
            root: root.to_path_buf(),
            config,
        }
    }
}
```

The indexer holds:
- A database connection (to store results)
- The root path (where to start walking)
- Configuration (how to index)

---

## 8.4 The Main Indexing Loop

See `indexer.rs:95-182`:

```rust
/// Index all files in the project directory (incremental).
pub fn index_directory(&mut self) -> Result<IndexStats> {
    // Transaction threshold: start transaction after 50 files
    const TRANSACTION_THRESHOLD: usize = 50;

    let start = SystemTime::now();

    // Create gitignore-aware directory walker
    let walk = WalkBuilder::new(&self.root)
        .standard_filters(true)  // Respect .gitignore
        .same_file_system(true)  // Don't cross filesystems
        .follow_links(self.config.follow_symlinks) // Opt-in symlink traversal
        .build();

    let mut stats = IndexStats::default();
    let mut batch_count = 0;
    let mut transaction_started = false;

    // Walk through all entries
    for result in walk {
        match result {
            Ok(entry) => {
                // Process file or skip
                match self.process_entry(&entry, &mut stats) {
                    Ok(needs_commit) => {
                        if needs_commit {
                            batch_count += 1;

                            // Start transaction after threshold
                            if batch_count == TRANSACTION_THRESHOLD && !transaction_started {
                                self.db.conn().execute("BEGIN IMMEDIATE", [])?;
                                transaction_started = true;
                            }

                            // Commit every batch_size files
                            if transaction_started && batch_count >= self.config.batch_size {
                                self.db.conn().execute("COMMIT", [])?;
                                self.db.conn().execute("BEGIN IMMEDIATE", [])?;
                                batch_count = TRANSACTION_THRESHOLD;
                            }
                        }
                    }
                    Err(e @ IndexerError::Database { .. }) => {
                        // Database errors are fatal: avoid silent partial indexes.
                        if transaction_started {
                            let _ = self.db.conn().execute("ROLLBACK", []);
                        }
                        return Err(e);
                    }
                    Err(e) => {
                        // File-level errors are non-fatal; log and continue.
                        tracing::warn!(path = %entry.path().display(), error = %e, "Failed to index file");
                        stats.files_skipped += 1;
                    }
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "Directory walk error");
            }
        }
    }

    // Final commit if transaction was started
    if transaction_started {
        self.db.conn().execute("COMMIT", [])?;
    }

    // Run optimizations after bulk changes
    self.db.conn().execute("ANALYZE", [])?;
    self.db.optimize()?;
    self.db.optimize_fts()?;

    stats.duration = start.elapsed().unwrap_or_default();
    Ok(stats)
}
```

### Transaction Batching Explained

| Phase | Files Processed | Action |
|-------|-----------------|--------|
| 1 | 1-49 | Auto-commit each INSERT |
| 2 | 50-499 | Single transaction, commit at 500 |
| 3 | 500+ | Commit every 500 files |

This reduces disk I/O significantly for large codebases while still failing fast
on database write errors to avoid silently incomplete indexes.

---

## 8.5 Processing Each Entry

See `indexer.rs:184-273`:

```rust
fn process_entry(&self, entry: &DirEntry, stats: &mut IndexStats) -> Result<bool> {
    let path = entry.path();

    // Skip database files themselves
    if Self::is_database_file(path) {
        return Ok(false);
    }

    // Handle symlinks (symlink_metadata avoids following links)
    let is_symlink = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata.file_type().is_symlink(),
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "Failed to read symlink metadata");
            stats.files_skipped += 1;
            return Ok(false);
        }
    };

    if is_symlink {
        if !self.config.follow_symlinks {
            stats.files_skipped += 1;
            return Ok(false);
        }

        // Verify symlink stays within project root
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

    // Get file metadata
    let metadata = entry.metadata()?;

    // Skip large files
    if metadata.len() > self.config.max_file_size {
        stats.files_skipped += 1;
        return Ok(false);
    }

    // Read file content
    let content = match self.read_file_content(path, metadata.len()) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "Failed to read file");
            stats.files_skipped += 1;
            return Ok(false);
        }
    };

    // Calculate relative path from root
    let rel_path = path.strip_prefix(&self.root)
        .map_err(|_| IndexerError::PathTraversal { path: path.to_string_lossy().to_string() })?;

    // Get modification time
    let mtime = metadata.modified()?.duration_since(UNIX_EPOCH)?.as_secs() as i64;

    // Upsert into database (lazy invalidation)
    self.db.upsert_file(&rel_path.to_string_lossy(), &content, mtime, metadata.len() as i64)?;

    stats.files_indexed += 1;
    stats.bytes_indexed += metadata.len();

    Ok(true)
}
```

---

## 8.6 Reading File Content

See `indexer.rs:285-305`:

```rust
fn read_file_content(&self, path: &Path, size: u64) -> Result<String> {
    // Check size limit first
    if size > self.config.max_file_size {
        return Err(IndexerError::FileTooLarge { size, max: self.config.max_file_size });
    }

    // Open file
    let file = File::open(path)
        .map_err(|e| IndexerError::Io { source: e })?;

    let max_size = self.config.max_file_size;
    // Pre-allocate buffer with known capacity
    let capacity = std::cmp::min(size, max_size);
    let mut bytes = Vec::with_capacity(capacity as usize);

    // Read at most max_size + 1 bytes to detect concurrent growth
    let read_limit = max_size.saturating_add(1);
    file.take(read_limit)
        .read_to_end(&mut bytes)
        .map_err(|e| IndexerError::Io { source: e })?;

    if bytes.len() as u64 > max_size {
        return Err(IndexerError::FileTooLarge { size: bytes.len() as u64, max: max_size });
    }

    // Convert to String with UTF-8 validation
    String::from_utf8(bytes)
        .map_err(|_| IndexerError::InvalidUtf8 { path: path.to_string_lossy().to_string() })
}
```

### Why Pre-allocate?

```rust
let mut bytes = Vec::with_capacity(capacity as usize);
```

This tells Rust: "We're going to need `size` bytes." Rust allocates once, avoiding reallocations as we read.

---

## 8.7 Gitignore-Aware Walking

See `indexer.rs:101-105`:

```rust
let walk = WalkBuilder::new(&self.root)
    .standard_filters(true)  // Enable .gitignore support
    .same_file_system(true)  // Don't cross filesystem boundaries
    .follow_links(self.config.follow_symlinks) // Opt-in symlink traversal
    .build();
```

The `ignore` crate's `WalkBuilder` provides:
- `.gitignore` parsing
- `.git` directory automatic exclusion
- Standard filters (node_modules, etc.)

---

## 8.8 Security: Symlink Handling

See `indexer.rs:198-223` and `indexer.rs:308-324`:

```rust
#[inline]
fn is_within_root(&self, path: &Path) -> bool {
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
```

This prevents **symlink attacks** where someone creates a symlink like:
```
ln -s /etc/passwd project/malicious
```

Without this check, the indexer would read `/etc/passwd` and index it!

---

## 8.9 Atomic Reindex

See `indexer.rs:395-440`:

```rust
/// Force a full reindex by creating a new database.
pub fn atomic_reindex(root: &Path, config: &PragmaConfig) -> Result<IndexStats> {
    atomic_reindex_with_config(root, config, IndexerConfig::default())
}

/// Atomic reindex with explicit indexer configuration.
pub fn atomic_reindex_with_config(
    root: &Path,
    config: &PragmaConfig,
    indexer_config: IndexerConfig,
) -> Result<IndexStats> {
    let db_path = root.join(DB_NAME);
    let tmp_path = root.join(DB_TMP_NAME);

    // Clean up any existing temp file
    let _ = fs::remove_file(&tmp_path);

    // Create new database in temp location
    let db = Database::open(&tmp_path, config)?;
    db.init_schema()?;

    // Index all files (index_directory runs ANALYZE + optimize passes)
    let mut indexer = Indexer::new(root, db, indexer_config);
    let stats = indexer.index_directory()?;

    // Checkpoint WAL to main file
    indexer.db.conn().query_row(
        "PRAGMA wal_checkpoint(TRUNCATE)",
        [],
        |row| {
            let _busy: i64 = row.get(0)?;
            let _log: i64 = row.get(1)?;
            let _checkpointed: i64 = row.get(2)?;
            Ok(())
        },
    )?;

    // Close connection before rename
    drop(indexer);

    // Atomic rename (cross-platform)
    atomic_replace(&tmp_path, &db_path)?;

    // Clean up old WAL files
    let _ = fs::remove_file(root.join(format!("{DB_NAME}{DB_SHM_SUFFIX}")));
    let _ = fs::remove_file(root.join(format!("{DB_NAME}{DB_WAL_SUFFIX}")));

    Ok(stats)
}
```

The atomic reindex process:
1. Create new database at `.ffts-index.db.tmp`
2. Index all files
3. Checkpoint WAL (move all changes to main file)
4. Close connection
5. Atomic rename `.ffts-index.db.tmp` → `.ffts-index.db`
6. Clean up old WAL files

---

## 8.10 Chapter Summary

| Concept | What We Learned |
|---------|-----------------|
| WalkBuilder | Gitignore-aware directory traversal |
| Transaction batching | Start transaction after 50 files |
| UTF-8 validation | Binary files are rejected |
| Symlink security | Verify symlinks stay within root |
| Lazy invalidation | Skip unchanged files |
| Atomic reindex | Create temp db, then rename |

---

## Exercises

### Exercise 8.1: Explore File Walking

Create a test directory with various files and symlinks:

```bash
mkdir -p test_dir/src
echo "code" > test_dir/src/main.rs
echo "config" > test_dir/config.json
ln -s /etc/hostname test_dir/symlink
echo "binary" > test_dir/binary.bin
```

Run the indexer and observe which files get indexed.

**Deliverable:** List which files were indexed and which were skipped.

### Exercise 8.2: Symlink Attack

Try to index a symlink that escapes the project root. What happens?

**Deliverable:** Explain the security check.

### Exercise 8.3: Large File

Create a file larger than 1MB and try to index it. What happens?

**Deliverable:** Show the error message.

### Exercise 8.4: Binary File

Create a file with invalid UTF-8 bytes. What happens?

**Deliverable:** Show the error message.

### Exercise 8.5: Transaction Batching

Add logging to count how often transactions start/commit:

```rust
tracing::info!("Starting transaction at file {}", batch_count);
```

**Deliverable:** Show the log output for 1000 files.

---

**Next Chapter**: [Chapter 9: search.rs - Query Execution](09-search_rs.md)
