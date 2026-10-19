# Dragon Glass — Plan

A native, local-first Obsidian replacement. No Electron. Fast, low-memory, native.

**Status:** Active — Phase 0 (foundations)
**Platforms:** Windows + Linux (priority), macOS (later)
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
| Math | **KaTeX** | `$` / `$$`, native host bridge |
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
- CommonMark + GFM (tables, task lists, strikethrough), footnotes, math via **KaTeX**
- Code-fence syntax highlighting
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

**Organization**
- Bookmarks / starred
- Daily & periodic notes + templates + core variables
- Outline pane
- Tabs, split panes, multiple windows
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
| **0 — Foundations** | 3–5 wk | repo + CI (Windows/Linux) ✅, `core` crate ✅, FS vault model ✅, **GPUI spike ✅** | GPUI builds on Windows ✅ and Linux ✅; editor approach = build our own ✅ |
| **1 — Alpha** | 3–5 mo | rope buffer + undo/redo, editor element (source + live preview), file tree, tabs, quick switcher, full-text search, tags, wikilinks + backlinks, reading mode, KaTeX, **git init/commit/push/pull** | dogfood on your own vault, zero data loss, git round-trip works |
| **2 — Beta** | 3–4 mo | graph view, outline, daily notes, templates, properties, bookmarks, command palette, hotkeys, themes/CSS, embeds, attachments, file recovery, update prompt | feature-complete core |
| **3 — Power** | 3–5 mo | WASM plugins, CLI, URI scheme, multi-window, workspaces, query/Bases views | public beta |
| **4 — Ecosystem** | TBD | plugin/theme store, importers (Notion/Evernote/Roam), export/publish, MCP | ecosystem |
| **5 — macOS** | TBD | cross-compile + platform polish, code signing/notarization | macOS parity |

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
18. **Math/dialect:** KaTeX, callouts, `==highlight==`, refs, note linking all in scope. ✅
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
daily notes + templates, properties display/edit, git sync, KaTeX/dialect still to expand.
Remaining: word wrap, horizontal scroll, richer live preview.

**Phase 2 — complete.** Properties, daily/weekly/monthly notes, templates, outline,
bookmarks, command palette, hotkey editor, theming (light/dark, user themes, snippets),
embeds and attachments (images/PDF/audio, `![[note]]` transclusion, and pasting or pasting a
path to an image to file it in the attachments folder), file-recovery snapshots, graph view
with filters/depth/folder grouping, external-change handling and the update prompt are in.

## 11. What is deliberately not done

- **Phase 5 (macOS).** Not started, and not verifiable from this machine: it needs a Mac for
  signing and notarization. The code is already cross-platform and CI builds the GPUI app on
  Linux and Windows.
- **Community/online theme and plugin store.** The locked decision is "no network by default";
  the local store covers installation from a directory. A network store would contradict that.
- **End-to-end-encrypted sync.** Listed conditionally ("if pursued") and not pursued; git sync
  covers the sync requirement.
- **Mobile.** Explicitly deferred by the locked decisions.
- **Full LaTeX layout.** Math is transliterated to Unicode (commands, scripts, fractions,
  roots, blackboard letters) rather than typeset on a baseline grid; a native TeX layout
  engine is a project of its own.
- **Mermaid diagrams.** Out of scope per the locked decisions.
- **Horizontal scroll.** Moot now that the editor wraps.

**Phase 3 — done except the online store.** Everything listed is in: command palette,
CLI/headless mode, HTML export, query views, graph view, the WASM plugin host (with note
write-back), URIs, workspaces, a **local** package store with user themes, and multi-window.
A community (network) store is deliberately out of scope while the app stays offline-first.

**Phase 4 — done except optional sync.** HTML and PDF export, the read-only local HTTP API,
importers (Evernote/Roam), and an MCP server over stdio are in. End-to-end-encrypted sync
remains optional and unstarted, and Notion import is not implemented (its export is simply
markdown, which drops straight into a vault).

### Phase 1 remainder

Word wrap landed (the editor wraps at spaces, and caret/selection/mouse/vertical movement
follow visual rows). Comments, callouts, `==highlights==` and `$math$` are parsed; callouts,
highlights and math render in the app and in exported HTML. A live preview pane renders the
reading view beside the editor. Open: full LaTeX layout (see §11) and horizontal scroll (moot).

**Phase 5 — not started.** macOS cross-compile, signing/notarization, platform polish.
