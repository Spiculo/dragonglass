//! Editorial review: cheap, deterministic checks over a note.
//!
//! These are not style-guide rules imposed on the writer; they are the papercuts an editor
//! wants surfaced while working — broken links, unmatched fences, empty or duplicate
//! headings, unfinished markers, and paragraphs that have grown too long to scan.

use std::collections::{HashMap, HashSet};

use crate::frontmatter::{self, PropertyValue};
use crate::relations;
use crate::resolve::NoteIndex;
use crate::wikilink;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Info,
    Warning,
    Error,
}

impl Severity {
    pub fn label(self) -> &'static str {
        match self {
            Severity::Info => "info",
            Severity::Warning => "warning",
            Severity::Error => "error",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issue {
    /// 1-based source line.
    pub line: usize,
    pub severity: Severity,
    pub rule: &'static str,
    pub message: String,
}

/// Review a document. `known_notes` are vault-relative paths; link targets are resolved
/// against their paths and stems.
pub fn review(src: &str, known_notes: &[String]) -> Vec<Issue> {
    review_with(src, &NoteIndex::from_paths(known_notes.to_vec()))
}

/// Review a document, resolving link targets through a vault index that also knows
/// frontmatter slugs and aliases.
pub fn review_with(src: &str, index: &NoteIndex) -> Vec<Issue> {
    let resolver = index;
    let body = frontmatter::split(src).body;
    let line_offset = src[..src.len().saturating_sub(body.len())]
        .matches('\n')
        .count();

    let mut issues = Vec::new();
    let mut in_fence = false;
    let mut fence_start = 0usize;
    let mut headings: HashSet<String> = HashSet::new();
    let mut last_level: Option<usize> = None;
    let mut paragraph_start: Option<usize> = None;
    let mut paragraph_len = 0usize;
    let mut saw_heading = false;
    let mut in_comment = false;
    let mut comment_start = 0usize;

    for (index, line) in body.lines().enumerate() {
        let line_number = line_offset + index + 1;
        let trimmed = line.trim();
        let fence = trimmed.starts_with("```") || trimmed.starts_with("~~~");

        if fence {
            if !in_fence {
                in_fence = true;
                fence_start = line_number;
            } else {
                in_fence = false;
            }
            flush(
                &mut issues,
                &mut paragraph_start,
                &mut paragraph_len,
                line_number,
            );
            continue;
        }

        if in_fence {
            continue;
        }

        if in_comment {
            if trimmed.matches("%%").count() % 2 == 1 {
                in_comment = false;
            }
            continue;
        }

        // Strip inline comments before checking the line; an odd number of `%%` opens a
        // comment that continues on the following lines.
        let opens_comment = trimmed.matches("%%").count() % 2 == 1;
        let cleaned = crate::markdown::strip_comments(line);
        let trimmed = cleaned.trim();
        if opens_comment {
            in_comment = true;
            comment_start = line_number;
        }

        if trimmed.is_empty() {
            flush(
                &mut issues,
                &mut paragraph_start,
                &mut paragraph_len,
                line_number,
            );
            continue;
        }

        if let Some((level, text)) = heading(trimmed) {
            flush(
                &mut issues,
                &mut paragraph_start,
                &mut paragraph_len,
                line_number,
            );
            saw_heading = true;

            if text.is_empty() {
                issues.push(Issue {
                    line: line_number,
                    severity: Severity::Warning,
                    rule: "empty-heading",
                    message: "heading has no text".to_string(),
                });
            } else {
                let key = text.to_lowercase();
                if !headings.insert(key) {
                    issues.push(Issue {
                        line: line_number,
                        severity: Severity::Warning,
                        rule: "duplicate-heading",
                        message: format!("duplicate heading: {text}"),
                    });
                }
            }

            if let Some(previous) = last_level {
                if level > previous + 1 {
                    issues.push(Issue {
                        line: line_number,
                        severity: Severity::Info,
                        rule: "heading-jump",
                        message: format!("heading jumps from H{previous} to H{level}"),
                    });
                }
            }
            last_level = Some(level);
            continue;
        }

        paragraph_start.get_or_insert(line_number);
        paragraph_len += trimmed.chars().count();

        for marker in ["TODO", "FIXME", "TBD"] {
            if trimmed.to_lowercase().contains(&marker.to_lowercase()) {
                issues.push(Issue {
                    line: line_number,
                    severity: Severity::Info,
                    rule: "unfinished-marker",
                    message: format!("unfinished marker: {marker}"),
                });
            }
        }

        for link in wikilink::parse(&cleaned) {
            let target = link.target.trim();
            if target.is_empty() || is_asset(target) || resolver.resolve(target).is_some() {
                continue;
            }
            issues.push(Issue {
                line: line_number,
                severity: Severity::Warning,
                rule: "broken-link",
                message: format!("unresolved link target: {target}"),
            });
        }
    }

    if in_fence {
        issues.push(Issue {
            line: fence_start,
            severity: Severity::Error,
            rule: "unclosed-fence",
            message: "code fence is never closed".to_string(),
        });
    }

    if in_comment {
        issues.push(Issue {
            line: comment_start,
            severity: Severity::Warning,
            rule: "unclosed-comment",
            message: "unclosed %% comment".to_string(),
        });
    }

    check_relations(src, resolver, &mut issues);

    flush(
        &mut issues,
        &mut paragraph_start,
        &mut paragraph_len,
        body.lines().count() + line_offset + 1,
    );

    if !saw_heading && !body.trim().is_empty() {
        issues.push(Issue {
            line: 1,
            severity: Severity::Info,
            rule: "no-heading",
            message: "document has no headings".to_string(),
        });
    }

    issues.sort_by(|a, b| {
        a.line
            .cmp(&b.line)
            .then_with(|| b.severity.cmp(&a.severity))
    });
    issues
}

/// Frontmatter relation keys (`up`, `prev`, `related`, …) must point at real notes.
///
/// A typo in `up:` otherwise disappears silently: the relation pane and the relation graph
/// simply omit it. The issue is reported on the key's own line.
fn check_relations(src: &str, resolver: &NoteIndex, issues: &mut Vec<Issue>) {
    let Some(frontmatter) = frontmatter::split(src).frontmatter else {
        return;
    };

    let mut key_lines: HashMap<String, usize> = HashMap::new();
    for (index, line) in frontmatter.lines().enumerate() {
        let Some((key, _)) = line.split_once(':') else {
            continue;
        };
        key_lines
            .entry(key.trim().to_lowercase())
            .or_insert(index + 2);
    }

    for property in frontmatter::properties(src).unwrap_or_default() {
        let Some(kind) = relations::relation_key(&property.key) else {
            continue;
        };
        let line = key_lines
            .get(&property.key.trim().to_lowercase())
            .copied()
            .unwrap_or(1);

        let values = match property.value {
            PropertyValue::Scalar(value) => vec![value],
            PropertyValue::List(items) => items,
            PropertyValue::Raw(_) => Vec::new(),
        };

        let mut reported: HashSet<String> = HashSet::new();
        for raw in values {
            let Some(target) = relations::clean_target(&raw) else {
                continue;
            };
            if is_asset(&target) || resolver.resolve(&target).is_some() {
                continue;
            }
            if !reported.insert(target.to_lowercase()) {
                continue;
            }
            issues.push(Issue {
                line,
                severity: Severity::Warning,
                rule: "broken-relation",
                message: format!("unresolved {} target: {target}", kind.label()),
            });
        }
    }
}

fn flush(
    issues: &mut Vec<Issue>,
    paragraph_start: &mut Option<usize>,
    paragraph_len: &mut usize,
    _line: usize,
) {
    if let Some(start) = paragraph_start.take() {
        if *paragraph_len > 1200 {
            issues.push(Issue {
                line: start,
                severity: Severity::Info,
                rule: "long-paragraph",
                message: format!(
                    "paragraph is {} characters; consider splitting it",
                    *paragraph_len
                ),
            });
        }
    }
    *paragraph_len = 0;
}

fn heading(trimmed: &str) -> Option<(usize, String)> {
    let level = trimmed.chars().take_while(|c| *c == '#').count();
    if level == 0 || level > 6 {
        return None;
    }
    let rest = &trimmed[level..];
    if !rest.is_empty() && !rest.starts_with(' ') {
        return None;
    }
    Some((level, rest.trim().trim_end_matches('#').trim().to_string()))
}

fn is_asset(target: &str) -> bool {
    matches!(
        std::path::Path::new(target)
            .extension()
            .and_then(|extension| extension.to_str())
            .map(|extension| extension.to_ascii_lowercase())
            .as_deref(),
        Some(
            "png"
                | "jpg"
                | "jpeg"
                | "gif"
                | "webp"
                | "bmp"
                | "svg"
                | "avif"
                | "ico"
                | "pdf"
                | "mp3"
                | "wav"
                | "mp4"
                | "webm"
        )
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn known() -> Vec<String> {
        vec!["notes/a.md".to_string(), "b.md".to_string()]
    }

    #[test]
    fn a_clean_document_has_no_issues() {
        let src = "---\ntitle: x\n---\n# Title\n\nSee [[b]] and [[a]].\n\n## Section\n\ntext\n";
        assert!(review(src, &known()).is_empty());
    }

    #[test]
    fn broken_links_are_warnings_with_line_numbers() {
        let src = "# T\n\nsee [[missing]]\n";
        let issues = review(src, &known());
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].rule, "broken-link");
        assert_eq!(issues[0].line, 3);
        assert_eq!(issues[0].severity, Severity::Warning);
    }

    #[test]
    fn image_embeds_are_not_treated_as_broken_notes() {
        let src = "# T\n\n![[shot.png]]\n";
        assert!(review(src, &known()).is_empty());
    }

    #[test]
    fn duplicate_empty_and_jumping_headings_are_reported() {
        let src = "# T\n\n### Deep\n\n## Same\n\n## Same\n\n##\n";
        let issues = review(src, &known());
        assert!(issues.iter().any(|issue| issue.rule == "heading-jump"));
        assert!(issues.iter().any(|issue| issue.rule == "duplicate-heading"));
        assert!(issues.iter().any(|issue| issue.rule == "empty-heading"));
    }

    #[test]
    fn unmatched_fences_are_errors() {
        let src = "# T\n\n```rust\nfn x() {}\n";
        let issues = review(src, &known());
        assert!(issues
            .iter()
            .any(|issue| issue.rule == "unclosed-fence" && issue.severity == Severity::Error));
    }

    #[test]
    fn markers_and_unclosed_comments_are_surfaced() {
        let src = "# T\n\nTODO: finish %%note\n";
        let issues = review(src, &known());
        assert!(issues.iter().any(|issue| issue.rule == "unfinished-marker"));
        assert!(issues.iter().any(|issue| issue.rule == "unclosed-comment"));
    }

    #[test]
    fn long_paragraphs_are_flagged_once() {
        let src = format!("# T\n\n{}\n", "word ".repeat(300));
        let issues = review(&src, &known());
        assert_eq!(
            issues
                .iter()
                .filter(|issue| issue.rule == "long-paragraph")
                .count(),
            1
        );
    }

    #[test]
    fn links_inside_code_are_ignored() {
        let src = "# T\n\n```\n[[missing]]\n```\n";
        assert!(review(src, &known()).is_empty());
    }

    #[test]
    fn broken_relation_targets_are_reported_on_the_key_line() {
        let src = "---\nup: missing\nrelated: [b, missing.png]\n---\n# T\n";
        let issues = review(src, &known());
        let broken: Vec<&Issue> = issues
            .iter()
            .filter(|issue| issue.rule == "broken-relation")
            .collect();
        assert_eq!(broken.len(), 1);
        assert_eq!(broken[0].line, 2);
        assert!(broken[0].message.contains("missing"));
    }

    #[test]
    fn valid_relations_are_quiet() {
        let src = "---\nup: notes/a\nnext: b\nrelated: shot.png\n---\n# T\n";
        assert!(review(src, &known()).is_empty());
    }

    #[test]
    fn non_relation_frontmatter_is_not_checked() {
        let src = "---\ntitle: missing\n---\n# T\n";
        assert!(review(src, &known()).is_empty());
    }

    #[test]
    fn comments_are_ignored_including_multiline_ones() {
        let src = "# T\n\n%% hidden %%\n\n%%\n[[missing]]\nTODO\n%%\n";
        assert!(review(src, &known()).is_empty());
    }
}
