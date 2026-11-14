//! A rope-backed editable text buffer: the spine of the editor.
//!
//! Deliberately UI-free so it can be unit-tested and shared with the CLI. Positions are
//! **character indices**, never byte offsets, so multibyte text is handled correctly.
//!
//! The editor owns cursor/selection state; the buffer stores the primary caret and applies
//! (possibly batched) edits.

use ropey::Rope;

/// FNV-1a constants, matching `fingerprint` so a buffer hash and a file hash are comparable.
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

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
    /// Several replacements applied as one undoable step. Used by multi-cursor edits.
    Batch(Vec<Edit>),
}

#[derive(Debug, Clone)]
pub struct Buffer {
    rope: Rope,
    /// Cursor position as a character index into `rope`.
    cursor: usize,
    undo: Vec<Edit>,
    redo: Vec<Edit>,
    /// Content hash of the version last loaded or saved. Dirty state is derived from the
    /// text itself, so typing and undoing back to the saved bytes reports clean again.
    saved_hash: u64,
    /// `(revision, hash)` of the current text, so repeated `is_dirty` calls cost one hash
    /// per edit rather than one per call.
    hash_cache: std::cell::Cell<Option<(u64, u64)>>,
    /// Bumped on every change, so the UI can cache derived layout.
    revision: u64,
}

impl Default for Buffer {
    fn default() -> Self {
        Self::new()
    }
}

impl Buffer {
    pub fn new() -> Self {
        Self::from_text("")
    }

    pub fn from_text(text: &str) -> Self {
        let mut buffer = Self {
            rope: Rope::from_str(text),
            cursor: 0,
            undo: Vec::new(),
            redo: Vec::new(),
            saved_hash: FNV_OFFSET,
            hash_cache: std::cell::Cell::new(None),
            revision: 0,
        };
        buffer.saved_hash = buffer.content_hash();
        buffer
    }

    /// FNV-1a over the buffer contents, cached per revision.
    ///
    /// Dirty state compares a hash instead of a boolean so that an edit and its undo, or
    /// typing and deleting the same characters, return to clean.
    fn content_hash(&self) -> u64 {
        if let Some((revision, hash)) = self.hash_cache.get() {
            if revision == self.revision {
                return hash;
            }
        }
        let mut hash = FNV_OFFSET;
        for chunk in self.rope.chunks() {
            for byte in chunk.bytes() {
                hash ^= u64::from(byte);
                hash = hash.wrapping_mul(FNV_PRIME);
            }
        }
        self.hash_cache.set(Some((self.revision, hash)));
        hash
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
        self.content_hash() != self.saved_hash
    }

    /// Monotonic change counter, for cache invalidation.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn mark_saved(&mut self) {
        self.saved_hash = self.content_hash();
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

    /// Whether `line`'s content equals `text`, ignoring the trailing newline.
    ///
    /// Lets the editor validate a cached line layout without allocating a copy of the
    /// line, which matters on every frame of a long note.
    pub fn line_eq(&self, line: usize, text: &str) -> bool {
        if line >= self.rope.len_lines() {
            return text.is_empty();
        }
        let n = self.line_content_len(line);
        let mut chars = self.rope.line(line).chars().take(n);
        for expected in text.chars() {
            if chars.next() != Some(expected) {
                return false;
            }
        }
        chars.next().is_none()
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
        self.revision = self.revision.wrapping_add(1);
    }

    /// Apply several replacements as one undoable step, for multi-cursor edits.
    ///
    /// Ranges may be supplied in any order; they are applied bottom-up so earlier edits do
    /// not shift later offsets. Overlapping ranges are ignored rather than corrupting the
    /// buffer. Returns the number of edits that actually changed something.
    pub fn replace_many(&mut self, mut edits: Vec<(std::ops::Range<usize>, String)>) -> usize {
        let len = self.rope.len_chars();
        for (range, _) in &mut edits {
            range.start = range.start.min(len);
            range.end = range.end.min(len).max(range.start);
        }

        // Keep the first of any duplicate/overlapping ranges. The caller normally sends
        // selections from top to bottom; sorting by start makes overlap checks predictable.
        edits.sort_by_key(|(range, _)| (range.start, range.end));
        let mut kept: Vec<(std::ops::Range<usize>, String)> = Vec::new();
        let mut last_end = 0usize;
        let mut seen: Vec<(usize, usize)> = Vec::new();
        for (range, text) in edits {
            if range.start < last_end {
                continue;
            }
            if seen.contains(&(range.start, range.end)) {
                continue;
            }
            let removed_len = range.end - range.start;
            if removed_len == 0 && text.is_empty() {
                continue;
            }
            seen.push((range.start, range.end));
            last_end = if range.start == range.end {
                range.start + 1
            } else {
                range.end
            };
            kept.push((range, text));
        }

        if kept.is_empty() {
            return 0;
        }

        let count = kept.len();
        let lowest_start = kept[0].0.start;
        let lowest_inserted = kept[0].1.chars().count();
        let mut batch = Vec::with_capacity(count);
        for (range, text) in kept.into_iter().rev() {
            let removed = self.rope.slice(range.clone()).to_string();
            self.rope.remove(range.clone());
            self.rope.insert(range.start, &text);
            batch.push(Edit::Replace {
                at: range.start,
                removed,
                inserted: text,
            });
        }

        self.cursor = lowest_start + lowest_inserted;
        self.undo.push(Edit::Batch(batch));
        self.redo.clear();
        self.revision = self.revision.wrapping_add(1);
        count
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
    ///
    /// ASCII chunks are counted a chunk at a time; the platform asks for this on every
    /// keystroke (and again for the IME candidate window), so it must stay cheap on large
    /// notes.
    pub fn char_to_utf16(&self, char_idx: usize) -> usize {
        let n = char_idx.min(self.rope.len_chars());
        let mut units = 0usize;
        for chunk in self.rope.slice(0..n).chunks() {
            if chunk.is_ascii() {
                units += chunk.len();
            } else {
                units += chunk.chars().map(char::len_utf16).sum::<usize>();
            }
        }
        units
    }

    /// Convert a UTF-16 offset to a character index.
    pub fn utf16_to_char(&self, utf16_idx: usize) -> usize {
        let mut units = 0usize;
        let mut chars = 0usize;
        for chunk in self.rope.chunks() {
            if chunk.is_ascii() {
                let len = chunk.len();
                if units + len >= utf16_idx {
                    return chars + utf16_idx.saturating_sub(units);
                }
                units += len;
                chars += len;
            } else {
                for ch in chunk.chars() {
                    if units >= utf16_idx {
                        return chars;
                    }
                    units += ch.len_utf16();
                    chars += 1;
                }
            }
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
        self.revision = self.revision.wrapping_add(1);
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
        self.revision = self.revision.wrapping_add(1);
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
        self.revision = self.revision.wrapping_add(1);
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

    /// Move to the start of the previous word (Ctrl+Left).
    pub fn move_word_left(&mut self) {
        self.cursor = self.word_left_from(self.cursor);
    }

    /// Move to the start of the next word (Ctrl+Right).
    pub fn move_word_right(&mut self) {
        self.cursor = self.word_right_from(self.cursor);
    }

    /// Start of the current paragraph, or of the previous one when already at the start
    /// (Ctrl+Up).
    pub fn move_paragraph_up(&mut self) {
        let (line, _) = self.line_col();
        let mut start = line;
        while start > 0 && !self.line(start - 1).trim().is_empty() {
            start -= 1;
        }

        if self.cursor != self.rope.line_to_char(start) {
            self.cursor = self.rope.line_to_char(start);
            return;
        }

        let mut previous = start.saturating_sub(1);
        while previous > 0 && self.line(previous).trim().is_empty() {
            previous -= 1;
        }
        while previous > 0 && !self.line(previous - 1).trim().is_empty() {
            previous -= 1;
        }
        self.cursor = self.rope.line_to_char(previous);
    }

    /// End of the current paragraph, or of the next one when already at the end
    /// (Ctrl+Down).
    pub fn move_paragraph_down(&mut self) {
        let (line, _) = self.line_col();
        let last = self.rope.len_lines().saturating_sub(1);
        let mut end = line;
        while end < last && !self.line(end + 1).trim().is_empty() {
            end += 1;
        }

        let end_index = self.rope.line_to_char(end) + self.line_content_len(end);
        if self.cursor < end_index {
            self.cursor = end_index;
            return;
        }

        let mut next = (end + 1).min(last);
        while next < last && self.line(next).trim().is_empty() {
            next += 1;
        }
        while next < last && !self.line(next + 1).trim().is_empty() {
            next += 1;
        }
        self.cursor = self.rope.line_to_char(next) + self.line_content_len(next);
    }

    /// Move to the very start of the document (Ctrl+Home).
    pub fn move_document_start(&mut self) {
        self.cursor = 0;
    }

    /// Move to the very end of the document (Ctrl+End).
    pub fn move_document_end(&mut self) {
        self.cursor = self.rope.len_chars();
    }

    /// Delete from the start of the previous word to the cursor (Ctrl+Backspace).
    pub fn delete_word_left(&mut self) -> bool {
        let start = self.word_left_from(self.cursor);
        if start == self.cursor {
            return false;
        }
        self.replace(start..self.cursor, "");
        true
    }

    /// Delete from the cursor to the start of the next word (Ctrl+Delete).
    pub fn delete_word_right(&mut self) -> bool {
        let end = self.word_right_from(self.cursor);
        if end == self.cursor {
            return false;
        }
        self.replace(self.cursor..end, "");
        true
    }

    fn char_at(&self, index: usize) -> Option<char> {
        (index < self.rope.len_chars()).then(|| self.rope.char(index))
    }

    /// The start of the word before `from`, for word-wise deletion and Vim motions.
    pub fn word_left_at(&self, from: usize) -> usize {
        self.word_left_from(from)
    }

    /// The start of the word after `from`.
    pub fn word_right_at(&self, from: usize) -> usize {
        self.word_right_from(from)
    }

    fn word_left_from(&self, from: usize) -> usize {
        let mut index = from.min(self.rope.len_chars());
        while index > 0 && self.char_at(index - 1).is_some_and(|ch| ch.is_whitespace()) {
            index -= 1;
        }
        while index > 0
            && self
                .char_at(index - 1)
                .is_some_and(|ch| !ch.is_whitespace())
        {
            index -= 1;
        }
        index
    }

    fn word_right_from(&self, from: usize) -> usize {
        let len = self.rope.len_chars();
        let mut index = from.min(len);
        while index < len && self.char_at(index).is_some_and(|ch| ch.is_whitespace()) {
            index += 1;
        }
        while index < len && self.char_at(index).is_some_and(|ch| !ch.is_whitespace()) {
            index += 1;
        }
        index
    }

    pub fn undo(&mut self) -> bool {
        let Some(edit) = self.undo.pop() else {
            return false;
        };
        self.revert(&edit);
        self.redo.push(edit);
        self.revision = self.revision.wrapping_add(1);
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(edit) = self.redo.pop() else {
            return false;
        };
        self.apply(&edit);
        self.undo.push(edit);
        self.revision = self.revision.wrapping_add(1);
        true
    }

    /// Revert one stored edit. Batches are reverted bottom-up so offsets stay valid.
    fn revert(&mut self, edit: &Edit) {
        match edit {
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
            Edit::Batch(edits) => {
                for edit in edits.iter().rev() {
                    self.revert(edit);
                }
            }
        }
    }

    /// Re-apply one stored edit. Batches are applied in their original order.
    fn apply(&mut self, edit: &Edit) {
        match edit {
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
            Edit::Batch(edits) => {
                for edit in edits {
                    self.apply(edit);
                }
            }
        }
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

        // Undoing back to the saved bytes is clean, even though a revision passed.
        // (Inserting at 0 rather than at the caret keeps it a separate undo step; a typing
        // burst at the caret coalesces into one.)
        b.set_cursor(0);
        b.insert("y");
        assert!(b.is_dirty());
        b.undo();
        assert_eq!(b.text(), "x");
        assert!(!b.is_dirty());

        // Typing and deleting the same text is clean too.
        b.insert("z");
        b.backspace();
        assert!(!b.is_dirty());
    }

    #[test]
    fn dirty_state_matches_the_bytes_through_random_edits_and_undos() {
        let mut state = 0x9e37_79b9_7f4a_7c15u64;
        let mut rand = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };

        let mut b = Buffer::from_text("alpha beta");
        let mut saved = b.text();
        for _ in 0..200 {
            match rand() % 4 {
                0 => {
                    let at = (rand() as usize) % (b.len_chars() + 1);
                    b.set_cursor(at);
                    b.insert("xy");
                }
                1 => {
                    b.backspace();
                }
                2 => {
                    b.undo();
                }
                _ => {
                    b.redo();
                }
            }
            if rand() % 7 == 0 {
                b.mark_saved();
                saved = b.text();
            }
            assert_eq!(
                b.is_dirty(),
                b.text() != saved,
                "dirty must follow the bytes: {:?}",
                b.text()
            );
        }
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
    fn replace_many_is_one_undo_step() {
        let mut b = Buffer::from_text("one two one");
        let count = b.replace_many(vec![(0..3, "1".to_string()), (8..11, "1".to_string())]);
        assert_eq!(count, 2);
        assert_eq!(b.text(), "1 two 1");
        assert!(b.undo());
        assert_eq!(b.text(), "one two one");
        assert!(b.redo());
        assert_eq!(b.text(), "1 two 1");
    }

    #[test]
    fn replace_many_ignores_overlaps_and_noops() {
        let mut b = Buffer::from_text("abcdef");
        let count = b.replace_many(vec![
            (1..4, "X".to_string()),
            (2..5, "Y".to_string()),
            (6..6, String::new()),
        ]);
        assert_eq!(count, 1);
        assert_eq!(b.text(), "aXef");
        assert!(b.undo());
        assert_eq!(b.text(), "abcdef");
    }

    #[test]
    fn replace_many_deduplicates_identical_carets() {
        let mut b = Buffer::from_text("ab");
        let count = b.replace_many(vec![(1..1, "X".to_string()), (1..1, "X".to_string())]);
        assert_eq!(count, 1);
        assert_eq!(b.text(), "aXb");
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
    fn line_eq_ignores_the_newline() {
        let b = Buffer::from_text("hello\nhi\r\nbye");
        assert!(b.line_eq(0, "hello"));
        assert!(b.line_eq(1, "hi"));
        assert!(b.line_eq(2, "bye"));
        assert!(b.line_eq(3, ""));
        assert!(!b.line_eq(0, "hell"));
        assert!(!b.line_eq(0, "helloo"));
        assert!(!b.line_eq(0, "Hello"));
        assert!(!b.line_eq(9, "anything"));
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

    #[test]
    fn word_movement_skips_whitespace_and_words() {
        let mut b = Buffer::from_text("one two  three");
        b.set_cursor(0);
        b.move_word_right();
        assert_eq!(b.cursor(), 3);
        b.move_word_right();
        assert_eq!(b.cursor(), 7);
        b.move_word_right();
        assert_eq!(b.cursor(), 14);

        b.move_word_left();
        assert_eq!(b.cursor(), 9);
        b.move_word_left();
        assert_eq!(b.cursor(), 4);
        b.move_word_left();
        assert_eq!(b.cursor(), 0);
    }

    #[test]
    fn word_movement_crosses_line_breaks() {
        let mut b = Buffer::from_text("alpha\nbeta");
        b.set_cursor(3);
        b.move_word_right();
        assert_eq!(b.cursor(), 5);
        b.set_cursor(6);
        b.move_word_left();
        assert_eq!(b.cursor(), 0);
    }

    #[test]
    fn paragraph_movement_stops_at_blank_lines() {
        let mut b = Buffer::from_text("one line\ntwo line\n\nnext para\nstill next");
        b.set_cursor(2);
        b.move_paragraph_down();
        assert_eq!(b.line_col(), (1, 8));
        b.move_paragraph_down();
        assert_eq!(b.line_col(), (4, 10));
        b.move_paragraph_up();
        assert_eq!(b.line_col(), (3, 0));
        b.move_paragraph_up();
        assert_eq!(b.line_col(), (0, 0));
    }

    #[test]
    fn document_start_and_end() {
        let mut b = Buffer::from_text("abc\ndef");
        b.move_document_end();
        assert_eq!(b.cursor(), 7);
        b.move_document_start();
        assert_eq!(b.cursor(), 0);
    }

    #[test]
    fn word_deletion_removes_one_word_at_a_time() {
        let mut b = Buffer::from_text("one two three");
        b.set_cursor(13);
        assert!(b.delete_word_left());
        assert_eq!(b.text(), "one two ");
        b.set_cursor(4);
        assert!(b.delete_word_right());
        assert_eq!(b.text(), "one  ");
        b.move_document_end();
        assert!(!b.delete_word_right());
    }
}
