# Dragon Glass — Plan

A native, local-first Obsidian replacement. No Electron. Fast, low-memory, native.

**Status:** Active — Phase 0 (foundations)
**Platforms:** Windows + Linux (priority), macOS (in progress)
**Team:** Solo developer
**License:** GPL-3.0-or-later

---

## 1. Problem & positioning

Obsidian is Electron: ~300–500 MB RAM idle, slow cold start, ~200 MB install, non-native
widgets, poor laptop battery life. Dragon Glass targets the same job — a plain-text,
local-first knowledge base — as a **native** application.

**Positioning: compat-first.** Dragon Glass opens an existing Obsidian vault losslessly:
it reads/writes plain `.md`, preserves YAML frontmatter, and never clobbers `.obsidian/`.
A user can point Dragon Glass at their current vault and keep working. Migration friction
is the primary adoption lever.

**What we are not doing:** we do not run Obsidian's JavaScript plugins. Compat here means
**vault + file-format compat**, not plugin-ABI compat. Our own plugin model (see §3) is a
plus, not a v1 requirement.

---

## 2. Scope

**v1 (desktop, Windows + Linux):** open an existing vault, edit markdown in source +
live-preview modes, wikilinks + backlinks, tags, full-text search, quick switcher, graph
view, git-based sync. No data loss on any real vault.

**Explicit non-goals for v1:** own sync service, mobile, plugin marketplace, Canvas,
publish/export, **Mermaid diagrams**.

**Later:** WASM plugin model, query/Bases views, Mermaid, macOS, mobile.

---

## 3. Tech stack

| Layer | Choice | Notes |
|---|---|---|
| Core engine | **Rust** — `dragonglass-core` (headless) | vault/FS, parser, index, link graph, search; testable without UI |
| Text buffer | **rope** (`ropey`) + own undo/cursor | the editor's spine; unit-tested headless |
| Parsing | own line tokenizer (`syntax`) | one token kind per character, for syntax highlighting |
| Search index | on-demand scan (`search`) | no index to build or keep fresh, which is what keeps idle CPU at zero |
| FS watch | `notify` | external-change reconciliation |
| UI | **GPUI** (Zed's framework, Apache-2.0) | GPU-rendered, truly native, single codebase |
| Editor | **Own, on GPUI** | rope buffer + GPUI text shaping + own tokenizer; port Zed's (GPL) editor code where useful |
| Math | **RaTeX** (KaTeX-compatible, pure Rust) + GPUI vector painter | `$` / `$$`, native display list; see `docs/math-typesetting.md` |
| Git | the user's `git` binary | init/status/commit/pull/push; reuses their credentials and SSH config; conflicts surfaced as terminal instructions |
| Plugins | `wasmtime` (WASM sandbox) | sandboxed, language-agnostic — **post-v1** |

**Rejected:** Tauri and any webview stack — still a browser engine, fails the "not
Electron" bar. Mermaid — a JS library; deferred (see §2). **Zed's `editor` crate as a
dependency** — `publish = false` and depends on `client`, `dap`, `rpc`, `project`,
`workspace`, `telemetry`; consuming it means vendoring all of Zed. **`libgit2`/`git2`** —
linking it would duplicate the user's credential helper, SSH agent and merge behaviour;
driving their `git` binary avoids a heavy native dep and behaves exactly as they expect.

**License:** GPL-3.0-or-later. GPUI is Apache-2.0 (compatible). GPL-3.0 also means we may
port code from Zed's GPL editor into our own crates without adopting its dependency graph.

**Architecture:** FS events → incremental tree-sitter parse → in-memory doc cache → index
(Tantivy + link graph) → UI queries. On-disk data stays plain text; the index lives **in
the vault** at `.dragon-glass/` and is a **rebuildable cache** (auto-added to the vault's
`.gitignore`; safe to delete). Crate split:

- `dragonglass-core` — headless engine (library-first, unit-testable)
- `dragonglass-ui` — GPUI application
- `dragonglass-cli` — headless mode / automation
- `dragonglass-plugins` — WASM runtime (post-v1)

---

## 4. Feature list

**Vault / FS**
- Multiple vaults; open/switch
- File tree: create, rename, move, delete, drag-drop
- Atomic autosave + crash safety (temp + rename), backups
- External-change conflict handling (edit-on-disk while open)
- Attachments (images, PDF, audio) + embeds
- File recovery / version snapshots

**Editor**
- Source mode, Live Preview, Reading mode
- Reading mode: character selection across blocks (Ctrl+A/Ctrl+C), clickable wikilinks and
  external links, a metadata panel above the document with clickable frontmatter links,
  title, description and excerpt, footnote references and definitions, line-number gutter in
  the source editor, and a status bar with line/word counts, git added/removed lines and
  selection statistics
- Resizable side panels (file tree, table of contents, relations) with draggable splitters
- CommonMark + GFM (tables, task lists, strikethrough), footnotes, math via **RaTeX**
  (KaTeX-compatible; spec: `docs/math-typesetting.md`)
- Code-fence syntax highlighting in the editor and reading view
- Reading view: padded page layout, GFM tables, highlighted code, inline math in headings,
  tables, quotes and callouts
- Multi-cursor, regex find/replace, vim mode (optional)
- Slash commands; link/tag autocomplete
- IME, RTL, spellcheck, accessibility
- YAML frontmatter / property UI
- Callouts; `==highlight==`; `%%comments%%`; paste/drag image → auto-link to attachments
- Mermaid diagrams: **out of scope for v1**

**Knowledge**
- `[[wikilinks]]`, aliases, heading + block refs (`[[note#heading]]`, `^blockid`)
- Backlinks pane + unlinked mentions
- Outgoing links
- Nested tags + tag pane
- Graph view (global + local), filters/forces/grouping
- Transclusion `![[note]]`

**Search**
- Instant fuzzy quick switcher (files, headings, aliases)
- Full-text with operators (`path:`, `tag:`, `file:`, `line:`, `section:`), regex
- In-file search/replace

**Editorial**
- Block model: move a block up/down (Ctrl+Shift+↑/↓), duplicate it, delete it, or extract it
- A/B versions stored as plain markdown under `.dragon-glass/versions/`; compare with the
  current note or another variant, and promote a variant without losing the old note
- Typed document relations from frontmatter (`up`, `down`, `prev`, `next`, `related`,
  `series`, `supersedes`) plus links, embeds, backlinks and shared tags; a relation-enriched
  graph view
- Shared text blocks (`{{shared:slug}}`) stored under `.dragon-glass/shared/`, expanded in
  reading mode, previews and exports; extract a selection or block into one
- Template discovery and insertion, plus new notes from templates with standard variables
  (`title`, `date`, `time`, `weekday`, `vault`, `folder`, `{{cursor}}`)
- Editorial review: broken links and relation targets, unmatched fences, empty/duplicate/jumping
  headings, unfinished markers and overlong paragraphs, with jump-to-line
- Word count, reading time and paragraph/sentence stats in the header

**Organization**
- Bookmarks / starred
- Daily & periodic notes + templates + core variables
- Table of contents pane
- Tabs, split panes, multiple windows
- Drag and drop tabs: drop on the tab bar or another window to merge, drop on the left/right
  edge of the editor for a side-by-side pane, drop on the top/bottom edge for a stacked pane
  (up to four panes), or drag out to pop the tab into a chrome-free editor-only window with a
  slim draggable title strip so it can be dragged back in; read/write side by side remains a
  palette command
- Workspaces (layout presets)

**Customization**
- Themes (light/dark), CSS variables, snippets — **our own theming** (Obsidian community
  CSS themes target a DOM and cannot run natively)
- Hotkey editor + command palette; **default keybindings match Obsidian**
- Layout customization

**Sync (git-based)**
- Init / connect to a git remote
- Auto-commit with debounce; manual commit
- Pull / push; on conflict, print copy-paste `git` resolution instructions (no native merge UI)
- `.gitignore` defaults for `.dragon-glass/`
- Later: git LFS guidance for large attachments

**Distribution**
- Windows (MSI / winget) + Linux (AppImage / deb / Flatpak)
- **No silent auto-update**: show an in-app "update available" prompt; user downloads

**Extensibility (post-v1)**
- WASM plugin API + manifest, enable/disable
- Theme store (local, then community)
- URI scheme for automation
- CLI / headless mode (`dragon-glass export`, scripting)
- Local HTTP API / MCP for AI tooling

---

## 5. Non-functional requirements

- **Performance:** <50 MB RAM idle, <200 ms cold start, 100k notes searchable.
- **Idle = ~0% CPU.** Event-driven only (FS `notify` events + input); no polling, no
  timers, no background work when not editing and not syncing. Idle documents are unloaded;
  only the mmap'd index stays resident. This is the headline metric where we beat Obsidian.
  (Exact index/search latency budgets to be set in Phase 0.)
- **Data safety:** never lose data; atomic writes + backups; plain text = no lock-in.
- **Security:** no network by default; plugins sandboxed (post-v1).
- **Accessibility:** keyboard-first, screen-reader support.

---

## 6. Roadmap (solo)

Durations are honest solo estimates, not team estimates. Time budget is currently lax —
optimize for shipping working code over schedule precision.

| Phase | Duration | Contents | Milestone |
|---|---|---|---|
| **0 — Foundations** | 3–5 wk | repo + CI (Windows/Linux/macOS) ✅, `core` crate ✅, FS vault model ✅, **GPUI spike ✅** | GPUI builds on Windows ✅ and Linux ✅; macOS CI/bundle in progress; editor approach = build our own ✅ |
| **1 — Alpha** | 3–5 mo | rope buffer + undo/redo, editor element (source + live preview), file tree, tabs, quick switcher, full-text search, tags, wikilinks + backlinks, reading mode, **RaTeX math**, **git init/commit/push/pull** | dogfood on your own vault, zero data loss, git round-trip works |
| **2 — Beta** | 3–4 mo | graph view, outline, daily notes, templates, properties, bookmarks, command palette, hotkeys, themes/CSS, embeds, attachments, file recovery, update prompt | feature-complete core |
| **3 — Power** | 3–5 mo | WASM plugins, CLI, URI scheme, multi-window, workspaces, query/Bases views | public beta |
| **4 — Ecosystem** | TBD | plugin/theme store, importers (Notion/Evernote/Roam), export/publish, MCP | ecosystem |
| **5 — macOS** | in progress | CI + unsigned `.app`/DMG ✅, Cmd+Q save ✅; signing/notarization, Intel/universal, runtime polish | macOS parity |

**Critical path:** Phase 0 → 1. Everything else is gated on a proven native editor.

---

## 7. Risks

- **Native text editing is the hard part** (IME, BiDi, selection, multi-cursor, undo,
  a11y). Mitigation: build on a rope buffer + GPUI text shaping, port Zed's GPL editor
  code for the fiddly parts, and grow it incrementally behind tests.
- **GPUI on Windows+Linux** — validated: builds and opens a window on Windows; builds on
  Linux (Wayland + X11 + Vulkan) in a container. CI covers both.
- **Solo bandwidth** — phases are sequential and cuttable; milestones are shippable.
- **Scope creep** (Canvas, Bases, Dataview, Mermaid) — gated behind v1.
- **Git on large vaults** (attachments, big repos) — debounced commits + LFS guidance;
  never auto-force push.
- **No plugin ABI parity** — accepted by design; positioned as compat-first, not
  plugin-compatible.
- **~0% idle CPU is a fragile promise** — one stray timer or poll breaks it. Mitigation:
  enforce with an idle-CPU regression test in CI from Phase 0.

---

## 8. Decisions (resolved)

1. **UI stack:** GPUI. ✅
2. **Positioning:** compat-first — lossless Obsidian vault round-trip. ✅
3. **Obsidian JS plugin compat:** not required; own plugins are a plus (post-v1). ✅
4. **Platforms:** Windows + Linux priority; no mobile; macOS later. ✅
5. **Team:** solo. ✅
6. **Sync:** git-based; drive the user's `git` binary (their credentials, SSH and merge
   behaviour). ✅
7. **Name:** Dragon Glass. ✅
8. **Idle behavior:** beat Obsidian — ~0% CPU idle, event-driven, unload idle docs. ✅
9. **Git conflicts:** no native merge UI; print copy-paste resolution instructions. ✅
10. **Index location:** in-vault `.dragon-glass/`, git-ignored. ✅
11. **Plugin substrate:** WASM, language-agnostic (JS, Python, Rust — anything targeting
    WASM); shell/`bash` scripts run via the CLI/automation escape hatch. ✅
12. **License:** GPL-3.0-or-later (open source). ✅
13. **Editor:** build our own on GPUI (rope buffer, cosmic-text, tree-sitter); port Zed's
    GPL editor code where useful — its `editor` crate is not library-usable. ✅
14. **Keybindings:** match Obsidian defaults. ✅
15. **Theming:** our own; Obsidian community CSS themes not supported. ✅
16. **Updates:** no auto-update; in-app "update available" prompt. ✅
17. **Diagrams:** Mermaid out of scope for v1. ✅
18. **Math/dialect:** KaTeX-compatible math via **RaTeX** (native, no JS), callouts, `==highlight==`, refs, note linking all in scope. ✅ Spec: `docs/math-typesetting.md`.
19. **Test vault:** local copy of `Projects\Docs` at `test-vault/` (git-ignored). ✅
20. **Time budget:** lax; prioritize shipping code. ✅

## 9. Open items

- Editor element API (how GPUI widgets wrap the buffer) — Phase 1 design output.
- Distribution format specifics (winget vs. MSI; AppImage/deb/Flatpak combo) — Phase 2+.
- Plugin host API surface once we reach Phase 3 (WASM engine decided; API not).

### Pinned dependencies

- **`libc` is pinned to `=0.2.188`** in `Cargo.lock`. Zed's `zed-async-tar` fork depends on
  `xattr 0.2.3`, which references `libc::ENOATTR`; libc removed that constant for Linux in
  0.2.189+. `rustix` requires `>= 0.2.182`, so 0.2.188 is the only valid window. Drop the
  pin once Zed updates its `async-tar` fork. Linux builds fail loudly if it moves.

---

## 10. Progress

**Phase 0 — complete.** Workspace, CI (Windows + Linux, including the GPUI build), core
engine, CLI, GPUI validated on both platforms.

**Phase 1 — complete except two items.** Vault open/round-trip, file tree, rope buffer with
undo/redo, in-house GPUI editor (caret, mouse placement, drag selection, shift-extend,
copy/cut/paste/select-all, IME), atomic save + dirty indicator + autosave on switch,
reading mode, quick switcher, full-text search, tags, wikilinks/backlinks data, outline pane,
daily notes + templates, properties display/edit, git sync, math/dialect still to expand.
Remaining: word wrap, horizontal scroll, richer live preview.

**Phase 2 — complete.** Properties, daily/weekly/monthly notes, templates, outline,
bookmarks, command palette, hotkey editor, theming (light/dark, user themes, snippets),
embeds and attachments (images/PDF/audio, `![[note]]` transclusion, and dragging or pasting
an image — or a path to one — to file it in the attachments folder), file-recovery snapshots,
graph view with filters/depth/folder grouping and dashed relation edges, external-change
handling and the update prompt are in. Attachment embeds resolve by file name anywhere in the
vault, the way Obsidian does.

## 11. What is deliberately not done

- **Phase 5 (macOS).** In progress and verified through CI. The macOS build, unsigned
  `.app`/DMG bundler and launch smoke test pass on `macos-latest`, and `Info.plist` declares
  the URL scheme and Markdown document types. Cmd+Q saves dirty buffers through an app-quit
  observer. Signing/notarization, Intel/universal builds,
  the native app menu and runtime testing (IME, VoiceOver, trackpad, Metal) still require a
  Mac and an Apple Developer ID.
- **Community/online theme and plugin store.** The locked decision is "no network by default";
  the local store covers installation from a directory. A network store would contradict that.
- **End-to-end-encrypted sync.** Listed conditionally ("if pursued") and not pursued; git sync
  covers the sync requirement.
- **Packaging.** In place: `register-uri`/`unregister-uri`, a release profile, a Windows icon
  and VERSIONINFO resource embedded at build time, a `.deb` builder, Inno Setup and winget
  manifests, Flatpak and AppImage recipes, a macOS `.app`/DMG bundler, and a tag-triggered
  release workflow (macOS artifacts are currently unsigned).
- **Mobile.** Originally deferred. An Android shell now exists in `mobile/android` on top of
  the UniFFI bridge in `crates/dragonglass-ffi`; iOS can reuse the same bridge with Swift
  bindings. The first slice edits app-private vaults; SAF/USB-document access is next.
- **Full LaTeX layout.** Implemented per **`docs/math-typesetting.md`**: RaTeX
  (KaTeX-compatible, pure Rust) lays formulas out into a display list that a GPUI vector
  painter draws. Display math is fully typeset (fractions, radicals, limits, matrices,
  stretchy delimiters, `aligned`); inline math is typeset by a measured flow element (shaped
  text + atomic boxes, baseline-aligned, selectable across formulas); HTML export emits
  self-contained SVG and PDF export draws display and inline formulas as vector paths.
  Hardening covers corpus parsing, truncation fuzz, geometry/SVG goldens, a typesetting
  budget, and KaTeX reference-image ink overlap on seven fixtures. Hovering a formula shows
  its TeX source, and copying a selection preserves `$…$` TeX source.
- **Mermaid diagrams.** Out of scope per the locked decisions.
- **Horizontal scroll.** Moot now that the editor wraps.

**Phase 3 — done except the online store.** Everything listed is in: command palette,
CLI/headless mode, HTML export, query views, graph view, the WASM plugin host (with note
write-back), URIs, workspaces, a **local** package store with user themes, and multi-window.
A community (network) store is deliberately out of scope while the app stays offline-first.

**Phase 4 — done except optional sync.** HTML and PDF export, the read-only local HTTP API,
importers (Evernote/Roam), and an MCP server over stdio are in. HTML export copies every
referenced attachment next to the pages, preserving vault-relative paths. End-to-end-encrypted
sync remains optional and unstarted, and Notion import is not implemented (its export is
simply markdown, which drops straight into a vault).

### Vaults

Launching the app with no argument now reopens the last vault instead of assuming
`test-vault` happened to be the working directory. The list of vaults — most recent first —
lives outside any vault (`%APPDATA%\DragonGlass\vaults.txt`, `~/.config/dragon-glass/vaults.txt`,
or `$DRAGON_GLASS_CONFIG_DIR` when set). On a first run, or when the last vault has moved, a
chooser lists the recent vaults and offers a native folder picker; hovering a row highlights
it, and stale entries can be removed without touching the folder. The active note and reading
mode are remembered per vault in `.dragon-glass/session.txt`, so the app reopens the note you
were last in. "Open vault" in the command palette opens the chooser while the app is running;
the chosen vault starts in its own process so each window keeps its vault, theme and keymap.
An explicit path (a shortcut or a `dragonglass://` URI) still wins over the remembered vault.

### Phase 1 remainder

Word wrap landed (the editor wraps at spaces, and caret/selection/mouse/vertical movement
follow visual rows). Comments, callouts, `==highlights==` and `$math$` are parsed; callouts,
highlights and math render in the app and in exported HTML. A live preview pane renders the
reading view beside the editor. Reading mode now has page padding, GFM tables, math in
headings/table cells/quotes/callouts, a code highlighter with the fence language and line
count shown above each block, drag selection with Ctrl+C, and clickable links. The source
editor has a line-number gutter and a status bar with line/word counts, per-file git
added/removed lines, and selection statistics; the side panels resize by dragging their
splitters. Tabs can be moved into real windows, shown two-up, or paired with the reading
view. Open: full LaTeX layout (spec: `docs/math-typesetting.md`); horizontal scroll is
moot in the editor but code blocks scroll horizontally.

**Typing latency.** The editor shapes text per logical line and caches the result, so a
keystroke or a newline only re-shapes the line that changed (line insertions and deletions
shift the cached tail instead of dropping it). Before this, every keystroke re-measured and
re-shaped the whole note — noticeable on the 1,000+ line notes in the test vault. The view
also follows the caret, and `Ctrl+Z` / `Ctrl+Shift+Z` / `Ctrl+Y` undo and redo.

**Phase 5 — in progress.** macOS is in the CI matrix and releases get an unsigned `.app`
and DMG from `packaging/macos/`. The `macos-bundle` job builds and launch-tests the app on
`macos-latest`; the first full run produced a runnable arm64 bundle and DMG. The bundle declares the `dragonglass://` URL scheme
and Markdown document types, Cmd+Q is wired to an app-quit save so unsaved tabs are not
dropped, the vault registry already uses `~/Library/Application Support/DragonGlass`, and
recent-vault deduplication is case-insensitive on macOS as well as Windows. Still required:
Developer ID signing and notarization, Intel/universal binaries, a native app menu and
runtime validation of IME, VoiceOver, trackpad gestures and Metal on a Mac.

### Editor tooling

The editor now has an in-note find/replace bar (`Ctrl+F`, `Ctrl+H` for replace) with
literal or regex patterns, case-sensitive and whole-word toggles, live match counts,
invalid-pattern feedback, per-match replacement and replace-all. Regex replacements support
`$1`/`${name}` capture expansion; literal replacements are literal (including `$`). Search
and replacement operate on character offsets, so emoji and accented text never split.

Multi-cursor editing is built into the editor rather than bolted onto the buffer: `Ctrl+D`
selects the word under the caret and then each next occurrence, `Ctrl+Alt+Up/Down` adds a
caret on the visual row above or below, and `Alt+click` places extra carets. Typing,
Enter, Backspace/Delete, paste, cut/copy and selections apply at every caret, and all edits
in one keystroke are a single undo step (`Buffer::replace_many`).

Autocomplete works while typing: `[[` lists vault notes, `#` lists tags used anywhere in the
vault, and a line that starts with `/` lists slash commands (headings, lists, tasks, tables,
code, math, callouts). `Up`/`Down` and `Enter` choose, `Esc` dismisses.

Selecting text reveals a **Format** bar above the status line. Bold (`Ctrl+B`), italic
(`Ctrl+I`), strikethrough (`Ctrl+Shift+X`), highlight (`Ctrl+Shift+H`) and inline code
(`Ctrl+Shift+C`) wrap the selection, and pressing the same shortcut again unwraps it.
Comments and wikilinks are on the bar. `Ctrl+K` starts a wikilink from the caret, or from the
selection: picking a note writes `[[target|selected text]]` (or `[[target]]` when the text
already matches). Typing `[[` over a selection is the direct route: it wraps the selection
as `[[selected text]]` without opening the picker.

**Settings** (`Ctrl+,`) is an in-app page for the theme, accent colour, editor font size, Vim
mode, snippets and the daily/weekly/monthly/templates/attachments folders. Values are written
to `.dragon-glass/config.txt`, so they remain plain text and per vault.

Vim mode is optional and per vault (`vim_mode=true` in `.dragon-glass/config.txt`, the
command palette, or `Ctrl+Alt+V`). It covers normal/visual/visual-line/insert modes,
counts, `h j k l w b e 0 ^ $ gg G`, `i a I A o O`, `x X s S D C J`, `d/y/c` operators,
`p/P`, `u`, `r`, `/` search with `n/N`, and `:w`, `:wq`, `:N`, `:s/…/…/`, `:%s/…/…/g`.

### Editorial system

A headless editorial layer sits in `dragonglass-core` and the GPUI app exposes it through the
command palette and a few shortcuts:

- `blocks` — parses headings, paragraphs, lists, code, quotes, callouts, math, rules and
  embeds into movable line ranges; operations are pure text transforms, so they are unit
  tested and keep the file plain markdown.
- `diff` — deterministic line diff with old/new line numbers, used by version comparison.
- `versions` — A/B (and named) variants under `.dragon-glass/versions/<note-key>/`; promoting
  snapshots the current note first, so it is reversible through normal history.
- `relations` — typed relations from links, embeds, frontmatter keys and shared tags, with
  reverse edges; also produces a link graph enriched with frontmatter relations. The graph
  view paints those relation edges dashed and tinted, and review flags relation keys that
  point at notes that do not exist.
- `assets` — a light index of the vault's files, so `![[shot.png]]` resolves by name anywhere
  in the vault for the reading view, the detached preview and HTML export.
- `resolve` — one link resolver shared by the graph, relations, review, reading view and
  exports: paths, stems, frontmatter `Name`/`Slug`/aliases, and trailing-path matches, so
  `[[kebab-case-slug]]` and `/assets/...` embeds find their files.
- `shared` — `{{shared:slug}}` single-sourced blocks with recursive expansion and cycle
  guards, expanded by reading mode, the detached preview, HTML export and PDF export.
- `lint` — the review rules behind the "Review document" command.
- `stats` — word/sentence/paragraph counts and reading time.
- `templates` — template discovery, insertion and the `{{cursor}}` marker.
- `math` — LaTeX handling: Unicode transliteration today; the RaTeX display-list pipeline
  (reading view, inline flow, HTML/PDF export) is specified in
  **`docs/math-typesetting.md`**.

UI entry points: Ctrl+Shift+R toggles the document-relations pane; Ctrl+Shift+↑/↓ moves a
block; the command palette has versions, templates, shared blocks (insert, extract, edit),
review, relations, the relation graph, block duplicate/delete, "reload note from disk" and
"add related note". Compare output renders as a line-numbered diff until Escape.

### Data safety

Closing a window with dirty tabs now shows a **Save and quit / Quit without saving / Cancel**
bar instead of dropping the edits; the same save runs from **Save all open notes** in Settings.
Deleting from the file tree moves the note or folder to `.dragon-glass/trash/<stamp>/`, closing
its tabs first (and refusing to continue if a dirty tab cannot be saved). Renaming or moving
keeps open tabs pointed at the new path. **File recovery** in the palette lists the snapshots
that are written whenever a note is overwritten, with a diff against the current buffer and a
one-click restore that snapshots the current buffer first.
