#!/usr/bin/env bash
#
# Build "Dragon Glass.app" (and a DMG + ZIP) from already-built release binaries.
# Must run on macOS: it uses sips, iconutil, ditto and hdiutil.
#
# usage: packaging/macos/build-app.sh <version> <out-dir>
#
# Optional environment:
#   BUNDLE_ID            bundle identifier (default: io.github.dragonglass.DragonGlass)
#   CODESIGN_IDENTITY    Developer ID Application identity; enables signing
#   APPLE_ID             Apple ID for notarization
#   APPLE_TEAM_ID        Apple team id for notarization
#   APPLE_APP_PASSWORD   app-specific password for notarization
#
# Without CODESIGN_IDENTITY the result is an unsigned app that still runs locally after
# clearing the quarantine attribute (see README.md).

set -euo pipefail

VERSION="${1:?usage: build-app.sh <version> <out-dir>}"
OUT="${2:?output directory required}"
HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
BIN_DIR="$REPO/target/release"
APP_NAME="Dragon Glass"
APP="$OUT/$APP_NAME.app"
BUNDLE_ID="${BUNDLE_ID:-io.github.dragonglass.DragonGlass}"
# CFBundleVersion must be a dotted numeric build number; keep the full version for
# CFBundleShortVersionString and the artefact names.
BUNDLE_VERSION="${VERSION%%-*}"
BUNDLE_VERSION="${BUNDLE_VERSION%%+*}"

if [ "$(uname -s)" != "Darwin" ]; then
    echo "build-app.sh must run on macOS (sips, iconutil, ditto and hdiutil are required)" >&2
    exit 1
fi
for binary in dragon-glass-app dragon-glass; do
    if [ ! -x "$BIN_DIR/$binary" ]; then
        echo "missing $BIN_DIR/$binary; run cargo build --release -p dragonglass-ui -p dragonglass-cli first" >&2
        exit 1
    fi
done

echo "==> staging $APP"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
install -m 0755 "$BIN_DIR/dragon-glass-app" "$APP/Contents/MacOS/dragon-glass-app"
install -m 0755 "$BIN_DIR/dragon-glass" "$APP/Contents/MacOS/dragon-glass"

# Info.plist: substitute the version and bundle id. `|` avoids escaping the slashes in
# the identifier and version.
sed \
    -e "s|@VERSION@|$VERSION|g" \
    -e "s|@BUNDLE_VERSION@|$BUNDLE_VERSION|g" \
    -e "s|@BUNDLE_ID@|$BUNDLE_ID|g" \
    "$HERE/Info.plist" > "$APP/Contents/Info.plist"
printf 'APPL????' > "$APP/Contents/PkgInfo"

echo "==> building DragonGlass.icns"
ICONSET="$OUT/DragonGlass.iconset"
rm -rf "$ICONSET"
mkdir -p "$ICONSET"
SOURCE_ICON="$REPO/packaging/logo.png"
if [ -f "$SOURCE_ICON" ]; then
    for size in 16 32 128 256 512; do
        sips -z "$size" "$size" "$SOURCE_ICON" \
            --out "$ICONSET/icon_${size}x${size}.png" >/dev/null
        double=$((size * 2))
        sips -z "$double" "$double" "$SOURCE_ICON" \
            --out "$ICONSET/icon_${size}x${size}@2x.png" >/dev/null
    done
    iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/DragonGlass.icns"
    rm -rf "$ICONSET"
else
    echo "warning: $SOURCE_ICON is missing; the app will have a generic icon" >&2
fi

if [ -n "${CODESIGN_IDENTITY:-}" ]; then
    echo "==> signing with $CODESIGN_IDENTITY"
    codesign --force --deep --options runtime --timestamp \
        --sign "$CODESIGN_IDENTITY" "$APP"
    codesign --verify --deep --strict --verbose=2 "$APP"
else
    echo "==> no CODESIGN_IDENTITY set; building an unsigned app"
fi

ZIP="$OUT/DragonGlass-$VERSION-macos.zip"
if [ -n "${CODESIGN_IDENTITY:-}" ] \
    && [ -n "${APPLE_ID:-}" ] \
    && [ -n "${APPLE_TEAM_ID:-}" ] \
    && [ -n "${APPLE_APP_PASSWORD:-}" ]; then
    echo "==> notarizing"
    ditto -c -k --keepParent "$APP" "$ZIP"
    xcrun notarytool submit "$ZIP" \
        --apple-id "$APPLE_ID" \
        --team-id "$APPLE_TEAM_ID" \
        --password "$APPLE_APP_PASSWORD" \
        --wait
    xcrun stapler staple "$APP"
else
    echo "==> skipping notarization (set CODESIGN_IDENTITY, APPLE_ID, APPLE_TEAM_ID and APPLE_APP_PASSWORD)"
fi

echo "==> writing $ZIP"
rm -f "$ZIP"
ditto -c -k --keepParent "$APP" "$ZIP"

echo "==> writing DMG"
DMG="$OUT/DragonGlass-$VERSION-macos.dmg"
STAGE="$OUT/dmg-stage"
rm -rf "$STAGE"
mkdir -p "$STAGE"
cp -R "$APP" "$STAGE/"
ln -s /Applications "$STAGE/Applications"
rm -f "$DMG"
hdiutil create -volname "$APP_NAME" -srcfolder "$STAGE" -ov -format UDZO "$DMG" >/dev/null
rm -rf "$STAGE"

echo "wrote:"
echo "  $APP"
echo "  $ZIP"
echo "  $DMG"
