//! Shared text blocks: single-sourced snippets reused across notes.
//!
//! A shared block lives in `.dragon-glass/shared/<slug>.md`. Notes reference it with
//! `{{shared:slug}}`; the reference is expanded when the note is rendered or exported, so
//! editing the shared block updates every document that uses it without rewriting any of
//! them. References stay visible in source mode, which keeps the markdown honest.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::storage;
use crate::Result;

/// How deep nested shared references may go before expansion stops.
pub const MAX_DEPTH: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharedBlock {
    pub slug: String,
    pub title: String,
    pub path: PathBuf,
    pub bytes: usize,
}

pub fn dir(root: &Path) -> PathBuf {
    root.join(".dragon-glass").join("shared")
}

/// Every shared block, sorted by slug. Titles come from the first heading when present.
pub fn list(root: &Path) -> Vec<SharedBlock> {
    let dir = dir(root);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };

    let mut blocks: Vec<SharedBlock> = entries
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let path = entry.path();
            if !path.is_file()
                || !path
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .map(|extension| extension.eq_ignore_ascii_case("md"))
                    .unwrap_or(false)
            {
                return None;
            }
            let slug = path.file_stem()?.to_string_lossy().to_string();
            if slug.is_empty() {
                return None;
            }
            let text = std::fs::read_to_string(&path).unwrap_or_default();
            Some(SharedBlock {
                title: title_of(&text, &slug),
                bytes: text.len(),
                slug,
                path,
            })
        })
        .collect();

    blocks.sort_by(|a, b| a.slug.cmp(&b.slug));
    blocks
}

pub fn save(root: &Path, slug: &str, content: &str) -> Result<SharedBlock> {
    let id = sanitize(slug);
    let dir = dir(root);
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{id}.md"));
    storage::write_atomic(&path, content)?;

    Ok(SharedBlock {
        title: title_of(content, &id),
        slug: id,
        path,
        bytes: content.len(),
    })
}

pub fn read(root: &Path, slug: &str) -> Result<String> {
    Ok(std::fs::read_to_string(
        dir(root).join(format!("{}.md", sanitize(slug))),
    )?)
}

pub fn remove(root: &Path, slug: &str) -> Result<()> {
    let path = dir(root).join(format!("{}.md", sanitize(slug)));
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

/// Every `{{shared:slug}}` reference in `src`, with 1-based line numbers.
///
/// References inside fenced code blocks are ignored: they are documentation, not embeds.
pub fn references(src: &str) -> Vec<(String, usize)> {
    let mut out = Vec::new();
    let mut in_fence = false;

    for (index, line) in src.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        for slug in line_references(line) {
            out.push((slug, index + 1));
        }
    }

    out
}

/// References whose shared block does not exist.
pub fn missing(src: &str, root: &Path) -> Vec<(String, usize)> {
    references(src)
        .into_iter()
        .filter(|(slug, _)| !dir(root).join(format!("{}.md", sanitize(slug))).is_file())
        .collect()
}

/// Expand every shared reference in `src`.
///
/// Missing blocks and cycles are left as literal references rather than deleting content.
pub fn expand(src: &str, root: &Path) -> String {
    let mut visited = HashSet::new();
    expand_depth(src, root, &mut visited, 0)
}

fn expand_depth(src: &str, root: &Path, visited: &mut HashSet<String>, depth: usize) -> String {
    if depth >= MAX_DEPTH {
        return src.to_string();
    }

    let mut out = String::with_capacity(src.len());
    let mut in_fence = false;

    for line in src.split_inclusive('\n') {
        let trimmed = line.trim_start();
        let fence = trimmed.starts_with("```") || trimmed.starts_with("~~~");
        if fence {
            in_fence = !in_fence;
            out.push_str(line);
            continue;
        }
        if in_fence || !line.contains("{{shared:") {
            out.push_str(line);
            continue;
        }

        let expanded = expand_line(line, root, visited, depth);
        out.push_str(&expanded);
    }

    out
}

fn expand_line(line: &str, root: &Path, visited: &mut HashSet<String>, depth: usize) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;

    while let Some(start) = rest.find("{{shared:") {
        out.push_str(&rest[..start]);
        let after = &rest[start + "{{shared:".len()..];
        let Some(end) = after.find("}}") else {
            out.push_str(&rest[start..]);
            return out;
        };
        let slug = sanitize(&after[..end]);
        let reference = format!("{{{{shared:{slug}}}}}");

        if visited.contains(&slug) || visited.len() >= 64 {
            out.push_str(&reference);
            rest = &after[end + 2..];
            continue;
        }

        let path = dir(root).join(format!("{slug}.md"));
        let Ok(content) = std::fs::read_to_string(path) else {
            out.push_str(&reference);
            rest = &after[end + 2..];
            continue;
        };

        visited.insert(slug.clone());
        let nested = expand_depth(&content, root, visited, depth + 1);
        visited.remove(&slug);
        // The enclosing line supplies its own newline, so inline expansion stays on one
        // line while a block used on its own line keeps its internal line breaks.
        out.push_str(nested.trim_end_matches(['\r', '\n']));

        rest = &after[end + 2..];
    }

    out.push_str(rest);
    out
}

fn line_references(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = line;

    while let Some(start) = rest.find("{{shared:") {
        let after = &rest[start + "{{shared:".len()..];
        let Some(end) = after.find("}}") else {
            break;
        };
        let slug = after[..end].trim();
        if !slug.is_empty() {
            out.push(sanitize(slug));
        }
        rest = &after[end + 2..];
    }

    out
}

/// A file-safe slug, also used for the display name in menus.
pub fn sanitize(name: &str) -> String {
    let mut out = String::new();
    for ch in name.trim().chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            out.push(ch.to_ascii_lowercase());
        } else if (ch.is_whitespace() || ch == '.') && !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_matches('-').to_string();
    let out: String = out.chars().take(64).collect();
    if out.is_empty() {
        "block".to_string()
    } else {
        out
    }
}

fn title_of(content: &str, fallback: &str) -> String {
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("# ") {
            let title = rest.trim();
            if !title.is_empty() {
                return title.to_string();
            }
        }
        if !trimmed.is_empty() {
            break;
        }
    }
    fallback.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "dragon-glass-shared-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn saves_lists_and_reads_blocks() {
        let root = root("save");
        save(&root, "Voice Guide", "# Voice\n\nUse short sentences.\n").unwrap();

        let blocks = list(&root);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].slug, "voice-guide");
        assert_eq!(blocks[0].title, "Voice");
        assert!(read(&root, "voice-guide")
            .unwrap()
            .contains("short sentences"));

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn expands_references_including_inline_ones() {
        let root = root("expand");
        save(&root, "tagline", "Ship it.\n").unwrap();
        let src = "The tagline is {{shared:tagline}} for now.\n";
        assert_eq!(expand(src, &root), "The tagline is Ship it. for now.\n");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn expands_nested_references() {
        let root = root("nested");
        save(&root, "a", "A plus {{shared:b}}\n").unwrap();
        save(&root, "b", "B\n").unwrap();
        assert_eq!(expand("{{shared:a}}\n", &root), "A plus B\n");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn cycles_do_not_recurse_forever() {
        let root = root("cycle");
        save(&root, "a", "{{shared:b}}\n").unwrap();
        save(&root, "b", "{{shared:a}}\n").unwrap();
        let expanded = expand("{{shared:a}}\n", &root);
        assert!(expanded.contains("{{shared:a}}"));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn missing_blocks_are_left_as_references() {
        let root = root("missing");
        let src = "{{shared:absent}}\n";
        assert_eq!(expand(src, &root), src);
        assert_eq!(missing(src, &root), vec![("absent".to_string(), 1)]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn references_in_code_fences_are_ignored() {
        let root = root("fence");
        save(&root, "x", "X\n").unwrap();
        let src = "```\n{{shared:x}}\n```\n";
        assert_eq!(expand(src, &root), src);
        assert!(references(src).is_empty());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn finds_reference_lines() {
        let src = "a {{shared:one}}\n\n{{shared:two}}\n";
        assert_eq!(
            references(src),
            vec![("one".to_string(), 1), ("two".to_string(), 3)]
        );
    }

    #[test]
    fn slugs_are_filesystem_safe() {
        assert_eq!(sanitize("Voice Guide"), "voice-guide");
        assert_eq!(sanitize("../x"), "x");
        assert_eq!(sanitize("***"), "block");
    }
}
