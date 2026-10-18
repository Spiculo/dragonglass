//! Crash-safe file writes.
//!
//! A vault is the user's data, so writes go to a temporary file in the same directory and
//! are then renamed over the target. A crash mid-write leaves the original intact.

use std::io::Write;
use std::path::Path;

/// Atomically replace `path` with `contents`.
pub fn write_atomic(path: &Path, contents: &str) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("note.md");
    let tmp = dir.join(format!(".{name}.dragon-glass.tmp"));

    let write_result = (|| -> std::io::Result<()> {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(contents.as_bytes())?;
        file.sync_all()?;
        Ok(())
    })();

    if write_result.is_err() {
        let _ = std::fs::remove_file(&tmp);
        return write_result;
    }

    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_atomic_roundtrip_and_overwrite() {
        let dir = std::env::temp_dir().join(format!(
            "dragon-glass-write-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("note.md");

        write_atomic(&path, "hello").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello");

        write_atomic(&path, "world \u{1F600}").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "world \u{1F600}");

        // No temp file left behind.
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains("tmp"))
            .collect();
        assert!(leftovers.is_empty());

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
