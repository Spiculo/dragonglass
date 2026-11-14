# UX contract: edit and read flows

This is the shared contract for every Dragon Glass client — desktop, Android, iOS. It is
the test oracle for [`docs/qol-plan.md`](qol-plan.md): a client conforms when every
invariant below holds, regardless of how the UI looks.

Status: **active**. Desktop enforces the whole contract except where marked. Android
enforces the marked rows; iOS starts from this document rather than from `Screens.kt`.

## 1. Invariants

| # | Invariant | Enforced by | Tested by |
|---|---|---|---|
| 1 | **No silent text loss.** A save either succeeds visibly or stops with an actionable conflict/error. Nothing writes over a file that changed underneath the editor without asking. | `core::fingerprint::save_if_unchanged`; `Editor::save_checked`; conflict bar | `fingerprint` tests (clean, external edit, deleted file, mtime-only, write error) |
| 2 | **No silent text mutation.** Reading mode never edits the source. Focus in a reader scrolls and selects, never changes bytes. | `Editor::can_mutate` on every mutating action; both IME insert paths divert to reading navigation/find; Tab/Enter walk links | — (a property-based shortcut fuzz harness is still wanted) |
| 3 | **No lost place.** Per tab the client remembers mode, cursor/selection, edit scroll and reading scroll across tab switch, mode switch, quit and relaunch. | `core::session` v2; `remember_session`; startup restore | `session` tests; restore-filter tests |
| 4 | **Reading and editing are continuous.** Entering reading starts at the block containing the caret; leaving restores the source line. Following a link keeps the mode. | `open_note_impl` carries the mode; `jump_to_heading`; `parse_blocks_spanned` | span and line-mapping tests in `markdown` |
| 5 | **Links are complete.** `[[Note]]`, `[[Note#Heading]]`, `[[#Local]]`, `[[Note#^block]]` navigate to the target, and every navigation has a back path. | `open_target`; `jump_to_fragment`; nav history | fragment tests; `resolveLinkFragment` tests |
| 6 | **Leaving is reversible or explicit.** Closing a dirty tab/note offers Save / Discard / Cancel; quit cannot drop a failed save. | `close_tab`, `request_close`, `quit_action` | — |
| 7 | **Every action reports back.** Save, sync, export, conflict and failed link produce a typed result (info/success/warn/error), not a stale line. | `StatusKind` + `set_status`; `expire_status` | UI status rewrite |
| 8 | **No incidental layout jump.** Find, autocomplete, the format bar and toasts do not move the document under the caret. | fixed format bar; overlaid autocomplete | — |

Android rows in force: 1 (save-before-navigate), 2 (preview is read-only), 3 (note,
preview flag), 6 (a failed save keeps the note open), 7 (snackbar errors), 8 (IME insets).

## 2. State transitions

| From | Action | To | Rules |
|---|---|---|---|
| No note | Open note | Edit | Load, capture fingerprint, check for a draft |
| Edit | Ctrl/Cmd+E | Reading | Keep the caret line; scroll the reader to that block (Q1.2) |
| Reading | Ctrl/Cmd+E | Edit | Restore the source line of the visible block (Q1.2) |
| Edit | Type | Edit (dirty) | Dirty is content-based; undo back to saved bytes is clean |
| Edit (dirty) | Idle 1.5 s | Edit (draft saved) | Draft written; no note write unless `autosave=on` |
| Edit (dirty) | Save, disk unchanged | Edit (clean) | Fingerprint refreshed, draft deleted |
| Edit (dirty) | Save, disk changed | Conflict | Nothing written; Reload / Keep mine / Compare / Save as copy |
| Conflict | Reload | Edit (clean) | Fingerprint refreshed, draft deleted |
| Conflict | Keep mine | Edit (clean) | Forced write, fingerprint refreshed |
| Edit/Reading | Link / quick switch / search hit | Edit/Reading (same mode) | Nav entry recorded with mode, line, reading block |
| Any | Back (`Ctrl/Cmd+[`) | Previous entry | Mode, caret and reading block restored |
| Any | Forward (`Ctrl/Cmd+]`) | Next entry | Forward tail dropped when a new place is visited |
| Edit (dirty) | Close tab / window / Cmd+Q | Prompt | Save and quit / Quit without saving / Cancel; failed save stays open |
| Edit (dirty) | Close, save fails | Prompt | Tab stays open with its text and the error |
| Launch | Session v2 has tabs | Restored tabs | Missing files pruned; a bad file shows Retry / Close |
| Launch | Draft differs from file | Draft bar | Recover / Compare / Discard; Recover is one undoable edit |

## 3. Save contract

```
DocumentFingerprint { size, modified: Option<SystemTime>, content_hash }
SaveReport::{ Saved{fingerprint}, Conflict{disk: Option<fingerprint>}, Error{message} }
```

- `content_hash` is FNV-1a and is the source of truth; size and modified are hints.
- `expected = None` means "no baseline" (a brand-new file) and writes.
- `Conflict { disk: None }` means the file was deleted; saving is an explicit recreation.
- Every client saves through this contract. No client calls `write_atomic` for a note
  without a fingerprint check except the explicit "keep mine" path.

## 4. Shared FFI surface

Every client calls these instead of reimplementing policy. All are synchronous and
local-only; nothing here starts background work.

| Call | Purpose |
|---|---|
| `MobileVault.documentFingerprint(relative)` | Capture the baseline before editing |
| `MobileVault.saveDocumentWithFingerprint(relative, text, expectedHash?)` | Conflict-safe save; returns `saved` / `conflict` (with the disk hash) / `error` |
| `saveDraft` / `loadDraft` / `discardDraft` | Crash-recovery drafts, written from a lifecycle hook |
| `readerSettings` / `setReaderSettings(size, width, font, lineHeight)` | Typography shared with desktop (`reader_*` keys in `config.txt`) |
| `syncState(changed, untracked, ahead, behind, conflicts, busy, failed)` | One sync state machine: idle, dirty, syncing, conflict, error |
| `resolveLinkFragment(source, fragment)` | `#Heading` / `#^block` to a 1-based source line |
| `findInText(source, pattern, regex, case, wholeWord)` | UTF-16 match ranges for a text field |
| `syntaxSpans(source)` | UTF-16 markdown highlight runs for the editor |
| `MobileVault.resolveTarget(target)` | Wikilink/relation target to a vault path |
| `MobileVault.filterByProperty(key, value)` | Exact frontmatter filter rows |

## 5. What is still open

Tracked in [`qol-plan.md`](qol-plan.md): inline live preview beyond display math (Q2.12),
the Android text field's own scroll offset (Q1.7), optional editor↔preview scroll sync
(Q2.11), line numbers on phones (Q4.3, deliberately omitted), and the iOS client (no iOS
toolchain in this environment; this document is the porting target).
