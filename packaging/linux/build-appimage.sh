#!/usr/bin/env bash
#
# Build an AppImage from already-built binaries and the system GTK/GL libraries.
#
# Needs `appimagetool` and `linuxdeploy` on PATH, so it normally runs inside a container
# rather than on a developer machine.
#
# usage: build-appimage.sh <version> <out-dir>

set -euo pipefail

VERSION="${1:?usage: build-appimage.sh <version> <out-dir>}"
OUT="${2:?output directory required}"
HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"

APPDIR="$(mktemp -d)"
trap 'rm -rf "$APPDIR"' EXIT

install -Dm755 "$REPO/target/release/dragon-glass-app" "$APPDIR/usr/bin/dragon-glass-app"
install -Dm755 "$REPO/target/release/dragon-glass" "$APPDIR/usr/bin/dragon-glass"
install -Dm644 "$HERE/dragon-glass.desktop" "$APPDIR/usr/share/applications/dragon-glass.desktop"
install -Dm644 "$REPO/packaging/windows/dragon-glass.ico" \
  "$APPDIR/usr/share/icons/hicolor/32x32/apps/dragon-glass.png"
install -Dm644 "$REPO/LICENSE" "$APPDIR/usr/share/doc/dragon-glass/copyright"

# Pull in the GTK/Vulkan/X11 libraries the app links against.
linuxdeploy --appdir "$APPDIR"

cat > "$APPDIR/AppRun" <<'EOF'
#!/bin/sh
HERE="$(dirname "$(readlink -f "$0")")"
exec "$HERE/usr/bin/dragon-glass-app" "$@"
EOF
chmod +x "$APPDIR/AppRun"

mkdir -p "$OUT"
ARCH=x86_64 appimagetool "$APPDIR" "$OUT/dragon-glass-${VERSION}-x86_64.AppImage"
echo "wrote $OUT/dragon-glass-${VERSION}-x86_64.AppImage"
