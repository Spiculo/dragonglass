//! State for editing a GFM table as a grid in Dynamic mode.
//!
//! The editor keeps the authoritative text in its `Buffer`; this state mirrors one table's
//! cells while the overlay is open. Every change regenerates the table's markdown and
//! replaces exactly those lines in the buffer, so drafts, autosave, undo and the split
//! preview all see a normal edit.

use std::ops::Range;

use dragonglass_core::markdown::{self, Align, Block};

/// Which axis a drag is reordering.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TableAxis {
    Row,
    Column,
}

/// A drag in progress between two rows or two columns.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TableDrag {
    pub axis: TableAxis,
    /// Body row (1-based, header is 0) or column index the drag started on.
    pub from: usize,
    /// Row or column currently under the pointer.
    pub to: usize,
}

/// One table being edited in place.
#[derive(Clone, Debug)]
pub(crate) struct TableEdit {
    /// Source lines of the table, 0-based and inclusive.
    pub first_line: usize,
    pub last_line: usize,
    /// Position among the document's tables, for finding the painted row range.
    pub table_ordinal: usize,
    pub align: Vec<Align>,
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
    /// Row 0 is the header; body rows are 1..=rows.len().
    pub active_row: usize,
    pub active_col: usize,
    /// Caret offset in characters within the active cell.
    pub cursor: usize,
    /// IME composition range within the active cell, in characters.
    pub marked: Option<Range<usize>>,
    /// Buffer revision the state mirrors. A mismatch means the text moved under us.
    pub revision: u64,
    pub drag: Option<TableDrag>,
}

impl TableEdit {
    /// Parse a table from its own source lines.
    pub fn from_lines(
        first_line: usize,
        lines: &[&str],
        table_ordinal: usize,
        revision: u64,
    ) -> Option<Self> {
        let Some(Block::Table {
            align,
            headers,
            rows,
        }) = markdown::parse_table(lines)
        else {
            return None;
        };
        Some(Self {
            first_line,
            last_line: first_line + lines.len().saturating_sub(1),
            table_ordinal,
            align,
            headers,
            rows,
            active_row: 0,
            active_col: 0,
            cursor: 0,
            marked: None,
            revision,
            drag: None,
        })
    }

    pub fn column_count(&self) -> usize {
        self.headers.len()
    }

    pub fn row_count(&self) -> usize {
        self.rows.len() + 1
    }

    pub fn cell(&self, row: usize, col: usize) -> Option<&String> {
        if row == 0 {
            self.headers.get(col)
        } else {
            self.rows.get(row - 1).and_then(|cells| cells.get(col))
        }
    }

    fn cell_mut(&mut self, row: usize, col: usize) -> Option<&mut String> {
        if row == 0 {
            self.headers.get_mut(col)
        } else {
            self.rows
                .get_mut(row - 1)
                .and_then(|cells| cells.get_mut(col))
        }
    }

    fn active_text(&self) -> &str {
        self.cell(self.active_row, self.active_col)
            .map(String::as_str)
            .unwrap_or("")
    }

    fn active_text_mut(&mut self) -> Option<&mut String> {
        let (row, col) = (self.active_row, self.active_col);
        self.cell_mut(row, col)
    }

    pub fn activate(&mut self, row: usize, col: usize) {
        self.active_row = row.min(self.row_count().saturating_sub(1));
        self.active_col = col.min(self.column_count().saturating_sub(1));
        self.cursor = self.active_text().chars().count();
        self.marked = None;
    }

    pub fn insert_text(&mut self, text: &str) {
        self.marked = None;
        self.replace_at_cursor(text);
    }

    /// Replace the current IME composition with `text` and mark the result.
    pub fn mark_text(&mut self, text: &str) {
        if let Some(range) = self.marked.take() {
            self.remove_chars(range.clone());
            self.cursor = range.start;
        }
        let before = self.cursor;
        self.replace_at_cursor(text);
        let len = text.chars().count();
        self.marked = Some(before..before + len);
    }

    /// Finish an IME composition, or insert `text` when none is active.
    pub fn finish_text(&mut self, text: &str) {
        if let Some(range) = self.marked.take() {
            self.remove_chars(range.clone());
            self.cursor = range.start;
        }
        self.replace_at_cursor(text);
    }

    pub fn unmark(&mut self) {
        self.marked = None;
    }

    fn replace_at_cursor(&mut self, text: &str) {
        let cleaned: String = text
            .chars()
            .map(|ch| if ch == '\n' || ch == '\r' { ' ' } else { ch })
            .collect();
        if cleaned.is_empty() {
            return;
        }
        let cursor = self.cursor;
        if let Some(cell) = self.active_text_mut() {
            let byte = char_to_byte(cell, cursor);
            cell.insert_str(byte, &cleaned);
        }
        self.cursor = cursor + cleaned.chars().count();
    }

    fn remove_chars(&mut self, range: Range<usize>) {
        if range.is_empty() {
            return;
        }
        if let Some(cell) = self.active_text_mut() {
            let len = cell.chars().count();
            let start = range.start.min(len);
            let end = range.end.min(len).max(start);
            let start_byte = char_to_byte(cell, start);
            let end_byte = char_to_byte(cell, end);
            cell.replace_range(start_byte..end_byte, "");
        }
    }

    pub fn backspace(&mut self) {
        self.marked = None;
        if self.cursor == 0 {
            return;
        }
        let at = self.cursor - 1;
        if let Some(cell) = self.active_text_mut() {
            let start = char_to_byte(cell, at);
            let end = char_to_byte(cell, at + 1);
            cell.replace_range(start..end, "");
        }
        self.cursor = at;
    }

    pub fn delete(&mut self) {
        self.marked = None;
        let at = self.cursor;
        let len = self.active_text().chars().count();
        if at >= len {
            return;
        }
        if let Some(cell) = self.active_text_mut() {
            let start = char_to_byte(cell, at);
            let end = char_to_byte(cell, at + 1);
            cell.replace_range(start..end, "");
        }
    }

    /// Delete the word before the caret.
    pub fn delete_word_left(&mut self) {
        let chars: Vec<char> = self.active_text().chars().collect();
        let mut at = self.cursor.min(chars.len());
        if at == 0 {
            return;
        }
        while at > 0 && chars[at - 1].is_whitespace() {
            at -= 1;
        }
        while at > 0 && !chars[at - 1].is_whitespace() {
            at -= 1;
        }
        let start = at;
        let end = self.cursor.min(chars.len());
        if let Some(cell) = self.active_text_mut() {
            let start_byte = char_to_byte(cell, start);
            let end_byte = char_to_byte(cell, end);
            cell.replace_range(start_byte..end_byte, "");
        }
        self.cursor = start;
    }

    /// Delete the word after the caret.
    pub fn delete_word_right(&mut self) {
        let chars: Vec<char> = self.active_text().chars().collect();
        let start = self.cursor.min(chars.len());
        let mut at = start;
        while at < chars.len() && chars[at].is_whitespace() {
            at += 1;
        }
        while at < chars.len() && !chars[at].is_whitespace() {
            at += 1;
        }
        if let Some(cell) = self.active_text_mut() {
            let start_byte = char_to_byte(cell, start);
            let end_byte = char_to_byte(cell, at);
            cell.replace_range(start_byte..end_byte, "");
        }
    }

    pub fn move_left(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
        } else if let Some((row, col)) = self.previous_cell_from() {
            self.active_row = row;
            self.active_col = col;
            self.cursor = self.active_text().chars().count();
        }
    }

    pub fn move_right(&mut self) {
        let len = self.active_text().chars().count();
        if self.cursor < len {
            self.cursor += 1;
        } else if let Some((row, col)) = self.next_cell_from() {
            self.active_row = row;
            self.active_col = col;
            self.cursor = 0;
        }
    }

    pub fn move_up(&mut self) {
        if self.active_row > 0 {
            self.active_row -= 1;
            self.clamp_cursor();
        }
    }

    pub fn move_down(&mut self) {
        if self.active_row + 1 < self.row_count() {
            self.active_row += 1;
            self.clamp_cursor();
        }
    }

    pub fn move_home(&mut self) {
        self.cursor = 0;
    }

    pub fn move_end(&mut self) {
        self.cursor = self.active_text().chars().count();
    }

    pub fn move_page(&mut self, delta: isize) {
        let last = self.row_count().saturating_sub(1);
        let row = (self.active_row as isize + delta * 5).clamp(0, last as isize) as usize;
        self.active_row = row;
        self.clamp_cursor();
    }

    pub fn move_start(&mut self) {
        self.active_row = 0;
        self.active_col = 0;
        self.cursor = 0;
    }

    pub fn move_finish(&mut self) {
        self.active_row = self.row_count().saturating_sub(1);
        self.active_col = self.column_count().saturating_sub(1);
        self.cursor = self.active_text().chars().count();
    }

    /// Move to the next cell, wrapping to the next row.
    pub fn next_cell(&mut self) {
        let mut row = self.active_row;
        let mut col = self.active_col + 1;
        if col >= self.column_count().max(1) {
            col = 0;
            row += 1;
        }
        if row < self.row_count() {
            self.active_row = row;
            self.active_col = col;
            self.cursor = self.active_text().chars().count();
        }
    }

    /// Move to the previous cell, wrapping to the previous row.
    pub fn previous_cell(&mut self) {
        let mut row = self.active_row;
        let mut col = self.active_col;
        if col == 0 {
            if row == 0 {
                return;
            }
            row -= 1;
            col = self.column_count().saturating_sub(1);
        } else {
            col -= 1;
        }
        self.active_row = row;
        self.active_col = col;
        self.cursor = self.active_text().chars().count();
    }

    fn previous_cell_from(&self) -> Option<(usize, usize)> {
        let mut row = self.active_row;
        let mut col = self.active_col;
        if col == 0 {
            if row == 0 {
                return None;
            }
            row -= 1;
            col = self.column_count().saturating_sub(1);
        } else {
            col -= 1;
        }
        Some((row, col))
    }

    fn next_cell_from(&self) -> Option<(usize, usize)> {
        let mut row = self.active_row;
        let mut col = self.active_col + 1;
        if col >= self.column_count() {
            col = 0;
            row += 1;
        }
        (row < self.row_count()).then_some((row, col))
    }

    /// Add an empty row: after the active body row, or at the end when the header is active.
    pub fn add_row(&mut self) {
        let width = self.column_count().max(1);
        let at = if self.active_row == 0 {
            self.rows.len()
        } else {
            self.active_row.min(self.rows.len())
        };
        self.rows.insert(at, vec![String::new(); width]);
        self.active_row = at + 1;
        self.active_col = self.active_col.min(width.saturating_sub(1));
        self.cursor = 0;
    }

    /// Add an empty column to the right of the active column.
    pub fn add_column(&mut self) {
        let at = (self.active_col + 1).min(self.headers.len());
        self.headers.insert(at, String::new());
        for row in &mut self.rows {
            row.insert(at, String::new());
        }
        self.align.insert(at, Align::Left);
        self.active_col = at;
        self.cursor = 0;
    }

    /// Move a body row. Both indexes are 1-based (`1` is the first body row).
    pub fn move_row(&mut self, from: usize, to: usize) {
        if from == 0 || from > self.rows.len() || to == 0 || to > self.rows.len() {
            return;
        }
        if from == to {
            return;
        }
        let row = self.rows.remove(from - 1);
        self.rows.insert(to - 1, row);
        if self.active_row == from {
            self.active_row = to;
        }
    }

    pub fn move_column(&mut self, from: usize, to: usize) {
        if from >= self.column_count() || to >= self.column_count() || from == to {
            return;
        }
        self.headers.swap(from, to);
        for row in &mut self.rows {
            row.swap(from, to);
        }
        self.align.swap(from, to);
        if self.active_col == from {
            self.active_col = to;
        }
    }

    /// The table as markdown lines, without a trailing newline.
    pub fn markdown(&self) -> String {
        let mut out = String::new();
        out.push_str(&format_row(&self.headers));
        out.push('\n');
        out.push_str(&format_row(
            &self
                .align
                .iter()
                .map(|align| match align {
                    Align::Left => "---".to_string(),
                    Align::Center => ":---:".to_string(),
                    Align::Right => "---:".to_string(),
                })
                .collect::<Vec<_>>(),
        ));
        for row in &self.rows {
            out.push('\n');
            out.push_str(&format_row(row));
        }
        out
    }

    fn clamp_cursor(&mut self) {
        self.cursor = self.cursor.min(self.active_text().chars().count());
    }
}

fn format_row(cells: &[String]) -> String {
    let joined = cells
        .iter()
        .map(|cell| {
            cell.trim()
                .replace('|', "\\|")
                .replace('\n', " ")
                .replace('\r', "")
        })
        .collect::<Vec<_>>()
        .join(" | ");
    format!("| {joined} |")
}

fn char_to_byte(text: &str, char_index: usize) -> usize {
    text.char_indices()
        .nth(char_index)
        .map(|(byte, _)| byte)
        .unwrap_or(text.len())
}

/// The column index of a character position in a pipe-table line.
///
/// Used when opening the overlay from a click: the pointer lands on a character, and the
/// number of unescaped pipes before it names the cell.
pub(crate) fn column_at_char(line: &str, char_col: usize) -> usize {
    let mut pipes = 0usize;
    let mut escaped = false;
    let mut in_code = false;
    for ch in line.chars().take(char_col) {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' => escaped = true,
            '`' => in_code = !in_code,
            '|' if !in_code => pipes += 1,
            _ => {}
        }
    }
    pipes.saturating_sub(usize::from(line.trim_start().starts_with('|')))
}

/// Whether `line` is part of a table (used to decide where a click should open the grid).
pub(crate) fn is_table_line(line: &str) -> bool {
    line.trim().contains('|')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> TableEdit {
        let lines = ["| A | B |", "| --- | --- |", "| 1 | 2 |"];
        TableEdit::from_lines(4, &lines, 0, 0).expect("table")
    }

    #[test]
    fn round_trips_markdown() {
        let table = state();
        assert_eq!(table.markdown(), "| A | B |\n| --- | --- |\n| 1 | 2 |");
    }

    #[test]
    fn adds_rows_and_columns() {
        let mut table = state();
        table.activate(0, 0);
        table.add_column();
        assert_eq!(table.headers, vec!["A", "", "B"]);
        assert_eq!(table.rows[0], vec!["1", "", "2"]);
        assert_eq!(table.align, vec![Align::Left, Align::Left, Align::Left]);

        table.add_row();
        assert_eq!(table.rows.len(), 2);
        assert_eq!(table.rows[1], vec!["", "", ""]);
    }

    #[test]
    fn edits_the_active_cell() {
        let mut table = state();
        table.activate(1, 0);
        assert_eq!(table.cursor, 1);
        table.insert_text("0");
        assert_eq!(table.rows[0][0], "10");
        table.backspace();
        assert_eq!(table.rows[0][0], "1");
        table.move_end();
        table.delete();
        assert_eq!(table.rows[0][0], "1");
    }

    #[test]
    fn reorders_rows_and_columns() {
        let mut table = state();
        table.rows.push(vec!["3".into(), "4".into()]);
        table.move_row(1, 2);
        assert_eq!(table.rows[0], vec!["3", "4"]);
        table.move_column(0, 1);
        assert_eq!(table.headers, vec!["B", "A"]);
        assert_eq!(table.rows[0], vec!["4", "3"]);
    }

    #[test]
    fn replaces_an_ime_composition_in_place() {
        let mut table = state();
        table.activate(1, 0);
        table.mark_text("x");
        assert_eq!(table.rows[0][0], "1x");
        table.mark_text("xy");
        assert_eq!(table.rows[0][0], "1xy");
        table.finish_text("xyz");
        assert_eq!(table.rows[0][0], "1xyz");
        assert!(table.marked.is_none());
    }

    #[test]
    fn finds_the_clicked_column() {
        assert_eq!(column_at_char("| A | B |", 2), 0);
        assert_eq!(column_at_char("| A | B |", 6), 1);
        assert_eq!(column_at_char("A | B", 0), 0);
        assert_eq!(column_at_char("A | B", 4), 1);
    }
}
