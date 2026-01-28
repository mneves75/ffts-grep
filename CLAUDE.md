# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Fast full-text search file indexer using SQLite FTS5. Provides ~10ms queries on 10K file codebases. Uses BM25 ranking for relevant search results.

**Version**: 0.11.4 | **Rust**: 1.85+ (see `rust-fts5-indexer/Cargo.toml`)
**Session notes**: `agent-notes.md`

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
./deploy.sh                          # Build, install to ~/.local/bin (default), verify
./deploy_cc.sh                       # Build, install to ~/.claude/, update settings, reindex
```

`deploy.sh` installs to `~/.local/bin/ffts-grep` by default (override with `--install-dir`).
`deploy_cc.sh` installs to `~/.claude/ffts-grep` and can update Claude Code settings.
`deploy.sh` does not modify Claude Code settings.

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
├── main.rs            # Entry point, CLI dispatch, stdin JSON protocol
├── lib.rs             # Library exports, DB constants
├── cli.rs             # Argument parsing (clap subcommands)
├── db.rs              # SQLite FTS5 layer, triggers, BM25 ranking, PRAGMA config
├── indexer.rs         # Directory walker, UTF-8 validation, batch upserts, gitignore
├── search.rs          # Query execution, result formatting
├── doctor.rs          # Diagnostic checks (10 checks)
├── init.rs            # Gitignore updates, project initialization
├── error.rs           # Error types (IndexerError), exit codes
├── constants.rs       # Application ID, magic numbers
├── fs_utils.rs        # Platform-aware fsync, file operations
└── health.rs          # Auto-init, project root detection, database health checks
```

### Data Flow

1. **Index mode**: `Indexer::index_directory()` → `Database::upsert_file()` → FTS5 triggers auto-sync
2. **Search mode**: `Searcher::search()` → `Database::search()` → BM25-ranked FTS5 MATCH query

### Database Schema

- **`files`** table: `path` (PK), `filename`, `content_hash` (Wyhash), `mtime`, `size`, `indexed_at`, `content`
- **`files_fts`** virtual table: FTS5 index on `filename`, `path`, `content` with BM25 weights (100:50:1)
- **Triggers**: `files_ai` (INSERT), `files_au` (UPDATE), `files_ad` (DELETE) auto-sync FTS5
- **Location**: `.ffts-index.db` in project root (WAL mode enabled)
- **Migration**: Automatic schema upgrade from v0.9 to v0.11 via `migrate_schema()`

## Key Patterns

### Lazy Invalidation + Deletion Pruning
Reindexing skips unchanged files using `content_hash` + `mtime` comparison via `ON CONFLICT DO UPDATE`, and prunes deleted files by removing missing paths after indexing.

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
`deploy.sh` and `deploy_cc.sh` automatically re-sign the binary on macOS after copying to prevent SIGKILL.

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
| `--follow-symlinks` | Follow symbolic links during indexing (default: false) |
| `--refresh` | Refresh index before search (requires a non-empty query) |
| `--pragma-cache-size=<n>` | SQLite cache size in KB (default: -32000 = 32MB) |
| `--pragma-mmap-size=<n>` | Memory-mapped I/O size (platform-aware default) |
| `--pragma-page-size=<n>` | Database page size (default: 4096) |
| `--pragma-busy-timeout=<n>` | Concurrent access timeout in ms (default: 5000) |
| `--pragma-synchronous=<mode>` | Synchronous mode: OFF, NORMAL, FULL, EXTRA (default: NORMAL) |

## Bash Guidelines

**IMPORTANT: Avoid commands that cause output buffering issues**

- DO NOT pipe output through `head`, `tail`, `less`, or `more` when monitoring command output
- DO NOT use `| head -n X` or `| tail -n X` to truncate output - these cause buffering problems
- Let commands complete fully, or use command-specific flags if available
- For log monitoring, prefer reading files directly rather than piping through filters

**When checking command output:**
```bash
# GOOD: Use command-specific flags
git log -n 10                    # Not: git log | head -10
cargo test -- --test-threads=1  # Not: cargo test | head -50

# GOOD: Let commands complete
cargo build 2>&1                 # Full output, no pipes

# BAD: Causes buffering issues
cargo build | head -20           # May hang indefinitely
git log | tail -5                # Unpredictable behavior
```

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
