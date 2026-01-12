# Engineering Exec Spec: Repo Hardening, Toolchain, and CI

## Problem statement and goals
The repo is a cross‑platform CLI that mutates on‑disk state, but it lacked pinned tooling and continuous validation across OS/toolchain combinations. Earlier fixes hardened indexing and documentation; the remaining risk is verification drift (toolchain differences, untested platforms, and manual memory validation). The goal is to enforce reproducible tooling, add multi‑OS CI coverage, and automate expensive memory validation on a schedule, while keeping the MSRV stable and documentation accurate.

## Non‑goals and constraints
- Non‑goal: Change search semantics, schema design, or query ranking.
- Non‑goal: Remove ignored memory tests (they are intentionally heavy).
- Constraint: MSRV remains Rust 1.85 (Edition 2024).
- Constraint: CI and toolchain updates must not reduce local developer ergonomics.

## System overview (relevant modules/files)
- `rust-toolchain.toml` (pinned toolchain for dev and CI)
- `.github/workflows/ci.yml` (cross‑platform test + lint)
- `.github/workflows/memory-validation.yml` (scheduled memory validation)
- `.github/workflows/toolchain-bump.yml` (scheduled toolchain bump PR)
- `README.md` (toolchain/CI policy + badge)
- Rust crate: `rust-fts5-indexer/*`

## Comprehensive multi‑phase TODO checklist + acceptance criteria

### Phase 1: Toolchain pinning
1) **Pin the development toolchain**
   - Add `rust-toolchain.toml` targeting Rust 1.92.0 with `rustfmt` + `clippy`.
   - Acceptance: Local `cargo fmt`/`cargo clippy` are reproducible on any host.

### Phase 2: Cross‑platform CI
2) **Multi‑OS test matrix**
   - Run tests on Linux/macOS/Windows for both stable and MSRV (1.85.0).
   - Acceptance: CI detects platform regressions and MSRV drift.

3) **Lint job**
   - Run `cargo fmt -- --check` and `cargo clippy --all-targets -- -D warnings` on stable.
   - Acceptance: Linting fails fast before merge.

### Phase 3: Scheduled validation
4) **Memory validation schedule**
   - Add a weekly job that runs `memory_validation` ignored tests on Linux/macOS.
   - Acceptance: Memory regressions are visible without slowing normal CI.

5) **Automated toolchain bump PR**
   - Add a monthly job that opens a PR with the latest stable Rust in `rust-toolchain.toml`.
   - Acceptance: Toolchain stays current with minimal manual work.

### Phase 4: Documentation
6) **CI badge and toolchain policy**
   - Update README with CI + memory validation badges and toolchain/MSRV policy.
   - Acceptance: Users can see CI health and expected Rust versions.

7) **Contributing guidance**
   - Add `CONTRIBUTING.md` with toolchain and verification requirements.
   - Acceptance: Contributors have a single source of truth for local setup and checks.

## Test plan
- `cd rust-fts5-indexer && cargo fmt -- --check`
- `cd rust-fts5-indexer && cargo test`
- `cd rust-fts5-indexer && cargo clippy --all-targets -- -D warnings`
- Scheduled: `cargo test --test memory_validation -- --ignored --nocapture`

## Risks and mitigations
- **CI runtime increase**: mitigated by keeping lint separate and memory tests scheduled only.
- **Toolchain drift**: mitigated by monthly PR automation.
- **Platform‑specific failures**: mitigated by Windows/macOS/Linux coverage.

## Rollout/rollback plan
- Rollout: merge after CI green on the new matrix.
- Rollback: revert workflow/toolchain commits; no data migrations required.
