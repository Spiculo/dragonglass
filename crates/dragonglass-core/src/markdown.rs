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
}

/// Parse a document (frontmatter is skipped) into display blocks.
pub fn parse_blocks(src: &str) -> Vec<Block> {
    let body = frontmatter::split(src).body;
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

            let mut quote = String::new();
            while i < lines.len() {
                let Some(rest) = lines[i].trim_start().strip_prefix('>') else {
                    break;
                };
                if !quote.is_empty() {
                    quote.push('\n');
                }
                quote.push_str(rest.trim_start());
                i += 1;
            }

            blocks.push(Block::Quote(quote));
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
    let text = paragraph.join("\n");
    paragraph.clear();
    blocks.push(Block::Paragraph(text));
}

fn is_fence(trimmed: &str) -> bool {
    trimmed.starts_with("```") || trimmed.starts_with("~~~")
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
}
