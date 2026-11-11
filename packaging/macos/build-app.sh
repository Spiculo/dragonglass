#!/usr/bin/env bash
#
# Assemble DragonGlass.app (and a zip and dmg) from an already-built binary.
# No Rust toolchain needed, so this runs on a release runner or by hand.
#
# usage: build-app.sh <binary> <version> <out-dir> [cli-binary]

set -euo pipefail

BINARY="${1:?usage: build-app.sh <binary> <version> <out-dir> [cli-binary]}"
VERSION="${2:?version required}"
OUT="${3:?output directory required}"
CLI="${4:-}"
HERE="$(cd "$(dirname "$0")" && pwd)"
NAME="DragonGlass"

if [[ ! -f "$BINARY" ]]; then
  echo "no binary at $BINARY" >&2
  exit 1
fi

mkdir -p "$OUT"
APP="$OUT/$NAME.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"

install -m 0755 "$BINARY" "$APP/Contents/MacOS/dragon-glass-app"
if [[ -n "$CLI" ]]; then
  install -m 0755 "$CLI" "$APP/Contents/MacOS/dragon-glass"
fi
install -m 0644 "$HERE/dragon-glass.icns" "$APP/Contents/Resources/dragon-glass.icns"
install -m 0644 "$HERE/Info.plist" "$APP/Contents/Info.plist"
printf 'APPL????' > "$APP/Contents/PkgInfo"

# CFBundleVersion must be a dotted list of integers; a tag like v0.1.0-rc1 is not.
NUMERIC_VERSION="$(printf '%s' "$VERSION" | sed -E 's/[^0-9.].*$//; s/^\.//; s/\.$//')"
[[ -n "$NUMERIC_VERSION" ]] || NUMERIC_VERSION="0.0.0"
/usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString $VERSION" "$APP/Contents/Info.plist"
/usr/libexec/PlistBuddy -c "Set :CFBundleVersion $NUMERIC_VERSION" "$APP/Contents/Info.plist"

# Ad-hoc signature: enough for the app to launch on Apple Silicon and for Launch Services
# to register the bundle, without a Developer ID certificate.
codesign --force --deep --sign - "$APP" >/dev/null

ZIP="$OUT/$NAME-$VERSION-macos.zip"
ditto -c -k --sequesterRsrc --keepParent "$APP" "$ZIP"

# Stage the app next to an Applications link, so the dmg window supports drag-to-install.
DMG="$OUT/$NAME-$VERSION-macos.dmg"
STAGE="$OUT/.dmg-stage"
rm -rf "$STAGE"
mkdir -p "$STAGE"
cp -R "$APP" "$STAGE/$NAME.app"
ln -s /Applications "$STAGE/Applications"
rm -f "$DMG"
hdiutil create -quiet -volname "Dragon Glass" -srcfolder "$STAGE" -ov -format UDZO "$DMG"
rm -rf "$STAGE"

echo "wrote $APP"
echo "wrote $ZIP"
echo "wrote $DMG"
