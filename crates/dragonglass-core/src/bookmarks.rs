//! Bookmarked notes, stored inside the vault at `.dragon-glass/bookmarks.txt`.
//!
//! One vault-relative path per line, so the file is diffable and trivially inspectable.

use std::path::{Path, PathBuf};

use crate::{storage, Result};

fn file(root: &Path) -> PathBuf {
    root.join(".dragon-glass").join("bookmarks.txt")
}

/// Read the bookmark list. Missing file means no bookmarks.
pub fn load(root: &Path) -> Vec<String> {
    std::fs::read_to_string(file(root))
        .map(|text| {
            text.lines()
                .map(|line| line.trim().to_string())
                .filter(|line| !line.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

pub fn save(root: &Path, bookmarks: &[String]) -> Result<()> {
    std::fs::create_dir_all(root.join(".dragon-glass"))?;

    let mut text = String::new();
    for bookmark in bookmarks {
        text.push_str(bookmark);
        text.push('\n');
    }

    storage::write_atomic(&file(root), &text)?;
    Ok(())
}

/// Toggle a note. Returns `true` if it is now bookmarked.
pub fn toggle(bookmarks: &mut Vec<String>, relative: &str) -> bool {
    if let Some(index) = bookmarks.iter().position(|b| b == relative) {
        bookmarks.remove(index);
        false
    } else {
        bookmarks.push(relative.to_string());
        bookmarks.sort();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggle_adds_then_removes() {
        let mut bookmarks = Vec::new();
        assert!(toggle(&mut bookmarks, "b.md"));
        assert!(toggle(&mut bookmarks, "a.md"));
        assert_eq!(bookmarks, vec!["a.md", "b.md"]);

        assert!(!toggle(&mut bookmarks, "a.md"));
        assert_eq!(bookmarks, vec!["b.md"]);
    }

    #[test]
    fn save_and_load_roundtrip() {
        let dir = std::env::temp_dir().join(format!(
            "dragon-glass-bookmarks-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        assert!(load(&dir).is_empty());

        let bookmarks = vec!["notes/a.md".to_string(), "b.md".to_string()];
        save(&dir, &bookmarks).unwrap();
        assert_eq!(load(&dir), bookmarks);

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
