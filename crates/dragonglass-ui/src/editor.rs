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
use dragonglass_core::{storage, Buffer};
use gpui::{
    actions, div, fill, point, prelude::*, px, relative, rgb, rgba, size, AnyElement, App, Bounds,
    ClipboardItem, Context, CursorStyle, ElementId, ElementInputHandler, Entity,
    EntityInputHandler, FocusHandle, Focusable, GlobalElementId, InspectorElementId, KeyBinding,
    LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad, Pixels, Point,
    Render, ShapedLine, SharedString, Style, TextRun, UTF16Selection, Window,
};

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
    last_lines: Vec<ShapedLine>,
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
            last_lines: Vec::new(),
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
        self.path = Some(path.to_path_buf());
        Ok(())
    }

    /// Write the buffer back to its file, atomically.
    pub fn save(&mut self) -> std::io::Result<()> {
        let Some(path) = self.path.clone() else {
            return Ok(());
        };
        storage::write_atomic(&path, &self.buffer.text())?;
        self.buffer.mark_saved();
        Ok(())
    }

    pub fn is_dirty(&self) -> bool {
        self.buffer.is_dirty()
    }

    /// Toggle between edit and reading mode (used by the command palette).
    pub fn toggle_reading_mode(&mut self) {
        self.reading = !self.reading;
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
        self.buffer.move_up();
        cx.notify();
    }

    fn down(&mut self, _: &Down, _window: &mut Window, cx: &mut Context<Self>) {
        self.collapse_selection();
        self.buffer.move_down();
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
        self.buffer.move_up();
        cx.notify();
    }

    fn select_down(&mut self, _: &SelectDown, _window: &mut Window, cx: &mut Context<Self>) {
        self.begin_selection();
        self.buffer.move_down();
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

        let line = (f32::from(local.y) / f32::from(self.line_height))
            .floor()
            .max(0.0) as usize;
        let line = line.min(self.buffer.line_count().saturating_sub(1));
        let shaped = self.last_lines.get(line)?;

        let byte = shaped.closest_index_for_x(local.x);
        let line_text = self.buffer.line(line);
        let char_col = line_text[..byte.min(line_text.len())].chars().count();
        Some(self.buffer.char_index_at(line, char_col))
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
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
                            .bg(rgb(0x2a2a2a))
                            .text_xs()
                            .text_color(rgb(0xc0c0c0))
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
                    .bg(rgb(0x1b1b1b))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .justify_between()
                            .px_2()
                            .py_1()
                            .text_xs()
                            .text_color(rgb(0x9a9a9a))
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .gap_2()
                                    .child(title)
                                    .when(dirty, |d| {
                                        d.child(div().text_color(rgb(0xe0b050)).child("\u{25CF}"))
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
        let end = self.buffer.utf16_to_char(range_utf16.end);
        let (line, start_col) = self.buffer.line_col_at(start);
        let (_, end_col) = self.buffer.line_col_at(end);
        let shaped = self.last_lines.get(line)?;

        let x0 = shaped.x_for_index(self.buffer.char_col_to_byte(line, start_col));
        let x1 = shaped.x_for_index(self.buffer.char_col_to_byte(line, end_col));
        let top = bounds.top() + self.line_height * line as f32;

        Some(Bounds::from_corners(
            point(bounds.left() + x0, top),
            point(bounds.left() + x1, top + self.line_height),
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
    lines: Vec<ShapedLine>,
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
        let line_count = editor.buffer.line_count().max(1) as f32;
        style.size.height = (editor.line_height * line_count).into();
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
        let editor = self.editor.read(cx);
        let text_style = window.text_style();
        let font = text_style.font();
        let color = text_style.color;
        let font_size = editor.font_size;
        let line_height = editor.line_height;

        let count = editor.buffer.line_count().max(1);
        let mut lines = Vec::with_capacity(count);
        for i in 0..count {
            let text = editor.buffer.line(i);
            let run = TextRun {
                len: text.len(),
                font: font.clone(),
                color,
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            let shaped =
                window
                    .text_system()
                    .shape_line(SharedString::from(text), font_size, &[run], None);
            lines.push(shaped);
        }

        let mut selection_quads = Vec::new();
        if let Some(selection) = editor.selection() {
            for (i, shaped) in lines.iter().enumerate() {
                let line_start = editor.buffer.char_index_at(i, 0);
                let line_end = line_start + editor.buffer.line_char_len(i);
                let start = selection.start.max(line_start);
                let end = selection.end.min(line_end);
                if start >= end {
                    continue;
                }

                let x0 = shaped.x_for_index(editor.buffer.char_col_to_byte(i, start - line_start));
                let x1 = shaped.x_for_index(editor.buffer.char_col_to_byte(i, end - line_start));
                let top = bounds.top() + line_height * i as f32;
                selection_quads.push(fill(
                    Bounds::from_corners(
                        point(bounds.left() + x0, top),
                        point(bounds.left() + x1, top + line_height),
                    ),
                    rgba(0x3b82f640),
                ));
            }
        }

        let (line_idx, col) = editor.buffer.line_col();
        let cursor = match lines.get(line_idx) {
            Some(shaped) => {
                let byte = editor.buffer.char_col_to_byte(line_idx, col);
                let x = shaped.x_for_index(byte);
                Some(fill(
                    Bounds::new(
                        point(
                            bounds.left() + x,
                            bounds.top() + line_height * line_idx as f32,
                        ),
                        size(px(2.0), line_height),
                    ),
                    color,
                ))
            }
            None => None,
        };

        Prepaint {
            lines,
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

        for (i, line) in prepaint.lines.iter().enumerate() {
            let origin = point(bounds.left(), bounds.top() + line_height * i as f32);
            let _ = line.paint(origin, line_height, window, cx);
        }

        if focus_handle.is_focused(window) {
            if let Some(cursor) = prepaint.cursor.take() {
                window.paint_quad(cursor);
            }
        }

        let lines = std::mem::take(&mut prepaint.lines);
        self.editor.update(cx, |editor, _cx| {
            editor.last_bounds = Some(bounds);
            editor.last_lines = lines;
        });
    }
}
