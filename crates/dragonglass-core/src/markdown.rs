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
}

/// Remove `%%comment%%` spans, which may run across lines.
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

        if let Some((level, text)) = heading(trimmed) {
            flush_paragraph(&mut paragraph, &mut blocks);
            blocks.push(Block::Heading { level, text });
            i += 1;
            continue;
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

        if let Some((ordered, depth, marker, text)) = list_item(line) {
            flush_paragraph(&mut paragraph, &mut blocks);
            blocks.push(Block::ListItem {
                ordered,
                depth,
                marker,
                text,
            });
            i += 1;
            continue;
        }

        paragraph.push(trimmed);
        i += 1;
    }

    flush_paragraph(&mut paragraph, &mut blocks);
    blocks
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

    for (i, line) in body.lines().enumerate() {
        let trimmed = line.trim();
        if is_fence(trimmed) {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        if let Some((level, text)) = heading(trimmed) {
            out.push(Heading {
                level,
                text,
                line: prefix_lines + i + 1,
            });
        }
    }

    out
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

fn is_rule(trimmed: &str) -> bool {
    let compact: String = trimmed.chars().filter(|c| !c.is_whitespace()).collect();
    compact.len() >= 3
        && (compact.chars().all(|c| c == '-')
            || compact.chars().all(|c| c == '*')
            || compact.chars().all(|c| c == '_'))
}

fn list_item(line: &str) -> Option<(bool, usize, String, String)> {
    let indent = line.len() - line.trim_start().len();
    let depth = indent / 2;
    let trimmed = line.trim_start();

    for bullet in ["- ", "* ", "+ "] {
        if let Some(rest) = trimmed.strip_prefix(bullet) {
            return Some((
                false,
                depth,
                "\u{2022}".to_string(),
                rest.trim().to_string(),
            ));
        }
    }

    let digits_len = trimmed.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits_len > 0 {
        let rest = &trimmed[digits_len..];
        if let Some(text) = rest.strip_prefix(". ") {
            let number = &trimmed[..digits_len];
            return Some((true, depth, format!("{number}."), text.trim().to_string()));
        }
    }

    None
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
                    text: "a".into()
                },
                Block::ListItem {
                    ordered: false,
                    depth: 1,
                    marker: "\u{2022}".into(),
                    text: "nested".into()
                },
                Block::ListItem {
                    ordered: true,
                    depth: 0,
                    marker: "1.".into(),
                    text: "first".into()
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
}
