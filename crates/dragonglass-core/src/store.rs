//! A local package store: plugins and themes that can be installed into a vault.
//!
//! Nothing is fetched from the network. A store is a directory of packages; each package is
//! a directory containing either `manifest.json` (a plugin) or `theme.txt` (a theme).

use std::path::{Path, PathBuf};

use crate::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageKind {
    Plugin,
    Theme,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Package {
    pub id: String,
    pub name: String,
    pub kind: PackageKind,
    pub version: Option<String>,
    pub path: PathBuf,
}

/// Every installable package in `store`, sorted by id.
pub fn scan(store: &Path) -> Vec<Package> {
    let mut found = Vec::new();

    let Ok(entries) = std::fs::read_dir(store) else {
        return found;
    };

    for entry in entries.filter_map(|entry| entry.ok()) {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let Ok(id) = dir
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .ok_or(())
        else {
            continue;
        };
        if id.is_empty() {
            continue;
        }

        if dir.join("manifest.json").is_file() {
            let (name, version) = plugin_meta(&dir, &id);
            found.push(Package {
                id,
                name,
                kind: PackageKind::Plugin,
                version,
                path: dir,
            });
        } else if dir.join("theme.txt").is_file() {
            let name = std::fs::read_to_string(dir.join("theme.txt"))
                .map(|text| parse_theme(&text, &id).name)
                .unwrap_or_else(|_| id.clone());
            found.push(Package {
                id,
                name,
                kind: PackageKind::Theme,
                version: None,
                path: dir,
            });
        }
    }

    found.sort_by(|a, b| a.id.cmp(&b.id));
    found
}

/// Copy a package into the vault. Returns where it landed.
pub fn install(vault_root: &Path, package: &Package) -> Result<PathBuf> {
    let destination = match package.kind {
        PackageKind::Plugin => vault_root.join(".dragon-glass").join("plugins"),
        PackageKind::Theme => vault_root.join(".dragon-glass").join("themes"),
    }
    .join(&package.id);

    copy_dir(&package.path, &destination)?;
    Ok(destination)
}

/// Directory holding installed themes.
pub fn themes_dir(vault_root: &Path) -> PathBuf {
    vault_root.join(".dragon-glass").join("themes")
}

/// A theme described by a `theme.txt` file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeFile {
    pub name: String,
    pub dark: bool,
    /// `(key, RRGGBB)` pairs in the order they appeared.
    pub colors: Vec<(String, String)>,
}

/// Parse `key=value` theme text: `name`, `dark`, and `RRGGBB` colour keys.
pub fn parse_theme(text: &str, fallback_name: &str) -> ThemeFile {
    let mut theme = ThemeFile {
        name: fallback_name.to_string(),
        dark: true,
        colors: Vec::new(),
    };

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim().to_lowercase();
        let value = value.trim();
        if value.is_empty() {
            continue;
        }

        match key.as_str() {
            "name" => theme.name = value.to_string(),
            "dark" => theme.dark = !matches!(value.to_lowercase().as_str(), "false" | "0" | "no"),
            _ => {
                let cleaned = value.trim_start_matches('#').to_lowercase();
                if is_hex_color(&cleaned) {
                    theme.colors.push((key, cleaned));
                }
            }
        }
    }

    theme
}

fn is_hex_color(value: &str) -> bool {
    value.len() == 6 && value.chars().all(|c| c.is_ascii_hexdigit())
}

fn plugin_meta(dir: &Path, fallback: &str) -> (String, Option<String>) {
    let Ok(text) = std::fs::read_to_string(dir.join("manifest.json")) else {
        return (fallback.to_string(), None);
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return (fallback.to_string(), None);
    };

    let name = value
        .get("name")
        .and_then(|value| value.as_str())
        .unwrap_or(fallback)
        .to_string();
    let version = value
        .get("version")
        .and_then(|value| value.as_str())
        .map(str::to_string);

    (name, version)
}

fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    std::fs::create_dir_all(to)?;

    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "dragon-glass-store-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn scans_plugins_and_themes() {
        let store = temp_dir("scan");

        let plugin = store.join("demo");
        std::fs::create_dir_all(&plugin).unwrap();
        std::fs::write(
            plugin.join("manifest.json"),
            r#"{"id":"demo","name":"Demo Plugin","version":"2.0.0"}"#,
        )
        .unwrap();

        let theme = store.join("nord");
        std::fs::create_dir_all(&theme).unwrap();
        std::fs::write(theme.join("theme.txt"), "name=Nord\ndark=true\nbg=2e3440\n").unwrap();

        let packages = scan(&store);
        assert_eq!(packages.len(), 2);
        assert_eq!(packages[0].id, "demo");
        assert_eq!(packages[0].name, "Demo Plugin");
        assert_eq!(packages[0].kind, PackageKind::Plugin);
        assert_eq!(packages[0].version.as_deref(), Some("2.0.0"));
        assert_eq!(packages[1].name, "Nord");
        assert_eq!(packages[1].kind, PackageKind::Theme);

        std::fs::remove_dir_all(&store).unwrap();
    }

    #[test]
    fn installs_a_package_into_the_vault() {
        let store = temp_dir("install-store");
        let vault = temp_dir("install-vault");

        let theme = store.join("nord");
        std::fs::create_dir_all(&theme).unwrap();
        std::fs::write(theme.join("theme.txt"), "name=Nord\nbg=2e3440\n").unwrap();

        let package = scan(&store).into_iter().next().unwrap();
        let installed = install(&vault, &package).unwrap();

        assert!(installed.ends_with("themes/nord") || installed.ends_with("themes\\nord"));
        assert!(themes_dir(&vault).join("nord").join("theme.txt").is_file());

        std::fs::remove_dir_all(&store).unwrap();
        std::fs::remove_dir_all(&vault).unwrap();
    }

    #[test]
    fn parses_theme_files() {
        let theme = parse_theme(
            "# a comment\nname=Midnight\ndark=yes\nbg=#101010\ntext=eeeeee\nbogus=nothex\n",
            "fallback",
        );

        assert_eq!(theme.name, "Midnight");
        assert!(theme.dark);
        assert_eq!(
            theme.colors,
            vec![
                ("bg".to_string(), "101010".to_string()),
                ("text".to_string(), "eeeeee".to_string())
            ]
        );
    }

    #[test]
    fn theme_defaults_are_used_for_missing_fields() {
        let theme = parse_theme("", "fallback");
        assert_eq!(theme.name, "fallback");
        assert!(theme.dark);
        assert!(theme.colors.is_empty());

        assert!(!parse_theme("dark=false", "x").dark);
    }
}
