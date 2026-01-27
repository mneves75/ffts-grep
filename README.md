# ffts-grep

[![Version](https://img.shields.io/badge/version-0.11.2-blue.svg)](https://github.com/mneves75/ffts-grep/releases)
[![License](https://img.shields.io/badge/license-Apache%202.0-green.svg)](LICENSE)
[![CI](https://github.com/mneves75/ffts-grep/actions/workflows/ci.yml/badge.svg)](https://github.com/mneves75/ffts-grep/actions/workflows/ci.yml)
[![Memory Validation](https://github.com/mneves75/ffts-grep/actions/workflows/memory-validation.yml/badge.svg)](https://github.com/mneves75/ffts-grep/actions/workflows/memory-validation.yml)
[![Rust](https://img.shields.io/badge/rust-1.85%2B-orange.svg)](https://www.rust-lang.org/)

Fast full-text search file indexer using SQLite FTS5.

A high-performance file indexer that provides ~10ms queries on 10K file codebases. Uses SQLite FTS5 with BM25 ranking for relevant search results.

## Features

- **SQLite FTS5** full-text search with BM25 ranking
- **Filename-aware ranking** - Files with query terms in filename rank higher (e.g., `CLAUDE.md` ranks above `docs/MASTRA-VS-CLAUDE-SDK.md` for "claude")
- **~10ms queries** on 10K file codebases (benchmarked)
- **Incremental updates** - Only reindexes modified files
- **Deletion detection** - Prunes entries for files removed from disk
- **Content search** - Search filenames, paths, and file contents
- **Single binary** - No external dependencies (bundled SQLite)
- **Git-aware filtering** - Respects root `.gitignore` and always ignores `.git/`
- **Configurable performance** - Tune SQLite PRAGMAs via CLI flags
- **Platform-aware** - Automatically adjusts for macOS limitations
- **Schema migration** - Automatic upgrade from older versions

## Installation

```bash
# Build from source
git clone https://github.com/mneves75/ffts-grep
cd ffts-grep/rust-fts5-indexer
cargo build --release

# Copy binary to PATH
cp target/release/ffts-grep ~/.local/bin/
chmod +x ~/.local/bin/ffts-grep
```

Or use the deploy script for Claude Code integration:

```bash
./deploy_cc.sh
```

## Toolchain and CI

- **MSRV**: Rust 1.85+ (Edition 2024)
- **Pinned dev toolchain**: `rust-toolchain.toml` targets Rust 1.92.0
- **CI**: Tests run on Linux/macOS/Windows for both the latest stable and MSRV
- **Scheduled**: Weekly memory validation (Linux/macOS) and monthly toolchain bump PRs

## Assumptions & Limits

- **Max file size**: 1MB default (via `IndexerConfig` in the library); the CLI uses this default to protect memory usage.
- **Timestamp/storage bounds**: File mtimes and sizes are stored as `i64`. Files with mtimes beyond year 2262 or sizes > `i64::MAX` are skipped with a warning.
- **Symlinks**: Not followed by default; use `--follow-symlinks` to opt in.
- **Deletion pruning**: Removed files disappear from results on the next index run.

## Contributing

See `CONTRIBUTING.md` for toolchain and verification requirements.

## Usage

### Commands

| Command | Description |
|---------|-------------|
| `ffts-grep init` | Initialize project (gitignore + database) |
| `ffts-grep index` | Index or reindex files |
| `ffts-grep search <query>` | Search indexed files |
| `ffts-grep doctor` | Run diagnostic checks |

### Global Options

| Option | Description |
|--------|-------------|
| `--quiet, -q` | Suppress status messages (for CI/scripting) |
| `--project-dir <path>` | Project root directory (default: current directory) |
| `--follow-symlinks` | Follow symlinks when indexing (default: disabled for safety) |
| `--refresh` | Refresh index before search (search-only) |
| `--help` | Show help information |
| `--version` | Show version information |

### Subcommand: init

Initialize a new project with database and gitignore configuration.

```bash
# Full initialization (gitignore + database + index)
ffts-grep init

# Gitignore only (skip database creation)
ffts-grep init --gitignore-only

# Force reinitialization
ffts-grep init --force
```

The `init` command:
1. Adds `.ffts-index.db*` entries to `.gitignore` (idempotent)
2. Creates the SQLite database with proper schema
3. Indexes all files in the project

### Subcommand: index

Index or reindex files in the project directory.

```bash
# Incremental index (skips unchanged files)
ffts-grep index

# Force full reindex (atomic replace)
ffts-grep index --reindex

# Include symlink targets (opt-in)
ffts-grep index --follow-symlinks
```

### Subcommand: search

Search indexed files using FTS5 queries.

```bash
# Basic search
ffts-grep search "main function"

# Search paths only (no content)
ffts-grep search --paths "src/main"

# JSON output format
ffts-grep search --format json "error handling"

# Run performance benchmark
ffts-grep search --benchmark "test query"

# Refresh index before search (after creating files)
ffts-grep search --refresh "refresh_token"
```

### Subcommand: doctor

Run diagnostic checks on installation health.

```bash
# Basic health check
ffts-grep doctor

# Verbose output with detailed diagnostics
ffts-grep doctor --verbose

# JSON output for CI/automation
ffts-grep doctor --json
```

The `doctor` command checks:
- Database exists and is readable
- Application ID is correct
- Schema is complete (tables, triggers, indexes)
- FTS5 integrity
- Journal mode (WAL recommended)
- File count
- Gitignore entries
- Binary availability
- Orphan WAL files

### Pragma Tuning Options

Fine-tune SQLite performance for your environment.

| Option | Default | Description |
|--------|---------|-------------|
| `--pragma-cache-size` | -32000 | Cache size in KB (negative) or pages (positive) |
| `--pragma-mmap-size` | Platform-specific | Memory-mapped I/O size (0 on macOS, 256MB on Linux) |
| `--pragma-page-size` | 4096 | Database page size (512-65536, power of 2) |
| `--pragma-busy-timeout` | 5000 | Busy timeout in milliseconds (0 = disabled) |
| `--pragma-synchronous` | NORMAL | Synchronous mode (OFF, NORMAL, FULL, EXTRA) |

Example with custom PRAGMAs:

```bash
# Large codebase optimization (128MB cache)
ffts-grep index --pragma-cache-size=-131072

# Maximum durability
ffts-grep search --pragma-synchronous=FULL

# High-concurrency environments
ffts-grep index --pragma-busy-timeout=10000
```

## Architecture

```
rust-fts5-indexer/src/
├── main.rs            # Entry point, CLI dispatch
├── lib.rs             # Library exports
├── cli.rs             # Argument parsing (clap subcommands)
├── db.rs              # SQLite FTS5 layer, PRAGMA config
├── indexer.rs         # Directory walker, batch upserts
├── search.rs          # Query execution, result formatting
├── doctor.rs          # Diagnostic checks (10 checks)
└── init.rs            # Gitignore updates, project init
```

### Database Schema

- **`files`** table: `path` (PK), `filename`, `content_hash`, `mtime`, `size`, `indexed_at`, `content`
- **`files_fts`** virtual table: FTS5 index on `filename`, `path`, `content` with BM25 weights (100:50:1)
- **Triggers**: Auto-sync FTS5 on INSERT/UPDATE/DELETE
- **Location**: `.ffts-index.db` in project root (WAL mode)
- **Migration**: Automatic upgrade from legacy 2‑column FTS5 schema to current 3‑column schema (with filename)

## Performance

Benchmarked on Apple M-series, 10K synthetic files. Run `cargo bench` for your system.

### Query Latency

| Scenario | Target | Measured |
|----------|--------|----------|
| Cold query (fresh process, 10K files) | < 50ms | ~10ms |
| Warm query (same process, cached) | < 15ms | ~9ms |

> **Note**: Cold and warm queries perform similarly because OS filesystem cache
> dominates SQLite's internal caching on modern SSDs.

### Memory Usage

| Metric | Target | Measured |
|--------|--------|----------|
| Peak during indexing (10K files) | < 50MB | ~16MB |
| Search-only (no indexing) | < 20MB | ~9MB |

To validate memory usage on your machine, run the ignored integration tests:

```bash
cd rust-fts5-indexer
cargo test --test memory_validation -- --ignored --nocapture
```

## Claude Code Integration

[Claude Code](https://claude.com/product/claude-code) is Anthropic's agentic coding tool that lives in your terminal. This indexer provides fast file suggestions when Claude Code needs to find relevant files in your codebase.

See the official documentation: [File Suggestion Settings](https://code.claude.com/docs/en/settings#file-suggestion-settings)

### Configuration

Add to `~/.claude/settings.json`:

```json
{
  "fileSuggestion": {
    "command": "/path/to/ffts-grep"
  }
}
```

### How It Works

When you type `@` in Claude Code to reference a file, Claude Code invokes your custom file suggestion command:

1. **Claude Code sends a query** via stdin as JSON: `{"query": "src/comp"}`
2. **ffts-grep searches** the FTS5 index for matching files
3. **Results returned** via stdout as newline-separated paths
4. **Claude Code displays** the suggestions for you to select

```
┌─────────────────┐      stdin: {"query": "..."}      ┌─────────────────┐
│   Claude Code   │ ──────────────────────────────▶  │   ffts-grep     │
│                 │                                   │                 │
│   @ file picker │ ◀──────────────────────────────  │   FTS5 search   │
└─────────────────┘      stdout: path\npath\n...      └─────────────────┘
```

Optional refresh (force reindex before searching):
```json
{"query": "src/comp", "refresh": true}
```

### Features

- **Stdin JSON protocol**: Receives `{"query": "..."}` from Claude Code
- **Stdout response**: Returns matching file paths (newline-separated)
- **Auto-init**: Database initializes automatically on first search
- **Project detection**: Uses `CLAUDE_PROJECT_DIR` or finds project root via `.git`

## Exit Codes

| Code | Description |
|------|-------------|
| 0 | Success |
| 1 | Warnings (non-fatal issues) |
| 2 | Errors (diagnostic failures) |

## License

Apache License 2.0 - See [LICENSE](LICENSE) for details.

---

Built for [Claude Code](https://claude.com/product/claude-code). Made with ❤️ by Claude Code. May be used by other tools.
