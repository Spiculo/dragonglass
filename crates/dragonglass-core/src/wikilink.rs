//! `[[wikilink]]` parsing, Obsidian-style.
//!
//! Supports targets, aliases, heading fragments and block references, plus embeds:
//!
//! | Syntax                     | target  | heading | block | alias |
//! |----------------------------|---------|---------|-------|-------|
//! | `[[Note]]`                 | Note    | –       | –     | –     |
//! | `[[Note|Alias]]`           | Note    | –       | –     | Alias |
//! | `[[Note#Heading]]`         | Note    | Heading | –     | –     |
//! | `[[Note#^block]]`          | Note    | –       | block | –     |
//! | `[[Note#Heading|Alias]]`   | Note    | Heading | –     | Alias |
//! | `![[Note]]`                | Note    | –       | –     | – (embed) |
//!
//! Links inside fenced code blocks are ignored.

/// A single `[[...]]` link found in a document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub target: String,
    pub alias: Option<String>,
    pub heading: Option<String>,
    pub block: Option<String>,
    pub embed: bool,
}

/// Parse every wikilink in `src`, skipping fenced code blocks.
pub fn parse(src: &str) -> Vec<Link> {
    let text = strip_code_fences(src);
    let mut links = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0usize;

    while i + 1 < bytes.len() {
        if bytes[i] == b'[' && bytes[i + 1] == b'[' {
            let embed = i > 0 && bytes[i - 1] == b'!';
            let start = i + 2;
            match text[start..].find("]]") {
                Some(rel_end) => {
                    let end = start + rel_end;
                    if let Some(link) = parse_body(&text[start..end], embed) {
                        links.push(link);
                    }
                    i = end + 2;
                }
                None => break,
            }
        } else {
            i += 1;
        }
    }

    links
}

fn parse_body(raw: &str, embed: bool) -> Option<Link> {
    let (target_part, alias) = match raw.split_once('|') {
        Some((t, a)) => (t, Some(a.trim().to_string()).filter(|s| !s.is_empty())),
        None => (raw, None),
    };

    let (target, fragment) = match target_part.split_once('#') {
        Some((t, f)) => (t.trim(), Some(f.trim()).filter(|s| !s.is_empty())),
        None => (target_part.trim(), None),
    };

    if target.is_empty() && fragment.is_none() {
        return None;
    }

    let (heading, block) = match fragment {
        Some(f) if f.starts_with('^') => (None, Some(f[1..].trim().to_string())),
        Some(f) => (Some(f.to_string()), None),
        None => (None, None),
    };

    Some(Link {
        target: target.to_string(),
        alias,
        heading,
        block,
        embed,
    })
}

/// Remove fenced code blocks so their contents are not scanned for links/tags.
pub fn strip_code_fences(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut in_fence = false;

    for line in src.split_inclusive('\n') {
        let trimmed = line.trim_start();
        let is_fence = trimmed.starts_with("```") || trimmed.starts_with("~~~");
        if is_fence {
            in_fence = !in_fence;
            out.push('\n');
            continue;
        }
        if !in_fence {
            out.push_str(line);
        } else {
            out.push('\n');
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(s: &str) -> Link {
        let mut v = parse(s);
        assert_eq!(v.len(), 1, "expected exactly one link in {s:?}");
        v.pop().unwrap()
    }

    #[test]
    fn plain() {
        assert_eq!(one("see [[Note]]").target, "Note");
    }

    #[test]
    fn alias() {
        let l = one("[[Note|my alias]]");
        assert_eq!(l.target, "Note");
        assert_eq!(l.alias.as_deref(), Some("my alias"));
    }

    #[test]
    fn heading() {
        let l = one("[[Note#Section One]]");
        assert_eq!(l.heading.as_deref(), Some("Section One"));
    }

    #[test]
    fn block_ref() {
        let l = one("[[Note#^abc123]]");
        assert_eq!(l.block.as_deref(), Some("abc123"));
        assert_eq!(l.heading, None);
    }

    #[test]
    fn heading_and_alias() {
        let l = one("[[Note#H|Alias]]");
        assert_eq!(l.target, "Note");
        assert_eq!(l.heading.as_deref(), Some("H"));
        assert_eq!(l.alias.as_deref(), Some("Alias"));
    }

    #[test]
    fn embed() {
        assert!(one("![[Image]]").embed);
    }

    #[test]
    fn ignores_code_fences() {
        let s = "```\n[[NotALink]]\n```\n[[Real]]";
        let v = parse(s);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].target, "Real");
    }

    #[test]
    fn empty_target_only_fragment_is_kept() {
        // `[[#heading]]` links within the same file.
        let l = one("[[#Local]]");
        assert_eq!(l.target, "");
        assert_eq!(l.heading.as_deref(), Some("Local"));
    }
}
