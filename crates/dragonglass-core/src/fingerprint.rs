//! Conflict-safe saves.
//!
//! A note can change on disk while it is open: another editor, a git checkout, a sync
//! client, or the same vault open in a second window. Saving on top of that would lose the
//! other version. A fingerprint captured when the file was read makes the comparison cheap:
//! the content hash decides, and size and modification time are only hints.
//!
//! This module is the shared contract the plan asks for (`Q0.2`): every client loads a
//! fingerprint, saves with it, and gets a [`SaveReport`] back.

use std::fs;
use std::io;
use std::path::Path;
use std::time::SystemTime;

use crate::storage;

/// The identity of one version of a document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocumentFingerprint {
    /// Byte length of the file.
    pub size: u64,
    /// Modification time, when the platform reports one. A hint only.
    pub modified: Option<SystemTime>,
    /// Hash of the file contents. The source of truth.
    pub content_hash: u64,
}

impl DocumentFingerprint {
    /// Fingerprint a buffer that is about to be written.
    pub fn of_text(text: &str) -> Self {
        Self {
            size: text.len() as u64,
            modified: None,
            content_hash: hash_text(text),
        }
    }

    /// Fingerprint the file on disk.
    pub fn of_path(path: &Path) -> io::Result<Self> {
        let text = fs::read_to_string(path)?;
        let metadata = fs::metadata(path).ok();
        Ok(Self {
            size: text.len() as u64,
            modified: metadata.and_then(|meta| meta.modified().ok()),
            content_hash: hash_text(&text),
        })
    }

    /// Whether this fingerprint describes `text`.
    pub fn matches_text(&self, text: &str) -> bool {
        self.content_hash == hash_text(text)
    }
}

/// What happened when a document was saved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SaveReport {
    /// The write succeeded, and this is the new on-disk version.
    Saved { fingerprint: DocumentFingerprint },
    /// The file on disk is not the version the save was based on, so nothing was written.
    /// `disk` is `None` when the file was deleted.
    Conflict { disk: Option<DocumentFingerprint> },
    /// The write failed for an unrelated reason.
    Error { message: String },
}

impl SaveReport {
    pub fn is_saved(&self) -> bool {
        matches!(self, SaveReport::Saved { .. })
    }

    pub fn is_conflict(&self) -> bool {
        matches!(self, SaveReport::Conflict { .. })
    }
}

/// Save `text` to `path`, but only if the file still matches `expected`.
///
/// `expected = None` means there is no baseline (a new file), so the write proceeds. Pass a
/// fingerprint captured by [`DocumentFingerprint::of_path`] or [`DocumentFingerprint::of_text`]
/// everywhere else.
pub fn save_if_unchanged(
    path: &Path,
    text: &str,
    expected: Option<&DocumentFingerprint>,
) -> SaveReport {
    if let Some(expected) = expected {
        match DocumentFingerprint::of_path(path) {
            Ok(disk) => {
                if disk.content_hash != expected.content_hash {
                    return SaveReport::Conflict { disk: Some(disk) };
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return SaveReport::Conflict { disk: None };
            }
            // Unreadable for another reason (permissions, encoding): do not guess.
            Err(error) => {
                return SaveReport::Error {
                    message: error.to_string(),
                };
            }
        }
    }

    match storage::write_atomic(path, text) {
        Ok(()) => SaveReport::Saved {
            fingerprint: DocumentFingerprint::of_text(text),
        },
        Err(error) => SaveReport::Error {
            message: error.to_string(),
        },
    }
}

/// Save `text` to `path` without a conflict check. Used only by "keep mine", where the
/// user has just seen both versions and chosen this one.
pub fn save_forced(path: &Path, text: &str) -> SaveReport {
    match storage::write_atomic(path, text) {
        Ok(()) => SaveReport::Saved {
            fingerprint: DocumentFingerprint::of_text(text),
        },
        Err(error) => SaveReport::Error {
            message: error.to_string(),
        },
    }
}

/// FNV-1a, 64 bit. Small, dependency-free and stable across processes and releases, so a
/// fingerprint written to disk (a draft, for example) still means something next launch.
pub(crate) fn hash_text(text: &str) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "dragon-glass-fingerprint-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_same_text_has_the_same_fingerprint() {
        assert_eq!(
            DocumentFingerprint::of_text("hello").content_hash,
            DocumentFingerprint::of_text("hello").content_hash
        );
        assert_ne!(
            DocumentFingerprint::of_text("hello").content_hash,
            DocumentFingerprint::of_text("hello!").content_hash
        );
    }

    #[test]
    fn a_clean_save_writes_and_returns_the_new_fingerprint() {
        let dir = temp_dir("clean");
        let path = dir.join("note.md");
        fs::write(&path, "one").unwrap();

        let loaded = DocumentFingerprint::of_path(&path).unwrap();
        let report = save_if_unchanged(&path, "two", Some(&loaded));
        assert_eq!(
            report,
            SaveReport::Saved {
                fingerprint: DocumentFingerprint::of_text("two")
            }
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), "two");

        // Saving again with the fingerprint the first save returned is clean too.
        let again = save_if_unchanged(&path, "three", Some(&DocumentFingerprint::of_text("two")));
        assert!(again.is_saved());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn an_external_edit_is_a_conflict_and_is_not_overwritten() {
        let dir = temp_dir("conflict");
        let path = dir.join("note.md");
        fs::write(&path, "original").unwrap();
        let loaded = DocumentFingerprint::of_path(&path).unwrap();

        fs::write(&path, "someone else").unwrap();
        let report = save_if_unchanged(&path, "mine", Some(&loaded));
        assert!(report.is_conflict());
        match report {
            SaveReport::Conflict { disk } => {
                assert!(disk.is_some_and(|disk| disk.matches_text("someone else")));
            }
            _ => panic!("expected a conflict"),
        }
        assert_eq!(fs::read_to_string(&path).unwrap(), "someone else");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_deleted_file_is_a_conflict_not_a_silent_recreate() {
        let dir = temp_dir("deleted");
        let path = dir.join("note.md");
        fs::write(&path, "here").unwrap();
        let loaded = DocumentFingerprint::of_path(&path).unwrap();
        fs::remove_file(&path).unwrap();

        let report = save_if_unchanged(&path, "mine", Some(&loaded));
        assert_eq!(report, SaveReport::Conflict { disk: None });
        assert!(!path.exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_timestamp_change_with_identical_content_is_not_a_conflict() {
        let dir = temp_dir("mtime");
        let path = dir.join("note.md");
        fs::write(&path, "same").unwrap();
        let loaded = DocumentFingerprint::of_path(&path).unwrap();

        // Rewrite identical bytes: the mtime moves, the content hash does not.
        fs::write(&path, "same").unwrap();
        assert!(save_if_unchanged(&path, "new", Some(&loaded)).is_saved());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_write_error_is_reported_not_raised() {
        let dir = temp_dir("error");
        let missing = dir.join("no-such-dir").join("note.md");
        let report = save_if_unchanged(&missing, "text", None);
        assert!(matches!(report, SaveReport::Error { .. }));
        fs::remove_dir_all(dir).unwrap();
    }
}
