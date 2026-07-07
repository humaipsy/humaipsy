#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
APP_DIR="$ROOT_DIR/apps/humai"
DIST_DIR="${HUMAI_DIST_DIR:-$HOME/Desktop/humai}"

echo "[1/3] Building release binary..."
cd "$APP_DIR"
cargo build --release

TARGET_DIR="$(cargo metadata --format-version=1 --no-deps --manifest-path Cargo.toml \
  | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')"
RELEASE_BIN="$TARGET_DIR/release/humai"

if [[ ! -f "$RELEASE_BIN" ]]; then
  echo "ERROR: Release binary not found at $RELEASE_BIN" >&2
  exit 1
fi

echo "[2/3] Preparing distribution folder at $DIST_DIR ..."
rm -rf "$DIST_DIR"
mkdir -p "$DIST_DIR"

echo "[3/3] Copying release files..."
cp "$RELEASE_BIN" "$DIST_DIR/humai"
chmod +x "$DIST_DIR/humai"

if [[ -d "$APP_DIR/assets" ]]; then
  cp -a "$APP_DIR/assets" "$DIST_DIR/assets"
fi

if [[ -f "$APP_DIR/vault/.master_hash" ]]; then
  mkdir -p "$DIST_DIR/vault"
  cp "$APP_DIR/vault/.master_hash" "$DIST_DIR/vault/.master_hash"
else
  echo "WARNING: $APP_DIR/vault/.master_hash not found. Distribution will stay locked until hash is provided."
fi

echo
echo "Distribution complete: $DIST_DIR"
