//! Unsaved drafts, so a crash or a forced quit cannot lose typed text.
//!
//! Drafts live under `.dragon-glass/drafts/` (already outside git) as one small text file
//! per note, written on a debounce while the buffer is dirty and removed when the note is
//! saved. The file is human-readable:
//!
//! ```text
//! draft=1
//! note=notes/a.md
//! line=12
//! col=0
//! at=1763000000000
//! base=1311768467463790320
//! ---
//! the note text, exactly as typed
//! ```
//!
//! `base` is the content hash of the file the draft was based on, so recovery can tell
//! whether the note also changed on disk. Text after the `---` line is stored verbatim,
//! including trailing newlines.

use std::path::{Path, PathBuf};

use crate::fingerprint::hash_text;
use crate::{storage, Result};

/// Drafts are a convenience cache; cap how many are kept so the folder cannot grow forever.
pub const MAX_DRAFTS: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Draft {
    /// Vault-relative note path. Empty drafts are not written.
    pub note: String,
    /// Caret line and column, 0-based.
    pub line: usize,
    pub col: usize,
    /// Unix milliseconds when the draft was written.
    pub at: u64,
    /// Content hash of the note this draft was based on, when it had one.
    pub base: Option<u64>,
    pub text: String,
}

impl Draft {
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        out.push_str("draft=1\n");
        out.push_str(&format!("note={}\n", self.note));
        out.push_str(&format!("line={}\n", self.line));
        out.push_str(&format!("col={}\n", self.col));
        out.push_str(&format!("at={}\n", self.at));
        if let Some(base) = self.base {
            out.push_str(&format!("base={base}\n"));
        }
        out.push_str("---\n");
        out.push_str(&self.text);
        out
    }

    pub fn from_text(text: &str) -> Option<Self> {
        let mut draft = Draft {
            note: String::new(),
            line: 0,
            col: 0,
            at: 0,
            base: None,
            text: String::new(),
        };
        let mut offset = 0usize;
        let mut body_start = None;

        for line in text.split_inclusive('\n') {
            offset += line.len();
            let trimmed = line.trim_end_matches(['\r', '\n']);
            if trimmed == "---" {
                body_start = Some(offset);
                break;
            }
            let Some((key, value)) = trimmed.split_once('=') else {
                continue;
            };
            match key.trim() {
                "note" => draft.note = value.trim().to_string(),
                "line" => draft.line = value.trim().parse().unwrap_or(0),
                "col" => draft.col = value.trim().parse().unwrap_or(0),
                "at" => draft.at = value.trim().parse().unwrap_or(0),
                "base" => draft.base = value.trim().parse().ok(),
                _ => {}
            }
        }

        let body_start = body_start?;
        draft.text = text[body_start..].to_string();
        if draft.note.is_empty() {
            return None;
        }
        Some(draft)
    }
}

fn drafts_dir(root: &Path) -> PathBuf {
    root.join(".dragon-glass").join("drafts")
}

/// One stable file per note, so repeated writes replace the previous draft.
fn draft_file(root: &Path, note: &str) -> PathBuf {
    let mut normalized = note.replace('\\', "/");
    normalized.make_ascii_lowercase();
    drafts_dir(root).join(format!("{:016x}.draft", hash_text(&normalized)))
}

pub fn save(root: &Path, draft: &Draft) -> Result<()> {
    if draft.note.is_empty() {
        return Ok(());
    }
    std::fs::create_dir_all(drafts_dir(root))?;
    storage::write_atomic(&draft_file(root, &draft.note), &draft.to_text())?;
    prune(root);
    Ok(())
}

pub fn load(root: &Path, note: &str) -> Option<Draft> {
    let text = std::fs::read_to_string(draft_file(root, note)).ok()?;
    let draft = Draft::from_text(&text)?;
    // The hashed name is the lookup key; check the payload agrees.
    if draft.note != note {
        return None;
    }
    Some(draft)
}

pub fn discard(root: &Path, note: &str) {
    let _ = std::fs::remove_file(draft_file(root, note));
}

/// Every readable draft, newest first.
pub fn list(root: &Path) -> Vec<Draft> {
    let Ok(entries) = std::fs::read_dir(drafts_dir(root)) else {
        return Vec::new();
    };
    let mut drafts: Vec<Draft> = entries
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "draft")
        })
        .filter_map(|entry| std::fs::read_to_string(entry.path()).ok())
        .filter_map(|text| Draft::from_text(&text))
        .collect();
    drafts.sort_by_key(|draft| std::cmp::Reverse(draft.at));
    drafts
}

/// Keep the newest [`MAX_DRAFTS`] files and delete the rest.
fn prune(root: &Path) {
    let drafts = list(root);
    if drafts.len() <= MAX_DRAFTS {
        return;
    }
    for draft in drafts.into_iter().skip(MAX_DRAFTS) {
        discard(root, &draft.note);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("dragon-glass-drafts-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn roundtrips_text_exactly() {
        let draft = Draft {
            note: "notes/a.md".to_string(),
            line: 12,
            col: 3,
            at: 1_763_000_000_000,
            base: Some(42),
            text: "line one\n\nline three\n".to_string(),
        };
        assert_eq!(Draft::from_text(&draft.to_text()), Some(draft));
    }

    #[test]
    fn text_that_looks_like_a_header_is_kept() {
        let draft = Draft {
            note: "a.md".to_string(),
            line: 0,
            col: 0,
            at: 1,
            base: None,
            text: "note=not a header\n---\nstill text\n".to_string(),
        };
        assert_eq!(
            Draft::from_text(&draft.to_text()).unwrap().text,
            "note=not a header\n---\nstill text\n"
        );
    }

    #[test]
    fn a_draft_without_a_body_is_rejected() {
        assert_eq!(Draft::from_text("note=a.md\n"), None);
        assert_eq!(Draft::from_text("---\nbody"), None);
    }

    #[test]
    fn save_load_and_discard() {
        let dir = temp_dir("save");
        let draft = Draft {
            note: "notes/a.md".to_string(),
            line: 1,
            col: 2,
            at: 9,
            base: None,
            text: "hello".to_string(),
        };
        save(&dir, &draft).unwrap();
        assert_eq!(load(&dir, "notes/a.md"), Some(draft.clone()));
        assert_eq!(load(&dir, "notes/other.md"), None);
        assert_eq!(list(&dir), vec![draft.clone()]);

        // Saving again replaces, not appends.
        let newer = Draft {
            at: 10,
            text: "hello again".to_string(),
            ..draft.clone()
        };
        save(&dir, &newer).unwrap();
        assert_eq!(load(&dir, "notes/a.md"), Some(newer.clone()));
        assert_eq!(list(&dir).len(), 1);

        discard(&dir, "notes/a.md");
        assert_eq!(load(&dir, "notes/a.md"), None);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn an_untitled_draft_is_not_written() {
        let dir = temp_dir("untitled");
        let draft = Draft {
            note: String::new(),
            line: 0,
            col: 0,
            at: 0,
            base: None,
            text: "scratch".to_string(),
        };
        save(&dir, &draft).unwrap();
        assert!(list(&dir).is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
