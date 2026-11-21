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
    /// Anchor and head of a cell range. The head is the active cell.
    pub anchor_row: usize,
    pub anchor_col: usize,
    /// True while a mouse drag is extending the selection.
    pub selecting: bool,
    /// A whole row or column selected from its grip.
    pub whole: Option<TableAxis>,
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
            anchor_row: 0,
            anchor_col: 0,
            selecting: false,
            whole: None,
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
        self.anchor_row = self.active_row;
        self.anchor_col = self.active_col;
        self.whole = None;
        self.selecting = false;
        self.cursor = self.active_text().chars().count();
        self.marked = None;
    }

    /// Begin a selection at a cell. Shift extends from the existing anchor.
    pub fn begin_selection(&mut self, row: usize, col: usize, extend: bool) {
        let row = row.min(self.row_count().saturating_sub(1));
        let col = col.min(self.column_count().saturating_sub(1));
        if !extend {
            self.anchor_row = row;
            self.anchor_col = col;
        }
        self.active_row = row;
        self.active_col = col;
        self.whole = None;
        self.selecting = true;
        self.cursor = self.active_text().chars().count();
    }

    /// Extend the current selection to another cell.
    pub fn extend_selection(&mut self, row: usize, col: usize) {
        if !self.selecting {
            return;
        }
        self.active_row = row.min(self.row_count().saturating_sub(1));
        self.active_col = col.min(self.column_count().saturating_sub(1));
        self.whole = None;
        self.cursor = self.active_text().chars().count();
    }

    pub fn finish_selection(&mut self) {
        self.selecting = false;
    }

    /// Select every cell in a body row (the header row cannot be removed this way).
    pub fn select_row(&mut self, row: usize) {
        let row = row.min(self.row_count().saturating_sub(1));
        self.anchor_row = row;
        self.anchor_col = 0;
        self.active_row = row;
        self.active_col = 0;
        self.whole = Some(TableAxis::Row);
        self.selecting = false;
        self.cursor = self.active_text().chars().count();
    }

    /// Select every cell in a column, including the header cell.
    pub fn select_column(&mut self, col: usize) {
        let col = col.min(self.column_count().saturating_sub(1));
        self.anchor_row = 0;
        self.anchor_col = col;
        self.active_row = if self.rows.is_empty() { 0 } else { 1 };
        self.active_col = col;
        self.whole = Some(TableAxis::Column);
        self.selecting = false;
        self.cursor = self.active_text().chars().count();
    }

    /// Whether more than one cell, a whole row, or a whole column is selected.
    pub fn has_range(&self) -> bool {
        self.whole.is_some()
            || self.anchor_row != self.active_row
            || self.anchor_col != self.active_col
    }

    /// Inclusive `(top, left, bottom, right)` cell bounds of the selection.
    pub fn selection_bounds(&self) -> (usize, usize, usize, usize) {
        let top = self.anchor_row.min(self.active_row);
        let bottom = self.anchor_row.max(self.active_row);
        let left = self.anchor_col.min(self.active_col);
        let right = self.anchor_col.max(self.active_col);
        // A whole row or column always spans the table on the other axis.
        match self.whole {
            Some(TableAxis::Row) => (top, 0, bottom, self.column_count().saturating_sub(1)),
            Some(TableAxis::Column) => (0, left, self.row_count().saturating_sub(1), right),
            None => (top, left, bottom, right),
        }
    }

    pub fn is_selected(&self, row: usize, col: usize) -> bool {
        let (top, left, bottom, right) = self.selection_bounds();
        row >= top && row <= bottom && col >= left && col <= right
    }

    /// Clear every selected cell and collapse the selection onto the anchor.
    pub fn clear_selected(&mut self) {
        let (top, left, bottom, right) = self.selection_bounds();
        for row in top..=bottom {
            for col in left..=right {
                if let Some(cell) = self.cell_mut(row, col) {
                    cell.clear();
                }
            }
        }
        self.collapse_to_anchor();
    }

    /// The erase key: remove selected rows/columns, otherwise clear selected cells.
    pub fn delete_key(&mut self) {
        let (top, left, bottom, right) = self.selection_bounds();
        match self.whole {
            Some(TableAxis::Row) => self.delete_rows(top, bottom),
            Some(TableAxis::Column) => self.delete_columns(left, right),
            None => self.clear_selected(),
        }
        // If a structural delete was impossible (for example, all columns selected),
        // fall back to clearing the cells so every erase does something visible.
        if self.whole.is_some() {
            self.clear_selected();
        }
    }

    fn delete_rows(&mut self, start: usize, end: usize) {
        let start = start.max(1);
        let end = end.min(self.rows.len());
        if start > end {
            return;
        }
        self.rows.drain(start - 1..=end - 1);
        self.active_row = start.min(self.rows.len());
        self.active_col = self.active_col.min(self.column_count().saturating_sub(1));
        self.collapse_to_active();
    }

    fn delete_columns(&mut self, start: usize, end: usize) {
        if self.column_count() <= 1 || end < start || end >= self.column_count() {
            return;
        }
        self.headers.drain(start..=end);
        for row in &mut self.rows {
            row.drain(start..=end);
        }
        self.align.drain(start..=end);
        self.active_col = start.min(self.column_count().saturating_sub(1));
        self.active_row = self.active_row.min(self.row_count().saturating_sub(1));
        self.collapse_to_active();
    }

    /// Collapse the selection onto the active cell.
    fn collapse_to_active(&mut self) {
        self.anchor_row = self.active_row;
        self.anchor_col = self.active_col;
        self.whole = None;
        self.selecting = false;
        self.cursor = self.cursor.min(self.active_text().chars().count());
    }

    /// Collapse onto the top-left cell of a range, used before typing replaces it.
    fn collapse_to_anchor(&mut self) {
        let (top, left, ..) = self.selection_bounds();
        self.active_row = top;
        self.active_col = left;
        self.anchor_row = top;
        self.anchor_col = left;
        self.whole = None;
        self.selecting = false;
        self.cursor = 0;
    }

    /// Select every cell in the table.
    pub fn select_all(&mut self) {
        self.anchor_row = 0;
        self.anchor_col = 0;
        self.active_row = self.row_count().saturating_sub(1);
        self.active_col = self.column_count().saturating_sub(1);
        self.whole = None;
        self.selecting = false;
        self.cursor = self.active_text().chars().count();
    }

    /// Copy the selected cells as tab-separated rows.
    pub fn copy_text(&self) -> String {
        let (top, left, bottom, right) = self.selection_bounds();
        let mut out = String::new();
        for row in top..=bottom {
            for col in left..=right {
                if col > left {
                    out.push('\t');
                }
                out.push_str(self.cell(row, col).map(String::as_str).unwrap_or(""));
            }
            if row < bottom {
                out.push('\n');
            }
        }
        out
    }

    /// Paste a tab/newline matrix starting at the selection's top-left cell, growing the
    /// table when the clipboard is larger than the current grid.
    pub fn paste_text(&mut self, text: &str) {
        let mut matrix: Vec<Vec<String>> = text
            .lines()
            .map(|line| line.split('\t').map(|cell| cell.to_string()).collect())
            .collect();
        if matrix.is_empty() {
            return;
        }
        let width = matrix.iter().map(Vec::len).max().unwrap_or(0);
        if width == 0 {
            return;
        }
        for row in &mut matrix {
            row.resize(width, String::new());
        }

        let (top, left, ..) = self.selection_bounds();
        let needed_rows = top + matrix.len();
        while self.row_count() < needed_rows {
            let width = self.column_count().max(1);
            self.rows.push(vec![String::new(); width]);
        }
        let needed_columns = left + width;
        while self.column_count() < needed_columns {
            self.headers.push(String::new());
            for row in &mut self.rows {
                row.push(String::new());
            }
            self.align.push(Align::Left);
        }

        for (row_offset, cells) in matrix.iter().enumerate() {
            for (col_offset, value) in cells.iter().enumerate() {
                if let Some(cell) = self.cell_mut(top + row_offset, left + col_offset) {
                    *cell = value.replace(['\n', '\r'], " ");
                }
            }
        }

        self.anchor_row = top;
        self.anchor_col = left;
        self.active_row = top + matrix.len().saturating_sub(1);
        self.active_col = left + width.saturating_sub(1);
        self.whole = None;
        self.selecting = false;
        self.cursor = self.active_text().chars().count();
    }

    #[cfg(test)]
    pub fn insert_text(&mut self, text: &str) {
        self.prepare_typing();
        self.marked = None;
        self.replace_at_cursor(text);
    }

    /// Typing over a multi-cell selection replaces it, starting at the selection anchor.
    fn prepare_typing(&mut self) {
        if self.has_range() {
            self.clear_selected();
        }
    }

    /// Replace the current IME composition with `text` and mark the result.
    pub fn mark_text(&mut self, text: &str) {
        self.prepare_typing();
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
        self.prepare_typing();
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
        if self.whole.is_some() {
            self.delete_key();
            return;
        }
        if self.has_range() {
            self.clear_selected();
            return;
        }
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

    /// Delete the word before the caret, or clear a cell selection.
    pub fn delete_word_left(&mut self) {
        if self.has_range() {
            self.clear_selected();
            return;
        }
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

    /// Delete the word after the caret, or clear a cell selection.
    pub fn delete_word_right(&mut self) {
        if self.has_range() {
            self.clear_selected();
            return;
        }
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
        self.collapse_to_active();
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
        self.collapse_to_active();
    }

    pub fn move_up(&mut self) {
        if self.active_row > 0 {
            self.active_row -= 1;
            self.clamp_cursor();
        }
        self.collapse_to_active();
    }

    pub fn move_down(&mut self) {
        if self.active_row + 1 < self.row_count() {
            self.active_row += 1;
            self.clamp_cursor();
        }
        self.collapse_to_active();
    }

    pub fn move_home(&mut self) {
        self.cursor = 0;
        self.collapse_to_active();
    }

    pub fn move_end(&mut self) {
        self.cursor = self.active_text().chars().count();
        self.collapse_to_active();
    }

    pub fn move_page(&mut self, delta: isize) {
        let last = self.row_count().saturating_sub(1);
        let row = (self.active_row as isize + delta * 5).clamp(0, last as isize) as usize;
        self.active_row = row;
        self.clamp_cursor();
        self.collapse_to_active();
    }

    pub fn move_start(&mut self) {
        self.active_row = 0;
        self.active_col = 0;
        self.cursor = 0;
        self.collapse_to_active();
    }

    pub fn move_finish(&mut self) {
        self.active_row = self.row_count().saturating_sub(1);
        self.active_col = self.column_count().saturating_sub(1);
        self.cursor = self.active_text().chars().count();
        self.collapse_to_active();
    }

    /// Extend the cell selection one cell left.
    pub fn extend_left(&mut self) {
        if let Some((row, col)) = self.previous_cell_from() {
            self.active_row = row;
            self.active_col = col;
            self.cursor = self.active_text().chars().count();
            self.whole = None;
            self.selecting = true;
        }
    }

    /// Extend the cell selection one cell right.
    pub fn extend_right(&mut self) {
        if let Some((row, col)) = self.next_cell_from() {
            self.active_row = row;
            self.active_col = col;
            self.cursor = self.active_text().chars().count();
            self.whole = None;
            self.selecting = true;
        }
    }

    /// Extend the cell selection one row up.
    pub fn extend_up(&mut self) {
        if self.active_row > 0 {
            self.active_row -= 1;
            self.cursor = self.active_text().chars().count();
            self.whole = None;
            self.selecting = true;
        }
    }

    /// Extend the cell selection one row down.
    pub fn extend_down(&mut self) {
        if self.active_row + 1 < self.row_count() {
            self.active_row += 1;
            self.cursor = self.active_text().chars().count();
            self.whole = None;
            self.selecting = true;
        }
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
        self.collapse_to_active();
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
        self.collapse_to_active();
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

    /// Add an empty row at the bottom.
    pub fn add_row(&mut self) {
        let width = self.column_count().max(1);
        self.rows.push(vec![String::new(); width]);
        self.active_row = self.rows.len();
        self.active_col = self.active_col.min(width.saturating_sub(1));
        self.cursor = 0;
        self.collapse_to_active();
    }

    /// Add an empty column at the far right.
    pub fn add_column(&mut self) {
        let at = self.headers.len();
        self.headers.push(String::new());
        for row in &mut self.rows {
            row.push(String::new());
        }
        self.align.push(Align::Left);
        self.active_col = at;
        self.cursor = 0;
        self.collapse_to_active();
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
        if self.whole == Some(TableAxis::Row) {
            self.anchor_row = self.active_row;
            self.anchor_col = 0;
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
        if self.whole == Some(TableAxis::Column) {
            self.anchor_col = self.active_col;
            self.anchor_row = 0;
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
        assert_eq!(table.headers, vec!["A", "B", ""]);
        assert_eq!(table.rows[0], vec!["1", "2", ""]);
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
        table.delete_key();
        assert_eq!(table.rows[0][0], "");
    }

    #[test]
    fn selects_and_copies_a_range_then_clears_it() {
        let mut table = state();
        table.begin_selection(0, 0, false);
        table.extend_selection(1, 1);
        assert_eq!(table.copy_text(), "A\tB\n1\t2");
        table.delete_key();
        assert_eq!(table.headers, vec!["", ""]);
        assert_eq!(table.rows[0], vec!["", ""]);
    }

    #[test]
    fn erase_removes_selected_rows_and_columns() {
        let mut table = state();
        table.select_row(1);
        table.delete_key();
        assert!(table.rows.is_empty());
        assert_eq!(table.headers, vec!["A", "B"]);

        let mut table = state();
        table.select_column(0);
        table.delete_key();
        assert_eq!(table.headers, vec!["B"]);
        assert_eq!(table.rows[0], vec!["2"]);
        assert_eq!(table.align, vec![Align::Left]);
    }

    #[test]
    fn typing_replaces_a_range_and_paste_fills_a_matrix() {
        let mut table = state();
        table.begin_selection(0, 0, false);
        table.extend_selection(0, 1);
        table.insert_text("x");
        assert_eq!(table.headers, vec!["x", ""]);

        table.activate(1, 0);
        table.paste_text("p\tq\nr\ts");
        assert_eq!(table.rows[0], vec!["p", "q"]);
        assert_eq!(table.rows[1], vec!["r", "s"]);
        assert_eq!(table.anchor_row, 1);
        assert_eq!(table.anchor_col, 0);
        assert_eq!(table.active_row, 2);
        assert_eq!(table.active_col, 1);
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
