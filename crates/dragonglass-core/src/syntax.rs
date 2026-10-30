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

/// Kinds produced by the lightweight code highlighter used by reading mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodeKind {
    Text,
    Keyword,
    String,
    Comment,
    Number,
    Function,
    Type,
    Punctuation,
}

/// One coloured run inside a code line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeSpan {
    pub text: String,
    pub kind: CodeKind,
}

/// Colour a single code line. This is deliberately a lexer, not a parser: it recognises
/// comments, strings, numbers, keywords, calls and type-like names, which is enough to make
/// code in the reading view scannable without pulling in a language server.
pub fn code_spans(line: &str, language: &str) -> Vec<CodeSpan> {
    let language = Language::from(language);
    let chars: Vec<char> = line.chars().collect();
    let len = chars.len();
    let mut spans: Vec<CodeSpan> = Vec::new();
    let mut i = 0usize;

    while i < len {
        let ch = chars[i];

        if ch.is_whitespace() {
            push_span(&mut spans, &ch.to_string(), CodeKind::Text);
            i += 1;
            continue;
        }

        if let Some(end) = line_comment_end(&chars, i, language) {
            let text: String = chars[i..end].iter().collect();
            push_span(&mut spans, &text, CodeKind::Comment);
            i = end;
            continue;
        }

        if language.has_block_comments() && starts_at(&chars, i, "/*") {
            let end = find_from(&chars, i + 2, "*/")
                .map(|position| position + 2)
                .unwrap_or(len);
            let text: String = chars[i..end].iter().collect();
            push_span(&mut spans, &text, CodeKind::Comment);
            i = end;
            continue;
        }

        if matches!(ch, '"' | '\'' | '`') {
            let end = string_end(&chars, i, ch);
            let text: String = chars[i..end].iter().collect();
            push_span(&mut spans, &text, CodeKind::String);
            i = end;
            continue;
        }

        if ch.is_ascii_digit() {
            let mut end = i + 1;
            while end < len
                && (chars[end].is_ascii_alphanumeric()
                    || matches!(chars[end], '.' | '_' | 'x' | 'X'))
            {
                end += 1;
            }
            let text: String = chars[i..end].iter().collect();
            push_span(&mut spans, &text, CodeKind::Number);
            i = end;
            continue;
        }

        if ch.is_alphabetic() || ch == '_' {
            let mut end = i + 1;
            while end < len && (chars[end].is_alphanumeric() || chars[end] == '_') {
                end += 1;
            }
            let word: String = chars[i..end].iter().collect();

            let kind = if language.is_keyword(&word) {
                CodeKind::Keyword
            } else if next_non_space(&chars, end) == Some('(') {
                CodeKind::Function
            } else if word.chars().next().is_some_and(char::is_uppercase) {
                CodeKind::Type
            } else {
                CodeKind::Text
            };

            push_span(&mut spans, &word, kind);
            i = end;
            continue;
        }

        push_span(&mut spans, &ch.to_string(), CodeKind::Punctuation);
        i += 1;
    }

    spans
}

fn push_span(spans: &mut Vec<CodeSpan>, text: &str, kind: CodeKind) {
    if text.is_empty() {
        return;
    }
    match spans.last_mut() {
        Some(last) if last.kind == kind => last.text.push_str(text),
        _ => spans.push(CodeSpan {
            text: text.to_string(),
            kind,
        }),
    }
}

fn next_non_space(chars: &[char], mut at: usize) -> Option<char> {
    while at < chars.len() {
        if !chars[at].is_whitespace() {
            return Some(chars[at]);
        }
        at += 1;
    }
    None
}

fn string_end(chars: &[char], start: usize, quote: char) -> usize {
    let mut i = start + 1;
    while i < chars.len() {
        if chars[i] == '\\' {
            i += 2;
            continue;
        }
        if chars[i] == quote {
            return i + 1;
        }
        i += 1;
    }
    chars.len()
}

fn line_comment_end(chars: &[char], start: usize, language: Language) -> Option<usize> {
    match language {
        Language::Rust | Language::JavaScript | Language::C | Language::Go
            if starts_at(chars, start, "//") =>
        {
            Some(chars.len())
        }
        Language::Python | Language::Shell | Language::Yaml | Language::Toml
            if chars[start] == '#' =>
        {
            Some(chars.len())
        }
        Language::Sql if starts_at(chars, start, "--") => Some(chars.len()),
        Language::Html if starts_at(chars, start, "<!--") => Some(chars.len()),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Language {
    Rust,
    JavaScript,
    Python,
    C,
    Go,
    Shell,
    Json,
    Yaml,
    Toml,
    Html,
    Css,
    Sql,
    Other,
}

impl Language {
    fn from(name: &str) -> Self {
        match name.trim().to_ascii_lowercase().as_str() {
            "rust" | "rs" => Language::Rust,
            "js" | "javascript" | "jsx" | "ts" | "typescript" | "tsx" => Language::JavaScript,
            "python" | "py" => Language::Python,
            "c" | "cpp" | "c++" | "h" | "hpp" | "objc" => Language::C,
            "go" | "golang" => Language::Go,
            "sh" | "shell" | "bash" | "zsh" | "fish" => Language::Shell,
            "json" => Language::Json,
            "yaml" | "yml" => Language::Yaml,
            "toml" => Language::Toml,
            "html" | "htm" | "xml" => Language::Html,
            "css" | "scss" | "less" => Language::Css,
            "sql" => Language::Sql,
            _ => Language::Other,
        }
    }

    fn has_block_comments(self) -> bool {
        matches!(
            self,
            Language::Rust | Language::JavaScript | Language::C | Language::Go | Language::Css
        )
    }

    fn is_keyword(self, word: &str) -> bool {
        const RUST: &[&str] = &[
            "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum",
            "extern", "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod",
            "move", "mut", "pub", "ref", "return", "self", "Self", "static", "struct", "super",
            "trait", "true", "type", "unsafe", "use", "where", "while", "yield",
        ];
        const JS: &[&str] = &[
            "async",
            "await",
            "break",
            "case",
            "catch",
            "class",
            "const",
            "continue",
            "debugger",
            "default",
            "delete",
            "do",
            "else",
            "export",
            "extends",
            "false",
            "finally",
            "for",
            "from",
            "function",
            "if",
            "import",
            "in",
            "instanceof",
            "let",
            "new",
            "null",
            "of",
            "return",
            "static",
            "super",
            "switch",
            "this",
            "throw",
            "true",
            "try",
            "typeof",
            "undefined",
            "var",
            "void",
            "while",
            "with",
            "yield",
        ];
        const PYTHON: &[&str] = &[
            "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del",
            "elif", "else", "except", "False", "finally", "for", "from", "global", "if", "import",
            "in", "is", "lambda", "None", "nonlocal", "not", "or", "pass", "raise", "return",
            "True", "try", "while", "with", "yield",
        ];
        const C: &[&str] = &[
            "auto",
            "break",
            "case",
            "char",
            "const",
            "continue",
            "default",
            "do",
            "double",
            "else",
            "enum",
            "extern",
            "float",
            "for",
            "goto",
            "if",
            "inline",
            "int",
            "long",
            "namespace",
            "new",
            "return",
            "short",
            "signed",
            "sizeof",
            "static",
            "struct",
            "switch",
            "template",
            "this",
            "throw",
            "try",
            "typedef",
            "typename",
            "union",
            "unsigned",
            "using",
            "virtual",
            "void",
            "volatile",
            "while",
        ];
        const GO: &[&str] = &[
            "break",
            "case",
            "chan",
            "const",
            "continue",
            "default",
            "defer",
            "else",
            "fallthrough",
            "for",
            "func",
            "go",
            "goto",
            "if",
            "import",
            "interface",
            "map",
            "package",
            "range",
            "return",
            "select",
            "struct",
            "switch",
            "type",
            "var",
        ];
        const SHELL: &[&str] = &[
            "case", "do", "done", "elif", "else", "esac", "fi", "for", "function", "if", "in",
            "select", "then", "until", "while",
        ];
        const SQL: &[&str] = &[
            "ADD", "ALTER", "AND", "AS", "ASC", "BEGIN", "BETWEEN", "BY", "CASE", "CREATE",
            "DELETE", "DESC", "DISTINCT", "DROP", "ELSE", "END", "FROM", "GROUP", "HAVING", "IN",
            "INSERT", "INTO", "JOIN", "LEFT", "LIMIT", "NOT", "NULL", "ON", "OR", "ORDER", "OUTER",
            "RIGHT", "SELECT", "SET", "TABLE", "THEN", "UNION", "UPDATE", "VALUES", "WHEN",
            "WHERE",
        ];

        match self {
            Language::Rust => RUST.contains(&word),
            Language::JavaScript => JS.contains(&word),
            Language::Python => PYTHON.contains(&word),
            Language::C => C.contains(&word),
            Language::Go => GO.contains(&word),
            Language::Shell => SHELL.contains(&word),
            Language::Sql => SQL.iter().any(|keyword| keyword.eq_ignore_ascii_case(word)),
            Language::Json => matches!(word, "true" | "false" | "null"),
            Language::Yaml | Language::Toml => matches!(word, "true" | "false" | "null"),
            Language::Html | Language::Css | Language::Other => false,
        }
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

    fn code(line: &str, language: &str) -> Vec<CodeSpan> {
        code_spans(line, language)
    }

    #[test]
    fn highlights_rust_keywords_calls_numbers_and_comments() {
        let spans = code("fn main() { let x = 42; // note", "rust");
        assert!(spans
            .iter()
            .any(|span| span.text == "fn" && span.kind == CodeKind::Keyword));
        assert!(spans
            .iter()
            .any(|span| span.text == "main" && span.kind == CodeKind::Function));
        assert!(spans
            .iter()
            .any(|span| span.text == "42" && span.kind == CodeKind::Number));
        assert!(spans
            .iter()
            .any(|span| span.kind == CodeKind::Comment && span.text.contains("note")));
    }

    #[test]
    fn highlights_strings_and_python_comments() {
        let spans = code("name = \"dragon\" # a comment", "python");
        assert!(spans
            .iter()
            .any(|span| span.text == "\"dragon\"" && span.kind == CodeKind::String));
        assert!(spans
            .iter()
            .any(|span| span.kind == CodeKind::Comment && span.text.contains("a comment")));
    }

    #[test]
    fn type_names_are_distinguished_from_text() {
        let spans = code("let value: Vec<String> = Vec::new();", "rust");
        assert!(spans
            .iter()
            .any(|span| span.text == "Vec" && span.kind == CodeKind::Type));
    }

    #[test]
    fn unknown_languages_still_produce_lossless_spans() {
        let line = "plain text with 3 numbers";
        let spans = code_spans(line, "unknown");
        let rebuilt: String = spans.into_iter().map(|span| span.text).collect();
        assert_eq!(rebuilt, line);
    }
}
