#!/usr/bin/env bash
#
# Build a universal (Apple Silicon + Intel) DragonGlass.app, a zip and a dmg.
#
# usage: packaging/macos/build-universal.sh <version> [out-dir]
#
# Needs Rust with the two macOS targets. The first run adds them and downloads the
# standard library; later runs are offline.

set -euo pipefail

VERSION="${1:?usage: build-universal.sh <version> [out-dir]}"
OUT="${2:-dist}"
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
TARGETS=(aarch64-apple-darwin x86_64-apple-darwin)
STAGE="$OUT/universal"

cd "$ROOT"
rustup target add "${TARGETS[@]}" >/dev/null
for target in "${TARGETS[@]}"; do
  cargo build --release -p dragonglass-ui -p dragonglass-cli --target "$target"
done

mkdir -p "$STAGE"
lipo -create \
  "target/aarch64-apple-darwin/release/dragon-glass-app" \
  "target/x86_64-apple-darwin/release/dragon-glass-app" \
  -output "$STAGE/dragon-glass-app"
lipo -create \
  "target/aarch64-apple-darwin/release/dragon-glass" \
  "target/x86_64-apple-darwin/release/dragon-glass" \
  -output "$STAGE/dragon-glass"

BIN_DIR="$STAGE" "$HERE/build-app.sh" "$VERSION" "$OUT"
