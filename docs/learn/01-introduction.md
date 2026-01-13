# Chapter 1: Introduction - What This Application Does

> "Simplicity is the ultimate sophistication." — Leonardo da Vinci

## 1.1 What Does This App Do? (In Simple Terms)

Imagine you have a massive library with thousands of books, and you want to find all books that mention "artificial intelligence." Without an index, you'd have to read every book cover to cover. With an index, you just flip to the back, look up "artificial intelligence," and instantly see which books mention it.

**This app is exactly that—an index for your code files.**

Specifically:

1. **It scans your project directory** — Walking through every file and folder, respecting `.gitignore`
2. **It reads each file's contents** — Extracting the text from source code, documentation, etc.
3. **It stores everything in a database** — Using SQLite with Full-Text Search 5 (FTS5)
4. **It lets you search instantly** — Finding files that contain your search terms
5. **It ranks results by relevance** — Using BM25 (the same algorithm used by search engines)

The magic is that after the initial indexing (which takes time), **searches are blazing fast**—typically under 10 milliseconds, even for large codebases.

### Why Is This Useful?

As a developer, you often need to search through codebases:

- **Find where a function is defined** — "Where is `calculateTotal()` defined?"
- **Find all uses of a variable** — "Where is `userId` used?"
- **Search for patterns** — "Find all error handling code"
- **Navigate unfamiliar codebases** — "What files deal with authentication?"

Tools like `grep` work but are slow on large codebases. This app pre-indexes everything so searches are instant.

---

## 1.2 The Library Analogy

Let's make this concrete with an analogy:

| Real World | This Application |
|------------|------------------|
| A library with thousands of books | Your project directory |
| A librarian reading every book | The indexer scanning files |
| Index cards in a filing cabinet | SQLite FTS5 database |
| "Find all books by Tolkien" | Running a search query |
| Index cards sorted by title/relevance | BM25-ranked search results |
| A card catalog system | The database schema |

### The Card Catalog Metaphor

Think about how a library's card catalog works:

1. **The cards exist separately from the books** — The index doesn't contain the book content, just references
2. **Cards are organized** — By author, title, subject
3. **Finding is fast** — You don't search books, you search cards
4. **Cards are updated** — When a new book arrives, a new card is added

The SQLite FTS5 database works exactly the same way:

- The `files` table stores the actual file content (like books on shelves)
- The `files_fts` virtual table stores the index (like cards in a catalog)
- Triggers keep them in sync (like librarians updating cards)

---

## 1.3 Why Does This App Exist?

### The Problem: Searching Code is Slow

When you have thousands of files, `grep` has to:

1. Open each file
2. Read each file's contents
3. Search for your pattern
4. Repeat for every file

This is O(n) where n = number of files. For a project with 10,000 files, that's 10,000 file opens.

### The Solution: Pre-Indexing

Instead of searching files, we search the index:

1. **One-time cost**: Index all files once (may take seconds or minutes)
2. **Zero cost per search**: Search the index (microseconds)
3. **Incremental updates**: Only reindex changed files

The index is like the difference between:

- **Linear search**: Reading every page of a book to find a word
- **Index lookup**: Flipping to the back-of-book index

### The Performance Difference

| Operation | grep (10,000 files) | ffts-grep (after indexing) |
|-----------|---------------------|---------------------------|
| First search | ~5-10 seconds | ~10 milliseconds |
| Second search | ~5-10 seconds | ~10 milliseconds |
| After file change | ~5-10 seconds | ~10 milliseconds |

The first search with ffts-grep is slow (it indexes first), but all subsequent searches are instant.

---

## 1.4 The Big Picture: Architecture Overview

Here's how all the pieces fit together:

```
┌─────────────────────────────────────────────────────────────────┐
│                        User Commands                             │
│    ffts-grep init | ffts-grep search "query" | ffts-grep doctor  │
└─────────────────────────────────────────────────────────────────┘
                                │
                                ▼
┌─────────────────────────────────────────────────────────────────┐
│                       main.rs (Entry Point)                      │
│              Parses CLI args, dispatches to handlers             │
└─────────────────────────────────────────────────────────────────┘
                                │
            ┌───────────────────┼───────────────────┐
            ▼                   ▼                   ▼
    ┌───────────────┐   ┌───────────────┐   ┌───────────────┐
    │   cli.rs      │   │   db.rs       │   │  indexer.rs   │
    │  Arguments    │   │  Database     │   │  File Walker  │
    └───────────────┘   └───────────────┘   └───────────────┘
            │                   │                   │
            └───────────────────┼───────────────────┘
                                ▼
    ┌─────────────────────────────────────────────────────────────┐
    │              .ffts-index.db (SQLite FTS5)                    │
    │   ┌──────────────┐  ┌──────────────┐  ┌──────────────┐     │
    │   │   files      │  │  files_fts   │  │  triggers    │     │
    │   │  (main tbl)  │  │  (virtual)   │  │  (auto-sync) │     │
    │   └──────────────┘  └──────────────┘  └──────────────┘     │
    └─────────────────────────────────────────────────────────────┘
```

---

## 1.5 The Data Flow: Step by Step

Let's trace what happens when you run `ffts-grep search "main"`:

```
Step 1: User types command
        │
        ▼
Step 2: main.rs parses arguments → "search" command with query "main"
        │
        ▼
Step 3: health.rs checks if database exists and is healthy
        │
        ▼ (if needed)
Step 4: auto_init() creates database and indexes files
        │
        ▼
Step 5: Searcher::search() sanitizes query and calls db.search()
        │
        ▼
Step 6: SQLite FTS5 MATCH query executes against indexed content
        │
        ▼
Step 7: Results ranked by BM25, formatted, and printed
        │
        ▼
Step 8: User sees paths like: src/main.rs, tests/main_test.rs
```

---

## 1.6 Key Technologies Used

### Rust Programming Language

Rust is a systems programming language that provides:

- **Memory safety without garbage collection** — No buffer overflows, use-after-free
- **Fearless concurrency** — Data races are prevented at compile time
- **High performance** — Comparable to C and C++
- **Modern type system** — Catch bugs at compile time

This project uses **Rust Edition 2024** (requires Rust 1.85+).

### SQLite with FTS5 Extension

SQLite is the most-deployed database in the world. FTS5 (Full-Text Search 5) is an extension that adds:

- **BM25 ranking** — Industry-standard relevance scoring
- **Natural language queries** — Searches work like Google
- **Fast indexing** — Handles millions of documents
- **Zero configuration** — Built into SQLite, no server needed

### clap (Command-Line Parser)

clap is Rust's most popular CLI argument parser:

- **Derive-based API** — Declarative argument definitions
- **Subcommands** — Support commands like `search`, `index`, `init`
- **Type validation** — Automatic conversion and validation
- **Help generation** — Beautiful auto-generated help text

### Other Key Dependencies

| Crate | Purpose | Why It Matters |
|-------|---------|----------------|
| `rusqlite` | SQLite bindings | Database operations |
| `ignore` | Gitignore parsing | Respect `.gitignore` files |
| `thiserror` | Error types | Structured error handling |
| `tracing` | Structured logging | Observability |
| `serde` | Serialization | JSON output support |

---

## 1.7 Project Structure

The Rust project is organized as follows:

```
rust-fts5-indexer/
├── Cargo.toml              ← Project configuration and dependencies
├── src/
│   ├── main.rs             ← Entry point (uses library)
│   ├── lib.rs              ← Library exports and constants (76 lines)
│   ├── cli.rs              ← Command-line argument parsing (653 lines)
│   ├── db.rs               ← SQLite FTS5 database layer (1430 lines)
│   ├── indexer.rs          ← Directory walking and indexing (919 lines)
│   ├── search.rs           ← Query execution and formatting (506 lines)
│   ├── doctor.rs           ← Diagnostic checks (842 lines)
│   ├── init.rs             ← Project initialization (418 lines)
│   ├── error.rs            ← Error types (179 lines)
│   ├── health.rs           ← Database health checks (972 lines)
│   └── constants.rs        ← Constants (16 lines)
└── tests/
    └── integration.rs      ← Full integration tests
```

---

## 1.8 How to Build and Run

### Build the Project

```bash
cd rust-fts5-indexer
cargo build              # Debug build (faster to compile)
cargo build --release    # Optimized release build (faster to run)
```

### Run the Indexer

```bash
# Initialize a project (creates database, indexes files)
./target/debug/ffts-grep init

# Search for files containing "main"
./target/debug/ffts-grep search main

# Force reindex all files
./target/debug/ffts-grep index --reindex

# Run diagnostics
./target/debug/ffts-grep doctor
```

### Run Tests

```bash
cargo test               # Run all tests
cargo test test_name     # Run specific test
cargo test --lib         # Run library tests only
```

---

## 1.9 Key Concepts Preview

Before diving into the code, let's preview some key concepts you'll master:

### WAL Mode (Write-Ahead Logging)

SQLite can use different journaling modes. WAL mode allows:

- **Concurrent reads** — Multiple processes can read while one writes
- **Better performance** — Reduces disk seeks
- **Atomic commits** — Transactions are atomic

Think of WAL like Git: you make changes in a separate "working copy" (WAL file), then commit them to the main file atomically.

### BM25 Ranking

BM25 is a ranking function used by search engines. It considers:

- **Term frequency** — How often does the term appear?
- **Inverse document frequency** — Is the term rare or common?
- **Field weighting** — Filename matches are most important, then path, then content

In this application:
- **Filename matches get 100x weight** (highest priority)
- **Path matches get 50x weight** (medium priority)
- **Content matches get 1x weight** (lowest priority)

So if you search "claude":
- `CLAUDE.md` → 100x boost → appears first
- `docs/claude-sdk/main.rs` → 50x boost → appears second
- `README.md` with "claude" 100 times → 1x → appears last

### Lazy Invalidation

The indexer is "lazy"—it only reindexes files that have changed:

- **Hash comparison** — Uses wyhash to detect content changes
- **mtime comparison** — Checks file modification time
- **Skip unchanged files** — Massive speedup on incremental indexing

### Atomic Operations

The tool uses atomic file operations to prevent corruption:

- **Temp file + rename** — Write to `.tmp`, then rename to final location
- **Cross-platform** — Different strategies for Windows vs Unix
- **No partial files** — Either the operation completes fully or nothing changes

---

## 1.10 What You'll Learn in This Tutorial

By the end of this tutorial, you will understand:

1. **How to structure a Rust CLI application** — Modular design with clear separation of concerns
2. **SQLite FTS5 full-text search** — How to implement fast search in your apps
3. **Professional error handling** — Structured errors with actionable messages
4. **Testing strategies** — Unit tests, integration tests, and property tests
5. **Performance optimization** — Lazy invalidation, WAL mode, indexing strategies
6. **Cross-platform development** — Handling Windows vs Unix differences

---

## 1.11 Chapter Summary

| Concept | What You Learned |
|---------|-----------------|
| Purpose | Fast full-text search for code files |
| Architecture | Modular CLI tool with SQLite FTS5 backend |
| Key Technologies | Rust, SQLite FTS5, clap, tracing |
| Performance | Sub-10ms queries after indexing |
| Data Flow | CLI → Parse → Search → Database → Results |

---

## Exercises

### Exercise 1.1: Explore the Project

1. Clone the repository (if you haven't already)
2. Navigate to `rust-fts5-indexer/`
3. List all files: `find src -name "*.rs"`
4. Build the project: `cargo build`
5. Run the help command: `./target/debug/ffts-grep --help`

**Deliverable:** List all source files and note which one seems most complex to you.

### Exercise 1.2: First Run

1. Create a test directory with some files
2. Initialize the indexer: `ffts-grep init`
3. Search for something: `ffts-grep search "your search term"`
4. Run the doctor: `ffts-grep doctor`

**Deliverable:** Write down what you observed. Did it work as expected?

### Exercise 1.3: Research Assignment

Research one of these topics and write a 1-paragraph summary:

- What is SQLite and why is it the most-deployed database?
- What is full-text search vs. regular SQL LIKE queries?
- What is the BM25 ranking algorithm?
- What is WAL mode in database systems?

**Deliverable:** Submit your summary before the next chapter.

### Exercise 1.4: Compare Search Tools

Time how long it takes to search for a term using:

1. `grep -r "term" .` (the slow way)
2. `ffts-grep search "term"` (the fast way)

Try it on different sized directories.

**Deliverable:** Record the times and explain why ffts-grep is faster after the first run.

---

**Next Chapter**: [Chapter 2: Core Concepts](02-core-concepts.md)
