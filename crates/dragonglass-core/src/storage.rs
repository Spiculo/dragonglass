//! Crash-safe file writes.
//!
//! A vault is the user's data, so writes go to a temporary file in the same directory and
//! are then renamed over the target. A crash mid-write leaves the original intact.

use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Atomically replace `path` with `contents`.
///
/// The temporary file gets a fresh name and is created with `create_new`, so a planted
/// symlink cannot redirect a save outside the vault. An existing file's permissions are
/// copied to the replacement, and on Unix the parent directory is synced after the rename.
pub fn write_atomic(path: &Path, contents: &str) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("note.md");
    let permissions = std::fs::metadata(path).ok().map(|meta| meta.permissions());

    let mut tmp = None;
    let mut opened = None;
    let mut last_error = None;
    for _ in 0..64 {
        let nonce = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let candidate = dir.join(format!(
            ".{name}.dragon-glass.{}.{nonce}.tmp",
            std::process::id()
        ));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(file) => {
                tmp = Some(candidate);
                opened = Some(file);
                break;
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                last_error = Some(error);
            }
            Err(error) => return Err(error),
        }
    }

    let tmp = tmp.ok_or_else(|| {
        last_error.unwrap_or_else(|| {
            io::Error::new(
                io::ErrorKind::AlreadyExists,
                "could not create a temporary file",
            )
        })
    })?;
    let mut file = opened.expect("temporary file was created");

    let write_result = (|| -> std::io::Result<()> {
        file.write_all(contents.as_bytes())?;
        if let Some(permissions) = permissions {
            // Preserve a user's mode bits instead of falling back to the umask.
            file.set_permissions(permissions)?;
        }
        file.sync_all()?;
        Ok(())
    })();

    if write_result.is_err() {
        let _ = std::fs::remove_file(&tmp);
        return write_result;
    }

    if let Err(error) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(error);
    }

    #[cfg(unix)]
    {
        // A rename is only durable once the directory entry is synced.
        if let Ok(directory) = std::fs::File::open(dir) {
            let _ = directory.sync_all();
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn write_atomic_keeps_permissions_and_ignores_planted_temp_symlinks() {
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir().join(format!(
            "dragon-glass-write-mode-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("note.md");
        let outside = dir.join("outside.txt");
        std::fs::write(&path, "old").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        std::fs::write(&outside, "do not touch").unwrap();

        // A vault can ship a symlink at the old predictable temp name. The new write must
        // use a fresh name and leave the target alone.
        std::os::unix::fs::symlink(&outside, dir.join(".note.md.dragon-glass.tmp")).unwrap();
        write_atomic(&path, "new").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new");
        assert_eq!(std::fs::read_to_string(&outside).unwrap(), "do not touch");
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);

        std::fs::remove_dir_all(&dir).unwrap();
    }

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
