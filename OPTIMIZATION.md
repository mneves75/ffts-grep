# Optimization Guide for ffts-grep (Rust 1.85+ + SQLite FTS5)

This document captures *current* performance guidance for the project and the specific tradeoffs that apply to Rust 1.85+ and SQLite FTS5. It emphasizes measurement-first, safe-by-default tuning, and explicit tradeoffs when reducing durability or safety.

## Build Optimization

### Release Modes (Cargo build profiles)

```bash
# Debug (fastest compile, slowest runtime)
cargo build

# Release (standard release optimization)
cargo build --release

# Custom optimization levels
cargo build --release --profile lto  # Link-time optimization
```

**Guidance:** Prefer `cargo build --release` for production performance. The Cargo.toml already includes an optimized release profile (opt-level=3, lto=thin). Use debug builds during development for faster compile times.

### Linker and Distribution Notes

- This project uses **bundled SQLite** via the `rusqlite` crate with `features = ["bundled"]`. This guarantees FTS5 availability and works identically across all platforms without external dependencies.
- The release profile is configured in `Cargo.toml` with `opt-level = 3`, `lto = "thin"`, and `strip = true` for optimal binary size and performance.

## Code-Level Optimizations

### 1. Use `const` and `#[inline]` Only When Proven

```rust
// GOOD: Mark truly constant values as const
const MAX_FILE_SIZE: usize = 1024 * 1024;

// Only inline when profiling shows a benefit
#[inline]
fn fast_hash(content: &[u8]) -> u32 {
    // ...
}
```

### 2. Avoid Unnecessary Memory Allocations

```rust
// BAD: Multiple allocations in hot path
let path1 = path.to_owned();
let path2 = path1.clone();

// GOOD: Borrow and reuse references
fn process_path(path: &str) -> Result<()> {
    // Work with borrowed data
}
```

### 3. Prefer Value Types Over Boxed Types When Possible

```rust
// GOOD: Value types for small structs (stack allocation)
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub path: String,
    pub rank: f64,
}

// Consider Box only for large recursive types
```

### 4. Batch Operations (SQLite)

```rust
// GOOD: Batch SQLite operations in transactions
conn.execute("BEGIN TRANSACTION", [])?;
for file in files {
    db.upsert_file(&file.path, &file.content, file.mtime, file.size)?;
}
conn.execute("COMMIT", [])?;

// BAD: Individual operations (slow)
for file in files {
    // Each operation auto-commits - N times slower!
    db.upsert_file(&file.path, &file.content, file.mtime, file.size)?;
}
```

### 5. Use Specific Error Types (Not `anyhow` or `Box<dyn Error>`)

```rust
// GOOD: Specific errors = better optimization and clearer API
#[derive(Debug, thiserror::Error)]
pub enum IndexerError {
    #[error("Database error: {0}")]
    Database { source: rusqlite::Error },
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

// BAD: Generic error types lose type information and inhibit optimization
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
```

## SQLite FTS5 Optimizations

### 1. WAL Mode + Synchronous (Explicit Tradeoff)

```sql
PRAGMA journal_mode=WAL;     -- Better concurrency
PRAGMA synchronous=NORMAL;   -- Faster writes, less durable in WAL
```

`synchronous=NORMAL` in WAL mode trades durability for speed; committed transactions may roll back after power loss, but the database remains consistent. Choose `FULL` or `EXTRA` if durability is required.

### 2. Prepared Statements

```rust
// GOOD: Reuse prepared statements
let mut stmt = conn.prepare("SELECT path, rank FROM files_fts WHERE files_fts MATCH ?1")?;
let results = stmt.query_map([query], |row| {
    Ok(SearchResult {
        path: row.get(0)?,
        rank: row.get(1)?,
    })
})?;

// BAD: Parse query every time
conn.execute("SELECT ...", [])?;  // No parameterization
```

### 3. Batch Inserts

```sql
-- GOOD: Transaction + batch insert
BEGIN TRANSACTION;
INSERT INTO files VALUES (...);  -- 500 files
COMMIT;

-- BAD: Auto-commit per insert (slow)
INSERT;  -- Auto-commit
INSERT;  -- Auto-commit
```

### 4. PRAGMA optimize (Low-Cost Planner Tuning)

```sql
PRAGMA optimize;
```

SQLite recommends running `PRAGMA optimize` just before closing short-lived connections, and periodically for long-lived connections. This keeps query planner statistics fresh with minimal overhead.

### 5. FTS5 optimize / merge (Post-bulk indexing)

```sql
INSERT INTO files_fts(files_fts) VALUES('optimize');
```

The FTS5 `optimize` command merges internal b-trees to produce a smaller, faster index, but can be expensive on large datasets. Use it after large bulk indexing sessions or as a scheduled maintenance task.

## Memory Optimization

### 1. Streaming Reads

```rust
// GOOD: Stream large files with BufReader
use std::io::{BufReader, BufRead};

let file = File::open(path)?;
let reader = BufReader::new(file);
for line in reader.lines() {
    // Process line-by-line
}

// AVOID: Read entire file into memory when not needed
let content = fs::read_to_string(path)?;  // Allocates entire file
```

### 2. Use References Instead of Cloning

```rust
// GOOD: Borrow data where possible
fn process_files(files: &[File]) {
    for file in files {
        process_file(file);  // Pass by reference
    }
}

// BAD: Unnecessary cloning
fn process_files(files: &[File]) {
    for file in files {
        let file_clone = file.clone();
        process_file(&file_clone);  // Wasteful allocation
    }
}
```

### 3. Reuse Buffers for Temporary Data

```rust
// GOOD: Reuse buffers with `with_capacity`
let mut buffer = Vec::with_capacity(4096);
for item in items {
    buffer.clear();
    // Reuse buffer capacity
    serialize_to(&mut buffer, &item);
}
```

## CPU Optimization

### 1. Profile Before Optimizing

```bash
# Use Instruments on macOS
# Or: perf record on Linux

# Use system profilers (Instruments on macOS, `perf` on Linux)
# Cargo build profiles are selected via --profile or --release flag.
```

### 2. Hot Path Detection

The most expensive operations in this tool are:
1. **File reading** - Minimize with streaming
2. **SQLite queries** - Use prepared statements
3. **Hash computation** - Use fast non-cryptographic hashes

## Benchmarking

Run benchmarks to measure improvements:

```bash
# Build release
cd rust-fts5-indexer
cargo build --release

# Run benchmark
./target/release/cc-fts5-indexer --benchmark
```

Record results on your own hardware and track them over time. Avoid embedding absolute performance numbers in the doc unless they are produced and updated via CI.

## Common Pitfalls to Avoid

1. **Skipping measurement** - Optimize only after measuring where time is spent.
2. **Excessive cloning** - Clone operations can add significant overhead (measure before adding `.clone()`).
3. **Ignoring query planner stats** - Run `PRAGMA optimize` as recommended.
4. **Using WAL+NORMAL when durability is required** - Understand the durability tradeoff.
5. **Premature optimization** - Profile first, optimize hot paths only.

## Further Reading

- Rust Cargo profiles: https://doc.rust-lang.org/cargo/reference/profiles.html
- Rust Performance Book: https://nnethercote.github.io/perf-book/
- SQLite PRAGMA reference: https://www.sqlite.org/pragma.html
- SQLite FTS5 documentation: https://www.sqlite.org/fts5.html
