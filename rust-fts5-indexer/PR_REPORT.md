# Pull Request Report: Indexing Fail-Fast + Filename LIKE Escaping

**Branch**: `main`
**Date**: 2026-01-12
**Author**: Codex
**Status**: ✅ Ready for Review

---

## Summary

This change set hardens correctness in two hot paths:

1) **Indexing now fails fast on database write errors**, preventing silent partial indexes.
2) **Filename substring search treats `%` and `_` literally** by escaping SQL LIKE wildcards.

It also **isolates the crate doctest** to a temp directory and **updates tutorial docs** to match the new runtime behavior.

---

## Root Cause Analysis

- **Silent partial indexes**: the indexing loop logged database errors and continued, which could mask corrupted or locked database writes.
- **Wildcard leakage**: `search_filename_contains` used SQL `LIKE` without escaping user input, so `%` and `_` acted as wildcards.
- **Doctest collisions**: the doctest used the repo’s working directory, which could already contain a legacy `.ffts-index.db`.

---

## What Changed

### Code
- `src/indexer.rs`: treat `IndexerError::Database` as fatal; rollback active transaction; new regression test.
- `src/db.rs`: escape LIKE wildcards (`%`, `_`, `\`); add literal `%`/`_` tests.
- `src/lib.rs`: doctest uses a temp dir and cleans up after execution.

### Docs
- `docs/learn/08-indexer_rs.md`: clarify fail-fast DB error semantics.
- `docs/learn/09-search_rs.md`: document two-phase search and LIKE escaping.

### Spec
- `ENGINEERING_SPEC.md`: updated to reflect this workstream.

---

## Evidence (Commands Run)

```
cd rust-fts5-indexer
cargo fmt
cargo test
cargo clippy --all-targets -- -D warnings
```

Results:
- Unit + integration + doctests: **pass** (memory_validation ignored as designed).
- Clippy: **clean**.

---

## Risks / Mitigations

- **Users expecting `%`/`_` wildcards** in filename substring search now get literal matches.
  - Mitigation: behavior documented; can add explicit opt‑in wildcard mode later.
- **Fail-fast on DB errors** may abort indexing sooner than before.
  - Mitigation: avoids silent data loss; error is returned and logged.

---

## Follow‑ups (Optional)

- Add an explicit `--like` or `--wildcards` flag for filename substring search.
- Add a DB-lock integration test to validate rollback behavior under real contention.
