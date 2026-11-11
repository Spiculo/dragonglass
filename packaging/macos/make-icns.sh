#!/usr/bin/env bash
#
# Regenerate packaging/macos/dragon-glass.icns from make-icon.swift.
# Needs only the macOS developer tools (swift, sips, iconutil).

set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

swift "$HERE/make-icon.swift" "$WORK/icon-1024.png" >/dev/null

ICONSET="$WORK/Dragon Glass.iconset"
mkdir -p "$ICONSET"
for size in 16 32 64 128 256 512; do
  sips -z "$size" "$size" "$WORK/icon-1024.png" \
    --out "$ICONSET/icon_${size}x${size}.png" >/dev/null
  double=$((size * 2))
  sips -z "$double" "$double" "$WORK/icon-1024.png" \
    --out "$ICONSET/icon_${size}x${size}@2x.png" >/dev/null
done

iconutil -c icns "$ICONSET" -o "$HERE/dragon-glass.icns"
echo "wrote $HERE/dragon-glass.icns"
