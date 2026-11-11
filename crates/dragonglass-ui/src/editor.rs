//! A multi-line text editor view, built from scratch on GPUI.
//!
//! GPUI ships no multi-line editor, so this is a hand-rolled `Element` that shapes each
//! line of a `dragonglass_core::Buffer`, paints a caret and selection, and routes keyboard
//! input and platform IME text through `EntityInputHandler`.
//!
//! Still missing (tracked): word wrap, horizontal scroll, multi-cursor.

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use dragonglass_core::frontmatter::{self, Property, PropertyValue};
use dragonglass_core::inline;
use dragonglass_core::math::{self, MathOptions, MathStyle};
use dragonglass_core::{blocks, git, history, stats, storage, syntax, templates, wrap, Buffer};
use gpui::{
    actions, div, fill, point, prelude::*, px, relative, size, AnyElement, App, Bounds,
    ClipboardEntry, ClipboardItem, Context, CursorStyle, ElementId, ElementInputHandler, Entity,
    EntityInputHandler, ExternalPaths, FocusHandle, Focusable, GlobalElementId, ImageFormat,
    InspectorElementId, KeyBinding, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, PaintQuad, Pixels, Point, Render, ScrollHandle, ShapedLine, SharedString, Style,
    TextRun, UTF16Selection, Window,
};

use crate::math_element::MathElement;
use crate::reading::{LinkOpener, ReadingCache};
use crate::theme::Theme;

/// How many leading lines are inspected for YAML frontmatter. Real frontmatter is a few
/// lines; the cap keeps property refresh from copying a large note.
const FRONTMATTER_LINE_WINDOW: usize = 256;

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
        MoveBlockDown
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
        KeyBinding::new("enter", Enter, None),
        KeyBinding::new("ctrl-v", Paste, None),
        KeyBinding::new("cmd-v", Paste, None),
        KeyBinding::new("ctrl-s", Save, None),
        KeyBinding::new("cmd-s", Save, None),
        KeyBinding::new("ctrl-e", ToggleReading, None),
        KeyBinding::new("cmd-e", ToggleReading, None),
        KeyBinding::new("ctrl-shift-up", MoveBlockUp, None),
        KeyBinding::new("ctrl-shift-down", MoveBlockDown, None),
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
    /// True while a splitter is being dragged: the text keeps its old wrap width so a
    /// resize does not re-shape the whole document on every mouse tick.
    resizing: bool,
    /// Opens a link clicked in reading mode.
    link_opener: Option<LinkOpener>,
    /// Cached document statistics, recomputed per buffer revision.
    stats_cache: (u64, stats::Stats),
    /// Git added/removed lines for the file, refreshed when the saved buffer changes.
    git_stat: Option<git::DiffStat>,
    git_stat_key: Option<(u64, bool)>,
}

/// A wrapped visual row: `[start, end)` character columns of logical line `line`.
#[derive(Debug, Clone, Copy)]
struct RowMeta {
    line: usize,
    start: usize,
    end: usize,
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
            resizing: false,
            attachments_folder: "attachments".to_string(),
            link_opener: None,
            stats_cache: (u64::MAX, stats::Stats::default()),
            git_stat: None,
            git_stat_key: None,
        }
    }

    /// Load a note from disk, replacing the buffer.
    pub fn load(&mut self, path: &Path) -> std::io::Result<()> {
        let text = std::fs::read_to_string(path)?;
        self.buffer = Buffer::from_text(&text);
        self.buffer.mark_saved();
        self.marked_range = None;
        self.anchor = None;
        self.selecting = false;
        self.reset_layout_cache();
        self.scroll_handle = ScrollHandle::new();
        self.properties_revision = u64::MAX;
        self.doc_generation = self.doc_generation.wrapping_add(1);
        self.path = Some(path.to_path_buf());
        Ok(())
    }

    /// Write the buffer back to its file, atomically.
    pub fn save(&mut self) -> std::io::Result<()> {
        let Some(path) = self.path.clone() else {
            return Ok(());
        };

        let new_content = self.buffer.text();

        // Keep a snapshot of whatever is about to be overwritten.
        if let (Some(root), Ok(existing)) =
            (self.vault_root.as_ref(), std::fs::read_to_string(&path))
        {
            if existing != new_content {
                if let Ok(relative) = path.strip_prefix(root) {
                    let relative = relative.to_string_lossy().replace('\\', "/");
                    let _ = history::record(root, &relative, &existing);
                }
            }
        }

        storage::write_atomic(&path, &new_content)?;
        self.buffer.mark_saved();
        Ok(())
    }

    pub fn is_dirty(&self) -> bool {
        self.buffer.is_dirty()
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
            return Ok(());
        }
        self.buffer = Buffer::from_text(&text);
        self.buffer.mark_saved();
        self.anchor = None;
        self.marked_range = None;
        self.reset_layout_cache();
        self.scroll_handle = ScrollHandle::new();
        self.properties_revision = u64::MAX;
        self.doc_generation = self.doc_generation.wrapping_add(1);
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
    }

    /// Whether the editor is currently in reading mode.
    pub fn is_reading(&self) -> bool {
        self.reading
    }

    /// Set reading mode explicitly (used when restoring a workspace).
    pub fn set_reading_mode(&mut self, reading: bool) {
        self.reading = reading;
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
    }

    /// Force the shaped-line cache to rebuild on the next frame, keeping hit-testing
    /// geometry valid until then. Called when the theme changes.
    pub fn invalidate_layout(&mut self) {
        self.line_cache.clear();
        self.cache_revision = u64::MAX;
    }

    /// Freeze text wrapping while a splitter drag changes the editor width, then reflow
    /// once when it ends. Re-shaping the whole note on every mouse tick made resizing crawl.
    pub fn set_resizing(&mut self, resizing: bool) {
        if self.resizing == resizing {
            return;
        }
        self.resizing = resizing;
        if !resizing {
            self.cache_width = 0.0;
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
        if !width_changed && self.cache_revision == revision && !self.last_shaped.is_empty() {
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

        let mut in_code = false;
        let mut language = String::new();
        let mut rows: Vec<Rc<ShapedLine>> = Vec::new();
        let mut metas: Vec<RowMeta> = Vec::new();

        for line_index in 0..count {
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

            for span in layout.spans.iter() {
                metas.push(RowMeta {
                    line: line_index,
                    start: span.0,
                    end: span.1,
                });
            }
            rows.extend(layout.rows.iter().cloned());

            if line_index < self.line_cache.len() {
                self.line_cache[line_index] = Some(layout);
            } else {
                self.line_cache.push(Some(layout));
            }
        }
        self.line_cache.truncate(count);

        self.last_rows = Rc::new(metas);
        self.last_shaped = Rc::new(rows);
        self.cache_revision = revision;
        self.cache_width = width;
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
        if self.anchor.is_none() {
            self.anchor = Some(self.buffer.cursor());
        }
    }

    fn collapse_selection(&mut self) {
        self.anchor = None;
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
        if let Some(selection) = self.selection() {
            self.buffer.delete_range(selection);
            self.anchor = None;
            true
        } else {
            false
        }
    }

    fn insert_text(&mut self, text: &str) {
        if let Some(selection) = self.selection() {
            self.buffer.replace(selection, text);
        } else {
            self.buffer.insert(text);
        }
        self.anchor = None;
    }

    fn backspace(&mut self, _: &Backspace, _window: &mut Window, cx: &mut Context<Self>) {
        if !self.delete_selection() {
            self.buffer.backspace();
        }
        cx.notify();
    }

    fn delete(&mut self, _: &Delete, _window: &mut Window, cx: &mut Context<Self>) {
        if !self.delete_selection() {
            self.buffer.delete_forward();
        }
        cx.notify();
    }

    fn left(&mut self, _: &Left, _window: &mut Window, cx: &mut Context<Self>) {
        self.collapse_selection();
        self.buffer.move_left();
        cx.notify();
    }

    fn right(&mut self, _: &Right, _window: &mut Window, cx: &mut Context<Self>) {
        self.collapse_selection();
        self.buffer.move_right();
        cx.notify();
    }

    fn up(&mut self, _: &Up, _window: &mut Window, cx: &mut Context<Self>) {
        self.collapse_selection();
        self.move_vertical(-1);
        cx.notify();
    }

    fn down(&mut self, _: &Down, _window: &mut Window, cx: &mut Context<Self>) {
        self.collapse_selection();
        self.move_vertical(1);
        cx.notify();
    }

    fn home(&mut self, _: &Home, _window: &mut Window, cx: &mut Context<Self>) {
        self.collapse_selection();
        self.buffer.move_home();
        cx.notify();
    }

    fn end(&mut self, _: &End, _window: &mut Window, cx: &mut Context<Self>) {
        self.collapse_selection();
        self.buffer.move_end();
        cx.notify();
    }

    fn word_left(&mut self, _: &WordLeft, _window: &mut Window, cx: &mut Context<Self>) {
        self.collapse_selection();
        self.buffer.move_word_left();
        cx.notify();
    }

    fn word_right(&mut self, _: &WordRight, _window: &mut Window, cx: &mut Context<Self>) {
        self.collapse_selection();
        self.buffer.move_word_right();
        cx.notify();
    }

    fn paragraph_up(&mut self, _: &ParagraphUp, _window: &mut Window, cx: &mut Context<Self>) {
        self.collapse_selection();
        self.buffer.move_paragraph_up();
        cx.notify();
    }

    fn paragraph_down(&mut self, _: &ParagraphDown, _window: &mut Window, cx: &mut Context<Self>) {
        self.collapse_selection();
        self.buffer.move_paragraph_down();
        cx.notify();
    }

    fn document_start(&mut self, _: &DocumentStart, _window: &mut Window, cx: &mut Context<Self>) {
        self.collapse_selection();
        self.buffer.move_document_start();
        cx.notify();
    }

    fn document_end(&mut self, _: &DocumentEnd, _window: &mut Window, cx: &mut Context<Self>) {
        self.collapse_selection();
        self.buffer.move_document_end();
        cx.notify();
    }

    fn delete_word_left(
        &mut self,
        _: &DeleteWordLeft,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.delete_selection() {
            self.buffer.delete_word_left();
        }
        cx.notify();
    }

    fn delete_word_right(
        &mut self,
        _: &DeleteWordRight,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.delete_selection() {
            self.buffer.delete_word_right();
        }
        cx.notify();
    }

    fn select_left(&mut self, _: &SelectLeft, _window: &mut Window, cx: &mut Context<Self>) {
        self.begin_selection();
        self.buffer.move_left();
        cx.notify();
    }

    fn select_right(&mut self, _: &SelectRight, _window: &mut Window, cx: &mut Context<Self>) {
        self.begin_selection();
        self.buffer.move_right();
        cx.notify();
    }

    fn select_up(&mut self, _: &SelectUp, _window: &mut Window, cx: &mut Context<Self>) {
        self.begin_selection();
        self.move_vertical(-1);
        cx.notify();
    }

    fn select_down(&mut self, _: &SelectDown, _window: &mut Window, cx: &mut Context<Self>) {
        self.begin_selection();
        self.move_vertical(1);
        cx.notify();
    }

    fn select_home(&mut self, _: &SelectHome, _window: &mut Window, cx: &mut Context<Self>) {
        self.begin_selection();
        self.buffer.move_home();
        cx.notify();
    }

    fn select_end(&mut self, _: &SelectEnd, _window: &mut Window, cx: &mut Context<Self>) {
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
        self.buffer.set_cursor(self.buffer.len_chars());
        cx.notify();
    }

    fn copy(&mut self, _: &Copy, _window: &mut Window, cx: &mut Context<Self>) {
        if self.reading {
            if let Some(text) = self.reading_content.selected_text() {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
            }
            return;
        }
        if let Some(selection) = self.selection() {
            cx.write_to_clipboard(ClipboardItem::new_string(self.buffer.slice(selection)));
        }
    }

    fn cut(&mut self, _: &Cut, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(selection) = self.selection() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.buffer.slice(selection.clone()),
            ));
            self.buffer.delete_range(selection);
            self.anchor = None;
            cx.notify();
        }
    }

    fn undo(&mut self, _: &Undo, _window: &mut Window, cx: &mut Context<Self>) {
        if self.buffer.undo() {
            self.anchor = None;
            self.marked_range = None;
            cx.notify();
        }
    }

    fn redo(&mut self, _: &Redo, _window: &mut Window, cx: &mut Context<Self>) {
        if self.buffer.redo() {
            self.anchor = None;
            self.marked_range = None;
            cx.notify();
        }
    }

    fn enter(&mut self, _: &Enter, _window: &mut Window, cx: &mut Context<Self>) {
        self.delete_selection();
        self.buffer.insert("\n");
        cx.notify();
    }

    fn paste(&mut self, _: &Paste, _window: &mut Window, cx: &mut Context<Self>) {
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

    fn on_save(&mut self, _: &Save, _window: &mut Window, cx: &mut Context<Self>) {
        let _ = self.save();
        cx.notify();
    }

    fn toggle_reading(&mut self, _: &ToggleReading, _window: &mut Window, cx: &mut Context<Self>) {
        self.reading = !self.reading;
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

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus_handle);
        let Some(index) = self.index_for_point(event.position) else {
            return;
        };

        if event.modifiers.shift {
            self.begin_selection();
        } else {
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
        self.keep_cursor_visible();

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
                .child(self.reading_content.element(
                    self.vault_root.clone(),
                    theme,
                    run_template.clone(),
                    self.link_opener.clone(),
                    cx,
                ))
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

        let (line, column) = self.buffer.line_col();
        let line_count = self.buffer.line_count();
        let words = self.stats_cache.1.words;
        let git_stat = self.git_stat;
        let selection = self.selection_stats();
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
            .child(format!("Ln {}, Col {}", line + 1, column + 1))
            .child(format!("{line_count} lines"))
            .child(format!("{words} words"))
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
            .child(body)
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
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
            metadata_tags(&property.value, theme)
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

fn metadata_tags(value: &PropertyValue, theme: Theme) -> AnyElement {
    let items: Vec<String> = match value {
        PropertyValue::Scalar(text) => vec![text.clone()],
        PropertyValue::List(items) => items.clone(),
        PropertyValue::Raw(raw) => vec![raw.clone()],
    };
    let mut row = div().flex().flex_wrap().gap_1();
    for item in items {
        row = row.child(
            div()
                .px_2()
                .py(px(1.0))
                .rounded_md()
                .bg(theme.chip)
                .text_xs()
                .text_color(theme.text_dim)
                .child(item),
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

        if style.wiki {
            if let Some(href) = span.href.clone() {
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
        // `style.link && !style.wiki` deliberately falls through as plain text.

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

impl IntoElement for LineNumbers {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for LineNumbers {
    type RequestLayoutState = ();
    type PrepaintState = Vec<(usize, Rc<ShapedLine>)>;

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
        let rows = editor
            .last_shaped
            .len()
            .max(editor.buffer.line_count())
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
    ) -> Vec<(usize, Rc<ShapedLine>)> {
        let editor = self.editor.read(cx);
        let line_height = editor.line_height;
        let metas = editor.last_rows.clone();
        if metas.is_empty() {
            return Vec::new();
        }

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
            shaped.push((row, Rc::new(line)));
        }
        shaped
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _state: &mut (),
        lines: &mut Vec<(usize, Rc<ShapedLine>)>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let line_height = self.editor.read(cx).line_height;
        for (row, line) in lines.drain(..) {
            let origin = point(
                bounds.right() - line.width - px(8.0),
                bounds.top() + line_height * row as f32,
            );
            let _ = line.paint(origin, line_height, window, cx);
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
    current_line: Option<PaintQuad>,
    cursor: Option<PaintQuad>,
    /// Rows intersecting the viewport; only these are painted.
    visible: Range<usize>,
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
        });

        let editor = self.editor.read(cx);
        let line_height = editor.line_height;
        let rows = editor.last_shaped.clone();
        let metas = editor.last_rows.clone();

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
        if let Some(selection) = editor.selection() {
            for row_index in visible.clone() {
                let meta = metas[row_index];
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
            current_line,
            cursor,
            visible,
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

        for quad in prepaint.selection.drain(..) {
            window.paint_quad(quad);
        }

        let visible = prepaint.visible.clone();
        let rows = prepaint.rows.clone();
        for i in visible {
            let Some(row) = rows.get(i) else {
                continue;
            };
            let origin = point(bounds.left(), bounds.top() + line_height * i as f32);
            let _ = row.paint(origin, line_height, window, cx);
        }

        if focus_handle.is_focused(window) {
            if let Some(cursor) = prepaint.cursor.take() {
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
}
