#!/bin/bash
set -euo pipefail

#===============================================================================
# Deploy ffts-grep binary to a local install directory (default: ~/.local/bin)
#===============================================================================

usage() {
  cat <<'USAGE'
Usage: ./deploy.sh [--install-dir <dir>] [--skip-build] [--no-sign]

Options:
  --install-dir <dir>  Installation directory (default: ~/.local/bin or $INSTALL_DIR)
  --skip-build         Skip cargo build step (expects release binary to exist)
  --no-sign            Skip macOS codesign step
  -h, --help           Show this help
USAGE
}

INSTALL_DIR_DEFAULT="${INSTALL_DIR:-$HOME/.local/bin}"
INSTALL_DIR="$INSTALL_DIR_DEFAULT"
SKIP_BUILD=false
NO_SIGN=false

while [[ $# -gt 0 ]]; do
  case "$1" in
    --install-dir)
      INSTALL_DIR="$2"
      shift 2
      ;;
    --skip-build)
      SKIP_BUILD=true
      shift
      ;;
    --no-sign)
      NO_SIGN=true
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage
      exit 1
      ;;
  esac
 done

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RUST_DIR="$SCRIPT_DIR/rust-fts5-indexer"
BINARY_NAME="ffts-grep"
BUILD_ARTIFACT="$RUST_DIR/target/release/$BINARY_NAME"
INSTALL_PATH="$INSTALL_DIR/$BINARY_NAME"

echo "=== Deploying $BINARY_NAME ==="

if [[ "$SKIP_BUILD" == "false" ]]; then
  echo "[1/3] Building Rust project..."
  (cd "$RUST_DIR" && cargo build --release)
else
  echo "[1/3] Skipping build (using existing release binary)"
fi

if [[ ! -f "$BUILD_ARTIFACT" ]]; then
  echo "ERROR: Build artifact not found at $BUILD_ARTIFACT" >&2
  exit 1
fi

echo "[2/3] Installing binary to $INSTALL_PATH..."
mkdir -p "$INSTALL_DIR"
cp "$BUILD_ARTIFACT" "$INSTALL_PATH"
chmod +x "$INSTALL_PATH"

if [[ "$(uname)" == "Darwin" && "$NO_SIGN" == "false" ]]; then
  if ! codesign -s - --force "$INSTALL_PATH" 2>/dev/null; then
    echo "      Warning: codesign failed (binary may not run)" >&2
  fi
fi

echo "[3/3] Verifying installation..."
"$INSTALL_PATH" --version || true

if [[ ":$PATH:" != *":$INSTALL_DIR:"* ]]; then
  echo "Note: $INSTALL_DIR is not in your PATH."
  echo "      Add it with: export PATH=\"$INSTALL_DIR:\$PATH\""
fi

echo "=== Deployment complete ==="
