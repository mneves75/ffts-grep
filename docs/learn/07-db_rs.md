# Chapter 7: db.rs - The Database Layer

> "The database is the heart of the system." — Unknown

## 7.1 What Does This File Do? (In Simple Terms)

The `db.rs` file is the **heart** of this application. It handles all interaction with SQLite FTS5—the database that stores your indexed files and enables fast searching.

### The Library Card Catalog Analogy

Think of a library's card catalog:

| Library Catalog | This Application |
|-----------------|------------------|
| Card file drawers | SQLite database file |
| Index cards | `files_fts` table (FTS5 virtual table) |
| Book records | `files` table (main table) |
| Cross-reference | Triggers (auto-sync) |
| Card search | FTS5 MATCH query |

The database stores:
- **What files exist** (paths)
- **What's in them** (content)
- **An optimized index** for fast searching (FTS5)

---

## 7.2 PragmaConfig: Database Configuration

Here is the actual PragmaConfig implementation from `db.rs:15-62`:

```rust
/// Database configuration for PRAGMA settings.
#[derive(Debug, Clone)]
pub struct PragmaConfig {
    pub journal_mode: String,
    pub synchronous: String,
    pub cache_size: i64,
    pub temp_store: String,
    pub mmap_size: i64,
    pub page_size: i64,
    pub busy_timeout_ms: i64,
}

impl Default for PragmaConfig {
    fn default() -> Self {
        Self {
            journal_mode: "WAL".to_string(),
            synchronous: "NORMAL".to_string(),
            cache_size: -32000, // -32000 KB = 32MB (better for 10K+ file codebases)
            temp_store: "MEMORY".to_string(),
            mmap_size: Self::default_mmap_size(),
            page_size: 4096,
            busy_timeout_ms: 5000,
        }
    }
}

impl PragmaConfig {
    /// Platform-aware `mmap_size` default.
    ///
    /// - **macOS**: Returns 0 (mmap unreliable on HFS+/APFS with `SQLite`)
    /// - **Linux/Other**: Returns 256MB (significant read performance boost)
    #[cfg(target_os = "macos")]
    #[must_use]
    pub const fn default_mmap_size() -> i64 {
        0
    }

    /// Platform-aware mmap_size default (Linux/Other: 256MB).
    #[cfg(not(target_os = "macos"))]
    #[must_use]
    pub fn default_mmap_size() -> i64 {
        256 * 1024 * 1024 // 256MB
    }
}
```

### Platform-Specific Tuning

The `default_mmap_size()` method uses `#[cfg(target_os = "macos")]` to provide platform-specific defaults:
- **macOS**: Returns 0 (mmap unreliable on HFS+/APFS with SQLite)
- **Linux/Other**: Returns 256MB (significant read performance boost)

---

## 7.3 Opening the Database

Here is the actual Database::open implementation from `db.rs:64-141`:

```rust
/// FTS5 file index database.
///
/// Uses external content table pattern where `files_fts` virtual table
/// automatically syncs with `files` table via triggers.
pub struct Database {
    conn: rusqlite::Connection,
}

impl Database {
    fn apply_pragma(conn: &rusqlite::Connection, name: &str, value: impl ToSql) -> Result<()> {
        conn.pragma_update(None, name, value).map_err(|e| IndexerError::Database { source: e })
    }

    /// Open database at path, creating if needed.
    ///
    /// # Errors
    /// Returns `IndexerError::Database` if:
    /// - The database file cannot be opened or created
    /// - Any PRAGMA setting fails to apply
    pub fn open(db_path: &Path, config: &PragmaConfig) -> Result<Self> {
        if config.busy_timeout_ms < 0 {
            return Err(IndexerError::ConfigInvalid {
                field: "busy_timeout_ms".to_string(),
                value: config.busy_timeout_ms.to_string(),
                reason: "must be >= 0".to_string(),
            });
        }

        let conn = rusqlite::Connection::open(db_path)?;

        // Apply PRAGMAs with error context
        Self::apply_pragma(&conn, "journal_mode", &config.journal_mode)?;
        Self::apply_pragma(&conn, "synchronous", &config.synchronous)?;
        Self::apply_pragma(&conn, "cache_size", config.cache_size)?;
        Self::apply_pragma(&conn, "temp_store", &config.temp_store)?;
        Self::apply_pragma(&conn, "mmap_size", config.mmap_size)?;
        Self::apply_pragma(&conn, "page_size", config.page_size)?;

        // SQLite-GUIDELINES.md: Required PRAGMAs for safety and correctness
        Self::apply_pragma(&conn, "foreign_keys", "ON")?;
        Self::apply_pragma(&conn, "trusted_schema", "OFF")?;
        // Application ID: 0xA17E_6D42 signature for cc-fts5-indexer
        // Stored as i32 to preserve the intended bit pattern in SQLite.
        Self::apply_pragma(&conn, "application_id", APPLICATION_ID_I32)?;

        #[allow(clippy::cast_sign_loss)]
        let busy_timeout = Duration::from_millis(config.busy_timeout_ms as u64);
        conn.busy_timeout(busy_timeout).map_err(|e| IndexerError::Database { source: e })?;

        Ok(Self { conn })
    }
}
```

---

## 7.4 The Database Schema

Here is the actual schema initialization from `db.rs:245-335`:

```rust
/// Initialize schema (idempotent - safe to call multiple times).
///
/// # Errors
/// Returns `IndexerError::Database` if any CREATE TABLE, CREATE TRIGGER, or CREATE INDEX statement fails.
pub fn init_schema(&self) -> Result<()> {
    // Main files table
    // The `filename` column stores just the file name (e.g., "CLAUDE.md" from "docs/CLAUDE.md")
    // This enables precise BM25 weighting where filename matches rank higher than path matches
    self.conn
        .execute(
            "CREATE TABLE IF NOT EXISTS files (
            id INTEGER PRIMARY KEY,
            path TEXT UNIQUE NOT NULL,
            filename TEXT,
            content_hash TEXT,
            mtime INTEGER,
            size INTEGER,
            indexed_at INTEGER,
            content TEXT
        )",
            [],
        )
        .map_err(|e| IndexerError::Database { source: e })?;

    // FTS5 virtual table with external content
    // Column order: filename, path, content (for BM25 weight arguments)
    // BM25 weights: filename=100, path=50, content=1
    // This ensures "CLAUDE.md" ranks higher than "docs/MASTRA-VS-CLAUDE-SDK.md" for query "claude"
    // columnsize=0: saves 10-15% storage (BM25 ranking still works)
    self.conn
        .execute(
            "CREATE VIRTUAL TABLE IF NOT EXISTS files_fts USING fts5(
            filename, path, content,
            content='files',
            content_rowid='id',
            tokenize='porter unicode61',
            columnsize=0
        )",
            [],
        )
        .map_err(|e| IndexerError::Database { source: e })?;

    // Auto-sync triggers (include filename for FTS5 indexing)
    self.conn
        .execute(
            "CREATE TRIGGER IF NOT EXISTS files_ai AFTER INSERT ON files BEGIN
            INSERT INTO files_fts(rowid, filename, path, content)
            VALUES (new.id, new.filename, new.path, new.content);
        END",
            [],
        )
        .map_err(|e| IndexerError::Database { source: e })?;

    self.conn
        .execute(
            "CREATE TRIGGER IF NOT EXISTS files_au AFTER UPDATE ON files BEGIN
            INSERT INTO files_fts(files_fts, rowid, filename, path, content)
            VALUES('delete', old.id, old.filename, old.path, old.content);
            INSERT INTO files_fts(rowid, filename, path, content)
            VALUES (new.id, new.filename, new.path, new.content);
        END",
            [],
        )
        .map_err(|e| IndexerError::Database { source: e })?;

    self.conn
        .execute(
            "CREATE TRIGGER IF NOT EXISTS files_ad AFTER DELETE ON files BEGIN
            INSERT INTO files_fts(files_fts, rowid, filename, path, content)
            VALUES('delete', old.id, old.filename, old.path, old.content);
        END",
            [],
        )
        .map_err(|e| IndexerError::Database { source: e })?;

    // Indexes for efficient queries
    self.conn
        .execute("CREATE INDEX IF NOT EXISTS idx_files_mtime ON files(mtime)", [])
        .map_err(|e| IndexerError::Database { source: e })?;

    self.conn
        .execute("CREATE INDEX IF NOT EXISTS idx_files_path ON files(path)", [])
        .map_err(|e| IndexerError::Database { source: e })?;

    // Index on content_hash for faster lazy invalidation subquery
    self.conn
        .execute("CREATE INDEX IF NOT EXISTS idx_files_hash ON files(content_hash)", [])
        .map_err(|e| IndexerError::Database { source: e })?;

    Ok(())
}
```

### Schema Tables

**files Table (Main Storage)**:
| Column | Type | Purpose |
|--------|------|---------|
| `id` | INTEGER PRIMARY KEY | Unique identifier |
| `path` | TEXT UNIQUE NOT NULL | Relative file path (e.g., "src/main.rs") |
| `filename` | TEXT | Just the file name for filename boosting |
| `content_hash` | TEXT | wyhash for lazy invalidation |
| `mtime` | INTEGER | File modification time |
| `size` | INTEGER | File size in bytes |
| `indexed_at` | INTEGER | When file was indexed |
| `content` | TEXT | Full file content |

**files_fts Table (FTS5 Virtual Table)**:
| Parameter | Value | Purpose |
|-----------|-------|---------|
| `filename, path, content` | columns | Three columns for precise BM25 weighting |
| `content='files'` | external table | Data stored in `files` table |
| `content_rowid='id'` | link column | Links to `files.id` |
| `tokenize='porter unicode61'` | tokenizer | Stemming + unicode support |
| `columnsize=0` | optimization | Saves 10-15% storage |

### BM25 Weighting

BM25 weights: **filename=100, path=50, content=1**

| Weight | Column | Example: search "claude" |
|--------|--------|--------------------------|
| 100.0 | filename | `CLAUDE.md` (exact filename match) |
| 50.0 | path | `docs/MASTRA-VS-CLAUDE-SDK.md` (in directory) |
| 1.0 | content | `README.md` with "claude" in text |

### The Triggers (Auto-Sync)

Three triggers automatically sync the `files` table with the `files_fts` virtual table:
- **files_ai**: After INSERT on files
- **files_au**: After UPDATE on files (delete old, insert new)
- **files_ad**: After DELETE on files

---

## 7.5 The Application ID: Identifying Our Database

The application ID is set during database opening at `db.rs:126-132`:

```rust
// Application ID: 0xA17E_6D42 signature for cc-fts5-indexer
// Stored as i32 to preserve the intended bit pattern in SQLite.
Self::apply_pragma(&conn, "application_id", APPLICATION_ID_I32)?;
```

The application ID (`0xA17E_6D42`) is a unique identifier for ffts-grep databases. This prevents accidentally using a different SQLite database as if it were ours.

---

## 7.6 Lazy Invalidation: The upsert_file Method

Here is the actual upsert_file implementation from `db.rs:337-373`:

```rust
/// Insert or update a file (lazy invalidation via `content_hash`).
///
/// Uses ON CONFLICT to handle both insert and update in one query.
/// Skips update if `content_hash` matches (same content, same mtime).
///
/// The `filename` is automatically extracted from `path` for FTS5 ranking.
/// This enables BM25 to weight filename matches higher than path matches.
///
/// # Errors
/// Returns `IndexerError::Database` if the INSERT or UPDATE query fails.
pub fn upsert_file(&self, path: &str, content: &str, mtime: i64, size: i64) -> Result<()> {
    let hash = wyhash(content.as_bytes());
    let now = Utc::now().timestamp();

    // Extract filename from path for FTS5 ranking boost
    // e.g., "docs/CLAUDE.md" -> "CLAUDE.md"
    let filename = Path::new(path).file_name().and_then(|n| n.to_str()).unwrap_or(path);

    // Lazy invalidation: only update if content changed
    // The ON CONFLICT handles the case where path exists
    self.conn.execute(
        "INSERT INTO files (path, filename, content_hash, mtime, size, indexed_at, content)
         VALUES (?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(path) DO UPDATE SET
             filename = excluded.filename,
             content_hash = excluded.content_hash,
             mtime = excluded.mtime,
             size = excluded.size,
             indexed_at = excluded.indexed_at,
             content = excluded.content
         WHERE excluded.content_hash != (SELECT content_hash FROM files WHERE path = excluded.path)",
        rusqlite::params![path, filename, hash, mtime, size, now, content],
    )
    .map_err(|e| IndexerError::Database { source: e })?;

    Ok(())
}
```

### The Magic of ON CONFLICT

```sql
INSERT INTO files ... VALUES (...)
ON CONFLICT(path) DO UPDATE SET ...
WHERE excluded.content_hash != (SELECT content_hash FROM files WHERE path = excluded.path)
```

This single SQL statement:
1. **Tries to INSERT** a new row
2. **If conflict** (path already exists), UPDATE instead
3. **But only if** the content hash has changed

If the file hasn't changed, the UPDATE doesn't happen, and triggers don't fire!

### Wyhash Implementation

Here is the actual wyhash function from `db.rs:799-817`:

```rust
/// Wyhash 64-bit hash implementation.
///
/// This matches Zig's std.hash.Wyhash exactly:
/// - 64-bit output as 16 hex characters
/// - Big-endian hex encoding
///
/// IMPORTANT: This must match Zig's output for lazy invalidation to work.
///
/// # Performance
/// This function is called for every file during indexing and is marked `#[inline]`
/// to enable cross-crate inlining for maximum performance on the hot path.
/// Uses `format!` macro which is heavily optimized by LLVM for hex conversion.
#[inline]
#[must_use]
pub fn wyhash(content: &[u8]) -> String {
    let hash = wyhash::wyhash(content, 0);
    // format! with :016x is LLVM-optimized and produces identical big-endian output
    format!("{hash:016x}")
}
```

---

## 7.7 Searching with BM25

Here is the actual search implementation from `db.rs:386-431`:

```rust
/// Search with BM25 ranking (filename weight: 100, path weight: 50, content weight: 1).
///
/// Returns results sorted by BM25 score (lower = better match).
/// - Filename matches get 100x weight (highest priority for exact file matches)
/// - Path matches get 50x weight (directory structure relevance)
/// - Content matches get 1x weight (tiebreaker)
///
/// This ensures "CLAUDE.md" ranks higher than "docs/MASTRA-VS-CLAUDE-SDK.md" for query "claude".
///
/// # Errors
/// Returns `IndexerError::Database` if:
/// - The FTS5 MATCH query fails (e.g., invalid FTS5 syntax)
/// - Query preparation or execution fails
pub fn search(&self, query: &str, paths_only: bool, limit: u32) -> Result<Vec<SearchResult>> {
    // Handle empty queries gracefully
    if query.trim().is_empty() {
        return Ok(vec![]);
    }

    // BM25 weights: filename=100, path=50, content=1
    // Column order in FTS5: filename, path, content
    let sql = if paths_only {
        "SELECT path, bm25(files_fts, 100.0, 50.0, 1.0) FROM files_fts
         WHERE path MATCH ?1 ORDER BY bm25(files_fts, 100.0, 50.0, 1.0) LIMIT ?2"
    } else {
        "SELECT path, bm25(files_fts, 100.0, 50.0, 1.0) FROM files_fts
         WHERE files_fts MATCH ?1 ORDER BY bm25(files_fts, 100.0, 50.0, 1.0) LIMIT ?2"
    };

    let mut stmt =
        self.conn.prepare_cached(sql).map_err(|e| IndexerError::Database { source: e })?;

    // Pre-allocate Vec to avoid reallocation in hot path
    let mut results = Vec::with_capacity(limit as usize);
    let rows = stmt
        .query_map(rusqlite::params![query, limit], |row| {
            Ok(SearchResult { path: row.get::<_, String>(0)?, rank: row.get::<_, f64>(1)? })
        })
        .map_err(|e| IndexerError::Database { source: e })?;

    for row in rows {
        results.push(row.map_err(|e| IndexerError::Database { source: e })?);
    }

    Ok(results)
}
```

### The BM25 Query Explained

```sql
SELECT path, bm25(files_fts, 100.0, 50.0, 1.0)
FROM files_fts
WHERE files_fts MATCH ?1
ORDER BY bm25(files_fts, 100.0, 50.0, 1.0)
LIMIT 15
```

| Part | Meaning |
|------|---------|
| `bm25(files_fts, 100.0, 50.0, 1.0)` | 3-column BM25: filename=100, path=50, content=1 |
| `WHERE files_fts MATCH ?1` | FTS5 full-text search across all columns |
| `ORDER BY bm25(...)` | Sort by relevance (lower = more relevant) |
| `LIMIT 15` | Return top 15 results |

### Why These Weights?

If you search "claude":
- A file named `CLAUDE.md` → **Filename** contains "claude" → **100x boost** (highest!)
- A file at `docs/claude-sdk/ref.md` → **Path** contains "claude" → **50x boost** (medium)
- A file `README.md` with "claude" in text → **Content** contains "claude" → **1x** (lowest)

This ensures exact filename matches appear first, path matches second, and content-only matches last.

### SearchResult Type

```rust
/// Search result returned by FTS5 queries.
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub path: String,
    pub rank: f64,
}
```

---

## 7.8 Optimization: After Indexing

Here are the actual optimization methods from `db.rs:528-554`:

```rust
/// Optimize FTS5 index by merging b-trees.
///
/// # Errors
/// Returns `IndexerError::Database` if the FTS5 optimize command fails.
pub fn optimize_fts(&self) -> Result<()> {
    // 'optimize' command merges FTS5 segment b-trees
    self.conn
        .execute("INSERT INTO files_fts(files_fts) VALUES('optimize')", [])
        .map_err(|e| IndexerError::Database { source: e })?;

    Ok(())
}

/// Optimize `SQLite` query planner statistics (2025+ best practice).
///
/// Should be called after bulk indexing operations to update statistics
/// that `SQLite` uses for query planning. This improves query performance
/// by ensuring the query planner has accurate table/index statistics.
///
/// # Errors
/// Returns `IndexerError::Database` if the PRAGMA optimize command fails.
pub fn optimize(&self) -> Result<()> {
    self.conn
        .execute("PRAGMA optimize", [])
        .map_err(|e| IndexerError::Database { source: e })?;
    Ok(())
}
```

### What These Do

| PRAGMA | Purpose | When to Run |
|--------|---------|-------------|
| `PRAGMA optimize` | Updates SQLite query planner statistics | After any bulk changes |
| `FTS5 optimize` | Defragments FTS5 index | After significant FTS changes |

---

## 7.9 Schema Validation

Here is the actual SchemaCheck implementation from `db.rs:596-796`:

```rust
/// Check if all required schema objects exist.
///
/// Returns a `SchemaCheck` struct with details about what's present/missing.
#[must_use]
pub fn check_schema(&self) -> SchemaCheck {
    let mut check = SchemaCheck::default();

    // Query sqlite_master for all expected objects
    let query = r"
        SELECT
            (SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='files') AS has_files_table,
            (SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='files_fts') AS has_fts_table,
            (SELECT COUNT(*) FROM sqlite_master WHERE type='trigger' AND name='files_ai') AS has_insert_trigger,
            (SELECT COUNT(*) FROM sqlite_master WHERE type='trigger' AND name='files_au') AS has_update_trigger,
            (SELECT COUNT(*) FROM sqlite_master WHERE type='trigger' AND name='files_ad') AS has_delete_trigger,
            (SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='idx_files_mtime') AS has_mtime_idx,
            (SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='idx_files_path') AS has_path_idx,
            (SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='idx_files_hash') AS has_hash_idx
    ";

    if let Ok(row) = self.conn.query_row(query, [], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, i64>(3)?,
            row.get::<_, i64>(4)?,
            row.get::<_, i64>(5)?,
            row.get::<_, i64>(6)?,
            row.get::<_, i64>(7)?,
        ))
    }) {
        check.has_files_table = row.0 > 0;
        check.has_fts_table = row.1 > 0;
        check.has_insert_trigger = row.2 > 0;
        check.has_update_trigger = row.3 > 0;
        check.has_delete_trigger = row.4 > 0;
        check.has_mtime_index = row.5 > 0;
        check.has_path_index = row.6 > 0;
        check.has_hash_index = row.7 > 0;
    }

    check
}
```

### SchemaCheck Struct

```rust
/// Result of schema completeness check.
///
/// Each boolean flag represents an independent schema component.
/// Using individual bools (vs. bitfield) provides clearer diagnostic output.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Default, Clone)]
pub struct SchemaCheck {
    pub has_files_table: bool,
    pub has_fts_table: bool,
    pub has_insert_trigger: bool,
    pub has_update_trigger: bool,
    pub has_delete_trigger: bool,
    pub has_mtime_index: bool,
    pub has_path_index: bool,
    pub has_hash_index: bool,
}
```

---

## 7.10 Chapter Summary

| Concept | What We Learned |
|---------|-----------------|
| PragmaConfig | SQLite tuning parameters |
| WAL mode | Concurrent read/write |
| Schema design | files + files_fts + triggers |
| External content | FTS5 references main table |
| Triggers | Auto-sync between tables |
| Lazy invalidation | Skip unchanged files |
| BM25 ranking | Path gets 50x weight |
| Application ID | Identify our databases |

---

## Exercises

### Exercise 7.1: Explore the Schema

Run these commands to see the actual schema:

```bash
sqlite3 .ffts-index.db
> .schema
> SELECT * FROM files LIMIT 1;
> SELECT * FROM files_fts LIMIT 1;
> .quit
```

**Deliverable:** Draw the relationship between `files` and `files_fts`.

### Exercise 7.2: BM25 Ranking

Create files with different content and search:

1. File named `main.rs` containing "hello"
2. File named `hello.rs` containing "main main main"

Search for "main". What order? Why?

**Deliverable:** Explain the 50x path weight effect.

### Exercise 7.3: Lazy Invalidation

Modify a file and run the indexer again. Use `sqlite3` to check the `indexed_at` timestamp.

**Deliverable:** Show that unchanged files are NOT re-indexed.

### Exercise 7.4: Design a Schema

Design a schema for a note-taking app with FTS5:
- Notes have title and content
- Search should find in both
- Support tags (many-to-many)

**Deliverable:** Write the CREATE TABLE and CREATE VIRTUAL TABLE statements.

### Exercise 7.5: Application ID

What would happen if you opened a regular SQLite database with this application?

**Deliverable:** Explain the purpose of the application ID check.

---

**Next Chapter**: [Chapter 8: indexer.rs - Directory Walking](08-indexer_rs.md)
