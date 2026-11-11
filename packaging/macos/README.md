# macOS packaging

Builds `DragonGlass.app`, a zip and a dmg from an already-built binary. The bundle is
**ad-hoc signed**, not notarized: it runs, but the first launch needs a right-click →
**Open** (or System Settings → Privacy & Security → **Open Anyway**) because Gatekeeper
does not trust it.

## Build

```bash
packaging/macos/build-universal.sh 0.0.1 dist
```

This builds `dragon-glass-app` and `dragon-glass` for `aarch64-apple-darwin` and
`x86_64-apple-darwin`, merges them with `lipo`, then calls `build-app.sh`. Add the Rust
targets once with `rustup target add aarch64-apple-darwin x86_64-apple-darwin`.

To wrap a single-architecture build (for example a debug binary):

```bash
cargo build -p dragonglass-ui -p dragonglass-cli
packaging/macos/build-app.sh target/debug/dragon-glass-app 0.0.1 dist target/debug/dragon-glass
```

## Files

| File | Purpose |
|---|---|
| `build-universal.sh` | Universal release build, then packaging |
| `build-app.sh` | Assembles the `.app`, zip and dmg (with an Applications link for drag-to-install) from a binary |
| `Info.plist` | Bundle metadata and the `dragonglass://` URL type |
| `dragon-glass.icns` | App icon, generated from `make-icon.swift` |
| `make-icon.swift` | Draws the placeholder 1024×1024 icon |
| `make-icns.sh` | Regenerates `dragon-glass.icns` (`sips` + `iconutil`) |

The icon is a placeholder until real artwork exists; replace `dragon-glass.icns` or edit
`make-icon.swift` and rerun `make-icns.sh`.

## Signing and notarization

For a release that opens without a Gatekeeper prompt, sign with a Developer ID and
notarize:

```bash
codesign --force --deep --options runtime \
  --sign "Developer ID Application: <Name> (<TEAMID>)" dist/DragonGlass.app
xcrun notarytool submit dist/DragonGlass-0.0.1-macos.zip \
  --apple-id <id> --team-id <TEAMID> --password <app-specific-password> --wait
xcrun stapler staple dist/DragonGlass.app
```

`build-app.sh` never signs with a certificate; CI uses the ad-hoc signature.

## URL scheme

`Info.plist` declares `CFBundleURLTypes` for `dragonglass`. When the app runs from a
persistent location such as `/Applications`, it registers itself through
`NSWorkspace.setDefaultApplicationAtURL` (GPUI's `register_url_scheme`) and Launch Services
delivers later `dragonglass://` links to the running app. A bundle in `/tmp` is ignored by
Launch Services, so install it first.

`dragon-glass register-uri --apply` does the same with `lsregister` for a bundle; run it
from `DragonGlass.app/Contents/MacOS/dragon-glass`.
