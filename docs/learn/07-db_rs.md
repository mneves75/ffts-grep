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

See `db.rs:15-62`:

```rust
/// SQLite PRAGMA settings for performance tuning.
#[derive(Debug, Clone)]
pub struct PragmaConfig {
    /// Journal mode: "WAL" for concurrent access
    pub journal_mode: String,

    /// Synchronous mode: OFF, NORMAL, FULL, EXTRA
    pub synchronous: String,

    /// Cache size in KB (negative = KB, positive = pages)
    pub cache_size: i64,

    /// Memory-mapped I/O size in bytes
    pub mmap_size: usize,

    /// Busy timeout in milliseconds
    pub busy_timeout_ms: u32,

    /// Page size in bytes
    pub page_size: u32,
}

impl Default for PragmaConfig {
    fn default() -> Self {
        Self {
            journal_mode: "WAL".to_string(),
            synchronous: "NORMAL".to_string(),
            cache_size: -32000, // 32 MB
            #[cfg(target_os = "macos")]
            mmap_size: 0, // macOS limitation
            #[cfg(not(target_os = "macos"))]
            mmap_size: 256 * 1024 * 1024, // 256 MB on Linux
            busy_timeout_ms: 5000,
            page_size: 4096,
        }
    }
}
```

### Platform-Specific Tuning

```rust
#[cfg(target_os = "macos")]
const DEFAULT_MMAP_SIZE: usize = 0;  // macOS has mmap limitations

#[cfg(not(target_os = "macos"))]
const DEFAULT_MMAP_SIZE: usize = 256 * 1024 * 1024;  // 256 MB on Linux
```

macOS has known issues with memory-mapped files, so we disable that optimization there.

---

## 7.3 Opening the Database

See `db.rs:106-133`:

```rust
impl Database {
    /// Open a database connection with the given configuration.
    ///
    /// # Errors
    /// Returns `IndexerError::Database` if SQLite connection fails.
    pub fn open(db_path: &Path, config: &PragmaConfig) -> Result<Self> {
        // Ensure parent directory exists
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| IndexerError::Io { source: e })?;
        }

        // Open connection withrusqlite::Connection::open(db_path)
            .map_err(|e| IndexerError::Database { source: e })?;

        // Configure connection
        Self::configure_connection(&conn, config)?;

        Ok(Self { conn })
    }

    /// Configure a connection with PRAGMA settings.
    fn configure_connection(conn: &rusqlite::Connection, config: &PragmaConfig) -> Result<()> {
        // Apply each PRAGMA
        conn.pragma_update(None, "journal_mode", &config.journal_mode)
            .map_err(|e| IndexerError::Database { source: e })?;

        conn.pragma_update(None, "synchronous", &config.synchronous)
            .map_err(|e| IndexerError::Database { source: e })?;

        conn.pragma_update(None, "cache_size", config.cache_size)
            .map_err(|e| IndexerError::Database { source: e })?;

        conn.pragma_update(None, "mmap_size", config.mmap_size as i64)
            .map_err(|e| IndexerError::Database { source: e })?;

        conn.pragma_update(None, "busy_timeout", config.busy_timeout_ms as i64)
            .map_err(|e| IndexerError::Database { source: e })?;

        conn.pragma_update(None, "page_size", config.page_size as i64)
            .map_err(|e| IndexerError::Database { source: e })?;

        Ok(())
    }
}
```

---

## 7.4 The Database Schema

See `db.rs:233-304`:

### The files Table (Main Storage)

```sql
CREATE TABLE files (
    id INTEGER PRIMARY KEY,
    path TEXT UNIQUE NOT NULL,
    filename TEXT,
    content_hash TEXT,
    mtime INTEGER,
    size INTEGER,
    indexed_at INTEGER,
    content TEXT
)
```

| Column | Type | Purpose |
|--------|------|---------|
| `id` | INTEGER PRIMARY KEY | Unique identifier |
| `path` | TEXT UNIQUE NOT NULL | Relative file path (e.g., "src/main.rs") |
| `filename` | TEXT | Just the file name (e.g., "main.rs") for filename boosting |
| `content_hash` | TEXT | wyhash for lazy invalidation |
| `mtime` | INTEGER | File modification time |
| `size` | INTEGER | File size in bytes |
| `indexed_at` | INTEGER | When file was indexed |
| `content` | TEXT | Full file content |

### The files_fts Table (FTS5 Virtual Table)

```sql
CREATE VIRTUAL TABLE files_fts USING fts5(
    filename, path, content,
    content='files',
    content_rowid='id',
    tokenize='porter unicode61',
    columnsize=0
)
```

| Parameter | Value | Purpose |
|-----------|-------|---------|
| `filename, path, content` | columns | Three columns for precise BM25 weighting |
| `content='files'` | external table | Data stored in `files` table |
| `content_rowid='id'` | link column | Links to `files.id` |
| `tokenize='porter unicode61'` | tokenizer | Stemming + unicode support |
| `columnsize=0` | optimization | Don't store column sizes (saves 10-15% space) |

### BM25 Weighting: Why Three Columns?

See `db.rs:386-400`:

```rust
// BM25 weights: filename=100, path=50, content=1
// This ranking ensures:
// - "CLAUDE.md" (filename match) ranks highest
// - "docs/MASTRA-VS-CLAUDE-SDK.md" (path match) ranks second
// - File with "claude" only in content ranks lowest
let mut stmt = self.conn().prepare(&format!(
    r#"
    SELECT path, bm25(files_fts, 100.0, 50.0, 1.0)
    FROM files_fts
    WHERE files_fts MATCH ?1
    ORDER BY bm25(files_fts, 100.0, 50.0, 1.0)
    LIMIT {limit}
    "#,
    query_clause = query_clause,
    limit = limit
))?;
```

| Weight | Column | Example: search "claude" |
|--------|--------|--------------------------|
| 100.0 | filename | `CLAUDE.md` (exact filename match) |
| 50.0 | path | `docs/MASTRA-VS-CLAUDE-SDK.md` (in directory) |
| 1.0 | content | `README.md` with "claude" in text |

### The Triggers (Auto-Sync)

```sql
-- After INSERT
CREATE TRIGGER files_ai AFTER INSERT ON files
BEGIN
    INSERT INTO files_fts(rowid, filename, path, content)
    VALUES (new.id, new.filename, new.path, new.content);
END;

-- After UPDATE
CREATE TRIGGER files_au AFTER UPDATE ON files
BEGIN
    INSERT INTO files_fts(files_fts, rowid, filename, path, content)
    VALUES('delete', old.id, old.filename, old.path, old.content);
    INSERT INTO files_fts(rowid, filename, path, content)
    VALUES (new.id, new.filename, new.path, new.content);
END;

-- After DELETE
CREATE TRIGGER files_ad AFTER DELETE ON files
BEGIN
    INSERT INTO files_fts(files_fts, rowid, filename, path, content)
    VALUES('delete', old.id, old.filename, old.path, old.content);
END;
```

---

## 7.5 The Application ID: Identifying Our Database

See `db.rs:118-124`:

```rust
// Set application ID for identification
// 0xA17E_6D42 = "FFTS" (FFTS grep)
conn.pragma_update(None, "application_id", 0xA17E_6D42_i32)
    .map_err(|e| IndexerError::Database { source: e })?;
```

The application ID (`0xA17E_6D42`) is a unique identifier for ffts-grep databases. This prevents accidentally using a different SQLite database as if it were ours.

---

## 7.6 Lazy Invalidation: The upsert_file Method

See `db.rs:228-249`:

```rust
/// Upsert a file into the database.
///
/// Uses lazy invalidation: only updates if content has changed.
/// The ON CONFLICT clause handles the case where the file already exists.
pub fn upsert_file(
    &self,
    path: &str,
    content: &str,
    mtime: i64,
    size: i64,
) -> Result<()> {
    // Calculate content hash for change detection
    let content_hash = calculate_hash(content);

    // Use wyhash for fast hashing (rust standard library)
    let hash_str = format!("{:x}", content_hash);

    // Extract filename from path for BM25 ranking
    let filename = Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(path);

    // Upsert with lazy invalidation
    // Only updates if content_hash has changed!
    self.conn().execute(
        r#"
        INSERT INTO files (path, filename, content_hash, mtime, size, indexed_at, content)
        VALUES (?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(path) DO UPDATE SET
            filename = excluded.filename,
            content_hash = excluded.content_hash,
            mtime = excluded.mtime,
            size = excluded.size,
            indexed_at = excluded.indexed_at,
            content = excluded.content
        WHERE excluded.content_hash != (SELECT content_hash FROM files WHERE path = excluded.path)
        "#,
        (path, filename, &hash_str, mtime, size, mtime, content),
    ).map_err(|e| IndexerError::Database { source: e })?;

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

---

## 7.7 Searching with BM25

See `db.rs:386-418`:

```rust
/// Search the FTS5 index.
///
/// Returns results ranked by BM25 with 3-column weighting:
/// - filename: 100.0 (exact name matches rank highest)
/// - path: 50.0 (directory path matches rank second)
/// - content: 1.0 (content matches rank lowest)
pub fn search(
    &self,
    query: &str,
    paths_only: bool,
    limit: u32,
) -> Result<Vec<SearchResult>> {
    // Build the query - if paths_only, search just the path column
    let query_clause = if paths_only {
        // Search only in path column (filename + path columns)
        "files_fts MATCH ?1"
    } else {
        // Search in all three columns
        "files_fts MATCH ?1"
    };

    // Execute search with 3-column BM25 ranking
    // bm25(fts, filename_weight, path_weight, content_weight)
    let mut stmt = self.conn().prepare(&format!(
        r#"
        SELECT path, bm25(files_fts, 100.0, 50.0, 1.0)
        FROM files_fts
        WHERE {query_clause}
        ORDER BY bm25(files_fts, 100.0, 50.0, 1.0)
        LIMIT {limit}
        "#,
        query_clause = query_clause,
        limit = limit
    )).map_err(|e| IndexerError::Database { source: e })?;

    let results = stmt
        .query_map([query], |row| {
            Ok(SearchResult {
                path: row.get(0)?,
                rank: row.get(1)?,
            })
        })
        .map_err(|e| IndexerError::Database { source: e })?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| IndexerError::Database { source: e })?;

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

---

## 7.8 Optimization: After Indexing

See `db.rs:343-366`:

```rust
/// Run PRAGMA optimize to update query planner statistics.
///
/// Should be called after bulk inserts/updates.
pub fn optimize(&self) -> Result<()> {
    self.conn()
        .pragma_update(None, "optimize", true)
        .map_err(|e| IndexerError::Database { source: e })?;

    Ok(())
}

/// Run FTS5 OPTIMIZE to defragment the FTS5 index.
///
/// Should be called after bulk inserts/updates when >10% of rows changed.
pub fn optimize_fts(&self) -> Result<()> {
    self.conn()
        .execute("INSERT INTO files_fts(files_fts) VALUES('optimize')", [])
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

See `db.rs:218-226`:

```rust
/// Check if the database schema is complete.
pub fn check_schema(&self) -> SchemaCheck {
    // Check for required tables, triggers, indexes
    let has_files = self.table_exists("files");
    let has_fts = self.table_exists("files_fts");
    let has_triggers = self.trigger_count() >= 3;
    let has_indexes = self.index_count() >= 3;

    SchemaCheck {
        table_count: if has_files { 1 } else { 0 },
        trigger_count: if has_triggers { 3 } else { 0 },
        index_count: if has_indexes { 3 } else { 0 },
        has_files_table: has_files,
        has_fts_table: has_fts,
        is_complete: has_files && has_fts && has_triggers && has_indexes,
    }
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
