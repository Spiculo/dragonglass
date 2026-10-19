//! User keybinding overrides, stored at `.dragon-glass/keybindings.txt`.
//!
//! One `key=Action` pair per line, e.g. `ctrl+shift+g=OpenGraph`.

use std::path::{Path, PathBuf};

use crate::{storage, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    /// Canonical keystroke, as GPUI's `Keystroke::unparse` produces.
    pub key: String,
    /// Action name, e.g. `OpenGraph`.
    pub action: String,
}

fn file(root: &Path) -> PathBuf {
    root.join(".dragon-glass").join("keybindings.txt")
}

pub fn load(root: &Path) -> Vec<Binding> {
    std::fs::read_to_string(file(root))
        .map(|text| text.lines().filter_map(parse_line).collect())
        .unwrap_or_default()
}

pub fn save(root: &Path, bindings: &[Binding]) -> Result<()> {
    std::fs::create_dir_all(root.join(".dragon-glass"))?;

    let mut text = String::new();
    for binding in bindings {
        text.push_str(&format!("{}={}\n", binding.key, binding.action));
    }

    storage::write_atomic(&file(root), &text)?;
    Ok(())
}

/// Replace any existing override for the action.
pub fn upsert(bindings: &mut Vec<Binding>, binding: Binding) {
    match bindings
        .iter_mut()
        .find(|existing| existing.action == binding.action)
    {
        Some(existing) => *existing = binding,
        None => bindings.push(binding),
    }
}

/// The override for an action, if the user has set one.
pub fn key_for_action(bindings: &[Binding], action: &str) -> Option<String> {
    bindings
        .iter()
        .find(|binding| binding.action == action)
        .map(|binding| binding.key.clone())
}

fn parse_line(line: &str) -> Option<Binding> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let (key, action) = line.split_once('=')?;
    let key = key.trim().to_lowercase();
    let action = action.trim().to_string();
    if key.is_empty() || action.is_empty() {
        return None;
    }
    Some(Binding { key, action })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrips_through_a_file() {
        let dir = std::env::temp_dir().join(format!(
            "dragon-glass-keybindings-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let bindings = vec![
            Binding {
                key: "ctrl+alt+g".into(),
                action: "OpenGraph".into(),
            },
            Binding {
                key: "ctrl+alt+o".into(),
                action: "OpenSwitcher".into(),
            },
        ];

        save(&dir, &bindings).unwrap();
        assert_eq!(load(&dir), bindings);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn upsert_replaces_by_action() {
        let mut bindings = vec![Binding {
            key: "ctrl+g".into(),
            action: "OpenGraph".into(),
        }];

        upsert(
            &mut bindings,
            Binding {
                key: "ctrl+alt+g".into(),
                action: "OpenGraph".into(),
            },
        );

        assert_eq!(bindings.len(), 1);
        assert_eq!(
            key_for_action(&bindings, "OpenGraph").as_deref(),
            Some("ctrl+alt+g")
        );
        assert_eq!(key_for_action(&bindings, "Missing"), None);
    }

    #[test]
    fn ignores_malformed_lines_and_normalises_keys() {
        assert_eq!(
            parse_line("CTRL+Shift+G=OpenGraph"),
            Some(Binding {
                key: "ctrl+shift+g".into(),
                action: "OpenGraph".into()
            })
        );
        assert!(parse_line("no-equals-sign").is_none());
        assert!(parse_line("=Action").is_none());
        assert!(parse_line("# comment").is_none());
    }
}
