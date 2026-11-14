//! Resolve `[[link]]` targets to notes.
//!
//! A link normally names a file (`[[notes/Page]]`) or a stem (`[[Page]]`). Real vaults also
//! use frontmatter slugs and aliases: this corpus writes `Name: advanced-sorting-algorithms`
//! into each article and links `[[advanced-sorting-algorithms]]`, while the file itself is
//! called `Advanced Sorting Algorithms….md`. Ignoring those keys makes most backlinks and
//! relation edges disappear, so every resolver in the crate shares this index.

use std::collections::HashMap;

use crate::frontmatter::{self, PropertyValue};
use crate::{Note, NoteRef};

/// Frontmatter keys that name a note for link resolution, lowercased.
const ALIAS_KEYS: [&str; 4] = ["name", "slug", "alias", "aliases"];

#[derive(Debug, Clone, Default)]
pub struct NoteIndex {
    /// Normalised key (path, stem, slug or alias) to note ids.
    entries: HashMap<String, Vec<usize>>,
    /// Vault-relative paths, parallel to note ids.
    paths: Vec<String>,
}

impl NoteIndex {
    /// Index every note, reading frontmatter for slugs and aliases.
    pub fn new(notes: &[NoteRef]) -> Self {
        let mut index = Self::default();

        for (id, note) in notes.iter().enumerate() {
            index.paths.push(note.relative.clone());
            index.insert(note.path_no_ext(), id);
            index.insert(note.stem(), id);

            if let Ok(note) = Note::load(&note.path) {
                for key in frontmatter_keys(&note.source) {
                    index.insert(&key, id);
                }
            }
        }

        index
    }

    /// Index paths and stems only, without touching the disk. Used by tests and callers
    /// that only have a list of relative paths.
    pub fn from_paths<I: IntoIterator<Item = String>>(paths: I) -> Self {
        let mut index = Self::default();

        for relative in paths {
            let id = index.paths.len();
            index.paths.push(relative.clone());
            let without_ext = relative
                .rsplit_once('.')
                .map(|(path, _)| path)
                .unwrap_or(&relative);
            index.insert(without_ext, id);
            let stem = without_ext.rsplit('/').next().unwrap_or(without_ext);
            index.insert(stem, id);
        }

        index
    }

    fn insert(&mut self, key: &str, id: usize) {
        let key = normalize(key);
        if key.is_empty() {
            return;
        }
        let ids = self.entries.entry(key).or_default();
        if !ids.contains(&id) {
            ids.push(id);
        }
    }

    /// Resolve a link target to a note id.
    ///
    /// An exact path wins. Otherwise a unique path, stem, slug or alias resolves directly;
    /// when several notes share a key the shallowest path wins, and ties break
    /// alphabetically so the answer is stable.
    pub fn resolve(&self, target: &str) -> Option<usize> {
        let key = normalize(target);
        if key.is_empty() {
            return None;
        }

        let ids = self.entries.get(&key)?;

        if let Some(id) = ids
            .iter()
            .find(|id| normalize(&self.paths[**id]).trim_end_matches(".md") == key)
        {
            return Some(*id);
        }

        if ids.len() == 1 {
            return Some(ids[0]);
        }

        ids.iter()
            .min_by(|a, b| {
                depth(&self.paths[**a])
                    .cmp(&depth(&self.paths[**b]))
                    .then_with(|| self.paths[**a].cmp(&self.paths[**b]))
            })
            .copied()
    }

    /// The vault-relative path for a note id.
    pub fn relative(&self, id: usize) -> Option<&str> {
        self.paths.get(id).map(String::as_str)
    }

    /// Resolve a target straight to its vault-relative path.
    pub fn resolve_relative(&self, target: &str) -> Option<&str> {
        self.resolve(target).and_then(|id| self.relative(id))
    }

    pub fn len(&self) -> usize {
        self.paths.len()
    }

    pub fn is_empty(&self) -> bool {
        self.paths.is_empty()
    }
}

/// Normalise a link target or key for comparison.
pub fn normalize(target: &str) -> String {
    let target = target.trim().replace('\\', "/");
    let target = target.trim_start_matches("./");
    let target = target.trim_end_matches(".md");
    target.to_lowercase()
}

/// The frontmatter values that can name a note: `Name`, `Slug`, `alias` and `aliases`.
pub fn frontmatter_keys(src: &str) -> Vec<String> {
    let mut keys = Vec::new();

    for property in frontmatter::properties(src).unwrap_or_default() {
        if !ALIAS_KEYS
            .iter()
            .any(|key| property.key.trim().eq_ignore_ascii_case(key))
        {
            continue;
        }
        match property.value {
            PropertyValue::Scalar(value) => keys.push(value),
            PropertyValue::List(items) => keys.extend(items),
            PropertyValue::Raw(_) => {}
        }
    }

    keys.retain(|key| !key.trim().is_empty());
    keys
}

fn depth(path: &str) -> usize {
    path.matches('/').count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vault(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "dragon-glass-resolve-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn note(dir: &std::path::Path, name: &str, body: &str) -> NoteRef {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&path, body).unwrap();
        NoteRef {
            path,
            relative: name.to_string(),
        }
    }

    #[test]
    fn paths_and_stems_resolve() {
        let index = NoteIndex::from_paths(["notes/Page.md".to_string(), "Other.md".to_string()]);
        assert_eq!(index.resolve_relative("notes/Page"), Some("notes/Page.md"));
        assert_eq!(index.resolve_relative("Page"), Some("notes/Page.md"));
        assert_eq!(index.resolve_relative("Other.md"), Some("Other.md"));
        assert_eq!(index.resolve_relative("page"), Some("notes/Page.md"));
        assert_eq!(index.resolve_relative("missing"), None);
    }

    #[test]
    fn path_only_index_is_a_deterministic_fallback() {
        let dir = vault("fallback");
        let notes = vec![note(
            &dir,
            "Articles/Big Page.md",
            "---\nslug: big-page\n---\n# Big\n",
        )];
        let fast = NoteIndex::from_paths(notes.iter().map(|note| note.relative.clone()));
        assert_eq!(
            fast.resolve_relative("Big Page"),
            Some("Articles/Big Page.md")
        );
        assert_eq!(
            fast.resolve_relative("Articles/Big Page"),
            Some("Articles/Big Page.md")
        );
        // Aliases need the frontmatter pass, which is the deferred full build.
        assert_eq!(fast.resolve_relative("big-page"), None);
        let full = NoteIndex::new(&notes);
        assert_eq!(
            full.resolve_relative("big-page"),
            Some("Articles/Big Page.md")
        );
    }

    #[test]
    fn frontmatter_names_and_aliases_resolve() {
        let dir = vault("aliases");
        let notes = vec![
            note(
                &dir,
                "Advanced Sorting Algorithms. TimSort.md",
                "---\nName: advanced-sorting-algorithms\nAliases: [sorting, timsort]\n---\nbody\n",
            ),
            note(&dir, "Plain.md", "body\n"),
        ];

        let index = NoteIndex::new(&notes);
        assert_eq!(
            index.resolve_relative("advanced-sorting-algorithms"),
            Some("Advanced Sorting Algorithms. TimSort.md")
        );
        assert_eq!(
            index.resolve_relative("Sorting"),
            Some("Advanced Sorting Algorithms. TimSort.md")
        );
        assert_eq!(
            index.resolve_relative("timsort"),
            Some("Advanced Sorting Algorithms. TimSort.md")
        );
        assert_eq!(index.resolve_relative("Plain"), Some("Plain.md"));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn exact_paths_beat_duplicate_keys() {
        let dir = vault("duplicates");
        let notes = vec![
            note(&dir, "a/Shared.md", "one\n"),
            note(&dir, "b/Shared.md", "two\n"),
        ];

        let index = NoteIndex::new(&notes);
        assert_eq!(index.resolve_relative("a/Shared"), Some("a/Shared.md"));
        // Ambiguous stem: shallowest, then alphabetical.
        assert_eq!(index.resolve_relative("Shared"), Some("a/Shared.md"));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn frontmatter_keys_are_case_insensitive() {
        let keys = frontmatter_keys("---\nname: lower\nAlias: [One, Two]\nother: x\n---\n");
        assert_eq!(keys, vec!["lower", "One", "Two"]);
    }
}
