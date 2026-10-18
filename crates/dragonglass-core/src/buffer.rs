//! A rope-backed editable text buffer: the spine of the editor.
//!
//! Deliberately UI-free so it can be unit-tested and shared with the CLI. Positions are
//! **character indices**, never byte offsets, so multibyte text is handled correctly.
//!
//! Not yet implemented (tracked for Phase 1): multi-cursor, selections, and file loading/
//! saving through this type.

use ropey::Rope;

/// A single reversible edit, stored for undo/redo.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Edit {
    /// `text` was inserted at char index `at`.
    Insert { at: usize, text: String },
    /// `text.chars().count()` chars were removed starting at char index `at`.
    Remove { at: usize, text: String },
}

#[derive(Debug, Clone, Default)]
pub struct Buffer {
    rope: Rope,
    /// Cursor position as a character index into `rope`.
    cursor: usize,
    undo: Vec<Edit>,
    redo: Vec<Edit>,
    dirty: bool,
}

impl Buffer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_text(text: &str) -> Self {
        Self {
            rope: Rope::from_str(text),
            ..Self::default()
        }
    }

    /// Whole contents as a `String`.
    pub fn text(&self) -> String {
        self.rope.to_string()
    }

    pub fn len_chars(&self) -> usize {
        self.rope.len_chars()
    }

    pub fn is_empty(&self) -> bool {
        self.rope.len_chars() == 0
    }

    pub fn line_count(&self) -> usize {
        if self.rope.len_chars() == 0 {
            return 0;
        }
        // `len_lines` counts a trailing empty line for text ending in `\n`; drop it.
        let lines = self.rope.len_lines();
        if self.rope.char(self.rope.len_chars() - 1) == '\n' {
            lines.saturating_sub(1)
        } else {
            lines
        }
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Move the cursor to `at`, clamped to the buffer length.
    pub fn set_cursor(&mut self, at: usize) {
        self.cursor = at.min(self.rope.len_chars());
    }

    /// 0-based `(line, column)` of the cursor.
    pub fn line_col(&self) -> (usize, usize) {
        let line = self.rope.char_to_line(self.cursor);
        let line_start = self.rope.line_to_char(line);
        (line, self.cursor - line_start)
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn mark_saved(&mut self) {
        self.dirty = false;
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Insert `text` at the cursor, advancing it. Contiguous typing coalesces into one
    /// undo step.
    pub fn insert(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }

        let at = self.cursor;
        self.rope.insert(at, text);
        self.cursor = at + text.chars().count();

        match self.undo.last_mut() {
            Some(Edit::Insert {
                at: prev_at,
                text: prev,
            }) if *prev_at + prev.chars().count() == at => {
                prev.push_str(text);
            }
            _ => self.undo.push(Edit::Insert {
                at,
                text: text.to_string(),
            }),
        }

        self.redo.clear();
        self.dirty = true;
    }

    /// Delete the character before the cursor. Returns whether anything was deleted.
    pub fn backspace(&mut self) -> bool {
        if self.cursor == 0 {
            return false;
        }
        let at = self.cursor - 1;
        let removed = self.rope.slice(at..self.cursor).to_string();
        self.rope.remove(at..self.cursor);
        self.cursor = at;

        // Merge with the previous deletion when it was contiguous.
        match self.undo.last_mut() {
            Some(Edit::Remove {
                at: prev_at,
                text: prev,
            }) if at + removed.chars().count() == *prev_at => {
                *prev = format!("{removed}{prev}");
                *prev_at = at;
            }
            _ => self.undo.push(Edit::Remove { at, text: removed }),
        }

        self.redo.clear();
        self.dirty = true;
        true
    }

    /// Delete the character after the cursor. Returns whether anything was deleted.
    pub fn delete_forward(&mut self) -> bool {
        if self.cursor >= self.rope.len_chars() {
            return false;
        }
        let at = self.cursor;
        let removed = self.rope.slice(at..at + 1).to_string();
        self.rope.remove(at..at + 1);
        self.undo.push(Edit::Remove { at, text: removed });
        self.redo.clear();
        self.dirty = true;
        true
    }

    pub fn move_left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub fn move_right(&mut self) {
        self.set_cursor(self.cursor + 1);
    }

    pub fn move_up(&mut self) {
        let (line, col) = self.line_col();
        if line == 0 {
            return;
        }
        let target = line - 1;
        let start = self.rope.line_to_char(target);
        self.cursor = start + col.min(self.line_content_len(target));
    }

    pub fn move_down(&mut self) {
        let (line, col) = self.line_col();
        let last = self.rope.len_lines().saturating_sub(1);
        if line >= last {
            return;
        }
        let target = line + 1;
        let start = self.rope.line_to_char(target);
        self.cursor = start + col.min(self.line_content_len(target));
    }

    pub fn undo(&mut self) -> bool {
        let Some(edit) = self.undo.pop() else {
            return false;
        };
        match &edit {
            Edit::Insert { at, text } => {
                self.rope.remove(*at..*at + text.chars().count());
                self.cursor = *at;
            }
            Edit::Remove { at, text } => {
                self.rope.insert(*at, text);
                self.cursor = *at + text.chars().count();
            }
        }
        self.redo.push(edit);
        self.dirty = true;
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(edit) = self.redo.pop() else {
            return false;
        };
        match &edit {
            Edit::Insert { at, text } => {
                self.rope.insert(*at, text);
                self.cursor = *at + text.chars().count();
            }
            Edit::Remove { at, text } => {
                self.rope.remove(*at..*at + text.chars().count());
                self.cursor = *at;
            }
        }
        self.undo.push(edit);
        self.dirty = true;
        true
    }

    /// Number of chars before the newline (and any `\r`) at the end of `line`.
    fn line_content_len(&self, line: usize) -> usize {
        let slice = self.rope.line(line);
        let mut n = slice.len_chars();
        if n > 0 && slice.char(n - 1) == '\n' {
            n -= 1;
        }
        if n > 0 && slice.char(n - 1) == '\r' {
            n -= 1;
        }
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_and_cursor() {
        let mut b = Buffer::new();
        b.insert("hello");
        assert_eq!(b.text(), "hello");
        assert_eq!(b.cursor(), 5);
    }

    #[test]
    fn backspace_at_start_is_noop() {
        let mut b = Buffer::new();
        assert!(!b.backspace());
    }

    #[test]
    fn delete_forward_at_end_is_noop() {
        let mut b = Buffer::from_text("abc");
        b.set_cursor(3);
        assert!(!b.delete_forward());
    }

    #[test]
    fn typing_coalesces_into_one_undo() {
        let mut b = Buffer::new();
        for c in ["h", "e", "l", "l", "o"] {
            b.insert(c);
        }
        assert!(b.undo());
        assert_eq!(b.text(), "");
        assert!(!b.can_undo());
        assert!(b.redo());
        assert_eq!(b.text(), "hello");
    }

    #[test]
    fn non_contiguous_inserts_are_separate_undo_steps() {
        let mut b = Buffer::new();
        b.insert("abc");
        b.move_left();
        b.move_left();
        b.insert("X");
        assert_eq!(b.text(), "aXbc");
        b.undo();
        assert_eq!(b.text(), "abc");
        b.undo();
        assert_eq!(b.text(), "");
    }

    #[test]
    fn backspace_coalesces() {
        let mut b = Buffer::from_text("hello");
        b.set_cursor(5);
        b.backspace();
        b.backspace();
        assert_eq!(b.text(), "hel");
        b.undo();
        assert_eq!(b.text(), "hello");
    }

    #[test]
    fn undo_redo_roundtrip() {
        let mut b = Buffer::from_text("abc");
        b.set_cursor(3);
        b.insert("!");
        assert_eq!(b.text(), "abc!");
        assert!(b.undo());
        assert_eq!(b.text(), "abc");
        assert!(b.redo());
        assert_eq!(b.text(), "abc!");
    }

    #[test]
    fn multibyte_is_one_char() {
        let mut b = Buffer::new();
        b.insert("héllo");
        assert_eq!(b.len_chars(), 5);
        b.backspace();
        assert_eq!(b.text(), "héll");
    }

    #[test]
    fn vertical_movement_preserves_column() {
        let mut b = Buffer::from_text("hello\nhi\nyes");
        b.set_cursor(4); // line 0, col 4
        b.move_down(); // line 1 is "hi", col clamps to 2
        assert_eq!(b.line_col(), (1, 2));
        b.move_down();
        assert_eq!(b.line_col(), (2, 2));
        b.move_up();
        assert_eq!(b.line_col(), (1, 2));
    }

    #[test]
    fn line_count_ignores_trailing_newline() {
        assert_eq!(Buffer::from_text("a\nb\n").line_count(), 2);
        assert_eq!(Buffer::from_text("a\nb").line_count(), 2);
        assert_eq!(Buffer::new().line_count(), 0);
    }

    #[test]
    fn dirty_tracking() {
        let mut b = Buffer::new();
        assert!(!b.is_dirty());
        b.insert("x");
        assert!(b.is_dirty());
        b.mark_saved();
        assert!(!b.is_dirty());
        b.undo();
        assert!(b.is_dirty());
    }
}
