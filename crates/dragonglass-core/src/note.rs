//! A loaded note plus its lazily-derived structure.

use std::path::{Path, PathBuf};

use crate::{frontmatter, tags, wikilink, Result};

#[derive(Debug, Clone)]
pub struct Note {
    pub path: PathBuf,
    pub source: String,
}

impl Note {
    /// Read a note from disk. Vault notes are expected to be UTF-8.
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let source = std::fs::read_to_string(&path)?;
        Ok(Self { path, source })
    }

    /// Body text with frontmatter removed.
    pub fn body(&self) -> &str {
        frontmatter::split(&self.source).body
    }

    /// Raw YAML frontmatter, if any.
    pub fn frontmatter(&self) -> Option<&str> {
        frontmatter::split(&self.source).frontmatter
    }

    /// All wikilinks in the note.
    pub fn links(&self) -> Vec<wikilink::Link> {
        wikilink::parse(&self.source)
    }

    /// All tags (frontmatter + inline).
    pub fn tags(&self) -> Vec<String> {
        tags::extract(self.body(), self.frontmatter())
    }
}
