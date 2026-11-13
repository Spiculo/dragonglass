# Dragon Glass QOL plan: editing and reading flows

**Status:** Proposal — November 2026
**Applies to:** desktop (Windows, Linux, macOS), Android vertical slice, planned iOS client
**Focus:** the two loops users repeat all day — **edit a note** and **read a note** — and the
transitions between them.

> **Landed so far (quick wins, §10):** the reading-mode mutation guard (Q0.1), find/replace
> keyboard bindings and `find.message` (Q3.1), `Ctrl/Cmd+Shift+S` plus Save As from `Ctrl+S`
> on an untitled note (Q3.3), typed/expiring status messages (Q3.4), a conflict marker on
> every affected tab (Q3.5), undo-close with `Ctrl/Cmd+Shift+T` (Q3.6), Android IME insets and
> caret-safe keyboard options (Q0.6), Android preview `WebViewClient` links and Back from
> Preview to Edit (Q0.5/Q1.8), Android search clear/IME-Search/Back-to-clear (Q4.1), joined
> soft line breaks (Q2.10), clickable tag chips and frontmatter links (Q4.5/P2), the window
> title as note + dirty marker (Q3.12), Vim `:wq` close, redo and failed-search reporting
> (Q3.12), the code-fence copy button (Q2.7), the Escape order audit, and the first slice of
> live preview: `$$…$$` blocks typeset in the editor while the caret is outside them
> (Q2.12). Reading selection now has double/triple-click, Shift-click, keyboard extension,
> drag auto-scroll and a copy confirmation (Q2.3). Everything else in this document is still
> open. The Android changes compile with `:app:compileDebugKotlin` but have not been run on a
> device yet.

**Basis:** audit of the current working tree (commit `1cb0ec2` plus local session/mobile
work). Line references are approximate and should be re-checked before implementation.

---

## 1. The thesis

Dragon Glass already wins on speed, memory, local-first storage and format compatibility.
The next differentiator is not another feature; it is **removing the moments where the app
makes the user think about the app**:

- "Did that save?"
- "Where was I?"
- "Why did following that link take me to edit mode?"
- "Why did my scroll position reset?"
- "Did I just type into a read-only view?"
- "Which copy won?"

QOL here means **flow integrity**, not cosmetics. The target is an app where a user can open
a note, read, follow three links, come back, edit a paragraph, close the laptop, and reopen
to exactly the same place — without a single confirmation dialog they did not need.

### What "done" looks like

> After two weeks of daily use, a user should never lose text, never lose their place, and
> never wonder what a click or a save did.

### The ten moves that matter most

1. Make reading mode genuinely read-only (Q0.1).
2. Gate every save on an external-change fingerprint; never overwrite silently (Q0.2).
3. Add drafts/autosave and crash recovery on desktop and Android (Q0.3).
4. Make close/quit/background one explicit, failure-safe contract (Q0.4).
5. Remember cursor, scroll and mode per tab, and map edit position to read position (Q1.1, Q1.2).
6. Add back/forward navigation and make `#heading`/`^block` links work (Q1.3, Q1.4).
7. Keep the mode when following a link; open search hits at the hit line (Q1.5, Q1.6).
8. Give reading its own keyboard model, in-page find and selection/copy behavior (Q2.1–Q2.3).
9. Make find/replace, Tab indent, Save As and feedback keyboard-complete (Q3.1–Q3.4).
10. On Android: IME insets, preview link handling, Back hierarchy and scroll retention (Q0.5–Q0.7, Q1.7, Q1.8).

---

## 2. Non-negotiable flow invariants

Every client (desktop, Android, iOS) should satisfy these. They are the contract to test
against.

1. **No silent text loss.** A save either succeeds visibly or stops with an actionable
   conflict/error. No code path writes over a file that changed underneath the editor
   without asking.
2. **No silent text mutation.** Reading mode never edits the source. Keyboard focus in a
   reader scrolls and selects, never changes bytes.
3. **No lost place.** Per tab, the client remembers mode, cursor/selection, edit scroll and
   reading scroll. Quit/relaunch, tab switch, mode switch and external reload preserve them.
4. **Reading and editing are continuous.** Entering reading starts at the block containing
   the caret; leaving reading restores the source line for that block. Following a link
   keeps the current mode.
5. **Links are complete.** `[[Note]]`, `[[Note#Heading]]`, `[[Note#^block]]` and `[[#Local]]`
   navigate to the target, not the top of the file. Every navigation has a back path.
6. **Leaving is reversible or explicit.** Closing a dirty tab/note offers Save / Discard /
   Cancel (or an immediately undoable auto-save). Quit cannot silently drop a failed save.
7. **Every action reports back.** Save, sync, export, conflict, failed link and failed paste
   produce a visible, correctly typed result (info/warn/error), not a stale status line.
8. **No incidental layout jump.** Opening find, autocomplete, the format bar or a toast does
   not move the document under the caret.

**Performance guardrail:** autosave/debounce must be event-driven and stop when the buffer
is clean. Idle CPU stays at zero; no polling, no background timers while nothing is dirty.

---

## 3. Where the flows break today

### 3.1 Editing loop

| Stage | Intended experience | Current friction | Evidence |
|---|---|---|---|
| Resume | Reopen tabs exactly where left | Tabs/layout restore, but cursor, scroll and per-tab mode do not | `core/src/session.rs:15-35`, `ui/src/editor.rs:783-803` |
| Orient | See title, dirty state, mode | Dirty is a boolean that can lie; the main window title is fixed to "Dragon Glass" | `core/src/buffer.rs:100-111`, `ui/src/main.rs:6790` |
| Edit | Fast, keyboard-complete | Tab does nothing; find/replace actions are unbound; replace feedback is discarded | `ui/src/editor.rs:104-188`, `1755`, `2693-2722`, `2856-2869` |
| Save | Save is obvious and trustworthy | `on_save` discards the `Result`; Untitled fails with a hidden message; no autosave/draft | `ui/src/editor.rs:2456-2459`, `806-831` |
| Conflict | Detect external edits, choose | `Tab.conflict` exists but `save()` never checks it; Ctrl+S overwrites blindly | `ui/src/main.rs:336-342`, `editor.rs:806-831` |
| Leave | Close/quit is safe | Dirty close auto-saves with no choice; macOS quit-saver ignores errors; quit can drop Untitled notes | `ui/src/main.rs:1315-1326`, `1134-1141`, `1170-1182` |
| Return | Reopen closed tab / recover | No reopen-closed-tab; trash list/restore helpers have no UI | `core/src/vaultfs.rs:189-226` |
| Feedback | Know what happened | Status text never expires, errors look identical to info | `ui/src/main.rs:1195`, `6224-6228` |

### 3.2 Reading loop

| Stage | Intended experience | Current friction | Evidence |
|---|---|---|---|
| Enter | Read from where I was editing | Edit scroll and reading `ListState` are unrelated; reading usually starts at top | `ui/src/editor.rs:290`, `reading.rs:332`, `398-467` |
| Read | Read-only, keyboard-navigable | Typing/Backspace/Enter/Paste still mutate the hidden buffer; arrows move the hidden caret | `ui/src/editor.rs:1945`, `2274-2314`, `3337-3403` |
| Follow links | Stay in the reader, land on the section | Link click opens edit mode; `#heading`/`^block` fragments are stripped and discarded | `ui/src/main.rs:577-613`, `core/src/inline.rs:303-308` |
| Return | Go back to the previous note/section | No back/forward navigation stack; user hunts tabs or the sidebar | no equivalent in `ui/src/main.rs` |
| Search | Find within the page | `Ctrl+F` forces edit mode; no reading find bar | `ui/src/editor.rs:2536-2539` |
| Select/copy | Select naturally and copy | Mouse-drag only; no double/triple-click, Shift-click, keyboard selection or Escape clear | `ui/src/reading.rs:1781-1879` |
| Media | Read attachments in place | PDF/audio show "attachment — not previewed"; inline images become links; widths ignored | `ui/src/reading.rs:1575-1629`, `core/src/markdown.rs:307-333` |
| Polish | Comfortable typography | Body/width hardcoded; no reading zoom, no zen mode, no code-copy | `ui/src/reading.rs:666`, `766-814`, `1320-1424` |

### 3.3 Mobile loop (Android now, iOS later)

| Stage | Intended experience | Current friction | Evidence |
|---|---|---|---|
| Launch | Reopen last note exactly | Note + preview flag restore, but scroll and editor state do not | `DragonGlassViewModel.kt:167-212`, `243-266` |
| Edit | Keyboard-safe editing | No `imePadding`/IME insets; keyboard can cover caret and status bar | `MainActivity.kt:15`, `Screens.kt:656-685` |
| Save | Save is reliable | Save failure is swallowed and the note is cleared; no lifecycle/autosave; process death loses edits | `DragonGlassViewModel.kt:527-543`, `545-548` |
| Read | Preview links work | WebView has no `WebViewClient`; `.html` links fail or navigate away | `Screens.kt:1651-1682` |
| Back | Predictable hierarchy | Back from Preview closes the note instead of returning to Edit | `Screens.kt:574`, `601` |
| Browse | Return to the same list position | Swapping list/editor destroys `LazyColumn` state | `Screens.kt:110-116`, `389-425` |
| Sync | Know sync state | Sync only inside list menu; no chip, last-sync, retry or editor entry point | `Screens.kt:275-283`, `920-1119` |

---

## 4. Backlog

Tags: **D** desktop · **A** Android · **i** iOS (planned) · **S** shared core/FFI
Effort: S = <1 day · M = 1–3 days · L = 3–7 days · XL = >1 week

### P0 — safety and blocking issues (do first)

#### Q0.1 Reading mode is truly read-only — D · S–M
Guard every mutating action (`backspace`, `delete`, `enter`, `insert_text`, `paste`,
`cut`, `undo`, `redo`, formatting, block ops) and both `EntityInputHandler` insert paths
while `reading` is true. Prefer a single `can_mutate()` gate plus a read-only input handler,
mirroring the existing guard in `apply_style` (`editor.rs:523`).
**Done when:** a fuzz test of all editing shortcuts in reading mode leaves the source hash
unchanged; copy/select still work.

#### Q0.2 Conflict-safe save — D · A · S · M–L
Add a shared save contract in core: `DocumentFingerprint { mtime, size, content_hash }`,
captured on load/save, and `SaveReport { Saved, Conflict, Error }`. Save with an expected
fingerprint; if the disk differs, do **not** write. Desktop shows an inline
Reload / Compare / Keep mine bar; Android shows the same choice in a sheet. Reuse
`conflict::classify`, `diff::lines` and `history::record`.
**Done when:** an external edit followed by Ctrl+S never overwrites silently; the user can
see both versions and choose; a test covers clean reload, dirty conflict and deleted file.

**Landed (desktop + core):** `core::fingerprint` has `DocumentFingerprint` (FNV-1a content
hash, size and mtime as hints) and `SaveReport`, and `save_if_unchanged` refuses to write
when the on-disk hash moved, when the file was deleted, or when it cannot be read. The editor
captures a fingerprint on load/reload/save, `Ctrl+S` reports the conflict, and a bar offers
Reload / Keep mine / Compare (line diff in the existing diff view) / Save as copy. Close,
quit and Save all stop on a conflict instead of dropping or overwriting the buffer. Tests
cover clean save, external edit, deleted file, mtime-only change and write errors.
**Remaining:** the Android sheet (Q0.7 covers the save-before-navigate part).

#### Q0.3 Drafts, autosave and crash recovery — D · A · S · L
Persist a debounced draft under `.dragon-glass/drafts/` (path, base fingerprint, timestamp,
text, cursor). Autosave the note after idle only when the fingerprint still matches;
otherwise stop at the conflict bar. On launch, if a draft is newer than the note, offer
Recover / Compare / Discard. Keep drafts out of git (`.dragon-glass/` is already ignored).
**Done when:** killing the process 2 seconds after typing recovers every character; a
clean buffer writes no timer and no disk I/O; a conflicted buffer never auto-overwrites.

**Landed (desktop + core):** `core::drafts` stores one human-readable file per note
(metadata header, `---`, then the text verbatim), capped at 64 drafts, keyed by a stable hash
of the note path. The editor arms a trailing 1.5 s timer only while the buffer is dirty, and
writes the draft with its cursor and base content hash; a clean buffer arms nothing. A
successful save deletes the draft. Opening a note (or a restored tab) with a draft that
differs from the file shows a bar with Recover / Compare / Discard; Recover applies the text
as one undoable edit.
Note autosave is implemented behind `autosave=on` in `config.txt` (Settings → Autosave
notes): after the same idle, if the fingerprint still matches it saves and drops the draft,
and on a conflict it pauses and raises the conflict bar. It ships **off** by default rather
than the on-by-default recommendation in §8.1, because silent writes interact with git-sync
commits; that is the one deviation, and it is a per-vault toggle.
**Remaining:** Android drafts (Q0.7 covers the save-before-navigate part).

#### Q0.4 One exit contract for close/quit/background — D · A · M
Desktop: dirty close and quit must use Save / Discard / Cancel (or auto-save + an
immediately visible Undo). Fix the macOS quit-saver to respect save failures and keep the
app open on error. Android: observe `ProcessLifecycleOwner`/`onStop`, flush drafts, and do
not clear `selected/source` until the save has actually succeeded.
**Done when:** disk-full / read-only / conflict each leave the editor open with a Retry
path; no exit path can silently drop a dirty buffer; tests cover close, quit and background.

**Landed (desktop):** Cmd+Q now goes through the same `request_close` gate as the window
button: Save and quit / Quit without saving / Cancel, and a failed save keeps the app open
with the error and the conflict bar instead of quitting. The macOS
`applicationWillTerminate` observer remains only for system-initiated termination (logout,
shutdown), where no window exists to ask. Window close and Cmd+Q share one code path.
**Remaining:** Android lifecycle (Q0.7 covers the save-before-navigate half).

#### Q0.5 Mobile preview is a real navigation surface — A · M
Attach a `WebViewClient`: resolve internal `.html` links to `vm.openNote`, send external
links to `ACTION_VIEW`, and block in-place navigation for local files. Preserve the current
mode when a link opens a note. Back from Preview returns to Edit; Back from Edit follows
the normal save/close contract.
**Done when:** tapping `[[Note]]` in preview opens that note; external links leave the app;
Back never skips a level; the preview does not reload/reset on every recomposition.

#### Q0.6 Mobile keyboard never covers the caret — A · S
Apply `imePadding()`/`WindowInsets.ime` to the editor scaffold, bring the caret into view
on focus and selection changes, and set markdown-friendly `KeyboardOptions`
(no autocorrect/smart quotes). Keep the status bar above the keyboard.
**Done when:** on a 360×640 phone and a tablet, the caret and save state are visible with
the IME open at the start, middle and end of a long note.

#### Q0.7 Mobile save failure blocks navigation — A · S
`closeNote`, `switchVault`, `syncVault`, import/export and git sync must await
`saveDirtyNote()` and stop on failure. Show Retry / Discard / Cancel; do not clear state
before the write completes.
**Done when:** a read-only or full vault leaves the note open with its text intact and a
retry action; a fast reopen cannot race a pending save.

**Landed:** `closeNote` and `switchVault` await the save and keep the note (and its text)
open with an error when it fails; import, export and folder sync already aborted on a failed
save; git sync now saves the open buffer first and refuses to sync or clear the editor when
that save fails. `:app:compileDebugKotlin` passes.
**Remaining:** a Retry/Discard/Cancel sheet (today the snackbar carries the error and the
note stays open); lifecycle draft flush.

---

### P1 — flow continuity (the daily "where was I?" fixes)

#### Q1.1 Session v2: per-tab position and mode — D · A · S · M
Extend `Session` beyond tabs/layout (the local work already added tabs and widths) with,
per tab: cursor/selection, edit scroll offset, reading scroll offset, reading mode, and
outline/reading folds. Keep the file human-readable, versioned (`session=2`) and tolerant of
unknown keys. Cap state per tab and prune tabs that vanished.
**Done when:** open 3 tabs at different positions/modes, quit, relaunch, and every tab is
within one line/one block/16 px of its previous position.

**Landed:** `session.txt` is versioned and human-readable (`tab=note|reading|line|col|scroll|block`),
tolerant of unknown keys and malformed fields, capped at 32 tabs, and version 1 files still
load. Each tab round-trips its mode, caret line and column, editor scroll offset and reading
top block; startup reopens every saved tab whose file still exists, prunes missing files and
restores the active tab. Tests cover round-trip, untitled tabs, v1 migration, malformed
lines, caps and the restore filter.
**Remaining:** selection ranges and fold state are not stored yet.

#### Q1.2 Map edit position to read position (and back) — D · M
Add source line spans to the block parser (`markdown::parse_blocks` returns blocks plus
spans, without breaking the existing enum). On entering reading, reveal the block that
contains the caret line (or the nearest preceding heading). On leaving, move the caret to
the first line of the top visible block and keep the edit scroll near it.
**Done when:** place the caret in the middle of a long note, press Ctrl+E, and see that
paragraph; press Ctrl+E again and return to the same source area — repeated 20 times with
no drift.

#### Q1.3 Back/forward navigation stack — D · A · M
Maintain `(note, mode, anchor/scroll)` entries for link clicks, quick-switch, search hits,
outline jumps and daily notes. Bind Alt+Left/Right and mouse back/forward on desktop; expose
Back in mobile chrome without overloading "close note". Restore the exact reading block.
**Done when:** read A → link B#H → link C → Back returns B at H → Back returns A at the
original block; forward replays the path.

**Landed (desktop):** `NavBack`/`NavForward` (Ctrl/Cmd+`[` / `]`, in the hotkey editor)
walk a 100-entry history of `(note, mode, cursor line, reading block)` recorded on every
note open — links, quick switcher, search hits, daily notes, reopen — and on back/forward
restore the mode, caret and reading position. Visiting a new place after going back drops
the forward tail; a status line shows the position in the history.
**Remaining:** mouse back/forward buttons, the outline-jump entries, and the mobile Back
chrome.

#### Q1.4 Heading and block fragments — D · M
Keep the `#fragment` in parsed inline links instead of stripping it (`inline.rs`), parse
`^block-id` markers, and resolve after loading: heading → existing `scroll_to_heading_index`
path; block → a block-id index with a source-line fallback. Same-note `[[#Heading]]` must
scroll without a reload.
**Done when:** `[[Note#Heading]]`, `[[#Local]]` and `[[Note#^block]]` all land on the target
in edit and reading mode; missing fragments show a clear message instead of doing nothing.

#### Q1.5 Opening a link keeps the mode — D · S
Carry "opened from reader" (and the source tab's reading flag) into `open_note`; remember
mode per tab instead of only restoring the active tab at launch. External URLs and assets
open in the OS, not as bogus notes.
**Done when:** reading a note and clicking a wikilink opens the target in reading mode at
the right position; quick-switch from reading remains in reading; edit mode remains edit.

**Landed:** `open_note` reads the current tab's mode and applies it to the editor it loads
into, so following a wikilink or quick-switching from reading stays in reading, and edit
stays edit. External URLs and assets already go to the OS.

#### Q1.6 Search hits open at the hit line — D · A · S
Preserve `hit.line_number` through `Item::Note` (desktop) and `pendingLine` (mobile), focus
the editor, select the match and scroll it into view. Show the matching line/context in the
result list instead of only the title.
**Done when:** every search result opens with the hit visible and selected; Back returns to
the results with the query and scroll intact.

**Landed (desktop):** search results carry the line number (`Item::NoteAt`), list the line
text next to the path, and opening one focuses the editor with the query text selected on
that line; the caret-follow scroll brings it into view. Mobile already passed `pendingLine`;
its results list shows the hit text.

#### Q1.7 Mobile scroll retention — A · M
Remember the note-list `LazyListState`, editor selection/scroll and WebView scroll position
per vault/session. Keep the list composed (or hoist state) instead of destroying it when a
note opens. Preserve preview scroll across Edit ↔ Preview.
**Done when:** open a note from row 80, read to the bottom, press Back, and return to row
80; toggle Preview and return to the same paragraph.

#### Q1.8 Mobile back precedence — A · S
Define one hierarchy: Preview → Edit; search results → list; sheet/dialog → dismiss; dirty
note → save/discard prompt; clean note → list; list with a query → clear query; otherwise
exit. Use one `BackHandler` per level, not competing handlers.
**Done when:** every Back press is predictable and no screen is skipped; a scripted
back-through-the-app test passes.

#### Q1.9 Outline follows the reader — D · S
Highlight the current heading while scrolling, scroll the outline to it, and make outline
jumps reveal headings hidden by folds. Show a compact section/progress indicator in the
reading status bar.
**Done when:** scrolling a long note updates the active outline row; jumping to every
heading (including folded ones) visibly moves the page.

**Landed:** the reading view reports its nearest heading and progress; the outline row for
that section is highlighted (and the reading status bar shows `Section · 42%` instead of
Ln/Col). An outline jump first unfolds every ancestor of the target, so a heading hidden by
a folded parent is visible when the page moves.
**Remaining:** scrolling the outline panel itself to keep the active row in view.

---

### P1 — reading craft

#### Q2.1 Keyboard model for reading — D · M
When reading, route Up/Down, PageUp/PageDown, Home/End, Space/Shift+Space, `j/k`, `g/G` to
`ListState` scrolling; Enter activates the focused link; Escape clears selection; Ctrl+F
opens in-page find. Never move the hidden edit caret.
**Done when:** a keyboard-only user can read an entire long note, follow links and return
without touching the mouse; the source hash stays unchanged.

**Landed:** in reading mode Up/Down and Ctrl+Up/Down scroll a block, Home/End and
Ctrl+Home/End jump to the top/bottom, Left/Right and word motions do nothing (they can no
longer move the hidden caret), and `j`/`k`/`g`/`G` scroll through the input handler with all
other typing swallowed. Shift+arrows still extend the reading selection (Q2.3).
**Remaining:** Enter on a focused link, Space/PageUp/PageDown, and Ctrl+F in-page find (Q2.2).

#### Q2.2 In-page find — D · M
Build a reading find bar over block display text: case/regex/whole-word reuse from
`core::find`, match highlighting, Enter/F3 cycling, a count, and no mode switch. Reuse the
existing find-bar layout but target `ReadingCache` scroll positions.
**Done when:** Ctrl+F in reading highlights and cycles matches, Escape restores the prior
scroll, and the note is untouched.

#### Q2.3 Selection and copy polish — D · M
Add double-click word, triple-click paragraph/line, Shift-click extension, keyboard
selection, auto-scroll while dragging past the viewport, focus-on-mouse-down, Escape to
clear and a subtle "Copied" confirmation. Preserve the existing cross-block selection model.
**Done when:** selecting across blocks with mouse or keyboard copies the visible text with
math preserved; Ctrl+C works immediately after clicking a sidebar/outline row.

**Landed:** double-click selects the word under the pointer (multi-byte safe), triple-click
selects the whole paragraph/cell/callout line/code line, Shift-click extends from the anchor,
Shift+arrows/Home/End/PageUp/Down and Ctrl+Shift+Home/End extend the selection from the
keyboard across blocks, dragging past the top or bottom scrolls the list, Escape clears the
selection before it closes panels, and copy reports "copied N character(s)" in the status
bar (also when focus sits on a sidebar row, through the app-level Copy handler).

#### Q2.4 Reading typography controls — D · A · S · M
Move hardcoded sizes/width into persisted `ReaderSettings`: body size, line height, measure
(px), font family (sans/serif/mono for code), and zoom. Align the metadata header with the
body measure. Expose the same settings on mobile preview.
**Done when:** changing size/width updates all open readers and split previews immediately
and survives relaunch; no horizontal scroll appears at 320 px width.

**Landed (desktop):** `reading::ReaderSettings` (body size, measure) is a GPUI global backed
by `reader_size` and `reader_width` in `config.txt`, with Settings rows that cycle comfortable
values. Body text, list items, quotes, callouts and the column width all read it, display
math scales to the measure, and open readers plus the split preview repaint immediately.
**Remaining:** line height, font family and zoom; metadata header alignment; the mobile
preview.

#### Q2.5 Zen / focus reading — D · M
Add a distraction-free mode that hides the tab bar, sidebar, outline and relations, centers
the page and optionally dims non-active paragraphs. Make every panel toggleable and persist
the choice.
**Done when:** one shortcut/palette command enters and leaves zen mode; the document does
not jump when panels hide.

#### Q2.6 Media actually renders — D · A · M–L
Render inline images (not just standalone embeds), honor `![[img|300]]` width/alt, support
remote image URLs, and provide a real viewer or OS-open fallback for PDF/audio/video.
Add a lightbox with Escape/click-outside. On Android, pass an attachment index into
`render_html` so subfolder images resolve.
**Done when:** a note containing a local image, a sized image, a PDF and an audio file is
usable in reading mode on both platforms; no "not previewed" dead ends for supported types.

#### Q2.7 Code, tables and callouts — D · S–M
Add a copy button to code fences, horizontal scroll plus sticky headers for wide tables,
and collapsible callouts (remember per-note state). Do not let wide tables widen the page.
**Done when:** copying a fence works with one click; a 12-column table is readable without
scrolling the whole page; callouts remember their collapsed state for the session.

#### Q2.8 Footnote popover — D · S
Anchor the footnote popup near the clicked reference instead of `top(96)/left(24)`, show the
referenced text, support Escape/outside-click, and scroll correctly when the definition is
inside a fold.
**Done when:** every reference opens a readable popup that dismisses predictably and does
not cover the reference itself.

#### Q2.9 Transclusion fidelity — D · L
Give embedded blocks synthetic ids so link hit-testing and selection work, cache parsed
embeds by mtime, typeset math in embeds, honor `![[Note#Section]]`, and show a clear
missing-note placeholder. Do not re-read/re-parse the vault on every render.
**Done when:** a note with 3 nested embeds scrolls smoothly, links inside embeds work, and
editing the embedded note updates the host on the next render without a full vault scan.

#### Q2.10 Reading progress and typography details — D · S
Show section name and percent read in the status bar (replace edit-mode Ln/Col), join soft
line breaks with spaces while preserving explicit hard breaks, and keep footnote/heading
spacing consistent.
**Done when:** reading status is meaningful; one-sentence-per-line notes read as paragraphs.

**Landed:** the reading status bar shows the current section and percent read in place of
Ln/Col, and soft line breaks are joined into spaces while two-space and backslash hard breaks
are preserved.

#### Q2.11 Split preview performance and sync — D · M
Debounce full re-parse/re-typeset of split preview and the stats/outline computations on
every revision; cache by buffer revision. Optionally add scroll sync with a toggle.
**Done when:** typing in a 5k-line note does not drop frames or re-typeset every formula on
each keystroke; preview catches up within ~150 ms of typing stopping.

#### Q2.12 Live preview inside the source editor — D · M–L
Obsidian-style: the block holding the caret stays raw source and the rest of the note
renders. **Landed:** display `$$…$$` blocks are typeset in place with the reading view's
RaTeX scene whenever the caret is outside them, scaled to the column and centred; the source
returns while editing; empty, unclosed and invalid formulas keep their source instead of
leaving a gap; rows keep their geometry so clicks, scroll, selection and find are unchanged.
**Remaining:** a per-line display↔source column map before markers can be hidden
(`**`, `==`, `[[…]]`) or images, tables, callouts and rules rendered in place, because
clicks, selection, wrapping and IME all have to survive the substitution. Then render the
formula live while it is being typed (today it appears when the caret leaves the block).
**Done when:** a note with emphasis, an image and a table shows the caret's line as source
and the rest as preview, and every click lands on the source column the user aimed at.

---

### P1 — editing craft

#### Q3.1 Find/replace is keyboard-complete — D · M
Bind replace-one, replace-all, regex/case/word toggles; Tab/Shift+Tab cycles pattern →
replacement; Enter in replacement replaces; Shift+Enter finds previous; Escape closes and
clears highlights. Render `find.message` (replaced 1, no more matches, invalid regex).
Recompute matches after edits instead of using a stale list.
**Done when:** a full replace flow is possible without the mouse; every outcome is visible;
highlights disappear when the bar closes.

#### Q3.2 Tab and Shift+Tab indent — D · S
Add indent/outdent actions for selections (list-aware: continue `-`, `1.`, `>`, task
markers), and preserve Tab-to-accept when autocomplete is open. Never let Tab silently move
focus out of the editor.
**Done when:** Tab indents a multi-line selection, Shift+Tab outdents, list markers continue
correctly, and autocomplete still accepts with Tab/Enter.

**Landed:** `Tab` indents every selected line by four spaces (the selection maps through the
edit and stays selected); with a bare caret it inserts an indent at the caret. `Shift+Tab`
removes one tab or up to four leading spaces from each selected line, or from the caret's
line. `Tab` still accepts an autocomplete suggestion first, and Tab never leaves the editor.
Enter at the end of a bullet, task, numbered or quoted line continues its marker
(`3.` becomes `4.`, `- [x]` becomes `- [ ]`, indentation kept); an empty item does not
continue. Unit tests cover the marker rules.

#### Q3.3 Save As is discoverable — D · S
Bind Ctrl/Cmd+Shift+S. When Ctrl+S or Ctrl+W hits an Untitled note, open Save As immediately
(as a dialog or inline filename field) instead of discarding an error. After saving, focus
returns to the editor at the same position.
**Done when:** a new note can be named and saved in one flow; Ctrl+W on Untitled offers
Save As, not a dead end.

#### Q3.4 A real feedback system — D · A · S · M
Replace the single never-expiring muted `status_message` with toasts/status entries that
have severity (info/success/warn/error), expiry (errors persist until dismissed), optional
actions (Undo, Retry, Show) and screen-reader announcement. Keep a small persistent
save/sync state in the status bar.
**Done when:** save success, save failure, conflict, link-not-found and sync failure are
distinguishable at a glance; no stale message survives a context change.

#### Q3.5 Conflict and reload bar — D · M
When a file changes externally: clean buffer reloads silently **and preserves scroll/cursor**;
dirty buffer shows a per-tab badge plus Reload / Compare / Keep mine / Save as copy. Never
auto-reload a dirty buffer. Deleted files surface instead of being skipped.
**Done when:** external edits never surprise the user; inactive tabs show a conflict marker;
Compare uses the existing line diff and recovery history.

#### Q3.6 Close is undoable; trash is reachable — D · S–M
Add Ctrl/Cmd+Shift+T to reopen the last closed tab (store a small closed-tab stack with
path/mode/position) and a "Trash" command that lists, restores and empties using
`vaultfs::list_trash`/`restore`. After closing, show "Closed note — Undo".
**Done when:** accidental close is recoverable without the file tree; deleted notes can be
restored from inside the app.

#### Q3.7 Multi-cursor correctness — D · M
Apply formatting to every selection, remap extra cursors after edits, clear extras on
undo/redo/Vim transitions, and coalesce a multi-cursor typing burst into one undo step.
**Done when:** formatting a multi-selection edits all ranges; one undo reverses the whole
burst; carets never land at stale offsets.

#### Q3.8 Per-keystroke work is bounded — D · M
Debounce/cache stats, outline headings, reading attachments index, and split preview by
buffer revision. Build the attachments/alias index once per vault scan, not on first render.
**Done when:** a 1 MB note remains responsive while typing; idle CPU stays at zero; no
full-vault walk on mode entry.

#### Q3.9 Focused split is the command target — D · S
Track the focused pane/tab; Ctrl+W, Save, Find and format commands apply to the focused
editor, not always `active_tab`. Clicking a pane focuses it and updates the active tab.
**Done when:** two side-by-side notes can be edited and closed independently with the
keyboard; the tab bar highlights the focused pane.

**Landed:** clicking a split pane activates its tab and focuses its editor, so Ctrl+W, Save,
Find and formatting target the pane the user is looking at; the tab bar already marks
secondary panes.

#### Q3.10 Autocomplete belongs at the caret — D · M
Replace the bottom bar with a caret-anchored popup (or a non-resizing overlay), keep it
within the viewport, support Up/Down/Tab/Enter/Escape, and rank exact prefix matches before
substring matches.
**Done when:** `[[`, `#` and `/` suggestions appear next to the caret, never resize the
document, and are fully keyboard-operable.

#### Q3.11 Format bar does not move the page — D · S
Overlay the selection formatting bar near the selection (or reserve stable space) so
appearing/disappearing bars do not change the editor viewport.
**Done when:** selecting text at the top/bottom of the viewport does not scroll the
document.

**Landed:** the format bar reserves a fixed 30 px strip in edit mode (an empty bar when
nothing is selected), so selecting or deselecting text never resizes the editor.

#### Q3.12 Small editor gaps — D · S
Vim: support redo, make `:wq` close the tab, surface failed searches. Window title: show
note name and dirty marker. Tabs: overflow menu and middle-click close. Editor: right-click
context menu (cut/copy/paste/format/link) and feedback for copy-with-no-selection. Block
move/duplicate/delete: preserve cursor and scroll.
**Done when:** no common action is a silent no-op; the window title is a reliable dirty
indicator; block operations do not jump the viewport.

#### Q3.13 Dirty state reflects content — D · S
Compare against the saved text/revision rather than flipping a bool. Type-then-delete and
undo-after-save return to clean. Use this same signal for close prompts and tab dots.
**Done when:** a property test of random edit/undo sequences only reports dirty when bytes
differ from the saved revision.

**Landed:** `Buffer` keeps the content hash of the saved version and derives `is_dirty` from
the current text (FNV-1a over the rope, cached per revision), so typing and deleting, or
undoing back to the saved bytes, reports clean again. A 200-step randomized edit/undo test
asserts `is_dirty == (text != saved)` after every step; the tab dot, title marker and close
prompt all read this one signal.

#### Q3.14 Startup is honest — D · S
If a restored tab fails to load, show an inline error with Retry/Locate instead of a blank
looking tab. Do not block window creation on loading every tab; load the active tab first
and the rest lazily.
**Done when:** a missing/locked file is visible and recoverable; 50 restored tabs still
start under the cold-start budget.

**Landed:** a restored tab whose file cannot be read records the failure, and the pane shows
"… could not be opened", the path and reason, plus **Retry** and **Close tab** instead of an
empty editor that silently accepts typing. Retry reloads in place.
**Remaining:** tabs are still loaded eagerly at startup; lazy loading is open.

---

### P1/P2 — mobile daily flow

#### Q4.1 Search that behaves like search — A · S
Add a clear button, IME Search action, result count, match highlighting, an empty state, and
Back-to-clear. Keep the query when returning from a note.
**Done when:** a user can search, open a result, return and refine the same query without
losing state.

#### Q4.2 In-note find — A · M
Add find (and replace, if the editor remains plain text) with a compact bar above the IME,
match count, next/previous and scroll-to-match. Reuse the engine's find options.
**Done when:** finding a phrase in a long note is possible without scrolling manually.

#### Q4.3 Mobile editing toolbar and autocomplete — A · L
Add a keyboard accessory row: bold, italic, highlight, link, heading, list, task, code;
insert wikilinks, tags and slash commands from a caret/selection-aware picker. Add basic
syntax highlighting (even read-only spans) and line numbers if they do not hurt small
screens.
**Done when:** common markdown can be authored on a phone without typing punctuation
syntax; `[[` opens a searchable note picker; the toolbar does not cover the caret.

#### Q4.4 Sync is visible and recoverable — A · M
Show a sync chip/state in the editor and list (synced/unsaved/syncing/error/last sync),
allow sync from the editor after saving, make conflict rows actionable with Retry/Abort,
and never leak JGit error strings as the only explanation.
**Done when:** a user always knows whether their work is local-only; a failed sync can be
retried from where it failed.

#### Q4.5 New note and row actions — A · S
Prompt for title and folder on create (no root "Untitled"), add long-press row actions
(rename/move/delete/bookmark), and make properties/tags/relations chips clickable.
**Done when:** creating a note takes one intentional dialog; every chip in Details navigates
or filters.

#### Q4.6 Import/export progress — A · S
Report file/byte progress, allow cancel, and keep the UI responsive for large vaults.
**Done when:** importing 10k files shows progress and can be cancelled without corruption.

#### Q4.7 Preview typography and theme — A · S
Use the shared reader settings for size/width/theme; support pinch-zoom; avoid reloading
the WebView on every recomposition.
**Done when:** text is comfortable on a phone and tablet; zoom and theme survive mode
switches.

#### P2 — polish (schedule after P0/P1)
- Mobile: graph layout off the main thread, pan/zoom, share/deep-link entry, vault
  remove/rename, swipe gestures, pull-to-refresh, `![[image]]` subfolder fixes, tablet
  two-pane layout.
- Desktop: reading folds remembered, window/tab restore across monitors, tab overflow
  menu, collapsed callout persistence, `tel:`/`file://` link handling, `../` link
  normalization, unresolved-link confirmation, tag chips as navigation, code copy feedback.
- Shared: link preview on hover, external-link confirmation setting, reading time/word
  count in the reading status bar, accessibility labels and focus order audit, RTL and
  IME regression pass.
- Performance: background index/alias build, lazy tab loading, cache invalidation audit.

---

## 5. Shared architecture: fix it once, not per client

The current clients duplicate policy. Mobile owns save/sync/link-mirroring decisions in
`DragonGlassViewModel`; desktop owns them in `main.rs`/`editor.rs`. Before iOS, extract the
following into `dragonglass-core` (or a thin shared layer) and expose through the FFI:

| Piece | Why shared | Sketch |
|---|---|---|
| `DocumentFingerprint` + `SaveReport` | Conflict safety must be identical everywhere | Hash + mtime + size captured on load; save compares and returns `Saved`/`Conflict`/`Error` |
| Draft store | Crash recovery must work on every platform | `.dragon-glass/drafts/<id>` plus metadata; debounced writes; recover/compare/discard |
| `Session` v2 | Per-tab position/mode is the core continuity primitive | Versioned key=value or JSON; unknown keys ignored; per-tab cursor/scroll/mode/folds |
| `SourceSpan` anchors | Edit↔read mapping is parsing, not UI | `parse_blocks` returns spans or a parallel `Vec<Span>`; block-id index |
| Link reference model | `#heading`/`^block` resolution should not diverge | Parsed `LinkRef { target, fragment }`; resolve + anchor lookup in core |
| `ReaderSettings` | Typography/width/theme parity | Persisted in config; desktop + mobile readers consume |
| `SyncState` | Android/iOS/CLI need one state machine | States: idle, dirty, syncing, conflict, error; no JGit strings in UI |
| `NavEntry`/history | Back/forward semantics are product logic | `(path, mode, anchor)` stack with cap; clients render controls |

**FFI additions:** `save_document_with_fingerprint`, `document_fingerprint`,
`load_draft`/`save_draft`/`discard_draft`, `resolve_link_fragment`, `reader_settings`, and
`sync_state`. Keep all of them local-only and synchronous where possible; do not add
background work to the FFI.

**UX contract document:** write `docs/ux-contract.md` with the invariants in §2 and a table
of state transitions (open, edit, save, close, read, follow, back). Desktop and Android
should both point tests at it; iOS implements it rather than copying Android screens.

**CLI:** no separate edit/read flow, but `export`/search should reuse the same link-fragment
resolution and reader settings so exported HTML matches the in-app reader. Deep-link/URI
handling should also resolve fragments, not just notes.

### 5.1 Platform notes

**Desktop (Windows, Linux, macOS)**
- Keep one codebase and one flow contract, but respect native conventions: `Cmd` on macOS
  (`secondary`), menu-bar equivalents for every editing command, native file dialogs, and
  the platform's window-close/quit lifecycle.
- macOS quits via `applicationWillTerminate`, Windows/Linux via window close; both must run
  the same save/discard/cancel gate.
- Support mouse back/forward if GPUI exposes them on the platform; otherwise bind
  Alt+Left/Right and document it in the shortcut editor.
- Test IME (Japanese/Chinese/Korean), RTL, HiDPI, multiple monitors and 50 restored tabs.

**Android**
- Treat lifecycle as a first-class state machine: `onStop` flushes drafts; process death
  restores a draft; SAF mirror saves are fingerprinted and conflict-checked.
- Compose state holders own list/editor/preview scroll; WebView owns only its document.
- Use `imePadding`/insets, TalkBack labels, and Android's Back hierarchy from Q1.8.
- Do not chase desktop parity on phones: reading, capture, quick edits and search come
  before tabs/splits/Vim/multi-cursor.

**iOS (planned)**
- Start from `docs/ux-contract.md`, not from a line-by-line port of `Screens.kt`. The
  current Android client already contains the rough edges in §3.3; copying it would clone
  the bugs.
- `WKWebView` needs a navigation delegate for the same internal/external link policy as
  Q0.5. Use `keyboardLayoutGuide`/safe-area handling for Q0.6, Dynamic Type for reader
  settings, and `ProcessLifecycle`/scene background for drafts.
- Storage uses security-scoped bookmarks; token storage uses Keychain. Git moves into the
  FFI (`git2`) as recommended in `docs/ios-port.md`; no process spawning.
- The GPL/App Store question is unresolved. Do not spend iOS UI weeks before it is decided.

**CLI**
- No editing/reading UI, but it is the third consumer of link resolution and export. Keep
  `dragonglass export` consistent with the reader (headings, fragments, math, embeds) so
  automation and sharing do not diverge from what users see.

---

## 6. Sequencing

The order is deliberate: safety first, then continuity, then reading/editing depth, then
mobile parity, then iOS. Desktop and Android safety work can overlap; the shared pieces
should land before the second client starts.

| Phase | Goal | Main items | Estimate | Exit criteria |
|---|---|---|---|---|
| **0 — Safety** | Make it impossible to lose text | Q0.1–Q0.7, Q3.13, tests | 9–12 days | Reading cannot mutate; conflict never overwrites; crash recovery works; close/quit/background and mobile save/IME/preview safety tested |
| **1 — Continuity** | Never lose the place | Q1.1–Q1.9, Q3.9 | 7–9 days | Session v2 round-trip; edit↔read anchor; back/forward; fragments; mobile back/scroll |
| **2 — Reading** | Make reading a first-class mode | Q2.1–Q2.5, Q2.7–Q2.11 | 8–11 days | Keyboard-only read; in-page find; selection/copy; typography/zen; media/code/table/footnote/transclusion fixes |
| **3 — Editing** | Remove daily editor friction | Q3.1–Q3.8, Q3.10–Q3.12, Q3.14 | 7–9 days | Find/replace keyboard-complete; Tab indent; Save As; feedback system; undo close/trash; bounded per-keystroke work |
| **4 — Mobile depth** | Bring Android to the same flow quality | Q4.1–Q4.7 | 7–9 days | Search/find/toolbar; sync visibility; create/import/properties polish; preview typography |
| **5 — Hardening/iOS** | Share the contract and prepare the port | Shared layer, UX contract, accessibility, perf | 4–6 days | Core/FFI tests green; Android passes acceptance; iOS can implement from the contract |

**Total:** roughly 42–56 solo dev-days, excluding P2 and platform packaging. Treat this as a
sequence, not a deadline. Phase 0 desktop and Android safety work can run in parallel; the
later phases are ordered by user impact, not by platform.

**Dependency notes**
- Q0.2 must land before Q0.4, Q0.5, Q3.5 and any autosave that writes notes.
- Q1.1 is the foundation for Q1.2, Q1.3, Q1.7 and Q3.6 (closed-tab stack).
- Q1.2 needs parser source spans; do not bolt line numbers onto rendered blocks.
- Q2.2 can reuse the existing find state, but needs reading scroll APIs.
- Q4.3 (mobile toolbar) should not start before Q0.6 (IME) or it will be unusable.
- iOS implementation starts only after Phase 0–2 semantics are in the contract; otherwise
  the port copies today's rough edges.

---

## 7. How to measure success

Use local, opt-in instrumentation only; the app does not phone home. Most of these can be
test assertions rather than telemetry.

| Metric | Target | How |
|---|---|---|
| Silent overwrite count | 0 | Conflict test suite; every save path returns `SaveReport` |
| Crash-recovery loss | 0 characters | Kill process after typing; relaunch and compare draft |
| Position round-trip | ±1 line / ±1 block / ±16 px | Automated session save/restore test; manual 20x drift test |
| Link round-trip | 2 actions max (click, Back) | Scripted reading scenario |
| Mode continuity | 100% of internal links keep mode | Integration test through `open_target` |
| Save feedback coverage | 100% of save/sync/export paths | Code audit + UI tests |
| Reading immutability | Source hash unchanged after fuzz keys | Editor action fuzz test |
| Time to resume | < 1 s to a usable note, active tab first | Startup benchmark with 50 tabs |
| Idle CPU | 0% after clean save | Existing event-driven budget; verify no autosave timer |
| Mobile keyboard occlusion | 0 blocked carets | 360×640 and tablet manual matrix |

**Manual flow scripts** (run on Windows, macOS, Linux, Android; iOS later):
1. Open vault → note → type → wait → quit → relaunch → same note, same position, same mode.
2. Read long note → Ctrl+E at three positions → follow `[[Note#Heading]]` → Back twice.
3. External edit note while dirty → Save → assert conflict UI → Compare → Keep mine/Reload.
4. Kill app 2 s after typing → relaunch → Recover draft → save.
5. Close dirty tab → Undo close → verify text and position.
6. Phone: keyboard open at end of note → status bar visible → Preview → tap link → Back.
7. Search result at line 500 → open → hit visible/selected → Back → query/scroll intact.
8. 10k-note vault: quick switch, search, open, edit; no multi-second stalls.

**Test matrix**
- Vaults: empty, 10 notes, 10k notes, 1 MB note, CRLF, non-ASCII filenames, nested
  attachments, broken links, duplicate aliases, external edits during editing.
- Desktop: Windows/macOS/Linux, Cmd/Ctrl paths, IME (Japanese/Chinese), HiDPI, RTL,
  keyboard-only, 50 restored tabs.
- Android: API 26/33/37, phone/tablet, process death, SAF-linked vault, external keyboard,
  TalkBack, light/dark.
- iOS (when built): Dynamic Type, VoiceOver, external keyboard, WKWebView navigation,
  security-scoped bookmarks, background/foreground.

---

## 8. Decisions needed before coding

1. **Autosave default.** Recommendation: drafts always; autosave the note after ~1.5 s idle
   (configurable, on by default), stopping at any fingerprint conflict. The alternative is
   draft-only until explicit Save; safer for git users but adds a save step.
2. **Conflict policy.** Recommendation: never last-write-wins. Block the write and offer
   Reload / Compare / Keep mine / Save as copy.
3. **Session format.** Recommendation: versioned, human-readable, tolerant parser; include
   per-tab cursor/scroll/mode. Do not reuse the current single-boolean `reading` shape.
4. **Mobile storage.** Keep the app-private mirror for now, but add fingerprints and
   conflict detection; revisit direct SAF access only after the UX contract is stable.
5. **Reading settings scope.** Recommendation: global typography, per-vault optional width.
   One setting surface, applied to editor-adjacent preview and mobile WebView.
6. **Mobile editing depth.** Recommendation: Phase 4 adds a toolbar/autocomplete/find; do
   not attempt full desktop editor parity (tabs, splits, Vim, multi-cursor) on phones.
7. **iOS licensing.** The GPL/App Store question in `docs/ios-port.md` is still a blocker.
   Do not spend iOS UI weeks before it is resolved.

---

## 9. Risks and mitigations

| Risk | Mitigation |
|---|---|
| Autosave creates noisy git commits or races sync | Debounce, skip clean buffers, save drafts separately, pause while syncing; explicit Save still works |
| Fingerprint false positives (mtime granularity, clock skew) | Compare content hash as the source of truth; treat mtime as a hint |
| Session file grows or breaks across versions | Version key, cap entries, ignore unknown keys, prune missing notes, test old/new round-trips |
| Anchor mapping drifts in heavily edited notes | Store a line plus nearest heading/block id; clamp to document bounds; fall back to top with a notice |
| Reading find/selection touches hidden edit state | Make reading mode own its selection/scroll state; add hash-invariance tests |
| Mobile lifecycle differences (Android/iOS) | Put draft/save policy behind a shared contract; test process death explicitly |
| Scope creep from P2 | Ship P0/P1 as vertical slices; P2 only after acceptance scripts pass |
| Performance regression from debounce/caching | Keep caches revision-keyed; benchmark 1 MB note and 10k-note vault; verify idle CPU |

---

## 10. Quick wins (< 1 day each)

These can land before the phased work and immediately reduce friction:

1. Guard reading-mode mutations (Q0.1) — the single most important fix.
2. Bind find replace-one/all and the regex/case/word toggles; render `find.message` (Q3.1).
3. Ctrl/Cmd+Shift+S, and make Ctrl+S on Untitled open Save As (Q3.3).
4. Expire status messages; style errors differently from info (Q3.4).
5. Show a conflict marker on every affected tab, not just the active one (Q3.5).
6. Add "Undo close" (Ctrl+Shift+T) using a small closed-tab stack (Q3.6).
7. Android `imePadding()` + caret reveal (Q0.6).
8. Android `WebViewClient` for preview links + Back from Preview → Edit (Q0.5/Q1.8).
9. Android search clear button, IME Search action and Back-to-clear (Q4.1).
10. Join soft line breaks in reading paragraphs; keep hard breaks (Q2.10).
11. Make tag chips and frontmatter links clickable (Q4.5/P2).
12. Window title = note name + dirty marker (Q3.12).
13. Vim `:wq` closes; add redo; report failed search (Q3.12).
14. Add a code-fence copy button (Q2.7).
15. Audit `Esc` so every overlay (find, autocomplete, format bar, toast, footnote, panel)
    closes in a predictable order.

---

## 11. What not to do

- Do not add another mode until edit/read/position/mode transitions are trustworthy.
- Do not copy Android's screen structure into iOS before the contract exists; that would
  clone the rough edges listed in §3.3.
- Do not build a sync server or a native merge UI to compensate for save/conflict gaps.
- Do not turn mobile into a smaller desktop: prioritize reading, capture, quick edits and
  search over tabs/splits/Vim.
- Do not trade idle CPU or cold start for background indexing; keep work event-driven.

---

## Appendix: existing helpers worth reusing

- `storage::write_atomic` (atomic saves), `history::record` (pre-save snapshots),
  `diff::lines` (compare UI), `conflict::classify`.
- `find::Options` / `replacement_for` for find/replace; `text_format::toggle` for styles.
- `open_panel`/`close_panel` for focus restoration; `handle_escape` as the one escape chain.
- `apply_style`'s `reading || find.open` guard as the pattern for Q0.1.
- `apply_edit_targets` for batched multi-caret edits; `scroll_to_heading_index` for
  fragment navigation; `ReadingCache::list_state` for reading scroll persistence.
- `vaultfs::list_trash`/`restore` for the missing Trash UI.
- `GitSyncManager`'s `GitStatus`/conflict shape as the seed for the shared sync state.
