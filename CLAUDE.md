# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Fast full-text search file indexer using SQLite FTS5. Provides ~10ms queries on 10K file codebases. Uses BM25 ranking for relevant search results.

**Version**: 0.9 | **Rust**: 1.85+ (see `rust-fts5-indexer/Cargo.toml`)

## Build Commands

```bash
cd rust-fts5-indexer
cargo build                          # Development build
cargo build --release                 # Release build (LTO + strip + codegen-units=1)
cargo test                           # Run all tests
cargo test test_name                 # Run a single test
cargo test -- --nocapture            # Run tests with output visible
cargo bench                          # Run Criterion benchmarks (search, index, hash)
cargo run -- --help                   # Run locally
```

## Library Usage

This project can be used as a Rust library. Example:

```rust
use ffts_indexer::{Database, Indexer, IndexerConfig, PragmaConfig, DB_NAME};

let db = Database::open(Path::new(DB_NAME), PragmaConfig::default())?;
db.init_schema()?;

let mut indexer = Indexer::new(Path::new("."), db, IndexerConfig::default());
indexer.index_directory()?;
```

See `rust-fts5-indexer/src/lib.rs` for exported modules and types.

## Deployment

```bash
./deploy_cc.sh                       # Build, install to ~/.claude/, reindex
```

This installs to `~/.claude/ffts-grep` and can update Claude Code settings.

## Claude Code Integration

Add to `~/.claude/settings.json`:
```json
{
  "fileSuggestion": {
    "command": "/path/to/ffts-grep"
  }
}
```

The tool respects `CLAUDE_PROJECT_DIR` env var for project root detection.

## Architecture

```
rust-fts5-indexer/src/
├── main.rs            # Entry point, CLI dispatch
├── lib.rs             # Library exports, DB constants
├── cli.rs             # Argument parsing (clap subcommands)
├── db.rs              # SQLite FTS5 layer, triggers, BM25 ranking, PRAGMA config
├── indexer.rs         # Directory walker, UTF-8 validation, batch upserts, gitignore
├── search.rs          # Query execution, result formatting
├── doctor.rs          # Diagnostic checks (10 checks)
└── init.rs            # Gitignore updates, project initialization
```

### Data Flow

1. **Index mode**: `Indexer::index_directory()` → `Database::upsert_file()` → FTS5 triggers auto-sync
2. **Search mode**: `Searcher::search()` → `Database::search()` → BM25-ranked FTS5 MATCH query

### Database Schema

- **`files`** table: `path` (PK), `filename`, `content_hash` (Wyhash), `mtime`, `size`, `indexed_at`, `content`
- **`files_fts`** virtual table: FTS5 index on `filename`, `path`, `content` with BM25 weights (100:50:1)
- **Triggers**: `files_ai` (INSERT), `files_au` (UPDATE), `files_ad` (DELETE) auto-sync FTS5
- **Location**: `.ffts-index.db` in project root (WAL mode enabled)
- **Migration**: Automatic schema upgrade from v0.9/v0.10 to v0.11 via `migrate_schema()`

## Key Patterns

### Lazy Invalidation
Reindexing skips unchanged files using `content_hash` + `mtime` comparison via `ON CONFLICT DO UPDATE`.

### Atomic Reindex
`index --reindex` builds index in `.tmp` file, then atomically renames to avoid race conditions.

### Platform Detection
macOS: `mmap_size` set to 0 (platform limitation). Linux: Full 256MB mmap enabled.

### Git-aware Filtering
Respects root `.gitignore` and always ignores `.git/` directories.

### Filename-Aware Ranking
Search results prioritize files where the query matches the filename. BM25 weights: filename=100, path=50, content=1. This means `CLAUDE.md` ranks above `docs/MASTRA-VS-CLAUDE-SDK.md` when searching "claude".

## Important Technical Details

### SQLite FTS5 Bundled
Uses `rusqlite` with `bundled` feature to guarantee FTS5 availability. The test suite includes `test_fts5_available()` as a P0 correctness check.

### Rust Edition 2024
Project uses Rust Edition 2024 (requires Rust 1.85+). See `rust-fts5-indexer/Cargo.toml`.

### Release Profile
Heavily optimized release builds:
- LTO: "thin"
- Strip: true
- codegen-units: 1
- panic: "abort"

### macOS Code Signing
`deploy_cc.sh` automatically re-signs the binary on macOS after copying to prevent SIGKILL.

## CLI Commands Reference

| Command | Description |
|---------|-------------|
| `ffts-grep init` | Initialize project (gitignore + database) |
| `ffts-grep init --gitignore-only` | Only update gitignore |
| `ffts-grep init --force` | Force reinitialization |
| `ffts-grep index` | Incremental index (skips unchanged files) |
| `ffts-grep index --reindex` | Force full reindex (atomic replace) |
| `ffts-grep search <query>` | Search indexed files |
| `ffts-grep search --paths` | Search paths only (no content) |
| `ffts-grep search --format json` | JSON output format |
| `ffts-grep search --benchmark` | Run performance benchmark |
| `ffts-grep doctor` | Run diagnostic checks |
| `ffts-grep doctor --verbose` | Verbose output for diagnostics |
| `ffts-grep doctor --json` | JSON output for CI/automation |

### Global Options

| Option | Description |
|--------|-------------|
| `--quiet`, `-q` | Suppress status messages (for CI/scripting) |
| `--project-dir=<path>` | Override project directory |
| `--pragma-cache-size=<n>` | SQLite cache size in KB (default: -32000 = 32MB) |
| `--pragma-mmap-size=<n>` | Memory-mapped I/O size (platform-aware default) |
| `--pragma-page-size=<n>` | Database page size (default: 4096) |
| `--pragma-busy-timeout=<n>` | Concurrent access timeout in ms (default: 5000) |
| `--pragma-synchronous=<mode>` | Synchronous mode: OFF, NORMAL, FULL, EXTRA (default: NORMAL) |

## Development Guidelines

This project follows the [GUIDELINES-REF](https://github.com/mneves/GUIDELINES-REF) knowledge base.

**Relevant guidelines:**

| Guideline | Applies To |
|-----------|------------|
| `PRAGMATIC-RULES.md` | Daily defaults - READ FIRST before any task |
| `RUST-GUIDELINES.md` | Rust patterns, error handling, Edition 2024 |
| `SQLITE-GUIDELINES.md` | WAL mode, indexing, query patterns |
| `DB-GUIDELINES.md` | Schema design, soft deletes, transactions |
| `DEV-GUIDELINES.md` | Code quality, testing, type safety |

**Key conventions:**

- **Clarity > cleverness** - Write code as if John Carmack is reviewing it
- **Never hard delete** - Use soft deletes with `deleted_at` (if adding user data)
- **Audit everything** - Structured logging with context for debugging
- **Test edge cases** - Integration tests for FTS5, WAL mode, concurrent access

## Consuming GUIDELINES-REF

This project consumes [GUIDELINES-REF](https://github.com/mneves/GUIDELINES-REF) via git subtree:

```bash
# Update to latest GUIDELINES-REF
git subtree pull --prefix=docs/GUIDELINES-REF https://github.com/mneves/GUIDELINES-REF.git master --squash
```

After updating, sync to Claude Code settings across providers:
```bash
cd ~/.claude && ./sync-settings.sh
```
