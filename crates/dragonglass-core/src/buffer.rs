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
    /// `removed` was replaced by `inserted` starting at char index `at`.
    Replace {
        at: usize,
        removed: String,
        inserted: String,
    },
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
        self.line_col_at(self.cursor)
    }

    /// 0-based `(line, column)` of an arbitrary character index.
    pub fn line_col_at(&self, idx: usize) -> (usize, usize) {
        let idx = idx.min(self.rope.len_chars());
        let line = self.rope.char_to_line(idx);
        let line_start = self.rope.line_to_char(line);
        (line, idx - line_start)
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

    /// Content of `line` without its trailing newline (and `\r`).
    pub fn line(&self, line: usize) -> String {
        if line >= self.rope.len_lines() {
            return String::new();
        }
        let n = self.line_content_len(line);
        self.rope.line(line).chars().take(n).collect()
    }

    /// Number of characters in `line`, excluding the trailing newline (and `\r`).
    pub fn line_char_len(&self, line: usize) -> usize {
        if line >= self.rope.len_lines() {
            0
        } else {
            self.line_content_len(line)
        }
    }

    /// The characters in `range`, clamped to the buffer.
    pub fn slice(&self, range: std::ops::Range<usize>) -> String {
        let start = range.start.min(self.rope.len_chars());
        let end = range.end.min(self.rope.len_chars()).max(start);
        self.rope.slice(start..end).to_string()
    }

    /// Byte offset, within `line`'s text, of character column `col`.
    pub fn char_col_to_byte(&self, line: usize, col: usize) -> usize {
        if line >= self.rope.len_lines() {
            return 0;
        }
        let n = self.line_content_len(line).min(col);
        self.rope
            .line(line)
            .chars()
            .take(n)
            .map(char::len_utf8)
            .sum()
    }

    /// The character index of `col` within `line`. Inverse of `line_col_at` for a line.
    pub fn char_index_at(&self, line: usize, col: usize) -> usize {
        if line >= self.rope.len_lines() {
            return self.rope.len_chars();
        }
        self.rope.line_to_char(line) + self.line_content_len(line).min(col)
    }

    /// Replace `range` with `text` as a single undoable step.
    pub fn replace(&mut self, range: std::ops::Range<usize>, text: &str) {
        let start = range.start.min(self.rope.len_chars());
        let end = range.end.min(self.rope.len_chars()).max(start);
        let removed = self.rope.slice(start..end).to_string();
        if removed.is_empty() && text.is_empty() {
            return;
        }

        self.rope.remove(start..end);
        self.rope.insert(start, text);
        self.cursor = start + text.chars().count();
        self.undo.push(Edit::Replace {
            at: start,
            removed,
            inserted: text.to_string(),
        });
        self.redo.clear();
        self.dirty = true;
    }

    /// Delete `range`, as a single undoable step.
    pub fn delete_range(&mut self, range: std::ops::Range<usize>) {
        self.replace(range, "");
    }

    pub fn move_home(&mut self) {
        let (line, _) = self.line_col();
        self.cursor = self.rope.line_to_char(line);
    }

    pub fn move_end(&mut self) {
        let (line, _) = self.line_col();
        self.cursor = self.rope.line_to_char(line) + self.line_content_len(line);
    }

    /// Convert a character index to a UTF-16 offset (what the platform sends us).
    pub fn char_to_utf16(&self, char_idx: usize) -> usize {
        let n = char_idx.min(self.rope.len_chars());
        self.rope.chars().take(n).map(char::len_utf16).sum()
    }

    /// Convert a UTF-16 offset to a character index.
    pub fn utf16_to_char(&self, utf16_idx: usize) -> usize {
        let mut units = 0usize;
        for (i, c) in self.rope.chars().enumerate() {
            if units >= utf16_idx {
                return i;
            }
            units += c.len_utf16();
        }
        self.rope.len_chars()
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
            Edit::Replace {
                at,
                removed,
                inserted,
            } => {
                self.rope.remove(*at..*at + inserted.chars().count());
                self.rope.insert(*at, removed);
                self.cursor = *at + removed.chars().count();
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
            Edit::Replace {
                at,
                removed,
                inserted,
            } => {
                self.rope.remove(*at..*at + removed.chars().count());
                self.rope.insert(*at, inserted);
                self.cursor = *at + inserted.chars().count();
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

    #[test]
    fn replace_is_one_undo_step() {
        let mut b = Buffer::from_text("hello world");
        b.replace(6..11, "there");
        assert_eq!(b.text(), "hello there");
        assert_eq!(b.cursor(), 11);
        assert!(b.undo());
        assert_eq!(b.text(), "hello world");
        assert!(b.redo());
        assert_eq!(b.text(), "hello there");
    }

    #[test]
    fn line_accessors() {
        let b = Buffer::from_text("hello\nhi\r\nbye");
        assert_eq!(b.line(0), "hello");
        assert_eq!(b.line(1), "hi");
        assert_eq!(b.line(2), "bye");
        assert_eq!(b.line(3), "");
        assert_eq!(b.line_char_len(0), 5);
        assert_eq!(b.line_char_len(1), 2);
    }

    #[test]
    fn slice_clamps() {
        let b = Buffer::from_text("abc");
        assert_eq!(b.slice(1..2), "b");
        assert_eq!(b.slice(1..99), "bc");
    }

    #[test]
    fn utf16_conversions_multibyte() {
        let b = Buffer::from_text("a\u{1F600}b");
        assert_eq!(b.len_chars(), 3);
        assert_eq!(b.char_to_utf16(1), 1);
        assert_eq!(b.char_to_utf16(2), 3);
        assert_eq!(b.char_to_utf16(3), 4);
        assert_eq!(b.utf16_to_char(1), 1);
        assert_eq!(b.utf16_to_char(3), 2);
        assert_eq!(b.utf16_to_char(4), 3);
    }

    #[test]
    fn char_col_to_byte_offsets() {
        let b = Buffer::from_text("h\u{e9}llo");
        assert_eq!(b.char_col_to_byte(0, 0), 0);
        assert_eq!(b.char_col_to_byte(0, 1), 1);
        assert_eq!(b.char_col_to_byte(0, 2), 3);
        assert_eq!(b.char_col_to_byte(0, 99), 6);
    }

    #[test]
    fn home_and_end() {
        let mut b = Buffer::from_text("hello\nworld");
        b.set_cursor(8);
        b.move_home();
        assert_eq!(b.line_col(), (1, 0));
        b.move_end();
        assert_eq!(b.line_col(), (1, 5));
    }
}
