# Engineering Exec Spec: Indexing Failure Semantics + Filename LIKE Escaping

## Problem statement and goals
Two correctness gaps surfaced during full-repo review: (1) indexing continued after database write failures, producing silent partial indexes, and (2) filename substring search used SQL LIKE without escaping `%` and `_`, letting wildcard semantics leak into literal searches. In addition, the crate-level doctest used the repo’s working directory and could collide with existing databases. The goal is to make database write errors fail fast with rollback, treat `%` and `_` literally in filename substring matching, harden the doctest to use an isolated temp directory, and update tutorial docs to match runtime behavior.

## Non-goals and constraints
- Non-goal: Change BM25 weighting, schema design, or search ranking strategy.
- Non-goal: Introduce new dependencies for doctest isolation.
- Constraint: Preserve current public APIs and MSRV policy.

## System overview (relevant modules/files)
- `rust-fts5-indexer/src/indexer.rs` (indexing loop, transaction batching)
- `rust-fts5-indexer/src/db.rs` (filename substring search)
- `rust-fts5-indexer/src/lib.rs` (public doctest example)
- `docs/learn/08-indexer_rs.md`, `docs/learn/09-search_rs.md` (tutorial alignment)

## Comprehensive multi-phase TODO checklist + acceptance criteria

### Phase 1: Indexing failure semantics
1) **Fail fast on database write errors**
   - Treat `IndexerError::Database` as fatal during indexing.
   - Roll back an active transaction before returning.
   - Acceptance: DB write errors stop indexing and return an error (no silent partial index).

2) **Regression test for DB error behavior**
   - Force writes to fail via `PRAGMA query_only=ON` and verify error propagation.
   - Acceptance: Unit test asserts `IndexerError::Database` on indexing.

### Phase 2: Filename substring LIKE escaping
3) **Escape LIKE wildcards**
   - Escape `%`, `_`, and `\` in filename substring search.
   - Use separate parameters for equality vs. LIKE patterns.
   - Acceptance: Queries containing `%` or `_` match literally.

4) **Regression tests for literal matching**
   - Add tests for `%` and `_` in filenames.
   - Acceptance: Results exclude wildcard matches.

### Phase 3: Doctest isolation
5) **Use temp directory in doctest**
   - Create an isolated temp directory and clean up after the example.
   - Acceptance: Doctest no longer depends on repo state.

### Phase 4: Documentation
6) **Update tutorial docs**
   - Align indexing error handling description.
   - Update search tutorial to reflect two-phase flow and LIKE escaping.
   - Acceptance: Docs match actual behavior and examples.

## Test plan
- `cd rust-fts5-indexer && cargo fmt`
- `cd rust-fts5-indexer && cargo test`
- `cd rust-fts5-indexer && cargo clippy --all-targets -- -D warnings`
- Optional (scheduled): `cargo test --test memory_validation -- --ignored --nocapture`

## Risks and mitigations
- **Behavior change for `%`/`_` queries**: Now treated literally; mitigated by docs and tests.
- **Fail-fast on DB errors**: Users now see errors instead of partial results; mitigated by explicit logging and deterministic exit.
- **Doctest temp dir cleanup**: Failure to remove temp dir is benign; best-effort cleanup used.

## Rollout/rollback plan
- Rollout: merge once tests and clippy are green.
- Rollback: revert indexing and LIKE-escaping commits if regressions appear; no data migrations required.
