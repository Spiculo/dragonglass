//! A multi-line text editor view, built from scratch on GPUI.
//!
//! GPUI ships no multi-line editor, so this is a hand-rolled `Element` that shapes each
//! line of a `dragonglass_core::Buffer`, paints a caret and selection, and routes keyboard
//! input and platform IME text through `EntityInputHandler`.
//!
//! Still missing (tracked): word wrap, horizontal scroll, multi-cursor.

use std::ops::Range;
use std::path::{Path, PathBuf};

use dragonglass_core::frontmatter::{self, Property};
use dragonglass_core::{history, storage, wrap, Buffer};
use gpui::{
    actions, div, fill, point, prelude::*, px, relative, size, AnyElement, App, Bounds,
    ClipboardItem, Context, CursorStyle, ElementId, ElementInputHandler, Entity,
    EntityInputHandler, FocusHandle, Focusable, GlobalElementId, InspectorElementId, KeyBinding,
    LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad, Pixels, Point,
    Render, ShapedLine, SharedString, Style, TextRun, UTF16Selection, Window,
};

use crate::theme::Theme;

actions!(
    editor_actions,
    [
        Backspace,
        Delete,
        Left,
        Right,
        Up,
        Down,
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
        SelectHome,
        SelectEnd,
        SelectAll,
        Copy,
        Cut
    ]
);

/// Register the editor's default key bindings. Obsidian-compatible where it matters.
pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("backspace", Backspace, None),
        KeyBinding::new("delete", Delete, None),
        KeyBinding::new("left", Left, None),
        KeyBinding::new("right", Right, None),
        KeyBinding::new("up", Up, None),
        KeyBinding::new("down", Down, None),
        KeyBinding::new("home", Home, None),
        KeyBinding::new("end", End, None),
        KeyBinding::new("shift-left", SelectLeft, None),
        KeyBinding::new("shift-right", SelectRight, None),
        KeyBinding::new("shift-up", SelectUp, None),
        KeyBinding::new("shift-down", SelectDown, None),
        KeyBinding::new("shift-home", SelectHome, None),
        KeyBinding::new("shift-end", SelectEnd, None),
        KeyBinding::new("ctrl-a", SelectAll, None),
        KeyBinding::new("cmd-a", SelectAll, None),
        KeyBinding::new("ctrl-c", Copy, None),
        KeyBinding::new("cmd-c", Copy, None),
        KeyBinding::new("ctrl-x", Cut, None),
        KeyBinding::new("cmd-x", Cut, None),
        KeyBinding::new("enter", Enter, None),
        KeyBinding::new("ctrl-v", Paste, None),
        KeyBinding::new("cmd-v", Paste, None),
        KeyBinding::new("ctrl-s", Save, None),
        KeyBinding::new("cmd-s", Save, None),
        KeyBinding::new("ctrl-e", ToggleReading, None),
        KeyBinding::new("cmd-e", ToggleReading, None),
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
    last_bounds: Option<Bounds<Pixels>>,
    /// Visual rows produced by the last paint, for hit-testing and IME.
    last_rows: Vec<RowMeta>,
    last_shaped: Vec<ShapedLine>,
}

/// A wrapped visual row: `[start, end)` character columns of logical line `line`.
#[derive(Debug, Clone, Copy)]
struct RowMeta {
    line: usize,
    start: usize,
    end: usize,
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
            last_rows: Vec::new(),
            last_shaped: Vec::new(),
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
        self.last_rows.clear();
        self.last_shaped.clear();
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
        self.buffer = Buffer::from_text(&text);
        self.buffer.mark_saved();
        self.anchor = None;
        self.marked_range = None;
        self.last_rows.clear();
        self.last_shaped.clear();
        Ok(())
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

    /// Tell the editor where the vault lives, for resolving embeds.
    pub fn set_vault_root(&mut self, root: &Path) {
        self.vault_root = Some(root.to_path_buf());
    }

    /// Parsed frontmatter properties. Empty when absent or unsupported.
    pub fn properties(&self) -> Vec<Property> {
        frontmatter::properties(&self.buffer.text()).unwrap_or_default()
    }

    /// Replace the frontmatter with `properties`, keep the body, and save.
    pub fn set_properties(&mut self, properties: &[Property]) -> std::io::Result<()> {
        let updated = frontmatter::with_properties(&self.buffer.text(), properties);
        self.buffer = Buffer::from_text(&updated);
        self.anchor = None;
        self.save()
    }

    /// Current document text.
    pub fn text(&self) -> String {
        self.buffer.text()
    }

    /// Move the caret to the start of `line` (0-based) and clear any selection.
    pub fn jump_to_line(&mut self, line: usize) {
        let line = line.min(self.buffer.line_count().saturating_sub(1));
        self.buffer.set_cursor(self.buffer.char_index_at(line, 0));
        self.anchor = None;
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

    fn on_select_all(&mut self, _: &SelectAll, _window: &mut Window, cx: &mut Context<Self>) {
        self.anchor = Some(0);
        self.buffer.set_cursor(self.buffer.len_chars());
        cx.notify();
    }

    fn copy(&mut self, _: &Copy, _window: &mut Window, cx: &mut Context<Self>) {
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

    fn enter(&mut self, _: &Enter, _window: &mut Window, cx: &mut Context<Self>) {
        self.delete_selection();
        self.buffer.insert("\n");
        cx.notify();
    }

    fn paste(&mut self, _: &Paste, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.insert_text(&text);
            cx.notify();
        }
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
            .unwrap_or_default();
        let dirty = self.buffer.is_dirty();
        let reading = self.reading;

        let properties = self.properties();
        let chips: Option<AnyElement> = if properties.is_empty() {
            None
        } else {
            Some(
                div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .gap_2()
                    .px_2()
                    .pb_1()
                    .children(properties.iter().map(|property| {
                        div()
                            .px_2()
                            .py_1()
                            .rounded_md()
                            .bg(theme.chip)
                            .text_xs()
                            .text_color(theme.text_dim)
                            .child(format!("{}: {}", property.key, property.value.display()))
                    }))
                    .into_any_element(),
            )
        };

        let body: AnyElement = if reading {
            div()
                .id("reading-scroll")
                .flex_1()
                .min_h(px(0.0))
                .overflow_y_scroll()
                .child(crate::reading::reading_view(
                    &self.buffer.text(),
                    self.vault_root.as_deref(),
                    0,
                    theme,
                    &run_template,
                ))
                .into_any_element()
        } else {
            div()
                .id("editor-scroll")
                .flex_1()
                .min_h(px(0.0))
                .cursor(CursorStyle::IBeam)
                .overflow_y_scroll()
                .on_action(cx.listener(Self::backspace))
                .on_action(cx.listener(Self::delete))
                .on_action(cx.listener(Self::left))
                .on_action(cx.listener(Self::right))
                .on_action(cx.listener(Self::up))
                .on_action(cx.listener(Self::down))
                .on_action(cx.listener(Self::home))
                .on_action(cx.listener(Self::end))
                .on_action(cx.listener(Self::select_left))
                .on_action(cx.listener(Self::select_right))
                .on_action(cx.listener(Self::select_up))
                .on_action(cx.listener(Self::select_down))
                .on_action(cx.listener(Self::select_home))
                .on_action(cx.listener(Self::select_end))
                .on_action(cx.listener(Self::on_select_all))
                .on_action(cx.listener(Self::copy))
                .on_action(cx.listener(Self::cut))
                .on_action(cx.listener(Self::enter))
                .on_action(cx.listener(Self::paste))
                .on_action(cx.listener(Self::on_save))
                .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
                .on_mouse_move(cx.listener(Self::on_mouse_move))
                .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
                .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
                .child(EditorText {
                    editor: cx.entity(),
                })
                .into_any_element()
        };

        div()
            .key_context("Editor")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::toggle_reading))
            .flex()
            .flex_col()
            .size_full()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .bg(theme.panel_alt)
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .justify_between()
                            .px_2()
                            .py_1()
                            .text_xs()
                            .text_color(theme.muted)
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .gap_2()
                                    .child(title)
                                    .when(dirty, |d| {
                                        d.child(div().text_color(theme.warn).child("\u{25CF}"))
                                    }),
                            )
                            .child(if reading { "read" } else { "edit" }),
                    )
                    .children(chips),
            )
            .child(body)
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

struct EditorText {
    editor: Entity<Editor>,
}

impl IntoElement for EditorText {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

struct Prepaint {
    rows: Vec<ShapedLine>,
    metas: Vec<RowMeta>,
    selection: Vec<PaintQuad>,
    cursor: Option<PaintQuad>,
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
        let editor = self.editor.read(cx);
        let text_style = window.text_style();
        let font = text_style.font();
        let color = text_style.color;
        let font_size = editor.font_size;
        let line_height = editor.line_height;
        let available = f32::from(bounds.size.width).max(1.0);

        let shape = |text: &str| {
            let run = TextRun {
                len: text.len(),
                font: font.clone(),
                color,
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            window.text_system().shape_line(
                SharedString::from(text.to_string()),
                font_size,
                &[run],
                None,
            )
        };

        let count = editor.buffer.line_count().max(1);
        let mut rows: Vec<ShapedLine> = Vec::new();
        let mut metas: Vec<RowMeta> = Vec::new();

        for line_index in 0..count {
            let text = editor.buffer.line(line_index);
            let chars: Vec<char> = text.chars().collect();

            // Byte offset of every character column, for measuring and slicing.
            let mut offsets = Vec::with_capacity(chars.len() + 1);
            let mut acc = 0usize;
            for ch in &chars {
                offsets.push(acc);
                acc += ch.len_utf8();
            }
            offsets.push(acc);

            let ranges: Vec<(usize, usize)> = if chars.is_empty() {
                vec![(0, 0)]
            } else {
                let full = shape(&text);
                let width_of = |from: usize, to: usize| -> f32 {
                    f32::from(full.x_for_index(offsets[to]))
                        - f32::from(full.x_for_index(offsets[from]))
                };
                wrap::wrap(&chars, width_of, available)
            };

            for (start, end) in ranges {
                let segment: String = chars[start..end].iter().collect();
                rows.push(shape(&segment));
                metas.push(RowMeta {
                    line: line_index,
                    start,
                    end,
                });
            }
        }

        let row_byte = |line: usize, row_start: usize, col: usize| -> usize {
            editor.buffer.char_col_to_byte(line, col)
                - editor.buffer.char_col_to_byte(line, row_start)
        };

        let mut selection_quads = Vec::new();
        if let Some(selection) = editor.selection() {
            for (row_index, meta) in metas.iter().enumerate() {
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
        for (row_index, meta) in metas.iter().enumerate() {
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

        Prepaint {
            rows,
            metas,
            selection: selection_quads,
            cursor,
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

        for quad in prepaint.selection.drain(..) {
            window.paint_quad(quad);
        }

        for (i, row) in prepaint.rows.iter().enumerate() {
            let origin = point(bounds.left(), bounds.top() + line_height * i as f32);
            let _ = row.paint(origin, line_height, window, cx);
        }

        if focus_handle.is_focused(window) {
            if let Some(cursor) = prepaint.cursor.take() {
                window.paint_quad(cursor);
            }
        }

        let rows = std::mem::take(&mut prepaint.rows);
        let metas = std::mem::take(&mut prepaint.metas);
        self.editor.update(cx, |editor, _cx| {
            editor.last_bounds = Some(bounds);
            editor.last_rows = metas;
            editor.last_shaped = rows;
        });
    }
}
