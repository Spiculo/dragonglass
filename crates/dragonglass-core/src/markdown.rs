//! Minimal block-level markdown parser, used by reading mode.
//!
//! This intentionally does *not* try to be a full CommonMark implementation. It recognises
//! the block structure Obsidian notes actually use (headings, paragraphs, lists, fenced
//! code, quotes, rules) so the reading view can style them. Inline formatting (bold,
//! italics, links, math) is rendered as plain text for now.

use crate::frontmatter;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    Heading {
        level: u8,
        text: String,
    },
    Paragraph(String),
    ListItem {
        ordered: bool,
        depth: usize,
        /// Display marker: `•` for bullets, `1.` for ordered items.
        marker: String,
        /// `Some(checked)` for a GFM task item (`- [ ]` / `- [x]`).
        checked: Option<bool>,
        text: String,
    },
    Code {
        language: String,
        text: String,
    },
    Quote(String),
    Rule,
    /// A standalone `![[target]]` or `![alt](target)` embed.
    Embed {
        target: String,
    },
    /// An Obsidian callout: a blockquote starting with `[!kind] Title`.
    Callout {
        kind: String,
        title: String,
        text: String,
    },
    /// Display math: `$$ … $$`.
    Math {
        tex: String,
    },
    /// A footnote definition (`[^id]: text`), collected at the end of the document.
    Footnote {
        id: String,
        text: String,
    },
    /// A GFM pipe table.
    Table {
        align: Vec<Align>,
        headers: Vec<String>,
        rows: Vec<Vec<String>>,
    },
}

/// Column alignment in a pipe table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

/// Remove `%%comment%%` spans (which may run across lines) and `<!-- html comments -->`.
pub fn strip_comments(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut rest = src;

    while let Some(start) = rest.find("%%") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        match after.find("%%") {
            Some(end) => rest = &after[end + 2..],
            // Unterminated comment: drop everything to the end.
            None => return out,
        }
    }

    out.push_str(rest);
    strip_html_comments(&out)
}

fn strip_html_comments(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut rest = src;

    while let Some(start) = rest.find("<!--") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 4..];
        match after.find("-->") {
            Some(end) => rest = &after[end + 3..],
            None => return out,
        }
    }

    out.push_str(rest);
    out
}

/// Split text into `(segment, is_highlight)` runs around `==highlight==` spans.
pub fn split_highlights(text: &str) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    let mut rest = text;
    let mut highlighted = false;

    loop {
        match rest.find("==") {
            Some(index) => {
                if index > 0 {
                    out.push((rest[..index].to_string(), highlighted));
                }
                rest = &rest[index + 2..];
                highlighted = !highlighted;
            }
            None => {
                if !rest.is_empty() {
                    out.push((rest.to_string(), highlighted));
                }
                break;
            }
        }
    }

    out
}

/// Parse a document (frontmatter is skipped) into display blocks.
pub fn parse_blocks(src: &str) -> Vec<Block> {
    let cleaned = strip_comments(src);
    let body = frontmatter::split(&cleaned).body;
    let lines: Vec<&str> = body.lines().collect();

    let mut blocks = Vec::new();
    let mut footnotes = Vec::new();
    let mut paragraph: Vec<&str> = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();

        if is_fence(trimmed) {
            flush_paragraph(&mut paragraph, &mut blocks);

            let fence_char = trimmed.chars().next().unwrap_or('`');
            let language = trimmed.trim_start_matches(fence_char).trim().to_string();
            i += 1;

            let mut code = String::new();
            while i < lines.len() {
                let candidate = lines[i].trim();
                if is_fence(candidate) && candidate.starts_with(fence_char) {
                    break;
                }
                code.push_str(lines[i]);
                code.push('\n');
                i += 1;
            }
            if i < lines.len() {
                i += 1;
            }

            blocks.push(Block::Code {
                language,
                text: code,
            });
            continue;
        }

        if trimmed.is_empty() {
            flush_paragraph(&mut paragraph, &mut blocks);
            i += 1;
            continue;
        }

        if let Some((id, text)) = footnote_definition(trimmed) {
            flush_paragraph(&mut paragraph, &mut blocks);
            footnotes.push(Block::Footnote { id, text });
            i += 1;
            continue;
        }

        if let Some(tex) = single_line_math(trimmed) {
            flush_paragraph(&mut paragraph, &mut blocks);
            blocks.push(Block::Math { tex });
            i += 1;
            continue;
        }

        if trimmed == "$$" {
            flush_paragraph(&mut paragraph, &mut blocks);
            i += 1;

            let mut tex = String::new();
            while i < lines.len() && lines[i].trim() != "$$" {
                tex.push_str(lines[i]);
                tex.push('\n');
                i += 1;
            }
            if i < lines.len() {
                i += 1;
            }

            blocks.push(Block::Math {
                tex: tex.trim_end().to_string(),
            });
            continue;
        }

        if let Some((table, consumed)) = table_at(&lines, i) {
            flush_paragraph(&mut paragraph, &mut blocks);
            blocks.push(table);
            i += consumed;
            continue;
        }

        if let Some((level, text)) = heading(trimmed) {
            flush_paragraph(&mut paragraph, &mut blocks);
            blocks.push(Block::Heading { level, text });
            i += 1;
            continue;
        }

        // Setext headings: a paragraph line underlined with `===` or `---`.
        if !paragraph.is_empty() {
            if let Some(level) = setext_underline(trimmed) {
                let text = paragraph.join(" ");
                paragraph.clear();
                blocks.push(Block::Heading { level, text });
                i += 1;
                continue;
            }
        }

        if is_rule(trimmed) {
            flush_paragraph(&mut paragraph, &mut blocks);
            blocks.push(Block::Rule);
            i += 1;
            continue;
        }

        if trimmed.starts_with('>') {
            flush_paragraph(&mut paragraph, &mut blocks);

            let mut quoted: Vec<String> = Vec::new();
            while i < lines.len() {
                let Some(rest) = lines[i].trim_start().strip_prefix('>') else {
                    break;
                };
                quoted.push(rest.trim_start().to_string());
                i += 1;
            }

            // `> [!kind] Title` is a callout; everything else is a plain quote.
            if let Some(callout) = parse_callout(&quoted) {
                blocks.push(callout);
            } else {
                blocks.push(Block::Quote(quoted.join("\n")));
            }
            continue;
        }

        if let Some((ordered, depth, marker, checked, text)) = list_item(line) {
            flush_paragraph(&mut paragraph, &mut blocks);
            blocks.push(Block::ListItem {
                ordered,
                depth,
                marker,
                checked,
                text,
            });
            i += 1;
            continue;
        }

        paragraph.push(trimmed);
        i += 1;
    }

    flush_paragraph(&mut paragraph, &mut blocks);
    if !footnotes.is_empty() {
        blocks.push(Block::Rule);
        blocks.extend(footnotes);
    }
    blocks
}

/// `[^id]: text` — a footnote definition.
fn footnote_definition(line: &str) -> Option<(String, String)> {
    let rest = line.strip_prefix("[^")?;
    let close = rest.find("]:")?;
    let id = rest[..close].trim();
    if id.is_empty() {
        return None;
    }
    Some((id.to_string(), rest[close + 2..].trim().to_string()))
}

fn flush_paragraph(paragraph: &mut Vec<&str>, blocks: &mut Vec<Block>) {
    if paragraph.is_empty() {
        return;
    }

    if paragraph.len() == 1 {
        if let Some(target) = standalone_embed(paragraph[0]) {
            paragraph.clear();
            blocks.push(Block::Embed { target });
            return;
        }
    }

    let text = paragraph.join("\n");
    paragraph.clear();
    blocks.push(Block::Paragraph(text));
}

/// A line that is nothing but an embed, as Obsidian writes embedded images and notes.
fn standalone_embed(line: &str) -> Option<String> {
    let trimmed = line.trim();

    if trimmed.starts_with("![[") && trimmed.ends_with("]]") {
        let inner = &trimmed[3..trimmed.len() - 2];
        let target = inner
            .split('|')
            .next()
            .unwrap_or(inner)
            .split('#')
            .next()
            .unwrap_or(inner)
            .trim();
        if !target.is_empty() {
            return Some(target.to_string());
        }
    }

    if let Some(rest) = trimmed.strip_prefix("![") {
        if let Some(end) = rest.find("](") {
            if let Some(source) = rest[end + 2..].strip_suffix(')') {
                let source = source.trim();
                if !source.is_empty() {
                    return Some(source.to_string());
                }
            }
        }
    }

    None
}

fn is_fence(trimmed: &str) -> bool {
    trimmed.starts_with("```") || trimmed.starts_with("~~~")
}

/// `$$…$$` on a single line.
fn single_line_math(trimmed: &str) -> Option<String> {
    let inner = trimmed.strip_prefix("$$")?.strip_suffix("$$")?;
    if inner.is_empty() {
        None
    } else {
        Some(inner.trim().to_string())
    }
}

/// Recognise `[!kind] Title` at the start of a quoted block.
fn parse_callout(quoted: &[String]) -> Option<Block> {
    let first = quoted.first()?;
    let rest = first.strip_prefix("[!")?;
    let end = rest.find(']')?;

    let kind = rest[..end].trim().to_lowercase();
    if kind.is_empty() {
        return None;
    }

    let title: String = rest[end + 1..].trim().to_string();
    let title = if title.is_empty() {
        kind.clone()
    } else {
        title
    };

    Some(Block::Callout {
        kind,
        title,
        text: quoted[1..].join("\n"),
    })
}

/// A heading and the 1-based source line it appears on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Heading {
    pub level: u8,
    pub text: String,
    pub line: usize,
}

/// Headings in document order, with source line numbers. Frontmatter is skipped but
/// still counted, and headings inside fenced code are ignored.
pub fn headings(src: &str) -> Vec<Heading> {
    let body = frontmatter::split(src).body;
    let offset = src.len().saturating_sub(body.len());
    let prefix_lines = src[..offset].matches('\n').count();

    let mut out = Vec::new();
    let mut in_fence = false;
    // The last plain paragraph line, which a following `===`/`---` underlines.
    let mut previous: Option<(usize, String)> = None;

    for (i, line) in body.lines().enumerate() {
        let trimmed = line.trim();

        if is_fence(trimmed) {
            in_fence = !in_fence;
            previous = None;
            continue;
        }
        if in_fence {
            continue;
        }

        if let Some(level) = setext_underline(trimmed) {
            if let Some((line, text)) = previous.take() {
                out.push(Heading {
                    level,
                    text: crate::inline::plain(&text),
                    line: prefix_lines + line + 1,
                });
                continue;
            }
        }

        if let Some((level, text)) = heading(trimmed) {
            out.push(Heading {
                level,
                // The outline shows the rendered text, not the markup.
                text: crate::inline::plain(&text),
                line: prefix_lines + i + 1,
            });
            previous = None;
            continue;
        }

        if trimmed.is_empty() || starts_other_block(trimmed) {
            previous = None;
            continue;
        }

        previous = Some((i, trimmed.to_string()));
    }

    out
}

/// Lines that begin a block other than a paragraph, so a following `---` is a rule (or a
/// list marker) rather than a setext underline.
fn starts_other_block(trimmed: &str) -> bool {
    trimmed.starts_with('>')
        || trimmed.starts_with('|')
        || is_rule(trimmed)
        || matches!(trimmed.get(..2), Some("- ") | Some("* ") | Some("+ "))
        || trimmed.chars().take_while(|ch| ch.is_ascii_digit()).count() > 0
}

fn heading(trimmed: &str) -> Option<(u8, String)> {
    let level = trimmed.chars().take_while(|c| *c == '#').count();
    if level == 0 || level > 6 {
        return None;
    }
    let rest = &trimmed[level..];
    if !rest.is_empty() && !rest.starts_with(' ') {
        return None;
    }
    let text = rest.trim().trim_end_matches('#').trim().to_string();
    Some((level as u8, text))
}

/// A GFM pipe table: header row, delimiter row, then zero or more body rows.
fn table_at(lines: &[&str], start: usize) -> Option<(Block, usize)> {
    if start + 1 >= lines.len() {
        return None;
    }

    let header = lines[start].trim();
    let delimiter = lines[start + 1].trim();
    if !header.contains('|') || !is_table_delimiter(delimiter) {
        return None;
    }

    let align = table_align(delimiter);
    if align.is_empty() {
        return None;
    }
    let width = align.len();

    let mut headers = split_row(header);
    headers.truncate(width);
    headers.resize(width, String::new());

    let mut rows = Vec::new();
    let mut index = start + 2;
    while index < lines.len() {
        let row = lines[index].trim();
        if row.is_empty() || !row.contains('|') {
            break;
        }
        let mut cells = split_row(row);
        cells.truncate(width);
        cells.resize(width, String::new());
        rows.push(cells);
        index += 1;
    }

    Some((
        Block::Table {
            align,
            headers,
            rows,
        },
        index - start,
    ))
}

fn split_row(line: &str) -> Vec<String> {
    let trimmed = line.trim();
    let trimmed = trimmed.strip_prefix('|').unwrap_or(trimmed);
    let trimmed = trimmed.strip_suffix('|').unwrap_or(trimmed);

    // Split on unescaped pipes that are not inside an inline code span.
    let mut cells: Vec<String> = Vec::new();
    let mut cell = String::new();
    let mut in_code = false;
    let mut chars = trimmed.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '\\' if chars.peek() == Some(&'|') => {
                cell.push('|');
                chars.next();
            }
            '`' => {
                in_code = !in_code;
                cell.push(ch);
            }
            '|' if !in_code => {
                cells.push(cell.trim().to_string());
                cell.clear();
            }
            _ => cell.push(ch),
        }
    }
    cells.push(cell.trim().to_string());
    cells
}

fn is_table_delimiter(line: &str) -> bool {
    if !line.contains('|') {
        return false;
    }
    !table_align(line).is_empty()
}

fn table_align(delimiter: &str) -> Vec<Align> {
    let cells = split_row(delimiter);
    if cells.is_empty() {
        return Vec::new();
    }

    let mut align = Vec::with_capacity(cells.len());
    for cell in cells {
        let left = cell.starts_with(':');
        let right = cell.ends_with(':');
        let inner = cell.trim_matches(':').trim();
        if inner.len() < 3 || !inner.chars().all(|ch| ch == '-') {
            return Vec::new();
        }
        align.push(match (left, right) {
            (true, true) => Align::Center,
            (false, true) => Align::Right,
            _ => Align::Left,
        });
    }
    align
}

fn is_rule(trimmed: &str) -> bool {
    let compact: String = trimmed.chars().filter(|c| !c.is_whitespace()).collect();
    compact.len() >= 3
        && (compact.chars().all(|c| c == '-')
            || compact.chars().all(|c| c == '*')
            || compact.chars().all(|c| c == '_'))
}

fn list_item(line: &str) -> Option<(bool, usize, String, Option<bool>, String)> {
    let indent = line.len() - line.trim_start().len();
    let depth = indent / 2;
    let trimmed = line.trim_start();

    for bullet in ["- ", "* ", "+ "] {
        if let Some(rest) = trimmed.strip_prefix(bullet) {
            let (checked, text) = task_marker(rest);
            return Some((false, depth, "\u{2022}".to_string(), checked, text));
        }
    }

    let digits_len = trimmed.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits_len > 0 {
        let rest = &trimmed[digits_len..];
        // Both `1.` and `1)` are valid ordered-list markers in GFM.
        let rest = rest.strip_prefix(". ").or_else(|| rest.strip_prefix(") "));
        if let Some(text) = rest {
            let number = &trimmed[..digits_len];
            return Some((
                true,
                depth,
                format!("{number}."),
                None,
                text.trim().to_string(),
            ));
        }
    }

    None
}

/// Strip a GFM task-list checkbox from the start of a list item.
fn task_marker(text: &str) -> (Option<bool>, String) {
    let trimmed = text.trim_start();
    for (marker, checked) in [("[ ] ", false), ("[x] ", true), ("[X] ", true)] {
        if let Some(rest) = trimmed.strip_prefix(marker) {
            return (Some(checked), rest.trim().to_string());
        }
    }
    (None, text.trim().to_string())
}

/// `===` underlines a level-1 heading, `---` a level-2 heading.
fn setext_underline(trimmed: &str) -> Option<u8> {
    let first = trimmed.chars().next()?;
    if first != '=' && first != '-' {
        return None;
    }
    if !trimmed.chars().all(|ch| ch == first) {
        return None;
    }
    // A single `-` is a list marker, not an underline.
    if first == '-' && trimmed.len() < 2 {
        return None;
    }
    Some(if first == '=' { 1 } else { 2 })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skips_frontmatter() {
        let blocks = parse_blocks("---\ntitle: x\n---\n# Real\n");
        assert_eq!(
            blocks,
            vec![Block::Heading {
                level: 1,
                text: "Real".into()
            }]
        );
    }

    #[test]
    fn headings_by_level() {
        let blocks = parse_blocks("# One\n### Three\n");
        assert_eq!(
            blocks,
            vec![
                Block::Heading {
                    level: 1,
                    text: "One".into()
                },
                Block::Heading {
                    level: 3,
                    text: "Three".into()
                },
            ]
        );
    }

    #[test]
    fn paragraph_joins_until_blank_line() {
        let blocks = parse_blocks("line one\nline two\n\nnext");
        assert_eq!(
            blocks,
            vec![
                Block::Paragraph("line one\nline two".into()),
                Block::Paragraph("next".into()),
            ]
        );
    }

    #[test]
    fn unordered_and_ordered_lists() {
        let blocks = parse_blocks("- a\n  - nested\n1. first\n");
        assert_eq!(
            blocks,
            vec![
                Block::ListItem {
                    ordered: false,
                    depth: 0,
                    marker: "\u{2022}".into(),
                    checked: None,
                    text: "a".into()
                },
                Block::ListItem {
                    ordered: false,
                    depth: 1,
                    marker: "\u{2022}".into(),
                    checked: None,
                    text: "nested".into()
                },
                Block::ListItem {
                    ordered: true,
                    depth: 0,
                    marker: "1.".into(),
                    checked: None,
                    text: "first".into()
                },
            ]
        );
    }

    #[test]
    fn task_list_items_carry_their_checkbox() {
        let blocks = parse_blocks("- [ ] todo\n- [x] done\n- [X] also done\n- plain\n");
        let states: Vec<Option<bool>> = blocks
            .iter()
            .map(|block| match block {
                Block::ListItem { checked, .. } => *checked,
                other => panic!("expected a list item, got {other:?}"),
            })
            .collect();
        assert_eq!(states, vec![Some(false), Some(true), Some(true), None]);
    }

    #[test]
    fn footnote_definitions_move_to_the_end() {
        let blocks = parse_blocks("Body[^a]\n\n[^a]: The note.\n");
        assert!(matches!(&blocks[0], Block::Paragraph(text) if text == "Body[^a]"));
        assert!(matches!(
            blocks.last(),
            Some(Block::Footnote { id, text }) if id == "a" && text == "The note."
        ));
    }

    #[test]
    fn ordered_lists_accept_paren_markers() {
        let blocks = parse_blocks("1) first\n2) second\n");
        assert_eq!(blocks.len(), 2);
        assert!(matches!(
            &blocks[0],
            Block::ListItem { ordered: true, marker, .. } if marker == "1."
        ));
    }

    #[test]
    fn setext_headings() {
        assert_eq!(
            parse_blocks("Title\n=====\n\nSubtitle\n---\n"),
            vec![
                Block::Heading {
                    level: 1,
                    text: "Title".into()
                },
                Block::Heading {
                    level: 2,
                    text: "Subtitle".into()
                },
            ]
        );
    }

    #[test]
    fn fenced_code_captures_language_and_body() {
        let blocks = parse_blocks("```rust\nfn main() {}\n```\nafter");
        assert_eq!(
            blocks,
            vec![
                Block::Code {
                    language: "rust".into(),
                    text: "fn main() {}\n".into()
                },
                Block::Paragraph("after".into()),
            ]
        );
    }

    #[test]
    fn blockquote_consumes_consecutive_lines() {
        let blocks = parse_blocks("> one\n> two\n\nafter");
        assert_eq!(
            blocks,
            vec![
                Block::Quote("one\ntwo".into()),
                Block::Paragraph("after".into()),
            ]
        );
    }

    #[test]
    fn horizontal_rule() {
        assert_eq!(parse_blocks("---\n"), vec![Block::Rule]);
    }

    #[test]
    fn standalone_wikilink_embed() {
        let blocks = parse_blocks("![[image.png]]\n\ntext\n");
        assert_eq!(
            blocks,
            vec![
                Block::Embed {
                    target: "image.png".into()
                },
                Block::Paragraph("text".into()),
            ]
        );
    }

    #[test]
    fn markdown_image_embed() {
        let blocks = parse_blocks("![alt](attachments/pic.png)\n");
        assert_eq!(
            blocks,
            vec![Block::Embed {
                target: "attachments/pic.png".into()
            }]
        );
    }

    #[test]
    fn embed_with_alias_and_heading() {
        let blocks = parse_blocks("![[Note#Section|Alias]]\n");
        assert_eq!(
            blocks,
            vec![Block::Embed {
                target: "Note".into()
            }]
        );
    }

    #[test]
    fn headings_report_source_lines() {
        let src = "---\ntitle: x\n---\n# One\n\ntext\n## Two\n";
        let found = headings(src);
        assert_eq!(
            found,
            vec![
                Heading {
                    level: 1,
                    text: "One".into(),
                    line: 4
                },
                Heading {
                    level: 2,
                    text: "Two".into(),
                    line: 7
                },
            ]
        );
    }

    #[test]
    fn comments_are_stripped_before_parsing() {
        let blocks = parse_blocks("%%hidden%%# Visible\n");
        assert_eq!(
            blocks,
            vec![Block::Heading {
                level: 1,
                text: "Visible".into()
            }]
        );
    }

    #[test]
    fn strip_comments_spans_lines_and_truncates_unterminated() {
        assert_eq!(strip_comments("a %%x\ny%% b"), "a  b");
        assert_eq!(strip_comments("a %%never closed"), "a ");
    }

    #[test]
    fn callouts_are_parsed() {
        let blocks = parse_blocks("> [!warning] Watch out\n> body line\n");
        assert_eq!(
            blocks,
            vec![Block::Callout {
                kind: "warning".into(),
                title: "Watch out".into(),
                text: "body line".into()
            }]
        );
    }

    #[test]
    fn callouts_default_the_title_to_the_kind() {
        let blocks = parse_blocks("> [!note]\n");
        assert_eq!(
            blocks,
            vec![Block::Callout {
                kind: "note".into(),
                title: "note".into(),
                text: String::new()
            }]
        );
    }

    #[test]
    fn plain_quotes_stay_quotes() {
        assert_eq!(
            parse_blocks("> just a quote\n"),
            vec![Block::Quote("just a quote".into())]
        );
    }

    #[test]
    fn display_math_blocks() {
        assert_eq!(
            parse_blocks("$$x^2$$\n"),
            vec![Block::Math { tex: "x^2".into() }]
        );
        assert_eq!(
            parse_blocks("$$\n\\frac{a}{b}\n$$\n"),
            vec![Block::Math {
                tex: "\\frac{a}{b}".into()
            }]
        );
    }

    #[test]
    fn highlight_runs_reconstruct_the_text() {
        // GPUI's StyledText::with_runs asserts the run lengths rebuild the rendered text,
        // which is the input with the `==` delimiters removed.
        for sample in [
            "",
            "plain",
            "==all==",
            "a ==b== c",
            "==a== and ==b==",
            "un==closed",
            "===",
        ] {
            let rebuilt: String = split_highlights(sample)
                .into_iter()
                .map(|(segment, _)| segment)
                .collect();
            assert_eq!(rebuilt, sample.replace("==", ""), "sample: {sample:?}");
        }
    }

    #[test]
    fn highlights_split_into_runs() {
        assert_eq!(
            split_highlights("a ==b== c"),
            vec![
                ("a ".to_string(), false),
                ("b".to_string(), true),
                (" c".to_string(), false),
            ]
        );
        assert_eq!(
            split_highlights("plain"),
            vec![("plain".to_string(), false)]
        );
    }

    #[test]
    fn headings_skip_fenced_code() {
        let src = "# Real\n```\n# Fake\n```\n## Also\n";
        let found = headings(src);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].text, "Real");
        assert_eq!(found[1].text, "Also");
    }

    #[test]
    fn parses_gfm_tables() {
        let src = "| Name | Value |\n| :--- | ---: |\n| alpha | 1 |\n| beta | 2 |\n\nafter\n";
        let blocks = parse_blocks(src);
        assert_eq!(
            blocks,
            vec![
                Block::Table {
                    align: vec![Align::Left, Align::Right],
                    headers: vec!["Name".into(), "Value".into()],
                    rows: vec![
                        vec!["alpha".into(), "1".into()],
                        vec!["beta".into(), "2".into()],
                    ],
                },
                Block::Paragraph("after".into()),
            ]
        );
    }

    #[test]
    fn table_rows_are_padded_and_truncated_to_the_header() {
        let src = "a | b\n--- | ---\n| 1 |\n| 2 | 3 | 4 |\n";
        let blocks = parse_blocks(src);
        assert_eq!(
            blocks,
            vec![Block::Table {
                align: vec![Align::Left, Align::Left],
                headers: vec!["a".into(), "b".into()],
                rows: vec![
                    vec!["1".into(), String::new()],
                    vec!["2".into(), "3".into()]
                ],
            }]
        );
    }

    #[test]
    fn a_horizontal_rule_is_not_a_table() {
        assert_eq!(
            parse_blocks("a | b\n\n---\n"),
            vec![Block::Paragraph("a | b".into()), Block::Rule]
        );
    }

    #[test]
    fn table_cells_handle_escaped_pipes_and_code_spans() {
        let blocks = parse_blocks("| a \\| b | `x | y` |\n| --- | --- |\n| 1 | 2 |\n");
        let Block::Table { headers, .. } = &blocks[0] else {
            panic!("expected table, got {blocks:?}");
        };
        assert_eq!(headers, &vec!["a | b".to_string(), "`x | y`".to_string()]);
    }

    #[test]
    fn heading_outline_text_drops_inline_markup() {
        let found = headings("# **Bold** $x^2$ [[Note|Alias]] `code`\n");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].text, "Bold x² Alias code");
    }

    #[test]
    fn outline_includes_setext_headings() {
        let src = "Title\n=====\n\ntext\n\nSub\n---\n";
        assert_eq!(
            headings(src),
            vec![
                Heading {
                    level: 1,
                    text: "Title".into(),
                    line: 1
                },
                Heading {
                    level: 2,
                    text: "Sub".into(),
                    line: 6
                },
            ]
        );
    }

    #[test]
    fn html_comments_are_stripped() {
        assert_eq!(strip_comments("a <!-- hidden\nstill hidden --> b"), "a  b");
        assert_eq!(
            parse_blocks("<!-- gone -->\n# Kept\n")[0],
            Block::Heading {
                level: 1,
                text: "Kept".into()
            }
        );
    }

    #[test]
    fn table_cells_keep_inline_markup_for_rendering() {
        let blocks = parse_blocks("| $x^2$ | **bold** |\n| --- | --- |\n");
        let Block::Table { headers, .. } = &blocks[0] else {
            panic!("expected table");
        };
        assert_eq!(headers, &vec!["$x^2$".to_string(), "**bold**".to_string()]);
    }
}
