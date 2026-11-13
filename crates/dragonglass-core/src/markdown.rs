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
        /// Requested width in pixels, from `![[image.png|300]]`.
        width: Option<u32>,
        /// Alt text, from `![alt](src)` or `![[image.png|alt text]]`.
        alt: Option<String>,
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
            Some(end) => {
                // Keep the comment's line breaks so block spans still point at the right
                // source lines.
                for _ in 0..after[..end].matches('\n').count() {
                    out.push('\n');
                }
                rest = &after[end + 2..];
            }
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
            Some(end) => {
                for _ in 0..after[..end].matches('\n').count() {
                    out.push('\n');
                }
                rest = &after[end + 3..];
            }
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

/// The source lines a block was parsed from, 0-based and inclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockSpan {
    pub first_line: usize,
    pub last_line: usize,
}

/// One paragraph being collected: its lines and the source lines they came from.
#[derive(Default)]
struct ParagraphLines<'a> {
    lines: Vec<&'a str>,
    first: Option<usize>,
    last: Option<usize>,
}

impl<'a> ParagraphLines<'a> {
    fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    fn push(&mut self, line: &'a str, index: usize) {
        if self.first.is_none() {
            self.first = Some(index);
        }
        self.lines.push(line);
        self.last = Some(index);
    }

    fn clear(&mut self) {
        self.lines.clear();
        self.first = None;
        self.last = None;
    }

    fn span(&self) -> (usize, usize) {
        (self.first.unwrap_or(0), self.last.unwrap_or(0))
    }
}

fn push_block(
    blocks: &mut Vec<Block>,
    spans: &mut Vec<BlockSpan>,
    block: Block,
    first: usize,
    last: usize,
) {
    blocks.push(block);
    spans.push(BlockSpan {
        first_line: first,
        last_line: last,
    });
}

/// Parse a document (frontmatter is skipped) into display blocks.
pub fn parse_blocks(src: &str) -> Vec<Block> {
    parse_blocks_spanned(src).0
}

/// Parse a document into blocks and the source lines each block came from.
///
/// Spans are absolute 0-based lines in `src`, so a block can be mapped back to the editor
/// cursor and the reading view can scroll to the block under the caret.
pub fn parse_blocks_spanned(src: &str) -> (Vec<Block>, Vec<BlockSpan>) {
    let cleaned = strip_comments(src);
    let body = frontmatter::split(&cleaned).body;
    // `body` is a suffix of the cleaned text, so this counts the frontmatter lines that
    // were skipped. Comment line breaks are preserved above, so these lines match `src`.
    let offset = cleaned[..cleaned.len().saturating_sub(body.len())]
        .matches('\n')
        .count();
    let lines: Vec<&str> = body.lines().collect();

    let mut blocks = Vec::new();
    let mut spans: Vec<BlockSpan> = Vec::new();
    let mut footnotes: Vec<(Block, BlockSpan)> = Vec::new();
    let mut paragraph = ParagraphLines::default();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();

        if is_fence(trimmed) {
            flush_paragraph(&mut paragraph, &mut blocks, &mut spans, offset);
            let start = i;

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
            let end = i.saturating_sub(1).min(lines.len().saturating_sub(1));

            push_block(
                &mut blocks,
                &mut spans,
                Block::Code {
                    language,
                    text: code,
                },
                offset + start,
                offset + end,
            );
            continue;
        }

        if trimmed.is_empty() {
            flush_paragraph(&mut paragraph, &mut blocks, &mut spans, offset);
            i += 1;
            continue;
        }

        if let Some((id, text)) = footnote_definition(trimmed) {
            flush_paragraph(&mut paragraph, &mut blocks, &mut spans, offset);
            footnotes.push((
                Block::Footnote { id, text },
                BlockSpan {
                    first_line: offset + i,
                    last_line: offset + i,
                },
            ));
            i += 1;
            continue;
        }

        if let Some(tex) = single_line_math(trimmed) {
            flush_paragraph(&mut paragraph, &mut blocks, &mut spans, offset);
            push_block(
                &mut blocks,
                &mut spans,
                Block::Math { tex },
                offset + i,
                offset + i,
            );
            i += 1;
            continue;
        }

        if trimmed == "$$" {
            flush_paragraph(&mut paragraph, &mut blocks, &mut spans, offset);
            let start = i;
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
            let end = i.saturating_sub(1).min(lines.len().saturating_sub(1));

            push_block(
                &mut blocks,
                &mut spans,
                Block::Math {
                    tex: tex.trim_end().to_string(),
                },
                offset + start,
                offset + end,
            );
            continue;
        }

        if let Some((table, consumed)) = table_at(&lines, i) {
            flush_paragraph(&mut paragraph, &mut blocks, &mut spans, offset);
            let end = i + consumed.saturating_sub(1);
            push_block(&mut blocks, &mut spans, table, offset + i, offset + end);
            i += consumed;
            continue;
        }

        if let Some((level, text)) = heading(trimmed) {
            flush_paragraph(&mut paragraph, &mut blocks, &mut spans, offset);
            push_block(
                &mut blocks,
                &mut spans,
                Block::Heading { level, text },
                offset + i,
                offset + i,
            );
            i += 1;
            continue;
        }

        // Setext headings: a paragraph line underlined with `===` or `---`.
        if !paragraph.is_empty() {
            if let Some(level) = setext_underline(trimmed) {
                let text = join_soft_breaks(&paragraph.lines);
                let (first, _) = paragraph.span();
                paragraph.clear();
                push_block(
                    &mut blocks,
                    &mut spans,
                    Block::Heading { level, text },
                    offset + first,
                    offset + i,
                );
                i += 1;
                continue;
            }
        }

        if is_rule(trimmed) {
            flush_paragraph(&mut paragraph, &mut blocks, &mut spans, offset);
            push_block(&mut blocks, &mut spans, Block::Rule, offset + i, offset + i);
            i += 1;
            continue;
        }

        if trimmed.starts_with('>') {
            flush_paragraph(&mut paragraph, &mut blocks, &mut spans, offset);
            let start = i;

            let mut quoted: Vec<String> = Vec::new();
            while i < lines.len() {
                let Some(rest) = lines[i].trim_start().strip_prefix('>') else {
                    break;
                };
                quoted.push(rest.trim_start().to_string());
                i += 1;
            }
            let end = i.saturating_sub(1);

            // `> [!kind] Title` is a callout; everything else is a plain quote.
            if let Some(callout) = parse_callout(&quoted) {
                push_block(
                    &mut blocks,
                    &mut spans,
                    callout,
                    offset + start,
                    offset + end,
                );
            } else {
                let borrowed: Vec<&str> = quoted.iter().map(String::as_str).collect();
                push_block(
                    &mut blocks,
                    &mut spans,
                    Block::Quote(join_soft_breaks(&borrowed)),
                    offset + start,
                    offset + end,
                );
            }
            continue;
        }

        if let Some((ordered, depth, marker, checked, text)) = list_item(line) {
            flush_paragraph(&mut paragraph, &mut blocks, &mut spans, offset);
            push_block(
                &mut blocks,
                &mut spans,
                Block::ListItem {
                    ordered,
                    depth,
                    marker,
                    checked,
                    text,
                },
                offset + i,
                offset + i,
            );
            i += 1;
            continue;
        }

        paragraph.push(line, i);
        i += 1;
    }

    flush_paragraph(&mut paragraph, &mut blocks, &mut spans, offset);
    if !footnotes.is_empty() {
        // The separator is synthetic: pin it to the document's last line.
        let last = offset + lines.len().saturating_sub(1);
        push_block(&mut blocks, &mut spans, Block::Rule, last, last);
        for (block, span) in footnotes {
            blocks.push(block);
            spans.push(span);
        }
    }
    (blocks, spans)
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

/// Whether a source line ends with a hard break: two trailing spaces or a backslash.
fn hard_break(line: &str) -> bool {
    let trimmed = line.trim_end();
    line.len() - trimmed.len() >= 2 || trimmed.ends_with('\\')
}

/// Join the source lines of one paragraph the way a reader shows them.
///
/// Obsidian writes one sentence or clause per line, so a soft line break renders as a
/// space. A line that ends with two spaces or a backslash keeps an explicit line break.
fn join_soft_breaks(lines: &[&str]) -> String {
    let mut out = String::new();
    for (index, line) in lines.iter().enumerate() {
        if index > 0 {
            out.push(if hard_break(lines[index - 1]) {
                '\n'
            } else {
                ' '
            });
        }
        let mut content = line.trim();
        if content.ends_with('\\') {
            // The backslash itself is markup, not text.
            content = content.trim_end_matches('\\').trim_end();
        }
        out.push_str(content);
    }
    out
}

fn flush_paragraph(
    paragraph: &mut ParagraphLines,
    blocks: &mut Vec<Block>,
    spans: &mut Vec<BlockSpan>,
    offset: usize,
) {
    if paragraph.is_empty() {
        return;
    }

    if paragraph.lines.len() == 1 {
        if let Some(spec) = standalone_embed(paragraph.lines[0]) {
            let (first, last) = paragraph.span();
            paragraph.clear();
            push_block(
                blocks,
                spans,
                Block::Embed {
                    target: spec.target,
                    width: spec.width,
                    alt: spec.alt,
                },
                offset + first,
                offset + last,
            );
            return;
        }
    }

    let text = join_soft_breaks(&paragraph.lines);
    let (first, last) = paragraph.span();
    paragraph.clear();
    push_block(
        blocks,
        spans,
        Block::Paragraph(text),
        offset + first,
        offset + last,
    );
}

/// A line that is nothing but an embed, as Obsidian writes embedded images and notes.
#[derive(Debug, Clone, PartialEq, Eq)]
struct EmbedSpec {
    target: String,
    width: Option<u32>,
    alt: Option<String>,
}

fn standalone_embed(line: &str) -> Option<EmbedSpec> {
    let trimmed = line.trim();

    if trimmed.starts_with("![[") && trimmed.ends_with("]]") {
        let inner = &trimmed[3..trimmed.len() - 2];
        let mut parts = inner.split('|');
        let raw_target = parts.next().unwrap_or(inner);
        let target = raw_target
            .split('#')
            .next()
            .unwrap_or(raw_target)
            .trim()
            .to_string();
        if target.is_empty() {
            return None;
        }

        // `![[image.png|300]]` sets a width; `![[image.png|Alt text]]` sets alt text.
        let mut width = None;
        let mut alt = None;
        for part in parts {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            if part.chars().all(|ch| ch.is_ascii_digit()) {
                width = part
                    .parse::<u32>()
                    .ok()
                    .filter(|width| *width > 0)
                    .map(|width| width.min(2000));
            } else {
                alt = Some(part.to_string());
            }
        }
        return Some(EmbedSpec { target, width, alt });
    }

    if let Some(rest) = trimmed.strip_prefix("![") {
        if let Some(end) = rest.find("](") {
            if let Some(source) = rest[end + 2..].strip_suffix(')') {
                let source = source.trim();
                if !source.is_empty() {
                    let alt = rest[..end].trim();
                    return Some(EmbedSpec {
                        target: source.to_string(),
                        width: None,
                        alt: (!alt.is_empty()).then(|| alt.to_string()),
                    });
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
                Block::Paragraph("line one line two".into()),
                Block::Paragraph("next".into()),
            ]
        );
    }

    #[test]
    fn soft_breaks_join_but_hard_breaks_stay() {
        // Two trailing spaces and a trailing backslash are hard breaks; everything else is
        // a soft break that reads as a space, as Obsidian and CommonMark render it.
        let blocks = parse_blocks("one\ntwo  \nthree\\\nfour\n\nfive");
        assert_eq!(
            blocks,
            vec![
                Block::Paragraph("one two\nthree\nfour".into()),
                Block::Paragraph("five".into()),
            ]
        );
    }

    #[test]
    fn quotes_join_soft_breaks_too() {
        let blocks = parse_blocks("> first\n> second\n> third  \n> fourth");
        assert_eq!(
            blocks,
            vec![Block::Quote("first second third\nfourth".into())]
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
                Block::Quote("one two".into()),
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
                    target: "image.png".into(),
                    width: None,
                    alt: None
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
                target: "attachments/pic.png".into(),
                width: None,
                alt: Some("alt".into())
            }]
        );
    }

    #[test]
    fn embed_with_alias_and_heading() {
        let blocks = parse_blocks("![[Note#Section|Alias]]\n");
        assert_eq!(
            blocks,
            vec![Block::Embed {
                target: "Note".into(),
                width: None,
                alt: Some("Alias".into())
            }]
        );
    }

    #[test]
    fn an_embed_keeps_its_width_and_alt_text() {
        assert_eq!(
            parse_blocks("![[shot.png|300]]")[0],
            Block::Embed {
                target: "shot.png".into(),
                width: Some(300),
                alt: None
            }
        );
        assert_eq!(
            parse_blocks("![[shot.png|A screenshot]]")[0],
            Block::Embed {
                target: "shot.png".into(),
                width: None,
                alt: Some("A screenshot".into())
            }
        );
        // Widths are clamped to something sane.
        assert_eq!(
            parse_blocks("![[shot.png|999999]]")[0],
            Block::Embed {
                target: "shot.png".into(),
                width: Some(2000),
                alt: None
            }
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
        // Comment line breaks are preserved, so block spans keep pointing at the right
        // source lines.
        assert_eq!(strip_comments("a %%x\ny%% b"), "a \n b");
        assert_eq!(strip_comments("a %%never closed"), "a ");
        assert_eq!(
            strip_comments("%%one\ntwo\nthree%%\nbody").lines().count(),
            4
        );
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
        assert_eq!(
            strip_comments("a <!-- hidden\nstill hidden --> b"),
            "a \n b"
        );
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

    #[test]
    fn blocks_carry_their_source_lines() {
        let text = "---\ntitle: x\n---\n# One\nalpha\nbeta\n\n```\ncode\n```\n- item\n> quote\n";
        let (blocks, spans) = parse_blocks_spanned(text);
        assert_eq!(blocks.len(), spans.len());

        // Frontmatter lines are counted, so spans point at the buffer.
        assert_eq!(
            spans[0],
            BlockSpan {
                first_line: 3,
                last_line: 3
            }
        );
        assert!(matches!(blocks[0], Block::Heading { .. }));

        // A paragraph spans both of its soft-wrapped lines.
        assert_eq!(
            spans[1],
            BlockSpan {
                first_line: 4,
                last_line: 5
            }
        );

        // A fence spans from the opening to the closing fence.
        assert_eq!(
            spans[2],
            BlockSpan {
                first_line: 7,
                last_line: 9
            }
        );
        assert!(matches!(blocks[2], Block::Code { .. }));

        assert_eq!(
            spans[3],
            BlockSpan {
                first_line: 10,
                last_line: 10
            }
        );
        assert_eq!(
            spans[4],
            BlockSpan {
                first_line: 11,
                last_line: 11
            }
        );
    }

    #[test]
    fn a_multiline_table_and_rule_keep_their_lines() {
        let text = "| a | b |\n|---|---|\n| 1 | 2 |\n---\n";
        let (blocks, spans) = parse_blocks_spanned(text);
        assert!(matches!(blocks[0], Block::Table { .. }));
        assert_eq!(
            spans[0],
            BlockSpan {
                first_line: 0,
                last_line: 2
            }
        );
        assert_eq!(blocks[1], Block::Rule);
        assert_eq!(
            spans[1],
            BlockSpan {
                first_line: 3,
                last_line: 3
            }
        );
    }

    #[test]
    fn setext_headings_span_the_paragraph_and_underline() {
        let (blocks, spans) = parse_blocks_spanned("title\n=====\n\nbody\n");
        assert!(matches!(blocks[0], Block::Heading { .. }));
        assert_eq!(
            spans[0],
            BlockSpan {
                first_line: 0,
                last_line: 1
            }
        );
        assert_eq!(
            spans[1],
            BlockSpan {
                first_line: 3,
                last_line: 3
            }
        );
    }

    #[test]
    fn comment_lines_do_not_shift_block_spans() {
        let text = "%%one\ntwo%%\n# After\nbody\n";
        let (blocks, spans) = parse_blocks_spanned(text);
        assert!(matches!(blocks[0], Block::Heading { .. }));
        assert_eq!(
            spans[0],
            BlockSpan {
                first_line: 2,
                last_line: 2
            }
        );
        assert_eq!(
            spans[1],
            BlockSpan {
                first_line: 3,
                last_line: 3
            }
        );
    }
}
