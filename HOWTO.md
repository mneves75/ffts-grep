# ffts-grep Installation Guide

Fast full-text search file indexer for Claude Code using SQLite FTS5.

**Binary name**: `ffts-grep`
**Version**: 0.11

## About

This project was inspired by **[@leocooout](https://x.com/leocooout/status/2009337600742707335)**. The idea of using SQLite FTS5 for file indexing in Claude Code was originally proposed by Leo.

Thank you, **[@leocooout](https://x.com/leocooout)**, for the inspiration!

## Quick Install

### 1. Build the Binary

```bash
cd ffts-grep/rust-fts5-indexer

# Release build (recommended)
cargo build --release
```

### 2. Deploy to Claude Code

The easiest way is to use the deploy script:

```bash
./deploy_cc.sh
```

This will:
1. Build the release binary
2. Install to `~/.claude/ffts-grep`
3. Update `~/.claude/settings.json` with the binary path
4. Initialize the current project

### 3. Manual Installation

**Option A: Install to ~/.claude (recommended for Claude Code)**
```bash
mkdir -p ~/.claude
cp target/release/ffts-grep ~/.claude/
chmod +x ~/.claude/ffts-grep

# On macOS, re-sign the binary
codesign -s - --force ~/.claude/ffts-grep
```

**Option B: Install to ~/.local/bin**
```bash
mkdir -p ~/.local/bin
cp target/release/ffts-grep ~/.local/bin/
chmod +x ~/.local/bin/ffts-grep

# Add to PATH if not already there
echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.zshrc
source ~/.zshrc
```

### 4. Verify Installation

```bash
ffts-grep --version
# ffts-grep 0.11
```

## Configure Claude Code

### Edit `~/.claude/settings.json`

Add or modify the `fileSuggestion` section:

```json
{
  "fileSuggestion": {
    "command": "/Users/yourname/.claude/ffts-grep"
  }
}
```

## Usage

### Auto-Init (v0.9+)

The database automatically initializes on first search:

```bash
cd /path/to/your/project
ffts-grep search "main"
# Creates .ffts-index.db automatically if missing
```

### Manual Initialization

```bash
# Full initialization (gitignore + database + index)
ffts-grep init

# Gitignore only
ffts-grep init --gitignore-only

# Force reinitialization
ffts-grep init --force
```

### Indexing

```bash
# Incremental index (skips unchanged files)
ffts-grep index

# Force full reindex
ffts-grep index --reindex
```

### Searching

```bash
# Basic search
ffts-grep search "main function"

# Search paths only (no content)
ffts-grep search --paths "src/main"

# JSON output
ffts-grep search --format json "error"

# Disable auto-init (for CI/scripts)
ffts-grep search --no-auto-init "test"
```

### Diagnostics

```bash
# Health check
ffts-grep doctor

# Verbose output
ffts-grep doctor --verbose

# JSON output for automation
ffts-grep doctor --json
```

## Project-Specific Database

Each project gets its own `.ffts-index.db` file:

```
my-project/
├── .ffts-index.db       # SQLite FTS5 index (gitignored)
├── .ffts-index.db-wal   # WAL file (gitignored)
├── .ffts-index.db-shm   # Shared memory (gitignored)
├── src/
└── ...
```

The database files are automatically added to `.gitignore`.

## Comparison: Claude Code vs ffts-grep

| Feature | Claude Code Search | ffts-grep |
|---------|-------------------|-----------|
| First query | Variable | ~100-300ms (cold) |
| Subsequent queries | Variable | **< 10ms** (warm) |
| Content search | Limited | Full FTS5 |
| Relevance ranking | Basic | BM25 with filename boost |
| Auto-init | No | Yes |
| Schema migration | N/A | Automatic |
| Memory (idle) | N/A | ~5MB |

### Filename-Aware Ranking

Search results now prioritize files where the query matches the filename:

```bash
ffts-grep search "claude"
# Results (ranked by relevance):
# 1. CLAUDE.md          ← filename match (weight: 100)
# 2. src/claude/mod.rs  ← path match (weight: 50)
# 3. docs/tutorial.md   ← content match only (weight: 1)
```

The BM25 weights are: **filename (100) > path (50) > content (1)**

## Environment Variables

| Variable | Description |
|----------|-------------|
| `CLAUDE_PROJECT_DIR` | Override project directory |
| `RUST_LOG` | Control log verbosity (error, warn, info, debug) |

## Troubleshooting

### "Database belongs to different application"

A corrupt or foreign database exists in an ancestor directory. The tool now automatically skips these (v0.9+).

### "No results" when searching

```bash
ffts-grep index --reindex
```

### Slow indexing

The indexer skips:
- Binary files (non-UTF8)
- Files > 1MB
- Files matching `.gitignore` patterns
- `.git/` directories

## Uninstallation

```bash
# Remove binary
rm ~/.claude/ffts-grep

# Remove per-project databases
find ~/projects -name ".ffts-index.db*" -delete

# Remove from settings.json
# Set "fileSuggestion": null
```

## Development

```bash
cd rust-fts5-indexer

# Build
cargo build --release

# Test
cargo test

# Benchmark
cargo bench
```
