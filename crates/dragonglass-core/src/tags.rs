//! Tag extraction: inline `#tags` and YAML frontmatter `tags:`/`tag:`.

use crate::wikilink::strip_code_fences;

/// Collect all tags from a document body plus optional frontmatter, sorted and deduped.
pub fn extract(body: &str, frontmatter: Option<&str>) -> Vec<String> {
    let mut tags = Vec::new();
    if let Some(fm) = frontmatter {
        tags.extend(from_frontmatter(fm));
    }
    tags.extend(inline(body));
    tags.sort();
    tags.dedup();
    tags
}

/// Inline `#tag` scanning. Skips fenced code and ATX heading markers (`# Title`).
pub fn inline(src: &str) -> Vec<String> {
    let text = strip_code_fences(src);
    let mut out = Vec::new();

    for line in text.lines() {
        if is_atx_heading(line) {
            continue;
        }
        let chars: Vec<char> = line.chars().collect();
        let mut i = 0usize;
        while i < chars.len() {
            if chars[i] == '#' {
                let prev_ok = i == 0 || !chars[i - 1].is_alphanumeric();
                let mut j = i + 1;
                let mut tag = String::new();
                while j < chars.len() && is_tag_char(chars[j]) {
                    tag.push(chars[j]);
                    j += 1;
                }
                if prev_ok && is_valid_tag(&tag) {
                    out.push(tag);
                }
                i = j.max(i + 1);
            } else {
                i += 1;
            }
        }
    }

    out
}

/// Parse `tags:` / `tag:` from raw frontmatter. Supports inline lists and block lists.
pub fn from_frontmatter(frontmatter: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut in_block_list = false;

    for line in frontmatter.lines() {
        let trimmed = line.trim_start();
        let indented = line.len() != trimmed.len();

        if let Some(value) = top_level_value(trimmed, indented) {
            in_block_list = value.trim().is_empty();
            out.extend(parse_inline_list(value));
            continue;
        }

        if in_block_list && indented {
            if let Some(item) = trimmed.strip_prefix("- ") {
                out.extend(parse_inline_list(item));
                continue;
            }
        }

        if !indented || !trimmed.starts_with("- ") {
            in_block_list = false;
        }
    }

    out
}

fn top_level_value(trimmed: &str, indented: bool) -> Option<&str> {
    if indented {
        return None;
    }
    for key in ["tags:", "tag:"] {
        if let Some(v) = trimmed.strip_prefix(key) {
            return Some(v);
        }
    }
    None
}

fn parse_inline_list(value: &str) -> Vec<String> {
    value
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .split(',')
        .map(|s| {
            s.trim()
                .trim_matches(['"', '\''])
                .trim_start_matches('#')
                .to_string()
        })
        .filter(|s| !s.is_empty())
        .collect()
}

fn is_atx_heading(line: &str) -> bool {
    let mut chars = line.chars();
    matches!(chars.next(), Some('#')) && matches!(chars.next(), None | Some(' ') | Some('\t'))
}

fn is_tag_char(c: char) -> bool {
    c.is_alphanumeric() || c == '-' || c == '_' || c == '/'
}

fn is_valid_tag(tag: &str) -> bool {
    !tag.is_empty() && tag.chars().any(|c| !c.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_basic() {
        assert_eq!(inline("a #one b #two/three"), vec!["one", "two/three"]);
    }

    #[test]
    fn skips_headings() {
        assert!(inline("# Heading\n## Another").is_empty());
    }

    #[test]
    fn skips_code() {
        assert!(inline("```\n#notatag\n```").is_empty());
    }

    #[test]
    fn requires_non_numeric() {
        assert!(inline("#123").is_empty());
        assert_eq!(inline("#v2"), vec!["v2"]);
    }

    #[test]
    fn frontmatter_inline_list() {
        let t = from_frontmatter("tags: [alpha, beta]\n");
        assert_eq!(t, vec!["alpha", "beta"]);
    }

    #[test]
    fn frontmatter_block_list() {
        let t = from_frontmatter("tags:\n  - alpha\n  - beta\ntitle: x\n");
        assert_eq!(t, vec!["alpha", "beta"]);
    }

    #[test]
    fn frontmatter_bare() {
        assert_eq!(from_frontmatter("tag: solo\n"), vec!["solo"]);
    }
}
