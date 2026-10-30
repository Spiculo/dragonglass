//! Per-vault session state: the note to reopen on the next launch.
//!
//! Stored at `.dragon-glass/session.txt` as `key=value` lines, mirroring `config.txt`.
//! Like the rest of `.dragon-glass`, it is a rebuildable convenience file: losing it only
//! means the app opens the vault without a note selected.

use std::path::{Path, PathBuf};

use crate::{storage, Result};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Session {
    /// Vault-relative path of the active note.
    pub note: Option<String>,
    /// Whether the note was in reading mode.
    pub reading: bool,
}

impl Session {
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        if let Some(note) = &self.note {
            out.push_str(&format!("note={note}\n"));
        }
        out.push_str(&format!("reading={}\n", u8::from(self.reading)));
        out
    }

    pub fn from_text(text: &str) -> Self {
        let mut session = Session::default();

        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.trim();

            match key.trim() {
                "note" => {
                    if !value.is_empty() {
                        session.note = Some(value.to_string());
                    }
                }
                "reading" => {
                    session.reading =
                        !matches!(value.to_lowercase().as_str(), "0" | "false" | "no" | "");
                }
                _ => {}
            }
        }

        session
    }
}

fn file(root: &Path) -> PathBuf {
    root.join(".dragon-glass").join("session.txt")
}

pub fn load(root: &Path) -> Session {
    std::fs::read_to_string(file(root))
        .map(|text| Session::from_text(&text))
        .unwrap_or_default()
}

pub fn save(root: &Path, session: &Session) -> Result<()> {
    std::fs::create_dir_all(root.join(".dragon-glass"))?;
    storage::write_atomic(&file(root), &session.to_text())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrips_through_text() {
        let session = Session {
            note: Some("notes/a.md".to_string()),
            reading: true,
        };
        assert_eq!(Session::from_text(&session.to_text()), session);
    }

    #[test]
    fn a_fresh_session_has_no_note() {
        assert_eq!(Session::from_text(""), Session::default());
        assert_eq!(Session::from_text("note=\nreading=0\n"), Session::default());
    }

    #[test]
    fn tolerates_unknown_and_malformed_lines() {
        let session = Session::from_text("# comment\nbogus\nnote=Daily/2026-10-30.md\nother=1\n");
        assert_eq!(session.note.as_deref(), Some("Daily/2026-10-30.md"));
        assert!(!session.reading);
    }

    #[test]
    fn save_and_load() {
        let dir = std::env::temp_dir().join(format!(
            "dragon-glass-session-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        assert_eq!(load(&dir), Session::default());

        let session = Session {
            note: Some("Index.md".to_string()),
            reading: true,
        };
        save(&dir, &session).unwrap();
        assert_eq!(load(&dir), session);

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
