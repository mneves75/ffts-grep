# Chapter 2: Core Concepts - SQLite FTS5 and Database Fundamentals

> "Knowledge is a treasure, but practice is the key to it." — Thomas Fuller

This chapter covers the fundamental concepts you need to understand before diving into the code. We'll explain these concepts using the Feynman technique—simple explanations, analogies, and practical examples.

---

## 2.1 What is Full-Text Search?

### Simple Explanation

Regular database searches use exact matching. If you search for "main", you only find records containing exactly the word "main". Full-text search (FTS) is smarter—it understands:

- **Stemming**: "run", "running", "ran" are all the same root
- **Ranking**: Results are sorted by relevance, not just presence
- **Boolean operators**: "main AND function" finds both terms
- **Phrase searches**: "main function" finds the phrase, not separate words

### The Textbook Analogy

Imagine finding information in a textbook:

- **LIKE query** — Like searching for a word by reading every page
- **FTS query** — Like using the index at the back of the book

The index (FTS) tells you exactly which pages mention your term, ranked by importance.

### How FTS5 Works

SQLite FTS5 creates a virtual table that maintains an inverted index:

| Document | Terms |
|----------|-------|
| file1.rs | main, fn, let, println |
| file2.rs | main, struct, impl |
| file3.rs | config, load, parse |

Inverted index (what FTS5 actually stores):

| Term | Documents |
|------|-----------|
| main | file1.rs, file2.rs |
| fn | file1.rs |
| struct | file2.rs |
| config | file3.rs |

This allows instant lookups: "Which documents contain 'main'?" → Check index → Return file1.rs, file2.rs

---

## 2.2 The BM25 Ranking Algorithm

### Simple Explanation

BM25 is a relevance scoring algorithm used by search engines. It answers the question: "How relevant is this document to this search term?"

BM25 considers three factors:

1. **Term Frequency (TF)**: How often does the term appear?
   - More occurrences = more relevant (up to a point)

2. **Inverse Document Frequency (IDF)**: How rare is the term?
   - Rare terms are more valuable ("elephant" in 1 doc > "the" in 1000 docs)

3. **Field Length**: How long is the document?
   - A short document mentioning "main" is more relevant than a long one

### The Party Analogy

Think of BM25 like finding someone at a party:

- **Term Frequency**: How many times did they mention the topic?
- **IDF**: How unique is their knowledge?
- **Field Length**: Did they say it briefly or ramble for hours?

Someone who briefly says "I work on AI" is more relevant than someone who says "um, uh, you know, like, I was working on, uh, this thing, AI, you know" for 10 minutes.

### BM25 Formula (Simplified)

```
score = IDF × (TF × (k1 + 1)) / (TF + k1 × (1 - b + b × field_length / avg_field_length))
```

Where:
- `k1` = saturation parameter (usually 1.2-2.0)
- `b` = field length normalization (usually 0.75)
- `IDF` = log(1 + (N - n + 0.5) / (n + 0.5))

You don't need to memorize this—just understand the intuition!

### Path Boosting in This Application

See `db.rs:386-400`:

```sql
SELECT path, bm25(files_fts, 100.0, 50.0, 1.0) FROM files_fts
WHERE files_fts MATCH ?1 ORDER BY bm25(files_fts, 100.0, 50.0, 1.0) LIMIT ?2
```

The `bm25(files_fts, 100.0, 50.0, 1.0)` call gives:
- **100x weight** to filename matches (highest)
- **50x weight** to path matches (medium)
- **1x weight** to content matches (lowest)

So if you search "claude":
- File named `CLAUDE.md` gets 100x boost → appears first
- File at `docs/claude-sdk/main.rs` gets 50x boost → appears second
- File `README.md` containing "claude" 100 times → appears last

---

## 2.3 External Content Tables

### The Problem FTS5 Solves

FTS5 needs to store indexed data. But we already have the data in the `files` table. Do we duplicate it?

### External Content: The Solution

**External content tables** let FTS5 reference data in another table instead of storing it.

See `db.rs:260-269`:

```sql
CREATE VIRTUAL TABLE files_fts USING fts5(
    filename, path, content,
    content='files',           -- Data lives in 'files' table
    content_rowid='id',        -- Link via 'id' column
    tokenize='porter unicode61',
    columnsize=0
)
```

### The Library Card Metaphor

Think of it like a library:

| Component | Real Library | Database |
|-----------|--------------|----------|
| Books on shelves | Actual books | `files` table |
| Index cards | Cards with book info | `files_fts` table |
| Card references shelf | "See shelf 3, book 7" | `content_rowid='id'` |
| Librarian updates | Adds cards when books arrive | Triggers |

The cards (FTS5) don't contain the book content—they just point to where the book is. When you add a book, you update both the shelf (files table) and the cards (FTS5 table).

---

## 2.4 Triggers: Automatic Synchronization

### What Are Triggers?

Triggers are database commands that run automatically when data changes. See `db.rs:172-202`:

```sql
-- When you INSERT into files, automatically INSERT into files_fts
CREATE TRIGGER files_ai AFTER INSERT ON files
BEGIN
    INSERT INTO files_fts(rowid, path, content)
    VALUES (new.id, new.path, new.content);
END;
```

### The Synchronization Flow

```
User adds file
      │
      ▼
INSERT INTO files (path, content) VALUES ('main.rs', 'fn main() {}')
      │
      ▼
Trigger fires: INSERT INTO files_fts(rowid, path, content)
                                 VALUES (1, 'main.rs', 'fn main() {}')
      │
      ▼
Both tables are updated atomically!
```

### Why Triggers Are Powerful

1. **No application code needed** — Database handles sync automatically
2. **Consistent** — Even direct SQL access keeps tables in sync
3. **Fast** — Runs inside the database transaction

---

## 2.5 WAL Mode: Write-Ahead Logging

### The Problem: Concurrent Access

What happens when one process is writing to the database while another is reading?

- **Without WAL**: Reader might see partial/inconsistent data
- **With WAL**: Reader sees consistent snapshot from before writes

### WAL Explained Simply

Think of WAL like Git for your database:

| Git | WAL Mode |
|-----|----------|
| Work in working copy | Write to WAL file |
| Stage and commit | Checkpoint WAL to main |
| Anyone pulling sees consistent state | Anyone reading sees consistent snapshot |
| Atomic updates | Atomic commits |

### WAL Benefits

1. **Concurrent reads** — Readers never block writers
2. **Better performance** — No reader blocking, sequential writes
3. **Crash recovery** — If crash during write, WAL can be recovered

See `db.rs:70-79` for WAL configuration:

```rust
// Enable WAL mode for concurrent access
conn.pragma_update(None, "journal_mode", "WAL")?;
```

---

## 2.6 PRAGMA Settings: Database Configuration

### What Are PRAGMAs?

PRAGMAs are SQLite-specific commands that configure database behavior. See `db.rs:15-62` for `PragmaConfig`:

```rust
pub struct PragmaConfig {
    pub journal_mode: String,      // "WAL"
    pub synchronous: String,       // "NORMAL"
    pub cache_size: i64,           // -32000 = 32MB
    pub mmap_size: usize,          // Memory-mapped I/O
    pub busy_timeout_ms: u32,      // 5000ms
    pub page_size: u32,            // 4096 bytes
}
```

### Key PRAGMAs Explained

| PRAGMA | Value | Purpose |
|--------|-------|---------|
| `journal_mode` | WAL | Enable write-ahead logging |
| `synchronous` | NORMAL | Balance safety and speed |
| `cache_size` | -32000 | 32MB cache (negative = KB) |
| `mmap_size` | 0 or 256MB | Memory-mapped I/O (platform-specific) |
| `busy_timeout` | 5000 | Wait 5s when locked before error |
| `page_size` | 4096 | Database page size |

### Platform-Specific Optimization

See `db.rs:64-72`:

```rust
#[cfg(target_os = "macos")]
const DEFAULT_MMAP_SIZE: usize = 0;  // macOS limitation

#[cfg(not(target_os = "macos"))]
const DEFAULT_MMAP_SIZE: usize = 256 * 1024 * 1024;  // 256MB on Linux
```

macOS has limitations with memory-mapped files, so we disable it there.

---

## 2.7 Lazy Invalidation: Efficiency Pattern

### The Problem: Reindexing Everything is Slow

On incremental updates, do we reindex every file?

- **Naive approach**: Yes, reindex everything → O(n) each time
- **Lazy approach**: Only reindex changed files → O(changed files)

### Lazy Invalidation Explained

See `db.rs:233-249`:

```sql
INSERT INTO files(path, content_hash, mtime, size, indexed_at, content)
VALUES (?, ?, ?, ?, ?, ?)
ON CONFLICT(path) DO UPDATE SET
    content_hash = excluded.content_hash,
    mtime = excluded.mtime,
    size = excluded.size,
    indexed_at = excluded.indexed_at,
    content = excluded.content
WHERE excluded.content_hash != (SELECT content_hash FROM files WHERE path = excluded.path)
```

The key: `WHERE excluded.content_hash != current_hash`

If the hash matches, we skip the update entirely!

### The Stamp Collection Analogy

Imagine you collect stamps and want to update your catalog:

- **Naive**: Check every stamp's condition every time
- **Lazy**: Only update when you get a NEW stamp or an existing one changes

You check the "last modified" date (mtime) and the "fingerprint" (hash) to avoid unnecessary work.

---

## 2.8 Atomic Operations: Safety Pattern

### The Problem: Partial File Writes

What if you're writing a file and the power goes out?

- **Without atomic**: Corrupted file (half-written data)
- **With atomic**: Either complete or nothing changes

### The Solution: Temp File + Rename

See `init.rs:125-136`:

```rust
// Write to temp file
let mut file = fs::File::create(&tmp_path)?;
file.write_all(new_content.as_bytes())?;
file.flush()?;
drop(file);

// Atomic rename (either complete or file unchanged)
fs::rename(&tmp_path, &gitignore_path)?;
```

### Why This Works

1. **Write to temp** → If crash, temp file is garbage but real file is fine
2. **Atomic rename** → Operating system guarantees this completes fully

### The Diploma Analogy

Think of atomic operations like getting a diploma:

- **Non-atomic**: They overwrite your old diploma with the new one → If printer jams, you have nothing
- **Atomic**: They print a new diploma, then hand it to you → You always have either old or new

---

## 2.9 Chapter Summary

| Concept | Explanation | Analogy |
|---------|-------------|---------|
| Full-Text Search | Smart search with stemming, ranking | Textbook index |
| BM25 | Relevance scoring algorithm | Party conversation relevance |
| External Content | FTS5 references external table | Library cards point to books |
| Triggers | Auto-sync between tables | Librarian updates cards |
| WAL Mode | Concurrent read/write | Git-style commits |
| PRAGMA Settings | Database configuration | Performance tuning knobs |
| Lazy Invalidation | Only update changed data | Stamp catalog updates |
| Atomic Operations | All-or-nothing file writes | Diploma replacement |

---

## Exercises

### Exercise 2.1: Understanding Search Differences

Create a test with these files:
- `main.rs` containing "fn main()"
- `config.txt` containing "main configuration"
- `other.txt` containing "nothing relevant"

Search for "main" and observe the order. Why does `main.rs` appear first?

**Deliverable:** Explain the BM25 ranking in your own words.

### Exercise 2.2: Explore the Schema

Run these commands to see the actual database:

```bash
sqlite3 .ffts-index.db
> .schema
> SELECT * FROM files_fts;
> .quit
```

**Deliverable:** Draw a diagram showing how `files` and `files_fts` relate.

### Exercise 3.3: Understanding Triggers

Try directly inserting into the database:

```bash
sqlite3 .ffts-index.db
> INSERT INTO files(path, content) VALUES('test.txt', 'test content');
> SELECT * FROM files_fts;
> .quit
```

**Deliverable:** What happened? Explain why the FTS5 table was updated.

### Exercise 2.4: WAL Mode Experiment

Check the WAL mode:

```bash
sqlite3 .ffts-index.db
> PRAGMA journal_mode;
> .quit
```

**Deliverable:** What mode is it in? Why is this important?

---

**Next Chapter**: [Chapter 3: lib.rs - The Library Root](03-lib_rs.md)
