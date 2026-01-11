#!/bin/bash
set -euo pipefail

#===============================================================================
# Deploy Rust FTS5 file indexer to ~/.claude/ and configure for integrations
#===============================================================================

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RUST_DIR="$SCRIPT_DIR/rust-fts5-indexer"
BINARY_NAME="ffts-grep"
INSTALL_DIR="$HOME/.claude"
SETTINGS_FILE="$INSTALL_DIR/settings.json"
BINARY_PATH="$INSTALL_DIR/$BINARY_NAME"
BUILD_ARTIFACT="$RUST_DIR/target/release/$BINARY_NAME"

echo "=== Deploying Rust FTS5 file indexer ==="

# 1. Build the Rust project
echo "[1/4] Building Rust project..."
cd "$RUST_DIR"
cargo build --release

# Validate build artifact exists
if [[ ! -f "$BUILD_ARTIFACT" ]]; then
    echo "ERROR: Build failed - binary not found at $BUILD_ARTIFACT" >&2
    exit 1
fi

# 2. Copy binary to ~/.claude/
echo "[2/4] Installing binary to $BINARY_PATH..."
mkdir -p "$INSTALL_DIR"
cp "$BUILD_ARTIFACT" "$BINARY_PATH"
chmod +x "$BINARY_PATH"

# Re-sign binary for macOS (required after copying to prevent SIGKILL)
if [[ "$(uname)" == "Darwin" ]]; then
    if ! codesign -s - --force "$BINARY_PATH" 2>/dev/null; then
        echo "      Warning: codesign failed (binary may not run)"
    fi
fi

# 3. Update settings.json to use the new binary (for Claude Code integration)
echo "[3/4] Updating settings..."
if [[ -f "$SETTINGS_FILE" ]]; then
    # Create backup
    cp "$SETTINGS_FILE" "$SETTINGS_FILE.bak"
    SETTINGS_TMP="$SETTINGS_FILE.tmp.$$"

    # Use jq if available for safer JSON manipulation, otherwise fall back to sed
    if command -v jq &>/dev/null; then
        # Atomic JSON update: write to temp, validate, then mv
        if jq --arg path "$BINARY_PATH" '.fileSuggestion.command = $path' "$SETTINGS_FILE.bak" > "$SETTINGS_TMP"; then
            # Validate the output is valid JSON
            if jq empty "$SETTINGS_TMP" 2>/dev/null; then
                mv "$SETTINGS_TMP" "$SETTINGS_FILE"
                echo "      Updated $SETTINGS_FILE (via jq, atomic)"
            else
                rm -f "$SETTINGS_TMP"
                echo "ERROR: jq produced invalid JSON, restoring backup" >&2
                cp "$SETTINGS_FILE.bak" "$SETTINGS_FILE"
                exit 1
            fi
        else
            rm -f "$SETTINGS_TMP"
            echo "ERROR: jq failed, settings unchanged" >&2
            exit 1
        fi
    else
        # Fallback to sed - platform-aware
        # Warning: This replaces ALL "command" keys, not just fileSuggestion
        if [[ "$(uname)" == "Darwin" ]]; then
            sed 's|"command":[ ]*"[^"]*"|"command": "'"$BINARY_PATH"'"|' "$SETTINGS_FILE.bak" > "$SETTINGS_TMP"
        else
            sed 's|"command":[ ]*"[^"]*"|"command": "'"$BINARY_PATH"'"|' "$SETTINGS_FILE.bak" > "$SETTINGS_TMP"
        fi
        mv "$SETTINGS_TMP" "$SETTINGS_FILE"
        echo "      Updated $SETTINGS_FILE (via sed, atomic)"
        echo "      Warning: sed-based update may affect other 'command' keys. Install jq for safer updates."
    fi
else
    echo "      Warning: $SETTINGS_FILE not found, skipping settings update"
fi

# 4. Index the current project
echo "[4/4] Indexing current project ($SCRIPT_DIR)..."
cd "$SCRIPT_DIR"
"$BINARY_PATH" init

echo "=== Deployment complete ==="
echo ""
echo "Binary installed: $BINARY_PATH"
echo "Version: $("$BINARY_PATH" --version)"
echo "Run '$BINARY_PATH --help' for usage"
