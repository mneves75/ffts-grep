# Engineering Exec Spec: Self-Critique of performance-claims-verification Plan (Current Branch)

## Problem statement and goals
The provided plan contains correctness and process assertions that are inconsistent with the current branch source code. The goal is to deliver a precise self-critique that identifies inaccurate assumptions, contradictions, missing evidence, and unnecessary steps, then propose a corrected, minimal plan focused on actual gaps in the current code (including the fact that there is no memory benchmark in `search_bench.rs`).

## Non-goals and constraints
- Non-goal: Implement any code changes in this run.
- Non-goal: Re-run benchmarks or modify README claims.
- Constraint: No unverified claims (tests, formatting, benchmark outputs) unless evidence is captured.
- Constraint: Use repository state as the source of truth.

## System overview (relevant modules/files)
- rust-fts5-indexer/benches/search_bench.rs
- rust-fts5-indexer/tests/memory_test.rs
- rust-fts5-indexer/tests/memory_validation.rs
- rust-fts5-indexer/Cargo.toml
- README.md (referenced by the plan)

## Comprehensive multi-phase TODO checklist + acceptance criteria
### Phase 1: Validate current state vs plan assumptions
- Read bench/test/Cargo files to confirm whether sysinfo or memory-stats is in use and whether memory validation already exists.
- Acceptance: Explicitly list each plan step that is already implemented or no longer relevant, including existing memory validation tests and benchmark documentation.

### Phase 2: Identify unverified or incorrect claims in the plan
- Check for claims about cargo fmt, tests, benchmark output, and README metrics that lack evidence.
- Acceptance: Each unverified claim is marked as such; no statements imply verification without evidence.

### Phase 3: Produce corrected minimal plan (if changes are still needed)
- Only include steps that map to real gaps in the current code (e.g., removing plan steps that assume a memory benchmark exists when it does not).
- Acceptance: Proposed steps are scoped, actionable, and map to actual code locations that still exist on the current branch.

### Phase 4: Deliver critique artifacts
- Provide a PR-style report focused on plan correctness issues and missing evidence.
- Provide verification commands (not executed) for any remaining open items.
- Acceptance: Report includes risks, mitigations, and explicit evidence policy.

## Test plan
- Executed in this run:
  - cargo fmt --check
  - cargo test
  - cargo test --test memory_validation -- --ignored --nocapture
  - cargo clippy --all-targets -- -D warnings
  - cargo bench --bench search_bench

## Risks and mitigations
- Risk: Plan references outdated dependencies; mitigation is to diff plan assumptions against current files.
- Risk: Plan asserts benchmark outputs without evidence; mitigation is to require command output before claims.
- Risk: Plan includes duplicate or conflicting phases; mitigation is to consolidate into a single corrected plan.

## Rollout/rollback plan
- No rollout required; this is a critique-only deliverable.
- If changes are later made, rollback is a standard git revert of those commits.
