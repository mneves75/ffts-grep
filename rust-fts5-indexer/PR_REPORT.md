# Pull Request Report: Deletion Detection + Release Tooling + Safety Guards

**Branch**: `main`
**Date**: 2026-01-13
**Author**: Codex
**Status**: ✅ Ready for Review

---

## Summary

- Incremental indexing now prunes entries for files deleted from disk.
- Added `release-tools` binary + wrapper scripts for release checklist, version checks, and release notes from changelog.
- Added CI job to enforce README version badge consistency.
- Hardened metadata casts (mtime/size) with checked conversions and tests; clarified application_id storage.
- Updated docs and changelogs to reflect new behavior, tooling, and version bump to 0.11.

---

## Root Cause Analysis

- **Stale results**: Deleted files remained indexed because incremental indexing only upserted existing files.
- **Release drift**: Manual release steps allowed README badge and changelog to diverge from Cargo.toml version.
- **Safety gap**: u64→i64 casts for mtime/size were unchecked; application_id was set via a runtime cast.
- **Issue statement (pre-change)**: "The current indexer design uses lazy invalidation (checking content_hash + mtime on existing files) but lacks deletion detection. A future improvement could be to track indexed paths and prune entries for files that no longer exist on disk during incremental indexing."

---

## What Changed

### Code
- `src/db.rs`: new `prune_missing_files` method.
- `src/indexer.rs`: prune missing files post-index.
- `src/indexer.rs`: checked u64→i64 conversions for mtime/size with overflow tests.
- `src/constants.rs`: centralized application_id constants.
- `src/db.rs`: application_id stored via a dedicated i32 constant (no cast at pragma site).
- `src/bin/release_tools.rs`: release tooling CLI.
- `tests/release_tools.rs`: cross-platform release tooling tests.

### Tooling
- `scripts/`: checklist, release notes, and version check wrappers.
- `.github/workflows/ci.yml`: version consistency job.

### Docs
- `README.md`: deletion detection feature.
- `CONTRIBUTING.md`: release tooling instructions.
- `docs/learn/08-indexer_rs.md`: prune step documented.
- `docs/learn/07-db_rs.md`: application_id snippet updated.
- `README.md`: Assumptions & Limits section added.
- `rust-fts5-indexer/SELF_CRITIQUE.md`: TODOs updated to reflect resolved items.
- `CLAUDE.md`, `HOWTO.md`, `docs/learn/README.md`, `docs/state-machines/README.md`: version 0.11 updates and deletion-pruning notes.

### Changelog
- `CHANGELOG.md` and `rust-fts5-indexer/CHANGELOG.md`: 0.11 entries updated for pruning + tooling.

---

## Evidence (Commands Run)

```
cd rust-fts5-indexer
cargo fmt
cargo test
cargo clippy --all-targets -- -D warnings
```

Results:
- All tests and doctests passed; memory_validation ignored by design.
- Clippy clean.

---

## Risks / Mitigations

- **Pruning cost**: O(n) FS checks per index. Mitigated by only keeping a list of missing entries and pruning once per run.
- **Release tooling misuse**: Guarded by tests and CI version check.
- **Out-of-range metadata**: mtime/size overflow now guarded; files are skipped with a warning instead of corrupting stored values.

---

## Self‑Critique

### 3. What can you do better?

- Add an automated release checklist script to prevent manual omissions. (Done in this change via `scripts/release-checklist.sh` and `release-tools checklist`.)
- Include a CI job that checks README badge matches Cargo.toml version. (Done via `version-consistency` job.)
- Generate release notes directly from changelog entries to avoid drift. (Done via `release-tools release-notes`.)

---

## Follow‑ups (Optional)

- Add a `--prune`/`--no-prune` flag if users want to control deletion detection.
- Extend version check to verify changelog link targets.
