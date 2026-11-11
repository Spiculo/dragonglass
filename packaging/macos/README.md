# macOS packaging

Builds `DragonGlass.app`, a zip and a dmg. The bundle is **ad-hoc signed** by default,
not notarized: it runs, but the first launch of a downloaded copy needs a right-click →
**Open** (or System Settings → Privacy & Security → **Open Anyway**) because Gatekeeper
does not trust it.

## Build

```bash
packaging/macos/build-universal.sh 0.0.1 dist
```

This builds `dragon-glass-app` and `dragon-glass` for `aarch64-apple-darwin` and
`x86_64-apple-darwin`, merges them with `lipo`, then calls `build-app.sh`. Add the Rust
targets once with `rustup target add aarch64-apple-darwin x86_64-apple-darwin`.

To wrap a single-architecture build, for example a debug binary:

```bash
cargo build -p dragonglass-ui -p dragonglass-cli
BIN_DIR=target/debug packaging/macos/build-app.sh 0.0.1 dist
```

Xcode command line tools are required once (`xcode-select --install`). GPUI compiles its
Metal shaders at runtime on macOS, so a full Xcode install is not needed.

Artifacts:

- `dist/DragonGlass.app`
- `dist/DragonGlass-<version>-macos.zip`
- `dist/DragonGlass-<version>-macos.dmg`, with an `Applications` link for drag-to-install

## Files

| File | Purpose |
|---|---|
| `build-universal.sh` | Universal release build, then packaging |
| `build-app.sh` | Assembles the `.app`, zip and dmg from built binaries |
| `Info.plist` | Bundle metadata, `dragonglass://` URL type and Markdown document types |

The icon is generated from `packaging/logo.png` with `sips` and `iconutil`.

## First launch of a downloaded build

A ZIP or dmg downloaded from the internet gets a quarantine flag. Either right-click the
app and choose **Open**, or clear the flag:

```bash
xattr -dr com.apple.quarantine DragonGlass.app
open DragonGlass.app
```

## Signing and notarization

Without `CODESIGN_IDENTITY` the script ad-hoc signs the bundle. That is enough to run
locally and for Launch Services to register the URL scheme.

For a release that opens without a Gatekeeper prompt, sign with a Developer ID and
notarize:

```bash
export CODESIGN_IDENTITY="Developer ID Application: <Name> (<TEAMID>)"
export APPLE_ID="you@example.com"
export APPLE_TEAM_ID="TEAMID"
export APPLE_APP_PASSWORD="xxxx-xxxx-xxxx-xxxx"

packaging/macos/build-universal.sh 0.0.1 dist
```

The script then signs with the hardened runtime, submits the ZIP with `notarytool`, and
staples the ticket to the app.

## Markdown documents

`Info.plist` declares Markdown as a document type, so Finder lists Dragon Glass under
**Open With**. Launching the app with a `.md` file opens that note: the app resolves the
file to its vault (the most recent vault that contains it, otherwise its folder) and loads
the note.

While the app is already running, macOS delivers the file as an Apple Event and GPUI 0.2.2
only surfaces URL opens (`on_open_urls`), so the request is dropped. Quit the app first, or
use `dragonglass://open?vault=/path/to/vault&note=notes/a.md`.

## URL scheme

`Info.plist` declares `CFBundleURLTypes` for `dragonglass`. When the app runs from a
persistent location such as `/Applications`, it registers itself through
`NSWorkspace.setDefaultApplicationAtURL` (GPUI's `register_url_scheme`) and Launch Services
delivers later `dragonglass://` links to the running app. A bundle in `/tmp` is ignored by
Launch Services, so install it first.

`dragon-glass register-uri --apply` does the same with `lsregister` for a bundle; run it
from `DragonGlass.app/Contents/MacOS/dragon-glass`.

## Known gaps

None of these block v0.0.1:

- No **About** panel. GPUI has no standard about window and the app does not build one.
- `Cmd+W` closes the tab, not the window, and there is no **Close Window** menu item.
- The dmg has no background image or arranged window; it holds the app and an
  `Applications` link.
- The CLI ships in `Contents/MacOS` but is not linked into `PATH`.
- Release builds are ad-hoc signed until a Developer ID and notarization credentials are
  available.
- The Intel (`x86_64`) slice is built and lipo'd but has not been executed on Intel
  hardware or under Rosetta.
- Double-clicking a `.md` while the app runs does nothing; see **Markdown documents** above.
