//! Minimal template substitution for newly created notes.
//!
//! Obsidian's templating is much richer; this covers the core variables needed for daily
//! notes and note scaffolding.

use std::path::{Path, PathBuf};

/// A markdown file in the vault's templates folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateFile {
    /// Display name without the `.md` extension.
    pub name: String,
    /// Vault-relative path with forward slashes.
    pub relative: String,
    pub path: PathBuf,
}

/// Every `.md` template in `root/folder`, sorted by name.
pub fn list(root: &Path, folder: &str) -> Vec<TemplateFile> {
    let dir = root.join(folder);
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };

    let mut templates: Vec<TemplateFile> = entries
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let path = entry.path();
            if !path.is_file()
                || !path
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .map(|extension| extension.eq_ignore_ascii_case("md"))
                    .unwrap_or(false)
            {
                return None;
            }
            let name = path.file_stem()?.to_string_lossy().to_string();
            if name.is_empty() {
                return None;
            }
            Some(TemplateFile {
                relative: format!("{}/{name}.md", folder.trim_end_matches('/')),
                name,
                path,
            })
        })
        .collect();

    templates.sort_by(|a, b| a.name.cmp(&b.name));
    templates
}

/// Replace `{{key}}` placeholders with the provided values.
pub fn render(template: &str, vars: &[(&str, &str)]) -> String {
    let mut out = template.to_string();
    for (key, value) in vars {
        out = out.replace(&format!("{{{{{}}}}}", key), value);
    }
    out
}

/// Remove the `{{cursor}}` marker, returning the text and the character index where the
/// caret should land.
pub fn strip_cursor(template: &str) -> (String, Option<usize>) {
    let marker = "{{cursor}}";
    match template.find(marker) {
        Some(byte) => {
            let index = template[..byte].chars().count();
            let mut out = String::with_capacity(template.len() - marker.len());
            out.push_str(&template[..byte]);
            out.push_str(&template[byte + marker.len()..]);
            (out, Some(index))
        }
        None => (template.to_string(), None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaces_placeholders() {
        let out = render(
            "# {{title}}\n\n{{date}} at {{time}}\n",
            &[
                ("title", "Today"),
                ("date", "2026-10-19"),
                ("time", "09:30"),
            ],
        );
        assert_eq!(out, "# Today\n\n2026-10-19 at 09:30\n");
    }

    #[test]
    fn leaves_unknown_placeholders() {
        assert_eq!(render("{{nope}}", &[]), "{{nope}}");
    }

    #[test]
    fn lists_templates_from_the_folder() {
        let root = std::env::temp_dir().join(format!(
            "dragon-glass-templates-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("Templates")).unwrap();
        std::fs::write(root.join("Templates/Meeting.md"), "# Meeting\n").unwrap();
        std::fs::write(root.join("Templates/Daily.md"), "# Daily\n").unwrap();
        std::fs::write(root.join("Templates/notes.txt"), "ignore").unwrap();

        let found = list(&root, "Templates");
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].name, "Daily");
        assert_eq!(found[0].relative, "Templates/Daily.md");
        assert_eq!(found[1].name, "Meeting");
        assert!(found[0].path.is_file());

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn cursor_marker_is_removed_and_located() {
        let (text, at) = strip_cursor("# Title\n\n{{cursor}}body");
        assert_eq!(text, "# Title\n\nbody");
        assert_eq!(at, Some(9));
        assert_eq!(strip_cursor("plain").1, None);
    }
}
