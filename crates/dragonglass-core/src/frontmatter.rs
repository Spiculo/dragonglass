//! YAML frontmatter detection and splitting.
//!
//! We only *split* the frontmatter here; structured YAML parsing is deferred. This keeps
//! the first cut dependency-free while still giving the index body/frontmatter boundaries.

/// Result of splitting a document into its frontmatter and body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Split<'a> {
    /// Raw YAML between the `---` fences, if present.
    pub frontmatter: Option<&'a str>,
    /// Everything after the closing fence (or the whole document).
    pub body: &'a str,
}

/// Split a document at its leading `---` frontmatter block.
///
/// Only a fence on the very first line counts, matching Obsidian/YAML frontmatter.
/// BOM is tolerated.
pub fn split(src: &str) -> Split<'_> {
    let s = src.strip_prefix('\u{feff}').unwrap_or(src);

    let Some(after_open) = s
        .strip_prefix("---\r\n")
        .or_else(|| s.strip_prefix("---\n"))
    else {
        return Split {
            frontmatter: None,
            body: src,
        };
    };

    let open_len = s.len() - after_open.len();
    let mut cur = open_len;

    for line in after_open.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed == "---" || trimmed == "..." {
            return Split {
                frontmatter: Some(&s[open_len..cur]),
                body: &s[cur + line.len()..],
            };
        }
        cur += line.len();
    }

    // Unterminated fence: treat the whole document as body.
    Split {
        frontmatter: None,
        body: src,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_frontmatter() {
        let s = "# Title\n\nbody\n";
        let r = split(s);
        assert_eq!(r.frontmatter, None);
        assert_eq!(r.body, s);
    }

    #[test]
    fn basic_frontmatter() {
        let s = "---\ntags: [a, b]\n---\n# Title\n";
        let r = split(s);
        assert_eq!(r.frontmatter, Some("tags: [a, b]\n"));
        assert_eq!(r.body, "# Title\n");
    }

    #[test]
    fn unterminated_fence_is_body() {
        let s = "---\ntags: a\n";
        let r = split(s);
        assert_eq!(r.frontmatter, None);
        assert_eq!(r.body, s);
    }

    #[test]
    fn tolerates_bom() {
        let s = "\u{feff}---\ntitle: x\n---\nbody";
        let r = split(s);
        assert_eq!(r.frontmatter, Some("title: x\n"));
        assert_eq!(r.body, "body");
    }
}
