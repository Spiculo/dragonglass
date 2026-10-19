//! Lightweight version history: a snapshot of a note is kept whenever it is overwritten.
//!
//! Snapshots live under `.dragon-glass/snapshots/<note-key>/<stamp>-<n>.md` and the newest
//! `KEEP` per note are retained. This is a safety net, not a VCS.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{storage, Result};

/// How many snapshots to keep per note.
pub const KEEP: usize = 20;

fn dir(root: &Path, relative: &str) -> PathBuf {
    let key = relative.replace(['/', '\\'], "__");
    root.join(".dragon-glass").join("snapshots").join(key)
}

/// Store `content` as a snapshot of `relative`.
pub fn record(root: &Path, relative: &str, content: &str) -> Result<()> {
    let dir = dir(root, relative);
    std::fs::create_dir_all(&dir)?;

    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let index = list(root, relative).len();

    storage::write_atomic(&dir.join(format!("{stamp:013}-{index:04}.md")), content)?;
    prune(root, relative, KEEP)
}

/// Snapshot ids for a note, oldest first.
pub fn list(root: &Path, relative: &str) -> Vec<String> {
    let mut ids: Vec<String> = std::fs::read_dir(dir(root, relative))
        .map(|entries| {
            entries
                .filter_map(|entry| entry.ok())
                .map(|entry| entry.file_name().to_string_lossy().to_string())
                .collect()
        })
        .unwrap_or_default();
    ids.sort();
    ids
}

pub fn read(root: &Path, relative: &str, id: &str) -> Result<String> {
    Ok(std::fs::read_to_string(dir(root, relative).join(id))?)
}

fn prune(root: &Path, relative: &str, keep: usize) -> Result<()> {
    let ids = list(root, relative);
    if ids.len() <= keep {
        return Ok(());
    }

    let remove = ids.len() - keep;
    for id in ids.into_iter().take(remove) {
        let _ = std::fs::remove_file(dir(root, relative).join(id));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "dragon-glass-history-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn records_and_reads_snapshots() {
        let root = temp_root("record");
        record(&root, "notes/a.md", "first").unwrap();
        record(&root, "notes/a.md", "second").unwrap();

        let ids = list(&root, "notes/a.md");
        assert_eq!(ids.len(), 2);
        assert_eq!(read(&root, "notes/a.md", &ids[0]).unwrap(), "first");
        assert_eq!(read(&root, "notes/a.md", &ids[1]).unwrap(), "second");

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn keeps_only_the_newest_snapshots() {
        let root = temp_root("prune");
        for i in 0..KEEP + 3 {
            record(&root, "a.md", &format!("v{i}")).unwrap();
        }

        let ids = list(&root, "a.md");
        assert_eq!(ids.len(), KEEP);
        // The newest entry holds the most recent content.
        assert_eq!(
            read(&root, "a.md", ids.last().unwrap()).unwrap(),
            format!("v{}", KEEP + 2)
        );

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn notes_do_not_share_snapshot_dirs() {
        let root = temp_root("distinct");
        record(&root, "a/note.md", "one").unwrap();
        record(&root, "b/note.md", "two").unwrap();
        assert_eq!(list(&root, "a/note.md").len(), 1);
        assert_eq!(list(&root, "b/note.md").len(), 1);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
