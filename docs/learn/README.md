# Rust FTS5 File Indexer - Complete Learning Tutorial

> "If you can't explain it simply, you don't understand it well enough."
> — Richard Feynman

Welcome to the **Rust FTS5 File Indexer** learning tutorial! This comprehensive guide will teach you how this high-performance full-text search tool works, one file at a time. Whether you're a junior developer looking to learn Rust, SQLite FTS5, or just want to understand how professional CLI tools are built, this tutorial is for you.

---

## What You Will Learn

This tutorial covers all the core technologies used in this project:

- **Rust Programming** — Modern systems programming with strong type safety and Rust 2024 Edition patterns
- **SQLite FTS5** — Full-text search built into the world's most deployed database
- **CLI Development** — Building professional command-line interfaces with clap derive API
- **Database Design** — Schema patterns, triggers, external content tables, and performance optimization
- **Error Handling** — Structured errors with actionable messages following thiserror patterns
- **Testing** — Comprehensive unit tests, integration tests, and property-based testing
- **Concurrency** — Safe concurrent operations with race condition handling

---

## The Feynman Technique Applied

Richard Feynman was famous for his ability to explain complex physics concepts in simple terms. He believed that if you couldn't explain something simply, you didn't truly understand it.

This tutorial applies his technique through five layers:

1. **Simple Explanation** — What does this code do in plain English?
2. **Analogies** — Connect to familiar, everyday concepts
3. **Key Concepts** — The important ideas you need to understand
4. **Code Walkthrough** — Line-by-line explanation with actual references
5. **Exercises** — Hands-on practice to reinforce your learning

---

## Table of Contents

| Chapter | Topic | File | Key Concepts |
|---------|-------|------|--------------|
| 01 | [Introduction](01-introduction.md) | — | What this app does, architecture overview |
| 02 | [Core Concepts](02-core-concepts.md) | — | SQLite FTS5, WAL mode, BM25, triggers |
| 03 | [lib.rs](03-lib_rs.md) | `lib.rs` | Library exports, module organization |
| 04 | [error.rs](04-error_rs.md) | `error.rs` | Error types, exit codes, thiserror |
| 05 | [cli.rs](05-cli_rs.md) | `cli.rs` | clap derive, argument validation |
| 06 | [main.rs](06-main_rs.md) | `main.rs` | CLI orchestration, platform-specific code |
| 07 | [db.rs](07-db_rs.md) | `db.rs` | Database schema, FTS5, triggers, BM25 |
| 08 | [indexer.rs](08-indexer_rs.md) | `indexer.rs` | Directory walking, gitignore, transactions |
| 09 | [search.rs](09-search_rs.md) | `search.rs` | Query sanitization, result formatting |
| 10 | [init.rs](10-init_rs.md) | `init.rs` | Gitignore management, atomic updates |
| 11 | [doctor.rs](11-doctor_rs.md) | `doctor.rs` | Diagnostic checks, severity levels |
| 12 | [health.rs](12-health_rs.md) | `health.rs` | Fast health checks, race condition handling |
| 13 | [Testing](13-testing.md) | All files | Unit tests, integration tests, patterns |
| 14 | [Exercises & Solutions](14-exercises-solutions.md) | — | Hands-on problems with complete answers |

---

## Quick Start

If you want to follow along with the actual code:

```bash
# Navigate to the Rust indexer
cd rust-fts5-indexer

# Build the project
cargo build

# Run the indexer to initialize
./target/debug/ffts-grep init

# Search for something
./target/debug/ffts-grep search "main"

# Run diagnostics
./target/debug/ffts-grep doctor

# Run tests
cargo test
```

---

## How to Use This Tutorial

### For Maximum Learning

1. **Read each chapter from start to finish** — Don't skip ahead
2. **Study the code references** — Each section links to actual implementation
3. **Try the exercises** — Practice makes permanent (don't just read them!)
4. **Experiment** — Modify the code and see what happens
5. **Review** — Before moving on, summarize what you learned in your own words

### Code Reference Format

Throughout this tutorial, you'll see references like:

- `[lib.rs:22]` — File `lib.rs`, line 22
- `[db.rs:140-166]` — File `db.rs`, lines 140 through 166
- `indexer.rs:101-105` — The `WalkBuilder` configuration

These reference the actual source files in `rust-fts5-indexer/src/`.

---

## The Big Picture: What Does This Application Do?

### In Simple Terms

Imagine you have a massive library with thousands of books, and you want to find all books that mention "artificial intelligence." Without an index, you'd have to read every book cover to cover. With an index, you just flip to the back, look up "artificial intelligence," and instantly see which books mention it.

**This app is exactly that—an index for your code files.**

Specifically:

1. **It scans your project directory** — Walking through every file and folder, respecting `.gitignore`
2. **It reads each file's contents** — Extracting text from source code, documentation, etc.
3. **It stores everything in a database** — Using SQLite with Full-Text Search 5 (FTS5)
4. **It lets you search instantly** — Finding files that contain your search terms
5. **It ranks results by relevance** — Using BM25 (the same algorithm used by search engines like Elasticsearch)

The magic is that after the initial indexing (which takes time), **searches are blazing fast**—typically under 10 milliseconds, even for large codebases.

### Real-World Analogy: Building a Library Index

| Real World | This Application |
|------------|------------------|
| A library with thousands of books | Your project directory |
| A librarian reading every book | The indexer scanning files |
| Index cards in a filing cabinet | SQLite FTS5 database |
| "Find all books by Tolkien" | Running a search query |
| Index cards sorted by relevance | BM25-ranked search results |

---

## Architecture Overview

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

### Data Flow: Search Request

```
1. User types: ffts-grep search "main"
                │
                ▼
2. main.rs parses → "search" command with query "main"
                │
                ▼
3. health.rs checks if database exists and is healthy
                │
                ▼ (if needed)
4. auto_init() creates database and indexes files
                │
                ▼
5. Searcher::search() sanitizes query
                │
                ▼
6. db.rs executes: FTS5 MATCH query with BM25 ranking
                │
                ▼
7. Results formatted (plain or JSON) and printed
                │
                ▼
8. User sees: src/main.rs, tests/main_test.rs
```

---

## Key Technologies

### Rust Programming Language (Edition 2024)

Rust is a systems programming language that provides:

- **Memory safety without garbage collection** — No buffer overflows, use-after-free bugs
- **Fearless concurrency** — Data races are prevented at compile time
- **High performance** — Comparable to C and C++
- **Modern type system** — Catch bugs at compile time
- **Rust 2024 Edition** — Uses the latest Rust features (requires 1.85+)

### SQLite with FTS5 Extension

SQLite is the most-deployed database in the world (it's in every smartphone, browser, and countless applications). FTS5 (Full-Text Search 5) adds:

- **BM25 ranking** — Industry-standard relevance scoring
- **Natural language queries** — Searches work like Google
- **Fast indexing** — Handles millions of documents
- **Zero configuration** — Built into SQLite, no server needed

### clap (Command-Line Parser)

clap is Rust's most popular CLI argument parser:

- **Derive-based API** — Declarative argument definitions
- **Subcommands** — Support commands like `search`, `index`, `doctor`
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
| `clap` | CLI parsing | Command-line interface |

---

## Key Concepts Preview

Before diving into the code, here are concepts you'll master:

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
- **Filename matches get 100x weight** (highest)
- **Path matches get 50x weight** (medium)
- **Content matches get 1x weight** (lowest)

Example: searching "claude" → `CLAUDE.md` first, `docs/claude/main.rs` second, `README.md` with "claude" in text last.

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

## Project Structure

```
rust-fts5-indexer/
├── Cargo.toml              ← Project configuration and dependencies
├── src/
│   ├── main.rs             ← Entry point (uses library)
│   ├── lib.rs              ← Library exports and constants (64 lines)
│   ├── cli.rs              ← Command-line argument parsing (628 lines)
│   ├── db.rs               ← SQLite FTS5 database layer (1008 lines)
│   ├── indexer.rs          ← Directory walking and indexing (607 lines)
│   ├── search.rs           ← Query execution and formatting (294 lines)
│   ├── doctor.rs           ← Diagnostic checks (845 lines)
│   ├── init.rs             ← Project initialization (388 lines)
│   ├── error.rs            ← Error types (180 lines)
│   └── health.rs           ← Database health checks (931 lines)
├── tests/
│   └── integration.rs      ← Full integration tests
└── benches/
    └── search_bench.rs     ← Criterion benchmarks
```

---

## Version Information

This is **version 0.11** using **Rust Edition 2024** (requires Rust 1.85+).

The semantic versioning scheme:
- **0.x.y** — Software is in initial development
- **Breaking changes may occur** — APIs aren't stable yet
- **1.0.0** — Will mark stable API

---

## John Carmack's Review Standard

This code was written with John Carmack's standards in mind:

> "The quality of the code should be such that any competent programmer can read and understand it, make modifications, and be confident they haven't broken anything."

As you learn, ask yourself: "Would Carmack approve of this code?" Clarity, correctness, and performance matter.

---

## Ready to Begin?

Head to **[Chapter 1: Introduction](01-introduction.md)** to understand what this application does, then continue through the chapters in order.

Remember: the goal isn't just to read code—it's to understand it deeply enough that you could write it yourself. Take your time, do the exercises, and don't move on until each concept clicks.

---

*Tutorial created using the Feynman Technique for the Rust FTS5 File Indexer project. Version 0.11.*
