# macOS packaging

This builds an unsigned **Dragon Glass.app** that runs on Apple Silicon (the current
`macos-latest` runner). Intel/universal builds and notarization are follow-up work.

## Build locally

```bash
# Xcode command line tools are required once:
xcode-select --install

cargo build --release -p dragonglass-ui -p dragonglass-cli
chmod +x packaging/macos/build-app.sh
packaging/macos/build-app.sh 0.0.1 dist
open "dist/Dragon Glass.app"
```

Artifacts:

- `dist/Dragon Glass.app`
- `dist/DragonGlass-<version>-macos.zip`
- `dist/DragonGlass-<version>-macos.dmg`

The app bundle declares the `dragonglass://` URL scheme and Markdown document types in
`Info.plist`.

## Unsigned builds

Because the app is not notarized yet, macOS may refuse the first launch or add a
quarantine flag when it was downloaded. For a local build there is no quarantine. For a
build copied from another machine:

```bash
xattr -dr com.apple.quarantine "Dragon Glass.app"
open "Dragon Glass.app"
```

## Signing and notarization

Set these in the environment before running `build-app.sh`:

```bash
export CODESIGN_IDENTITY="Developer ID Application: Your Name (TEAMID)"
export APPLE_ID="you@example.com"
export APPLE_TEAM_ID="TEAMID"
export APPLE_APP_PASSWORD="xxxx-xxxx-xxxx-xxxx"
```

The script then signs with the hardened runtime, submits the ZIP with `notarytool`,
staples the ticket to the app, and rebuilds the ZIP/DMG. The same variables are wired to
the `macos` job in `.github/workflows/release.yml` through repository secrets.

CI runs this script on `macos-latest` and then launches the resulting app for eight
seconds; that check passes. The artifact is uploaded as `dragon-glass-macos-ci`.

## Current limits

- `macos-latest` is arm64. Intel and universal builds need a second build and `lipo`.
- The app menu and macOS-specific runtime behavior (Cmd+Q, IME, VoiceOver, trackpad
  gestures) still need testing on a Mac.
- The CLI is shipped inside `Contents/MacOS` but is not linked into `PATH`; it can be
  invoked directly or symlinked by the user.
