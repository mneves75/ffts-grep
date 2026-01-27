# FOR_YOU_KNOW_MARCUS.md

> "The difference between a junior and senior engineer isn't what they know—it's how they think about problems."

## The Story: Why This Project Exists

Imagine you're working in Claude Code on a large codebase. You press `@` to reference a file and... wait. And wait. The default file search is doing a brute-force scan through thousands of files. On a 10K file codebase, this takes seconds. On every keystroke.

This project fixes that. It's a **full-text search index** that provides ~10ms queries on codebases of any size. The trick? Instead of searching files every time, we build an index once, then search the index.

Think of it like the difference between:
- **Linear search**: Looking through every book in a library to find one about cats
- **Indexed search**: Checking the library's catalog card system (or database)

---

## The Architecture: A Restaurant Kitchen Analogy

The codebase is organized like a professional restaurant kitchen:

```
rust-fts5-indexer/src/
├── main.rs      → The head chef (orchestrates everything)
├── cli.rs       → The host stand (takes orders from customers)
├── indexer.rs   → The prep cook (prepares ingredients ahead of time)
├── search.rs    → The line cook (executes orders quickly)
├── db.rs        → The walk-in freezer (organized storage)
├── health.rs    → The health inspector (checks everything is safe)
├── doctor.rs    → The kitchen consultant (diagnoses problems)
├── init.rs      → The restaurant opening crew (sets up the space)
├── error.rs     → The fire extinguisher (handles emergencies)
├── constants.rs → The recipe book (fixed values everyone uses)
└── fs_utils.rs  → The dishwasher (handles the dirty work nobody sees)
```

### Data Flow: How an Order Gets Fulfilled

**Indexing (Prep Work)**:
```
Files on disk → indexer.rs (reads them) → db.rs (stores them) → SQLite FTS5 (indexes them)
```

**Searching (Order Execution)**:
```
User query → cli.rs (parses it) → search.rs (finds matches) → main.rs (formats output)
```

The key insight: **All the slow work happens at prep time (indexing), so order execution (search) is lightning fast.**

---

## The Technology Stack: Why SQLite FTS5?

### The Decision

I could have used:
- **Elasticsearch**: Overkill for local file search, requires running a server
- **Tantivy** (Rust's Lucene): Powerful but adds 5MB+ to binary size
- **Custom inverted index**: Fun to build, nightmare to maintain
- **SQLite FTS5**: ✅ Bundled with rusqlite, battle-tested, tiny footprint

### Why FTS5 Specifically?

FTS5 is SQLite's "Full-Text Search version 5" extension. It gives you:

1. **BM25 ranking** out of the box (same algorithm Google uses)
2. **Prefix matching** (`MATCH 'clau*'` finds "claude", "clause", "claus")
3. **Phrase queries** (`MATCH '"hello world"'` matches exact phrase)
4. **Column weighting** (we weight filename:100, path:50, content:1)

The weighting is crucial: when you search "claude", you want `CLAUDE.md` to rank above `docs/some-file-mentioning-claude.md`.

### The BM25 Algorithm (Simplified)

BM25 scores documents based on:
- **Term frequency**: How many times does the search term appear?
- **Inverse document frequency**: How rare is this term across all documents?
- **Document length normalization**: Shorter documents get a boost (a 10-line file mentioning "auth" once is more relevant than a 10,000-line file mentioning it once)

You don't need to understand the math—just know that FTS5 handles it automatically.

---

## The Database Schema: Designing for Speed

```sql
-- The main table (stores file content)
CREATE TABLE files (
    path TEXT PRIMARY KEY,      -- "src/main.rs"
    filename TEXT NOT NULL,     -- "main.rs" (for ranking boost)
    content_hash INTEGER,       -- Wyhash of content (fast skip check)
    mtime INTEGER,              -- Last modified time
    size INTEGER,
    indexed_at INTEGER,
    content TEXT                -- The actual file content
);

-- The FTS5 virtual table (the magic)
CREATE VIRTUAL TABLE files_fts USING fts5(
    filename,
    path,
    content,
    content='files',            -- Sync with 'files' table
    content_rowid='rowid',
    tokenize='unicode61'
);

-- Auto-sync triggers (keep FTS in sync with main table)
CREATE TRIGGER files_ai AFTER INSERT ON files BEGIN
    INSERT INTO files_fts(rowid, filename, path, content)
    VALUES (new.rowid, new.filename, new.path, new.content);
END;
```

### Why Triggers Instead of Manual Sync?

Early versions manually updated the FTS table after each insert. This was:
1. Error-prone (forget to sync = stale index)
2. Slower (two round-trips to SQLite)
3. Not atomic (crash between writes = corrupted state)

Triggers fix all three: SQLite guarantees they run atomically with the main operation.

---

## Key Engineering Patterns

### 1. Content Hashing for Incremental Updates

**The Problem**: Reindexing 10K files takes 2+ seconds. But most files haven't changed.

**The Solution**: Store a hash of each file's content. On reindex:
```rust
// Pseudo-code
if file.mtime == stored.mtime && hash(file.content) == stored.hash {
    skip();  // File unchanged
} else {
    reindex(file);
}
```

We use **Wyhash** instead of SHA-256. Why?
- SHA-256: Cryptographically secure, ~300 MB/s
- Wyhash: Not secure, but ~10 GB/s

We don't need security—we just need to detect changes. Wyhash is 30x faster.

### 2. Atomic Reindex Pattern

**The Problem**: If reindexing crashes halfway, you have a corrupted database.

**The Solution**: Build the new index in a temp file, then atomically rename.

```rust
// Simplified from indexer.rs
fn atomic_reindex(root: &Path) -> Result<()> {
    let tmp_path = root.join(".ffts-index.db.tmp");
    let final_path = root.join(".ffts-index.db");

    // Build index in temp location
    let db = Database::open(&tmp_path)?;
    index_all_files(&db, root)?;

    // WAL checkpoint (flush all data to main file)
    db.checkpoint()?;
    drop(db);

    // Atomic rename (POSIX guarantees this is atomic)
    fs::rename(tmp_path, final_path)?;
}
```

This pattern is used everywhere in production systems. If the process crashes at any point, you either have the old database (rename didn't happen) or the new database (rename completed). Never a corrupt hybrid.

### 3. Platform-Aware PRAGMA Configuration

**The Discovery**: macOS and Linux handle memory-mapped files differently.

```rust
impl Default for PragmaConfig {
    fn default() -> Self {
        let mmap_size = if cfg!(target_os = "macos") {
            0  // macOS has mmap bugs with WAL mode
        } else {
            256 * 1024 * 1024  // 256MB on Linux
        };
        // ...
    }
}
```

This came from a real bug: macOS would intermittently fail with mmap enabled. The fix was platform detection.

---

## Lessons Earned (The Hard Way)

### Bug #1: The Stale Index Problem

**What happened**: Files created during a Claude Code session didn't appear in search results.

**Why**: The search path only auto-initializes when the database is *missing* or *empty*. If it exists and is "healthy", search proceeds with stale data.

**Root cause discovery**: Reading `health.rs:240-271` and `main.rs:358-377` revealed the logic:
```rust
match health {
    DatabaseHealth::Healthy => { /* NO REFRESH - uses stale index */ }
    DatabaseHealth::Missing | DatabaseHealth::Empty => { /* auto-init */ }
}
```

**The deeper insight**: The incremental indexer reads ALL file contents on every run (~100-500ms for 10K files). This is too slow for per-keystroke searches.

**Solution approach**: Explicit refresh protocol—add `{"refresh": true}` to stdin JSON so callers decide when to pay the latency cost.

### Bug #2: macOS Code Signing Death

**What happened**: After copying the binary to `~/.claude/`, macOS would SIGKILL it immediately.

**Why**: macOS Gatekeeper quarantines unsigned binaries. Copying breaks the ad-hoc signature.

**Fix**: Re-sign after copying:
```bash
if [[ "$(uname)" == "Darwin" ]]; then
    codesign -s - --force "$BINARY_PATH"
fi
```

### Bug #3: WAL Checkpoint Before Rename

**What happened**: Atomic reindex sometimes produced a 0-byte database.

**Why**: SQLite WAL mode writes to `.db-wal` file, not the main `.db` file. If you rename before checkpointing, you orphan the WAL file (which contains the actual data).

**Fix**: Always checkpoint before rename:
```rust
db.conn().query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))?;
drop(db);  // Close connection
fs::rename(tmp_path, final_path)?;
```

### Bug #4: Foreign Database Detection

**What happened**: If a user had another SQLite database named `.ffts-index.db` (from a different tool), we'd try to use it and fail with cryptic schema errors.

**Fix**: Use SQLite's `application_id` pragma as a magic number:
```rust
const APPLICATION_ID: u32 = 0x46465453; // "FFTS" in ASCII

// On open, verify it's our database
fn is_valid_ffts_database(path: &Path) -> bool {
    let db = Database::open_readonly(path)?;
    db.get_application_id() == Some(APPLICATION_ID)
}
```

---

## How Good Engineers Think

### 1. Design for the Common Case, Handle the Edge Cases

The common case: User searches, gets results in <10ms.
The edge case: Index is stale, user needs fresh results.

We optimize for the common case (no refresh by default) and provide explicit escape hatches for edge cases (`--refresh` flag).

### 2. Fail Fast, Fail Loud

When the database is corrupted, we don't try to "fix" it silently. We:
1. Log the corruption
2. Create a timestamped backup
3. Reinitialize from scratch

Users prefer "your index was rebuilt" over "search results are subtly wrong".

### 3. Make Invalid States Unrepresentable

The `DatabaseHealth` enum:
```rust
pub enum DatabaseHealth {
    Healthy,           // Good to search
    Empty,             // Schema exists, no files
    Missing,           // No database file
    Unreadable,        // Can't open (permissions?)
    WrongApplicationId,// Not our database
    SchemaInvalid,     // Corrupted schema
    Corrupted,         // FTS5 integrity check failed
}
```

Each variant has clear semantics. You can't accidentally confuse "missing" with "corrupted"—they're different types.

### 4. Measure Before Optimizing

Before adding complex caching:
```bash
cargo bench                          # Measure baseline
hyperfine 'ffts-grep search test'   # Real-world timing
```

The numbers told us: search is already 5-10ms. We don't need more optimization—we need correctness (the stale index bug).

---

## The Testing Philosophy

### P0: Correctness Tests (Must Never Fail)

```rust
#[test]
fn test_fts5_available() {
    // If this fails, FTS5 is not bundled correctly
    let conn = Connection::open_in_memory().unwrap();
    conn.execute("CREATE VIRTUAL TABLE t USING fts5(content)", []).unwrap();
}
```

### P1: Integration Tests (Real Workflows)

```rust
#[test]
fn test_prunes_missing_files() {
    // Create file, index, delete file, reindex
    // Verify deleted file is removed from index
}
```

### P2: Performance Tests (Regressions)

```rust
#[test]
fn test_health_check_fast_performance() {
    // 100 health checks should complete in <500ms
    for _ in 0..100 {
        check_health_fast(dir.path());
    }
    assert!(elapsed.as_millis() < 500);
}
```

---

## Session Continuity Notes

**2026-01-26**: Investigated stale index bug.
- Root cause: Search path doesn't refresh on `Healthy` state
- Deeper finding: Incremental index reads ALL files (~100-500ms)
- Decision: Add explicit `--refresh` flag, document trade-offs
- Plan file: `~/.claude/plans/bright-crafting-hare.md`

---

## Quick Reference for Future Sessions

| Task | File | Key Function |
|------|------|-------------|
| Change search ranking | `db.rs` | `search()` with BM25 weights |
| Modify incremental logic | `indexer.rs` | `process_entry()` |
| Add new CLI flag | `cli.rs` | Clap derive structs |
| Change auto-init behavior | `health.rs` | `auto_init()` |
| Add new diagnostic | `doctor.rs` | `run_checks()` |
| Platform-specific code | `fs_utils.rs` | `sync_file()`, `sync_parent_dir()` |

---

*"Good software is not just code that works. It's code that clearly expresses what it does, handles edge cases gracefully, and can be understood by the next person who reads it."*
