//! Toggle markdown formatting around a character range.
//!
//! The editor works in character offsets; this module does the same and returns one
//! replacement plus the selection the caller should restore afterwards. Keeping it
//! headless makes the "wrap or unwrap" rules unit-testable.

use std::ops::Range;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    Bold,
    Italic,
    Strikethrough,
    Highlight,
    InlineCode,
    Comment,
}

impl Style {
    /// Opening and closing markers for the style.
    pub fn markers(self) -> (&'static str, &'static str) {
        match self {
            Style::Bold => ("**", "**"),
            Style::Italic => ("*", "*"),
            Style::Strikethrough => ("~~", "~~"),
            Style::Highlight => ("==", "=="),
            Style::InlineCode => ("`", "`"),
            Style::Comment => ("%%", "%%"),
        }
    }

    /// Short label for the selection toolbar.
    pub fn label(self) -> &'static str {
        match self {
            Style::Bold => "B",
            Style::Italic => "I",
            Style::Strikethrough => "S",
            Style::Highlight => "H",
            Style::InlineCode => "`",
            Style::Comment => "%%",
        }
    }
}

/// One formatting change, expressed in character offsets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    /// Characters to replace.
    pub range: Range<usize>,
    /// Replacement text.
    pub text: String,
    /// Selection inside `text`, relative to `range.start`. An empty range is a caret.
    pub selection: Range<usize>,
}

/// Wrap `range` in `style`'s markers, or remove markers that already surround it.
///
/// Three shapes are recognised:
/// - the selection includes the markers (`**bold**` selected) — markers are removed;
/// - the markers sit immediately outside the selection (`bold` with `**` on both sides) —
///   they are removed as well, so formatting toggles from either selection style;
/// - otherwise the selection is wrapped and stays selected inside the markers.
pub fn toggle(text: &str, range: Range<usize>, style: Style) -> Edit {
    let (open, close) = style.markers();
    let start = byte_at(text, range.start);
    let end = byte_at(text, range.end);
    let selected = &text[start..end];

    if start == end {
        let caret = open.chars().count();
        return Edit {
            range,
            text: format!("{open}{close}"),
            selection: caret..caret,
        };
    }

    let open_chars = open.chars().count();
    let close_chars = close.chars().count();
    let selected_chars = selected.chars().count();

    let open_char = open.chars().next().unwrap_or('*');
    let close_char = close.chars().last().unwrap_or('*');

    // Markers inside the selection.
    if selected_chars >= open_chars + close_chars
        && selected.starts_with(open)
        && selected.ends_with(close)
        && marker_fits(run_at_start(selected, open_char), open_chars)
        && marker_fits(run_at_end(selected, close_char), close_chars)
    {
        let inner = &selected[open.len()..selected.len() - close.len()];
        return Edit {
            range,
            text: inner.to_string(),
            selection: 0..inner.chars().count(),
        };
    }

    // Markers immediately outside the selection.
    let before = &text[..start];
    let after = &text[end..];
    if before.ends_with(open)
        && after.starts_with(close)
        && marker_fits(run_at_end(before, open_char), open_chars)
        && marker_fits(run_at_start(after, close_char), close_chars)
    {
        let new_start = range.start.saturating_sub(open_chars);
        let new_end = range.end + close_chars;
        return Edit {
            range: new_start..new_end,
            text: selected.to_string(),
            selection: 0..selected_chars,
        };
    }

    Edit {
        range,
        text: format!("{open}{selected}{close}"),
        selection: open_chars..open_chars + selected_chars,
    }
}

fn byte_at(text: &str, char_index: usize) -> usize {
    text.char_indices()
        .nth(char_index)
        .map(|(byte, _)| byte)
        .unwrap_or(text.len())
}

/// How many `marker` characters run at the start of `text`.
fn run_at_start(text: &str, marker: char) -> usize {
    text.chars().take_while(|ch| *ch == marker).count()
}

/// How many `marker` characters run at the end of `text`.
fn run_at_end(text: &str, marker: char) -> usize {
    text.chars().rev().take_while(|ch| *ch == marker).count()
}

/// Whether a run of markers can belong to this style.
///
/// Two-character markers may be part of a longer run (`***bold italic***`, where the outer
/// pair is a heading-style run). A single-character marker only matches an odd run, so
/// pressing italic on `**bold**` wraps it instead of silently turning it into italic.
fn marker_fits(run: usize, marker_width: usize) -> bool {
    if marker_width == 1 {
        run > 0 && run % 2 == 1
    } else {
        run >= marker_width
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_and_keeps_the_inner_text_selected() {
        let edit = toggle("hello", 0..5, Style::Bold);
        assert_eq!(edit.text, "**hello**");
        assert_eq!(edit.selection, 2..7);
    }

    #[test]
    fn removes_markers_inside_the_selection() {
        let edit = toggle("**hello**", 0..9, Style::Bold);
        assert_eq!(edit.text, "hello");
        assert_eq!(edit.selection, 0..5);
    }

    #[test]
    fn removes_markers_outside_the_selection() {
        let edit = toggle("**hello**", 2..7, Style::Bold);
        assert_eq!(edit.range, 0..9);
        assert_eq!(edit.text, "hello");
        assert_eq!(edit.selection, 0..5);
    }

    #[test]
    fn an_empty_selection_puts_the_caret_between_the_markers() {
        let edit = toggle("abc", 1..1, Style::Italic);
        assert_eq!(edit.range, 1..1);
        assert_eq!(edit.text, "**");
        assert_eq!(edit.selection, 1..1);
        assert_eq!(edit.text.chars().count(), 2);
    }

    #[test]
    fn handles_unicode_without_splitting_characters() {
        let text = "héllo 😀 world";
        let edit = toggle(text, 6..7, Style::Highlight); // the emoji
        assert_eq!(edit.text, "==😀==");
        assert_eq!(edit.selection, 2..3);
        let edit = toggle("==😀==", 0..5, Style::Highlight);
        assert_eq!(edit.text, "😀");
    }

    #[test]
    fn italic_does_not_turn_bold_into_italic() {
        // The two asterisks around `bold` are bold markers, not two italic markers.
        let edit = toggle("**bold**", 0..8, Style::Italic);
        assert_eq!(edit.text, "***bold***");
        let edit = toggle("**bold**", 2..6, Style::Italic);
        assert_eq!(edit.range, 2..6);
        assert_eq!(edit.text, "*bold*");
    }

    #[test]
    fn nested_runs_remove_the_requested_style() {
        let edit = toggle("***bold italic***", 0..17, Style::Bold);
        assert_eq!(edit.text, "*bold italic*");
        let edit = toggle("***bold italic***", 0..17, Style::Italic);
        assert_eq!(edit.text, "**bold italic**");
    }

    #[test]
    fn italic_on_bold_selection_wraps() {
        let edit = toggle("*bold*", 0..6, Style::Bold);
        assert_eq!(edit.text, "***bold***");
    }

    #[test]
    fn toggles_each_style() {
        for style in [
            Style::Bold,
            Style::Italic,
            Style::Strikethrough,
            Style::Highlight,
            Style::InlineCode,
            Style::Comment,
        ] {
            let (open, close) = style.markers();
            let wrapped = format!("{open}text{close}");
            let edit = toggle(&wrapped, 0..wrapped.chars().count(), style);
            assert_eq!(edit.text, "text", "{style:?}");
        }
    }
}
