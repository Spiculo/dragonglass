//! Git sync, by driving the user's own `git` binary.
//!
//! Why not `libgit2`? Pushing and pulling need the user's credentials, SSH agent and
//! config, and merges need to behave exactly like the command line. Delegating to `git`
//! gives all of that for free, keeps the binary lean, and means conflicts resolve with the
//! same commands the user already knows. We surface conflict instructions rather than
//! pretending to have a merge UI.

use std::path::Path;
use std::process::Command;

use crate::{Error, Result};

pub const DEFAULT_NAME: &str = "Dragon Glass";
pub const DEFAULT_EMAIL: &str = "dragon-glass@localhost";

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Status {
    pub branch: String,
    pub changed: usize,
    pub untracked: usize,
    pub conflicted: Vec<String>,
    pub ahead: usize,
    pub behind: usize,
}

impl Status {
    pub fn is_clean(&self) -> bool {
        self.changed == 0 && self.untracked == 0 && self.conflicted.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Nothing needed doing.
    UpToDate,
    /// Work was pulled or pushed.
    Updated,
    /// The remote has conflicting work; resolve it in a terminal.
    Conflicts(Vec<String>),
    /// The vault has no remote configured.
    NoRemote,
}

pub fn is_repo(root: &Path) -> bool {
    matches!(run(root, &["rev-parse", "--is-inside-work-tree"]), Ok(out) if out.trim() == "true")
}

pub fn init(root: &Path) -> Result<()> {
    run(root, &["init", "-b", "main"])?;
    Ok(())
}

pub fn status(root: &Path) -> Result<Status> {
    let porcelain = run(root, &["status", "--porcelain"])?;
    let mut status = Status {
        branch: current_branch(root),
        ..Status::default()
    };

    for line in porcelain.lines() {
        if line.len() < 3 {
            continue;
        }
        let code = &line[..2];
        let path = line[3..].to_string();

        if code == "??" {
            status.untracked += 1;
        } else if is_conflict_code(code) {
            status.conflicted.push(path);
        } else {
            status.changed += 1;
        }
    }

    let (ahead, behind) = ahead_behind(root);
    status.ahead = ahead;
    status.behind = behind;
    Ok(status)
}

/// Stage everything and commit. Returns `false` when there was nothing to commit.
pub fn commit_all(root: &Path, message: &str) -> Result<bool> {
    run(root, &["add", "-A"])?;

    let staged = run(root, &["diff", "--cached", "--name-only"])?;
    if staged.trim().is_empty() {
        return Ok(false);
    }

    run(
        root,
        &[
            "-c",
            &format!("user.name={DEFAULT_NAME}"),
            "-c",
            &format!("user.email={DEFAULT_EMAIL}"),
            "commit",
            "-m",
            message,
        ],
    )?;
    Ok(true)
}

/// `git pull --ff-only`. Diverged histories are reported, not auto-merged.
pub fn pull(root: &Path) -> Result<Outcome> {
    if !has_remote(root)? {
        return Ok(Outcome::NoRemote);
    }

    let before = head_oid(root);
    match run(root, &["pull", "--ff-only"]) {
        Ok(_) => {
            let after = head_oid(root);
            Ok(if before == after {
                Outcome::UpToDate
            } else {
                Outcome::Updated
            })
        }
        Err(err) => {
            let conflicts = conflicted_files(root);
            if conflicts.is_empty() {
                Err(err)
            } else {
                Ok(Outcome::Conflicts(conflicts))
            }
        }
    }
}

pub fn push(root: &Path) -> Result<Outcome> {
    if !has_remote(root)? {
        return Ok(Outcome::NoRemote);
    }
    run(root, &["push"])?;
    Ok(Outcome::Updated)
}

/// Copy-paste instructions for resolving conflicts in a terminal.
pub fn conflict_instructions(files: &[String]) -> String {
    let mut text = String::from("Conflicts detected:\n");
    for file in files {
        text.push_str(&format!("  {file}\n"));
    }
    text.push_str(
        "\nResolve them in a terminal:\n\
         \x20 git status                 # list conflicted files\n\
         \x20 # edit each file, keep the parts you want, delete the markers\n\
         \x20 git add <file>\n\
         \x20 git commit\n\
         \x20 git push\n",
    );
    text
}

/// Branch name. `symbolic-ref` works even on an unborn branch (no commits yet).
fn current_branch(root: &Path) -> String {
    run(root, &["symbolic-ref", "--short", "-q", "HEAD"])
        .or_else(|_| run(root, &["rev-parse", "--abbrev-ref", "HEAD"]))
        .map(|out| out.trim().to_string())
        .unwrap_or_default()
}

fn head_oid(root: &Path) -> Option<String> {
    run(root, &["rev-parse", "HEAD"])
        .ok()
        .map(|out| out.trim().to_string())
}

fn has_remote(root: &Path) -> Result<bool> {
    Ok(!run(root, &["remote"])?.trim().is_empty())
}

/// `(ahead, behind)` relative to the upstream branch; `(0, 0)` when there is none.
fn ahead_behind(root: &Path) -> (usize, usize) {
    let Ok(output) = run(
        root,
        &["rev-list", "--left-right", "--count", "@{upstream}...HEAD"],
    ) else {
        return (0, 0);
    };
    let mut parts = output.split_whitespace();
    let behind = parts.next().and_then(|v| v.parse().ok()).unwrap_or(0);
    let ahead = parts.next().and_then(|v| v.parse().ok()).unwrap_or(0);
    (ahead, behind)
}

fn conflicted_files(root: &Path) -> Vec<String> {
    run(root, &["diff", "--name-only", "--diff-filter=U"])
        .map(|out| {
            out.lines()
                .map(|l| l.to_string())
                .filter(|l| !l.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

fn is_conflict_code(code: &str) -> bool {
    matches!(code, "DD" | "AU" | "UD" | "UA" | "DU" | "AA" | "UU")
}

fn run(root: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git").args(args).current_dir(root).output()?;
    if !output.status.success() {
        return Err(Error::Git {
            command: format!("git {}", args.join(" ")),
            message: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_repo(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "dragon-glass-git-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn init_status_and_commit_roundtrip() {
        let dir = temp_repo("roundtrip");
        assert!(!is_repo(&dir));

        init(&dir).unwrap();
        assert!(is_repo(&dir));

        let clean = status(&dir).unwrap();
        assert!(clean.is_clean());
        assert_eq!(clean.branch, "main");

        std::fs::write(dir.join("note.md"), "# hello\n").unwrap();
        let dirty = status(&dir).unwrap();
        assert_eq!(dirty.untracked, 1);
        assert!(!dirty.is_clean());

        assert!(commit_all(&dir, "first").unwrap());
        assert!(status(&dir).unwrap().is_clean());

        // Nothing changed: no empty commit.
        assert!(!commit_all(&dir, "second").unwrap());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn pull_without_remote_reports_no_remote() {
        let dir = temp_repo("noremote");
        init(&dir).unwrap();
        assert_eq!(pull(&dir).unwrap(), Outcome::NoRemote);
        assert_eq!(push(&dir).unwrap(), Outcome::NoRemote);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn conflict_instructions_mention_files() {
        let text = conflict_instructions(&["notes/a.md".to_string()]);
        assert!(text.contains("notes/a.md"));
        assert!(text.contains("git status"));
    }
}
