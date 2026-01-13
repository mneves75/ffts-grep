# Engineering Exec Spec: Deletion Detection + Release Tooling + Safety Guards

## Problem statement and goals
Incremental indexing currently updates modified files but does not remove entries for files deleted from disk, leading to stale search results. Release tasks are also manual, risking omissions (badge mismatch, missing changelog entries, inconsistent release notes). Additionally, indexer metadata casts (mtime/size) relied on unchecked u64→i64 conversions and `application_id` used a runtime cast with a negative i32 value. The goals are to (1) prune missing files during incremental indexing, (2) add automated release tooling: checklist, version consistency checks, and release notes generation from the changelog plus CI enforcement, and (3) harden metadata conversions and document operational limits.

## Non-goals and constraints
- Non-goal: change search ranking, schema, or indexing semantics beyond deletion pruning.
- Non-goal: introduce new external dependencies for release tooling.
- Constraint: tools must run cross-platform in CI (Linux/macOS/Windows).

## System overview (relevant modules/files)
- `rust-fts5-indexer/src/db.rs` (prune missing files)
- `rust-fts5-indexer/src/indexer.rs` (call prune after indexing)
- `rust-fts5-indexer/src/indexer.rs` (checked u64→i64 conversions for mtime/size)
- `rust-fts5-indexer/src/db.rs` (application_id constant stored as i32)
- `rust-fts5-indexer/src/bin/release_tools.rs` (release tooling)
- `rust-fts5-indexer/tests/release_tools.rs` (tooling tests)
- `scripts/` (release scripts)
- `.github/workflows/ci.yml` (version consistency job)
- `CHANGELOG.md`, `rust-fts5-indexer/CHANGELOG.md` (release notes source)
- `README.md`, `CONTRIBUTING.md`, `docs/learn/08-indexer_rs.md`, `docs/learn/07-db_rs.md`, `rust-fts5-indexer/SELF_CRITIQUE.md`

## Comprehensive multi-phase TODO checklist + acceptance criteria

### Phase 1: Deletion detection
1) **Prune missing files after incremental indexing**
   - Add `Database::prune_missing_files` to remove rows whose files are gone on disk.
   - Call prune in `Indexer::index_directory` before ANALYZE/optimize.
   - Acceptance: deleted files disappear from search results after the next index run.

2) **Regression test**
   - Create a file, index, delete it, re-index, and assert DB count is 0.
   - Acceptance: test passes reliably across OSes.

### Phase 2: Release tooling
3) **Release tools binary**
   - Add `release-tools` with `check-version`, `release-notes`, and `checklist` subcommands.
   - Acceptance: binary outputs release notes from changelog and validates version badge.

4) **Cross-platform tests**
   - Add integration tests that execute `release-tools` commands.
   - Acceptance: tests pass on Linux/macOS/Windows CI.

5) **Release scripts**
   - Add wrapper scripts in `scripts/` for checklist, notes, and version check.
   - Acceptance: scripts run locally and invoke `release-tools`.

6) **CI guardrail**
   - Add a CI job to run `release-tools check-version`.
   - Acceptance: CI fails if README badge and Cargo.toml version diverge.

### Phase 3: Documentation
7) **Docs update**
   - Mention deletion pruning in README and indexing tutorial.
   - Document release tooling in CONTRIBUTING.
   - Acceptance: docs match actual behavior and tooling.

### Phase 4: Safety guards + assumptions
8) **Checked metadata conversions**
   - Add a helper for u64→i64 conversion with explicit bounds checks.
   - Use it for mtime/size to avoid overflow on extreme values.
   - Acceptance: new unit tests cover OK + overflow paths.

9) **Document assumptions & limits**
   - Add an “Assumptions & Limits” section to README.
   - Update db tutorial to show application_id constant usage without casts.
   - Acceptance: docs reflect real behavior and limits.

## Test plan
- `cd rust-fts5-indexer && cargo fmt`
- `cd rust-fts5-indexer && cargo test`
- `cd rust-fts5-indexer && cargo clippy --all-targets -- -D warnings`

## Risks and mitigations
- **Prune cost on large repos**: O(n) filesystem checks; mitigated by only tracking missing entries and running once per index.
- **Release tools drift**: mitigated by tests that run in CI and by deriving notes from changelog.
- **Extreme timestamps/sizes**: now guarded by checked conversions and logged skips.

## Rollout/rollback plan
- Rollout: merge after CI green (including version consistency job).
- Rollback: revert pruning/tools commit; no schema migrations required.
