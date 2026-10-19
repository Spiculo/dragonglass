//! Named layout presets, stored at `.dragon-glass/workspaces.txt`.
//!
//! One workspace per line: `name|graph|reading|note`. Kept trivial to read and diff.

use std::path::{Path, PathBuf};

use crate::{storage, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workspace {
    pub name: String,
    /// Graph pane open.
    pub graph: bool,
    /// Editor in reading mode.
    pub reading: bool,
    /// Vault-relative note that was active.
    pub note: Option<String>,
}

fn file(root: &Path) -> PathBuf {
    root.join(".dragon-glass").join("workspaces.txt")
}

pub fn load(root: &Path) -> Vec<Workspace> {
    std::fs::read_to_string(file(root))
        .map(|text| text.lines().filter_map(parse_line).collect())
        .unwrap_or_default()
}

pub fn save(root: &Path, workspaces: &[Workspace]) -> Result<()> {
    std::fs::create_dir_all(root.join(".dragon-glass"))?;

    let mut text = String::new();
    for workspace in workspaces {
        text.push_str(&format!(
            "{}|{}|{}|{}\n",
            sanitize(&workspace.name),
            u8::from(workspace.graph),
            u8::from(workspace.reading),
            workspace.note.clone().unwrap_or_default()
        ));
    }

    storage::write_atomic(&file(root), &text)?;
    Ok(())
}

/// Insert or replace a workspace by name.
pub fn upsert(workspaces: &mut Vec<Workspace>, workspace: Workspace) {
    match workspaces
        .iter_mut()
        .find(|existing| existing.name == workspace.name)
    {
        Some(existing) => *existing = workspace,
        None => workspaces.push(workspace),
    }
}

fn sanitize(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .filter(|c| *c != '|' && *c != '\n' && *c != '\r')
        .collect();
    cleaned.trim().to_string()
}

fn parse_line(line: &str) -> Option<Workspace> {
    let mut parts = line.split('|');
    let name = parts.next()?.trim().to_string();
    if name.is_empty() {
        return None;
    }

    Some(Workspace {
        name,
        graph: parts.next().map(|v| v.trim() == "1").unwrap_or(false),
        reading: parts.next().map(|v| v.trim() == "1").unwrap_or(false),
        note: parts
            .next()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrips_through_a_file() {
        let dir = std::env::temp_dir().join(format!(
            "dragon-glass-workspaces-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let workspaces = vec![
            Workspace {
                name: "writing".into(),
                graph: false,
                reading: true,
                note: Some("notes/a.md".into()),
            },
            Workspace {
                name: "explore".into(),
                graph: true,
                reading: false,
                note: None,
            },
        ];

        save(&dir, &workspaces).unwrap();
        assert_eq!(load(&dir), workspaces);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn upsert_replaces_by_name() {
        let mut workspaces = vec![Workspace {
            name: "writing".into(),
            graph: false,
            reading: false,
            note: None,
        }];

        upsert(
            &mut workspaces,
            Workspace {
                name: "writing".into(),
                graph: true,
                reading: true,
                note: Some("b.md".into()),
            },
        );

        assert_eq!(workspaces.len(), 1);
        assert!(workspaces[0].graph);
        assert_eq!(workspaces[0].note.as_deref(), Some("b.md"));
    }

    #[test]
    fn tolerates_short_and_malformed_lines() {
        assert!(parse_line("").is_none());
        assert!(parse_line("|1|1|").is_none());

        let parsed = parse_line("solo").unwrap();
        assert_eq!(parsed.name, "solo");
        assert!(!parsed.graph);
        assert_eq!(parsed.note, None);
    }
}
