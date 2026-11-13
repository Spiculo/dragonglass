//! Inline markdown: the formatting *inside* a paragraph, heading, list item or table
//! cell, as a flat list of styled spans.
//!
//! The block parser in `markdown` decides where paragraphs, lists and tables are; this
//! module decides how their text is decorated. It covers the inline syntax a GitHub or
//! Obsidian note actually uses: emphasis, strong, strikethrough, inline code, links and
//! autolinks (including `[[wikilinks]]`), `==highlights==`, and `$math$`/`$$math$$`.
//!
//! Spans are flat on purpose: the reading view and the HTML exporter both need to turn
//! them into styled runs, and neither wants to walk a tree to find out which font to use.

use crate::math;

/// Which inline decorations apply to a run of text.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
    pub strikethrough: bool,
    pub code: bool,
    pub highlight: bool,
    pub math: bool,
    /// Link text, either `[text](url)` or a `[[wikilink]]`.
    pub link: bool,
    /// The link points at another note by name (`[[Note]]`) rather than at a URL.
    pub wiki: bool,
    /// A `[^id]` footnote reference.
    pub footnote: bool,
}

/// One run of inline text with a single style.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inline {
    pub text: String,
    pub style: Style,
    /// Link destination, when `style.link` is set.
    pub href: Option<String>,
    /// Original TeX for a math span (without delimiters). `text` holds the plain-text
    /// transliteration, which is what search, outline entries and copy fallbacks use.
    pub math: Option<String>,
}

/// Parse inline markdown into styled spans, in document order.
pub fn parse(text: &str) -> Vec<Inline> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    parse_into(&chars, Style::default(), None, &mut out);
    merge(out)
}

/// Inline text with the markup removed: what an outline entry or a tooltip should show.
pub fn plain(text: &str) -> String {
    parse(text)
        .into_iter()
        .map(|span| span.text)
        .collect::<String>()
        .replace('\n', " ")
}

fn parse_into(chars: &[char], base: Style, href: Option<&str>, out: &mut Vec<Inline>) {
    let mut buffer = String::new();
    let mut i = 0;

    while i < chars.len() {
        let ch = chars[i];

        // Backslash escapes any character.
        if ch == '\\' && i + 1 < chars.len() {
            buffer.push(chars[i + 1]);
            i += 2;
            continue;
        }

        // A newline is a space, unless the line ends in two spaces or a backslash.
        if ch == '\n' {
            let hard = buffer.ends_with("  ") || buffer.ends_with('\\');
            while buffer.ends_with(' ') {
                buffer.pop();
            }
            if buffer.ends_with('\\') {
                buffer.pop();
            }
            buffer.push(if hard { '\n' } else { ' ' });
            i += 1;
            while i < chars.len() && chars[i] == ' ' {
                i += 1;
            }
            continue;
        }

        // Inline code: copied verbatim, no nested formatting.
        if ch == '`' {
            let run = run_len(chars, i, '`');
            if let Some(close) = find_exact_run(chars, i + run, '`', run) {
                flush(&mut buffer, base, href, out);
                let content: String = chars[i + run..close].iter().collect();
                let mut style = base;
                style.code = true;
                push_span(out, content.replace('\n', " "), style, href);
                i = close + run;
                continue;
            }
            push_run(&mut buffer, '`', run);
            i += run;
            continue;
        }

        // Math: `$…$` inline, `$$…$$` display (also valid here).
        if ch == '$' {
            let width = if chars.get(i + 1) == Some(&'$') { 2 } else { 1 };
            if let Some(close) = math_close(chars, i + width, width) {
                let inner: String = chars[i + width..close].iter().collect();
                if !inner.trim().is_empty()
                    && !inner.starts_with(char::is_whitespace)
                    && !inner.ends_with(char::is_whitespace)
                {
                    flush(&mut buffer, base, href, out);
                    let mut style = base;
                    style.math = true;
                    push_span(
                        out,
                        math::to_unicode(inner.trim()).replace('\n', " "),
                        style,
                        href,
                    );
                    if let Some(span) = out.last_mut() {
                        span.math = Some(inner.trim().to_string());
                    }
                    i = close + width;
                    continue;
                }
            }
            push_run(&mut buffer, '$', width);
            i += width;
            continue;
        }

        // `==highlight==`.
        if ch == '=' && chars.get(i + 1) == Some(&'=') {
            if let Some(close) = find_exact_run(chars, i + 2, '=', 2) {
                if close > i + 2 && chars[i + 2..close].iter().any(|ch| !ch.is_whitespace()) {
                    flush(&mut buffer, base, href, out);
                    let mut style = base;
                    style.highlight = true;
                    parse_into(&chars[i + 2..close], style, href, out);
                    i = close + 2;
                    continue;
                }
            }
            buffer.push('=');
            i += 1;
            continue;
        }

        // `~~strikethrough~~`.
        if ch == '~' && chars.get(i + 1) == Some(&'~') {
            if let Some(close) = find_exact_run(chars, i + 2, '~', 2) {
                if close > i + 2 && chars[i + 2..close].iter().any(|ch| !ch.is_whitespace()) {
                    flush(&mut buffer, base, href, out);
                    let mut style = base;
                    style.strikethrough = true;
                    parse_into(&chars[i + 2..close], style, href, out);
                    i = close + 2;
                    continue;
                }
            }
            buffer.push('~');
            i += 1;
            continue;
        }

        // Emphasis: `***both***`, `**strong**`, `*em*` (and the `_` forms).
        if ch == '*' || ch == '_' {
            let run = run_len(chars, i, ch).min(3);
            let opened = (1..=run).rev().find_map(|len| {
                if !opens(chars, i, len, ch) {
                    return None;
                }
                find_closing(chars, i + len, ch, len).map(|close| (len, close))
            });

            if let Some((len, close)) = opened {
                flush(&mut buffer, base, href, out);
                let mut style = base;
                if len >= 2 {
                    style.bold = true;
                }
                if len != 2 {
                    style.italic = true;
                }
                parse_into(&chars[i + len..close], style, href, out);
                i = close + len;
                continue;
            }

            let width = run_len(chars, i, ch).min(3);
            push_run(&mut buffer, ch, width);
            i += width;
            continue;
        }

        // `[^1]` footnote reference.
        if ch == '[' && chars.get(i + 1) == Some(&'^') {
            if let Some(close) = find(chars, i + 2, &[']'][..]) {
                let id: String = chars[i + 2..close].iter().collect();
                let id = id.trim();
                if !id.is_empty() {
                    flush(&mut buffer, base, href, out);
                    let mut style = base;
                    style.footnote = true;
                    push_span(out, id.to_string(), style, None);
                    i = close + 1;
                    continue;
                }
            }
        }

        // `[[wikilink]]` / `![[embed]]`.
        if ch == '[' && chars.get(i + 1) == Some(&'[') {
            if let Some(close) = find(chars, i + 2, &[']', ']'][..]) {
                let inner: String = chars[i + 2..close].iter().collect();
                flush(&mut buffer, base, href, out);
                let mut style = base;
                style.link = true;
                style.wiki = true;
                let (target, label) = split_wikilink(&inner);
                push_span(out, label, style, Some(&target));
                i = close + 2;
                continue;
            }
        }

        // `[label](url)` and `![alt](src)`.
        if (ch == '[' || (ch == '!' && chars.get(i + 1) == Some(&'[')))
            && chars.get(i + 1) != Some(&'[')
        {
            let open = if ch == '!' { i + 1 } else { i };
            if chars.get(open + 1) != Some(&'[') {
                if let Some((label_end, dest_end)) = link_bounds(chars, open) {
                    let label: String = chars[open + 1..label_end].iter().collect();
                    let destination: String = chars[label_end + 2..dest_end].iter().collect();
                    let href = link_destination(&destination);
                    flush(&mut buffer, base, href.as_deref(), out);
                    let mut style = base;
                    style.link = true;
                    if label.trim().is_empty() {
                        push_span(out, destination.trim().to_string(), style, href.as_deref());
                    } else {
                        let label_chars: Vec<char> = label.chars().collect();
                        parse_into(&label_chars, style, href.as_deref(), out);
                    }
                    i = dest_end + 1;
                    continue;
                }
            }
        }

        // `<https://…>` and `<user@example.com>`.
        if ch == '<' {
            if let Some(end) = find(chars, i + 1, &['>'][..]) {
                let inner: String = chars[i + 1..end].iter().collect();
                if is_url(&inner) || is_email(&inner) {
                    flush(&mut buffer, base, href, out);
                    let mut style = base;
                    style.link = true;
                    push_span(out, inner.clone(), style, Some(&inner));
                    i = end + 1;
                    continue;
                }
            }
        }

        // Bare URLs.
        if (ch == 'h' || ch == 'w') && at_word_start(chars, i) {
            if let Some(len) = bare_url_len(chars, i) {
                flush(&mut buffer, base, href, out);
                let url: String = chars[i..i + len].iter().collect();
                let mut style = base;
                style.link = true;
                let href = if url.starts_with("www.") {
                    format!("https://{url}")
                } else {
                    url.clone()
                };
                push_span(out, url, style, Some(&href));
                i += len;
                continue;
            }
        }

        buffer.push(ch);
        i += 1;
    }

    flush(&mut buffer, base, href, out);
}

fn split_wikilink(inner: &str) -> (String, String) {
    let (raw_target, alias) = match inner.split_once('|') {
        Some((target, alias)) => (target, Some(alias)),
        None => (inner, None),
    };
    // Keep `#heading` / `#^block` in the target: the opener resolves it after loading.
    let target = raw_target.trim().to_string();
    let (label_target, fragment) = match target.split_once('#') {
        Some((note, fragment)) => (note.trim().to_string(), Some(fragment.trim())),
        None => (target.clone(), None),
    };
    let label = alias
        .map(str::trim)
        .filter(|label| !label.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            if !label_target.is_empty() {
                label_target
            } else {
                // `[[#Local]]` shows the fragment, since there is no note name.
                fragment
                    .map(|fragment| format!("#{fragment}"))
                    .unwrap_or_default()
            }
        });
    (target, label)
}

/// Byte-scan `[label](dest)`: returns `(index of ']', index of ')')`.
fn link_bounds(chars: &[char], open: usize) -> Option<(usize, usize)> {
    let close = find(chars, open + 1, &[']'][..])?;
    if chars.get(close + 1) != Some(&'(') {
        return None;
    }
    let end = find(chars, close + 2, &[')'][..])?;
    Some((close, end))
}

fn link_destination(destination: &str) -> Option<String> {
    let destination = destination.trim();
    let destination = destination
        .split_whitespace()
        .next()
        .unwrap_or(destination)
        .trim_matches(|ch| ch == '<' || ch == '>');
    if destination.is_empty() {
        None
    } else {
        Some(destination.to_string())
    }
}

fn merge(spans: Vec<Inline>) -> Vec<Inline> {
    let mut merged: Vec<Inline> = Vec::with_capacity(spans.len());
    for span in spans {
        if span.text.is_empty() {
            continue;
        }
        match merged.last_mut() {
            Some(last)
                if last.style == span.style && last.href == span.href && last.math == span.math =>
            {
                last.text.push_str(&span.text);
            }
            _ => merged.push(span),
        }
    }
    merged
}

fn flush(buffer: &mut String, style: Style, href: Option<&str>, out: &mut Vec<Inline>) {
    if buffer.is_empty() {
        return;
    }
    push_span(out, std::mem::take(buffer), style, href);
}

fn push_span(out: &mut Vec<Inline>, text: String, style: Style, href: Option<&str>) {
    if text.is_empty() {
        return;
    }
    out.push(Inline {
        text,
        style,
        href: href.map(str::to_string),
        math: None,
    });
}

fn push_run(buffer: &mut String, ch: char, count: usize) {
    for _ in 0..count {
        buffer.push(ch);
    }
}

fn run_len(chars: &[char], at: usize, ch: char) -> usize {
    chars[at..]
        .iter()
        .take_while(|candidate| **candidate == ch)
        .count()
}

/// Find the next run of exactly `len` `ch` characters at or after `from`.
fn find_exact_run(chars: &[char], from: usize, ch: char, len: usize) -> Option<usize> {
    let mut i = from;
    while i < chars.len() {
        if chars[i] == '\\' {
            i += 2;
            continue;
        }
        if chars[i] == '`' && ch != '`' {
            let run = run_len(chars, i, '`');
            match find_exact_run(chars, i + run, '`', run) {
                Some(close) => {
                    i = close + run;
                    continue;
                }
                None => {
                    i += run;
                    continue;
                }
            }
        }
        if chars[i] == ch {
            let run = run_len(chars, i, ch);
            if run == len {
                return Some(i);
            }
            i += run;
            continue;
        }
        i += 1;
    }
    None
}

fn find(chars: &[char], from: usize, needle: &[char]) -> Option<usize> {
    if needle.is_empty() {
        return None;
    }
    (from..chars.len().saturating_sub(needle.len() - 1))
        .find(|&i| chars.len() >= i + needle.len() && chars[i..i + needle.len()] == *needle)
}

fn math_close(chars: &[char], from: usize, width: usize) -> Option<usize> {
    let mut i = from;
    while i < chars.len() {
        if chars[i] == '\\' {
            i += 2;
            continue;
        }
        if chars[i] == '$' {
            let run = run_len(chars, i, '$');
            if run == width {
                return Some(i);
            }
            i += run;
            continue;
        }
        i += 1;
    }
    None
}

/// Whether an emphasis run can open here: followed by text, and (for `_`) not inside a word.
fn opens(chars: &[char], at: usize, len: usize, ch: char) -> bool {
    if chars.get(at + len).is_none_or(|next| next.is_whitespace()) {
        return false;
    }
    if ch == '_' && at > 0 && chars[at - 1].is_alphanumeric() {
        return false;
    }
    true
}

fn find_closing(chars: &[char], from: usize, ch: char, len: usize) -> Option<usize> {
    let mut i = from;
    while i < chars.len() {
        if chars[i] == '\\' {
            i += 2;
            continue;
        }
        if chars[i] == '`' && ch != '`' {
            let run = run_len(chars, i, '`');
            match find_exact_run(chars, i + run, '`', run) {
                Some(close) => {
                    i = close + run;
                    continue;
                }
                None => {
                    i += run;
                    continue;
                }
            }
        }
        if chars[i] == ch {
            let run = run_len(chars, i, ch);
            let closes = run == len
                && i > from
                && !chars[i - 1].is_whitespace()
                && !chars[from..i].iter().all(|ch| ch.is_whitespace())
                && (ch != '_'
                    || chars
                        .get(i + run)
                        .is_none_or(|next| !next.is_alphanumeric()));
            if closes {
                return Some(i);
            }
            i += run;
            continue;
        }
        i += 1;
    }
    None
}

fn at_word_start(chars: &[char], at: usize) -> bool {
    at == 0 || !chars[at - 1].is_alphanumeric()
}

fn bare_url_len(chars: &[char], at: usize) -> Option<usize> {
    let rest: String = chars[at..].iter().take(8).collect();
    if !(rest.starts_with("http://") || rest.starts_with("https://") || rest.starts_with("www.")) {
        return None;
    }

    let mut len = 0;
    while at + len < chars.len() {
        let ch = chars[at + len];
        if ch.is_whitespace() || matches!(ch, ')' | ']' | '}' | '<' | '>' | '"' | '\'' | ',') {
            break;
        }
        len += 1;
    }

    // Trailing punctuation belongs to the sentence, not the link.
    while len > 0 && matches!(chars[at + len - 1], '.' | ';' | ':' | '!' | '?') {
        len -= 1;
    }

    (len > 0).then_some(len)
}

fn is_url(value: &str) -> bool {
    let Some((scheme, rest)) = value.split_once(':') else {
        return false;
    };
    !scheme.is_empty()
        && !rest.is_empty()
        && scheme
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_alphabetic())
        && scheme
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '+' | '-' | '.'))
}

fn is_email(value: &str) -> bool {
    !value.contains(char::is_whitespace)
        && value.split_once('@').is_some_and(|(user, host)| {
            !user.is_empty() && host.contains('.') && !host.starts_with('.')
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_of(spans: &[Inline]) -> String {
        spans.iter().map(|span| span.text.as_str()).collect()
    }

    fn one<'a>(spans: &'a [Inline], text: &str) -> &'a Inline {
        spans
            .iter()
            .find(|span| span.text == text)
            .unwrap_or_else(|| panic!("no span {text:?} in {spans:?}"))
    }

    #[test]
    fn footnote_references_are_marked() {
        let spans = parse("See[^1].");
        let marker = one(&spans, "1");
        assert!(marker.style.footnote);
        assert_eq!(marker.href, None);
        assert_eq!(text_of(&spans), "See1.");
    }

    #[test]
    fn plain_text_is_one_span() {
        assert_eq!(
            parse("just words"),
            vec![Inline {
                text: "just words".into(),
                style: Style::default(),
                href: None,
                math: None,
            }]
        );
    }

    #[test]
    fn strong_and_emphasis() {
        let spans = parse("a **bold** and *italic* and ***both***");
        assert!(one(&spans, "bold").style.bold);
        assert!(!one(&spans, "bold").style.italic);
        assert!(one(&spans, "italic").style.italic);
        assert!(!one(&spans, "italic").style.bold);
        assert!(one(&spans, "both").style.bold && one(&spans, "both").style.italic);
        assert_eq!(text_of(&spans), "a bold and italic and both");
    }

    #[test]
    fn underscore_forms_do_not_eat_snake_case() {
        assert_eq!(text_of(&parse("__bold__")), "bold");
        assert_eq!(text_of(&parse("snake_case_name")), "snake_case_name");
    }

    #[test]
    fn nested_emphasis_composes() {
        let spans = parse("*a **b** c*");
        let b = one(&spans, "b");
        assert!(b.style.bold && b.style.italic);
    }

    #[test]
    fn strikethrough_and_highlight() {
        let spans = parse("~~gone~~ ==marked==");
        assert!(one(&spans, "gone").style.strikethrough);
        assert!(one(&spans, "marked").style.highlight);
        assert_eq!(text_of(&spans), "gone marked");
    }

    #[test]
    fn inline_code_is_verbatim() {
        let spans = parse("see `**not bold**` now");
        let code = one(&spans, "**not bold**");
        assert!(code.style.code);
        assert!(!code.style.bold);
    }

    #[test]
    fn links_and_wikilinks() {
        let spans = parse("[docs](https://example.com/a) and [[Note|Alias]]");
        let docs = one(&spans, "docs");
        assert!(docs.style.link);
        assert_eq!(docs.href.as_deref(), Some("https://example.com/a"));

        let alias = one(&spans, "Alias");
        assert!(alias.style.link);
        assert_eq!(alias.href.as_deref(), Some("Note"));
    }

    #[test]
    fn wikilinks_keep_heading_anchors_in_the_target() {
        let spans = parse("[[Note#Section]]");
        assert_eq!(spans[0].text, "Note");
        assert_eq!(spans[0].href.as_deref(), Some("Note#Section"));
    }

    #[test]
    fn autolinks_and_bare_urls() {
        let spans = parse("<https://a.example> and https://b.example/x.");
        assert_eq!(spans[0].href.as_deref(), Some("https://a.example"));
        assert!(spans
            .iter()
            .any(|span| span.href.as_deref() == Some("https://b.example/x")));
    }

    #[test]
    fn math_becomes_unicode_without_delimiters() {
        let spans = parse("energy $E = mc^2$ here");
        let math = one(&spans, "E = mc²");
        assert!(math.style.math);
        assert_eq!(text_of(&spans), "energy E = mc² here");
    }

    #[test]
    fn escapes_keep_special_characters() {
        assert_eq!(text_of(&parse("a \\*b\\* c")), "a *b* c");
        assert_eq!(text_of(&parse("cost \\$5")), "cost $5");
    }

    #[test]
    fn unmatched_delimiters_stay_literal() {
        assert_eq!(text_of(&parse("2 * 3 * 4")), "2 * 3 * 4");
        assert_eq!(text_of(&parse("a **b")), "a **b");
        assert_eq!(text_of(&parse("broken $x^2")), "broken $x^2");
    }

    #[test]
    fn soft_breaks_are_spaces_and_hard_breaks_are_newlines() {
        assert_eq!(text_of(&parse("one\ntwo")), "one two");
        assert_eq!(text_of(&parse("one  \ntwo")), "one\ntwo");
        assert_eq!(text_of(&parse("one\\\ntwo")), "one\ntwo");
    }

    #[test]
    fn plain_strips_markup_for_outlines() {
        assert_eq!(plain("**Bold** $x^2$ [[Note|Alias]]"), "Bold x² Alias");
        assert_eq!(plain("**Bold**"), "Bold");
    }

    #[test]
    fn wikilink_targets_keep_their_fragment() {
        let spans = parse("[[Note#Heading|label]]");
        assert_eq!(spans[0].href.as_deref(), Some("Note#Heading"));
        assert_eq!(spans[0].text, "label");

        // Without an alias the visible text is the note name, without the fragment.
        let spans = parse("[[Note#Heading]]");
        assert_eq!(spans[0].href.as_deref(), Some("Note#Heading"));
        assert_eq!(spans[0].text, "Note");

        let spans = parse("[[#Local]]");
        assert_eq!(spans[0].href.as_deref(), Some("#Local"));
        assert_eq!(spans[0].text, "#Local");

        let spans = parse("[[Note#^block-1]]");
        assert_eq!(spans[0].href.as_deref(), Some("Note#^block-1"));
    }
}
