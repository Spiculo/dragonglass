//! Per-vault settings, stored at `.dragon-glass/config.txt` as `key=value` lines.
//!
//! Deliberately a tiny hand-rolled format: no dependency, diffable, and obvious to edit.

use std::path::{Path, PathBuf};

use crate::{storage, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub daily_folder: String,
    pub weekly_folder: String,
    pub monthly_folder: String,
    pub templates_folder: String,
    pub attachments_folder: String,
    /// Optional accent colour as `RRGGBB`.
    pub accent: Option<String>,
    /// Whether `.dragon-glass/snippets` overrides are applied.
    pub snippets: bool,
    /// Whether Vim-style modal editing is enabled for this vault.
    pub vim_mode: bool,
    /// Editor text size in points, persisted so the setting follows the vault.
    pub font_size: u16,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            daily_folder: "Daily".to_string(),
            weekly_folder: "Weekly".to_string(),
            monthly_folder: "Monthly".to_string(),
            templates_folder: "Templates".to_string(),
            attachments_folder: "attachments".to_string(),
            accent: None,
            snippets: true,
            vim_mode: false,
            font_size: 14,
        }
    }
}

impl Config {
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("daily_folder={}\n", self.daily_folder));
        out.push_str(&format!("weekly_folder={}\n", self.weekly_folder));
        out.push_str(&format!("monthly_folder={}\n", self.monthly_folder));
        out.push_str(&format!("templates_folder={}\n", self.templates_folder));
        out.push_str(&format!("attachments_folder={}\n", self.attachments_folder));
        if let Some(accent) = &self.accent {
            out.push_str(&format!("accent={accent}\n"));
        }
        out.push_str(&format!("snippets={}\n", self.snippets));
        out.push_str(&format!("vim_mode={}\n", self.vim_mode));
        out.push_str(&format!("font_size={}\n", self.font_size));
        out
    }

    pub fn from_text(text: &str) -> Self {
        let mut config = Config::default();

        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.trim().to_string();
            if value.is_empty() {
                continue;
            }

            match key.trim() {
                "daily_folder" => config.daily_folder = value,
                "weekly_folder" => config.weekly_folder = value,
                "monthly_folder" => config.monthly_folder = value,
                "templates_folder" => config.templates_folder = value,
                "attachments_folder" => config.attachments_folder = value,
                "accent" => config.accent = Some(value),
                "snippets" => {
                    config.snippets = !matches!(value.to_lowercase().as_str(), "false" | "0" | "no")
                }
                "vim_mode" => {
                    config.vim_mode = !matches!(value.to_lowercase().as_str(), "false" | "0" | "no")
                }
                "font_size" => {
                    if let Ok(size) = value.parse::<u16>() {
                        config.font_size = size.clamp(9, 32);
                    }
                }
                _ => {}
            }
        }

        config
    }
}

fn file(root: &Path) -> PathBuf {
    root.join(".dragon-glass").join("config.txt")
}

pub fn load(root: &Path) -> Config {
    std::fs::read_to_string(file(root))
        .map(|text| Config::from_text(&text))
        .unwrap_or_default()
}

pub fn save(root: &Path, config: &Config) -> Result<()> {
    std::fs::create_dir_all(root.join(".dragon-glass"))?;
    storage::write_atomic(&file(root), &config.to_text())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_sensible() {
        let config = Config::default();
        assert_eq!(config.daily_folder, "Daily");
        assert_eq!(config.attachments_folder, "attachments");
        assert_eq!(config.accent, None);
        assert!(!config.vim_mode);
        assert_eq!(config.font_size, 14);
    }

    #[test]
    fn font_size_roundtrips_and_is_clamped() {
        let config = Config {
            font_size: 18,
            ..Config::default()
        };
        assert_eq!(Config::from_text(&config.to_text()).font_size, 18);
        assert_eq!(Config::from_text("font_size=2").font_size, 9);
        assert_eq!(Config::from_text("font_size=99").font_size, 32);
    }

    #[test]
    fn vim_mode_roundtrips() {
        let config = Config {
            vim_mode: true,
            ..Config::default()
        };
        let parsed = Config::from_text(&config.to_text());
        assert!(parsed.vim_mode);
        assert!(!Config::from_text("vim_mode=false").vim_mode);
    }

    #[test]
    fn roundtrips_through_text() {
        let config = Config {
            daily_folder: "Journal/Daily".to_string(),
            accent: Some("ff8800".to_string()),
            ..Config::default()
        };

        let parsed = Config::from_text(&config.to_text());
        assert_eq!(parsed, config);
    }

    #[test]
    fn ignores_unknown_and_malformed_lines() {
        let config = Config::from_text("# comment\nbogus\ndaily_folder=Notes\nunknown=1\n");
        assert_eq!(config.daily_folder, "Notes");
        assert_eq!(config.weekly_folder, "Weekly");
        assert!(config.snippets);
        assert!(!Config::from_text("snippets=false").snippets);
    }

    #[test]
    fn save_and_load() {
        let dir = std::env::temp_dir().join(format!(
            "dragon-glass-config-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let config = Config {
            monthly_folder: "Monthly Notes".to_string(),
            ..Config::default()
        };
        save(&dir, &config).unwrap();
        assert_eq!(load(&dir), config);

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
