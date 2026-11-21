#!/usr/bin/env bash
#
# Build a .deb from an already-built binary. No build toolchain needed, so it can run on a
# release runner or in a container.
#
# usage: build-deb.sh <binary> <version> <out-dir>
#
# The GUI binary is picked up from the same directory as the given CLI binary.

set -euo pipefail

BINARY="${1:?usage: build-deb.sh <binary> <version> <out-dir>}"
VERSION="${2:?version required}"
OUT="${3:?output directory required}"
NAME="dragon-glass"
HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
APP_BINARY="$(dirname "$BINARY")/dragon-glass-app"
ARCH="${ARCH:-$(dpkg --print-architecture 2>/dev/null || echo amd64)}"

ROOT="$(mktemp -d)"
trap 'rm -rf "$ROOT"' EXIT

mkdir -p "$ROOT/DEBIAN" \
         "$ROOT/usr/bin" \
         "$ROOT/usr/share/applications" \
         "$ROOT/usr/share/icons/hicolor/256x256/apps" \
         "$ROOT/usr/share/doc/$NAME"

install -m 0755 "$BINARY" "$ROOT/usr/bin/$NAME"
if [ -x "$APP_BINARY" ]; then
  install -m 0755 "$APP_BINARY" "$ROOT/usr/bin/dragon-glass-app"
else
  echo "warning: $APP_BINARY not found; the desktop entry will not launch" >&2
fi
install -m 0644 "$HERE/dragon-glass.desktop" "$ROOT/usr/share/applications/$NAME.desktop"
install -m 0644 "$REPO/packaging/logo.png" \
  "$ROOT/usr/share/icons/hicolor/256x256/apps/$NAME.png"

cat > "$ROOT/DEBIAN/control" <<EOF
Package: $NAME
Version: $VERSION
Section: editors
Priority: optional
Architecture: $ARCH
Maintainer: Dragon Glass contributors
Homepage: https://github.com/Spiculo/dragonglass
Description: Native, local-first markdown knowledge base
 A fast, native, Obsidian-compatible note vault that keeps notes as plain text.
EOF

cat > "$ROOT/usr/share/doc/$NAME/copyright" <<EOF
Dragon Glass is free software, distributed under the GNU General Public License
version 3 or later. See /usr/share/common-licenses/GPL-3 for the full text.
EOF

mkdir -p "$OUT"
dpkg-deb --root-owner-group --build "$ROOT" "$OUT/${NAME}_${VERSION}_${ARCH}.deb"
echo "wrote $OUT/${NAME}_${VERSION}_${ARCH}.deb"
