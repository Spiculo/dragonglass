//! Per-vault session state: the tabs to reopen on the next launch, with their mode and
//! position.
//!
//! Stored at `.dragon-glass/session.txt` as `key=value` lines, mirroring `config.txt`.
//! Like the rest of `.dragon-glass`, it is a rebuildable convenience file: losing it only
//! means the app opens the vault without a note selected.
//!
//! Format (`session=2`):
//!
//! ```text
//! version=2
//! active=0
//! tab=notes/a.md|1|12|3|0.0|4
//! ```
//!
//! A `tab` line is `note|reading|line|col|scroll|reading_block|anchor|folds`. The note is
//! vault-relative and empty for an untitled tab; `reading` is `0` or `1`; the rest are
//! 0-based cursor columns, the editor's vertical scroll offset in pixels, the reading view's
//! top block, the selection anchor as a character index (empty when there is no selection)
//! and a comma-separated list of folded heading indices. Unknown keys and malformed fields
//! are ignored, and version 1 files (`note=…`, `reading=…`) still load as a single tab.

use std::path::{Path, PathBuf};

use crate::{storage, Result};

/// Sessions grow one tab per opened note, so cap them to keep the file small.
pub const MAX_TABS: usize = 32;

/// One remembered tab.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TabSession {
    /// Vault-relative path. Empty means an untitled tab.
    pub note: String,
    pub reading: bool,
    /// Caret line and column, 0-based.
    pub line: usize,
    pub col: usize,
    /// The editor's vertical scroll offset, in pixels.
    pub scroll: f32,
    /// The reading view's top block index.
    pub reading_block: usize,
    /// Selection anchor as a character index; the caret end is `line`/`col`.
    pub anchor: Option<usize>,
    /// Heading indices whose sections are folded in the editor.
    pub folded: Vec<usize>,
    /// Heading block indices folded in the reading view.
    pub reading_folded: Vec<usize>,
}

/// The tabs that were open, and which one was active.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Session {
    pub version: u32,
    pub active: usize,
    pub tabs: Vec<TabSession>,
}

impl Session {
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        out.push_str("version=2\n");
        out.push_str(&format!("active={}\n", self.active.min(self.tabs.len())));
        for tab in self.tabs.iter().take(MAX_TABS) {
            let folds: Vec<String> = tab.folded.iter().map(|index| index.to_string()).collect();
            let reading_folds: Vec<String> = tab
                .reading_folded
                .iter()
                .map(|index| index.to_string())
                .collect();
            out.push_str(&format!(
                "tab={}|{}|{}|{}|{:.1}|{}|{}|{}|{}\n",
                tab.note,
                u8::from(tab.reading),
                tab.line,
                tab.col,
                tab.scroll.max(0.0),
                tab.reading_block,
                tab.anchor
                    .map(|anchor| anchor.to_string())
                    .unwrap_or_default(),
                folds.join(","),
                reading_folds.join(",")
            ));
        }
        out
    }

    pub fn from_text(text: &str) -> Self {
        let mut session = Session {
            version: 2,
            ..Session::default()
        };
        let mut legacy_note: Option<String> = None;
        let mut legacy_reading = false;

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
                "version" => {
                    session.version = value.parse().unwrap_or(2);
                }
                "active" => {
                    session.active = value.parse().unwrap_or(0);
                }
                "tab" => {
                    if let Some(tab) = parse_tab(value) {
                        if session.tabs.len() < MAX_TABS {
                            session.tabs.push(tab);
                        }
                    }
                }
                // Version 1: a single note and its mode.
                "note" => {
                    if !value.is_empty() {
                        legacy_note = Some(value.to_string());
                    }
                }
                "reading" => {
                    legacy_reading =
                        !matches!(value.to_lowercase().as_str(), "0" | "false" | "no" | "");
                }
                _ => {}
            }
        }

        if session.tabs.is_empty() {
            if let Some(note) = legacy_note {
                session.tabs.push(TabSession {
                    note,
                    reading: legacy_reading,
                    ..TabSession::default()
                });
            }
        }
        if session.active >= session.tabs.len() {
            session.active = session.tabs.len().saturating_sub(1);
        }

        session
    }
}

/// Parse one `tab=` line, tolerating missing or unreadable trailing fields.
fn parse_tab(value: &str) -> Option<TabSession> {
    let mut fields = value.split('|');
    let note = fields.next()?.trim().to_string();
    let mut tab = TabSession {
        note,
        ..TabSession::default()
    };
    if let Some(reading) = fields.next() {
        tab.reading = !matches!(reading.trim(), "0" | "false" | "no" | "");
    }
    tab.line = fields
        .next()
        .and_then(|field| field.trim().parse().ok())
        .unwrap_or(0);
    tab.col = fields
        .next()
        .and_then(|field| field.trim().parse().ok())
        .unwrap_or(0);
    tab.scroll = fields
        .next()
        .and_then(|field| field.trim().parse::<f32>().ok())
        .filter(|scroll| scroll.is_finite() && *scroll >= 0.0)
        .unwrap_or(0.0);
    tab.reading_block = fields
        .next()
        .and_then(|field| field.trim().parse().ok())
        .unwrap_or(0);
    tab.anchor = fields.next().and_then(|field| field.trim().parse().ok());
    if let Some(folds) = fields.next() {
        tab.folded = folds
            .split(',')
            .filter_map(|index| index.trim().parse().ok())
            .collect();
    }
    if let Some(folds) = fields.next() {
        tab.reading_folded = folds
            .split(',')
            .filter_map(|index| index.trim().parse().ok())
            .collect();
    }
    // A tab with no note and every other field at its default is an untitled buffer, which
    // is valid; anything else is at least a note path.
    Some(tab)
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
            version: 2,
            active: 1,
            tabs: vec![
                TabSession {
                    note: "notes/a.md".to_string(),
                    reading: true,
                    line: 12,
                    col: 3,
                    scroll: 240.0,
                    reading_block: 4,
                    anchor: Some(9),
                    folded: vec![0, 3],
                    reading_folded: vec![2],
                },
                TabSession {
                    note: "notes/b.md".to_string(),
                    reading: false,
                    line: 0,
                    col: 0,
                    scroll: 0.0,
                    reading_block: 0,
                    anchor: None,
                    folded: Vec::new(),
                    reading_folded: Vec::new(),
                },
            ],
        };
        assert_eq!(Session::from_text(&session.to_text()), session);
    }

    #[test]
    fn a_tab_without_the_new_fields_still_loads() {
        // The format before selection and folds were stored.
        let session = Session::from_text("version=2\nactive=0\ntab=notes/a.md|1|12|3|0.0|4\n");
        assert_eq!(session.tabs[0].anchor, None);
        assert!(session.tabs[0].folded.is_empty());
        assert!(session.tabs[0].reading_folded.is_empty());
        assert_eq!(session.tabs[0].line, 12);
    }

    #[test]
    fn an_untitled_tab_roundtrips() {
        let session = Session {
            version: 2,
            active: 0,
            tabs: vec![TabSession::default()],
        };
        assert_eq!(Session::from_text(&session.to_text()), session);
    }

    #[test]
    fn version_one_files_still_load() {
        let session = Session::from_text("note=Daily/2026-10-30.md\nreading=1\n");
        assert_eq!(session.tabs.len(), 1);
        assert_eq!(session.tabs[0].note, "Daily/2026-10-30.md");
        assert!(session.tabs[0].reading);
        assert_eq!(session.active, 0);
    }

    #[test]
    fn tolerates_unknown_and_malformed_lines() {
        let session = Session::from_text("# comment\nbogus\nnote=Daily/a.md\nother=1\ntab=|1\n");
        assert_eq!(session.tabs.len(), 1);
        // A tab line wins over the legacy single-note keys, and a missing leading field is
        // an untitled tab rather than a parse failure.
        assert_eq!(session.tabs[0].note, "");
        assert!(session.tabs[0].reading);
        assert_eq!(session.tabs[0].line, 0);
        assert_eq!(session.active, 0);
    }

    #[test]
    fn the_tab_list_is_capped_and_active_is_clamped() {
        let mut text = String::from("version=2\nactive=99\n");
        for index in 0..MAX_TABS + 10 {
            text.push_str(&format!("tab=note-{index}.md|0|1|2|3.0|4\n"));
        }
        let session = Session::from_text(&text);
        assert_eq!(session.tabs.len(), MAX_TABS);
        assert_eq!(session.active, MAX_TABS - 1);
    }

    #[test]
    fn a_bad_scroll_value_falls_back_to_zero() {
        let session = Session::from_text("tab=a.md|0|1|2|not-a-number|4\n");
        assert_eq!(session.tabs[0].scroll, 0.0);
        assert_eq!(session.tabs[0].reading_block, 4);
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
            version: 2,
            active: 0,
            tabs: vec![TabSession {
                note: "Index.md".to_string(),
                reading: true,
                line: 3,
                col: 0,
                scroll: 12.0,
                reading_block: 2,
                anchor: None,
                folded: vec![1],
                reading_folded: vec![2, 4],
            }],
        };
        save(&dir, &session).unwrap();
        assert_eq!(load(&dir), session);

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
