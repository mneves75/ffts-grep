# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- **Explicit refresh**: `--refresh` flag for search and stdin protocol to reindex before searching

### Fixed
- **Refresh validation**: Reject empty or whitespace-only queries across CLI and stdin refresh modes

## [0.11.2] - 2026-01-27

### Added
- **Documentation**: Added `FOR_YOU_KNOW_MARCUS.md` session learning journal on stale index behavior

### Changed
- **Search sanitization**: Single-pass whitespace collapsing to reduce allocations
- **JSON output**: Zero-copy serialization for search results
- **Filename contains ordering**: Case-insensitive ordering without LOWER() calls
- **Prune deletes**: Reuse prepared DELETE statement inside transactions

### Fixed
- Nothing yet.

## [0.11.1] - 2026-01-13

### Added
- **Durability**: Fsync helpers for files and parent directories to harden atomic reindex and recovery paths
- **Windows coverage**: Directory fsync smoke test in CI

### Fixed
- **Windows fsync**: Use read/write handles for `sync_all` and correct Win32 handle usage to avoid access errors
- **Atomic reindex**: Ensure directory fsyncs are durable across platforms; stabilize WAL cleanup tests on Windows
- **Symlink containment**: Canonical root handling prevents symlink escape while allowing explicit symlink roots
- **CI stability**: Performance tests account for CI timing variability without relaxing local budgets

## [0.11] - 2026-01-13

### Added
- **Release tooling**: Automated checklist, version consistency check, and release-note extraction scripts
- **CI guardrail**: Version badge consistency job for README vs Cargo.toml
- **Safety guards**: Checked conversions for file mtime/size to avoid overflow
- **Constants**: Centralized application_id constants to avoid casting surprises

### Fixed
- **Deletion detection**: Incremental indexing now prunes entries for files removed from disk

## [0.10] - 2026-01-13

### Added
- **Two-phase search**: Filename substring matching now works correctly
  - Phase A: SQL LIKE '%query%' for filename CONTAINS matches (bypasses FTS5 tokenization)
  - Phase B: FTS5 BM25 for content/path matches
  - Query "intro" now finds "01-introduction.md" (previously only exact token matches worked)
- **Auto-prefix detection**: Trailing `-` or `_` triggers FTS5 prefix query
  - "01-" becomes "01*" (matches "01-introduction", "01-chapter")
  - "test_" becomes "test*" (matches "test_utils", "test_config")
- **Filename ordering**: Results ordered by exact match > prefix match > contains match > shorter filename
- **Tests**: Added coverage for two-phase search, auto-prefix behavior, DB error handling, and literal `%`/`_` filename queries

### Fixed
- **Indexing correctness**: Database write errors now fail fast with rollback (no silent partial indexes)
- **Filename searches**: Escaped SQL LIKE wildcards so `%` and `_` are treated literally
- **Doctest isolation**: Examples run in a temporary directory to avoid repo-state collisions

## [0.9] - 2026-01-11

Initial public release. Fast full-text search file indexer using SQLite FTS5.

### Added

- **CLI**: `ffts-grep init`, `index`, `search`, `doctor` subcommands
- **FTS5 search**: BM25 ranking with filename-aware weights (100:50:1)
- **Claude Code integration**: Stdin JSON protocol, auto-init on first search
- **Reliability**: Atomic reindex, race-safe temp files, WAL mode
- **Quality**: 193 tests, clippy pedantic compliance, Rust Edition 2024

[Unreleased]: https://github.com/mneves75/ffts-grep/compare/v0.11.2...HEAD
[0.11.2]: https://github.com/mneves75/ffts-grep/releases/tag/v0.11.2
[0.11.1]: https://github.com/mneves75/ffts-grep/releases/tag/v0.11.1
[0.11]: https://github.com/mneves75/ffts-grep/releases/tag/v0.11
[0.10]: https://github.com/mneves75/ffts-grep/releases/tag/v0.10
[0.9]: https://github.com/mneves75/ffts-grep/releases/tag/v0.9
