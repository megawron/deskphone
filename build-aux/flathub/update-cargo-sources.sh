#!/usr/bin/env bash
# Regenerates cargo-sources.json for offline Flathub builds after Cargo.lock changes.
#
# Usage: build-aux/flathub/update-cargo-sources.sh
set -euo pipefail

cd "$(dirname "$0")/../.."
GENERATOR_URL="https://raw.githubusercontent.com/flatpak/flatpak-builder-tools/master/cargo/flatpak-cargo-generator.py"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

curl -fsSL -o "$TMP_DIR/flatpak-cargo-generator.py" "$GENERATOR_URL"
python3 -m venv "$TMP_DIR/venv"
"$TMP_DIR/venv/bin/pip" install -q "aiohttp>=3.9.5,<4" "PyYAML>=6.0.2,<7" "tomlkit>=0.13.3,<1"
"$TMP_DIR/venv/bin/python" "$TMP_DIR/flatpak-cargo-generator.py" Cargo.lock \
    -o build-aux/flathub/cargo-sources.json

echo "Updated build-aux/flathub/cargo-sources.json"
