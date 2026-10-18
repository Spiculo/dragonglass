//! Vault discovery: a vault is a directory of markdown files.
//!
//! Mirrors Obsidian's rules loosely: dot-directories (`​.git`, `.dragon-glass`,
//! `.obsidian`, …) are not indexed, and neither are build dirs like `target`.

use std::path::{Path, PathBuf};

use walkdir::WalkDir;

use crate::{Error, Result};

/// A markdown file inside a vault.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteRef {
    /// Absolute path on disk.
    pub path: PathBuf,
    /// Vault-relative path with forward slashes, e.g. `notes/Index.md`.
    pub relative: String,
}

impl NoteRef {
    /// Vault-relative stem, e.g. `Index`.
    pub fn stem(&self) -> &str {
        let name = self.relative.rsplit('/').next().unwrap_or(&self.relative);
        name.rsplit_once('.').map(|(s, _)| s).unwrap_or(name)
    }

    /// Vault-relative path without its extension, e.g. `notes/Index`.
    pub fn path_no_ext(&self) -> &str {
        self.relative
            .rsplit_once('.')
            .map(|(s, _)| s)
            .unwrap_or(&self.relative)
    }
}

#[derive(Debug, Clone)]
pub struct Vault {
    root: PathBuf,
}

impl Vault {
    /// Open a vault rooted at `root`. Canonicalizes so relative lookups are stable.
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref().canonicalize()?;
        if !root.is_dir() {
            return Err(Error::NotADirectory(root));
        }
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// All markdown notes, sorted by vault-relative path.
    pub fn notes(&self) -> Vec<NoteRef> {
        let mut notes: Vec<NoteRef> = WalkDir::new(&self.root)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| !is_skipped(e.path(), &self.root))
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
            .filter(|e| is_markdown(e.path()))
            .map(|e| {
                let path = e.into_path();
                NoteRef {
                    relative: relative_string(&self.root, &path),
                    path,
                }
            })
            .collect();
        notes.sort_by(|a, b| a.relative.cmp(&b.relative));
        notes
    }
}

fn is_skipped(path: &Path, root: &Path) -> bool {
    if path == root {
        return false;
    }
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    name.starts_with('.') || name == "target" || name == "node_modules"
}

fn is_markdown(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("md") || e.eq_ignore_ascii_case("markdown"))
        .unwrap_or(false)
}

fn relative_string(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}
