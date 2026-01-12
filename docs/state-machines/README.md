# State Machine Diagrams

Visual documentation of the ffts-grep (Rust FTS5 Indexer) component state machines using Mermaid diagrams.

**Generated**: 2026-01-11
**Source Version**: 0.9 (see `Cargo.toml`)

## Diagrams

| File | Component | Description |
|------|-----------|-------------|
| [01-cli-dispatch.md](01-cli-dispatch.md) | CLI Entry | Command parsing, stdin JSON, exit codes |
| [02-indexer-lifecycle.md](02-indexer-lifecycle.md) | Indexer | **Conditional transaction strategy**, batch reset logic |
| [03-database-states.md](03-database-states.md) | Database | PRAGMA config, FTS5 triggers, lazy invalidation |
| [04-search-flow.md](04-search-flow.md) | Search | Health-gated auto-init, BM25 ranking |
| [05-doctor-diagnostics.md](05-doctor-diagnostics.md) | Doctor | 10-check diagnostic pipeline |
| [06-init-flow.md](06-init-flow.md) | Init | Gitignore atomic updates, force reinit |
| [07-error-types.md](07-error-types.md) | Errors | IndexerError variants, recovery patterns |

## Viewing Diagrams

These diagrams use [Mermaid](https://mermaid.js.org/) syntax:

- **GitHub**: Renders automatically in markdown preview
- **VS Code**: Install "Markdown Preview Mermaid Support" extension
- **Obsidian**: Built-in Mermaid support
- **Online**: Paste into [mermaid.live](https://mermaid.live)

## Key Architecture Patterns

### 1. Conditional Transaction Strategy
```
THRESHOLD = 50 files   → Start transaction after this many
BATCH_SIZE = 500 files → Commit every 500 files
RESET = 50 (not 0!)    → After commit, reset to threshold
```

**Why?** Transaction overhead dominates for small operations (<50 files).

### 2. Lazy Invalidation
```sql
ON CONFLICT(path) DO UPDATE SET ...
WHERE excluded.content_hash !=
      (SELECT content_hash FROM files WHERE path = excluded.path)
```
Skip FTS5 rebuild if content unchanged (same wyhash).

### 3. FTS5 Auto-Sync Triggers
- `files_ai` (AFTER INSERT) → Insert into FTS5
- `files_au` (AFTER UPDATE) → Delete old + insert new
- `files_ad` (AFTER DELETE) → Delete from FTS5

### 4. Health-Gated Search
```
check_health_fast() → auto_init() or backup_and_reinit()
```
Search auto-repairs database before executing query.

### 5. Atomic Reindex
```
Build in .ffts-index.db.tmp → checkpoint WAL → atomic rename → cleanup old WAL
```

## Verification

These diagrams were verified against source code:
- `rust-fts5-indexer/src/main.rs`
- `rust-fts5-indexer/src/indexer.rs` (lines 95-182)
- `rust-fts5-indexer/src/db.rs`
- `rust-fts5-indexer/src/search.rs`
- `rust-fts5-indexer/src/doctor.rs`
- `rust-fts5-indexer/src/init.rs`

## Related Documentation

- [agent-notes.md](../../rust-fts5-indexer/agent-notes.md) - Engineering decisions log
- [CLAUDE.md](../../CLAUDE.md) - Project overview
- [STATE_MACHINES.md](../STATE_MACHINES.md) - Legacy diagrams (Zig version, outdated)
