//! A multi-line text editor view, built from scratch on GPUI.
//!
//! GPUI ships no multi-line editor, so this is a hand-rolled `Element` that shapes each
//! line of a `dragonglass_core::Buffer`, paints a caret, and routes keyboard input and
//! platform IME text through `EntityInputHandler`.
//!
//! Phase 1 spike scope: single cursor, no selection, no horizontal scroll, no wrapping.

use std::ops::Range;
use std::path::{Path, PathBuf};

use dragonglass_core::{storage, Buffer};
use gpui::{
    actions, div, fill, point, prelude::*, px, relative, rgb, size, App, Bounds, Context,
    CursorStyle, ElementId, ElementInputHandler, Entity, EntityInputHandler, FocusHandle,
    Focusable, GlobalElementId, InspectorElementId, KeyBinding, LayoutId, PaintQuad, Pixels, Point,
    Render, ShapedLine, SharedString, Style, TextRun, UTF16Selection, Window,
};

actions!(
    editor_actions,
    [Backspace, Delete, Left, Right, Up, Down, Home, End, Enter, Paste, Save]
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
        KeyBinding::new("enter", Enter, None),
        KeyBinding::new("ctrl-v", Paste, None),
        KeyBinding::new("cmd-v", Paste, None),
        KeyBinding::new("ctrl-s", Save, None),
        KeyBinding::new("cmd-s", Save, None),
    ]);
}

pub struct Editor {
    buffer: Buffer,
    path: Option<PathBuf>,
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

    fn backspace(&mut self, _: &Backspace, _window: &mut Window, cx: &mut Context<Self>) {
        self.buffer.backspace();
        cx.notify();
    }

    fn delete(&mut self, _: &Delete, _window: &mut Window, cx: &mut Context<Self>) {
        self.buffer.delete_forward();
        cx.notify();
    }

    fn left(&mut self, _: &Left, _window: &mut Window, cx: &mut Context<Self>) {
        self.buffer.move_left();
        cx.notify();
    }

    fn right(&mut self, _: &Right, _window: &mut Window, cx: &mut Context<Self>) {
        self.buffer.move_right();
        cx.notify();
    }

    fn up(&mut self, _: &Up, _window: &mut Window, cx: &mut Context<Self>) {
        self.buffer.move_up();
        cx.notify();
    }

    fn down(&mut self, _: &Down, _window: &mut Window, cx: &mut Context<Self>) {
        self.buffer.move_down();
        cx.notify();
    }

    fn home(&mut self, _: &Home, _window: &mut Window, cx: &mut Context<Self>) {
        self.buffer.move_home();
        cx.notify();
    }

    fn end(&mut self, _: &End, _window: &mut Window, cx: &mut Context<Self>) {
        self.buffer.move_end();
        cx.notify();
    }

    fn enter(&mut self, _: &Enter, _window: &mut Window, cx: &mut Context<Self>) {
        self.buffer.insert("\n");
        cx.notify();
    }

    fn paste(&mut self, _: &Paste, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.buffer.insert(&text);
            cx.notify();
        }
    }

    fn on_save(&mut self, _: &Save, _window: &mut Window, cx: &mut Context<Self>) {
        let _ = self.save();
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

        div()
            .flex()
            .flex_col()
            .size_full()
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap_2()
                    .px_2()
                    .py_1()
                    .bg(rgb(0x1b1b1b))
                    .text_xs()
                    .text_color(rgb(0x9a9a9a))
                    .child(title)
                    .when(self.buffer.is_dirty(), |d| {
                        d.child(div().text_color(rgb(0xe0b050)).child("\u{25CF}"))
                    }),
            )
            .child(
                div()
                    .key_context("Editor")
                    .track_focus(&self.focus_handle)
                    .cursor(CursorStyle::IBeam)
                    .size_full()
                    .id("editor-scroll")
                    .overflow_y_scroll()
                    .on_action(cx.listener(Self::backspace))
                    .on_action(cx.listener(Self::delete))
                    .on_action(cx.listener(Self::left))
                    .on_action(cx.listener(Self::right))
                    .on_action(cx.listener(Self::up))
                    .on_action(cx.listener(Self::down))
                    .on_action(cx.listener(Self::home))
                    .on_action(cx.listener(Self::end))
                    .on_action(cx.listener(Self::enter))
                    .on_action(cx.listener(Self::paste))
                    .on_action(cx.listener(Self::on_save))
                    .child(EditorText {
                        editor: cx.entity(),
                    }),
            )
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
        let cursor = self.buffer.char_to_utf16(self.buffer.cursor());
        Some(UTF16Selection {
            range: cursor..cursor,
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
            .unwrap_or_else(|| self.buffer.cursor()..self.buffer.cursor());

        if range.is_empty() && !new_text.is_empty() {
            self.buffer.insert(new_text);
        } else {
            self.buffer.replace(range, new_text);
        }
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
            .unwrap_or_else(|| self.buffer.cursor()..self.buffer.cursor());
        let start = range.start;
        self.buffer.replace(range, new_text);
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
        let bounds = self.last_bounds?;
        let local = bounds.localize(&point)?;

        let line = (f32::from(local.y) / f32::from(self.line_height))
            .floor()
            .max(0.0) as usize;
        let line = line.min(self.buffer.line_count().saturating_sub(1));
        let shaped = self.last_lines.get(line)?;

        let byte = shaped.closest_index_for_x(local.x);
        let line_text = self.buffer.line(line);
        let char_col = line_text[..byte.min(line_text.len())].chars().count();
        let char_index = self.buffer.char_index_at(line, char_col);

        Some(self.buffer.char_to_utf16(char_index))
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

        Prepaint { lines, cursor }
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
