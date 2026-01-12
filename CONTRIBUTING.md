# Contributing

Thank you for contributing to ffts-grep. This project is a cross-platform CLI
that mutates on-disk state; reproducibility and correctness are mandatory.

## Toolchain policy

- **MSRV**: Rust 1.85 (Edition 2024).
- **Pinned dev toolchain**: `rust-toolchain.toml` targets Rust 1.92.0.
- The pinned toolchain is for consistent formatting/linting and CI parity.

## Local setup

```bash
rustc --version
cargo --version
```

Rustup users will automatically pick up `rust-toolchain.toml` when running
commands in this repo.

## Verification

Run these before submitting a PR:

```bash
cd rust-fts5-indexer
cargo fmt -- --check
cargo test
cargo clippy --all-targets -- -D warnings
```

## Memory validation

The memory validation tests are heavy and ignored by default. To run them:

```bash
cd rust-fts5-indexer
cargo test --test memory_validation -- --ignored --nocapture
```

## CI

CI runs tests on Linux/macOS/Windows for both stable and MSRV. A scheduled
workflow runs memory validation weekly, and a monthly workflow opens a PR to
bump the pinned toolchain.
