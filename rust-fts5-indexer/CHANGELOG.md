# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Nothing yet.

### Fixed
- Nothing yet.

## [0.11.4] - 2026-01-28

### Added
- **Release verification**: Recorded post-release smoke coverage for refresh guardrails and checklist validation

### Fixed
- Nothing yet.

## [0.11.3] - 2026-01-28

### Added
- **Explicit refresh**: `--refresh` flag for search and stdin protocol to reindex before searching
- **Benchmarks**: Updated baseline/final benchmark artifacts for regression tracking

### Fixed
- **Refresh validation**: Reject empty or whitespace-only queries across CLI and stdin refresh modes
- **Implicit query parsing**: Ignore whitespace-only query tokens when deriving implicit searches

## [0.11] - 2026-01-13

### Added
- **Release tooling**: `release-tools` binary for release notes, checklist, and version checks
- **CI guardrail**: Version badge consistency check
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
- FTS5 token matching limitation: "intro" not matching "introduction" due to tokenization
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

[Unreleased]: https://github.com/mneves75/ffts-grep/compare/v0.11.4...HEAD
[0.11.4]: https://github.com/mneves75/ffts-grep/releases/tag/v0.11.4
[0.11.3]: https://github.com/mneves75/ffts-grep/releases/tag/v0.11.3
[0.11]: https://github.com/mneves75/ffts-grep/releases/tag/v0.11
[0.10]: https://github.com/mneves75/ffts-grep/releases/tag/v0.10
[0.9]: https://github.com/mneves75/ffts-grep/releases/tag/v0.9
