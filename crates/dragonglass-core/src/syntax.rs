//! A small, line-based markdown tokenizer, used to colour the editor.
//!
//! Deliberately not tree-sitter: a full incremental parser is overkill for colouring a line
//! at a time, and this keeps the editor fast and dependency-free. Each call returns one
//! `TokenKind` per character, so the renderer can slice tokens by visual row.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    Text,
    Heading,
    Emphasis,
    Code,
    CodeBlock,
    Link,
    Markup,
    Math,
    Comment,
}

/// True when the line opens or closes a fenced code block.
pub fn is_fence(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with("```") || trimmed.starts_with("~~~")
}

/// One token kind per character of `line`.
pub fn line_tokens(line: &str, in_code_block: bool) -> Vec<TokenKind> {
    let chars: Vec<char> = line.chars().collect();
    let len = chars.len();

    if in_code_block || is_fence(line) {
        return vec![TokenKind::CodeBlock; len];
    }

    let mut kinds = vec![TokenKind::Text; len];

    // A heading colours its whole line.
    let hashes = chars.iter().take_while(|ch| **ch == '#').count();
    if hashes > 0 && hashes <= 6 && matches!(chars.get(hashes), Some(' ') | Some('\t')) {
        return vec![TokenKind::Heading; len];
    }

    let mut i = 0;
    while i < len {
        let (open, close, kind) = if starts_at(&chars, i, "%%") {
            ("%%", "%%", TokenKind::Comment)
        } else if starts_at(&chars, i, "![[") {
            ("![[", "]]", TokenKind::Link)
        } else if starts_at(&chars, i, "[[") {
            ("[[", "]]", TokenKind::Link)
        } else if starts_at(&chars, i, "==") {
            ("==", "==", TokenKind::Markup)
        } else if starts_at(&chars, i, "$$") {
            ("$$", "$$", TokenKind::Math)
        } else if chars[i] == '$' {
            ("$", "$", TokenKind::Math)
        } else if chars[i] == '`' {
            ("`", "`", TokenKind::Code)
        } else if chars[i] == '*' {
            ("*", "*", TokenKind::Emphasis)
        } else if chars[i] == '_' {
            ("_", "_", TokenKind::Emphasis)
        } else {
            i += 1;
            continue;
        };

        let body_start = i + open.chars().count();
        let end = match find_from(&chars, body_start, close) {
            Some(position) => position + close.chars().count(),
            // Unterminated: colour to the end of the line.
            None => len,
        };

        fill(&mut kinds, i, end, kind);
        i = end.max(i + 1);
    }

    kinds
}

fn starts_at(chars: &[char], at: usize, needle: &str) -> bool {
    let needle: Vec<char> = needle.chars().collect();
    chars.len() >= at + needle.len() && chars[at..at + needle.len()] == needle[..]
}

fn find_from(chars: &[char], from: usize, needle: &str) -> Option<usize> {
    let needle: Vec<char> = needle.chars().collect();
    if needle.is_empty() || chars.len() < needle.len() || from > chars.len() - needle.len() {
        return None;
    }
    (from..=chars.len() - needle.len()).find(|&i| chars[i..i + needle.len()] == needle[..])
}

fn fill(kinds: &mut [TokenKind], from: usize, to: usize, kind: TokenKind) {
    let end = to.min(kinds.len());
    for slot in kinds.iter_mut().take(end).skip(from) {
        *slot = kind;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(line: &str) -> Vec<TokenKind> {
        line_tokens(line, false)
    }

    #[test]
    fn plain_text() {
        assert!(kinds("just words").iter().all(|k| *k == TokenKind::Text));
    }

    #[test]
    fn headings_colour_the_whole_line() {
        assert!(kinds("# Title").iter().all(|k| *k == TokenKind::Heading));
        assert!(kinds("### Deep").iter().all(|k| *k == TokenKind::Heading));
        // A `#` without a following space is not a heading.
        assert!(kinds("#hashtag").iter().all(|k| *k == TokenKind::Text));
    }

    #[test]
    fn inline_spans() {
        let tokens = kinds("a `code` b");
        assert_eq!(tokens.len(), 10);
        assert_eq!(&tokens[0..2], &[TokenKind::Text, TokenKind::Text]);
        assert!(tokens[2..8].iter().all(|kind| *kind == TokenKind::Code));
        assert_eq!(&tokens[8..10], &[TokenKind::Text, TokenKind::Text]);
    }

    #[test]
    fn links_highlights_math_and_comments() {
        assert_eq!(kinds("[[Note]]")[0], TokenKind::Link);
        assert_eq!(kinds("![[img.png]]")[0], TokenKind::Link);
        assert_eq!(kinds("==hi==")[0], TokenKind::Markup);
        assert_eq!(kinds("$x^2$")[0], TokenKind::Math);
        assert_eq!(kinds("$$x$$")[0], TokenKind::Math);
        assert_eq!(kinds("%%secret%%")[0], TokenKind::Comment);
        assert_eq!(kinds("**bold**")[0], TokenKind::Emphasis);
    }

    #[test]
    fn code_fences_colour_their_lines() {
        assert!(kinds("```rust").iter().all(|k| *k == TokenKind::CodeBlock));

        let inside = line_tokens("fn main() {}", true);
        assert!(inside.iter().all(|k| *k == TokenKind::CodeBlock));
    }

    #[test]
    fn unterminated_span_colours_to_end_of_line() {
        let line = kinds("a `unclosed");
        assert_eq!(line[0], TokenKind::Text);
        assert!(line[2..].iter().all(|k| *k == TokenKind::Code));
    }
}
