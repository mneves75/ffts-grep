# Chapter 14: Exercises and Solutions

> "The only way to learn a new programming language is by writing programs in it." — Dennis Ritchie

This chapter provides solutions and explanations for all exercises from the previous chapters. Use these as a reference after attempting the exercises yourself.

---

## Chapter 1: Introduction Exercises

### Exercise 1.1: Explore the Project Structure

**Question:** Explore the project structure and identify key directories and files.

**Solution:**

```
rust-fts5-indexer/
├── src/
│   ├── main.rs           # Entry point (657 lines)
│   ├── lib.rs            # Library exports (76 lines)
│   ├── cli.rs            # Argument parsing (653 lines)
│   ├── db.rs             # Database layer (1430 lines)
│   ├── indexer.rs        # Directory walking (919 lines)
│   ├── search.rs         # Query execution (506 lines)
│   ├── doctor.rs         # Diagnostics (842 lines)
│   ├── init.rs           # Initialization (418 lines)
│   ├── error.rs          # Error types (179 lines)
│   ├── health.rs         # Health checking (972 lines)
│   └── constants.rs      # Constants (16 lines)
├── tests/
│   └── integration.rs    # Integration tests
├── Cargo.toml            # Project manifest
└── docs/
    └── learn/            # This tutorial
```

---

### Exercise 1.2: Run the Application

**Question:** Run the application with `--help` and identify available commands.

**Solution:**

```bash
$ ffts-grep --help

Fast full-text search file indexer using SQLite FTS5

Usage: ffts-grep [OPTIONS] <COMMAND>

Commands:
  index    Index files in the project directory
  search   Search indexed files
  doctor   Run diagnostic checks
  init     Initialize project (gitignore + database)

Options:
  --project-dir <PATH>    Override project directory
  --quiet, -q             Suppress status messages
  --refresh               Refresh index before search (requires a non-empty query)
  --help                  Print help
  --version               Print version
```

---

### Exercise 1.3: Measure Search Speed

**Question:** Measure the search speed after indexing.

**Solution:**

```bash
# Initialize and index
ffts-grep init
ffts-grep index

# Search and measure (time command)
$ time ffts-grep search "fn "

# On a medium-sized project (1000 files), expect:
# real    0m0.015s  (~15ms)
# user    0m0.008s
# sys     0m0.005s

# This is sub-10ms as promised!
```

---

## Chapter 2: Core Concepts Exercises

### Exercise 2.1: Create a Simple FTS5 Table

**Question:** Create a simple FTS5 table and query it.

**Solution:**

```bash
sqlite3 test.db "
-- Create a simple FTS5 virtual table
CREATE VIRTUAL TABLE articles USING fts5(title, content);

-- Insert some data
INSERT INTO articles (title, content) VALUES
    ('Rust Programming', 'Rust is a systems programming language.'),
    ('SQLite FTS5', 'FTS5 is SQLite full-text search module.');

-- Search
SELECT title FROM articles WHERE articles MATCH 'Rust';
-- Output: Rust Programming
```

---

### Exercise 2.2: Understand BM25 Scoring

**Question:** Create files with different content and observe BM25 scores.

**Solution:**

```bash
# Create test files
mkdir -p /tmp/test_bm25
cd /tmp/test_bm25

echo "main function entry point" > main.rs
echo "main main main main main" > repeated.rs
echo "entry point function" > other.rs

ffts-grep init
ffts-grep index

# Search and check order (path matches should come first)
ffts-grep search "main"

# Expected order:
# 1. main.rs (path match + content)
# 2. repeated.rs (path doesn't match, but high frequency)
```

---

### Exercise 2.3: Explore WAL Mode

**Question:** Check if WAL mode is enabled on your database.

**Solution:**

```bash
ffts-grep init
ffts-grep index

sqlite3 .ffts-index.db "PRAGMA journal_mode;"
# Output: wal

# Check WAL files exist
ls -la .ffts-index.db*
# Output:
# .ffts-index.db
# .ffts-index.db-wal
# .ffts-index.db-shm
```

---

## Chapter 3: lib.rs Exercises

### Exercise 3.1: List Library Exports

**Question:** What types and modules are exported from the library?

**Solution:**

From `lib.rs:42-63`, the library exports:

```rust
// Modules
pub mod cli;
pub mod db;
pub mod doctor;
pub mod error;
pub mod health;
pub mod indexer;
pub mod init;
pub mod search;

// Types
pub use crate::db::{Database, Indexer, Searcher};
pub use crate::doctor::Doctor;
pub use crate::error::{ExitCode, IndexerError, Result};

// Constants
pub use crate::DB_NAME;
pub use crate::DB_SHM_NAME;
pub use crate::DB_WAL_NAME;
pub use crate::DB_TMP_NAME;
```

---

### Exercise 3.2: Use Library Programmatically

**Question:** Write a simple program using the library.

**Solution:**

```rust
// my_indexer.rs
use ffts_indexer::{Database, Indexer, IndexerConfig, PragmaConfig, DB_NAME};
use std::path::Path;

fn main() -> Result<(), ffts_indexer::error::IndexerError> {
    let root = Path::new(".");

    // Open/create database
    let db = Database::open(&root.join(DB_NAME), &PragmaConfig::default())?;
    db.init_schema()?;

    // Create indexer
    let mut indexer = Indexer::new(root, db, IndexerConfig::default());

    // Index files
    let stats = indexer.index_directory()?;

    println!("Indexed {} files ({} bytes) in {:?}",
        stats.files_indexed,
        stats.bytes_indexed,
        stats.duration);

    Ok(())
}
```

---

## Chapter 4: error.rs Exercises

### Exercise 4.1: Trigger Different Errors

**Question:** Trigger at least 3 different error types.

**Solution:**

```bash
# Error 1: Missing database (IoErr = 3)
rm .ffts-index.db
ffts-grep search "main"
# Exit code: 3

# Error 2: Invalid SQLite (DataErr = 2)
echo "not a database" > .ffts-index.db
ffts-grep search "main"
# Exit code: 2

# Error 3: Missing query (NoInput = 4)
ffts-grep search
# Output: error: required argument was not provided
# Exit code: 4
```

---

### Exercise 4.2: Design an Error Type

**Question:** Design an error type for a file processor.

**Solution:**

```rust
use thiserror::thiserror;

#[derive(Debug, thiserror::Error)]
pub enum FileProcessorError {
    #[error("File not found: {path}")]
    FileNotFound { path: String },

    #[error("Permission denied: {path}")]
    PermissionDenied { path: String },

    #[error("Invalid encoding: {path}")]
    InvalidEncoding { path: String },

    #[error("File too large: {path} ({size} bytes, max: {max} bytes)")]
    FileTooLarge { path: String, size: u64, max: u64 },

    #[error("IO error: {source}")]
    Io { #[from] source: std::io::Error },
}
```

---

## Chapter 5: cli.rs Exercises

### Exercise 5.1: Test Argument Validation

**Question:** Test the validation functions with invalid inputs.

**Solution:**

```bash
# Invalid cache size (must be negative KB or positive pages)
ffts-g --pragma-cacherep index-size 1000
# Error: cache_size must be negative (KB) or positive (pages)

# Invalid page size (must be power of 2)
ffts-grep index --pragma-page-size 1000
# Error: page_size must be power of 2 between 512 and 65536

# Invalid synchronous mode
ffts-grep index --pragma-synchronous INVALID
# Error: synchronous must be one of: OFF, NORMAL, FULL, EXTRA
```

---

### Exercise 5.2: Create a Simple CLI

**Question:** Create a simple CLI with clap.

**Solution:**

```rust
// Cargo.toml
// [dependencies]
// clap = { version = "4.4", features = ["derive"] }

// main.rs
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "myapp")]
#[command(author = "Me")]
#[command(version = "1.0")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Add { name: String },
    Remove { name: String },
    List,
}

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Commands::Add { name } => println!("Adding: {name}"),
        Commands::Remove { name } => println!("Removing: {name}"),
        Commands::List => println!("Listing all"),
    }
}
```

---

## Chapter 6: main.rs Exercises

### Exercise 6.1: Trace the Search Flow

**Question:** Trace through what happens when you run `ffts-grep search "main"`.

**Solution:**

1. `main()` (line 57) parses CLI arguments
2. `run_search()` (line 432) is called with query
3. Project root detection via `find_project_root()` (health.rs:187-220)
4. Health check via `check_health()` (health.rs:45-58)
5. If healthy: open database and search
6. If missing/empty: call `auto_init()` (health.rs:302-444)
7. Output results via `Searcher::format_results()` (search.rs:187-193)

---

### Exercise 6.2: Platform-Specific Code

**Question:** Compare Windows and Unix atomic replace implementations.

**Solution:**

From `main.rs:28-55`:

```rust
#[cfg(unix)]
fn atomic_replace(src: &Path, dst: &Path) -> std::io::Result<()> {
    // Unix: fs::rename is atomic on POSIX systems
    fs::rename(src, dst)
}

#[cfg(windows)]
fn atomic_replace(src: &Path, dst: &Path) -> std::io::Result<()> {
    // Windows: Use MoveFileExW with MOVEFILE_REPLACE_EXISTING
    use std::os::windows::fs::MetadataExt;
    use winapi::um::winbase::MoveFileExW;

    let src_wide = src.as_os_str().encode_wide().chain(Some(0)).collect::<Vec<_>>();
    let dst_wide = dst.as_os_str().encode_wide().chain(Some(0)).collect::<Vec<_>>();

    let result = unsafe {
        MoveFileExW(src_wide.as_ptr(), dst_wide.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH)
    };

    if result == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}
```

---

## Chapter 7: db.rs Exercises

### Exercise 7.1: Explore the Schema

**Question:** Explore the actual schema using sqlite3.

**Solution:**

```bash
sqlite3 .ffts-index.db ".schema"
# Output: Full CREATE TABLE, CREATE VIRTUAL TABLE, triggers, indexes

sqlite3 .ffts-index.db "SELECT * FROM files LIMIT 1;"
# Output: File record

sqlite3 .ffts-index.db "SELECT * FROM files_fts LIMIT 1;"
# Output: FTS5 record
```

---

### Exercise 7.2: Lazy Invalidation

**Question:** Prove lazy invalidation works.

**Solution:**

```bash
# Create and index a file
echo "version 1" > test.rs
ffts-grep index

# Get indexed_at timestamp
sqlite3 .ffts-index.db "SELECT indexed_at FROM files WHERE path = 'test.rs';"
# Output: 1700000000

# Wait and reindex without changing
sleep 2
ffts-grep index

# Check indexed_at again - should be same!
sqlite3 .ffts-index.db "SELECT indexed_at FROM files WHERE path = 'test.rs';"
# Output: 1700000000 (same - lazy invalidation worked)

# Now modify and reindex
echo "version 2" > test.rs
ffts-grep index

# indexed_at should now be different
sqlite3 .ffts-index.db "SELECT indexed_at FROM files WHERE path = 'test.rs';"
# Output: 1700000002 (updated)
```

---

## Chapter 8: indexer_rs Exercises

### Exercise 8.1: Explore File Walking

**Question:** Create test files and observe which get indexed.

**Solution:**

```bash
mkdir -p test_dir/src
echo "code" > test_dir/src/main.rs
echo "config" > test_dir/config.json
ln -s /etc/hostname test_dir/symlink  # Outside project
echo "binary" > test_dir/binary.bin

ffts-grep index --project-dir test_dir

# Expected:
# src/main.rs indexed
# config.json indexed
# symlink skipped (outside project)
# binary.bin skipped (not UTF-8)
```

---

### Exercise 8.2: Large File Test

**Question:** Test the 1MB size limit.

**Solution:**

```bash
# Create file larger than 1MB
dd if=/dev/zero of=large.rs bs=1024 count=1025  # 1025 KB

ffts-grep index

# Output should show:
# files_indexed: 0 (or whatever was in directory)
# files_skipped: 1 (large.rs)
```

---

## Chapter 9: search_rs Exercises

### Exercise 9.1: Test Sanitization

**Question:** Test query sanitization with various inputs.

**Solution:**

| Query | Sanitized | Reason |
|-------|-----------|--------|
| `main` | `main` | Plain text, no change |
| `main -test` | `main test` | `-` replaced with space |
| `main "test"` | `main test` | `"` removed |
| `main*` | `main` | `*` removed |
| `main(test)` | `main test` | `()` removed |

---

### Exercise 9.2: JSON Output

**Question:** Compare plain and JSON output.

**Solution:**

```bash
# Plain output
$ ffts-grep search "main" --format plain
src/main.rs
examples/main.rs

# JSON output
$ ffts-grep search "main" --format json
{
  "results": [
    {
      "path": "src/main.rs",
      "rank": -1.2
    },
    {
      "path": "examples/main.rs",
      "rank": -0.8
    }
  ]
}
```

---

## Chapter 10: init_rs Exercises

### Exercise 10.1: Test Gitignore

**Question:** Test gitignore functionality.

**Solution:**

```bash
# Before init
$ cat .gitignore
# (empty or existing content)

$ ffts-grep init
# Gitignore updated with 4 entries

$ cat .gitignore
# # ======== ffts-grep ========
# .ffts-index.db
# .ffts-index.db-shm
# .ffts-index.db-wal
# .ffts-index.db.tmp*
```

---

### Exercise 10.2: Idempotency Test

**Question:** Run init multiple times.

**Solution:**

```bash
# First run
$ ffts-grep init
Initialized gitignore (Created 4 entries)

# Second run
$ ffts-grep init
Gitignore already complete (AlreadyComplete)

# Content should be identical
diff <(ffts-grep init 2>&1 | head -1) <(ffts-grep init 2>&1 | head -1)
# No difference
```

---

## Chapter 11: doctor_rs Exercises

### Exercise 11.1: Run Doctor

**Question:** Run doctor in all output formats.

**Solution:**

```bash
# Compact (default)
$ ffts-grep doctor
✓ Database: .ffts-index.db (1.2 MB)
✓ Schema: 2 tables, 3 triggers, 3 indexes
✓ Journal mode: WAL
✓ File count: 150 files indexed
✓ Gitignore: 4 entries present

# Verbose
$ ffts-grep doctor --verbose
[1/10] Database exists
       PASS ✓ Database: .ffts-index.db (1.2 MB)
       path: /project/.ffts-index.db
       size_bytes: 1258291

# JSON (for CI/CD)
$ ffts-grep doctor --json
{
  "version": "0.11.4",
  "project_dir": "/project",
  "checks": [...],
  "summary": { "pass": 10, "info": 0, "warn": 0, "fail": 0 },
  "exit_code": 0
}
```

---

### Exercise 11.2: Intentionally Break Something

**Question:** Break the database and run doctor.

**Solution:**

```bash
# Backup database
cp .ffts-index.db .ffts-index.db.backup

# Break it
echo "corrupt" > .ffts-index.db

# Run doctor
$ ffts-grep doctor
✗ Database exists
       Database not found: .ffts-index.db
       path: /project/.ffts-index.db
       remediation: Run: ffts-grep init

# Restore
mv .ffts-index.db.backup .ffts-index.db
```

---

## Chapter 12: health_rs Exercises

### Exercise 12.1: Test Health States

**Question:** Create databases in various health states.

**Solution:**

```bash
# Missing database
rm -f .ffts-index.db
ffts-grep doctor --json | jq '.summary'
# { "pass": 0, "warn": 0, "fail": 10 }

# Empty database (after init)
ffts-grep init --force
ffts-grep doctor --json | jq '.summary'
# { "pass": 7, "info": 0, "warn": 0, "fail": 0, "empty": 3 }
```

---

### Exercise 12.2: Project Root Detection

**Question:** Test project root detection in nested directories.

**Solution:**

```bash
# Structure:
# /tmp/project/
#   .git/
#   src/
#     code.rs

cd /tmp/project/src

# Should find /tmp/project (via .git)
ffts-grep doctor --project-dir .
# Output shows /tmp/project as project root
```

---

## Chapter 13: testing.md Exercises

### Exercise 13.1: Run Test Suite

**Question:** Run all tests and analyze results.

**Solution:**

```bash
$ cargo test

running 56 tests
test test_index_and_search_roundtrip ... ok
test test_index_skips_gitignored ... ok
test test_unicode_content ... ok
...
test result: ok. 56 passed; 0 failed

$ cargo test --lib

running 12 tests
...
test result: ok. 12 passed; 0 failed

$ cargo test --test integration

running 44 tests
...
test result: ok. 44 passed; 0 failed
```

---

### Exercise 13.2: Add a New Test

**Question:** Add a test for special character searching.

**Solution:**

```rust
// Add to tests/integration.rs

#[test]
fn test_search_with_special_characters() {
    let dir = tempdir().unwrap();
    let mut indexer = create_test_indexer(&dir);

    // Create file with special characters in content
    fs::write(dir.path().join("test.rs"), "fn add(a: i32, b: i32) -> i32 { a + b }").unwrap();

    let stats = indexer.index_directory().unwrap();
    assert_eq!(stats.files_indexed, 1);

    let db = indexer.db_mut();

    // Search for function name
    let results = db.search("add", false, 10).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].path, "test.rs");
}
```

---

## Bonus: Project Ideas

After completing this tutorial, extend your learning with these projects:

### Project 1: Simple File Indexer

Build a minimal version without FTS5:
- Use SQLite with LIKE queries
- Implement basic file walking
- Add simple search

### Project 2: HTTP API

Add a web interface:
- Use Actix-web or Rocket
- Expose search endpoint
- Add JSON output

### Project 3: Plugin System

Add extensibility:
- Define trait for indexers
- Support custom tokenizers
- Add configuration hooks

### Project 4: Benchmark Suite

Create performance benchmarks:
- Compare indexing speed
- Measure search latency
- Profile memory usage

---

## Final Review Questions

1. **What is FTS5 and why is it used in this project?**
   - FTS5 is SQLite's full-text search module. It enables fast searches across large amounts of text by building an inverted index.

2. **What is the difference between doctor.rs and health.rs?**
   - doctor.rs runs comprehensive diagnostics (10 checks, ~10-50ms). health.rs runs fast checks (<100μs) for auto-init decisions.

3. **How does lazy invalidation work?**
   - The upsert uses `ON CONFLICT DO UPDATE WHERE excluded.content_hash != current_hash`, only updating when content actually changes.

4. **What are the BM25 weights for filename, path, and content?**
   - Filename: 100x (exact name matches rank highest)
   - Path: 50x (directory path matches rank second)
   - Content: 1x (content matches rank lowest)

5. **What is the transaction batching strategy?**
   - Start transaction after 50 files, commit every 500 files. This balances memory usage with I/O overhead.

---

## Conclusion

Congratulations on completing the Rust FTS5 File Indexer tutorial! You now understand:

- **Full-text search** with SQLite FTS5 and BM25 ranking
- **Database design** with external content tables and triggers
- **File indexing** with gitignore awareness and UTF-8 validation
- **Error handling** with thiserror and structured exit codes
- **Testing strategies** for unit, integration, and performance tests
- **Health checking** for fast auto-init decisions

Apply these concepts to your own projects and continue learning!

---

## Appendix: Quick Reference

### Key Commands

```bash
# Setup
cargo build --release
./target/release/ffts-grep init

# Indexing
ffts-grep index
ffts-grep index --reindex

# Searching
ffts-grep search "query"
ffts-grep search "query" --paths
ffts-grep search "query" --format json
ffts-grep search "query" --refresh

# Diagnostics
ffts-grep doctor
ffts-grep doctor --verbose
ffts-grep doctor --json

# Help
ffts-grep --help
ffts-grep index --help
ffts-grep search --help
```

### Key Files

| File | Purpose | Lines |
|------|---------|-------|
| `src/lib.rs` | Library exports | 76 |
| `src/db.rs` | Database layer | 1430 |
| `src/indexer.rs` | File walking | 919 |
| `src/search.rs` | Query execution | 506 |
| `src/doctor.rs` | Diagnostics | 842 |
| `src/health.rs` | Health checking | 972 |
| `src/main.rs` | Entry point | 657 |
| `src/cli.rs` | CLI parsing | 653 |
| `src/init.rs` | Initialization | 418 |
| `src/error.rs` | Error types | 179 |
| `src/constants.rs` | Constants | 16 |

---

**End of Tutorial**
