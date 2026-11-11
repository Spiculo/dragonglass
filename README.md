# Dragon Glass

[![CI](https://github.com/Spiculo/dragonglass/actions/workflows/ci.yml/badge.svg)](https://github.com/Spiculo/dragonglass/actions/workflows/ci.yml)
[![License: GPL-3.0-or-later](https://img.shields.io/badge/license-GPL--3.0--or--later-blue.svg)](LICENSE)

A fast, native, local-first knowledge base for a folder of markdown notes, built in Rust
with [GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui). No Electron, no
webview, no account.

Point Dragon Glass at an existing Obsidian vault. It reads and writes plain `.md`, preserves
YAML frontmatter, and leaves `.obsidian/` alone. There is no import step.

> **Status: v0.0.1, early.** Windows, Linux and macOS (Apple Silicon and Intel, as a
> universal app) are supported. Keep backups (git is a good one) and expect rough edges.

## Why another notes app?

Obsidian is Electron: hundreds of megabytes of RAM idle, slow cold starts, and a browser
engine between you and your text. Dragon Glass keeps the same job — a plain-text,
local-first vault — in a native window. The goals are low memory, a cold start under 200 ms,
and effectively zero idle CPU: work happens on filesystem events and input, not on timers
or polling.

The other half is compatibility. A vault is just a directory, so migrating should be as
simple as opening it. Dragon Glass reads and writes the same files Obsidian does. (It does
not run Obsidian's JavaScript plugins; its own plugin model is a WASM sandbox.)

## Features

A lot already works. This is a working application, not a mockup — but it is early.

**Editing**
- Source editor with a rope buffer, undo/redo, IME, word wrap and a line-number gutter
- Reading view and a live-preview pane beside the editor
- Syntax highlighting, code fences, tables, task lists, footnotes, callouts and
  `==highlights==`
- KaTeX-compatible math (`$…$` / `$$…$$`) via [RaTeX](https://github.com/erweixin/RaTeX),
  rendered natively and exported as vector SVG
- Tabs, split panes, detached windows, multiple windows per vault
- Find and replace with regex, capture groups, case and whole-word toggles
- Multi-cursor editing (Ctrl+D, Ctrl+Alt+Up/Down, Alt+click) and optional Vim mode
  (normal/visual/insert, counts, motions, operators, `/` search, `:w`, `:%s/…/…/g`)
- A formatting bar on text selection plus shortcuts for bold, italic, strikethrough,
  highlight, inline code and comments; `Ctrl+K` turns a selection into
  `[[target|selection]]`
- Wikilink, tag and slash-command autocomplete while typing
- Command palette, a settings page (`Ctrl+,`), configurable hotkeys, light/dark themes and
  snippets

**Vault and knowledge**
- File tree with new note/folder, rename, move and delete to a recoverable trash
  (right-click a row), plus the quick switcher and full-text search
- Wikilinks, backlinks, tags, outline, document relations and a graph view
- Daily/weekly/monthly notes, templates, properties, bookmarks
- Embeds and attachments (images, PDF, audio), transclusion, shared blocks
- A/B versions with a line diff, document review, block move/duplicate/delete/extract
- Recovery snapshots, browsable and restorable from **File recovery** in the palette

**Around the editor**
- Git sync by driving your own `git` binary (your credentials, SSH config and hooks)
- HTML and PDF export
- Read-only local HTTP API and an MCP server over stdio for AI tooling
- Importers for Evernote (`.enex`) and Roam exports
- A local package store for themes and WASM plugins
- A vault picker that remembers recent vaults and reopens the note you were last in

**Not there yet:** notarized macOS builds, Mermaid diagrams, a community (online)
theme/plugin store, end-to-end-encrypted sync, and full mobile parity (an Android vertical
slice and an assessed iOS path exist). See [`PLAN.md`](PLAN.md) for the full roadmap and
the decisions behind it.

## Build from source

Requirements: Rust **1.85+** (stable) and `git`.

On Windows, install the MSVC toolchain and Visual Studio Build Tools with the "Desktop
development with C++" workload. No other system libraries are needed.

On macOS, install the Xcode Command Line Tools (`xcode-select --install`). A full Xcode
install is not required: the UI crate compiles GPUI's Metal shaders at startup
(`runtime_shaders`), because the build-time shader compiler ships only with Xcode.

On Linux, install the build and GPU/windowing dependencies:

```bash
sudo apt-get update
sudo apt-get install -y --no-install-recommends \
  build-essential pkg-config cmake clang libclang-dev libssl-dev \
  libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev libx11-dev \
  libx11-xcb-dev libxcb1-dev libxcb-render0-dev libxcb-shape0-dev \
  libxcb-xfixes0-dev libvulkan-dev libfontconfig1-dev libfreetype-dev \
  libasound2-dev
```

Then:

```bash
git clone https://github.com/Spiculo/dragonglass.git
cd dragonglass
cargo build --release -p dragonglass-ui -p dragonglass-cli
```

On Windows, `packaging\windows\build.ps1` does the same and adds the Windows SDK
`fxc.exe` directory to `PATH` first (GPUI's shader build step needs it even when the C++
workload is installed).

On macOS, install the Xcode command-line tools once (`xcode-select --install`); a full
Xcode install is not required, because GPUI compiles its Metal shaders at runtime. Build a
universal (Apple Silicon + Intel) app bundle:

```bash
packaging/macos/build-universal.sh 0.0.1 dist
open dist/DragonGlass.app
```

The bundle is ad-hoc signed. Move `DragonGlass.app` to `/Applications` before you rely on
`dragonglass://` links: Launch Services ignores bundles in a temporary directory. CI runs
the same script on a macOS runner, launches the app for eight seconds and uploads the dmg
and zip as the `dragon-glass-macos-ci` artifact. Signing, notarization and the
single-architecture path are covered in
[`packaging/macos/README.md`](packaging/macos/README.md).

`target/release/dragon-glass-app` is the app; `target/release/dragon-glass` is the CLI.
The UI crate pulls in GPUI, so the workspace's default build (`cargo build`) deliberately
skips it — build it explicitly with `-p dragonglass-ui`.

## Run

```bash
# Open a specific vault
dragon-glass-app /path/to/vault

# Or no argument: reopen the last vault, or show the vault picker on a first run
dragon-glass-app
```

`dragonglass://` URIs (for example from a shortcut or automation) are also accepted:

```text
dragonglass://open?vault=/path/to/vault&note=notes/Index.md
```

### Vaults

The app remembers the vaults you open, outside any vault, in
`%APPDATA%\DragonGlass\vaults.txt` on Windows,
`~/Library/Application Support/DragonGlass/vaults.txt` on macOS, or
`~/.config/dragon-glass/vaults.txt` on Linux (`$XDG_CONFIG_HOME` is respected, and
`DRAGON_GLASS_CONFIG_DIR` overrides all of them).
On launch it reopens the last vault and the note you were last in — stored per vault in
`.dragon-glass/session.txt`. Use **Open vault** in the command palette (`Ctrl+P`, `Cmd+P`
on macOS) to open another vault in its own window, or remove stale entries in the picker.

## Editing shortcuts

| Key | Action |
|---|---|
| `Ctrl+F` | Find in the current note |
| `Ctrl+H` | Find and replace |
| `F3` / `Shift+F3` | Next / previous match |
| `Ctrl+D` | Select the word and then its next occurrence (multi-cursor) |
| `Ctrl+Alt+Up/Down` | Add a cursor on the row above/below |
| `Alt+click` | Add a cursor at the click |
| `Ctrl+Alt+V` | Toggle optional Vim mode |
| `Ctrl+B` | Bold the selection (or insert `****` with the caret inside) |
| `Ctrl+I` | Italicise the selection |
| `Ctrl+Shift+X` | Strikethrough the selection |
| `Ctrl+Shift+H` | Highlight the selection |
| `Ctrl+Shift+C` | Inline code |
| `Ctrl+K` | Start a wikilink from the selection or caret |
| `Ctrl+,` | Settings |

On macOS, use `Cmd` wherever the table says `Ctrl`, except find and replace, which is
`Cmd+Alt+F` because `Cmd+H` hides the app. The menu bar carries the same commands with
their shortcuts.

With text selected, a **Format** bar appears above the status line with buttons for bold,
italic, strikethrough, inline code, highlight, comments and a wikilink. Formatting toggles:
selecting `**bold**` (or just `bold` between markers) and pressing `Ctrl+B` removes them.
Selecting text and typing `[[` wraps it directly as `[[selected text]]`. Pressing `Ctrl+K`
(or the `[[ ]]` button) opens the note picker instead and produces `[[target|selected text]]`;
when the selected text matches the target, the alias is omitted.

Closing the window with unsaved notes offers **Save and quit**, **Quit without saving**
and **Cancel** instead of discarding the edits.

Vim mode is per vault (`.dragon-glass/config.txt`, `vim_mode=true`) and supports
normal/visual/insert modes, counts, the common motions and `d/y/c` operators, `/` search
with `n`/`N`, `:w`, and `:s/…/…/` / `:%s/…/…/g`.

## CLI

`dragon-glass` is the headless companion: useful for scripting, CI and cron.

```bash
dragon-glass stats   /path/to/vault          # counts, words, tags, links
dragon-glass broken  /path/to/vault          # unresolved [[links]]
dragon-glass export  /path/to/vault out/     # static HTML
dragon-glass query   /path/to/vault 'tag:#project'
dragon-glass pdf     /path/to/vault notes/Index.md Index.pdf
dragon-glass git-pull /path/to/vault
dragon-glass --help
```

## Development

```bash
cargo run -p dragonglass-ui -- /path/to/vault   # debug build
DRAGON_GLASS_VAULT=/path/to/vault cargo run -p dragonglass-ui

cargo test                                      # core, CLI and plugin crates
cargo fmt --all -- --check
cargo clippy --all-targets --all-features
```

CI runs exactly those checks on Windows, Linux and macOS, and builds the GPUI app on all
three. See [`CONTRIBUTING.md`](CONTRIBUTING.md) for the ground rules, and
[`PLAN.md`](PLAN.md) for architecture, phases and settled design decisions.

## Project layout

| Path | What lives there |
|---|---|
| `crates/dragonglass-core` | Headless engine: vault/FS, parser, index, search, graph, math, export |
| `crates/dragonglass-ui` | The GPUI desktop application |
| `crates/dragonglass-cli` | `dragon-glass`, the headless CLI |
| `crates/dragonglass-plugins` | WASM plugin host (experimental) |
| `crates/dragonglass-ffi` | UniFFI bindings that expose the engine to mobile clients |
| `mobile/android` | Native Android app (Kotlin + Jetpack Compose) |
| `docs/` | Design notes, such as the math/typesetting spec |
| `packaging/` | Windows (Inno Setup, winget), Linux (deb, AppImage, Flatpak) and macOS (`.app`, DMG) recipes |

## Design principles

- **Plain text, no lock-in.** Everything is `.md` plus small, diffable config files.
- **Local-first.** No network by default, no telemetry, no account.
- **Data safety.** Atomic writes, backups and snapshots; the app never silently loses edits.
- **Idle means idle.** No polling, no background timers; filesystem events and input only.
- **Obsidian compatibility, not ABI compatibility.** Vaults and file formats yes; running
  Obsidian's JS plugins no.
- **Native, not a web page in a frame.** GPUI renders GPU-side and ships as a single binary.

## Mobile

The Android app lives in `mobile/android`. It reuses the headless engine through the
`dragonglass-ffi` crate rather than the GPUI desktop UI. See
[`mobile/android/README.md`](mobile/android/README.md) for the native-library build step
and the current feature slice, and [`docs/ios-port.md`](docs/ios-port.md) for the iOS
feasibility assessment.

## Contributing

Issues and pull requests are welcome. Read [`CONTRIBUTING.md`](CONTRIBUTING.md) first, and
please follow the [Code of Conduct](CODE_OF_CONDUCT.md). Security issues should go through
[`SECURITY.md`](SECURITY.md), not a public issue.

## License

Dragon Glass is free software under the **GNU General Public License v3.0 or later**
(`GPL-3.0-or-later`). See [`LICENSE`](LICENSE).
