#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT_DIR}/rust-fts5-indexer"

exec cargo run --quiet --bin release-tools -- check-version "$@"
