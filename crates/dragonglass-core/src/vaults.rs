//! The list of known vaults, kept outside any vault.
//!
//! The app needs to know which vault to open before it can read that vault's own config,
//! so the "last opened" path and the recent list live in the user's config directory:
//!
//! - Windows: `%APPDATA%\DragonGlass\vaults.txt`
//! - macOS: `~/Library/Application Support/DragonGlass/vaults.txt`
//! - Linux/other: `$XDG_CONFIG_HOME/dragon-glass/vaults.txt` (or `~/.config/...`)
//!
//! `DRAGON_GLASS_CONFIG_DIR` overrides the directory, which is handy for tests and for a
//! portable install. One absolute path per line, most recent first.

use std::path::{Path, PathBuf};

use crate::{storage, Result};

/// How many vaults to remember. Old entries fall off the end.
pub const MAX_RECENT: usize = 12;

const FILE_NAME: &str = "vaults.txt";

/// The persistent vault list: recent paths, most recent first.
#[derive(Debug, Clone)]
pub struct Registry {
    dir: PathBuf,
    recent: Vec<PathBuf>,
}

impl Registry {
    /// Load the registry from the platform config directory (or the override).
    pub fn load() -> Self {
        Self::load_in(config_dir())
    }

    /// Load the registry stored in `dir`. A missing or unreadable file is an empty list.
    pub fn load_in(dir: impl Into<PathBuf>) -> Self {
        let dir = dir.into();
        let recent = std::fs::read_to_string(file(&dir))
            .map(|text| parse(&text))
            .unwrap_or_default();
        Self { dir, recent }
    }

    /// The directory the registry is stored in.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The registry file itself.
    pub fn file(&self) -> PathBuf {
        file(&self.dir)
    }

    /// Recent vaults, most recent first.
    pub fn recent(&self) -> &[PathBuf] {
        &self.recent
    }

    /// The vault to reopen on launch, if one has been opened before.
    pub fn last(&self) -> Option<&Path> {
        self.recent.first().map(PathBuf::as_path)
    }

    /// Move `path` to the front of the recent list and persist the list.
    ///
    /// The path is canonicalized when it exists, so `.\notes` and `C:\notes` are the same
    /// vault. Saving is best-effort at the call site, but the error is returned so the
    /// caller can surface it.
    pub fn record(&mut self, path: impl AsRef<Path>) -> Result<()> {
        let path = canonical(path.as_ref());
        self.recent.retain(|existing| !same(existing, &path));
        self.recent.insert(0, path);
        self.recent.truncate(MAX_RECENT);
        self.save()
    }

    /// Drop a vault from the recent list and persist the change.
    pub fn forget(&mut self, path: &Path) -> Result<()> {
        self.recent.retain(|existing| !same(existing, path));
        self.save()
    }

    fn save(&self) -> Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let text: String = self
            .recent
            .iter()
            .map(|path| format!("{}\n", path.to_string_lossy()))
            .collect();
        storage::write_atomic(&file(&self.dir), &text)?;
        Ok(())
    }
}

fn file(dir: &Path) -> PathBuf {
    dir.join(FILE_NAME)
}

/// Where the registry lives when no explicit directory is configured.
pub fn config_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("DRAGON_GLASS_CONFIG_DIR") {
        if !dir.is_empty() {
            return PathBuf::from(dir);
        }
    }

    #[cfg(target_os = "windows")]
    {
        if let Some(appdata) = std::env::var_os("APPDATA").filter(|value| !value.is_empty()) {
            return PathBuf::from(appdata).join("DragonGlass");
        }
        if let Some(profile) = std::env::var_os("USERPROFILE").filter(|value| !value.is_empty()) {
            return PathBuf::from(profile)
                .join("AppData")
                .join("Roaming")
                .join("DragonGlass");
        }
    }

    #[cfg(target_os = "macos")]
    {
        if let Some(home) = std::env::var_os("HOME").filter(|value| !value.is_empty()) {
            return PathBuf::from(home)
                .join("Library")
                .join("Application Support")
                .join("DragonGlass");
        }
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        if let Some(config) = std::env::var_os("XDG_CONFIG_HOME").filter(|value| !value.is_empty())
        {
            return PathBuf::from(config).join("dragon-glass");
        }
        if let Some(home) = std::env::var_os("HOME").filter(|value| !value.is_empty()) {
            return PathBuf::from(home).join(".config").join("dragon-glass");
        }
    }

    std::env::temp_dir().join("dragon-glass")
}

/// Canonicalize `path` when it exists, and drop Windows' `\\?\` verbatim prefix.
///
/// The prefix is an implementation detail of `canonicalize`: it is noise in the UI and in
/// the vault list, and plain paths are what a user expects to see (and copy). It also keeps
/// deduplication working when a path arrives from a dialog, a shortcut or the list itself.
pub fn canonical(path: &Path) -> PathBuf {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    strip_verbatim(path)
}

fn strip_verbatim(path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy();

    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{rest}"))
    } else if let Some(rest) = text.strip_prefix(r"\\?\") {
        PathBuf::from(rest.to_string())
    } else {
        path
    }
}

/// Compare paths the way the platform would: separators and, on Windows, case are
/// insensitive.
fn same(a: &Path, b: &Path) -> bool {
    let normalize = |path: &Path| {
        let mut text = path.to_string_lossy().replace('\\', "/");
        if let Some(rest) = text.strip_prefix("//?/UNC/") {
            text = format!("//{rest}");
        } else if let Some(rest) = text.strip_prefix("//?/") {
            text = rest.to_string();
        }
        if cfg!(windows) {
            text.to_lowercase()
        } else {
            text
        }
    };
    normalize(a) == normalize(b)
}

fn parse(text: &str) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = Vec::new();

    // Tolerate a BOM from editors that add one.
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let path = strip_verbatim(PathBuf::from(line));
        if paths.iter().any(|existing| same(existing, &path)) {
            continue;
        }
        paths.push(path);
    }

    paths.truncate(MAX_RECENT);
    paths
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "dragon-glass-vaults-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn records_most_recent_first_and_deduplicates() {
        let dir = scratch("record");
        let a = dir.join("a");
        let b = dir.join("b");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();

        let mut registry = Registry::load_in(&dir);
        assert!(registry.recent().is_empty());
        assert_eq!(registry.last(), None);

        registry.record(&a).unwrap();
        registry.record(&b).unwrap();
        registry.record(&a).unwrap();

        assert_eq!(registry.recent(), [canonical(&a), canonical(&b)]);

        // A fresh load sees the same order.
        let reloaded = Registry::load_in(&dir);
        assert_eq!(reloaded.recent(), registry.recent());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn forget_removes_only_the_named_vault() {
        let dir = scratch("forget");
        let a = dir.join("a");
        let b = dir.join("b");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        let canonical_a = canonical(&a);

        let mut registry = Registry::load_in(&dir);
        registry.record(&a).unwrap();
        registry.record(&b).unwrap();
        registry.forget(&canonical_a).unwrap();

        assert_eq!(registry.recent(), [canonical(&b)]);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn caps_the_list_at_the_limit() {
        let dir = scratch("cap");
        let mut registry = Registry::load_in(&dir);
        for index in 0..(MAX_RECENT + 3) {
            registry.record(dir.join(format!("vault-{index}"))).unwrap();
        }
        assert_eq!(registry.recent().len(), MAX_RECENT);
        assert!(registry.recent()[0].ends_with(format!("vault-{}", MAX_RECENT + 2)));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn parses_blank_comments_and_duplicates() {
        let parsed = parse("\n# comment\n/nonexistent/a\n\n/nonexistent/a\n/nonexistent/b\n");
        assert_eq!(
            parsed,
            [
                PathBuf::from("/nonexistent/a"),
                PathBuf::from("/nonexistent/b")
            ]
        );
    }

    #[test]
    fn strips_a_byte_order_mark() {
        let parsed = parse("\u{feff}/nonexistent/a\n");
        assert_eq!(parsed, [PathBuf::from("/nonexistent/a")]);
    }

    #[cfg(windows)]
    #[test]
    fn strips_verbatim_prefixes() {
        assert_eq!(
            strip_verbatim(PathBuf::from(r"\\?\C:\notes")),
            PathBuf::from(r"C:\notes")
        );
        assert_eq!(
            strip_verbatim(PathBuf::from(r"\\?\UNC\server\share")),
            PathBuf::from(r"\\server\share")
        );
        // A path without the prefix is left alone.
        assert_eq!(
            strip_verbatim(PathBuf::from(r"C:\notes")),
            PathBuf::from(r"C:\notes")
        );
    }

    #[cfg(windows)]
    #[test]
    fn same_ignores_verbatim_prefixes_and_case() {
        assert!(same(Path::new(r"\\?\C:\Notes"), Path::new(r"c:\notes")));
    }

    #[test]
    fn missing_files_load_as_empty() {
        let dir = scratch("missing");
        std::fs::remove_dir_all(&dir).unwrap();
        let registry = Registry::load_in(&dir);
        assert!(registry.recent().is_empty());
    }
}
