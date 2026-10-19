//! Plugin manifests and discovery.
//!
//! Plugins live in `<vault>/.dragon-glass/plugins/<id>/`, each with a `manifest.json` and a
//! WebAssembly entry module. The manifest is small and intentionally JSON, matching what
//! plugin authors expect.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::Result;

fn default_entry() -> String {
    "plugin.wasm".to_string()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Command {
    pub id: String,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub version: Option<String>,
    /// Wasm file, relative to the plugin directory.
    #[serde(default = "default_entry")]
    pub entry: String,
    #[serde(default)]
    pub commands: Vec<Command>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginInfo {
    pub manifest: Manifest,
    pub dir: PathBuf,
}

impl PluginInfo {
    pub fn wasm_path(&self) -> PathBuf {
        self.dir.join(&self.manifest.entry)
    }
}

/// Directory that holds this vault's plugins.
pub fn plugins_dir(vault_root: &Path) -> PathBuf {
    vault_root.join(".dragon-glass").join("plugins")
}

/// Every plugin with a readable manifest, sorted by id.
pub fn discover(vault_root: &Path) -> Vec<PluginInfo> {
    let mut found = Vec::new();

    let Ok(entries) = std::fs::read_dir(plugins_dir(vault_root)) else {
        return found;
    };

    for entry in entries.filter_map(|entry| entry.ok()) {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(dir.join("manifest.json")) else {
            continue;
        };
        let Ok(manifest) = serde_json::from_str::<Manifest>(&text) else {
            continue;
        };
        found.push(PluginInfo { manifest, dir });
    }

    found.sort_by(|a, b| a.manifest.id.cmp(&b.manifest.id));
    found
}

/// Load a manifest from an explicit directory.
pub fn load(dir: &Path) -> Result<PluginInfo> {
    let text = std::fs::read_to_string(dir.join("manifest.json"))?;
    let manifest: Manifest = serde_json::from_str(&text)?;
    Ok(PluginInfo {
        manifest,
        dir: dir.to_path_buf(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vault(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "dragon-glass-plugins-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".dragon-glass/plugins")).unwrap();
        dir
    }

    fn write_plugin(vault: &Path, id: &str, manifest: &str) {
        let dir = plugins_dir(vault).join(id);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("manifest.json"), manifest).unwrap();
        std::fs::write(dir.join("plugin.wasm"), b"\0asm").unwrap();
    }

    #[test]
    fn parses_a_manifest_with_defaults() {
        let manifest: Manifest = serde_json::from_str(
            r#"{"id":"hello","name":"Hello","commands":[{"id":"hi","title":"Say hi"}]}"#,
        )
        .unwrap();
        assert_eq!(manifest.id, "hello");
        assert_eq!(manifest.entry, "plugin.wasm");
        assert_eq!(manifest.commands.len(), 1);
        assert_eq!(manifest.version, None);
    }

    #[test]
    fn discovers_plugins_and_ignores_broken_ones() {
        let root = vault("discover");
        write_plugin(&root, "b", r#"{"id":"b","name":"B"}"#);
        write_plugin(&root, "a", r#"{"id":"a","name":"A"}"#);
        write_plugin(&root, "broken", "not json");

        let found = discover(&root);
        assert_eq!(
            found
                .iter()
                .map(|p| p.manifest.id.as_str())
                .collect::<Vec<_>>(),
            vec!["a", "b"]
        );
        assert!(found[0].wasm_path().ends_with("plugin.wasm"));

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn missing_directory_is_not_an_error() {
        let root = std::env::temp_dir().join("dragon-glass-plugins-missing");
        assert!(discover(&root).is_empty());
    }
}
