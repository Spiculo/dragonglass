//! Safe file operations on a vault: create, rename/move, and trash.
//!
//! The UI never touches paths directly. Everything goes through this module so that a
//! relative path can never escape the vault and a delete is always recoverable: "delete"
//! moves the entry into `.dragon-glass/trash/<stamp>/<relative>` rather than unlinking it.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{storage, Error, Result};

/// Normalise a user-entered vault-relative path.
///
/// Empty segments and `.`/`..` are dropped, so `../notes//a` becomes `notes/a` and can
/// never point outside the vault. Characters Windows rejects in file names become `-`.
pub fn sanitize_relative(input: &str) -> String {
    input
        .split(['/', '\\'])
        .filter(|segment| !segment.is_empty() && *segment != "." && *segment != "..")
        .map(|segment| {
            segment
                .chars()
                .map(|ch| match ch {
                    ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\\' => '-',
                    other => other,
                })
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn checked(root: &Path, relative: &str) -> Result<(String, PathBuf)> {
    let relative = sanitize_relative(relative);
    if relative.is_empty() {
        return Err(Error::InvalidPath(relative));
    }
    Ok((relative.clone(), root.join(relative)))
}

/// Validate a path that already exists on disk. Unlike `checked`, this keeps the name
/// exactly as it is: a note legally named `a:b.md` on Linux must still be renameable.
fn existing(root: &Path, relative: &str) -> Result<(String, PathBuf)> {
    let relative = relative.replace('\\', "/");
    if relative.is_empty()
        || Path::new(&relative).is_absolute()
        || relative.split('/').any(|part| part == "..")
    {
        return Err(Error::InvalidPath(relative));
    }
    Ok((relative.clone(), root.join(relative)))
}

/// Create a new markdown note. Adds `.md` when the name has no markdown extension.
pub fn create_note(root: &Path, relative: &str, content: &str) -> Result<PathBuf> {
    let (relative, _) = checked(root, relative)?;
    let relative = if is_markdown(&relative) {
        relative
    } else {
        format!("{relative}.md")
    };
    let path = root.join(&relative);

    if path.exists() {
        return Err(Error::AlreadyExists(relative));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    storage::write_atomic(&path, content)?;
    Ok(path)
}

/// Create a folder (and any missing parents).
pub fn create_folder(root: &Path, relative: &str) -> Result<PathBuf> {
    let (relative, path) = checked(root, relative)?;
    if path.exists() {
        return Err(Error::AlreadyExists(relative));
    }
    std::fs::create_dir_all(&path)?;
    Ok(path)
}

/// Rename or move a note/folder. Refuses to overwrite an existing entry or to move a
/// folder inside itself. Returns the sanitized vault-relative destination.
pub fn rename(root: &Path, from: &str, to: &str) -> Result<String> {
    let (from_relative, from_path) = existing(root, from)?;
    let (to_relative, to_path) = checked(root, to)?;

    if from_relative == to_relative {
        return Ok(to_relative);
    }
    if !from_path.exists() {
        return Err(Error::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("{from_relative} does not exist"),
        )));
    }
    if to_path.exists() {
        // Allow a case-only rename on case-insensitive filesystems.
        let same_entry = match (from_path.canonicalize(), to_path.canonicalize()) {
            (Ok(from), Ok(to)) => from == to,
            _ => false,
        };
        if !same_entry {
            return Err(Error::AlreadyExists(to_relative));
        }
    }
    if to_relative.starts_with(&format!("{from_relative}/")) {
        return Err(Error::InvalidPath(to_relative));
    }
    if let Some(parent) = to_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::rename(&from_path, &to_path)?;
    Ok(to_relative)
}

/// Move a note/folder into the vault trash. Returns the trashed path.
///
/// Each delete gets its own stamp directory containing `entry` (the moved note or folder)
/// and `meta.json` (the original relative path), so nested notes can be restored exactly.
pub fn trash(root: &Path, relative: &str) -> Result<PathBuf> {
    let (relative, path) = existing(root, relative)?;
    if !path.exists() {
        return Err(Error::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("{relative} does not exist"),
        )));
    }

    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    let mut base = trash_root(root).join(format!("{stamp:013}"));

    // Two deletes in the same millisecond must not collide.
    let mut suffix = 1usize;
    while base.exists() {
        base = trash_root(root).join(format!("{stamp:013}-{suffix}"));
        suffix += 1;
    }

    std::fs::create_dir_all(&base)?;
    let destination = base.join("entry");
    std::fs::rename(&path, &destination)?;
    write_trash_meta(&base, &relative)?;
    Ok(destination)
}

/// A deleted entry waiting in the vault trash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrashedEntry {
    /// Vault-relative path of the stamp directory that holds the deleted entry.
    pub trash_relative: String,
    /// Where the entry was before it was deleted.
    pub original: String,
    /// Wall-clock deletion time in milliseconds since the Unix epoch, when known.
    pub deleted_at_ms: u64,
}

const TRASH_META: &str = "meta.json";
const TRASH_ENTRY: &str = "entry";

fn write_trash_meta(base: &Path, original: &str) -> Result<()> {
    let meta = serde_json::json!({ "original": original });
    std::fs::write(base.join(TRASH_META), meta.to_string())?;
    Ok(())
}

fn read_trash_meta(base: &Path) -> Option<String> {
    let text = std::fs::read_to_string(base.join(TRASH_META)).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    value
        .get("original")
        .and_then(|original| original.as_str())
        .map(|original| original.to_string())
}

/// The stamp prefix, e.g. `.dragon-glass/trash/000001760000000-a`.
fn stamp_from_name(name: &str) -> u64 {
    name.split('-')
        .next()
        .and_then(|stamp| stamp.parse::<u64>().ok())
        .unwrap_or(0)
}

/// List everything in the trash, newest first.
pub fn list_trash(root: &Path) -> Result<Vec<TrashedEntry>> {
    let trash = trash_root(root);
    if !trash.is_dir() {
        return Ok(Vec::new());
    }

    let mut entries = Vec::new();
    for item in std::fs::read_dir(&trash)? {
        let item = item?;
        if !item.file_type()?.is_dir() {
            continue;
        }
        let name = item.file_name().to_string_lossy().to_string();
        let base = item.path();
        let original = match read_trash_meta(&base) {
            Some(original) => original,
            // Trash written before meta.json existed: the stamp directory holds the
            // original path directly. Descend while the path is unambiguous.
            None => match legacy_entry(&base) {
                Some(original) => original,
                None => continue,
            },
        };
        entries.push(TrashedEntry {
            trash_relative: format!(".dragon-glass/trash/{name}"),
            original,
            deleted_at_ms: stamp_from_name(&name),
        });
    }

    entries.sort_by_key(|entry| std::cmp::Reverse(entry.deleted_at_ms));
    Ok(entries)
}

/// Restore one trash entry to its original path, refusing to overwrite an existing file.
/// Returns the restored vault-relative path.
pub fn restore(root: &Path, trash_relative: &str) -> Result<String> {
    let (_, base) = existing(root, trash_relative)?;
    let trash = trash_root(root);
    let base = base
        .canonicalize()
        .map_err(|_| Error::InvalidPath(trash_relative.to_string()))?;
    let trash = trash
        .canonicalize()
        .map_err(|_| Error::InvalidPath(trash_relative.to_string()))?;
    if !base.starts_with(&trash) {
        return Err(Error::InvalidPath(trash_relative.to_string()));
    }

    let original = match read_trash_meta(&base) {
        Some(original) => original,
        None => match legacy_entry(&base) {
            Some(original) => original,
            None => {
                return Err(Error::Io(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("{trash_relative} is empty"),
                )))
            }
        },
    };
    let (original, target) = checked(root, &original)?;
    if target.exists() {
        return Err(Error::AlreadyExists(original));
    }

    // Current layout stores `entry` next to `meta.json`; the legacy layout put the moved
    // file or folder straight into the stamp directory.
    let entry = base.join(TRASH_ENTRY);
    let source = if entry.exists() {
        entry
    } else {
        base.join(&original)
    };

    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::rename(&source, &target)?;
    let _ = std::fs::remove_dir_all(&base);
    Ok(original)
}

/// Delete everything in the trash. Returns the number of entries removed.
pub fn empty_trash(root: &Path) -> Result<usize> {
    let trash = trash_root(root);
    if !trash.is_dir() {
        return Ok(0);
    }
    let mut removed = 0;
    for item in std::fs::read_dir(&trash)? {
        let item = item?;
        if item.file_type()?.is_dir() {
            std::fs::remove_dir_all(item.path())?;
            removed += 1;
        }
    }
    Ok(removed)
}

/// Reconstruct the original relative path of a legacy trash entry by descending while each
/// directory has exactly one child. Returns `None` when two entries were trashed together.
fn legacy_entry(base: &Path) -> Option<String> {
    let mut current = base.to_path_buf();
    let mut relative = String::new();

    loop {
        let children: Vec<PathBuf> = std::fs::read_dir(&current)
            .ok()?
            .flatten()
            .map(|entry| entry.path())
            .collect();
        if children.len() != 1 {
            return if relative.is_empty() {
                None
            } else {
                Some(relative)
            };
        }
        let child = &children[0];
        let name = child.file_name()?.to_string_lossy().to_string();
        relative = if relative.is_empty() {
            name
        } else {
            format!("{relative}/{name}")
        };
        if !child.is_dir() {
            return Some(relative);
        }
        current = child.clone();
    }
}

/// Trash root, for the UI's "open trash" shortcut and for tests.
pub fn trash_root(root: &Path) -> PathBuf {
    root.join(".dragon-glass").join("trash")
}

fn is_markdown(relative: &str) -> bool {
    Path::new(relative)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| {
            extension.eq_ignore_ascii_case("md") || extension.eq_ignore_ascii_case("markdown")
        })
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "dragon-glass-vaultfs-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn sanitizes_and_blocks_escapes() {
        assert_eq!(sanitize_relative("../notes//a"), "notes/a");
        assert_eq!(sanitize_relative("a\\b"), "a/b");
        assert_eq!(sanitize_relative("bad:name?.md"), "bad-name-.md");
        assert_eq!(sanitize_relative(".."), "");
    }

    #[test]
    fn creates_a_note_and_adds_the_extension() {
        let root = temp_root("create");
        let path = create_note(&root, "notes/one", "# One\n").unwrap();
        assert!(path.ends_with("notes/one.md"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "# One\n");

        // A second create must not clobber the first.
        assert!(matches!(
            create_note(&root, "notes/one", "other"),
            Err(Error::AlreadyExists(_))
        ));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn renames_and_moves_a_note() {
        let root = temp_root("rename");
        create_note(&root, "a.md", "one").unwrap();
        rename(&root, "a.md", "folder/b.md").unwrap();
        assert!(!root.join("a.md").exists());
        assert_eq!(
            std::fs::read_to_string(root.join("folder/b.md")).unwrap(),
            "one"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn refuses_to_overwrite_or_move_a_folder_into_itself() {
        let root = temp_root("refuse");
        create_note(&root, "a.md", "one").unwrap();
        create_note(&root, "b.md", "two").unwrap();
        std::fs::create_dir_all(root.join("folder")).unwrap();
        assert!(matches!(
            rename(&root, "a.md", "b.md"),
            Err(Error::AlreadyExists(_))
        ));
        assert!(matches!(
            rename(&root, "folder", "folder/inside"),
            Err(Error::InvalidPath(_))
        ));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn trash_keeps_the_content_and_leaves_the_vault() {
        let root = temp_root("trash");
        create_note(&root, "notes/gone.md", "keep me").unwrap();
        let trashed = trash(&root, "notes/gone.md").unwrap();
        assert!(!root.join("notes/gone.md").exists());
        assert_eq!(std::fs::read_to_string(&trashed).unwrap(), "keep me");
        assert!(trashed.starts_with(trash_root(&root)));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn lists_restores_and_empties_trash() {
        let root = temp_root("trash-list");
        create_note(&root, "notes/deep/gone.md", "keep").unwrap();
        create_note(&root, "second.md", "two").unwrap();
        trash(&root, "notes/deep/gone.md").unwrap();
        trash(&root, "second.md").unwrap();

        let listed = list_trash(&root).unwrap();
        assert_eq!(listed.len(), 2);
        let nested = listed
            .iter()
            .find(|entry| entry.original == "notes/deep/gone.md")
            .expect("nested entry is listed with its original path");
        assert!(listed.iter().any(|entry| entry.original == "second.md"));

        let restored = restore(&root, &nested.trash_relative).unwrap();
        assert_eq!(restored, "notes/deep/gone.md");
        assert_eq!(
            std::fs::read_to_string(root.join("notes/deep/gone.md")).unwrap(),
            "keep"
        );
        assert_eq!(list_trash(&root).unwrap().len(), 1);

        // Restoring over an existing note is refused rather than overwriting it.
        assert!(restore(&root, &nested.trash_relative).is_err());

        assert_eq!(empty_trash(&root).unwrap(), 1);
        assert!(list_trash(&root).unwrap().is_empty());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn restores_legacy_trash_without_meta() {
        let root = temp_root("trash-legacy");
        create_note(&root, "old.md", "legacy").unwrap();
        let legacy = trash_root(&root).join("0000000000001");
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::rename(root.join("old.md"), legacy.join("old.md")).unwrap();

        let listed = list_trash(&root).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].original, "old.md");
        restore(&root, &listed[0].trash_relative).unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("old.md")).unwrap(),
            "legacy"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn restores_a_legacy_nested_path() {
        let root = temp_root("trash-legacy-nested");
        create_note(&root, "notes/nested.md", "nested").unwrap();
        let legacy = trash_root(&root).join("0000000000002");
        std::fs::create_dir_all(legacy.join("notes")).unwrap();
        std::fs::rename(root.join("notes/nested.md"), legacy.join("notes/nested.md")).unwrap();

        let listed = list_trash(&root).unwrap();
        assert_eq!(listed[0].original, "notes/nested.md");
        restore(&root, &listed[0].trash_relative).unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("notes/nested.md")).unwrap(),
            "nested"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }
}
