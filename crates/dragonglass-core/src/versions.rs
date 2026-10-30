//! A/B versions of a note, stored beside the vault as plain markdown files.
//!
//! Variants live under `.dragon-glass/versions/<note-key>/<label>.md`. They are ordinary
//! files, so nothing is hidden in a database and a user can inspect them with any editor.
//! A variant can be promoted back over the note; the current note is snapshotted first,
//! so promoting is reversible through the normal file-recovery history.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{diff, history, storage, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    /// File-safe identifier, used for reading and promoting.
    pub id: String,
    /// Human-facing label (the original label, cleaned up).
    pub label: String,
    pub created_ms: u128,
    pub bytes: usize,
}

impl Version {
    /// `A` and `B` are shown uppercase; every other label is shown as stored.
    pub fn display_label(&self) -> String {
        if self.id.len() == 1 {
            self.id.to_uppercase()
        } else {
            self.label.clone()
        }
    }
}

/// Directory holding every variant of `relative`.
pub fn dir(root: &Path, relative: &str) -> PathBuf {
    let key = relative.replace(['/', '\\'], "__");
    root.join(".dragon-glass").join("versions").join(key)
}

/// Variants of a note, sorted by label.
pub fn list(root: &Path, relative: &str) -> Vec<Version> {
    let dir = dir(root, relative);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };

    let mut found: Vec<Version> = entries
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

            let id = path.file_stem()?.to_string_lossy().to_string();
            if id.is_empty() {
                return None;
            }
            let metadata = entry.metadata().ok()?;
            let created_ms = metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map(|duration| duration.as_millis())
                .unwrap_or(0);

            Some(Version {
                label: id.clone(),
                id,
                created_ms,
                bytes: metadata.len() as usize,
            })
        })
        .collect();

    found.sort_by(|a, b| a.label.cmp(&b.label));
    found
}

/// Save `content` as (or over) the variant `label`. Returns the stored variant.
pub fn save(root: &Path, relative: &str, label: &str, content: &str) -> Result<Version> {
    let id = sanitize(label);
    let dir = dir(root, relative);
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{id}.md"));
    storage::write_atomic(&path, content)?;

    Ok(Version {
        label: label.trim().to_string(),
        id,
        created_ms: now_ms(),
        bytes: content.len(),
    })
}

/// The content of a variant.
pub fn read(root: &Path, relative: &str, id: &str) -> Result<String> {
    Ok(std::fs::read_to_string(
        dir(root, relative).join(format!("{}.md", sanitize(id))),
    )?)
}

/// Delete a variant. Missing variants are not an error.
pub fn remove(root: &Path, relative: &str, id: &str) -> Result<()> {
    let path = dir(root, relative).join(format!("{}.md", sanitize(id)));
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

/// Write a variant over the note, snapshotting the current note first.
///
/// Returns the promoted content so the caller can reload an open editor without a second
/// disk read.
pub fn promote(root: &Path, relative: &str, id: &str) -> Result<String> {
    let content = read(root, relative, id)?;
    let path = root.join(relative);
    if let Ok(existing) = std::fs::read_to_string(&path) {
        if existing != content {
            let _ = history::record(root, relative, &existing);
        }
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    storage::write_atomic(&path, &content)?;
    Ok(content)
}

/// Compare the note as it is on disk with a variant.
pub fn compare_with_note(root: &Path, relative: &str, id: &str) -> Result<diff::Diff> {
    let current = std::fs::read_to_string(root.join(relative)).unwrap_or_default();
    let variant = read(root, relative, id)?;
    Ok(diff::lines(&current, &variant))
}

/// Compare two variants.
pub fn compare(root: &Path, relative: &str, a: &str, b: &str) -> Result<diff::Diff> {
    let old = read(root, relative, a)?;
    let new = read(root, relative, b)?;
    Ok(diff::lines(&old, &new))
}

/// A file-safe variant id. Labels stay readable in the UI, while ids never contain path
/// separators or characters Windows rejects.
pub fn sanitize(label: &str) -> String {
    let mut out = String::new();
    for ch in label.trim().chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            out.push(ch.to_ascii_lowercase());
        } else if (ch.is_whitespace() || ch == '.') && !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_matches('-').to_string();
    let out: String = out.chars().take(64).collect();
    if out.is_empty() {
        "version".to_string()
    } else {
        out
    }
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "dragon-glass-versions-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn save_list_and_read_variants() {
        let root = temp_root("save");
        save(&root, "notes/a.md", "A", "alpha").unwrap();
        save(&root, "notes/a.md", "B", "beta").unwrap();

        let versions = list(&root, "notes/a.md");
        assert_eq!(versions.len(), 2);
        assert_eq!(versions[0].id, "a");
        assert_eq!(versions[1].id, "b");
        assert_eq!(read(&root, "notes/a.md", "a").unwrap(), "alpha");

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn saving_the_same_label_overwrites_it() {
        let root = temp_root("overwrite");
        save(&root, "a.md", "A", "first").unwrap();
        save(&root, "a.md", "A", "second").unwrap();
        assert_eq!(list(&root, "a.md").len(), 1);
        assert_eq!(read(&root, "a.md", "a").unwrap(), "second");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn notes_do_not_share_version_directories() {
        let root = temp_root("distinct");
        save(&root, "a/note.md", "A", "one").unwrap();
        save(&root, "b/note.md", "A", "two").unwrap();
        assert_eq!(read(&root, "a/note.md", "a").unwrap(), "one");
        assert_eq!(read(&root, "b/note.md", "a").unwrap(), "two");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn promote_writes_the_variant_and_snapshots_the_old_note() {
        let root = temp_root("promote");
        std::fs::create_dir_all(root.join("notes")).unwrap();
        std::fs::write(root.join("notes/a.md"), "current").unwrap();
        save(&root, "notes/a.md", "A", "promoted").unwrap();

        let content = promote(&root, "notes/a.md", "a").unwrap();
        assert_eq!(content, "promoted");
        assert_eq!(
            std::fs::read_to_string(root.join("notes/a.md")).unwrap(),
            "promoted"
        );

        let snapshots = history::list(&root, "notes/a.md");
        assert_eq!(snapshots.len(), 1);
        assert_eq!(
            history::read(&root, "notes/a.md", &snapshots[0]).unwrap(),
            "current"
        );

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn compare_reports_added_and_removed_lines() {
        let root = temp_root("compare");
        std::fs::write(root.join("a.md"), "one\ntwo\n").unwrap();
        save(&root, "a.md", "A", "one\ntwo\n").unwrap();
        save(&root, "a.md", "B", "one\nthree\n").unwrap();

        let diff = compare_with_note(&root, "a.md", "b").unwrap();
        assert_eq!(diff.summary(), "+1 -1");

        let diff = compare(&root, "a.md", "a", "b").unwrap();
        assert_eq!(diff.summary(), "+1 -1");

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn remove_is_idempotent() {
        let root = temp_root("remove");
        save(&root, "a.md", "A", "x").unwrap();
        remove(&root, "a.md", "a").unwrap();
        remove(&root, "a.md", "a").unwrap();
        assert!(list(&root, "a.md").is_empty());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn labels_are_sanitized_for_the_filesystem() {
        assert_eq!(sanitize("Draft A"), "draft-a");
        assert_eq!(sanitize("../escape"), "escape");
        assert_eq!(sanitize("***"), "version");
        assert_eq!(sanitize("A/B"), "ab");
    }

    #[test]
    fn display_labels_uppercase_single_letters() {
        let version = Version {
            id: "a".into(),
            label: "A".into(),
            created_ms: 0,
            bytes: 0,
        };
        assert_eq!(version.display_label(), "A");

        let long = Version {
            id: "draft-a".into(),
            label: "Draft A".into(),
            created_ms: 0,
            bytes: 0,
        };
        assert_eq!(long.display_label(), "Draft A");
    }
}
