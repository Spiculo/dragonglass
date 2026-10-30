//! A light index of the non-note files in a vault.
//!
//! `Vault` indexes markdown; embeds also point at images, PDFs and audio. Obsidian resolves
//! `![[shot.png]]` by file name anywhere in the vault, so the reading view, the HTML exporter
//! and the CLI all need the same lookup. The index is built once per vault and kept small:
//! a map from lowercase path to relative path, plus a map from lowercase file name to every
//! path carrying it.

use std::collections::HashMap;
use std::path::Path;

use walkdir::WalkDir;

/// Vault-relative paths, indexed by full path and by file name.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Attachments {
    by_path: HashMap<String, String>,
    by_name: HashMap<String, Vec<String>>,
    /// Every trailing path segment (`docs/x.png` for `articles/docs/x.png`), so a target
    /// written relative to a content folder still resolves.
    by_suffix: HashMap<String, Vec<String>>,
    /// Frontmatter `Name`/alias keys of markdown files, to their relative paths.
    aliases: HashMap<String, String>,
}

impl Attachments {
    /// Walk `root`, skipping dot-directories and build directories.
    pub fn index(root: &Path) -> Self {
        let mut files = Vec::new();
        for entry in WalkDir::new(root)
            .follow_links(false)
            .into_iter()
            .filter_entry(|entry| !is_skipped(entry.path(), root))
            .filter_map(|entry| entry.ok())
        {
            if !entry.file_type().is_file() {
                continue;
            }
            let relative = entry
                .path()
                .strip_prefix(root)
                .unwrap_or(entry.path())
                .to_string_lossy()
                .replace('\\', "/");
            if !relative.is_empty() {
                files.push(relative);
            }
        }

        let mut index = Self::from_paths(files.clone());

        // Markdown files also answer to their frontmatter slug and aliases.
        for relative in &files {
            let lower = relative.to_ascii_lowercase();
            if !(lower.ends_with(".md") || lower.ends_with(".markdown")) {
                continue;
            }
            let Ok(source) = std::fs::read_to_string(root.join(relative)) else {
                continue;
            };
            for key in crate::resolve::frontmatter_keys(&source) {
                let key = crate::resolve::normalize(&key);
                if !key.is_empty() {
                    index.aliases.entry(key).or_insert_with(|| relative.clone());
                }
            }
        }

        index
    }

    /// Build an index from vault-relative paths, without touching the disk.
    pub fn from_paths<I: IntoIterator<Item = String>>(paths: I) -> Self {
        let mut by_path = HashMap::new();
        let mut by_name: HashMap<String, Vec<String>> = HashMap::new();
        let mut by_suffix: HashMap<String, Vec<String>> = HashMap::new();

        for relative in paths {
            let key = relative.to_lowercase();
            by_name
                .entry(file_name(&key).to_string())
                .or_default()
                .push(relative.clone());
            for suffix in suffixes(&key) {
                by_suffix.entry(suffix).or_default().push(relative.clone());
            }
            by_path.insert(key, relative);
        }

        for paths in by_name.values_mut() {
            paths.sort();
        }

        Self {
            by_path,
            by_name,
            by_suffix,
            aliases: HashMap::new(),
        }
    }

    /// Resolve a link target to a vault-relative path.
    ///
    /// An exact path wins, then a file name, then a markdown file's frontmatter
    /// `Name`/alias. When several files share a name the shallowest path wins, and ties
    /// are broken alphabetically so the result is stable.
    pub fn resolve(&self, target: &str) -> Option<&str> {
        let target = target.trim().replace('\\', "/");
        let target = target.trim_start_matches("./");
        let target = target.trim_start_matches('/');
        if target.is_empty() {
            return None;
        }

        if let Some(path) = self.by_path.get(&target.to_lowercase()) {
            return Some(path);
        }

        if let Some(candidates) = self.by_name.get(file_name(&target.to_lowercase())) {
            return candidates
                .iter()
                .min_by(|a, b| {
                    depth(a)
                        .cmp(&depth(b))
                        .then_with(|| a.to_lowercase().cmp(&b.to_lowercase()))
                })
                .map(String::as_str);
        }

        if target.contains('/') {
            if let Some(candidates) = self.by_suffix.get(&target.to_lowercase()) {
                return candidates
                    .iter()
                    .min_by(|a, b| {
                        depth(a)
                            .cmp(&depth(b))
                            .then_with(|| a.to_lowercase().cmp(&b.to_lowercase()))
                    })
                    .map(String::as_str);
            }
        }

        self.aliases
            .get(&crate::resolve::normalize(target))
            .map(String::as_str)
    }

    pub fn len(&self) -> usize {
        self.by_path.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_path.is_empty()
    }
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Every trailing path at a `/` boundary: `docs/x.png` and `x.png` for
/// `articles/docs/x.png`.
fn suffixes(path: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut start = 0;
    while let Some(index) = path[start..].find('/') {
        start += index + 1;
        out.push(path[start..].to_string());
    }
    out
}

fn depth(path: &str) -> usize {
    path.matches('/').count()
}

fn is_skipped(path: &Path, root: &Path) -> bool {
    if path == root {
        return false;
    }
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    name.starts_with('.') || name == "target" || name == "node_modules"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index() -> Attachments {
        Attachments::from_paths([
            "attachments/shot.png".to_string(),
            "notes/deep/diagram.svg".to_string(),
            "attachments/other/diagram.svg".to_string(),
            "notes/Chapter.md".to_string(),
        ])
    }

    #[test]
    fn exact_paths_win() {
        let index = index();
        assert_eq!(
            index.resolve("attachments/shot.png"),
            Some("attachments/shot.png")
        );
        assert_eq!(
            index.resolve("Attachments/Shot.PNG"),
            Some("attachments/shot.png")
        );
        assert_eq!(
            index.resolve("./attachments/shot.png"),
            Some("attachments/shot.png")
        );
    }

    #[test]
    fn bare_names_resolve_anywhere() {
        let index = index();
        assert_eq!(index.resolve("shot.png"), Some("attachments/shot.png"));
        assert_eq!(index.resolve("Chapter.md"), Some("notes/Chapter.md"));
    }

    #[test]
    fn duplicate_names_prefer_the_shallowest_path() {
        let index = index();
        assert_eq!(
            index.resolve("diagram.svg"),
            Some("attachments/other/diagram.svg")
        );
    }

    #[test]
    fn unknown_targets_are_none() {
        let index = index();
        assert_eq!(index.resolve("missing.png"), None);
        assert_eq!(index.resolve(""), None);
        assert_eq!(Attachments::default().resolve("shot.png"), None);
    }

    #[test]
    fn leading_slashes_and_trailing_paths_resolve() {
        let index = Attachments::from_paths(["articles/assets/docs/e1.png".to_string()]);
        assert_eq!(
            index.resolve("/assets/docs/e1.png"),
            Some("articles/assets/docs/e1.png")
        );
        assert_eq!(
            index.resolve("assets/docs/e1.png"),
            Some("articles/assets/docs/e1.png")
        );
        assert_eq!(
            index.resolve("docs/e1.png"),
            Some("articles/assets/docs/e1.png")
        );
    }

    #[test]
    fn frontmatter_names_resolve_markdown_files() {
        let dir = std::env::temp_dir().join(format!(
            "dragon-glass-assets-names-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("articles")).unwrap();
        std::fs::write(
            dir.join("articles/Git Good.md"),
            "---\nName: git-internals\nAliases: [git]\n---\nbody\n",
        )
        .unwrap();

        let index = Attachments::index(&dir);
        assert_eq!(index.resolve("git-internals"), Some("articles/Git Good.md"));
        assert_eq!(index.resolve("git"), Some("articles/Git Good.md"));
        assert_eq!(index.resolve("Git Good.md"), Some("articles/Git Good.md"));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn indexing_skips_hidden_directories() {
        let dir = std::env::temp_dir().join(format!(
            "dragon-glass-assets-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("attachments")).unwrap();
        std::fs::create_dir_all(dir.join(".dragon-glass")).unwrap();
        std::fs::write(dir.join("attachments/shot.png"), b"png").unwrap();
        std::fs::write(dir.join(".dragon-glass/secret.png"), b"png").unwrap();

        let index = Attachments::index(&dir);
        assert_eq!(index.len(), 1);
        assert_eq!(index.resolve("shot.png"), Some("attachments/shot.png"));
        assert_eq!(index.resolve("secret.png"), None);

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
