//! Optional Vim-style modal editing.
//!
//! This is deliberately a focused subset, not a Vim emulator: normal/insert/visual modes,
//! counts, the common motions and operators, `/` search, and a few `:` commands. It is
//! off by default and can be toggled per vault from the command palette.
//!
//! Printable keys arrive through `EntityInputHandler`, so only `handle_text` is needed for
//! commands; Enter, Backspace and Escape come in as editor actions via `handle_key`.

use dragonglass_core::{find, Buffer};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Insert,
    Visual,
    VisualLine,
    Command,
    Search,
}

impl Mode {
    pub fn label(self) -> &'static str {
        match self {
            Mode::Normal => "NORMAL",
            Mode::Insert => "INSERT",
            Mode::Visual => "VISUAL",
            Mode::VisualLine => "V-LINE",
            Mode::Command => "COMMAND",
            Mode::Search => "SEARCH",
        }
    }
}

/// What the editor should do after the key was handled.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Outcome {
    /// The key was consumed and the buffer may have changed.
    pub handled: bool,
    /// `:w` / `:wq` asked the editor to save.
    pub save: bool,
    /// A short message for the status bar.
    pub message: Option<String>,
}

impl Outcome {
    fn handled() -> Self {
        Self {
            handled: true,
            ..Self::default()
        }
    }

    fn message(message: impl Into<String>) -> Self {
        Self {
            handled: true,
            message: Some(message.into()),
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone)]
pub struct Vim {
    enabled: bool,
    mode: Mode,
    /// A pending operator (`d`, `y`, `c`), `r`, or the first `g` of `gg`.
    pending: Option<char>,
    /// Digits typed before a command.
    count: String,
    /// The `:` or `/` line being edited.
    command: String,
    last_search: Option<String>,
    register: String,
    register_linewise: bool,
    message: Option<String>,
}

impl Default for Vim {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: Mode::Normal,
            pending: None,
            count: String::new(),
            command: String::new(),
            last_search: None,
            register: String::new(),
            register_linewise: false,
            message: None,
        }
    }
}

impl Vim {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled {
            self.mode = Mode::Normal;
            self.pending = None;
            self.count.clear();
            self.command.clear();
        }
    }

    /// Return to normal mode without changing whether Vim is enabled.
    pub fn reset(&mut self) {
        self.mode = Mode::Normal;
        self.pending = None;
        self.count.clear();
        self.command.clear();
        self.message = None;
    }

    pub fn set_message(&mut self, message: Option<String>) {
        self.message = message;
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn status(&self) -> String {
        let mut status = self.mode.label().to_string();
        if !self.count.is_empty() {
            status.push(' ');
            status.push_str(&self.count);
        }
        if let Some(pending) = self.pending {
            status.push(' ');
            status.push(pending);
        }
        if matches!(self.mode, Mode::Command | Mode::Search) {
            let prefix = if self.mode == Mode::Command { ':' } else { '/' };
            status.push(' ');
            status.push(prefix);
            status.push_str(&self.command);
        }
        if let Some(message) = &self.message {
            status.push_str("  ·  ");
            status.push_str(message);
        }
        status
    }

    /// Consume printable text. In insert mode this returns `handled: false` so the editor
    /// performs its normal insertion.
    pub fn handle_text(
        &mut self,
        text: &str,
        buffer: &mut Buffer,
        anchor: &mut Option<usize>,
    ) -> Outcome {
        if !self.enabled || text.is_empty() {
            return Outcome::default();
        }

        if self.mode == Mode::Insert {
            return Outcome::default();
        }

        if matches!(self.mode, Mode::Command | Mode::Search) {
            for ch in text.chars() {
                if !ch.is_control() {
                    self.command.push(ch);
                }
            }
            self.message = None;
            return Outcome::handled();
        }

        let mut outcome = Outcome::default();
        for ch in text.chars() {
            let result = self.handle_char(ch, buffer, anchor);
            if result.handled {
                outcome.handled = true;
            }
            if result.save {
                outcome.save = true;
            }
            if let Some(message) = result.message {
                outcome.message = Some(message);
            }
        }
        outcome
    }

    /// Handle named keys that arrive as editor actions.
    pub fn handle_key(
        &mut self,
        key: &str,
        buffer: &mut Buffer,
        anchor: &mut Option<usize>,
    ) -> Outcome {
        if !self.enabled {
            return Outcome::default();
        }

        match key {
            "escape" => {
                let was_command = matches!(self.mode, Mode::Command | Mode::Search);
                self.pending = None;
                self.count.clear();
                self.command.clear();
                self.mode = Mode::Normal;
                if was_command {
                    self.message = None;
                    return Outcome::handled();
                }
                *anchor = None;
                Outcome::handled()
            }
            "backspace" => match self.mode {
                Mode::Command | Mode::Search => {
                    self.command.pop();
                    Outcome::handled()
                }
                Mode::Insert => Outcome::default(),
                _ => {
                    self.pending = None;
                    self.count.clear();
                    let cursor = buffer.cursor().saturating_sub(1);
                    if self.mode == Mode::Normal {
                        *anchor = None;
                    }
                    buffer.set_cursor(cursor);
                    Outcome::handled()
                }
            },
            "enter" => match self.mode {
                Mode::Command => self.execute_command(buffer, anchor),
                Mode::Search => self.execute_search(buffer, anchor),
                Mode::Insert => Outcome::default(),
                _ => {
                    self.pending = None;
                    self.count.clear();
                    *anchor = None;
                    buffer.move_down();
                    Outcome::handled()
                }
            },
            "delete" => {
                if self.mode == Mode::Insert {
                    Outcome::default()
                } else {
                    self.handle_char('x', buffer, anchor)
                }
            }
            _ => Outcome::default(),
        }
    }

    fn handle_char(
        &mut self,
        ch: char,
        buffer: &mut Buffer,
        anchor: &mut Option<usize>,
    ) -> Outcome {
        if matches!(self.mode, Mode::Command | Mode::Search) {
            if !ch.is_control() {
                self.command.push(ch);
            }
            return Outcome::handled();
        }

        self.message = None;

        // A pending `r` takes the next character literally.
        if self.pending == Some('r') {
            self.pending = None;
            self.count.clear();
            return self.replace_char(ch, buffer);
        }

        // `g` is only useful as `gg`; anything else cancels it.
        if self.pending == Some('g') {
            self.pending = None;
            if ch == 'g' {
                if self.count.is_empty() {
                    buffer.set_cursor(0);
                } else {
                    let line = self.count.parse::<usize>().unwrap_or(1).saturating_sub(1);
                    self.count.clear();
                    self.goto_line(buffer, line);
                }
                *anchor = None;
                return Outcome::handled();
            }
            return Outcome::handled();
        }

        if self.pending.is_some_and(|op| matches!(op, 'd' | 'y' | 'c')) {
            return self.apply_operator(ch, buffer, anchor);
        }

        if ch.is_ascii_digit() && !(ch == '0' && self.count.is_empty()) {
            self.count.push(ch);
            return Outcome::handled();
        }

        let count_text = std::mem::take(&mut self.count);
        let explicit_count = !count_text.is_empty();
        let count = count_text.parse::<usize>().unwrap_or(1);

        if self.mode == Mode::Visual || self.mode == Mode::VisualLine {
            return self.visual_key(ch, count, buffer, anchor);
        }

        match ch {
            'h' => self.move_cursor(buffer, anchor, |b| b.move_left(), count),
            'l' => self.move_cursor(buffer, anchor, |b| b.move_right(), count),
            'j' => self.move_cursor(buffer, anchor, |b| b.move_down(), count),
            'k' => self.move_cursor(buffer, anchor, |b| b.move_up(), count),
            'w' => {
                for _ in 0..count {
                    buffer.move_word_right();
                }
                *anchor = None;
                Outcome::handled()
            }
            'b' => {
                for _ in 0..count {
                    buffer.move_word_left();
                }
                *anchor = None;
                Outcome::handled()
            }
            'e' => {
                for _ in 0..count {
                    self.move_word_end(buffer);
                }
                *anchor = None;
                Outcome::handled()
            }
            '0' => {
                buffer.move_home();
                *anchor = None;
                Outcome::handled()
            }
            '^' => {
                self.first_non_blank(buffer);
                *anchor = None;
                Outcome::handled()
            }
            '$' => {
                buffer.move_end();
                *anchor = None;
                Outcome::handled()
            }
            'G' => {
                let line = if explicit_count {
                    count.saturating_sub(1)
                } else {
                    buffer.line_count().saturating_sub(1)
                };
                self.goto_line(buffer, line);
                *anchor = None;
                Outcome::handled()
            }
            'g' => {
                self.pending = Some('g');
                if explicit_count {
                    self.count = count.to_string();
                }
                Outcome::handled()
            }
            'i' => {
                *anchor = None;
                self.mode = Mode::Insert;
                Outcome::handled()
            }
            'a' => {
                if buffer.cursor() < buffer.len_chars() {
                    buffer.move_right();
                }
                *anchor = None;
                self.mode = Mode::Insert;
                Outcome::handled()
            }
            'I' => {
                self.first_non_blank(buffer);
                *anchor = None;
                self.mode = Mode::Insert;
                Outcome::handled()
            }
            'A' => {
                buffer.move_end();
                *anchor = None;
                self.mode = Mode::Insert;
                Outcome::handled()
            }
            'o' => {
                buffer.move_end();
                buffer.insert("\n");
                *anchor = None;
                self.mode = Mode::Insert;
                Outcome::handled()
            }
            'O' => {
                buffer.move_home();
                buffer.insert("\n");
                buffer.set_cursor(buffer.cursor().saturating_sub(1));
                *anchor = None;
                self.mode = Mode::Insert;
                Outcome::handled()
            }
            'v' => {
                *anchor = Some(buffer.cursor());
                self.mode = Mode::Visual;
                Outcome::handled()
            }
            'V' => {
                buffer.move_home();
                *anchor = Some(buffer.cursor());
                buffer.move_end();
                self.mode = Mode::VisualLine;
                Outcome::handled()
            }
            'x' => {
                for _ in 0..count {
                    self.delete_char(buffer);
                }
                *anchor = None;
                Outcome::handled()
            }
            'X' => {
                for _ in 0..count {
                    if buffer.cursor() > 0 {
                        let at = buffer.cursor() - 1;
                        buffer.replace(at..buffer.cursor(), "");
                    }
                }
                *anchor = None;
                Outcome::handled()
            }
            's' => {
                for _ in 0..count {
                    self.delete_char(buffer);
                }
                self.mode = Mode::Insert;
                Outcome::handled()
            }
            'S' => {
                buffer.move_home();
                let end = buffer.cursor() + buffer.line_char_len(buffer.line_col().0);
                buffer.replace(buffer.cursor()..end, "");
                self.mode = Mode::Insert;
                Outcome::handled()
            }
            'D' => {
                let (line, _) = buffer.line_col();
                let end = buffer.char_index_at(line, buffer.line_char_len(line));
                buffer.replace(buffer.cursor()..end, "");
                Outcome::handled()
            }
            'C' => {
                let (line, _) = buffer.line_col();
                let end = buffer.char_index_at(line, buffer.line_char_len(line));
                buffer.replace(buffer.cursor()..end, "");
                self.mode = Mode::Insert;
                Outcome::handled()
            }
            'p' => {
                if !self.register.is_empty() {
                    if self.register_linewise {
                        self.paste_linewise(buffer, true);
                    } else {
                        let at = (buffer.cursor() + 1).min(buffer.len_chars());
                        buffer.replace(at..at, &self.register.clone());
                    }
                }
                Outcome::handled()
            }
            'P' => {
                if !self.register.is_empty() {
                    if self.register_linewise {
                        self.paste_linewise(buffer, false);
                    } else {
                        let at = buffer.cursor();
                        buffer.replace(at..at, &self.register.clone());
                    }
                }
                Outcome::handled()
            }
            'u' => {
                buffer.undo();
                *anchor = None;
                Outcome::handled()
            }
            'J' => {
                let (line, _) = buffer.line_col();
                if line + 1 < buffer.line_count() {
                    let end = buffer.char_index_at(line, buffer.line_char_len(line));
                    let next = buffer.line(line + 1);
                    let indent = next.chars().take_while(|c| c.is_whitespace()).count();
                    let next_content = buffer.char_index_at(line + 1, indent);
                    buffer.replace(end..next_content, " ");
                }
                Outcome::handled()
            }
            'd' | 'y' | 'c' => {
                self.pending = Some(ch);
                if explicit_count {
                    self.count = count.to_string();
                }
                Outcome::handled()
            }
            'r' => {
                self.pending = Some('r');
                Outcome::handled()
            }
            '/' => {
                self.mode = Mode::Search;
                self.command.clear();
                Outcome::handled()
            }
            ':' => {
                self.mode = Mode::Command;
                self.command.clear();
                Outcome::handled()
            }
            'n' => {
                if let Some(pattern) = self.last_search.clone() {
                    self.search(&pattern, false, buffer);
                }
                Outcome::handled()
            }
            'N' => {
                if let Some(pattern) = self.last_search.clone() {
                    self.search(&pattern, true, buffer);
                }
                Outcome::handled()
            }
            _ => Outcome {
                handled: true,
                ..Outcome::default()
            },
        }
    }

    fn visual_key(
        &mut self,
        ch: char,
        count: usize,
        buffer: &mut Buffer,
        anchor: &mut Option<usize>,
    ) -> Outcome {
        match ch {
            'h' => {
                for _ in 0..count {
                    buffer.move_left();
                }
                Outcome::handled()
            }
            'l' => {
                for _ in 0..count {
                    buffer.move_right();
                }
                Outcome::handled()
            }
            'j' => {
                for _ in 0..count {
                    buffer.move_down();
                }
                Outcome::handled()
            }
            'k' => {
                for _ in 0..count {
                    buffer.move_up();
                }
                Outcome::handled()
            }
            'w' => {
                for _ in 0..count {
                    buffer.move_word_right();
                }
                Outcome::handled()
            }
            'b' => {
                for _ in 0..count {
                    buffer.move_word_left();
                }
                Outcome::handled()
            }
            'e' => {
                for _ in 0..count {
                    self.move_word_end(buffer);
                }
                Outcome::handled()
            }
            '0' => {
                buffer.move_home();
                Outcome::handled()
            }
            '$' => {
                buffer.move_end();
                Outcome::handled()
            }
            'G' => {
                let line = buffer.line_count().saturating_sub(1);
                let index = buffer.char_index_at(line, 0);
                buffer.set_cursor(index);
                Outcome::handled()
            }
            'd' | 'x' | 'y' | 'c' => {
                let range = self.visual_range(buffer, anchor);
                match ch {
                    'y' => {
                        self.register = buffer.slice(range);
                        self.register_linewise = self.mode == Mode::VisualLine;
                        self.mode = Mode::Normal;
                        *anchor = None;
                        Outcome::message(format!("yanked {} chars", self.register.chars().count()))
                    }
                    'c' => {
                        buffer.replace(range, "");
                        *anchor = None;
                        self.mode = Mode::Insert;
                        Outcome::handled()
                    }
                    _ => {
                        self.register = buffer.slice(range.clone());
                        self.register_linewise = self.mode == Mode::VisualLine;
                        buffer.replace(range, "");
                        *anchor = None;
                        self.mode = Mode::Normal;
                        Outcome::handled()
                    }
                }
            }
            'v' | 'V' => {
                self.mode = Mode::Normal;
                *anchor = None;
                Outcome::handled()
            }
            'p' => {
                let range = self.visual_range(buffer, anchor);
                if !self.register.is_empty() {
                    buffer.replace(range, &self.register.clone());
                }
                *anchor = None;
                self.mode = Mode::Normal;
                Outcome::handled()
            }
            _ => Outcome::handled(),
        }
    }

    fn apply_operator(
        &mut self,
        ch: char,
        buffer: &mut Buffer,
        anchor: &mut Option<usize>,
    ) -> Outcome {
        let operator = self.pending.take().unwrap_or('d');
        let count = if self.count.is_empty() {
            1
        } else {
            self.count.parse::<usize>().unwrap_or(1)
        };
        self.count.clear();

        // `dd`, `yy`, `cc` are linewise.
        if ch == operator {
            let (line, _) = buffer.line_col();
            let start = buffer.char_index_at(line, 0);
            let end_line = (line + count - 1).min(buffer.line_count().saturating_sub(1));
            let content_end = buffer.char_index_at(end_line, buffer.line_char_len(end_line));
            if operator == 'c' {
                // Keep the trailing newline so `cc` leaves an empty line to type on.
                return self.finish_operator(operator, start..content_end, buffer, anchor, true);
            }
            let end = (content_end + 1).min(buffer.len_chars());
            return self.finish_operator(operator, start..end, buffer, anchor, true);
        }

        let Some(target) = self.motion_target(ch, buffer, count) else {
            return Outcome::handled();
        };
        let target = if ch == 'w' {
            let (cursor_line, _) = buffer.line_col();
            let (target_line, _) = buffer.line_col_at(target);
            let mut target = if target_line != cursor_line {
                // `dw` on the last word of a line stops at the line ending; it does not
                // swallow the newline and the next line.
                buffer.char_index_at(cursor_line, buffer.line_char_len(cursor_line))
            } else {
                target
            };
            let text: Vec<char> = buffer.text().chars().collect();
            while target < text.len() && text[target].is_whitespace() && text[target] != '\n' {
                target += 1;
            }
            target
        } else {
            target
        };
        let cursor = buffer.cursor();
        let range = if target < cursor {
            target..cursor
        } else {
            cursor..if ch == 'e' {
                (target + 1).min(buffer.len_chars())
            } else {
                target
            }
        };
        self.finish_operator(operator, range, buffer, anchor, false)
    }

    fn finish_operator(
        &mut self,
        operator: char,
        range: std::ops::Range<usize>,
        buffer: &mut Buffer,
        anchor: &mut Option<usize>,
        linewise: bool,
    ) -> Outcome {
        let text = buffer.slice(range.clone());
        match operator {
            'y' => {
                self.register = text;
                self.register_linewise = linewise;
                *anchor = None;
                Outcome::handled()
            }
            'c' => {
                self.register = text;
                self.register_linewise = linewise;
                buffer.replace(range.clone(), "");
                buffer.set_cursor(range.start);
                *anchor = None;
                self.mode = Mode::Insert;
                Outcome::handled()
            }
            _ => {
                self.register = text;
                self.register_linewise = linewise;
                buffer.replace(range.clone(), "");
                buffer.set_cursor(range.start);
                *anchor = None;
                if linewise {
                    buffer.move_home();
                }
                Outcome::handled()
            }
        }
    }

    /// Paste a linewise register below (`after`) or above the current line.
    fn paste_linewise(&mut self, buffer: &mut Buffer, after: bool) {
        let (line, _) = buffer.line_col();
        let text = self.register.trim_end_matches('\n').to_string();
        if after {
            let line_count = buffer.line_count();
            let at = if line + 1 < line_count {
                buffer.char_index_at(line + 1, 0)
            } else {
                buffer.len_chars()
            };
            let insertion = if at == buffer.len_chars() {
                format!("\n{text}")
            } else {
                format!("{text}\n")
            };
            buffer.replace(at..at, &insertion);
        } else {
            let at = buffer.char_index_at(line, 0);
            let insertion = format!("{text}\n");
            buffer.replace(at..at, &insertion);
            buffer.set_cursor(at);
        }
    }

    fn motion_target(&self, ch: char, buffer: &Buffer, count: usize) -> Option<usize> {
        let mut scratch = buffer.clone();
        for _ in 0..count {
            match ch {
                'h' => scratch.move_left(),
                'l' => scratch.move_right(),
                'j' => scratch.move_down(),
                'k' => scratch.move_up(),
                'w' => scratch.move_word_right(),
                'b' => scratch.move_word_left(),
                'e' => self.move_word_end_buf(&mut scratch),
                '0' => scratch.move_home(),
                '^' => first_non_blank_in(&mut scratch),
                '$' => scratch.move_end(),
                'G' => scratch.move_document_end(),
                _ => return None,
            }
        }
        Some(scratch.cursor())
    }

    fn move_cursor(
        &mut self,
        buffer: &mut Buffer,
        anchor: &mut Option<usize>,
        mut movement: impl FnMut(&mut Buffer),
        count: usize,
    ) -> Outcome {
        for _ in 0..count {
            movement(buffer);
        }
        *anchor = None;
        Outcome::handled()
    }

    fn move_word_end(&self, buffer: &mut Buffer) {
        self.move_word_end_buf(buffer)
    }

    fn move_word_end_buf(&self, buffer: &mut Buffer) {
        let text: Vec<char> = buffer.text().chars().collect();
        let mut index = buffer.cursor();
        if index >= text.len() {
            return;
        }
        // Skip whitespace/punctuation to the next word, then run to its last character.
        while index < text.len() && !is_word(text[index]) {
            index += 1;
        }
        while index < text.len() && is_word(text[index]) {
            index += 1;
        }
        buffer.set_cursor(index.saturating_sub(1));
    }

    fn visual_range(&self, buffer: &Buffer, anchor: &Option<usize>) -> std::ops::Range<usize> {
        let anchor = anchor.unwrap_or(buffer.cursor());
        let cursor = buffer.cursor();
        if self.mode == Mode::VisualLine {
            let (a_line, _) = buffer.line_col_at(anchor);
            let (c_line, _) = buffer.line_col_at(cursor);
            let (first, last) = if a_line <= c_line {
                (a_line, c_line)
            } else {
                (c_line, a_line)
            };
            let start = buffer.char_index_at(first, 0);
            let end = buffer.char_index_at(last, buffer.line_char_len(last));
            let end = (end + 1).min(buffer.len_chars());
            start..end
        } else {
            let start = anchor.min(cursor);
            let end = (anchor.max(cursor) + 1).min(buffer.len_chars());
            start..end
        }
    }

    fn delete_char(&self, buffer: &mut Buffer) {
        if buffer.cursor() < buffer.len_chars() {
            let cursor = buffer.cursor();
            buffer.replace(cursor..cursor + 1, "");
        }
    }

    fn replace_char(&mut self, ch: char, buffer: &mut Buffer) -> Outcome {
        if buffer.cursor() < buffer.len_chars() {
            let cursor = buffer.cursor();
            buffer.replace(cursor..cursor + 1, &ch.to_string());
        }
        Outcome::handled()
    }

    fn first_non_blank(&self, buffer: &mut Buffer) {
        first_non_blank_in(buffer)
    }

    fn goto_line(&self, buffer: &mut Buffer, line: usize) {
        let line = line.min(buffer.line_count().saturating_sub(1));
        let index = buffer.char_index_at(line, 0);
        buffer.set_cursor(index);
        self.first_non_blank(buffer);
    }

    fn execute_command(&mut self, buffer: &mut Buffer, anchor: &mut Option<usize>) -> Outcome {
        let command = self.command.trim().to_string();
        self.command.clear();
        self.mode = Mode::Normal;
        *anchor = None;

        if command.is_empty() {
            return Outcome::handled();
        }

        if let Ok(line) = command.parse::<usize>() {
            self.goto_line(buffer, line.saturating_sub(1));
            return Outcome::handled();
        }

        match command.as_str() {
            "w" | "write" => Outcome {
                handled: true,
                save: true,
                message: Some("saved".to_string()),
            },
            "wq" | "x" | "xit" => Outcome {
                handled: true,
                save: true,
                message: Some("saved (close with Ctrl+W)".to_string()),
            },
            "q" | "quit" => Outcome::message("use Ctrl+W to close the tab"),
            _ => {
                if let Some(rest) = command
                    .strip_prefix("%s")
                    .or_else(|| command.strip_prefix("s"))
                {
                    return self.substitute(rest, command.starts_with("%s"), buffer);
                }
                Outcome::message(format!("unknown command: :{command}"))
            }
        }
    }

    fn substitute(&mut self, rest: &str, whole_document: bool, buffer: &mut Buffer) -> Outcome {
        let Some(delimiter) = rest.chars().next() else {
            return Outcome::message("usage: :s/pattern/replacement/");
        };
        let body = &rest[delimiter.len_utf8()..];
        let parts: Vec<&str> = body.split(delimiter).collect();
        if parts.len() < 2 {
            return Outcome::message("usage: :s/pattern/replacement/");
        }
        let pattern = parts[0];
        let replacement = parts[1];
        let global = parts.get(2).is_some_and(|flags| flags.contains('g'));

        let options = find::Options {
            regex: true,
            case_sensitive: true,
            whole_word: false,
        };

        if whole_document {
            match find::replace_all(&buffer.text(), pattern, replacement, options) {
                Ok((text, count)) => {
                    if count > 0 {
                        buffer.replace(0..buffer.len_chars(), &text);
                    }
                    Outcome::message(format!("{count} substitution(s)"))
                }
                Err(error) => Outcome::message(error),
            }
        } else {
            let (line, _) = buffer.line_col();
            let start = buffer.char_index_at(line, 0);
            let end = buffer.char_index_at(line, buffer.line_char_len(line));
            let line_text = buffer.slice(start..end);
            let result = if global {
                find::replace_all(&line_text, pattern, replacement, options)
            } else {
                match find::find_all(&line_text, pattern, options) {
                    Ok(matches) => match matches.first() {
                        Some(found) => {
                            match find::replacement_for(
                                &line_text,
                                found,
                                pattern,
                                replacement,
                                options,
                            ) {
                                Ok(Some(text)) => {
                                    let mut new_text = line_text.clone();
                                    let range = found.range.clone();
                                    new_text.replace_range(byte_range(&line_text, range), &text);
                                    Ok((new_text, 1))
                                }
                                Ok(None) => Ok((line_text.clone(), 0)),
                                Err(error) => Err(error),
                            }
                        }
                        None => Ok((line_text.clone(), 0)),
                    },
                    Err(error) => Err(error),
                }
            };

            match result {
                Ok((text, count)) => {
                    if count > 0 {
                        buffer.replace(start..end, &text);
                    }
                    Outcome::message(format!("{count} substitution(s)"))
                }
                Err(error) => Outcome::message(error),
            }
        }
    }

    fn execute_search(&mut self, buffer: &mut Buffer, anchor: &mut Option<usize>) -> Outcome {
        let pattern = self.command.trim().to_string();
        self.command.clear();
        self.mode = Mode::Normal;
        *anchor = None;
        if !pattern.is_empty() {
            self.last_search = Some(pattern.clone());
        }
        match self.last_search.clone() {
            Some(pattern) => {
                self.search(&pattern, false, buffer);
                Outcome::handled()
            }
            None => Outcome::message("no previous search"),
        }
    }

    fn search(&self, pattern: &str, backwards: bool, buffer: &mut Buffer) {
        let text = buffer.text();
        let options = find::Options {
            regex: true,
            case_sensitive: true,
            whole_word: false,
        };
        let Ok(matches) = find::find_all(&text, pattern, options) else {
            return;
        };
        if matches.is_empty() {
            return;
        }
        let cursor = buffer.cursor();
        let target = if backwards {
            matches
                .iter()
                .rev()
                .find(|found| found.range.start < cursor)
                .or_else(|| matches.last())
        } else {
            matches
                .iter()
                .find(|found| found.range.start > cursor)
                .or_else(|| matches.first())
        };
        if let Some(found) = target {
            buffer.set_cursor(found.range.start);
        }
    }
}

fn is_word(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_'
}

fn byte_range(text: &str, chars: std::ops::Range<usize>) -> std::ops::Range<usize> {
    let start = text
        .char_indices()
        .nth(chars.start)
        .map(|(byte, _)| byte)
        .unwrap_or(text.len());
    let end = text
        .char_indices()
        .nth(chars.end)
        .map(|(byte, _)| byte)
        .unwrap_or(text.len());
    start..end
}

fn first_non_blank_in(buffer: &mut Buffer) {
    let (line, _) = buffer.line_col();
    let text = buffer.line(line);
    let indent = text.chars().take_while(|ch| ch.is_whitespace()).count();
    let index = buffer.char_index_at(line, indent);
    buffer.set_cursor(index);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vim() -> Vim {
        let mut vim = Vim::new();
        vim.set_enabled(true);
        vim
    }

    fn text(buffer: &Buffer) -> String {
        buffer.text()
    }

    #[test]
    fn insert_mode_lets_the_editor_type() {
        let mut vim = vim();
        let mut buffer = Buffer::from_text("");
        let mut anchor = None;
        let outcome = vim.handle_text("i", &mut buffer, &mut anchor);
        assert!(outcome.handled);
        assert_eq!(vim.mode(), Mode::Insert);
        let outcome = vim.handle_text("hello", &mut buffer, &mut anchor);
        assert!(!outcome.handled);
        assert_eq!(vim.mode(), Mode::Insert);
    }

    #[test]
    fn escape_returns_to_normal_and_x_deletes() {
        let mut vim = vim();
        let mut buffer = Buffer::from_text("abc");
        let mut anchor = None;
        vim.handle_text("i", &mut buffer, &mut anchor);
        vim.handle_key("escape", &mut buffer, &mut anchor);
        assert_eq!(vim.mode(), Mode::Normal);
        vim.handle_text("x", &mut buffer, &mut anchor);
        assert_eq!(text(&buffer), "bc");
    }

    #[test]
    fn counts_repeat_motions_and_deletions() {
        let mut vim = vim();
        let mut buffer = Buffer::from_text("abcdef");
        let mut anchor = None;
        buffer.set_cursor(0);
        vim.handle_text("3x", &mut buffer, &mut anchor);
        assert_eq!(text(&buffer), "def");
    }

    #[test]
    fn dw_deletes_a_word() {
        let mut vim = vim();
        let mut buffer = Buffer::from_text("one two three");
        let mut anchor = None;
        buffer.set_cursor(0);
        vim.handle_text("dw", &mut buffer, &mut anchor);
        assert_eq!(text(&buffer), "two three");
    }

    #[test]
    fn dd_deletes_a_line() {
        let mut vim = vim();
        let mut buffer = Buffer::from_text("one\ntwo\nthree");
        let mut anchor = None;
        buffer.set_cursor(0);
        vim.handle_text("dd", &mut buffer, &mut anchor);
        assert_eq!(text(&buffer), "two\nthree");
    }

    #[test]
    fn de_deletes_to_the_end_of_the_word() {
        let mut vim = vim();
        let mut buffer = Buffer::from_text("one two");
        let mut anchor = None;
        buffer.set_cursor(0);
        vim.handle_text("de", &mut buffer, &mut anchor);
        assert_eq!(text(&buffer), " two");
    }

    #[test]
    fn cc_keeps_an_empty_line_for_insert_mode() {
        let mut vim = vim();
        let mut buffer = Buffer::from_text("one\ntwo");
        let mut anchor = None;
        buffer.set_cursor(0);
        vim.handle_text("cc", &mut buffer, &mut anchor);
        assert_eq!(vim.mode(), Mode::Insert);
        assert_eq!(text(&buffer), "\ntwo");
    }

    #[test]
    fn dw_on_the_last_word_stops_at_the_newline() {
        let mut vim = vim();
        let mut buffer = Buffer::from_text("one\ntwo");
        let mut anchor = None;
        buffer.set_cursor(0);
        vim.handle_text("dw", &mut buffer, &mut anchor);
        assert_eq!(text(&buffer), "\ntwo");
    }

    #[test]
    fn yy_and_p_paste_a_line() {
        let mut vim = vim();
        let mut buffer = Buffer::from_text("one\ntwo");
        let mut anchor = None;
        buffer.set_cursor(0);
        vim.handle_text("yy", &mut buffer, &mut anchor);
        vim.handle_text("p", &mut buffer, &mut anchor);
        assert_eq!(text(&buffer), "one\none\ntwo");
    }

    #[test]
    fn visual_mode_deletes_the_selection() {
        let mut vim = vim();
        let mut buffer = Buffer::from_text("abcdef");
        let mut anchor = None;
        buffer.set_cursor(1);
        vim.handle_text("vll", &mut buffer, &mut anchor);
        assert_eq!(vim.mode(), Mode::Visual);
        vim.handle_text("d", &mut buffer, &mut anchor);
        assert_eq!(text(&buffer), "aef");
    }

    #[test]
    fn colon_write_asks_the_editor_to_save() {
        let mut vim = vim();
        let mut buffer = Buffer::from_text("x");
        let mut anchor = None;
        vim.handle_text(":", &mut buffer, &mut anchor);
        vim.handle_text("wq", &mut buffer, &mut anchor);
        let outcome = vim.handle_key("enter", &mut buffer, &mut anchor);
        assert!(outcome.save);
        assert_eq!(vim.mode(), Mode::Normal);
    }

    #[test]
    fn search_moves_to_the_next_match() {
        let mut vim = vim();
        let mut buffer = Buffer::from_text("one two one two");
        let mut anchor = None;
        buffer.set_cursor(0);
        vim.handle_text("/two", &mut buffer, &mut anchor);
        vim.handle_key("enter", &mut buffer, &mut anchor);
        assert_eq!(buffer.cursor(), 4);
        vim.handle_text("n", &mut buffer, &mut anchor);
        assert_eq!(buffer.cursor(), 12);
    }

    #[test]
    fn substitute_command_replaces_on_the_current_line() {
        let mut vim = vim();
        let mut buffer = Buffer::from_text("one two\none two");
        let mut anchor = None;
        buffer.set_cursor(0);
        vim.handle_text(":s/one/1/", &mut buffer, &mut anchor);
        vim.handle_key("enter", &mut buffer, &mut anchor);
        assert_eq!(text(&buffer), "1 two\none two");
    }

    #[test]
    fn global_substitute_replaces_everywhere() {
        let mut vim = vim();
        let mut buffer = Buffer::from_text("one two\none two");
        let mut anchor = None;
        vim.handle_text(":%s/one/1/g", &mut buffer, &mut anchor);
        vim.handle_key("enter", &mut buffer, &mut anchor);
        assert_eq!(text(&buffer), "1 two\n1 two");
    }
}
