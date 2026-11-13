//! A multi-line text editor view, built from scratch on GPUI.
//!
//! GPUI ships no multi-line editor, so this is a hand-rolled `Element` that shapes each
//! line of a `dragonglass_core::Buffer`, paints a caret and selection, and routes keyboard
//! input and platform IME text through `EntityInputHandler`.
//!
//! The editor also owns the in-note find/replace bar, optional Vim mode, and multi-cursor
//! state. Selections and extra carets live here; the buffer only stores the primary caret.

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use dragonglass_core::drafts;
use dragonglass_core::find as find_engine;
use dragonglass_core::fingerprint::{self, DocumentFingerprint, SaveReport};
use dragonglass_core::format as text_format;
use dragonglass_core::frontmatter::{self, Property, PropertyValue};
use dragonglass_core::inline;
use dragonglass_core::markdown;
use dragonglass_core::math::{self, MathOptions, MathScene, MathStyle};
use dragonglass_core::{blocks, git, history, stats, syntax, templates, wrap, Buffer};
use gpui::{
    actions, div, fill, point, prelude::*, px, relative, size, AnyElement, App, Bounds,
    ClipboardEntry, ClipboardItem, Context, CursorStyle, ElementId, ElementInputHandler, Entity,
    EntityInputHandler, ExternalPaths, FocusHandle, Focusable, GlobalElementId, ImageFormat,
    InspectorElementId, KeyBinding, KeyDownEvent, LayoutId, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, PaintQuad, Pixels, Point, Render, ScrollHandle, ShapedLine,
    SharedString, Style, TextRun, Timer, UTF16Selection, Window,
};

use crate::math_element::{paint_scene, MathElement};
use crate::reading::{LinkOpener, ReadingCache};
use crate::theme::Theme;
use crate::vim::{Mode as VimMode, Vim};

/// How many leading lines are inspected for YAML frontmatter. Real frontmatter is a few
/// lines; the cap keeps property refresh from copying a large note.
const FRONTMATTER_LINE_WINDOW: usize = 256;

/// Runs the app's Save As flow for an editor that has no file name yet.
pub type SaveAsHandler = Rc<dyn Fn(&mut Window, &mut App)>;

/// Asks the app to close the tab that owns this editor (Vim `:wq`).
pub type CloseTabHandler = Rc<dyn Fn(&mut Window, &mut App)>;

/// Reports a one-line result to the app's status bar.
pub type StatusReporter = Rc<dyn Fn(crate::StatusKind, &str, &mut App)>;

/// Tells the app that a save hit a conflict, so it can show the resolution bar.
pub type ConflictHandler = Rc<dyn Fn(&mut App)>;

actions!(
    editor_actions,
    [
        Backspace,
        Delete,
        DeleteWordLeft,
        DeleteWordRight,
        Left,
        Right,
        Up,
        Down,
        WordLeft,
        WordRight,
        ParagraphUp,
        ParagraphDown,
        DocumentStart,
        DocumentEnd,
        Home,
        End,
        Enter,
        Paste,
        Save,
        ToggleReading,
        SelectLeft,
        SelectRight,
        SelectUp,
        SelectDown,
        SelectWordLeft,
        SelectWordRight,
        SelectDocumentStart,
        SelectDocumentEnd,
        SelectHome,
        SelectEnd,
        SelectAll,
        Copy,
        Cut,
        Undo,
        Redo,
        MoveBlockUp,
        MoveBlockDown,
        ToggleFold,
        OpenFind,
        OpenFindReplace,
        FindNext,
        FindPrev,
        FindReplaceOne,
        FindReplaceAll,
        FindToggleRegex,
        FindToggleCase,
        FindToggleWord,
        SelectNextOccurrence,
        AddCursorAbove,
        AddCursorBelow,
        ToggleBold,
        ToggleItalic,
        ToggleStrikethrough,
        ToggleHighlight,
        ToggleInlineCode,
        ToggleComment,
        InsertWikilink,
        Indent,
        Outdent
    ]
);

/// Register the editor's default key bindings. Obsidian-compatible where it matters.
pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("backspace", Backspace, None),
        KeyBinding::new("delete", Delete, None),
        KeyBinding::new("ctrl-backspace", DeleteWordLeft, None),
        KeyBinding::new("ctrl-delete", DeleteWordRight, None),
        KeyBinding::new("left", Left, None),
        KeyBinding::new("right", Right, None),
        KeyBinding::new("up", Up, None),
        KeyBinding::new("down", Down, None),
        // Word-wise movement (the `cmd` forms mirror the rest of the keymap on macOS).
        KeyBinding::new("ctrl-left", WordLeft, None),
        KeyBinding::new("ctrl-right", WordRight, None),
        KeyBinding::new("ctrl-up", ParagraphUp, None),
        KeyBinding::new("ctrl-down", ParagraphDown, None),
        KeyBinding::new("ctrl-home", DocumentStart, None),
        KeyBinding::new("ctrl-end", DocumentEnd, None),
        KeyBinding::new("alt-left", WordLeft, None),
        KeyBinding::new("alt-right", WordRight, None),
        KeyBinding::new("home", Home, None),
        KeyBinding::new("end", End, None),
        KeyBinding::new("shift-left", SelectLeft, None),
        KeyBinding::new("shift-right", SelectRight, None),
        KeyBinding::new("shift-up", SelectUp, None),
        KeyBinding::new("shift-down", SelectDown, None),
        KeyBinding::new("shift-home", SelectHome, None),
        KeyBinding::new("shift-end", SelectEnd, None),
        KeyBinding::new("ctrl-shift-left", SelectWordLeft, None),
        KeyBinding::new("ctrl-shift-right", SelectWordRight, None),
        KeyBinding::new("ctrl-shift-home", SelectDocumentStart, None),
        KeyBinding::new("ctrl-shift-end", SelectDocumentEnd, None),
        KeyBinding::new("alt-shift-left", SelectWordLeft, None),
        KeyBinding::new("alt-shift-right", SelectWordRight, None),
        KeyBinding::new("ctrl-a", SelectAll, None),
        KeyBinding::new("cmd-a", SelectAll, None),
        KeyBinding::new("ctrl-c", Copy, None),
        KeyBinding::new("cmd-c", Copy, None),
        KeyBinding::new("ctrl-x", Cut, None),
        KeyBinding::new("cmd-x", Cut, None),
        // Undo/redo. Obsidian uses Ctrl+Z / Ctrl+Shift+Z (and Ctrl+Y on Windows).
        KeyBinding::new("ctrl-z", Undo, None),
        KeyBinding::new("cmd-z", Undo, None),
        KeyBinding::new("ctrl-shift-z", Redo, None),
        KeyBinding::new("cmd-shift-z", Redo, None),
        KeyBinding::new("ctrl-y", Redo, None),
        // Vim's redo key; harmless outside Vim mode and the only Control binding left.
        KeyBinding::new("ctrl-r", Redo, None),
        KeyBinding::new("enter", Enter, None),
        KeyBinding::new("ctrl-v", Paste, None),
        KeyBinding::new("cmd-v", Paste, None),
        KeyBinding::new("ctrl-s", Save, None),
        KeyBinding::new("cmd-s", Save, None),
        KeyBinding::new("ctrl-e", ToggleReading, None),
        KeyBinding::new("cmd-e", ToggleReading, None),
        KeyBinding::new("ctrl-shift-up", MoveBlockUp, None),
        KeyBinding::new("ctrl-shift-down", MoveBlockDown, None),
        // Fold or unfold the heading at the caret.
        KeyBinding::new("ctrl-shift-[", ToggleFold, None),
        KeyBinding::new("cmd-shift-[", ToggleFold, None),
        // In-note search/replace and multi-cursor editing.
        KeyBinding::new("ctrl-f", OpenFind, None),
        KeyBinding::new("cmd-f", OpenFind, None),
        KeyBinding::new("ctrl-h", OpenFindReplace, None),
        KeyBinding::new("cmd-alt-f", OpenFindReplace, None),
        KeyBinding::new("f3", FindNext, None),
        KeyBinding::new("shift-f3", FindPrev, None),
        // Let the find bar be driven from the keyboard once it is open: replace one/all and
        // the regex/case/word toggles. The handlers ignore the keys when the bar is closed.
        KeyBinding::new("secondary-alt-enter", FindReplaceOne, None),
        KeyBinding::new("secondary-alt-a", FindReplaceAll, None),
        KeyBinding::new("secondary-alt-r", FindToggleRegex, None),
        KeyBinding::new("secondary-alt-c", FindToggleCase, None),
        KeyBinding::new("secondary-alt-w", FindToggleWord, None),
        KeyBinding::new("ctrl-d", SelectNextOccurrence, None),
        KeyBinding::new("cmd-d", SelectNextOccurrence, None),
        KeyBinding::new("ctrl-alt-up", AddCursorAbove, None),
        KeyBinding::new("ctrl-alt-down", AddCursorBelow, None),
        KeyBinding::new("cmd-alt-up", AddCursorAbove, None),
        KeyBinding::new("cmd-alt-down", AddCursorBelow, None),
        // Inline formatting, selected text only.
        KeyBinding::new("ctrl-b", ToggleBold, None),
        KeyBinding::new("cmd-b", ToggleBold, None),
        KeyBinding::new("ctrl-i", ToggleItalic, None),
        KeyBinding::new("cmd-i", ToggleItalic, None),
        KeyBinding::new("ctrl-shift-x", ToggleStrikethrough, None),
        KeyBinding::new("cmd-shift-x", ToggleStrikethrough, None),
        KeyBinding::new("ctrl-shift-h", ToggleHighlight, None),
        KeyBinding::new("cmd-shift-h", ToggleHighlight, None),
        KeyBinding::new("ctrl-shift-c", ToggleInlineCode, None),
        KeyBinding::new("cmd-shift-c", ToggleInlineCode, None),
        // Start a wikilink from the caret, or from the selected text.
        KeyBinding::new("ctrl-k", InsertWikilink, None),
        KeyBinding::new("cmd-k", InsertWikilink, None),
        // Tab indents; Shift+Tab outdents. Autocomplete still accepts with Tab.
        KeyBinding::new("tab", Indent, None),
        KeyBinding::new("shift-tab", Outdent, None),
    ]);

    // macOS conventions: Command jumps to the line or document edge, Option works by word.
    #[cfg(target_os = "macos")]
    cx.bind_keys([
        KeyBinding::new("cmd-left", Home, None),
        KeyBinding::new("cmd-right", End, None),
        KeyBinding::new("cmd-up", DocumentStart, None),
        KeyBinding::new("cmd-down", DocumentEnd, None),
        KeyBinding::new("cmd-shift-left", SelectHome, None),
        KeyBinding::new("cmd-shift-right", SelectEnd, None),
        KeyBinding::new("cmd-shift-up", SelectDocumentStart, None),
        KeyBinding::new("cmd-shift-down", SelectDocumentEnd, None),
        KeyBinding::new("alt-backspace", DeleteWordLeft, None),
        KeyBinding::new("alt-delete", DeleteWordRight, None),
    ]);
}

/// Slash commands: `(inserted text, description)`. Typing `/desc` narrows the list.
/// Flatten a selection into something valid inside `[[target|alias]]`.
fn clean_link_alias(text: &str) -> String {
    let mut out = String::new();
    for word in text.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        for ch in word.chars() {
            match ch {
                '|' => out.push('-'),
                '[' | ']' => {}
                other => out.push(other),
            }
        }
    }
    out
}

/// Wrap selected text as a self-contained wikilink, e.g. `My Note` → `[[My Note]]`.
fn wikilink_for_selection(text: &str) -> Option<String> {
    let target = clean_link_alias(text);
    (!target.is_empty()).then(|| format!("[[{target}]]"))
}

/// Build the wikilink text for a suggestion, using the captured selection as an alias.
fn link_text(target: &str, label: &str, alias: Option<&str>) -> String {
    match alias
        .map(str::trim)
        .filter(|alias| !alias.is_empty() && !alias.eq_ignore_ascii_case(label))
    {
        Some(alias) => format!("[[{target}|{alias}]]"),
        None => format!("[[{target}]]"),
    }
}

const SLASH_COMMANDS: &[(&str, &str)] = &[
    ("# ", "h1 heading"),
    ("## ", "h2 heading"),
    ("### ", "h3 heading"),
    ("- ", "bullet list"),
    ("- [ ] ", "task todo"),
    ("1. ", "numbered list"),
    ("> ", "quote"),
    ("> [!note]\n> ", "callout note"),
    ("---\n", "divider rule"),
    ("```\n\n```", "code block fence"),
    ("$$\n\n$$", "math latex block"),
    ("| A | B |\n| --- | --- |\n|  |  |\n", "table"),
    ("[[]]", "wikilink"),
];

pub struct Editor {
    buffer: Buffer,
    path: Option<PathBuf>,
    /// Vault root, used to resolve embeds and attachments.
    vault_root: Option<PathBuf>,
    reading: bool,
    /// Selection anchor. `None` means no selection; the cursor is the active end.
    anchor: Option<usize>,
    selecting: bool,
    focus_handle: FocusHandle,
    font_size: Pixels,
    line_height: Pixels,
    marked_range: Option<Range<usize>>,
    /// Folder (vault-relative) that pasted images are filed into.
    attachments_folder: String,
    last_bounds: Option<Bounds<Pixels>>,
    /// Visual rows produced by the last paint, for hit-testing and IME.
    ///
    /// Kept behind an `Rc` so a frame can borrow the cache without copying every shaped
    /// line. That copy was the hot path while scrolling a long note.
    last_rows: Rc<Vec<RowMeta>>,
    last_shaped: Rc<Vec<Rc<ShapedLine>>>,
    /// Shaped layout per logical line, so a keystroke re-shapes one line instead of the
    /// whole document. Indexed by line number; `None` marks a line not yet shaped.
    line_cache: Vec<Option<LineLayout>>,
    /// Cache key: the buffer revision and wrap width the shaped rows were built for.
    cache_revision: u64,
    cache_width: f32,
    /// The `(caret, visual row)` the viewport was last scrolled to follow, so frames that
    /// do not move the caret leave the scroll position alone.
    caret_follow: Option<(usize, usize)>,
    /// Tracks the scroll offset so painting can skip off-screen rows.
    scroll_handle: ScrollHandle,
    /// Frontmatter properties, cached per buffer revision. Parsing them in `render` used
    /// to allocate a copy of the whole document on every scroll frame.
    properties: Vec<Property>,
    properties_revision: u64,
    /// Increments when a different document is loaded, so caches can tell "new note,
    /// revision 0" apart from "same note, revision 0".
    doc_generation: u64,
    /// Virtualized reading-mode content.
    reading_content: ReadingCache,
    /// Display-math blocks in the current document, with the rows they occupy.
    display_math: Rc<Vec<DisplayMathLayout>>,
    /// The buffer revision `display_math` was parsed from.
    display_math_revision: u64,
    /// Typeset display formulas, keyed by TeX source. `None` records a failed formula.
    math_scenes: HashMap<String, Option<Rc<MathScene>>>,
    /// The document text for one revision, shared by the heading and math parsers so a
    /// keystroke copies the note once rather than once per consumer.
    doc_text: ((u64, u64), Rc<String>),
    /// True while a splitter is being dragged: the text keeps its old wrap width so a
    /// resize does not re-shape the whole document on every mouse tick.
    resizing: bool,
    /// Opens a link clicked in reading mode.
    link_opener: Option<LinkOpener>,
    /// Asks the app to run Save As when a save has no file name yet.
    save_as: Option<SaveAsHandler>,
    /// Asks the app to close this tab.
    close_tab: Option<CloseTabHandler>,
    /// Reports one-line results (copy, save failures) to the app's status bar.
    status_reporter: Option<StatusReporter>,
    /// The version of the file this buffer was loaded from or last saved to.
    fingerprint: Option<DocumentFingerprint>,
    /// Tells the app when a save would overwrite an external change.
    conflict_handler: Option<ConflictHandler>,
    /// Debounce generation for the draft writer; a newer edit invalidates an older timer.
    draft_generation: u64,
    /// True while a draft timer is waiting.
    draft_armed: bool,
    /// The buffer revision the last draft was written for.
    draft_written: u64,
    /// Whether the note may be saved automatically after idle, when the disk still matches.
    autosave: bool,
    /// Cached document statistics, recomputed per buffer revision.
    stats_cache: (u64, stats::Stats),
    /// Git added/removed lines for the file, refreshed when the saved buffer changes.
    git_stat: Option<git::DiffStat>,
    git_stat_key: Option<(u64, bool)>,
    /// Extra carets/selections created with Ctrl+D or Ctrl+Alt+Up/Down.
    extra_cursors: Vec<CursorState>,
    /// The in-note find/replace bar.
    find: FindState,
    /// Optional Vim-style modal editing.
    vim: Vim,
    /// Heading indices whose sections are folded in the text view.
    folded: HashSet<usize>,
    /// Headings for the current document revision, used to compute fold ranges.
    heading_cache: (u64, u64, Rc<Vec<markdown::Heading>>),
    /// Bumped when `folded` changes so `lay_out` reflows.
    fold_revision: u64,
    /// The fold revision the current layout was built for.
    layout_fold_revision: u64,
    /// Per source line: 0 = no marker, 1 = expandable heading, 2 = folded heading.
    fold_markers: Rc<Vec<u8>>,
    /// Wikilink, tag and slash-command suggestions while typing.
    autocomplete: Autocomplete,
    /// `(insertion text, label)` for every note in the vault.
    note_suggestions: Rc<Vec<(String, String)>>,
    /// Every tag used in the vault.
    tag_suggestions: Rc<Vec<String>>,
    /// Selected text captured when a wikilink was started, used as the link alias.
    pending_link_alias: Option<String>,
    /// A `[` was pressed with a selection; the next text input wraps it as a wikilink.
    pending_link_start: bool,
    /// The selection to wrap, kept separately because the text-input handler can run
    /// after the platform has already collapsed the selection.
    pending_link_selection: Option<Range<usize>>,
    /// After auto-inserting `[[…]]`, ignore the second `[` the user was about to type.
    swallow_bracket: bool,
}

/// A caret plus its selection anchor. The primary caret lives in the buffer; extras live
/// here so the buffer's undo and text APIs stay simple.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CursorState {
    cursor: usize,
    anchor: Option<usize>,
}

/// One replacement to perform at a caret. `primary` keeps the primary caret identifiable
/// after edits shift every position.
#[derive(Debug, Clone)]
struct EditTarget {
    range: Range<usize>,
    text: String,
    primary: bool,
}

/// The next occurrence of `needle` at or after `from` (in characters).
fn literal_after(text: &str, needle: &str, from: usize) -> Option<usize> {
    if needle.is_empty() {
        return None;
    }
    for (char_index, (byte, _)) in text.char_indices().enumerate() {
        if char_index < from {
            continue;
        }
        if text[byte..].starts_with(needle) {
            return Some(char_index);
        }
    }
    None
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum FindField {
    #[default]
    Pattern,
    Replacement,
}

#[derive(Debug, Clone, Default)]
struct FindState {
    open: bool,
    replace: bool,
    pattern: String,
    replacement: String,
    field: FindField,
    regex: bool,
    case_sensitive: bool,
    whole_word: bool,
    error: Option<String>,
    /// Character ranges of every match in the current buffer.
    matches: Vec<Range<usize>>,
    current: usize,
    message: Option<String>,
}

impl FindState {
    fn options(&self) -> find_engine::Options {
        find_engine::Options {
            regex: self.regex,
            case_sensitive: self.case_sensitive,
            whole_word: self.whole_word,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SuggestionKind {
    Link,
    Tag,
    Slash,
}

#[derive(Debug, Clone)]
struct Suggestion {
    /// Primary text shown in the popup.
    label: String,
    /// Secondary text shown to the right of the name.
    detail: String,
    /// Text that replaces the trigger plus query.
    insert: String,
}

#[derive(Debug, Clone, Default)]
struct Autocomplete {
    open: bool,
    kind: Option<SuggestionKind>,
    /// Character offset where the replaced range starts (before `[[`, `#` or `/`).
    start: usize,
    query: String,
    items: Vec<Suggestion>,
    selected: usize,
}

/// The marker a new line continues, when Enter is pressed at the end of a list item.
fn list_continuation(line: &str, col: usize) -> Option<String> {
    if col != line.chars().count() {
        return None;
    }
    let trimmed = line.trim_start();
    let indent = &line[..line.len() - trimmed.len()];

    // Bullets and task items.
    for marker in ["- ", "* ", "+ "] {
        if let Some(rest) = trimmed.strip_prefix(marker) {
            if rest.trim().is_empty() {
                return None;
            }
            if rest.starts_with("[ ] ") || rest.starts_with("[x] ") || rest.starts_with("[X] ") {
                return Some(format!("{indent}{marker}[ ] "));
            }
            return Some(format!("{indent}{marker}"));
        }
    }

    // Ordered items keep their delimiter and increment.
    let digits: String = trimmed
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect();
    if !digits.is_empty() {
        let rest = &trimmed[digits.len()..];
        if let Some(after) = rest.strip_prefix(". ").or_else(|| rest.strip_prefix(") ")) {
            if after.trim().is_empty() {
                return None;
            }
            let next = digits.parse::<u64>().ok()?.saturating_add(1);
            let delimiter = if rest.starts_with(") ") { ")" } else { "." };
            return Some(format!("{indent}{next}{delimiter} "));
        }
    }

    // Blockquotes.
    if let Some(rest) = trimmed.strip_prefix("> ") {
        if rest.trim().is_empty() {
            return None;
        }
        return Some(format!("{indent}> "));
    }

    None
}

/// A wrapped visual row: `[start, end)` character columns of logical line `line`.
#[derive(Debug, Clone, Copy)]
struct RowMeta {
    line: usize,
    start: usize,
    end: usize,
}

/// A `$$…$$` block in the source, before its rows are known.
#[derive(Debug, Clone, PartialEq, Eq)]
struct DisplayMathSource {
    first_line: usize,
    last_line: usize,
    tex: String,
}

/// A display-math block plus the rows it occupies in the laid-out document.
#[derive(Debug, Clone)]
struct DisplayMathLayout {
    first_line: usize,
    last_line: usize,
    tex: String,
    first_row: usize,
    row_count: usize,
}

/// Find display-math blocks (`$$…$$`) by source line.
///
/// A block opens on a line that is exactly `$$` and closes on the next such line, or is a
/// complete `$$tex$$` on one line. Fenced code and inline `$…$` are ignored. An unclosed
/// block is not returned, so its source stays visible while it is being written.
fn display_math_blocks(text: &str) -> Vec<DisplayMathSource> {
    let mut blocks = Vec::new();
    let mut in_fence = false;
    let mut open: Option<(usize, Vec<&str>)> = None;

    for (index, line) in text.lines().enumerate() {
        let trimmed = line.trim();

        if let Some((first_line, body)) = &mut open {
            if trimmed == "$$" {
                let tex = body.join("\n");
                if !tex.trim().is_empty() {
                    blocks.push(DisplayMathSource {
                        first_line: *first_line,
                        last_line: index,
                        tex,
                    });
                }
                open = None;
            } else {
                body.push(line);
            }
            continue;
        }

        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }

        if trimmed == "$$" {
            open = Some((index, Vec::new()));
            continue;
        }

        if trimmed.len() > 4 && trimmed.starts_with("$$") && trimmed.ends_with("$$") {
            let tex = trimmed[2..trimmed.len() - 2].trim();
            if !tex.is_empty() {
                blocks.push(DisplayMathSource {
                    first_line: index,
                    last_line: index,
                    tex: tex.to_string(),
                });
            }
        }
    }

    blocks
}

/// The shaped rows of one logical line, reused across frames.
///
/// Shaping (measuring, wrapping and laying out glyphs) is by far the most expensive part
/// of drawing the editor, so it is done per line and only redone when that line — or the
/// fence state it depends on, or the wrap width — changes. `Rc` keeps reuse free.
#[derive(Clone)]
struct LineLayout {
    /// The line's text, compared against the buffer to detect edits.
    text: String,
    /// Whether the line was tokenised as inside a fenced code block.
    in_code: bool,
    /// Fence language at the time the line was shaped, so a changed fence re-shapes.
    language: String,
    /// Wrapped spans of the line, in character columns.
    spans: Rc<Vec<(usize, usize)>>,
    /// One shaped row per span. Shared with the flattened row list, so flattening the
    /// document costs a pointer per row rather than a copy of every shaped line.
    rows: Rc<Vec<Rc<ShapedLine>>>,
}

impl Editor {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            buffer: Buffer::new(),
            path: None,
            vault_root: None,
            reading: false,
            anchor: None,
            selecting: false,
            focus_handle: cx.focus_handle(),
            font_size: px(14.0),
            line_height: px(20.0),
            marked_range: None,
            last_bounds: None,
            last_rows: Rc::new(Vec::new()),
            last_shaped: Rc::new(Vec::new()),
            line_cache: Vec::new(),
            cache_revision: u64::MAX,
            cache_width: 0.0,
            caret_follow: None,
            scroll_handle: ScrollHandle::new(),
            properties: Vec::new(),
            properties_revision: u64::MAX,
            doc_generation: 0,
            reading_content: ReadingCache::new(),
            display_math: Rc::new(Vec::new()),
            display_math_revision: u64::MAX,
            math_scenes: HashMap::new(),
            doc_text: ((u64::MAX, u64::MAX), Rc::new(String::new())),
            resizing: false,
            attachments_folder: "attachments".to_string(),
            link_opener: None,
            save_as: None,
            close_tab: None,
            status_reporter: None,
            fingerprint: None,
            conflict_handler: None,
            draft_generation: 0,
            draft_armed: false,
            draft_written: u64::MAX,
            autosave: false,
            stats_cache: (u64::MAX, stats::Stats::default()),
            git_stat: None,
            git_stat_key: None,
            extra_cursors: Vec::new(),
            find: FindState::default(),
            vim: Vim::new(),
            folded: HashSet::new(),
            heading_cache: (u64::MAX, u64::MAX, Rc::new(Vec::new())),
            fold_revision: 0,
            layout_fold_revision: u64::MAX,
            fold_markers: Rc::new(Vec::new()),
            autocomplete: Autocomplete::default(),
            note_suggestions: Rc::new(Vec::new()),
            tag_suggestions: Rc::new(Vec::new()),
            pending_link_alias: None,
            pending_link_start: false,
            pending_link_selection: None,
            swallow_bracket: false,
        }
    }

    /// Replace the vault-wide suggestion lists used while typing.
    pub fn set_suggestions(&mut self, notes: Vec<(String, String)>, tags: Vec<String>) {
        self.note_suggestions = Rc::new(notes);
        self.tag_suggestions = Rc::new(tags);
    }

    /// Whether editing actions may change the buffer. Reading mode is view-only: every
    /// mutating action checks this gate, while scrolling, selecting and copying stay on.
    fn can_mutate(&self) -> bool {
        !self.reading
    }

    /// Wrap or unwrap the current selection (or insert a marker pair at the caret).
    fn apply_style(&mut self, style: text_format::Style) {
        if !self.can_mutate() || self.find.open {
            return;
        }
        let range = self
            .selection()
            .unwrap_or_else(|| self.buffer.cursor()..self.buffer.cursor());
        let text = self.buffer.text();
        let edit = text_format::toggle(&text, range, style);
        let base = edit.range.start;
        self.buffer.replace(edit.range, &edit.text);

        let start = base + edit.selection.start;
        let end = base + edit.selection.end;
        if start == end {
            self.anchor = None;
            self.buffer.set_cursor(start);
        } else {
            self.anchor = Some(start);
            self.buffer.set_cursor(end);
        }
        self.autocomplete = Autocomplete::default();
        self.pending_link_alias = None;
        self.pending_link_start = false;
        self.swallow_bracket = false;
    }

    /// Wrap the pending selection as `[[selected text]]`. Returns false when the selection
    /// was empty, in which case the caller opens the note picker instead.
    fn wrap_selection_as_wikilink(&mut self, swallow_second_bracket: bool) -> bool {
        let range = self
            .pending_link_selection
            .take()
            .or_else(|| self.selection());
        let Some(range) = range else {
            return false;
        };
        let target = clean_link_alias(&self.buffer.slice(range.clone()));
        let target = if target.is_empty() {
            self.pending_link_alias.clone().unwrap_or_default()
        } else {
            target
        };
        let Some(link) = wikilink_for_selection(&target) else {
            return false;
        };

        self.buffer.replace(range, &link);
        self.anchor = None;
        self.autocomplete = Autocomplete::default();
        self.pending_link_alias = None;
        self.pending_link_start = false;
        self.pending_link_selection = None;
        self.swallow_bracket = swallow_second_bracket;
        self.refresh_autocomplete();
        true
    }

    /// Start a wikilink at the caret, or from the selected text as an alias.
    fn start_wikilink(&mut self) {
        if self.reading || self.find.open {
            return;
        }
        self.autocomplete = Autocomplete::default();
        self.pending_link_start = false;
        self.pending_link_selection = None;
        self.swallow_bracket = false;
        if let Some(range) = self.selection() {
            let alias = clean_link_alias(&self.buffer.slice(range.clone()));
            self.pending_link_alias = (!alias.is_empty()).then_some(alias);
            self.buffer.replace(range, "[[");
            self.anchor = None;
        } else {
            self.pending_link_alias = None;
            self.buffer.insert("[[");
        }
        self.refresh_autocomplete();
    }

    /// Remember a selection when the user starts typing `[[` over it, so the picker can
    /// offer `[[target|selection]]`.
    fn capture_link_alias(&mut self, typed: &str) {
        if typed == "[" || typed == "[[" {
            if let Some(range) = self.selection() {
                let alias = clean_link_alias(&self.buffer.slice(range.clone()));
                self.pending_link_alias = (!alias.is_empty()).then_some(alias);
                self.pending_link_selection = Some(range);
            }
        }
    }

    fn dismiss_autocomplete(&mut self) {
        self.autocomplete = Autocomplete::default();
        self.pending_link_alias = None;
        self.pending_link_start = false;
        self.pending_link_selection = None;
        self.swallow_bracket = false;
    }

    /// Recompute the popup from the text before the caret.
    fn refresh_autocomplete(&mut self) {
        if self.reading || self.find.open {
            self.autocomplete = Autocomplete::default();
            self.pending_link_alias = None;
            return;
        }

        let (line, col) = self.buffer.line_col();
        let text = self.buffer.line(line);
        let head: String = text.chars().take(col).collect();

        // Slash commands: the line starts with `/` and the caret is inside the first word.
        let trimmed = head.trim_start();
        if trimmed.starts_with('/') && !trimmed.chars().any(char::is_whitespace) {
            let start_col = col.saturating_sub(trimmed.chars().count());
            let start = self.buffer.char_index_at(line, start_col);
            let query = trimmed.trim_start_matches('/').to_string();
            self.pending_link_alias = None;
            self.show_suggestions(SuggestionKind::Slash, start, query);
            return;
        }

        // Wikilinks: the most recent `[[` is still open.
        if let Some(byte) = head.rfind("[[") {
            let after = &head[byte + 2..];
            if !after.contains("]]") && !after.contains('\n') {
                let start_col = col.saturating_sub(head[byte..].chars().count());
                let start = self.buffer.char_index_at(line, start_col);
                self.show_suggestions(SuggestionKind::Link, start, after.to_string());
                return;
            }
        }

        // Tags: `#` at a word boundary and no whitespace after it.
        if let Some(byte) = head.rfind('#') {
            let before = head[..byte].chars().next_back();
            let boundary = before.map(|ch| !ch.is_alphanumeric()).unwrap_or(true);
            let after = &head[byte + 1..];
            if boundary && !after.is_empty() && !after.chars().any(char::is_whitespace) {
                let start_col = col.saturating_sub(head[byte..].chars().count());
                let start = self.buffer.char_index_at(line, start_col);
                self.pending_link_alias = None;
                self.show_suggestions(SuggestionKind::Tag, start, after.to_string());
                return;
            }
        }

        // Keep an alias only while the user is between the two opening brackets.
        if !head.ends_with('[') {
            self.pending_link_alias = None;
        }
        self.autocomplete = Autocomplete::default();
    }

    fn show_suggestions(&mut self, kind: SuggestionKind, start: usize, query: String) {
        let needle = query.to_lowercase();
        let mut items = Vec::new();

        match kind {
            SuggestionKind::Link => {
                let alias = self.pending_link_alias.clone();
                for (target, label) in self.note_suggestions.iter() {
                    if needle.is_empty()
                        || label.to_lowercase().contains(&needle)
                        || target.to_lowercase().contains(&needle)
                    {
                        items.push(Suggestion {
                            label: label.clone(),
                            detail: target.clone(),
                            insert: link_text(target, label, alias.as_deref()),
                        });
                        if items.len() == 8 {
                            break;
                        }
                    }
                }
            }
            SuggestionKind::Tag => {
                for tag in self.tag_suggestions.iter() {
                    if needle.is_empty() || tag.to_lowercase().contains(&needle) {
                        items.push(Suggestion {
                            label: format!("#{tag}"),
                            detail: "tag".to_string(),
                            insert: format!("#{tag}"),
                        });
                        if items.len() == 8 {
                            break;
                        }
                    }
                }
            }
            SuggestionKind::Slash => {
                for (insert, description) in SLASH_COMMANDS {
                    if needle.is_empty() || description.contains(&needle) {
                        items.push(Suggestion {
                            label: (*description).to_string(),
                            detail: (*insert).replace('\n', "\\n"),
                            insert: (*insert).to_string(),
                        });
                        if items.len() == 8 {
                            break;
                        }
                    }
                }
            }
        }

        if items.is_empty() {
            self.autocomplete = Autocomplete::default();
            if kind == SuggestionKind::Link {
                self.pending_link_alias = None;
            }
            return;
        }

        self.autocomplete = Autocomplete {
            open: true,
            kind: Some(kind),
            start,
            query,
            items,
            selected: 0,
        };
    }

    /// Insert the highlighted suggestion. Returns true when one was accepted.
    fn accept_autocomplete(&mut self) -> bool {
        if !self.autocomplete.open {
            return false;
        }
        let Some(item) = self
            .autocomplete
            .items
            .get(self.autocomplete.selected)
            .cloned()
        else {
            self.autocomplete = Autocomplete::default();
            self.pending_link_alias = None;
            return false;
        };
        let cursor = self.buffer.cursor();
        let start = self.autocomplete.start.min(cursor);
        self.buffer.replace(start..cursor, &item.insert);
        self.anchor = None;
        self.autocomplete = Autocomplete::default();
        self.pending_link_alias = None;
        self.pending_link_start = false;
        self.swallow_bracket = false;
        true
    }

    fn move_autocomplete(&mut self, delta: isize) {
        if !self.autocomplete.open || self.autocomplete.items.is_empty() {
            return;
        }
        let count = self.autocomplete.items.len() as isize;
        let selected = (self.autocomplete.selected as isize + delta).rem_euclid(count);
        self.autocomplete.selected = selected as usize;
    }

    /// Load a note from disk, replacing the buffer.
    pub fn load(&mut self, path: &Path) -> std::io::Result<()> {
        let text = std::fs::read_to_string(path)?;
        self.buffer = Buffer::from_text(&text);
        self.buffer.mark_saved();
        self.fingerprint = Some(DocumentFingerprint::of_text(&text));
        self.marked_range = None;
        self.anchor = None;
        self.selecting = false;
        self.extra_cursors.clear();
        self.find.open = false;
        self.find.matches.clear();
        self.find.error = None;
        self.find.message = None;
        self.autocomplete = Autocomplete::default();
        self.vim.reset();
        self.reset_layout_cache();
        self.scroll_handle = ScrollHandle::new();
        self.properties_revision = u64::MAX;
        self.doc_generation = self.doc_generation.wrapping_add(1);
        self.math_scenes.clear();
        self.draft_written = u64::MAX;
        self.draft_generation = self.draft_generation.wrapping_add(1);
        self.path = Some(path.to_path_buf());
        Ok(())
    }

    /// Write the buffer back to its file, atomically and only if the file on disk is still
    /// the version this buffer is based on. Returns a report so the caller can offer the
    /// conflict choices instead of losing either side.
    pub fn save_checked(&mut self) -> SaveReport {
        let Some(path) = self.path.clone() else {
            return SaveReport::Error {
                message: "note has no file name yet (use Save note as)".to_string(),
            };
        };

        let new_content = self.buffer.text();
        // Read the old version before the write, so the history snapshot is the one that
        // is about to be replaced.
        let previous = std::fs::read_to_string(&path).ok();

        // Never overwrite a file that changed underneath the editor.
        let report = fingerprint::save_if_unchanged(&path, &new_content, self.fingerprint.as_ref());

        if let SaveReport::Saved { fingerprint } = &report {
            // Keep a snapshot of whatever was overwritten.
            if let (Some(root), Some(existing)) = (self.vault_root.as_ref(), previous) {
                if existing != new_content {
                    if let Ok(relative) = path.strip_prefix(root) {
                        let relative = relative.to_string_lossy().replace('\\', "/");
                        let _ = history::record(root, &relative, &existing);
                    }
                }
            }
            self.buffer.mark_saved();
            self.fingerprint = Some(fingerprint.clone());
            // The note on disk is now the typed text, so its draft is obsolete.
            if let (Some(root), Some(path)) = (self.vault_root.as_ref(), self.path.as_ref()) {
                if let Ok(relative) = path.strip_prefix(root) {
                    let note = relative.to_string_lossy().replace('\\', "/");
                    drafts::discard(root, &note);
                }
            }
        }

        report
    }

    /// Write the buffer over whatever is on disk. Only "keep mine" calls this, after the
    /// user has seen both versions and chosen this one.
    pub fn save_forced(&mut self) -> SaveReport {
        let Some(path) = self.path.clone() else {
            return SaveReport::Error {
                message: "note has no file name yet (use Save note as)".to_string(),
            };
        };
        let report = fingerprint::save_forced(&path, &self.buffer.text());
        if let SaveReport::Saved { fingerprint } = &report {
            self.buffer.mark_saved();
            self.fingerprint = Some(fingerprint.clone());
        }
        report
    }

    /// Write the buffer back to its file, atomically. Conflicts surface as an error; use
    /// [`Editor::save_checked`] when the caller wants to tell them apart.
    pub fn save(&mut self) -> std::io::Result<()> {
        match self.save_checked() {
            SaveReport::Saved { .. } => Ok(()),
            SaveReport::Conflict { disk } => Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                if disk.is_some() {
                    "the note changed on disk since it was opened"
                } else {
                    "the note was deleted on disk"
                },
            )),
            SaveReport::Error { message } => Err(std::io::Error::other(message)),
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.buffer.is_dirty()
    }

    /// Whether this editor is backed by a file (false for an untitled buffer).
    pub fn has_path(&self) -> bool {
        self.path.is_some()
    }

    /// Re-read the note from disk, discarding in-memory edits.
    pub fn reload(&mut self) -> std::io::Result<()> {
        let Some(path) = self.path.clone() else {
            return Ok(());
        };
        let text = std::fs::read_to_string(&path)?;
        // Our own atomic save also trips the watcher; do not treat it as an external
        // change, which would reset the editor and preview scroll positions.
        if text == self.buffer.text() {
            self.buffer.mark_saved();
            self.fingerprint = Some(DocumentFingerprint::of_text(&text));
            return Ok(());
        }
        self.buffer = Buffer::from_text(&text);
        self.buffer.mark_saved();
        self.fingerprint = Some(DocumentFingerprint::of_text(&text));
        self.anchor = None;
        self.marked_range = None;
        self.reset_layout_cache();
        self.scroll_handle = ScrollHandle::new();
        self.properties_revision = u64::MAX;
        self.doc_generation = self.doc_generation.wrapping_add(1);
        self.math_scenes.clear();
        self.draft_written = u64::MAX;
        self.draft_generation = self.draft_generation.wrapping_add(1);
        self.reading_content.invalidate_attachments();
        Ok(())
    }

    /// Rebuild the reading view's attachment index on the next render.
    pub fn invalidate_attachments(&mut self) {
        self.reading_content.invalidate_attachments();
    }

    /// Toggle between edit and reading mode (used by the command palette).
    pub fn toggle_reading_mode(&mut self) {
        self.reading = !self.reading;
        if self.reading {
            self.find.open = false;
        }
    }

    /// Whether the editor is currently in reading mode.
    pub fn is_reading(&self) -> bool {
        self.reading
    }

    /// Set reading mode explicitly (used when restoring a workspace).
    pub fn set_reading_mode(&mut self, reading: bool) {
        self.reading = reading;
        if reading {
            self.find.open = false;
        }
    }

    /// Insert text at the caret (used by plugins).
    pub fn insert_at_cursor(&mut self, text: &str) {
        self.insert_text(text);
    }

    /// Append text to the end of the note.
    pub fn append_to_note(&mut self, text: &str) {
        self.buffer.set_cursor(self.buffer.len_chars());
        self.buffer.insert(text);
        self.anchor = None;
    }

    /// 0-based line containing the caret.
    pub fn cursor_line(&self) -> usize {
        self.buffer.line_col().0
    }

    /// Caret line and column, 0-based.
    pub fn cursor_position(&self) -> (usize, usize) {
        self.buffer.line_col()
    }

    /// Put the caret at a line and column, clamped to the document.
    pub fn set_cursor_position(&mut self, line: usize, col: usize) {
        let line = line.min(self.buffer.line_count().saturating_sub(1));
        self.buffer.set_cursor(self.buffer.char_index_at(line, col));
        self.anchor = None;
        self.extra_cursors.clear();
    }

    /// The editor's vertical scroll offset, in pixels.
    pub fn scroll_offset(&self) -> f32 {
        f32::from(self.scroll_handle.offset().y)
    }

    /// Restore the editor's vertical scroll offset.
    pub fn set_scroll_offset(&mut self, offset: f32) {
        let x = self.scroll_handle.offset().x;
        self.scroll_handle.set_offset(point(x, px(offset.max(0.0))));
    }

    /// The reading view's top block.
    pub fn reading_block(&self) -> usize {
        self.reading_content.top_block()
    }

    /// Restore the reading view's scroll position by block.
    pub fn scroll_reading_to(&mut self, block: usize) {
        self.reading_content.scroll_to_block(block);
    }

    /// The reading view's current section and progress, for the status bar and outline.
    pub fn reading_outline_state(&self) -> (Option<String>, f32) {
        (
            self.reading_content.current_heading(),
            self.reading_content.progress(),
        )
    }

    /// Put the caret at the start of a source line, clamped to the document.
    pub fn set_cursor_line(&mut self, line: usize) {
        let line = line.min(self.buffer.line_count().saturating_sub(1));
        self.buffer.set_cursor(self.buffer.char_index_at(line, 0));
        self.anchor = None;
        self.extra_cursors.clear();
    }

    /// Put the caret on a source line and select `needle` when it appears there.
    ///
    /// Search results use this so the match is already selected when the note opens.
    pub fn select_line_text(&mut self, line: usize, needle: &str) {
        let line = line.min(self.buffer.line_count().saturating_sub(1));
        self.buffer.set_cursor(self.buffer.char_index_at(line, 0));
        self.anchor = None;
        self.extra_cursors.clear();
        if needle.is_empty() {
            return;
        }
        let text = self.buffer.line(line);
        let Some(byte) = text.find(needle) else {
            return;
        };
        let start_col = text[..byte].chars().count();
        let start = self.buffer.char_index_at(line, start_col);
        let end = self
            .buffer
            .char_index_at(line, start_col + needle.chars().count());
        self.anchor = Some(start);
        self.buffer.set_cursor(end);
    }

    /// The selected text, if there is a non-empty selection.
    pub fn selection_text(&self) -> Option<String> {
        self.selection().map(|range| self.buffer.slice(range))
    }

    /// The block around the caret, as source text.
    pub fn current_block_text(&self) -> Option<String> {
        let text = self.buffer.text();
        let block = blocks::block_at(&text, self.cursor_line())?;
        Some(blocks::text_of(&text, &block))
    }

    /// Replace the selection when there is one, otherwise insert at the caret.
    pub fn replace_selection_or_insert(&mut self, text: &str) {
        if let Some(range) = self.selection() {
            self.buffer.replace(range, text);
            self.anchor = None;
        } else {
            self.insert_text(text);
        }
    }

    /// Insert a template at the caret and honour a `{{cursor}}` marker.
    pub fn insert_template(&mut self, text: &str) {
        let (text, cursor) = templates::strip_cursor(text);
        let start = self
            .selection()
            .map(|range| range.start)
            .unwrap_or_else(|| self.buffer.cursor());
        self.insert_text(&text);
        if let Some(offset) = cursor {
            self.buffer.set_cursor(start + offset);
        }
    }

    /// Move the block at the caret up. Returns whether anything changed.
    pub fn move_block_up(&mut self) -> bool {
        let text = self.buffer.text();
        match blocks::move_up(&text, self.cursor_line()) {
            Some(edit) => {
                self.apply_block_edit(edit);
                true
            }
            None => false,
        }
    }

    /// Move the block at the caret down. Returns whether anything changed.
    pub fn move_block_down(&mut self) -> bool {
        let text = self.buffer.text();
        match blocks::move_down(&text, self.cursor_line()) {
            Some(edit) => {
                self.apply_block_edit(edit);
                true
            }
            None => false,
        }
    }

    /// Duplicate the block at the caret.
    pub fn duplicate_block(&mut self) -> bool {
        let text = self.buffer.text();
        match blocks::duplicate(&text, self.cursor_line()) {
            Some(edit) => {
                self.apply_block_edit(edit);
                true
            }
            None => false,
        }
    }

    /// Delete the block at the caret.
    pub fn delete_block(&mut self) -> bool {
        let text = self.buffer.text();
        match blocks::delete(&text, self.cursor_line()) {
            Some(edit) => {
                self.apply_block_edit(edit);
                true
            }
            None => false,
        }
    }

    fn apply_block_edit(&mut self, edit: blocks::BlockEdit) {
        if !self.can_mutate() {
            return;
        }
        self.buffer.replace(0..self.buffer.len_chars(), &edit.text);
        self.buffer
            .set_cursor(self.buffer.char_index_at(edit.line, 0));
        self.anchor = None;
        self.marked_range = None;
        self.properties_revision = u64::MAX;
        self.reset_layout_cache();
    }

    /// Drop the shaped-line cache. The next prepaint re-shapes the whole document.
    ///
    /// Used when the document is replaced or the palette changes, both of which
    /// invalidate every cached run.
    fn reset_layout_cache(&mut self) {
        self.line_cache.clear();
        self.last_rows = Rc::new(Vec::new());
        self.last_shaped = Rc::new(Vec::new());
        self.cache_revision = u64::MAX;
        self.caret_follow = None;
        // Folds are keyed by heading index, which a replaced document invalidates.
        self.folded.clear();
        self.fold_revision = self.fold_revision.wrapping_add(1);
    }

    /// Force the shaped-line cache to rebuild on the next frame, keeping hit-testing
    /// geometry valid until then. Called when the theme changes.
    pub fn invalidate_layout(&mut self) {
        self.line_cache.clear();
        self.cache_revision = u64::MAX;
    }

    /// Freeze text wrapping while a splitter drag changes the editor width, then reflow
    /// once when it ends. Re-shaping the whole note on every mouse tick made resizing crawl.
    /// Change the editor text size. Re-shapes on the next frame.
    pub fn set_font_size(&mut self, size: Pixels) {
        if self.font_size == size {
            return;
        }
        self.font_size = size;
        self.line_height = size * 1.4286;
        self.invalidate_layout();
    }

    /// Point this editor at a moved/renamed file without discarding the buffer.
    pub fn set_path(&mut self, path: Option<PathBuf>) {
        self.path = path;
        self.git_stat = None;
        self.git_stat_key = None;
    }

    pub fn set_resizing(&mut self, resizing: bool) {
        if self.resizing == resizing {
            return;
        }
        self.resizing = resizing;
        if !resizing {
            self.cache_width = 0.0;
        }
    }

    /// Headings for the current document, cached by document identity and revision.
    fn document_headings(&mut self) -> Rc<Vec<markdown::Heading>> {
        let key = (self.doc_generation, self.buffer.revision());
        if self.heading_cache.0 != key.0 || self.heading_cache.1 != key.1 {
            let text = self.document_text();
            self.heading_cache = (key.0, key.1, Rc::new(markdown::headings(&text)));
        }
        self.heading_cache.2.clone()
    }

    /// The document text, copied at most once per revision and shared by its consumers.
    fn document_text(&mut self) -> Rc<String> {
        let key = (self.doc_generation, self.buffer.revision());
        if self.doc_text.0 != key {
            self.doc_text = (key, Rc::new(self.buffer.text()));
        }
        self.doc_text.1.clone()
    }

    /// Whether a heading has at least one heading nested under it.
    fn heading_has_children(headings: &[markdown::Heading], index: usize) -> bool {
        headings
            .get(index + 1)
            .is_some_and(|next| next.level > headings[index].level)
    }

    /// The 0-based line where the section under `index` stops (exclusive).
    fn fold_end(headings: &[markdown::Heading], index: usize, line_count: usize) -> usize {
        headings[index + 1..]
            .iter()
            .find(|next| next.level <= headings[index].level)
            .map(|next| next.line.saturating_sub(1))
            .unwrap_or(line_count)
            .min(line_count)
    }

    /// Whether a line is the underline of a setext heading (`===` or `---`).
    fn is_setext_underline(text: &str) -> bool {
        let trimmed = text.trim();
        !trimmed.is_empty()
            && (trimmed.chars().all(|ch| ch == '=') || trimmed.chars().all(|ch| ch == '-'))
    }

    /// Mark the source lines hidden by folded headings.
    fn hidden_lines(
        headings: &[markdown::Heading],
        folded: &HashSet<usize>,
        line_count: usize,
    ) -> Vec<bool> {
        let mut hidden = vec![false; line_count];
        for (index, heading) in headings.iter().enumerate() {
            if !folded.contains(&index) {
                continue;
            }
            let start = heading.line.saturating_sub(1);
            if start >= line_count {
                continue;
            }
            let end = Self::fold_end(headings, index, line_count);
            for slot in hidden.iter_mut().take(end).skip(start + 1) {
                *slot = true;
            }
        }
        hidden
    }

    /// Per-line gutter markers: 1 for an expandable heading, 2 for a folded one.
    fn fold_markers(
        headings: &[markdown::Heading],
        folded: &HashSet<usize>,
        line_count: usize,
    ) -> Vec<u8> {
        let mut markers = vec![0u8; line_count];
        for (index, heading) in headings.iter().enumerate() {
            if !Self::heading_has_children(headings, index) {
                continue;
            }
            let line = heading.line.saturating_sub(1);
            if line < line_count {
                markers[line] = if folded.contains(&index) { 2 } else { 1 };
            }
        }
        markers
    }

    /// Fold or unfold a heading's section. Returns false when there is nothing to fold.
    fn toggle_fold_index(&mut self, index: usize) -> bool {
        let headings = self.document_headings();
        if index >= headings.len() || !Self::heading_has_children(&headings, index) {
            return false;
        }

        if !self.folded.remove(&index) {
            self.folded.insert(index);
        }
        self.fold_revision = self.fold_revision.wrapping_add(1);

        // A caret inside a section that just closed would be invisible: move it to the
        // heading so typing continues somewhere the user can see.
        if self.folded.contains(&index) {
            let start = headings[index].line.saturating_sub(1);
            let end = Self::fold_end(&headings, index, self.buffer.line_count());
            let (line, _) = self.buffer.line_col();
            if line > start && line < end {
                let col = self.buffer.line(start).chars().count();
                self.buffer
                    .set_cursor(self.buffer.char_index_at(start, col));
                self.anchor = None;
            }
        }
        true
    }

    /// Fold or unfold the heading containing the caret.
    pub fn toggle_fold_at_cursor(&mut self) -> bool {
        let headings = self.document_headings();
        let (line, _) = self.buffer.line_col();
        let index = headings
            .iter()
            .enumerate()
            .rev()
            .find(|(_, heading)| heading.line.saturating_sub(1) <= line)
            .map(|(index, _)| index);
        index.is_some_and(|index| self.toggle_fold_index(index))
    }

    /// Fold or unfold the heading containing the caret.
    fn toggle_fold(&mut self, _: &ToggleFold, _window: &mut Window, cx: &mut Context<Self>) {
        if self.toggle_fold_at_cursor() {
            cx.notify();
        }
    }

    /// Fold or unfold the heading that starts on `line` (0-based).
    fn toggle_fold_line(&mut self, line: usize) -> bool {
        let headings = self.document_headings();
        let index = headings.iter().enumerate().find_map(|(index, heading)| {
            (heading.line.saturating_sub(1) == line && Self::heading_has_children(&headings, index))
                .then_some(index)
        });
        index.is_some_and(|index| self.toggle_fold_index(index))
    }

    /// Unfold any section that hides the caret, so keyboard jumps cannot leave it invisible.
    fn unfold_cursor_section(&mut self) {
        if self.folded.is_empty() {
            return;
        }
        let (line, _) = self.buffer.line_col();
        let headings = self.document_headings();
        let line_count = self.buffer.line_count();
        let mut changed = false;
        for (index, heading) in headings.iter().enumerate() {
            if !self.folded.contains(&index) {
                continue;
            }
            let start = heading.line.saturating_sub(1);
            let end = Self::fold_end(&headings, index, line_count);
            if line > start && line < end {
                self.folded.remove(&index);
                changed = true;
            }
        }
        if changed {
            self.fold_revision = self.fold_revision.wrapping_add(1);
        }
    }

    /// Re-shape the document, reusing per-line work.
    ///
    /// Only lines whose text, fence state or wrap width changed are shaped again, so
    /// typing in a long note costs one line of work rather than the whole file.
    fn lay_out(&mut self, window: &mut Window, theme: Theme, width: f32, color: gpui::Hsla) {
        // While a splitter is dragged, keep shaping at the old width; the drag ends with
        // `set_resizing(false)`, which forces one reflow at the final width.
        let width = if self.resizing && self.cache_width > 0.0 {
            self.cache_width
        } else {
            width
        };
        let revision = self.buffer.revision();
        let width_changed = (self.cache_width - width).abs() >= 0.5;
        let folds_changed = self.layout_fold_revision != self.fold_revision;
        if !width_changed
            && self.cache_revision == revision
            && !folds_changed
            && !self.last_shaped.is_empty()
        {
            return;
        }
        if width_changed {
            // Every row's wrapping depends on the width.
            self.line_cache.clear();
        }

        let mut font = window.text_style().font();
        // A monospace face for editing, with per-platform fallbacks.
        font.family = "Cascadia Mono".into();
        font.fallbacks = Some(gpui::FontFallbacks(std::sync::Arc::new(vec![
            "SF Mono".to_string(),
            "Menlo".to_string(),
            "Consolas".to_string(),
            "DejaVu Sans Mono".to_string(),
            "Liberation Mono".to_string(),
            "monospace".to_string(),
        ])));

        let font_size = self.font_size;
        let count = self.buffer.line_count().max(1);
        self.align_line_cache(count);

        // Headings drive both the fold ranges and the gutter markers, so they are parsed
        // once per document revision even when nothing is folded.
        let headings = self.document_headings();
        let mut hidden = Self::hidden_lines(&headings, &self.folded, count);
        // A setext heading's underline is part of the heading, not its body.
        for (index, heading) in headings.iter().enumerate() {
            if !self.folded.contains(&index) {
                continue;
            }
            let start = heading.line.saturating_sub(1);
            if start + 1 >= count || hidden[start] {
                continue;
            }
            if !self.buffer.line(start).trim_start().starts_with('#')
                && Self::is_setext_underline(&self.buffer.line(start + 1))
            {
                hidden[start + 1] = false;
            }
        }
        let markers = Self::fold_markers(&headings, &self.folded, count);

        // `$$…$$` blocks are drawn as typeset formulas whenever the caret is outside them.
        // Blocks are re-parsed only when the text changes; their row spans are rebuilt on
        // every full layout because folds and wrapping move rows.
        let sources: Vec<DisplayMathSource> = if self.display_math_revision == revision {
            self.display_math
                .iter()
                .map(|block| DisplayMathSource {
                    first_line: block.first_line,
                    last_line: block.last_line,
                    tex: block.tex.clone(),
                })
                .collect()
        } else {
            let text = self.document_text();
            display_math_blocks(text.as_str())
        };
        self.display_math_revision = revision;

        let mut blocks: Vec<DisplayMathLayout> = sources
            .into_iter()
            .map(|source| DisplayMathLayout {
                first_line: source.first_line,
                last_line: source.last_line,
                tex: source.tex,
                first_row: usize::MAX,
                row_count: 0,
            })
            .collect();
        let mut block_start: HashMap<usize, usize> = HashMap::new();
        let mut block_end: HashMap<usize, usize> = HashMap::new();
        for (index, block) in blocks.iter().enumerate() {
            block_start.insert(block.first_line, index);
            block_end.insert(block.last_line, index);
        }

        let mut in_code = false;
        let mut language = String::new();
        let mut rows: Vec<Rc<ShapedLine>> = Vec::new();
        let mut metas: Vec<RowMeta> = Vec::new();

        for (line_index, &is_hidden) in hidden.iter().enumerate().take(count) {
            if is_hidden {
                continue;
            }
            let reused = match self.line_cache.get(line_index).and_then(Option::as_ref) {
                Some(cached)
                    if cached.in_code == in_code
                        && cached.language == language
                        && self.buffer.line_eq(line_index, &cached.text) =>
                {
                    // Clone the layout, not the `&LineLayout`.
                    Some(LineLayout::clone(cached))
                }
                _ => None,
            };

            let layout = match reused {
                Some(layout) => layout,
                None => {
                    let text = self.buffer.line(line_index);
                    LineLayout::shape(
                        &text, in_code, &language, &font, font_size, width, color, theme, window,
                    )
                }
            };

            if syntax::is_fence(&layout.text) {
                if in_code {
                    language.clear();
                } else {
                    language = fence_language(&layout.text);
                }
                in_code = !in_code;
            }

            if let Some(&block) = block_start.get(&line_index) {
                blocks[block].first_row = rows.len();
            }
            for span in layout.spans.iter() {
                metas.push(RowMeta {
                    line: line_index,
                    start: span.0,
                    end: span.1,
                });
            }
            rows.extend(layout.rows.iter().cloned());
            if let Some(&block) = block_end.get(&line_index) {
                let first_row = blocks[block].first_row;
                if first_row != usize::MAX {
                    blocks[block].row_count = rows.len().saturating_sub(first_row);
                }
            }

            if line_index < self.line_cache.len() {
                self.line_cache[line_index] = Some(layout);
            } else {
                self.line_cache.push(Some(layout));
            }
        }
        self.line_cache.truncate(count);

        self.display_math = Rc::new(blocks);
        self.last_rows = Rc::new(metas);
        self.last_shaped = Rc::new(rows);
        self.cache_revision = revision;
        self.cache_width = width;
        self.fold_markers = Rc::new(markers);
        self.layout_fold_revision = self.fold_revision;
    }

    /// Shift the per-line cache to match whole lines that were inserted or removed.
    ///
    /// Pressing Enter (or deleting a line) moves every following line down one index.
    /// Without this, each of those lines would look "changed" and be shaped again. Here
    /// the tail is relocated once, and only the genuinely new lines are shaped.
    fn align_line_cache(&mut self, count: usize) {
        if self.line_cache.len() == count {
            return;
        }
        let cache = std::mem::take(&mut self.line_cache);
        self.line_cache = realign_cache(cache, count, |index, cached| {
            self.buffer.line_eq(index, &cached.text)
        });
    }

    /// Tell the editor where the vault lives, for resolving embeds.
    pub fn set_vault_root(&mut self, root: &Path) {
        self.vault_root = Some(root.to_path_buf());
    }

    /// Tell the editor how to open a link clicked in reading mode.
    pub fn set_link_opener(&mut self, opener: LinkOpener) {
        self.link_opener = Some(opener);
    }

    /// Tell the editor how to run Save As when a note has no file name yet.
    pub fn set_save_as(&mut self, save_as: SaveAsHandler) {
        self.save_as = Some(save_as);
    }

    /// Tell the editor how to close its tab.
    pub fn set_close_tab(&mut self, close_tab: CloseTabHandler) {
        self.close_tab = Some(close_tab);
    }

    /// Tell the editor where to report one-line results.
    pub fn set_status_reporter(&mut self, reporter: StatusReporter) {
        self.status_reporter = Some(reporter);
    }

    /// Tell the editor what to do when a save would overwrite an external change.
    pub fn set_conflict_handler(&mut self, handler: ConflictHandler) {
        self.conflict_handler = Some(handler);
    }

    /// Enable saving the note itself after idle, not just its draft.
    pub fn set_autosave(&mut self, autosave: bool) {
        self.autosave = autosave;
    }

    /// Whether a note may be autosaved after idle.
    pub fn autosave_enabled(&self) -> bool {
        self.autosave
    }

    /// Replace the buffer with a recovered draft, as one undoable edit.
    pub fn apply_recovery(&mut self, text: &str, line: usize, col: usize) {
        self.buffer.replace(0..self.buffer.len_chars(), text);
        self.set_cursor_position(line, col);
        self.reset_layout_cache();
        self.properties_revision = u64::MAX;
        self.doc_generation = self.doc_generation.wrapping_add(1);
        self.math_scenes.clear();
    }

    /// Arm the debounced draft writer when the buffer is dirty.
    ///
    /// Called from `render`, which already runs on every edit. Each change bumps the
    /// generation, so an outstanding timer either writes the newest text or arms one more
    /// wait, giving a trailing debounce. A clean buffer arms nothing.
    fn schedule_draft(&mut self, cx: &mut Context<Self>) {
        if !self.buffer.is_dirty() || self.path.is_none() || self.vault_root.is_none() {
            return;
        }
        let revision = self.buffer.revision();
        if self.draft_written == revision {
            return;
        }

        self.draft_generation = self.draft_generation.wrapping_add(1);
        if self.draft_armed {
            return;
        }
        self.draft_armed = true;
        self.spawn_draft_timer(cx);
    }

    fn spawn_draft_timer(&mut self, cx: &mut Context<Self>) {
        let generation = self.draft_generation;
        cx.spawn(async move |this, cx| {
            Timer::after(std::time::Duration::from_millis(1500)).await;
            let _ = this.update(cx, |editor, cx| {
                if editor.draft_generation != generation {
                    // Something changed while waiting: wait once more.
                    editor.spawn_draft_timer(cx);
                    return;
                }
                editor.draft_armed = false;
                editor.write_draft(cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// Write the pending draft, and autosave the note when that is enabled and safe.
    fn write_draft(&mut self, cx: &mut Context<Self>) {
        let (Some(root), Some(path)) = (self.vault_root.clone(), self.path.clone()) else {
            return;
        };
        let Ok(relative) = path.strip_prefix(&root) else {
            return;
        };
        let note = relative.to_string_lossy().replace('\\', "/");
        let (line, col) = self.buffer.line_col();
        let revision = self.buffer.revision();
        let draft = drafts::Draft {
            note: note.clone(),
            line,
            col,
            at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_millis() as u64)
                .unwrap_or(0),
            base: self.fingerprint.as_ref().map(|fp| fp.content_hash),
            text: self.buffer.text(),
        };
        if drafts::save(&root, &draft).is_ok() {
            self.draft_written = revision;
        }

        if !self.autosave {
            return;
        }
        match self.save_checked() {
            SaveReport::Saved { .. } => {
                drafts::discard(&root, &note);
                self.report(crate::StatusKind::Success, "autosaved", cx);
            }
            SaveReport::Conflict { .. } => {
                // The conflict bar takes over; stop writing until it is resolved.
                self.autosave = false;
                self.report(
                    crate::StatusKind::Warn,
                    "autosave paused: the note changed on disk",
                    cx,
                );
                if let Some(handler) = self.conflict_handler.clone() {
                    handler(cx);
                }
            }
            SaveReport::Error { message } => self.report(
                crate::StatusKind::Error,
                &format!("autosave failed: {message}"),
                cx,
            ),
        }
    }

    /// Report to the app's status bar, if one is wired up.
    fn report(&self, kind: crate::StatusKind, message: &str, cx: &mut App) {
        if let Some(reporter) = &self.status_reporter {
            reporter(kind, message, cx);
        }
    }

    /// Save and/or close after a Vim `:` command asked for it.
    fn run_vim_outcome(
        &mut self,
        outcome: &crate::vim::Outcome,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if outcome.save {
            let _ = self.save();
        }
        if outcome.close {
            if let Some(close) = self.close_tab.clone() {
                close(window, cx);
            }
        }
    }

    /// Recompute word counts when the buffer changes, and git stats when it is saved.
    fn refresh_stats(&mut self) {
        let revision = self.buffer.revision();
        if self.stats_cache.0 != revision {
            self.stats_cache = (revision, stats::compute(&self.buffer.text()));
        }

        // `git diff` is only meaningful for the saved file, so skip it while dirty.
        let dirty = self.buffer.is_dirty();
        if !dirty && self.git_stat_key != Some((revision, false)) {
            self.git_stat = self.compute_git_stat();
            self.git_stat_key = Some((revision, false));
        }
    }

    fn compute_git_stat(&self) -> Option<git::DiffStat> {
        let root = self.vault_root.as_ref()?;
        let path = self.path.as_ref()?;
        let relative = path
            .strip_prefix(root)
            .ok()?
            .to_string_lossy()
            .replace('\\', "/");
        git::file_diff(root, &relative).ok()
    }

    /// Words, characters and lines in the current selection, in either mode.
    fn selection_stats(&self) -> Option<crate::reading::SelectionStats> {
        if self.reading {
            return self.reading_content.selection_stats();
        }
        let selection = self.selection()?;
        let text = self.buffer.slice(selection);
        Some(crate::reading::SelectionStats {
            words: text.split_whitespace().count(),
            characters: text.chars().count(),
            lines: text.lines().count().max(1),
        })
    }

    /// Set the folder pasted images are filed into (vault-relative).
    pub fn set_attachments_folder(&mut self, folder: &str) {
        if !folder.trim().is_empty() {
            self.attachments_folder = folder.trim().to_string();
        }
    }

    /// Parsed frontmatter properties. Empty when absent or unsupported.
    pub fn properties(&mut self) -> Vec<Property> {
        self.refresh_properties();
        self.properties.clone()
    }

    /// Re-parse frontmatter only when the document actually changed.
    fn refresh_properties(&mut self) {
        let revision = self.buffer.revision();
        if revision != self.properties_revision {
            // Frontmatter lives at the top of the note, so only parse a leading window
            // instead of copying the whole document on every keystroke.
            let lines = self.buffer.line_count().min(FRONTMATTER_LINE_WINDOW);
            let end = self.buffer.char_index_at(lines, 0);
            let head = self.buffer.slice(0..end);
            self.properties = frontmatter::properties(&head).unwrap_or_default();
            self.properties_revision = revision;
        }
    }

    /// Typeset every display block that will be drawn and has no scene yet.
    ///
    /// This runs per frame rather than per edit, because moving the caret changes which
    /// block is drawn without changing the buffer revision. Successful and failed results
    /// are both cached, so each formula is typeset once.
    fn ensure_math_scenes(&mut self, caret_line: usize) {
        let blocks = self.display_math.clone();
        for block in blocks.iter() {
            if caret_line >= block.first_line && caret_line <= block.last_line {
                continue;
            }
            if self.math_scenes.contains_key(&block.tex) {
                continue;
            }
            let scene = math::typeset(
                &block.tex,
                &MathOptions {
                    style: MathStyle::Display,
                    color: [0.0, 0.0, 0.0, 1.0],
                },
            )
            .ok()
            .map(Rc::new);
            self.math_scenes.insert(block.tex.clone(), scene);
        }

        // A formula edited in place produces one cache entry per keystroke; drop the ones
        // no block references any more so the cache stays bounded.
        if self.math_scenes.len() > blocks.len() * 2 + 16 {
            let live: HashSet<&str> = blocks.iter().map(|block| block.tex.as_str()).collect();
            self.math_scenes
                .retain(|tex, _| live.contains(tex.as_str()));
        }
    }

    /// Replace the frontmatter with `properties`, keep the body, and save.
    pub fn set_properties(&mut self, properties: &[Property]) -> std::io::Result<()> {
        let updated = frontmatter::with_properties(&self.buffer.text(), properties);
        self.buffer = Buffer::from_text(&updated);
        self.properties_revision = u64::MAX;
        self.reset_layout_cache();
        self.doc_generation = self.doc_generation.wrapping_add(1);
        self.anchor = None;
        self.save()
    }

    /// Current document text.
    pub fn text(&self) -> String {
        self.buffer.text()
    }

    /// Document identity plus revision. The first value changes when a different note
    /// (or a reloaded note) is loaded, so caches can invalidate reliably.
    pub fn document_version(&self) -> (u64, u64) {
        (self.doc_generation, self.buffer.revision())
    }

    /// Move the caret to the start of `line` (0-based) and clear any selection.
    pub fn jump_to_line(&mut self, line: usize) {
        let line = line.min(self.buffer.line_count().saturating_sub(1));
        self.buffer.set_cursor(self.buffer.char_index_at(line, 0));
        self.anchor = None;
    }

    /// Jump to a heading by source line; in reading mode, scroll the reading view to it.
    pub fn jump_to_heading(&mut self, line: usize, text: &str, heading_index: usize) {
        self.jump_to_line(line);
        if self.reading && !self.reading_content.scroll_to_heading_index(heading_index) {
            self.reading_content.scroll_to_heading(text);
        }
    }

    /// The selected character range, if any.
    fn selection(&self) -> Option<Range<usize>> {
        let anchor = self.anchor?;
        let cursor = self.buffer.cursor();
        if anchor == cursor {
            None
        } else {
            Some(anchor.min(cursor)..anchor.max(cursor))
        }
    }

    fn begin_selection(&mut self) {
        if !self.extra_cursors.is_empty() {
            self.extra_cursors.clear();
        }
        if self.anchor.is_none() {
            self.anchor = Some(self.buffer.cursor());
        }
    }

    fn collapse_selection(&mut self) {
        self.anchor = None;
    }

    /// Every caret, primary first.
    fn all_cursor_states(&self) -> Vec<CursorState> {
        let mut states = Vec::with_capacity(self.extra_cursors.len() + 1);
        states.push(CursorState {
            cursor: self.buffer.cursor(),
            anchor: self.anchor,
        });
        states.extend(self.extra_cursors.iter().copied());
        states
    }

    /// Replace the caret set. The first entry becomes the primary caret.
    fn set_cursor_states(&mut self, states: Vec<CursorState>) {
        let mut unique = Vec::with_capacity(states.len());
        for state in states {
            if !unique.contains(&state) {
                unique.push(state);
            }
        }
        let Some(primary) = unique.first().copied() else {
            return;
        };
        self.buffer.set_cursor(primary.cursor);
        self.anchor = primary.anchor;
        self.extra_cursors = unique.into_iter().skip(1).collect();
    }

    /// Selected ranges for every caret, primary first. Empty selections are carets.
    fn selection_ranges(&self) -> Vec<Range<usize>> {
        self.all_cursor_states()
            .into_iter()
            .filter_map(|state| match state.anchor {
                Some(anchor) if anchor != state.cursor => {
                    Some(anchor.min(state.cursor)..anchor.max(state.cursor))
                }
                _ => None,
            })
            .collect()
    }

    /// Apply one edit per caret as a single undo step. `text` is inserted at each caret or
    /// replaces each selection. Returns the number of carets that produced an edit.
    fn apply_cursor_edits(&mut self, text: &str) -> usize {
        let states = self.all_cursor_states();
        let mut targets: Vec<EditTarget> = Vec::with_capacity(states.len());
        for (index, state) in states.into_iter().enumerate() {
            let range = match state.anchor {
                Some(anchor) if anchor != state.cursor => {
                    anchor.min(state.cursor)..anchor.max(state.cursor)
                }
                _ => state.cursor..state.cursor,
            };
            targets.push(EditTarget {
                range,
                text: text.to_string(),
                primary: index == 0,
            });
        }
        self.apply_edit_targets(targets)
    }

    /// Apply arbitrary ranges/replacements as one undo step and rebuild all carets.
    fn apply_edit_targets(&mut self, mut targets: Vec<EditTarget>) -> usize {
        if targets.is_empty() {
            return 0;
        }

        // Keep exactly one caret per location. The primary range wins over extras that
        // overlap it; overlapping extras are dropped.
        let primary_range = targets
            .iter()
            .find(|target| target.primary)
            .map(|target| target.range.clone());
        if let Some(primary) = &primary_range {
            let primary_empty = primary.start == primary.end;
            targets.retain(|target| {
                target.primary
                    || target.range.end <= primary.start
                    || if primary_empty {
                        target.range.start > primary.start
                    } else {
                        target.range.start >= primary.end
                    }
            });
        }

        targets.sort_by_key(|target| (target.range.start, target.range.end));
        let mut kept: Vec<EditTarget> = Vec::with_capacity(targets.len());
        let mut last_end = 0usize;
        let mut seen: Vec<(usize, usize)> = Vec::new();
        for target in targets {
            if target.range.start < last_end {
                continue;
            }
            if seen.contains(&(target.range.start, target.range.end)) {
                continue;
            }
            let empty = target.range.start == target.range.end && target.text.is_empty();
            if empty {
                continue;
            }
            seen.push((target.range.start, target.range.end));
            last_end = target.range.end.max(target.range.start);
            kept.push(target);
        }
        if kept.is_empty() {
            return 0;
        }

        // New caret after each edit, in document order. `delta` is the shift caused by
        // edits earlier in the document.
        let mut positions = Vec::with_capacity(kept.len());
        let mut primary_slot = 0usize;
        let mut delta: isize = 0;
        for (index, target) in kept.iter().enumerate() {
            let inserted = target.text.chars().count();
            let start = (target.range.start as isize + delta).max(0) as usize;
            positions.push(start + inserted);
            if target.primary {
                primary_slot = index;
            }
            delta += inserted as isize - (target.range.end - target.range.start) as isize;
        }

        let edits = kept
            .iter()
            .map(|target| (target.range.clone(), target.text.clone()))
            .collect();
        self.buffer.replace_many(edits);
        self.buffer.set_cursor(positions[primary_slot]);
        self.anchor = None;
        self.extra_cursors = positions
            .into_iter()
            .enumerate()
            .filter(|(index, _)| *index != primary_slot)
            .map(|(_, cursor)| CursorState {
                cursor,
                anchor: None,
            })
            .collect();
        kept.len()
    }

    /// Delete every selection, leaving bare carets alone.
    fn delete_selections(&mut self) -> usize {
        let states = self.all_cursor_states();
        let mut targets = Vec::new();
        for (index, state) in states.into_iter().enumerate() {
            if let Some(anchor) = state.anchor {
                if anchor != state.cursor {
                    targets.push(EditTarget {
                        range: anchor.min(state.cursor)..anchor.max(state.cursor),
                        text: String::new(),
                        primary: index == 0,
                    });
                }
            }
        }
        self.apply_edit_targets(targets)
    }

    /// Delete every selection, or one character on each side of each caret.
    fn delete_at_cursors(&mut self, backwards: bool) -> usize {
        let states = self.all_cursor_states();
        let mut targets = Vec::new();
        for (index, state) in states.into_iter().enumerate() {
            let range = match state.anchor {
                Some(anchor) if anchor != state.cursor => {
                    anchor.min(state.cursor)..anchor.max(state.cursor)
                }
                _ if backwards => {
                    if state.cursor == 0 {
                        continue;
                    }
                    state.cursor - 1..state.cursor
                }
                _ => {
                    if state.cursor >= self.buffer.len_chars() {
                        continue;
                    }
                    state.cursor..state.cursor + 1
                }
            };
            targets.push(EditTarget {
                range,
                text: String::new(),
                primary: index == 0,
            });
        }
        self.apply_edit_targets(targets)
    }

    fn select_word_at_cursor(&mut self) {
        let text = self.buffer.text();
        let chars: Vec<char> = text.chars().collect();
        let cursor = self.buffer.cursor().min(chars.len());
        if chars.is_empty() {
            return;
        }
        let word = |ch: char| ch.is_alphanumeric() || ch == '_';
        let mut start = cursor;
        let mut end = cursor;
        if start < chars.len() && word(chars[start]) {
            while start > 0 && word(chars[start - 1]) {
                start -= 1;
            }
            while end < chars.len() && word(chars[end]) {
                end += 1;
            }
        } else {
            while start > 0 && chars[start - 1].is_whitespace() {
                start -= 1;
            }
            while start > 0 && word(chars[start - 1]) {
                start -= 1;
            }
            end = start;
            while end < chars.len() && word(chars[end]) {
                end += 1;
            }
            if end == start && cursor < chars.len() {
                end = cursor + 1;
            }
        }
        if end > start {
            self.anchor = Some(start);
            self.buffer.set_cursor(end);
        }
    }

    fn select_next_occurrence(&mut self) {
        if self.selection().is_none() && self.extra_cursors.is_empty() {
            self.select_word_at_cursor();
            return;
        }

        let Some(primary) = self.selection() else {
            return;
        };
        let needle = self.buffer.slice(primary);
        if needle.is_empty() {
            return;
        }

        let furthest = self
            .selection_ranges()
            .into_iter()
            .map(|range| range.end)
            .max()
            .unwrap_or(0);
        let text = self.buffer.text();
        let Some(start) = literal_after(&text, &needle, furthest) else {
            self.find.message = Some("no more occurrences".to_string());
            return;
        };
        let selection = CursorState {
            cursor: start + needle.chars().count(),
            anchor: Some(start),
        };
        if !self.extra_cursors.contains(&selection) {
            self.extra_cursors.push(selection);
        }
    }

    fn add_cursor_vertical(&mut self, delta: isize) {
        let cursor = self.buffer.cursor();
        if let Some(target) = self.vertical_target(cursor, delta) {
            let state = CursorState {
                cursor: target,
                anchor: None,
            };
            if target != cursor && !self.extra_cursors.contains(&state) {
                self.extra_cursors.push(state);
            }
        }
    }

    /// The character index one visual row above or below `index`, following wrapped rows.
    fn vertical_target(&self, index: usize, delta: isize) -> Option<usize> {
        let (line, col) = self.buffer.line_col_at(index);
        let current = self
            .last_rows
            .iter()
            .rposition(|meta| meta.line == line && meta.start <= col)?;
        let target = current as isize + delta;
        if target < 0 || target as usize >= self.last_rows.len() {
            return None;
        }
        let current_meta = self.last_rows[current];
        let target_meta = self.last_rows[target as usize];
        let offset = (col - current_meta.start).min(target_meta.end - target_meta.start);
        Some(
            self.buffer
                .char_index_at(target_meta.line, target_meta.start + offset),
        )
    }

    /// Move every caret with a buffer movement, collapsing their selections.
    fn move_all_cursors(&mut self, mut movement: impl FnMut(&mut Buffer)) {
        if self.extra_cursors.is_empty() {
            self.collapse_selection();
            movement(&mut self.buffer);
            return;
        }

        let mut states = self.all_cursor_states();
        for state in &mut states {
            self.buffer.set_cursor(state.cursor);
            movement(&mut self.buffer);
            state.cursor = self.buffer.cursor();
            state.anchor = None;
        }
        self.set_cursor_states(states);
    }

    /// Move every caret one visual row. Used when several carets are active.
    fn move_all_vertical(&mut self, delta: isize) {
        if self.extra_cursors.is_empty() {
            self.collapse_selection();
            self.move_vertical(delta);
            return;
        }

        let mut states = self.all_cursor_states();
        for state in &mut states {
            if let Some(target) = self.vertical_target(state.cursor, delta) {
                state.cursor = target;
            }
            state.anchor = None;
        }
        self.set_cursor_states(states);
    }

    /// Scroll the editor just enough to keep the caret (with a line of context) visible.
    ///
    /// Called from `render`, using the geometry recorded by the last paint, so the caret
    /// never runs off the bottom of the viewport while typing in a long note.
    fn keep_cursor_visible(&mut self) {
        if self.reading {
            return;
        }
        let Some(bounds) = self.last_bounds else {
            return;
        };
        let (line, col) = self.buffer.line_col();
        let Some(row) = self
            .last_rows
            .iter()
            .rposition(|meta| meta.line == line && meta.start <= col)
        else {
            return;
        };

        // Only follow the caret when it actually moved. Repainting for any other reason
        // (a wheel scroll, a status message) must not drag the viewport back to the caret.
        let anchor = (self.buffer.cursor(), row);
        if self.caret_follow == Some(anchor) {
            return;
        }

        let viewport = self.scroll_handle.bounds();
        if f32::from(viewport.size.height) <= 0.0 {
            return;
        }

        // `bounds` is already translated by the current scroll offset.
        let line_height = self.line_height;
        let top = bounds.top() + line_height * row as f32;
        let bottom = top + line_height;

        let delta = scroll_delta(top, bottom, viewport.top(), viewport.bottom());
        if f32::from(delta).abs() < 0.5 {
            self.caret_follow = Some(anchor);
            return;
        }

        let offset = self.scroll_handle.offset();
        self.scroll_handle
            .set_offset(point(offset.x, offset.y + delta));
        self.caret_follow = Some(anchor);
    }

    /// Move the caret one visual row up (`delta < 0`) or down, following wrapped rows.
    fn move_vertical(&mut self, delta: isize) {
        let (line, col) = self.buffer.line_col();

        let Some(current) = self
            .last_rows
            .iter()
            .rposition(|meta| meta.line == line && meta.start <= col)
        else {
            // No geometry yet: fall back to logical movement.
            if delta < 0 {
                self.buffer.move_up();
            } else {
                self.buffer.move_down();
            }
            return;
        };

        let target = current as isize + delta;
        if target < 0 || target as usize >= self.last_rows.len() {
            return;
        }

        let current_meta = self.last_rows[current];
        let target_meta = self.last_rows[target as usize];
        let offset = (col - current_meta.start).min(target_meta.end - target_meta.start);

        self.buffer.set_cursor(
            self.buffer
                .char_index_at(target_meta.line, target_meta.start + offset),
        );
    }

    fn delete_selection(&mut self) -> bool {
        let has_extra_selection = self
            .extra_cursors
            .iter()
            .any(|state| state.anchor.is_some_and(|anchor| anchor != state.cursor));
        if self.selection().is_none() && !has_extra_selection {
            return false;
        }
        self.delete_selections() > 0
    }

    fn insert_text(&mut self, text: &str) {
        if self.extra_cursors.is_empty() {
            if let Some(selection) = self.selection() {
                self.buffer.replace(selection, text);
            } else {
                self.buffer.insert(text);
            }
            self.anchor = None;
            self.refresh_autocomplete();
            return;
        }

        self.apply_cursor_edits(text);
        self.refresh_autocomplete();
    }

    fn backspace(&mut self, _: &Backspace, _window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_mutate() {
            return;
        }
        if self.find.open {
            self.find_backspace();
            cx.notify();
            return;
        }
        if self.vim.enabled() && self.vim.mode() != VimMode::Insert {
            let mut anchor = self.anchor;
            let outcome = self
                .vim
                .handle_key("backspace", &mut self.buffer, &mut anchor);
            self.anchor = anchor;
            self.vim.set_message(outcome.message);
            cx.notify();
            return;
        }
        if self.extra_cursors.is_empty() {
            if !self.delete_selection() {
                self.buffer.backspace();
            }
        } else if self.delete_selections() == 0 {
            self.delete_at_cursors(true);
        }
        self.refresh_autocomplete();
        cx.notify();
    }

    fn delete(&mut self, _: &Delete, _window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_mutate() {
            return;
        }
        if self.find.open {
            return;
        }
        if self.vim.enabled() && self.vim.mode() != VimMode::Insert {
            let mut anchor = self.anchor;
            let outcome = self.vim.handle_key("delete", &mut self.buffer, &mut anchor);
            self.anchor = anchor;
            self.vim.set_message(outcome.message);
            cx.notify();
            return;
        }
        if self.extra_cursors.is_empty() {
            if !self.delete_selection() {
                self.buffer.delete_forward();
            }
        } else if self.delete_selections() == 0 {
            self.delete_at_cursors(false);
        }
        self.refresh_autocomplete();
        cx.notify();
    }

    fn left(&mut self, _: &Left, _window: &mut Window, cx: &mut Context<Self>) {
        if self.reading {
            return;
        }
        self.dismiss_autocomplete();
        self.move_all_cursors(|buffer| buffer.move_left());
        cx.notify();
    }

    fn right(&mut self, _: &Right, _window: &mut Window, cx: &mut Context<Self>) {
        if self.reading {
            return;
        }
        self.dismiss_autocomplete();
        self.move_all_cursors(|buffer| buffer.move_right());
        cx.notify();
    }

    fn up(&mut self, _: &Up, _window: &mut Window, cx: &mut Context<Self>) {
        if self.reading {
            self.reading_content.scroll_blocks(-1);
            cx.notify();
            return;
        }
        if self.autocomplete.open {
            self.move_autocomplete(-1);
            cx.notify();
            return;
        }
        self.move_all_vertical(-1);
        cx.notify();
    }

    fn down(&mut self, _: &Down, _window: &mut Window, cx: &mut Context<Self>) {
        if self.reading {
            self.reading_content.scroll_blocks(1);
            cx.notify();
            return;
        }
        if self.autocomplete.open {
            self.move_autocomplete(1);
            cx.notify();
            return;
        }
        self.move_all_vertical(1);
        cx.notify();
    }

    fn home(&mut self, _: &Home, _window: &mut Window, cx: &mut Context<Self>) {
        if self.reading {
            self.reading_content.scroll_blocks(isize::MIN / 2);
            cx.notify();
            return;
        }
        self.move_all_cursors(|buffer| buffer.move_home());
        cx.notify();
    }

    fn end(&mut self, _: &End, _window: &mut Window, cx: &mut Context<Self>) {
        if self.reading {
            self.reading_content.scroll_blocks(isize::MAX / 2);
            cx.notify();
            return;
        }
        self.move_all_cursors(|buffer| buffer.move_end());
        cx.notify();
    }

    fn word_left(&mut self, _: &WordLeft, _window: &mut Window, cx: &mut Context<Self>) {
        if self.reading {
            return;
        }
        self.move_all_cursors(|buffer| buffer.move_word_left());
        cx.notify();
    }

    fn word_right(&mut self, _: &WordRight, _window: &mut Window, cx: &mut Context<Self>) {
        if self.reading {
            return;
        }
        self.move_all_cursors(|buffer| buffer.move_word_right());
        cx.notify();
    }

    fn paragraph_up(&mut self, _: &ParagraphUp, _window: &mut Window, cx: &mut Context<Self>) {
        if self.reading {
            self.reading_content.scroll_blocks(-1);
            cx.notify();
            return;
        }
        self.move_all_cursors(|buffer| buffer.move_paragraph_up());
        cx.notify();
    }

    fn paragraph_down(&mut self, _: &ParagraphDown, _window: &mut Window, cx: &mut Context<Self>) {
        if self.reading {
            self.reading_content.scroll_blocks(1);
            cx.notify();
            return;
        }
        self.move_all_cursors(|buffer| buffer.move_paragraph_down());
        cx.notify();
    }

    fn document_start(&mut self, _: &DocumentStart, _window: &mut Window, cx: &mut Context<Self>) {
        if self.reading {
            self.reading_content.scroll_blocks(isize::MIN / 2);
            cx.notify();
            return;
        }
        self.move_all_cursors(|buffer| buffer.move_document_start());
        cx.notify();
    }

    fn document_end(&mut self, _: &DocumentEnd, _window: &mut Window, cx: &mut Context<Self>) {
        if self.reading {
            self.reading_content.scroll_blocks(isize::MAX / 2);
            cx.notify();
            return;
        }
        self.move_all_cursors(|buffer| buffer.move_document_end());
        cx.notify();
    }

    fn delete_word_left(
        &mut self,
        _: &DeleteWordLeft,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.can_mutate() {
            return;
        }
        if self.extra_cursors.is_empty() {
            if !self.delete_selection() {
                self.buffer.delete_word_left();
            }
        } else {
            self.delete_word_at_cursors(true);
        }
        cx.notify();
    }

    fn delete_word_right(
        &mut self,
        _: &DeleteWordRight,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.can_mutate() {
            return;
        }
        if self.extra_cursors.is_empty() {
            if !self.delete_selection() {
                self.buffer.delete_word_right();
            }
        } else {
            self.delete_word_at_cursors(false);
        }
        cx.notify();
    }

    fn delete_word_at_cursors(&mut self, backwards: bool) {
        let states = self.all_cursor_states();
        let mut targets = Vec::new();
        for (index, state) in states.into_iter().enumerate() {
            let range = if let Some(anchor) = state.anchor {
                if anchor != state.cursor {
                    anchor.min(state.cursor)..anchor.max(state.cursor)
                } else if backwards {
                    self.buffer.word_left_at(state.cursor)..state.cursor
                } else {
                    state.cursor..self.buffer.word_right_at(state.cursor)
                }
            } else if backwards {
                self.buffer.word_left_at(state.cursor)..state.cursor
            } else {
                state.cursor..self.buffer.word_right_at(state.cursor)
            };
            if range.start == range.end {
                continue;
            }
            targets.push(EditTarget {
                range,
                text: String::new(),
                primary: index == 0,
            });
        }
        self.apply_edit_targets(targets);
    }

    fn select_left(&mut self, _: &SelectLeft, _window: &mut Window, cx: &mut Context<Self>) {
        if self.reading {
            self.reading_content
                .extend_selection(crate::reading::SelectionMotion::Left);
            cx.notify();
            return;
        }
        self.begin_selection();
        self.buffer.move_left();
        cx.notify();
    }

    fn select_right(&mut self, _: &SelectRight, _window: &mut Window, cx: &mut Context<Self>) {
        if self.reading {
            self.reading_content
                .extend_selection(crate::reading::SelectionMotion::Right);
            cx.notify();
            return;
        }
        self.begin_selection();
        self.buffer.move_right();
        cx.notify();
    }

    fn select_up(&mut self, _: &SelectUp, _window: &mut Window, cx: &mut Context<Self>) {
        if self.reading {
            self.reading_content
                .extend_selection(crate::reading::SelectionMotion::Up);
            cx.notify();
            return;
        }
        self.begin_selection();
        self.move_vertical(-1);
        cx.notify();
    }

    fn select_down(&mut self, _: &SelectDown, _window: &mut Window, cx: &mut Context<Self>) {
        if self.reading {
            self.reading_content
                .extend_selection(crate::reading::SelectionMotion::Down);
            cx.notify();
            return;
        }
        self.begin_selection();
        self.move_vertical(1);
        cx.notify();
    }

    fn select_home(&mut self, _: &SelectHome, _window: &mut Window, cx: &mut Context<Self>) {
        if self.reading {
            self.reading_content
                .extend_selection(crate::reading::SelectionMotion::LineStart);
            cx.notify();
            return;
        }
        self.begin_selection();
        self.buffer.move_home();
        cx.notify();
    }

    fn select_end(&mut self, _: &SelectEnd, _window: &mut Window, cx: &mut Context<Self>) {
        if self.reading {
            self.reading_content
                .extend_selection(crate::reading::SelectionMotion::LineEnd);
            cx.notify();
            return;
        }
        self.begin_selection();
        self.buffer.move_end();
        cx.notify();
    }

    fn select_word_left(
        &mut self,
        _: &SelectWordLeft,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.begin_selection();
        self.buffer.move_word_left();
        cx.notify();
    }

    fn select_word_right(
        &mut self,
        _: &SelectWordRight,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.begin_selection();
        self.buffer.move_word_right();
        cx.notify();
    }

    fn select_document_start(
        &mut self,
        _: &SelectDocumentStart,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.reading {
            self.reading_content
                .extend_selection(crate::reading::SelectionMotion::DocumentStart);
            cx.notify();
            return;
        }
        self.begin_selection();
        self.buffer.move_document_start();
        cx.notify();
    }

    fn select_document_end(
        &mut self,
        _: &SelectDocumentEnd,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.reading {
            self.reading_content
                .extend_selection(crate::reading::SelectionMotion::DocumentEnd);
            cx.notify();
            return;
        }
        self.begin_selection();
        self.buffer.move_document_end();
        cx.notify();
    }

    fn on_select_all(&mut self, _: &SelectAll, _window: &mut Window, cx: &mut Context<Self>) {
        if self.reading {
            self.reading_content.select_all();
            cx.notify();
            return;
        }
        self.anchor = Some(0);
        self.extra_cursors.clear();
        self.buffer.set_cursor(self.buffer.len_chars());
        cx.notify();
    }

    fn copy(&mut self, _: &Copy, _window: &mut Window, cx: &mut Context<Self>) {
        self.copy_selection(cx);
    }

    /// Copy the current selection (reading or editing) and report what happened.
    pub fn copy_selection(&mut self, cx: &mut Context<Self>) {
        if self.reading {
            match self.reading_content.selected_text() {
                Some(text) => {
                    let count = text.chars().count();
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                    self.report(
                        crate::StatusKind::Info,
                        &format!("copied {count} character(s)"),
                        cx,
                    );
                }
                None => self.report(crate::StatusKind::Warn, "nothing selected to copy", cx),
            }
            return;
        }
        let ranges = self.selection_ranges();
        if ranges.is_empty() {
            return;
        }
        let text = ranges
            .into_iter()
            .map(|range| self.buffer.slice(range))
            .collect::<Vec<_>>()
            .join("\n");
        let count = text.chars().count();
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        self.report(
            crate::StatusKind::Info,
            &format!("copied {count} character(s)"),
            cx,
        );
    }

    fn cut(&mut self, _: &Cut, _window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_mutate() {
            return;
        }
        let ranges = self.selection_ranges();
        if ranges.is_empty() {
            return;
        }
        let text = ranges
            .iter()
            .map(|range| self.buffer.slice(range.clone()))
            .collect::<Vec<_>>()
            .join("\n");
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        self.delete_selections();
        cx.notify();
    }

    fn undo(&mut self, _: &Undo, _window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_mutate() {
            return;
        }
        if self.buffer.undo() {
            self.anchor = None;
            self.extra_cursors.clear();
            self.marked_range = None;
            self.vim.reset();
            cx.notify();
        }
    }

    fn redo(&mut self, _: &Redo, _window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_mutate() {
            return;
        }
        if self.buffer.redo() {
            self.anchor = None;
            self.extra_cursors.clear();
            self.marked_range = None;
            self.vim.reset();
            cx.notify();
        }
    }

    fn enter(&mut self, _: &Enter, window: &mut Window, cx: &mut Context<Self>) {
        if self.accept_autocomplete() {
            cx.notify();
            return;
        }
        if self.find.open {
            let backwards = window.modifiers().shift;
            if self.find.field == FindField::Replacement && !backwards {
                self.find_replace_one();
            } else {
                self.find_next(backwards);
            }
            cx.notify();
            return;
        }
        if !self.can_mutate() {
            return;
        }
        if self.vim.enabled() && self.vim.mode() != VimMode::Insert {
            let mut anchor = self.anchor;
            let outcome = self.vim.handle_key("enter", &mut self.buffer, &mut anchor);
            self.anchor = anchor;
            self.vim.set_message(outcome.message.clone());
            self.run_vim_outcome(&outcome, window, cx);
            cx.notify();
            return;
        }
        // Enter at the end of a list item continues its marker (and increments numbers).
        if self.selection().is_none() && self.extra_cursors.is_empty() {
            let (line, col) = self.buffer.line_col();
            if let Some(prefix) = list_continuation(&self.buffer.line(line), col) {
                self.insert_text(&format!("\n{prefix}"));
                cx.notify();
                return;
            }
        }
        self.delete_selections();
        self.insert_text("\n");
        cx.notify();
    }

    fn paste(&mut self, _: &Paste, _window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_mutate() {
            return;
        }
        if self.paste_clipboard_image(cx) {
            cx.notify();
            return;
        }

        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            if self.paste_image_path(&text) {
                cx.notify();
                return;
            }
            self.insert_text(&text);
            cx.notify();
        }
    }

    /// If the clipboard holds an image, file it in the attachments folder and link it.
    fn paste_clipboard_image(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(root) = self.vault_root.clone() else {
            return false;
        };
        let Some(item) = cx.read_from_clipboard() else {
            return false;
        };

        for entry in item.into_entries() {
            let ClipboardEntry::Image(image) = entry else {
                continue;
            };
            if image.bytes.is_empty() {
                return false;
            }

            let extension = image_extension(image.format);
            let relative = format!("{}/{}.{extension}", self.attachments_folder, stamp());
            let target = root.join(&relative);

            if let Some(parent) = target.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if std::fs::write(&target, &image.bytes).is_err() {
                return false;
            }

            self.reading_content.invalidate_attachments();
            self.insert_text(&format!("![[{relative}]]"));
            return true;
        }

        false
    }

    /// If the pasted text is a path to an existing image, copy it in and link it.
    fn paste_image_path(&mut self, text: &str) -> bool {
        let trimmed = text.trim().trim_matches('"');
        if trimmed.contains('\n') || trimmed.is_empty() {
            return false;
        }

        let source = Path::new(trimmed);
        if !is_image_file(source) || !source.is_file() {
            return false;
        }

        let Some(relative) = self.file_into_attachments(source) else {
            return false;
        };
        self.reading_content.invalidate_attachments();
        self.insert_text(&format!("![[{relative}]]"));
        true
    }

    /// Copy a file into the attachments folder and return its vault-relative path.
    fn file_into_attachments(&self, source: &Path) -> Option<String> {
        let root = self.vault_root.clone()?;
        let stem = source.file_stem()?.to_str()?;
        let extension = source.extension()?.to_str()?.to_ascii_lowercase();
        let folder = &self.attachments_folder;
        let stamp = stamp();

        let mut relative = format!("{folder}/{}-{stamp}.{extension}", sanitize(stem));
        let mut attempt = 1;
        while root.join(&relative).exists() {
            relative = format!("{folder}/{}-{stamp}-{attempt}.{extension}", sanitize(stem));
            attempt += 1;
        }

        let target = root.join(&relative);
        if let Some(parent) = target.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        std::fs::copy(source, &target).ok()?;
        Some(relative)
    }

    /// File dropped paths: images and other files are copied into the attachments folder and
    /// embedded, while markdown files are linked. One insertion per path, on its own line.
    pub fn drop_files(&mut self, paths: &[PathBuf], cx: &mut Context<Self>) {
        if !self.can_mutate() {
            return;
        }
        let mut inserted = 0usize;

        for path in paths {
            if !path.is_file() {
                continue;
            }

            let extension = path
                .extension()
                .and_then(|extension| extension.to_str())
                .map(|extension| extension.to_ascii_lowercase())
                .unwrap_or_default();

            let text = if extension == "md" || extension == "markdown" {
                let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
                    continue;
                };
                format!("[[{stem}]]")
            } else {
                let Some(relative) = self.file_into_attachments(path) else {
                    continue;
                };
                format!("![[{relative}]]")
            };

            if inserted > 0 {
                self.insert_text("\n");
            }
            self.insert_text(&text);
            inserted += 1;
        }

        if inserted > 0 {
            self.reading_content.invalidate_attachments();
            cx.notify();
        }
    }

    fn move_block_up_action(
        &mut self,
        _: &MoveBlockUp,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_block_up();
        cx.notify();
    }

    fn move_block_down_action(
        &mut self,
        _: &MoveBlockDown,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_block_down();
        cx.notify();
    }

    fn on_save(&mut self, _: &Save, window: &mut Window, cx: &mut Context<Self>) {
        // An untitled buffer has nowhere to write yet: ask for a name instead of failing
        // with a message the user never sees.
        if !self.has_path() {
            if let Some(save_as) = self.save_as.clone() {
                save_as(window, cx);
            }
            return;
        }

        match self.save_checked() {
            SaveReport::Saved { .. } => self.report(crate::StatusKind::Success, "saved", cx),
            SaveReport::Conflict { disk } => {
                self.report(
                    crate::StatusKind::Error,
                    if disk.is_some() {
                        "the note changed on disk — reload it or keep mine"
                    } else {
                        "the note was deleted on disk — saving puts it back"
                    },
                    cx,
                );
                if let Some(handler) = self.conflict_handler.clone() {
                    handler(cx);
                }
            }
            SaveReport::Error { message } => self.report(
                crate::StatusKind::Error,
                &format!("could not save: {message}"),
                cx,
            ),
        }
        cx.notify();
    }

    fn toggle_bold(&mut self, _: &ToggleBold, _window: &mut Window, cx: &mut Context<Self>) {
        self.apply_style(text_format::Style::Bold);
        cx.notify();
    }

    fn toggle_italic(&mut self, _: &ToggleItalic, _window: &mut Window, cx: &mut Context<Self>) {
        self.apply_style(text_format::Style::Italic);
        cx.notify();
    }

    fn toggle_strikethrough(
        &mut self,
        _: &ToggleStrikethrough,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.apply_style(text_format::Style::Strikethrough);
        cx.notify();
    }

    fn toggle_highlight(
        &mut self,
        _: &ToggleHighlight,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.apply_style(text_format::Style::Highlight);
        cx.notify();
    }

    fn toggle_inline_code(
        &mut self,
        _: &ToggleInlineCode,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.apply_style(text_format::Style::InlineCode);
        cx.notify();
    }

    fn toggle_comment(&mut self, _: &ToggleComment, _window: &mut Window, cx: &mut Context<Self>) {
        self.apply_style(text_format::Style::Comment);
        cx.notify();
    }

    fn insert_wikilink(
        &mut self,
        _: &InsertWikilink,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.start_wikilink();
        cx.notify();
    }

    /// Tab: accept an autocomplete suggestion, otherwise indent the selection or caret.
    fn indent(&mut self, _: &Indent, _window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_mutate() || self.find.open {
            return;
        }
        if self.accept_autocomplete() {
            cx.notify();
            return;
        }
        self.indent_selection(false);
        cx.notify();
    }

    /// Shift+Tab: remove one indent step from the selected lines or the caret's line.
    fn outdent(&mut self, _: &Outdent, _window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_mutate() || self.find.open {
            return;
        }
        self.indent_selection(true);
        cx.notify();
    }

    /// Add or remove one indent step (four spaces) for every line the selection touches.
    ///
    /// With a bare caret, Tab inserts at the caret and Shift+Tab removes the line's leading
    /// whitespace, which is what a plain text editor does.
    fn indent_selection(&mut self, outdent: bool) {
        const STEP: usize = 4;

        let Some(range) = self.selection() else {
            if outdent {
                let (line, _) = self.buffer.line_col();
                let line_start = self.buffer.char_index_at(line, 0);
                let leading: Vec<char> = self.buffer.line(line).chars().take(STEP).collect();
                let remove = if leading.first() == Some(&'\t') {
                    1
                } else {
                    leading.iter().take_while(|ch| **ch == ' ').count()
                };
                if remove > 0 {
                    self.buffer.replace(line_start..line_start + remove, "");
                    self.buffer
                        .set_cursor(self.buffer.cursor().saturating_sub(remove).max(line_start));
                    self.anchor = None;
                }
            } else {
                self.insert_text(&" ".repeat(STEP));
            }
            return;
        };

        let (start_line, _) = self.buffer.line_col_at(range.start);
        let (end_line, end_col) = self.buffer.line_col_at(range.end);
        // A selection that ends at column 0 of a line does not include that line.
        let last_line = if end_col == 0 && end_line > start_line {
            end_line - 1
        } else {
            end_line
        };

        let mut edits: Vec<(Range<usize>, String)> = Vec::new();
        for line in start_line..=last_line {
            let line_start = self.buffer.char_index_at(line, 0);
            if outdent {
                let leading: Vec<char> = self.buffer.line(line).chars().take(STEP).collect();
                let remove = if leading.first() == Some(&'\t') {
                    1
                } else {
                    leading.iter().take_while(|ch| **ch == ' ').count()
                };
                if remove > 0 {
                    edits.push((line_start..line_start + remove, String::new()));
                }
            } else {
                edits.push((line_start..line_start, " ".repeat(STEP)));
            }
        }
        if edits.is_empty() {
            return;
        }

        // Map the selection through the edits so the same text stays selected.
        let map = |position: usize| -> usize {
            let mut mapped = position as isize;
            for (range, text) in &edits {
                if range.start <= position {
                    let inserted = text.chars().count() as isize;
                    let removed = (range.end - range.start) as isize;
                    mapped += inserted - removed;
                }
            }
            mapped.max(0) as usize
        };
        let anchor = map(range.start);
        let head = map(range.end);

        self.buffer.replace_many(edits);
        self.anchor = Some(anchor);
        self.buffer.set_cursor(head);
        self.extra_cursors.clear();
        self.refresh_autocomplete();
    }

    fn toggle_reading(&mut self, _: &ToggleReading, _window: &mut Window, cx: &mut Context<Self>) {
        self.reading = !self.reading;
        cx.notify();
    }

    /// Character index under a window-space point, using the geometry from the last paint.
    /// Enable or disable Vim-style modal editing for this editor.
    pub fn set_vim_enabled(&mut self, enabled: bool) {
        self.vim.set_enabled(enabled);
    }

    pub fn vim_enabled(&self) -> bool {
        self.vim.enabled()
    }

    pub fn vim_status(&self) -> String {
        self.vim.status()
    }

    /// Open the in-note find bar, optionally with the replacement field.
    pub fn open_find(&mut self, replace: bool) {
        if self.reading {
            self.set_reading_mode(false);
        }
        if !self.find.open {
            let selection = self.selection().map(|range| self.buffer.slice(range));
            if let Some(text) = selection {
                if !text.contains('\n') && text.chars().count() <= 200 {
                    self.find.pattern = text;
                }
            }
        }
        self.find.open = true;
        self.find.replace |= replace;
        self.find.field = FindField::Pattern;
        self.find.message = None;
        self.refresh_find(true);
    }

    /// Escape closes the find bar, leaves Vim insert/visual mode, or drops extra carets.
    /// Returns whether the editor consumed the key.
    pub fn handle_escape(&mut self, cx: &mut Context<Self>) -> bool {
        if self.reading && self.reading_content.dismiss_popup(cx) {
            return true;
        }
        if self.reading && self.reading_content.clear_selection() {
            return true;
        }
        if self.autocomplete.open {
            self.dismiss_autocomplete();
            return true;
        }
        if self.find.open {
            self.find.open = false;
            self.find.error = None;
            self.find.message = None;
            return true;
        }
        if self.vim.enabled() {
            let mut anchor = self.anchor;
            let outcome = self.vim.handle_key("escape", &mut self.buffer, &mut anchor);
            self.anchor = anchor;
            self.vim.set_message(outcome.message);
            return true;
        }
        if !self.extra_cursors.is_empty() {
            self.extra_cursors.clear();
            return true;
        }
        false
    }

    fn refresh_find(&mut self, select: bool) {
        let text = self.buffer.text();
        match find_engine::find_all(&text, &self.find.pattern, self.find.options()) {
            Ok(matches) => {
                self.find.error = None;
                self.find.matches = matches.into_iter().map(|found| found.range).collect();
            }
            Err(error) => {
                self.find.error = Some(error);
                self.find.matches.clear();
            }
        }

        if self.find.matches.is_empty() {
            self.find.current = 0;
            if self.find.pattern.is_empty() {
                self.find.message = None;
            }
            return;
        }

        if select {
            let from = self
                .selection()
                .map(|range| range.start)
                .unwrap_or(self.buffer.cursor());
            let index = self
                .find
                .matches
                .iter()
                .position(|range| range.start >= from)
                .unwrap_or(0);
            self.select_find_match(index);
        } else if self.find.current >= self.find.matches.len() {
            self.find.current = self.find.matches.len() - 1;
        }
    }

    fn select_find_match(&mut self, index: usize) {
        if self.find.matches.is_empty() {
            return;
        }
        let index = index % self.find.matches.len();
        self.find.current = index;
        let range = self.find.matches[index].clone();
        self.anchor = Some(range.start);
        self.buffer.set_cursor(range.end);
    }

    fn find_append(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        let select = self.find.field == FindField::Pattern;
        match self.find.field {
            FindField::Pattern => self.find.pattern.push_str(text),
            FindField::Replacement => self.find.replacement.push_str(text),
        }
        self.find.message = None;
        self.refresh_find(select);
    }

    fn find_backspace(&mut self) {
        let select = self.find.field == FindField::Pattern;
        match self.find.field {
            FindField::Pattern => {
                self.find.pattern.pop();
            }
            FindField::Replacement => {
                self.find.replacement.pop();
            }
        }
        self.find.message = None;
        self.refresh_find(select);
    }

    fn find_next(&mut self, backwards: bool) {
        if self.find.matches.is_empty() {
            self.refresh_find(true);
            return;
        }
        let len = self.find.matches.len();
        let index = if backwards {
            (self.find.current + len - 1) % len
        } else {
            (self.find.current + 1) % len
        };
        self.select_find_match(index);
    }

    fn find_replace_one(&mut self) {
        if !self.can_mutate() {
            return;
        }
        if self.find.matches.is_empty() {
            self.refresh_find(true);
        }
        let Some(range) = self.find.matches.get(self.find.current).cloned() else {
            self.find.message = Some("no matches".to_string());
            return;
        };
        let text = self.buffer.text();
        let target = find_engine::Match {
            range: range.clone(),
        };
        match find_engine::replacement_for(
            &text,
            &target,
            &self.find.pattern,
            &self.find.replacement,
            self.find.options(),
        ) {
            Ok(Some(replacement)) => {
                self.buffer.replace(range, &replacement);
                self.find.message = Some("replaced 1".to_string());
                self.refresh_find(false);
                if !self.find.matches.is_empty() {
                    let index = self.find.current.min(self.find.matches.len() - 1);
                    self.select_find_match(index);
                }
            }
            Ok(None) => {
                self.find.message = Some("match changed; press Find next".to_string());
                self.refresh_find(true);
            }
            Err(error) => self.find.error = Some(error),
        }
    }

    fn find_replace_all(&mut self) {
        if !self.can_mutate() {
            return;
        }
        let text = self.buffer.text();
        match find_engine::replace_all(
            &text,
            &self.find.pattern,
            &self.find.replacement,
            self.find.options(),
        ) {
            Ok((new_text, count)) => {
                if count > 0 {
                    self.buffer.replace(0..self.buffer.len_chars(), &new_text);
                    self.anchor = None;
                    self.buffer.set_cursor(0);
                }
                self.find.message = Some(format!("replaced {count}"));
                self.find.error = None;
                self.refresh_find(false);
            }
            Err(error) => {
                self.find.error = Some(error);
                self.find.matches.clear();
            }
        }
    }

    fn open_find_action(&mut self, _: &OpenFind, _window: &mut Window, cx: &mut Context<Self>) {
        self.open_find(false);
        cx.notify();
    }

    fn open_find_replace_action(
        &mut self,
        _: &OpenFindReplace,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_find(true);
        cx.notify();
    }

    fn find_next_action(&mut self, _: &FindNext, _window: &mut Window, cx: &mut Context<Self>) {
        self.find_next(false);
        cx.notify();
    }

    fn find_prev_action(&mut self, _: &FindPrev, _window: &mut Window, cx: &mut Context<Self>) {
        self.find_next(true);
        cx.notify();
    }

    fn find_replace_one_action(
        &mut self,
        _: &FindReplaceOne,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.find.open {
            return;
        }
        self.find_replace_one();
        cx.notify();
    }

    fn find_replace_all_action(
        &mut self,
        _: &FindReplaceAll,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.find.open {
            return;
        }
        self.find_replace_all();
        cx.notify();
    }

    fn find_toggle_regex_action(
        &mut self,
        _: &FindToggleRegex,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.find.open {
            return;
        }
        self.find.regex = !self.find.regex;
        self.refresh_find(true);
        cx.notify();
    }

    fn find_toggle_case_action(
        &mut self,
        _: &FindToggleCase,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.find.open {
            return;
        }
        self.find.case_sensitive = !self.find.case_sensitive;
        self.refresh_find(true);
        cx.notify();
    }

    fn find_toggle_word_action(
        &mut self,
        _: &FindToggleWord,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.find.open {
            return;
        }
        self.find.whole_word = !self.find.whole_word;
        self.refresh_find(true);
        cx.notify();
    }

    fn select_next_occurrence_action(
        &mut self,
        _: &SelectNextOccurrence,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_next_occurrence();
        cx.notify();
    }

    fn add_cursor_above_action(
        &mut self,
        _: &AddCursorAbove,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.add_cursor_vertical(-1);
        cx.notify();
    }

    fn add_cursor_below_action(
        &mut self,
        _: &AddCursorBelow,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.add_cursor_vertical(1);
        cx.notify();
    }

    fn render_find_bar(&self, theme: Theme, cx: &mut Context<Self>) -> AnyElement {
        let pattern = if self.find.pattern.is_empty() {
            "Find".to_string()
        } else {
            self.find.pattern.clone()
        };
        let replacement = if self.find.replacement.is_empty() {
            "Replace with".to_string()
        } else {
            self.find.replacement.clone()
        };
        let pattern_empty = self.find.pattern.is_empty();
        let replacement_empty = self.find.replacement.is_empty();
        let pattern_active = self.find.field == FindField::Pattern;
        let replacement_active = self.find.field == FindField::Replacement;
        let status = if let Some(error) = &self.find.error {
            error.clone()
        } else if let Some(message) = &self.find.message {
            message.clone()
        } else if !self.find.pattern.is_empty() && self.find.matches.is_empty() {
            "no matches".to_string()
        } else if !self.find.matches.is_empty() {
            format!("{}/{}", self.find.current + 1, self.find.matches.len())
        } else {
            String::new()
        };
        let status_color = if self.find.error.is_some() {
            theme.warn
        } else {
            theme.muted
        };
        let active_caret = |active: bool| if active { "\u{2502}" } else { "" };

        let field = |text: String, empty: bool, active: bool, replacement: bool| {
            div()
                .id(if replacement {
                    "find-replace"
                } else {
                    "find-pattern"
                })
                .flex_1()
                .min_w(px(110.0))
                .px_2()
                .py_1()
                .rounded_md()
                .bg(theme.input_bg)
                .border_1()
                .border_color(if active { theme.accent } else { theme.border })
                .text_sm()
                .text_color(if empty { theme.muted } else { theme.text })
                .cursor_pointer()
                .child(format!("{text}{}", active_caret(active)))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _event, _window, cx| {
                        this.find.field = if replacement {
                            FindField::Replacement
                        } else {
                            FindField::Pattern
                        };
                        cx.notify();
                    }),
                )
        };

        let toggle =
            |id: &'static str,
             label: &'static str,
             on: bool,
             action: fn(&mut Self, &MouseDownEvent, &mut Window, &mut Context<Self>)| {
                div()
                    .id(id)
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .cursor_pointer()
                    .bg(if on {
                        theme.accent.opacity(0.25)
                    } else {
                        theme.chip
                    })
                    .text_color(if on { theme.accent_text } else { theme.muted })
                    .text_xs()
                    .child(label)
                    .on_mouse_down(MouseButton::Left, cx.listener(action))
            };

        let button =
            |id: &'static str,
             label: &'static str,
             action: fn(&mut Self, &MouseDownEvent, &mut Window, &mut Context<Self>)| {
                div()
                    .id(id)
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .cursor_pointer()
                    .bg(theme.chip)
                    .text_color(theme.text_dim)
                    .text_xs()
                    .hover(move |style| style.bg(theme.hover))
                    .child(label)
                    .on_mouse_down(MouseButton::Left, cx.listener(action))
            };

        div()
            .id("find-bar")
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .h(px(36.0))
            .px_3()
            .flex_shrink_0()
            .bg(theme.panel)
            .border_b_1()
            .border_color(theme.border)
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted)
                    .child(if self.find.replace { "REPLACE" } else { "FIND" }),
            )
            .child(field(pattern, pattern_empty, pattern_active, false))
            .child(toggle(
                "find-regex",
                ".*",
                self.find.regex,
                Self::find_toggle_regex_button,
            ))
            .child(toggle(
                "find-case",
                "Aa",
                self.find.case_sensitive,
                Self::find_toggle_case_button,
            ))
            .child(toggle(
                "find-word",
                "W",
                self.find.whole_word,
                Self::find_toggle_word_button,
            ))
            .child(
                div()
                    .min_w(px(56.0))
                    .text_xs()
                    .text_color(status_color)
                    .child(status),
            )
            .when(self.find.replace, |row| {
                row.child(field(
                    replacement,
                    replacement_empty,
                    replacement_active,
                    true,
                ))
            })
            .child(button("find-prev", "prev", Self::find_prev_button))
            .child(button("find-next", "next", Self::find_next_button))
            .when(self.find.replace, |row| {
                row.child(button(
                    "find-replace-one",
                    "replace",
                    Self::find_replace_one_button,
                ))
                .child(button(
                    "find-replace-all",
                    "all",
                    Self::find_replace_all_button,
                ))
            })
            .child(
                div()
                    .id("find-close")
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .cursor_pointer()
                    .text_color(theme.muted)
                    .hover(move |style| style.bg(theme.hover))
                    .child("\u{00d7}")
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _event, _window, cx| {
                            this.find.open = false;
                            this.find.error = None;
                            this.find.message = None;
                            cx.notify();
                        }),
                    ),
            )
            .into_any_element()
    }

    fn find_toggle_regex_button(
        &mut self,
        _: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.find.regex = !self.find.regex;
        self.refresh_find(true);
        cx.notify();
    }

    fn find_toggle_case_button(
        &mut self,
        _: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.find.case_sensitive = !self.find.case_sensitive;
        self.refresh_find(true);
        cx.notify();
    }

    fn find_toggle_word_button(
        &mut self,
        _: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.find.whole_word = !self.find.whole_word;
        self.refresh_find(true);
        cx.notify();
    }

    fn find_next_button(
        &mut self,
        _: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.find_next(false);
        cx.notify();
    }

    fn find_prev_button(
        &mut self,
        _: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.find_next(true);
        cx.notify();
    }

    fn find_replace_one_button(
        &mut self,
        _: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.find_replace_one();
        cx.notify();
    }

    fn find_replace_all_button(
        &mut self,
        _: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.find_replace_all();
        cx.notify();
    }

    /// Character index under a window-space point, using the geometry from the last paint.
    fn index_for_point(&self, position: Point<Pixels>) -> Option<usize> {
        let bounds = self.last_bounds?;
        let local = bounds.localize(&position)?;

        let row = (f32::from(local.y) / f32::from(self.line_height))
            .floor()
            .max(0.0) as usize;
        let meta = *self.last_rows.get(row)?;
        let shaped = self.last_shaped.get(row)?;

        let row_text: String = self
            .buffer
            .line(meta.line)
            .chars()
            .skip(meta.start)
            .take(meta.end.saturating_sub(meta.start))
            .collect();

        let byte = shaped.closest_index_for_x(local.x);
        let col_in_row = row_text[..byte.min(row_text.len())].chars().count();
        Some(
            self.buffer
                .char_index_at(meta.line, meta.start + col_in_row),
        )
    }

    /// Fold or unfold a heading when its gutter marker is clicked.
    fn fold_from_gutter(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) -> bool {
        let Some(bounds) = self.last_bounds else {
            return false;
        };
        let x = position.x;
        // The gutter sits just left of the text column, beyond the editor's padding.
        let right = bounds.left() - px(26.0);
        let left = right - px(GUTTER_WIDTH);
        if x >= right || x < left {
            return false;
        }
        let y = position.y - bounds.top();
        if y < px(0.0) {
            return false;
        }
        let row = (f32::from(y) / f32::from(self.line_height)).floor() as usize;
        let Some(meta) = self.last_rows.get(row) else {
            return false;
        };
        if meta.start != 0 {
            return false;
        }
        if self.toggle_fold_line(meta.line) {
            cx.notify();
            return true;
        }
        false
    }

    /// Capture the selection before the platform turns the first `[` into text, so
    /// `[[` over a selection can still use that text as the link alias.
    fn on_pre_key_down(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.reading {
            return;
        }
        if self.find.open {
            // Tab and Shift+Tab move between the pattern and the replacement field.
            if event.keystroke.key == "tab" && self.find.replace {
                self.find.field = if event.keystroke.modifiers.shift {
                    FindField::Pattern
                } else {
                    FindField::Replacement
                };
                cx.stop_propagation();
                cx.notify();
            }
            return;
        }
        match event.keystroke.key_char.as_deref() {
            Some("[") => {
                if self.selection().is_some() {
                    self.capture_link_alias("[");
                    self.pending_link_start = true;
                }
                // Keep `swallow_bracket`: the text-input event for this key fires next.
            }
            Some(_) | None if event.keystroke.key == "enter" || event.keystroke.key == "escape" => {
                self.pending_link_start = false;
                self.swallow_bracket = false;
            }
            Some(_) => {
                self.pending_link_start = false;
                self.swallow_bracket = false;
            }
            None => {}
        }
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_autocomplete();
        window.focus(&self.focus_handle);
        if self.fold_from_gutter(event.position, cx) {
            return;
        }
        let Some(index) = self.index_for_point(event.position) else {
            return;
        };

        if self.find.open {
            self.find.open = false;
            self.find.error = None;
            self.find.message = None;
        }

        if event.modifiers.shift {
            self.begin_selection();
        } else if event.modifiers.alt {
            let state = CursorState {
                cursor: index,
                anchor: None,
            };
            if !self.extra_cursors.contains(&state) {
                self.extra_cursors.push(state);
            }
            self.selecting = false;
            cx.notify();
            return;
        } else {
            self.extra_cursors.clear();
            self.anchor = Some(index);
        }
        self.buffer.set_cursor(index);
        self.selecting = true;
        cx.notify();
    }

    fn on_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.selecting {
            return;
        }
        if let Some(index) = self.index_for_point(event.position) {
            self.buffer.set_cursor(index);
            cx.notify();
        }
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _window: &mut Window, cx: &mut Context<Self>) {
        self.selecting = false;
        if self.selection().is_none() {
            self.anchor = None;
        }
        cx.notify();
    }
}

impl Focusable for Editor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for Editor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.global::<Theme>();

        // Keep the caret on screen before the tree is laid out, so typing never runs off
        // the bottom of a long note.
        self.unfold_cursor_section();
        self.keep_cursor_visible();
        self.schedule_draft(cx);

        // Text style for rich runs (highlights) in reading mode.
        let style = window.text_style();
        let run_template = TextRun {
            len: 0,
            font: style.font(),
            color: style.color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };

        let title = self
            .path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "Untitled".to_string());
        let dirty = self.buffer.is_dirty();
        let reading = self.reading;

        self.refresh_properties();
        self.refresh_stats();

        let body: AnyElement = if reading {
            let version = self.document_version();
            if self.reading_content.needs_update(version) {
                let text = self.buffer.text();
                let root = self.vault_root.clone();
                self.reading_content.update(version, &text, root.as_deref());
            }

            div()
                .flex_1()
                .min_h(px(0.0))
                .child({
                    let fold: crate::reading::FoldToggle = {
                        let weak = cx.entity().downgrade();
                        Rc::new(move |index: usize, window: &mut Window, cx: &mut App| {
                            let _ = weak.update(cx, |editor, cx| {
                                editor.reading_content.toggle_fold(index);
                                cx.notify();
                            });
                            window.refresh();
                        })
                    };
                    self.reading_content.element(
                        self.vault_root.clone(),
                        theme,
                        run_template.clone(),
                        self.link_opener.clone(),
                        Some(fold),
                        cx,
                    )
                })
                .into_any_element()
        } else {
            div()
                .id("editor-scroll")
                .flex_1()
                .min_h(px(0.0))
                .cursor(CursorStyle::IBeam)
                .overflow_y_scroll()
                .track_scroll(&self.scroll_handle)
                .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
                .on_mouse_move(cx.listener(Self::on_mouse_move))
                .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
                .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_start()
                        .py_4()
                        .child(LineNumbers {
                            editor: cx.entity(),
                        })
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .overflow_hidden()
                                .px_6()
                                .child(div().w_full().max_w(px(880.0)).mx_auto().child(
                                    EditorText {
                                        editor: cx.entity(),
                                    },
                                )),
                        ),
                )
                .into_any_element()
        };

        let autocomplete_bar = if self.autocomplete.open {
            let entity = cx.entity();
            let kind = match self.autocomplete.kind {
                Some(SuggestionKind::Link) => "link",
                Some(SuggestionKind::Tag) => "tag",
                Some(SuggestionKind::Slash) => "command",
                None => "",
            };
            let selected = self.autocomplete.selected;
            let query = self.autocomplete.query.clone();
            let rows: Vec<AnyElement> = self
                .autocomplete
                .items
                .iter()
                .take(8)
                .enumerate()
                .map(|(index, item)| {
                    let entity = entity.clone();
                    let mut row = div()
                        .id(ElementId::Name(format!("autocomplete-{index}").into()))
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_3()
                        .h(px(24.0))
                        .px_3()
                        .text_sm()
                        .text_color(theme.text)
                        .cursor_pointer()
                        .child(item.label.clone())
                        .child(div().text_color(theme.muted).child(item.detail.clone()));
                    if index == selected {
                        row = row.bg(theme.accent);
                    } else {
                        row = row.hover(move |style| style.bg(theme.hover));
                    }
                    row.on_mouse_down(MouseButton::Left, move |_event, _window, cx| {
                        entity.update(cx, |this, cx| {
                            this.autocomplete.selected = index;
                            if this.accept_autocomplete() {
                                cx.notify();
                            }
                        });
                    })
                    .into_any_element()
                })
                .collect();

            Some(
                div()
                    .id("autocomplete")
                    .flex()
                    .flex_col()
                    .flex_shrink_0()
                    .max_h(px(220.0))
                    .overflow_y_scroll()
                    .bg(theme.input_bg)
                    .border_t_1()
                    .border_color(theme.border)
                    .child(
                        div()
                            .px_3()
                            .py_1()
                            .text_xs()
                            .text_color(theme.muted)
                            .child(format!("{kind}: {query}")),
                    )
                    .children(rows)
                    .into_any_element(),
            )
        } else {
            None
        };

        // The format bar keeps a fixed height whether or not it has buttons, so selecting
        // text never resizes the editor underneath the caret.
        const FORMAT_BAR_HEIGHT: f32 = 30.0;
        let format_bar = if reading {
            None
        } else if self.selection().is_some() {
            let entity = cx.entity();
            let buttons: Vec<(&str, text_format::Style, &str)> = vec![
                ("B", text_format::Style::Bold, "format-bold"),
                ("I", text_format::Style::Italic, "format-italic"),
                ("S", text_format::Style::Strikethrough, "format-strike"),
                ("`", text_format::Style::InlineCode, "format-code"),
                ("==", text_format::Style::Highlight, "format-highlight"),
                ("%%", text_format::Style::Comment, "format-comment"),
            ];
            let buttons: Vec<AnyElement> = buttons
                .into_iter()
                .map(|(label, style, id)| {
                    let entity = entity.clone();
                    div()
                        .id(ElementId::Name(id.into()))
                        .px_2()
                        .py_1()
                        .rounded_sm()
                        .cursor_pointer()
                        .text_color(theme.text)
                        .hover(move |style| style.bg(theme.hover))
                        .child(label.to_string())
                        .on_mouse_down(MouseButton::Left, move |_event, window, cx| {
                            entity.update(cx, |this, cx| {
                                this.apply_style(style);
                                window.focus(&this.focus_handle);
                                cx.notify();
                            });
                        })
                        .into_any_element()
                })
                .collect();

            let link_entity = cx.entity();
            let link = div()
                .id("format-link")
                .px_2()
                .py_1()
                .rounded_sm()
                .cursor_pointer()
                .text_color(theme.text)
                .hover(move |style| style.bg(theme.hover))
                .child("[[ ]]")
                .on_mouse_down(MouseButton::Left, move |_event, window, cx| {
                    link_entity.update(cx, |this, cx| {
                        this.start_wikilink();
                        window.focus(&this.focus_handle);
                        cx.notify();
                    });
                })
                .into_any_element();

            Some(
                div()
                    .id("format-bar")
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .h(px(FORMAT_BAR_HEIGHT))
                    .px_3()
                    .flex_shrink_0()
                    .bg(theme.panel_alt)
                    .border_t_1()
                    .border_color(theme.border)
                    .text_xs()
                    .text_color(theme.text)
                    .child(div().mr_1().text_color(theme.muted).child("Format"))
                    .children(buttons)
                    .child(link)
                    .into_any_element(),
            )
        } else {
            Some(
                div()
                    .id("format-bar-spacer")
                    .h(px(FORMAT_BAR_HEIGHT))
                    .flex_shrink_0()
                    .bg(theme.panel_alt)
                    .border_t_1()
                    .border_color(theme.border)
                    .into_any_element(),
            )
        };

        let (line, column) = self.buffer.line_col();
        let line_count = self.buffer.line_count();
        let words = self.stats_cache.1.words;
        // Reading replaces the line/column readout with the section and how far in it is.
        let reading_status = if reading {
            let (heading, progress) = self.reading_outline_state();
            Some((heading, progress))
        } else {
            None
        };
        let git_stat = self.git_stat;
        let selection = self.selection_stats();
        let vim_status = self.vim_enabled().then(|| self.vim_status());
        let find_status = self.find.open.then(|| {
            if self.find.matches.is_empty() {
                "find".to_string()
            } else {
                format!("find {}/{}", self.find.current + 1, self.find.matches.len())
            }
        });
        let extra_cursors = (!self.extra_cursors.is_empty())
            .then(|| format!("{} cursors", self.extra_cursors.len() + 1));
        let status = div()
            .flex()
            .flex_row()
            .items_center()
            .gap_4()
            .h(px(22.0))
            .px_3()
            .flex_shrink_0()
            .bg(theme.panel_alt)
            .border_t_1()
            .border_color(theme.border)
            .text_xs()
            .text_color(theme.muted)
            .child(match &reading_status {
                Some((Some(heading), progress)) => {
                    format!("{heading} · {:.0}%", progress * 100.0)
                }
                Some((None, progress)) => format!("{:.0}% read", progress * 100.0),
                None => format!("Ln {}, Col {}", line + 1, column + 1),
            })
            .child(format!("{line_count} lines"))
            .child(format!("{words} words"))
            .when_some(extra_cursors, |d, label| {
                d.child(div().text_color(theme.accent_text).child(label))
            })
            .when_some(vim_status, |d, label| {
                d.child(
                    div()
                        .px_1()
                        .rounded_sm()
                        .bg(theme.chip)
                        .text_color(theme.accent_text)
                        .child(label),
                )
            })
            .when_some(find_status, |d, label| {
                d.child(div().text_color(theme.accent_text).child(label))
            })
            .when_some(git_stat, |d, stat| {
                d.child(
                    div()
                        .flex()
                        .gap_2()
                        .child(
                            div()
                                .text_color(theme.code_text)
                                .child(format!("+{}", stat.added)),
                        )
                        .child(
                            div()
                                .text_color(theme.warn)
                                .child(format!("-{}", stat.removed)),
                        ),
                )
            })
            .when_some(selection, |d, stats| {
                d.child(format!(
                    "sel: {} words · {} chars · {} lines",
                    stats.words, stats.characters, stats.lines
                ))
            });

        let find_bar = self.find.open.then(|| self.render_find_bar(theme, cx));

        div()
            .key_context("Editor")
            .track_focus(&self.focus_handle)
            // Files dropped on the editor are filed into the attachments folder.
            .on_drop::<ExternalPaths>(cx.listener(|this, paths: &ExternalPaths, _window, cx| {
                this.drop_files(paths.paths(), cx);
            }))
            // Action handlers must live on the node that tracks focus: GPUI dispatches actions
            // only along the root-to-focus path, so handlers on descendants would never run.
            .on_action(cx.listener(Self::toggle_reading))
            .on_action(cx.listener(Self::toggle_fold))
            .on_action(cx.listener(Self::toggle_bold))
            .on_action(cx.listener(Self::toggle_italic))
            .on_action(cx.listener(Self::toggle_strikethrough))
            .on_action(cx.listener(Self::toggle_highlight))
            .on_action(cx.listener(Self::toggle_inline_code))
            .on_action(cx.listener(Self::toggle_comment))
            .on_action(cx.listener(Self::insert_wikilink))
            .on_key_down(cx.listener(Self::on_pre_key_down))
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::delete_word_left))
            .on_action(cx.listener(Self::delete_word_right))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::word_left))
            .on_action(cx.listener(Self::word_right))
            .on_action(cx.listener(Self::paragraph_up))
            .on_action(cx.listener(Self::paragraph_down))
            .on_action(cx.listener(Self::document_start))
            .on_action(cx.listener(Self::document_end))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_up))
            .on_action(cx.listener(Self::select_down))
            .on_action(cx.listener(Self::select_word_left))
            .on_action(cx.listener(Self::select_word_right))
            .on_action(cx.listener(Self::select_document_start))
            .on_action(cx.listener(Self::select_document_end))
            .on_action(cx.listener(Self::select_home))
            .on_action(cx.listener(Self::select_end))
            .on_action(cx.listener(Self::on_select_all))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::undo))
            .on_action(cx.listener(Self::redo))
            .on_action(cx.listener(Self::enter))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::on_save))
            .on_action(cx.listener(Self::move_block_up_action))
            .on_action(cx.listener(Self::move_block_down_action))
            .on_action(cx.listener(Self::open_find_action))
            .on_action(cx.listener(Self::open_find_replace_action))
            .on_action(cx.listener(Self::find_next_action))
            .on_action(cx.listener(Self::find_prev_action))
            .on_action(cx.listener(Self::find_replace_one_action))
            .on_action(cx.listener(Self::find_replace_all_action))
            .on_action(cx.listener(Self::find_toggle_regex_action))
            .on_action(cx.listener(Self::find_toggle_case_action))
            .on_action(cx.listener(Self::find_toggle_word_action))
            .on_action(cx.listener(Self::select_next_occurrence_action))
            .on_action(cx.listener(Self::add_cursor_above_action))
            .on_action(cx.listener(Self::add_cursor_below_action))
            .flex()
            .flex_col()
            .size_full()
            .child(
                div().flex().flex_col().bg(theme.panel_alt).child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .h(px(32.0))
                        .px_3()
                        .border_b_1()
                        .border_color(theme.border)
                        .text_xs()
                        .text_color(theme.muted)
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(
                                    div()
                                        .font_weight(gpui::FontWeight::BOLD)
                                        .text_color(theme.text)
                                        .child(title),
                                )
                                .when(dirty, |d| {
                                    d.child(div().text_color(theme.warn).child("\u{25CF}"))
                                }),
                        )
                        .child(
                            div()
                                .id("mode-toggle")
                                .px_2()
                                .py_1()
                                .rounded_md()
                                .cursor_pointer()
                                .bg(theme.chip)
                                .text_color(if reading {
                                    theme.accent_text
                                } else {
                                    theme.muted
                                })
                                .hover(move |style| style.bg(theme.hover))
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(|this, _ev, _window, cx| {
                                        this.reading = !this.reading;
                                        cx.notify();
                                    }),
                                )
                                .child(if reading { "READ" } else { "EDIT" }),
                        ),
                ),
            )
            .when_some(find_bar, |d, bar| d.child(bar))
            .child(body)
            .when_some(format_bar, |d, bar| d.child(bar))
            .when_some(autocomplete_bar, |d, bar| d.child(bar))
            .child(status)
    }
}

impl EntityInputHandler for Editor {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let start = self.buffer.utf16_to_char(range_utf16.start);
        let end = self.buffer.utf16_to_char(range_utf16.end);
        actual_range.replace(self.buffer.char_to_utf16(start)..self.buffer.char_to_utf16(end));
        Some(self.buffer.slice(start..end))
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let cursor = self.buffer.cursor();
        let range = self.selection().unwrap_or(cursor..cursor);
        Some(UTF16Selection {
            range: self.buffer.char_to_utf16(range.start)..self.buffer.char_to_utf16(range.end),
            reversed: false,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        let range = self.marked_range.clone()?;
        Some(self.buffer.char_to_utf16(range.start)..self.buffer.char_to_utf16(range.end))
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        self.marked_range = None;
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.reading {
            // Reading mode owns its own navigation: j/k and g/G scroll the view, and every
            // other keystroke is swallowed so it cannot reach the hidden buffer.
            match new_text {
                "j" => self.reading_content.scroll_blocks(1),
                "k" => self.reading_content.scroll_blocks(-1),
                "g" => self.reading_content.scroll_blocks(isize::MIN / 2),
                "G" => self.reading_content.scroll_blocks(isize::MAX / 2),
                _ => return,
            }
            self.marked_range = None;
            cx.notify();
            return;
        }
        if !self.can_mutate() {
            return;
        }
        if self.find.open {
            self.find_append(new_text);
            self.marked_range = None;
            cx.notify();
            return;
        }
        if self.vim.enabled() && self.vim.mode() != VimMode::Insert {
            let mut anchor = self.anchor;
            let outcome = self
                .vim
                .handle_text(new_text, &mut self.buffer, &mut anchor);
            self.anchor = anchor;
            self.vim.set_message(outcome.message.clone());
            self.run_vim_outcome(&outcome, window, cx);
            if outcome.handled {
                self.marked_range = None;
                cx.notify();
                return;
            }
        }

        // A `[` pressed over a selection starts the wikilink immediately. Waiting for the
        // second `[` would already have replaced the selected text, losing the alias.
        if self.pending_link_start && (new_text == "[" || new_text == "[[") {
            self.pending_link_start = false;
            let swallow = new_text == "[";
            if !self.wrap_selection_as_wikilink(swallow) {
                self.swallow_bracket = swallow;
                self.start_wikilink();
            }
            self.marked_range = None;
            cx.notify();
            return;
        }
        if self.swallow_bracket && new_text == "[" {
            self.swallow_bracket = false;
            self.marked_range = None;
            self.refresh_autocomplete();
            cx.notify();
            return;
        }
        self.swallow_bracket = false;
        self.pending_link_start = false;

        self.capture_link_alias(new_text);
        let range = self
            .range_from_utf16(range_utf16)
            .or_else(|| self.marked_range.clone())
            .or_else(|| self.selection());

        match range {
            Some(range) => self.buffer.replace(range, new_text),
            None if !new_text.is_empty() => self.buffer.insert(new_text),
            None => {}
        }

        self.anchor = None;
        self.marked_range = None;
        self.refresh_autocomplete();
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        _new_selected_range_utf16: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.can_mutate() {
            return;
        }
        if self.find.open {
            self.find_append(new_text);
            cx.notify();
            return;
        }
        if self.vim.enabled() && self.vim.mode() != VimMode::Insert {
            let mut anchor = self.anchor;
            let outcome = self
                .vim
                .handle_text(new_text, &mut self.buffer, &mut anchor);
            self.anchor = anchor;
            self.vim.set_message(outcome.message);
            if outcome.handled {
                self.marked_range = None;
                cx.notify();
                return;
            }
        }

        if self.pending_link_start && (new_text == "[" || new_text == "[[") {
            self.pending_link_start = false;
            let swallow = new_text == "[";
            if !self.wrap_selection_as_wikilink(swallow) {
                self.swallow_bracket = swallow;
                self.start_wikilink();
            }
            self.marked_range = None;
            cx.notify();
            return;
        }
        if self.swallow_bracket && new_text == "[" {
            self.swallow_bracket = false;
            self.marked_range = None;
            self.refresh_autocomplete();
            cx.notify();
            return;
        }
        self.swallow_bracket = false;
        self.pending_link_start = false;

        self.capture_link_alias(new_text);
        let range = self
            .range_from_utf16(range_utf16)
            .or_else(|| self.marked_range.clone())
            .or_else(|| self.selection())
            .unwrap_or_else(|| self.buffer.cursor()..self.buffer.cursor());
        let start = range.start;
        self.buffer.replace(range, new_text);
        self.anchor = None;
        self.marked_range = if new_text.is_empty() {
            None
        } else {
            Some(start..start + new_text.chars().count())
        };
        self.refresh_autocomplete();
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let start = self.buffer.utf16_to_char(range_utf16.start);
        let (line, col) = self.buffer.line_col_at(start);

        let row = self
            .last_rows
            .iter()
            .rposition(|meta| meta.line == line && meta.start <= col)?;
        let meta = self.last_rows[row];
        let shaped = self.last_shaped.get(row)?;

        let col = col.min(meta.end);
        let byte = self.buffer.char_col_to_byte(line, col)
            - self.buffer.char_col_to_byte(line, meta.start);
        let x = shaped.x_for_index(byte);
        let top = bounds.top() + self.line_height * row as f32;

        Some(Bounds::from_corners(
            point(bounds.left() + x, top),
            point(bounds.left() + x, top + self.line_height),
        ))
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        self.index_for_point(point)
            .map(|index| self.buffer.char_to_utf16(index))
    }
}

impl Editor {
    fn range_from_utf16(&self, range_utf16: Option<Range<usize>>) -> Option<Range<usize>> {
        range_utf16.map(|r| self.buffer.utf16_to_char(r.start)..self.buffer.utf16_to_char(r.end))
    }
}

impl LineLayout {
    /// Tokenise, wrap and shape one logical line.
    #[allow(clippy::too_many_arguments)]
    fn shape(
        text: &str,
        in_code: bool,
        language: &str,
        font: &gpui::Font,
        font_size: Pixels,
        width: f32,
        color: gpui::Hsla,
        theme: Theme,
        window: &mut Window,
    ) -> Self {
        let chars: Vec<char> = text.chars().collect();
        let tokens = syntax::line_tokens(text, in_code);

        // Inside a fence with a known language, colour the line with the code lexer.
        let code_kinds: Option<Vec<syntax::CodeKind>> = if in_code && !language.is_empty() {
            let mut kinds = Vec::with_capacity(chars.len());
            for span in syntax::code_spans(text, language) {
                kinds.extend(std::iter::repeat_n(span.kind, span.text.chars().count()));
            }
            (kinds.len() == chars.len()).then_some(kinds)
        } else {
            None
        };

        // Byte offset of every character column, for measuring and slicing.
        let mut offsets = Vec::with_capacity(chars.len() + 1);
        let mut acc = 0usize;
        for ch in &chars {
            offsets.push(acc);
            acc += ch.len_utf8();
        }
        offsets.push(acc);

        let shape = |text: &str, runs: &[TextRun]| {
            window.text_system().shape_line(
                SharedString::from(text.to_string()),
                font_size,
                runs,
                None,
            )
        };

        let spans: Vec<(usize, usize)> = if chars.is_empty() {
            vec![(0, 0)]
        } else {
            // Measure the line once in a single run, then break it into visual rows.
            let full_run = TextRun {
                len: text.len(),
                font: font.clone(),
                color,
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            let full = shape(text, &[full_run]);
            let width_of = |from: usize, to: usize| -> f32 {
                f32::from(full.x_for_index(offsets[to]))
                    - f32::from(full.x_for_index(offsets[from]))
            };
            wrap::wrap(&chars, width_of, width)
        };

        let mut rows: Vec<Rc<ShapedLine>> = Vec::with_capacity(spans.len());
        for &(start, end) in &spans {
            let segment: String = chars[start..end].iter().collect();
            let runs = match &code_kinds {
                Some(kinds) => {
                    runs_for_code(&kinds[start..end], &offsets[start..=end], font, theme)
                }
                None => runs_for(
                    &tokens[start..end],
                    &offsets[start..=end],
                    font,
                    color,
                    theme,
                ),
            };
            rows.push(Rc::new(shape(&segment, &runs)));
        }

        Self {
            text: text.to_string(),
            in_code,
            language: language.to_string(),
            spans: Rc::new(spans),
            rows: Rc::new(rows),
        }
    }
}

/// The language tag of a fence line, e.g. `rust` for ```` ```rust ````.
fn fence_language(line: &str) -> String {
    let trimmed = line.trim_start();
    let rest = trimmed
        .trim_start_matches('`')
        .trim_start_matches('~')
        .trim();
    rest.split_whitespace().next().unwrap_or("").to_string()
}

/// Build runs for a code row, grouping consecutive characters of the same code token.
fn runs_for_code(
    kinds: &[syntax::CodeKind],
    offsets: &[usize],
    font: &gpui::Font,
    theme: Theme,
) -> Vec<TextRun> {
    let mut runs: Vec<TextRun> = Vec::new();
    let mut index = 0;

    while index < kinds.len() {
        let kind = kinds[index];
        let mut end = index + 1;
        while end < kinds.len() && kinds[end] == kind {
            end += 1;
        }

        let len = offsets[end] - offsets[index];
        if len > 0 {
            runs.push(TextRun {
                len,
                font: font.clone(),
                color: theme.code_color(kind),
                background_color: None,
                underline: None,
                strikethrough: None,
            });
        }

        index = end;
    }

    runs
}

/// Build runs for a row, grouping consecutive characters of the same token kind.
///
/// `offsets` spans the row's characters plus one, so byte lengths are plain differences.
fn runs_for(
    kinds: &[syntax::TokenKind],
    offsets: &[usize],
    font: &gpui::Font,
    base_color: gpui::Hsla,
    theme: Theme,
) -> Vec<TextRun> {
    let mut runs: Vec<TextRun> = Vec::new();
    let mut index = 0;

    while index < kinds.len() {
        let kind = kinds[index];
        let mut end = index + 1;
        while end < kinds.len() && kinds[end] == kind {
            end += 1;
        }

        let len = offsets[end] - offsets[index];
        let run_font = if kind == syntax::TokenKind::Heading {
            gpui::Font {
                weight: gpui::FontWeight::BOLD,
                ..font.clone()
            }
        } else {
            font.clone()
        };

        if len > 0 {
            runs.push(TextRun {
                len,
                font: run_font,
                color: token_color(kind, base_color, theme),
                background_color: None,
                underline: None,
                strikethrough: None,
            });
        }

        index = end;
    }

    runs
}

fn token_color(kind: syntax::TokenKind, base: gpui::Hsla, theme: Theme) -> gpui::Hsla {
    match kind {
        syntax::TokenKind::Text => base,
        syntax::TokenKind::Heading => theme.warn,
        syntax::TokenKind::Emphasis => theme.text_dim,
        syntax::TokenKind::Code | syntax::TokenKind::CodeBlock => theme.code_text,
        syntax::TokenKind::Link => theme.accent_text,
        syntax::TokenKind::Markup => theme.star,
        syntax::TokenKind::Math => theme.code_text,
        syntax::TokenKind::Comment => theme.muted,
    }
}

/// Milliseconds since the epoch, for unique attachment names.
fn stamp() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0)
}

fn image_extension(format: ImageFormat) -> &'static str {
    match format {
        ImageFormat::Png => "png",
        ImageFormat::Jpeg => "jpg",
        ImageFormat::Webp => "webp",
        ImageFormat::Gif => "gif",
        ImageFormat::Svg => "svg",
        ImageFormat::Bmp => "bmp",
        ImageFormat::Tiff => "tiff",
    }
}

fn is_image_file(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|extension| extension.to_str())
            .map(|extension| extension.to_ascii_lowercase())
            .as_deref(),
        Some("png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "svg" | "avif")
    )
}

fn sanitize(name: &str) -> String {
    name.chars()
        .map(|ch| match ch {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '-',
            other => other,
        })
        .collect::<String>()
        .trim()
        .to_string()
}

/// Shorten a long summary to roughly two lines in the reading header.
fn truncate_summary(text: &str, max_chars: usize) -> String {
    let mut out: String = text.chars().take(max_chars).collect();
    if text.chars().count() > max_chars {
        out.push('…');
    }
    out
}

/// A callback that toggles the metadata panel.
pub(crate) type MetadataToggle =
    Rc<dyn Fn(&gpui::MouseDownEvent, &mut gpui::Window, &mut gpui::App)>;

/// The frontmatter of a note, shown above the document with clickable links and lists.
pub(crate) fn metadata_panel(
    properties: &[Property],
    theme: Theme,
    opener: Option<LinkOpener>,
    reading: bool,
    collapsed: bool,
    on_toggle: Option<MetadataToggle>,
) -> Option<AnyElement> {
    if properties.is_empty() {
        return None;
    }

    let find = |key: &str| {
        properties
            .iter()
            .find(|property| property.key.eq_ignore_ascii_case(key))
            .map(|property| property.value.display())
            .filter(|value| !value.trim().is_empty())
    };

    let mut rows: Vec<AnyElement> = Vec::new();

    if let Some(title) = find("title") {
        rows.push(
            div()
                .text_lg()
                .font_weight(gpui::FontWeight::BOLD)
                .text_color(theme.text)
                .child(metadata_inline(&title, theme, &opener, 18.0))
                .into_any_element(),
        );
    }

    for key in ["description", "excerpt"] {
        if reading && key == "excerpt" {
            // The excerpt is a second, longer summary; one of them is enough on the
            // reading header, and dropping it keeps the note visible.
            continue;
        }
        if let Some(text) = find(key) {
            let text = if reading {
                truncate_summary(&text, 180)
            } else {
                text
            };
            rows.push(
                div()
                    .text_sm()
                    .line_height(relative(1.4))
                    .text_color(theme.text_dim)
                    .child(metadata_inline(&text, theme, &opener, 13.0))
                    .into_any_element(),
            );
        }
    }

    for property in properties {
        let key = property.key.to_ascii_lowercase();
        if matches!(key.as_str(), "title" | "description" | "excerpt") {
            continue;
        }
        // Empty keys (an unused `Next:` or a list that did not parse) add noise without
        // information; the properties editor still shows them.
        if property.value.display().trim().is_empty() {
            continue;
        }

        let value = if key == "tags" {
            metadata_tags(&property.value, theme, &opener)
        } else {
            metadata_value(&property.value, theme, &opener, reading)
        };

        rows.push(
            div()
                .flex()
                .flex_row()
                .items_start()
                .gap_2()
                .child(
                    div()
                        .w(px(104.0))
                        .flex_shrink_0()
                        .pt(px(1.0))
                        .text_xs()
                        .text_color(theme.muted)
                        .child(property.key.clone()),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .text_sm()
                        .text_color(theme.text)
                        .child(value),
                )
                .into_any_element(),
        );
    }

    let mut header = div()
        .w_full()
        .flex()
        .flex_row()
        .items_center()
        .gap_2()
        .px_4()
        .py_2()
        .cursor_pointer()
        .child(div().text_xs().text_color(theme.muted).child(format!(
            "PROPERTIES  {}{}",
            properties.len(),
            if collapsed { "  +" } else { "  -" }
        )));
    if let Some(toggle) = on_toggle {
        header = header.on_mouse_down(MouseButton::Left, move |event, window, cx| {
            toggle(event, window, cx);
        });
    }

    let body = div()
        .id("metadata-body")
        .flex()
        .flex_col()
        .gap_2()
        .px_4()
        .pb_3()
        .max_h(px(280.0))
        .overflow_y_scroll()
        .children(rows);

    Some(
        div()
            .id("metadata")
            .w_full()
            .flex()
            .flex_col()
            .bg(theme.panel_alt)
            .border_b_1()
            .border_color(theme.border)
            .flex_shrink_0()
            .child(header)
            .when(!collapsed, |d| d.child(body))
            .into_any_element(),
    )
}

/// A collapsible metadata header for the reading view.
pub(crate) struct MetadataView {
    properties: Vec<Property>,
    theme: Theme,
    opener: Option<LinkOpener>,
    collapsed: bool,
}

impl MetadataView {
    pub(crate) fn new(cx: &mut Context<Self>) -> Self {
        Self {
            properties: Vec::new(),
            theme: *cx.global::<Theme>(),
            opener: None,
            collapsed: false,
        }
    }

    /// Sync with the current document. Only notifies when the visible content changed.
    pub(crate) fn set(
        &mut self,
        properties: Vec<Property>,
        theme: Theme,
        opener: Option<LinkOpener>,
        cx: &mut Context<Self>,
    ) {
        let changed = self.properties != properties || self.theme != theme;
        self.properties = properties;
        self.theme = theme;
        self.opener = opener;
        if changed {
            cx.notify();
        }
    }
}

impl Render for MetadataView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let handle = cx.entity().downgrade();
        let toggle: MetadataToggle = Rc::new(move |_event, window, cx| {
            let _ = handle.update(cx, |this, cx| {
                this.collapsed = !this.collapsed;
                cx.notify();
            });
            window.refresh();
        });
        div().w_full().children(metadata_panel(
            &self.properties,
            self.theme,
            self.opener.clone(),
            true,
            self.collapsed,
            Some(toggle),
        ))
    }
}

fn metadata_tags(value: &PropertyValue, theme: Theme, opener: &Option<LinkOpener>) -> AnyElement {
    let items: Vec<String> = match value {
        PropertyValue::Scalar(text) => vec![text.clone()],
        PropertyValue::List(items) => items.clone(),
        PropertyValue::Raw(raw) => vec![raw.clone()],
    };
    let mut row = div().flex().flex_wrap().gap_1();
    for item in items {
        let label = item.trim().trim_start_matches('#').to_string();
        if label.is_empty() {
            continue;
        }
        let tag = item.trim().to_string();
        let opener = opener.clone();
        row = row.child(
            div()
                .px_2()
                .py(px(1.0))
                .rounded_md()
                .bg(theme.chip)
                .text_xs()
                .text_color(theme.text_dim)
                .cursor_pointer()
                .hover(move |style| style.bg(theme.hover).text_color(theme.text))
                .child(item)
                .on_mouse_down(MouseButton::Left, move |_event, window, cx| {
                    if let Some(opener) = &opener {
                        // The app turns `tag:` into a search for that tag.
                        opener(&format!("tag:{tag}"), window, cx);
                    }
                }),
        );
    }
    row.into_any_element()
}

fn metadata_value(
    value: &PropertyValue,
    theme: Theme,
    opener: &Option<LinkOpener>,
    reading: bool,
) -> AnyElement {
    match value {
        PropertyValue::Scalar(text) => metadata_inline(text, theme, opener, 13.0),
        PropertyValue::List(items) if reading => {
            // In reading mode keep lists on one wrapped line instead of one row per item,
            // so a long bibliography reference list cannot push the note off screen.
            metadata_inline(&items.join(", "), theme, opener, 13.0)
        }
        PropertyValue::List(items) => {
            let mut row = div().flex().flex_wrap().gap_2();
            for item in items {
                row = row.child(metadata_inline(item, theme, opener, 13.0));
            }
            row.into_any_element()
        }
        PropertyValue::Raw(raw) => metadata_raw(raw, theme, opener),
    }
}

/// A nested YAML block (bibliography, acknowledgements, …): one row per line.
fn metadata_raw(raw: &str, theme: Theme, opener: &Option<LinkOpener>) -> AnyElement {
    let mut rows = Vec::new();
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let line = line.strip_prefix("- ").unwrap_or(line);
        let element = match line.split_once(':') {
            Some((key, value)) if !key.is_empty() && !key.contains(' ') => {
                let value = value.trim().trim_matches('"');
                let rendered = if key.trim().eq_ignore_ascii_case("url")
                    && (value.starts_with("http://") || value.starts_with("https://"))
                {
                    metadata_url(value, theme, opener)
                } else {
                    metadata_inline(value, theme, opener, 12.0)
                };
                div()
                    .flex()
                    .flex_row()
                    .items_start()
                    .gap_2()
                    .child(
                        div()
                            .w(px(84.0))
                            .flex_shrink_0()
                            .text_xs()
                            .text_color(theme.muted)
                            .child(key.trim().to_string()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .text_xs()
                            .text_color(theme.text_dim)
                            .child(rendered),
                    )
                    .into_any_element()
            }
            _ => div()
                .text_xs()
                .text_color(theme.text_dim)
                .child(metadata_inline(line, theme, opener, 12.0))
                .into_any_element(),
        };
        rows.push(element);
    }

    div()
        .flex()
        .flex_col()
        .gap_1()
        .children(rows)
        .into_any_element()
}

/// A clickable URL, used for bibliography `url:` fields.
fn metadata_url(text: &str, theme: Theme, opener: &Option<LinkOpener>) -> AnyElement {
    let opener = opener.clone();
    let target = text.to_string();
    div()
        .text_color(theme.accent_text)
        .cursor_pointer()
        .child(text.to_string())
        .on_mouse_down(MouseButton::Left, move |_ev, window, cx| {
            if let Some(opener) = &opener {
                opener(&target, window, cx);
            }
        })
        .into_any_element()
}

/// Inline frontmatter text: LaTeX, emphasis, wikilinks, code and highlights.
///
/// External Markdown links are rendered as plain text: metadata only navigates within the
/// vault, and raw link syntax should never leak into the panel.
fn metadata_inline(text: &str, theme: Theme, opener: &Option<LinkOpener>, size: f32) -> AnyElement {
    let spans = inline::parse(text);
    let mut row = div().flex().flex_wrap().items_center();

    for span in spans {
        if let Some(tex) = &span.math {
            match math::typeset(
                tex,
                &MathOptions {
                    style: MathStyle::Inline,
                    color: [0.0, 0.0, 0.0, 1.0],
                },
            ) {
                Ok(scene) => {
                    row = row.child(MathElement::new(Rc::new(scene), size, theme.text));
                }
                Err(_) => {
                    row = row.child(div().text_color(theme.warn).child(format!("${tex}$")));
                }
            }
            continue;
        }

        let style = span.style;
        let mut element = div().child(span.text.clone());
        if style.bold {
            element = element.font_weight(gpui::FontWeight::BOLD);
        }
        if style.italic {
            element = element.italic();
        }
        if style.strikethrough {
            element = element.line_through();
        }
        if style.highlight {
            element = element.bg(theme.star);
        }
        if style.code {
            element = element.bg(theme.code_bg).text_color(theme.code_text);
        }

        if let Some(href) = span.href.clone() {
            if style.wiki || style.link {
                let opener = opener.clone();
                let target = href.clone();
                element = element
                    .text_color(theme.accent_text)
                    .cursor_pointer()
                    .on_mouse_down(MouseButton::Left, move |_ev, window, cx| {
                        if let Some(opener) = &opener {
                            opener(&target, window, cx);
                        }
                    });
            }
        }

        row = row.child(element);
    }

    row.into_any_element()
}

/// Width of the line-number gutter.
const GUTTER_WIDTH: f32 = 52.0;

/// The line-number gutter painted to the left of the editor text.
struct LineNumbers {
    editor: Entity<Editor>,
}

/// One gutter row: its line number, plus a fold marker on expandable headings.
struct GutterRow {
    row: usize,
    number: Rc<ShapedLine>,
    marker: Option<Rc<ShapedLine>>,
}

impl IntoElement for LineNumbers {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for LineNumbers {
    type RequestLayoutState = ();
    type PrepaintState = Vec<GutterRow>;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let editor = self.editor.read(cx);
        // Match the text's height. Using the logical line count here would leave blank
        // gutter rows under a folded section.
        let rows = if editor.last_shaped.is_empty() {
            editor.buffer.line_count()
        } else {
            editor.last_shaped.len()
        }
        .max(1);
        let mut style = Style::default();
        style.size.width = px(GUTTER_WIDTH).into();
        style.size.height = (editor.line_height * rows as f32).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _state: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Vec<GutterRow> {
        let editor = self.editor.read(cx);
        let line_height = editor.line_height;
        let metas = editor.last_rows.clone();
        if metas.is_empty() {
            return Vec::new();
        }
        let fold_markers = editor.fold_markers.clone();

        let theme = *cx.global::<Theme>();
        let mut font = window.text_style().font();
        font.family = "Cascadia Mono".into();
        let font_size = px(11.0);
        let visible = visible_row_range(
            bounds,
            line_height,
            editor.scroll_handle.bounds(),
            metas.len(),
        );

        let mut shaped = Vec::new();
        for row in visible {
            let meta = metas[row];
            if meta.start != 0 {
                continue;
            }
            let number = (meta.line + 1).to_string();
            let run = TextRun {
                len: number.len(),
                font: font.clone(),
                color: theme.muted,
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            let line = window.text_system().shape_line(
                SharedString::from(number),
                font_size,
                &[run],
                None,
            );

            let marker = fold_markers
                .get(meta.line)
                .copied()
                .filter(|marker| *marker != 0)
                .map(|marker| {
                    let text = if marker == 2 { "\u{25B8}" } else { "\u{25BE}" };
                    let run = TextRun {
                        len: text.len(),
                        font: font.clone(),
                        color: theme.muted,
                        background_color: None,
                        underline: None,
                        strikethrough: None,
                    };
                    Rc::new(window.text_system().shape_line(
                        SharedString::from(text),
                        font_size,
                        &[run],
                        None,
                    ))
                });

            shaped.push(GutterRow {
                row,
                number: Rc::new(line),
                marker,
            });
        }
        shaped
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _state: &mut (),
        lines: &mut Vec<GutterRow>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let line_height = self.editor.read(cx).line_height;
        for GutterRow {
            row,
            number,
            marker,
        } in lines.drain(..)
        {
            if let Some(marker) = marker {
                let _ = marker.paint(
                    point(
                        bounds.left() + px(8.0),
                        bounds.top() + line_height * row as f32,
                    ),
                    line_height,
                    window,
                    cx,
                );
            }
            let origin = point(
                bounds.right() - number.width - px(8.0),
                bounds.top() + line_height * row as f32,
            );
            let _ = number.paint(origin, line_height, window, cx);
        }
    }
}

struct EditorText {
    editor: Entity<Editor>,
}

/// Realign a per-line cache with a document of `count` lines after whole lines were
/// inserted or removed.
///
/// `same(index, cached)` reports whether the cached layout still describes the document's
/// line at `index`. The cache is only shifted when every line after the first mismatch
/// matches at the shifted index; otherwise it is returned untouched and the caller
/// re-shapes the lines that differ.
fn realign_cache<T>(
    mut cache: Vec<Option<T>>,
    count: usize,
    same: impl Fn(usize, &T) -> bool,
) -> Vec<Option<T>> {
    let old_len = cache.len();
    if old_len == count {
        return cache;
    }
    let delta = count as isize - old_len as isize;

    // The first index where the cache and the document disagree.
    let Some(start) = (0..old_len.min(count)).find(|&index| match &cache[index] {
        Some(cached) => !same(index, cached),
        None => true,
    }) else {
        // Only trailing lines were added or removed; the caller handles those.
        return cache;
    };

    let aligns = (start..count).all(|index| {
        let cached_index = index as isize - delta;
        if cached_index < start as isize || cached_index >= old_len as isize {
            return true; // A genuinely new line.
        }
        match &cache[cached_index as usize] {
            Some(cached) => same(index, cached),
            None => true,
        }
    });
    if !aligns {
        return cache;
    }

    let mut tail = cache.split_off(start);
    if delta < 0 {
        tail.drain(..(-delta) as usize);
    }
    if delta > 0 {
        cache.extend((0..delta).map(|_| None));
    }
    cache.append(&mut tail);
    cache
}

impl IntoElement for EditorText {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

struct Prepaint {
    rows: Rc<Vec<Rc<ShapedLine>>>,
    metas: Rc<Vec<RowMeta>>,
    selection: Vec<PaintQuad>,
    find_matches: Vec<PaintQuad>,
    current_line: Option<PaintQuad>,
    cursor: Option<PaintQuad>,
    extra_cursors: Vec<PaintQuad>,
    /// Rows intersecting the viewport; only these are painted.
    visible: Range<usize>,
    /// Source lines whose text is replaced by a rendered formula.
    hidden_lines: HashSet<usize>,
}

impl Element for EditorText {
    type RequestLayoutState = ();
    type PrepaintState = Prepaint;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let editor = self.editor.read(cx);
        let mut style = Style::default();
        style.size.width = relative(1.0).into();
        // Before the first paint only the logical line count is known; afterwards the row
        // count is exact, so the height settles after one frame.
        let rows = if editor.last_rows.is_empty() {
            editor.buffer.line_count()
        } else {
            editor.last_rows.len()
        };
        style.size.height = (editor.line_height * rows.max(1) as f32).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Prepaint {
        let theme = *cx.global::<Theme>();
        let color = window.text_style().color;
        let available = f32::from(bounds.size.width).max(1.0);

        // Shape only what changed since the last frame. This runs through `update`
        // because the cache lives on the editor, which the element only borrows.
        let editor_entity = self.editor.clone();
        editor_entity.update(cx, |editor, _cx| {
            editor.lay_out(window, theme, available, color);
            let caret_line = editor.buffer.line_col().0;
            editor.ensure_math_scenes(caret_line);
        });

        let editor = self.editor.read(cx);
        let line_height = editor.line_height;
        let rows = editor.last_shaped.clone();
        let metas = editor.last_rows.clone();

        // A block renders only while the caret is outside it, so editing a formula always
        // shows its source. Every line of a rendered block loses its glyphs; the rows keep
        // their height, which is what keeps scroll, click and caret geometry unchanged.
        let caret_line = editor.buffer.line_col().0;
        let mut hidden_lines: HashSet<usize> = HashSet::new();
        for block in editor.display_math.iter() {
            if block.row_count == 0 {
                continue;
            }
            if caret_line >= block.first_line && caret_line <= block.last_line {
                continue;
            }
            // Nothing is hidden unless there is a formula to put in its place, so a
            // formula the typesetter rejects stays visible as source instead of leaving
            // a blank gap.
            if !editor
                .math_scenes
                .get(&block.tex)
                .is_some_and(|scene| scene.is_some())
            {
                continue;
            }
            hidden_lines.extend(block.first_line..=block.last_line);
        }

        // Only the rows inside the scroll viewport need quads and glyphs. On a long note
        // this turns each scroll frame from "paint the whole document" into "paint a
        // screenful", which is where most of the scrolling cost came from.
        let visible = visible_row_range(
            bounds,
            line_height,
            editor.scroll_handle.bounds(),
            metas.len(),
        );

        let row_byte = |line: usize, row_start: usize, col: usize| -> usize {
            editor.buffer.char_col_to_byte(line, col)
                - editor.buffer.char_col_to_byte(line, row_start)
        };

        let mut selection_quads = Vec::new();
        for selection in editor.selection_ranges() {
            for row_index in visible.clone() {
                let meta = metas[row_index];
                if hidden_lines.contains(&meta.line) {
                    continue;
                }
                let line_start = editor.buffer.char_index_at(meta.line, 0);
                let start = selection.start.max(line_start + meta.start);
                let end = selection.end.min(line_start + meta.end);
                if start >= end {
                    continue;
                }

                let x0 = rows[row_index].x_for_index(row_byte(
                    meta.line,
                    meta.start,
                    start - line_start,
                ));
                let x1 =
                    rows[row_index].x_for_index(row_byte(meta.line, meta.start, end - line_start));
                let top = bounds.top() + line_height * row_index as f32;
                selection_quads.push(fill(
                    Bounds::from_corners(
                        point(bounds.left() + x0, top),
                        point(bounds.left() + x1, top + line_height),
                    ),
                    theme.selection,
                ));
            }
        }

        let mut find_quads = Vec::new();
        for (index, matched) in editor.find.matches.iter().enumerate() {
            let color = if index == editor.find.current {
                theme.accent.opacity(0.45)
            } else {
                theme.accent.opacity(0.20)
            };
            for row_index in visible.clone() {
                let meta = metas[row_index];
                if hidden_lines.contains(&meta.line) {
                    continue;
                }
                let line_start = editor.buffer.char_index_at(meta.line, 0);
                let start = matched.start.max(line_start + meta.start);
                let end = matched.end.min(line_start + meta.end);
                if start >= end {
                    continue;
                }
                let x0 = rows[row_index].x_for_index(row_byte(
                    meta.line,
                    meta.start,
                    start - line_start,
                ));
                let x1 =
                    rows[row_index].x_for_index(row_byte(meta.line, meta.start, end - line_start));
                let top = bounds.top() + line_height * row_index as f32;
                find_quads.push(fill(
                    Bounds::from_corners(
                        point(bounds.left() + x0, top),
                        point(bounds.left() + x1, top + line_height),
                    ),
                    color,
                ));
            }
        }

        let (line_idx, col) = editor.buffer.line_col();
        let mut cursor_row = None;
        for row_index in visible.clone() {
            let meta = metas[row_index];
            if meta.line == line_idx && meta.start <= col {
                cursor_row = Some(row_index);
            }
        }
        let cursor = cursor_row.map(|row_index| {
            let meta = metas[row_index];
            let col = col.min(meta.end);
            let x = rows[row_index].x_for_index(row_byte(meta.line, meta.start, col));
            fill(
                Bounds::new(
                    point(
                        bounds.left() + x,
                        bounds.top() + line_height * row_index as f32,
                    ),
                    size(px(2.0), line_height),
                ),
                color,
            )
        });

        let mut extra_cursor_quads = Vec::new();
        for state in &editor.extra_cursors {
            let (state_line, state_col) = editor.buffer.line_col_at(state.cursor);
            let Some(row_index) = visible.clone().find(|row_index| {
                let meta = metas[*row_index];
                meta.line == state_line && meta.start <= state_col && state_col <= meta.end
            }) else {
                continue;
            };
            let meta = metas[row_index];
            let col = state_col.min(meta.end);
            let x = rows[row_index].x_for_index(row_byte(meta.line, meta.start, col));
            extra_cursor_quads.push(fill(
                Bounds::new(
                    point(
                        bounds.left() + x,
                        bounds.top() + line_height * row_index as f32,
                    ),
                    size(px(2.0), line_height),
                ),
                theme.accent,
            ));
        }

        let current_line = cursor_row.map(|row_index| {
            fill(
                Bounds::new(
                    point(bounds.left(), bounds.top() + line_height * row_index as f32),
                    size(bounds.size.width, line_height),
                ),
                theme.current_line,
            )
        });

        Prepaint {
            rows,
            metas,
            selection: selection_quads,
            find_matches: find_quads,
            current_line,
            cursor,
            extra_cursors: extra_cursor_quads,
            visible,
            hidden_lines,
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut (),
        prepaint: &mut Prepaint,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus_handle = self.editor.read(cx).focus_handle.clone();
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.editor.clone()),
            cx,
        );

        let line_height = self.editor.read(cx).line_height;

        if let Some(quad) = prepaint.current_line.take() {
            window.paint_quad(quad);
        }

        for quad in prepaint.find_matches.drain(..) {
            window.paint_quad(quad);
        }

        for quad in prepaint.selection.drain(..) {
            window.paint_quad(quad);
        }

        let visible = prepaint.visible.clone();
        let rows = prepaint.rows.clone();
        let metas = prepaint.metas.clone();
        for i in visible.clone() {
            let Some(row) = rows.get(i) else {
                continue;
            };
            if metas
                .get(i)
                .is_some_and(|meta| prepaint.hidden_lines.contains(&meta.line))
            {
                continue;
            }
            let origin = point(bounds.left(), bounds.top() + line_height * i as f32);
            let _ = row.paint(origin, line_height, window, cx);
        }

        // Draw each rendered `$$…$$` block over the blank rows it owns.
        {
            let editor = self.editor.read(cx);
            let theme = *cx.global::<Theme>();
            let column_width = f32::from(bounds.size.width);
            for block in editor.display_math.iter() {
                if block.row_count == 0 || prepaint.hidden_lines.contains(&block.first_line) {
                    continue;
                }
                let first_row = block.first_row;
                let last_row = first_row + block.row_count;
                if last_row <= visible.start || first_row >= visible.end {
                    continue;
                }
                let Some(Some(scene)) = editor.math_scenes.get(&block.tex) else {
                    continue;
                };

                // Display size, scaled down when the formula is wider than the text column.
                let natural = 18.0f32;
                let scene_width = scene.width() as f32;
                let font_size = if scene_width > 0.0 && scene_width * natural > column_width - 24.0
                {
                    ((column_width - 24.0) / scene_width).clamp(9.0, natural)
                } else {
                    natural
                };
                let width = px(scene_width * font_size);
                let height = px(scene.total_height() as f32 * font_size);
                let block_height = line_height * block.row_count as f32;
                let left = bounds.left() + px(((column_width - f32::from(width)) / 2.0).max(0.0));
                let top = bounds.top()
                    + line_height * first_row as f32
                    + px(((f32::from(block_height) - f32::from(height)) / 2.0).max(0.0));
                paint_scene(window, scene, font_size, theme.text, point(left, top));
            }
        }

        if focus_handle.is_focused(window) {
            if let Some(cursor) = prepaint.cursor.take() {
                window.paint_quad(cursor);
            }
            for cursor in prepaint.extra_cursors.drain(..) {
                window.paint_quad(cursor);
            }
        }

        let rows = prepaint.rows.clone();
        let metas = prepaint.metas.clone();

        self.editor.update(cx, |editor, _cx| {
            editor.last_bounds = Some(bounds);
            editor.last_rows = metas;
            editor.last_shaped = rows;
        });
    }
}

/// The vertical scroll delta that brings `[top, bottom)` back inside the viewport.
///
/// Positive moves content down (the caret was above the viewport), negative moves it up.
/// Zero means the range is already visible, so scrolling stays put.
fn scroll_delta(
    top: Pixels,
    bottom: Pixels,
    viewport_top: Pixels,
    viewport_bottom: Pixels,
) -> Pixels {
    if top < viewport_top {
        viewport_top - top
    } else if bottom > viewport_bottom {
        viewport_bottom - bottom
    } else {
        px(0.0)
    }
}

/// The slice of rows that intersects the scroll viewport, with a little overdraw so
/// scrolling stays smooth.
fn visible_row_range(
    bounds: Bounds<Pixels>,
    line_height: Pixels,
    viewport: Bounds<Pixels>,
    row_count: usize,
) -> Range<usize> {
    if row_count == 0 {
        return 0..0;
    }

    let height = f32::from(viewport.size.height);
    if height <= 0.0 {
        // No viewport measured yet (first frame): paint everything once.
        return 0..row_count;
    }

    let line_height = f32::from(line_height).max(1.0);
    let top = (f32::from(viewport.top() - bounds.top()) / line_height).floor() as isize;
    let bottom = (f32::from(viewport.bottom() - bounds.top()) / line_height).ceil() as isize;

    let start = (top - 1).max(0) as usize;
    let end = (bottom + 2).max(0) as usize;
    start.min(row_count)..end.min(row_count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{point, size};

    #[test]
    fn lists_continue_on_enter() {
        assert_eq!(list_continuation("- item", 6).as_deref(), Some("- "));
        assert_eq!(list_continuation("  - item", 8).as_deref(), Some("  - "));
        assert_eq!(
            list_continuation("- [x] done", 10).as_deref(),
            Some("- [ ] ")
        );
        assert_eq!(list_continuation("3. three", 8).as_deref(), Some("4. "));
        assert_eq!(list_continuation("1) one", 6).as_deref(), Some("2) "));
        assert_eq!(list_continuation("> quote", 7).as_deref(), Some("> "));
    }

    #[test]
    fn list_continuation_stops_where_it_should() {
        // Plain text, an empty item, and a caret that is not at the end.
        assert_eq!(list_continuation("plain text", 10), None);
        assert_eq!(list_continuation("- ", 2), None);
        assert_eq!(list_continuation("- item", 2), None);
        assert_eq!(list_continuation("# heading", 9), None);
    }

    #[test]
    fn display_math_blocks_span_their_lines() {
        let blocks = display_math_blocks("before\n$$\na + b\n$$\nafter");
        assert_eq!(
            blocks,
            vec![DisplayMathSource {
                first_line: 1,
                last_line: 3,
                tex: "a + b".to_string(),
            }]
        );
    }

    #[test]
    fn a_single_line_display_block_counts() {
        let blocks = display_math_blocks("x\n$$e^{i\\pi}$$\n");
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].first_line, 1);
        assert_eq!(blocks[0].last_line, 1);
        assert_eq!(blocks[0].tex, "e^{i\\pi}");
    }

    #[test]
    fn inline_math_is_not_a_block() {
        assert!(display_math_blocks("a $x$ b\n$$\n").is_empty());
        assert!(display_math_blocks("price $$ 5").is_empty());
    }

    #[test]
    fn fenced_code_hides_its_dollars() {
        let text = "```\n$$\nnot math\n$$\n```\n$$\ny = x\n$$";
        let blocks = display_math_blocks(text);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].first_line, 5);
        assert_eq!(blocks[0].tex, "y = x");
    }

    #[test]
    fn an_unclosed_block_stays_source() {
        assert!(display_math_blocks("$$\nx + y\n").is_empty());
    }

    #[test]
    fn a_selection_becomes_the_wikilink_alias() {
        assert_eq!(
            link_text("notes/Other", "Other", Some("My Note")),
            "[[notes/Other|My Note]]"
        );
        assert_eq!(link_text("notes/Other", "Other", None), "[[notes/Other]]");
        assert_eq!(
            link_text("notes/Other", "Other", Some("")),
            "[[notes/Other]]"
        );
        assert_eq!(
            link_text("notes/Other", "Other", Some("  other  ")),
            "[[notes/Other]]"
        );
        assert_eq!(
            link_text("notes/Other", "Other", Some("  My Note  ")),
            "[[notes/Other|My Note]]"
        );
    }

    #[test]
    fn aliases_are_flattened_for_wikilinks() {
        assert_eq!(clean_link_alias("My\nNote"), "My Note");
        assert_eq!(clean_link_alias("a | b [c]"), "a - b c");
        assert_eq!(clean_link_alias("   "), "");
    }

    #[test]
    fn selected_text_becomes_a_wikilink() {
        assert_eq!(
            wikilink_for_selection("My Note").as_deref(),
            Some("[[My Note]]")
        );
        assert_eq!(
            wikilink_for_selection("  My\nNote  ").as_deref(),
            Some("[[My Note]]")
        );
        assert_eq!(wikilink_for_selection("   "), None);
    }

    #[test]
    fn formatting_round_trips_through_the_buffer() {
        // The same edits the toolbar and shortcuts apply, exercised on a plain string.
        let text = "hello";
        let bold =
            dragonglass_core::format::toggle(text, 0..5, dragonglass_core::format::Style::Bold);
        assert_eq!(bold.text, "**hello**");
        assert_eq!(bold.selection, 2..7);
        let again = dragonglass_core::format::toggle(
            &bold.text,
            bold.selection.clone(),
            dragonglass_core::format::Style::Bold,
        );
        assert_eq!(again.text, "hello");
    }

    #[test]
    fn literal_after_finds_multibyte_occurrences_by_character() {
        let text = "héllo héllo";
        assert_eq!(literal_after(text, "héllo", 0), Some(0));
        assert_eq!(literal_after(text, "héllo", 1), Some(6));
        assert_eq!(literal_after(text, "héllo", 7), None);
        assert_eq!(literal_after(text, "é", 2), Some(7));
    }

    #[test]
    fn literal_after_rejects_an_empty_needle() {
        assert_eq!(literal_after("abc", "", 0), None);
    }

    #[test]
    fn visible_rows_at_the_top_include_only_the_first_screen() {
        let bounds = Bounds::new(point(px(0.0), px(0.0)), size(px(800.0), px(2000.0)));
        let viewport = Bounds::new(point(px(0.0), px(0.0)), size(px(800.0), px(400.0)));

        let visible = visible_row_range(bounds, px(20.0), viewport, 100);

        assert_eq!(visible.start, 0);
        assert_eq!(visible.end, 22);
    }

    #[test]
    fn visible_rows_follow_the_scroll_offset() {
        // The content is scrolled up by 1000px, so its origin is 1000px above the viewport.
        let bounds = Bounds::new(point(px(0.0), px(-1000.0)), size(px(800.0), px(2000.0)));
        let viewport = Bounds::new(point(px(0.0), px(0.0)), size(px(800.0), px(400.0)));

        let visible = visible_row_range(bounds, px(20.0), viewport, 100);

        // Rows 50..70 are on screen; one row of overdraw on each side.
        assert_eq!(visible.start, 49);
        assert_eq!(visible.end, 72);
    }

    #[test]
    fn visible_rows_clamp_to_the_row_count() {
        let bounds = Bounds::new(point(px(0.0), px(0.0)), size(px(800.0), px(100.0)));
        let viewport = Bounds::new(point(px(0.0), px(0.0)), size(px(800.0), px(400.0)));

        let visible = visible_row_range(bounds, px(20.0), viewport, 3);

        assert_eq!(visible, 0..3);
    }

    #[test]
    fn empty_viewport_paints_everything_before_the_first_frame() {
        let bounds = Bounds::new(point(px(0.0), px(0.0)), size(px(800.0), px(2000.0)));
        let viewport = Bounds::new(point(px(0.0), px(0.0)), size(px(0.0), px(0.0)));

        assert_eq!(visible_row_range(bounds, px(20.0), viewport, 100), 0..100);
    }

    /// A cache of line texts, with `None` for a line that has not been shaped yet.
    fn cache(texts: &[&str]) -> Vec<Option<&'static str>> {
        texts
            .iter()
            .map(|text| Some(Box::leak(text.to_string().into_boxed_str()) as &'static str))
            .collect()
    }

    fn realign(
        cached: Vec<Option<&'static str>>,
        document: &[&'static str],
    ) -> Vec<Option<&'static str>> {
        realign_cache(cached, document.len(), |index, cached| {
            document.get(index) == Some(cached)
        })
    }

    #[test]
    fn inserted_line_shifts_the_cache_instead_of_dropping_it() {
        let aligned = realign(cache(&["a", "b", "c", "d"]), &["a", "x", "b", "c", "d"]);
        assert_eq!(
            aligned,
            vec![Some("a"), None, Some("b"), Some("c"), Some("d")]
        );
    }

    #[test]
    fn removed_line_shifts_the_cache_instead_of_dropping_it() {
        let aligned = realign(cache(&["a", "x", "b", "c"]), &["a", "b", "c"]);
        assert_eq!(aligned, vec![Some("a"), Some("b"), Some("c")]);
    }

    #[test]
    fn appended_lines_keep_the_whole_cache() {
        let aligned = realign(cache(&["a", "b"]), &["a", "b", "c"]);
        assert_eq!(aligned, vec![Some("a"), Some("b")]);
    }

    #[test]
    fn an_unrelated_change_leaves_the_cache_alone() {
        let cached = cache(&["a", "b", "c"]);
        let aligned = realign(cached.clone(), &["a", "B", "z", "c"]);
        assert_eq!(aligned, cached);
    }

    #[test]
    fn a_changed_line_and_an_insertion_is_not_mistaken_for_a_shift() {
        let cached = cache(&["a", "b", "c", "d"]);
        let aligned = realign(cached.clone(), &["a", "x", "B", "c", "d"]);
        assert_eq!(aligned, cached);
    }

    #[test]
    fn scroll_delta_follows_the_caret_only_when_it_leaves_the_viewport() {
        let visible = scroll_delta(px(120.0), px(140.0), px(100.0), px(500.0));
        assert_eq!(f32::from(visible), 0.0);

        // Caret above the viewport: move the content down.
        let above = scroll_delta(px(60.0), px(80.0), px(100.0), px(500.0));
        assert_eq!(f32::from(above), 40.0);

        // Caret below the viewport: move the content up, just enough to show it.
        let below = scroll_delta(px(520.0), px(540.0), px(100.0), px(500.0));
        assert_eq!(f32::from(below), -40.0);
    }

    #[test]
    fn setext_underlines_are_recognised() {
        assert!(Editor::is_setext_underline("==="));
        assert!(Editor::is_setext_underline("  ---  "));
        assert!(!Editor::is_setext_underline("- item"));
        assert!(!Editor::is_setext_underline(""));
    }

    fn heading(level: u8, line: usize) -> markdown::Heading {
        markdown::Heading {
            level,
            text: format!("h{line}"),
            line,
        }
    }

    #[test]
    fn a_folded_heading_hides_its_section_but_not_the_next_sibling() {
        let headings = vec![heading(1, 1), heading(2, 2), heading(3, 5), heading(2, 9)];
        let mut folded = HashSet::new();
        folded.insert(1);

        let hidden = Editor::hidden_lines(&headings, &folded, 12);

        // The level-2 heading on line 2 hides lines 3..=8; the next level-2 heading on
        // line 9 and everything after it stay visible.
        assert_eq!(&hidden[..2], &[false, false]);
        assert!(hidden[2..8].iter().all(|line| *line));
        assert!(hidden[8..].iter().all(|line| !*line));
    }

    #[test]
    fn folding_the_top_heading_hides_the_rest_of_the_document() {
        let headings = vec![heading(1, 1), heading(2, 2), heading(3, 4)];
        let mut folded = HashSet::new();
        folded.insert(0);

        let hidden = Editor::hidden_lines(&headings, &folded, 7);

        assert!(!hidden[0]);
        assert!(hidden[1..].iter().all(|line| *line));
    }

    #[test]
    fn gutter_markers_show_open_and_folded_headings_only() {
        let headings = vec![heading(1, 1), heading(2, 3), heading(3, 4), heading(2, 6)];
        let mut folded = HashSet::new();
        folded.insert(0);

        let markers = Editor::fold_markers(&headings, &folded, 8);

        assert_eq!(markers[0], 2);
        // A heading with a nested heading gets an open marker; the last level-2
        // heading has no children and gets none.
        assert_eq!(markers[2], 1);
        assert_eq!(markers[5], 0);
    }
}
