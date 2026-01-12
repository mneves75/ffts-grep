# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **Two-phase search**: Filename substring matching now works correctly
  - Phase A: SQL LIKE '%query%' for filename CONTAINS matches (bypasses FTS5 tokenization)
  - Phase B: FTS5 BM25 for content/path matches
  - Query "intro" now finds "01-introduction.md" (previously only exact token matches worked)
- **Auto-prefix detection**: Trailing `-` or `_` triggers FTS5 prefix query
  - "01-" becomes "01*" (matches "01-introduction", "01-chapter")
  - "test_" becomes "test*" (matches "test_utils", "test_config")
- **Filename ordering**: Results ordered by exact match > prefix match > contains match > shorter filename
- **Tests**: 19 new tests for two-phase search and auto-prefix behavior

### Fixed

- FTS5 token matching limitation: "intro" not matching "introduction" due to tokenization

## [0.9] - 2026-01-11

Initial public release. Fast full-text search file indexer using SQLite FTS5.

### Added

- **CLI**: `ffts-grep init`, `index`, `search`, `doctor` subcommands
- **FTS5 search**: BM25 ranking with filename-aware weights (100:50:1)
- **Claude Code integration**: Stdin JSON protocol, auto-init on first search
- **Reliability**: Atomic reindex, race-safe temp files, WAL mode
- **Quality**: 193 tests, clippy pedantic compliance, Rust Edition 2024

[0.9]: https://github.com/mneves75/ffts-grep/releases/tag/v0.9
