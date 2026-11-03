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
/// folder inside itself.
pub fn rename(root: &Path, from: &str, to: &str) -> Result<()> {
    let (from_relative, from_path) = existing(root, from)?;
    let (to_relative, to_path) = checked(root, to)?;

    if from_relative == to_relative {
        return Ok(());
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
    Ok(())
}

/// Move a note/folder into the vault trash. Returns the trashed path.
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
    let mut destination = root
        .join(".dragon-glass")
        .join("trash")
        .join(format!("{stamp:013}"))
        .join(&relative);

    // Two deletes in the same millisecond must not collide.
    let mut suffix = 1usize;
    while destination.exists() {
        destination = root
            .join(".dragon-glass")
            .join("trash")
            .join(format!("{stamp:013}-{suffix}"))
            .join(&relative);
        suffix += 1;
    }

    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::rename(&path, &destination)?;
    Ok(destination)
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
}
