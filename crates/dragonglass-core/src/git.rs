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

/// Commit identity override. Each field is optional: a missing field falls back to the
/// user's own `git config` (`user.name` / `user.email`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Author {
    pub name: Option<String>,
    pub email: Option<String>,
}

impl Author {
    /// Build an author from the per-vault settings, ignoring blank values.
    ///
    /// Returns `None` when neither field is set, so `git` uses the user's own config.
    pub fn new(name: Option<&str>, email: Option<&str>) -> Option<Self> {
        let name = name
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let email = email
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        if name.is_none() && email.is_none() {
            None
        } else {
            Some(Self { name, email })
        }
    }
}

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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DiffStat {
    pub added: usize,
    pub removed: usize,
}

/// Lines added and removed in one file, relative to `HEAD`.
///
/// Untracked files count every line as added; a tracked, unchanged file is `0 / 0`.
pub fn file_diff(root: &Path, relative: &str) -> Result<DiffStat> {
    let path = relative.replace('\\', "/");

    if let Ok(output) = run(root, &["diff", "--numstat", "HEAD", "--", &path]) {
        if let Some(stat) = parse_numstat(&output) {
            return Ok(stat);
        }
    }

    // Repositories with no commits yet have no `HEAD`.
    if let Ok(output) = run(root, &["diff", "--numstat", "--", &path]) {
        if let Some(stat) = parse_numstat(&output) {
            return Ok(stat);
        }
    }

    if run(root, &["ls-files", "--error-unmatch", "--", &path]).is_ok() {
        return Ok(DiffStat::default());
    }

    let text = std::fs::read_to_string(root.join(&path))?;
    Ok(DiffStat {
        added: text.lines().count(),
        removed: 0,
    })
}

fn parse_numstat(output: &str) -> Option<DiffStat> {
    let line = output.lines().find(|line| !line.trim().is_empty())?;
    let mut parts = line.split('\t');
    let added = parts.next()?.trim().parse().ok()?;
    let removed = parts.next()?.trim().parse().ok()?;
    Some(DiffStat { added, removed })
}

/// Stage everything and commit. Returns `false` when there was nothing to commit.
///
/// `author` overrides `user.name` / `user.email` for this commit only. When `None`, `git`
/// uses the user's own configuration, so desktop commits keep the user's normal identity.
pub fn commit_all(root: &Path, message: &str, author: Option<&Author>) -> Result<bool> {
    run(root, &["add", "-A"])?;

    let staged = run(root, &["diff", "--cached", "--name-only"])?;
    if staged.trim().is_empty() {
        return Ok(false);
    }

    let mut args: Vec<String> = Vec::new();
    if let Some(author) = author {
        if let Some(name) = &author.name {
            args.push("-c".to_string());
            args.push(format!("user.name={name}"));
        }
        if let Some(email) = &author.email {
            args.push("-c".to_string());
            args.push(format!("user.email={email}"));
        }
    }
    args.push("commit".to_string());
    args.push("-m".to_string());
    args.push(message.to_string());

    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    run(root, &refs)?;
    Ok(true)
}

/// Update remote-tracking refs without touching the working tree.
///
/// This is the safe half of a sync: it never changes notes, so it can run even when the
/// vault has uncommitted work.
pub fn fetch(root: &Path) -> Result<Outcome> {
    if !has_remote(root)? {
        return Ok(Outcome::NoRemote);
    }
    run(root, &["fetch"])?;
    Ok(Outcome::Updated)
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

    fn test_author() -> Author {
        Author {
            name: Some("Test User".to_string()),
            email: Some("test@example.com".to_string()),
        }
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

        assert!(commit_all(&dir, "first", Some(&test_author())).unwrap());
        assert!(status(&dir).unwrap().is_clean());

        // Nothing changed: no empty commit.
        assert!(!commit_all(&dir, "second", Some(&test_author())).unwrap());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn commit_uses_the_given_author() {
        let dir = temp_repo("author");
        init(&dir).unwrap();
        std::fs::write(dir.join("note.md"), "# hello\n").unwrap();

        let author = Author {
            name: Some("Ada Lovelace".to_string()),
            email: Some("ada@example.com".to_string()),
        };
        commit_all(&dir, "first", Some(&author)).unwrap();

        let log = run(&dir, &["log", "-1", "--format=%an <%ae>"]).unwrap();
        assert_eq!(log.trim(), "Ada Lovelace <ada@example.com>");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn commit_falls_back_to_git_config_without_an_author() {
        let dir = temp_repo("config-author");
        init(&dir).unwrap();
        run(&dir, &["config", "user.name", "Config User"]).unwrap();
        run(&dir, &["config", "user.email", "config@example.com"]).unwrap();
        std::fs::write(dir.join("note.md"), "# hello\n").unwrap();

        commit_all(&dir, "first", None).unwrap();

        let log = run(&dir, &["log", "-1", "--format=%an <%ae>"]).unwrap();
        assert_eq!(log.trim(), "Config User <config@example.com>");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn author_ignores_blank_fields() {
        assert_eq!(Author::new(None, None), None);
        assert_eq!(Author::new(Some("  "), Some("")), None);
        assert_eq!(
            Author::new(Some(" Ada "), None),
            Some(Author {
                name: Some("Ada".to_string()),
                email: None,
            })
        );
    }

    #[test]
    fn file_diff_counts_added_and_removed_lines() {
        let dir = temp_repo("diffstat");
        init(&dir).unwrap();
        std::fs::write(dir.join("note.md"), "one\ntwo\n").unwrap();
        assert_eq!(
            file_diff(&dir, "note.md").unwrap(),
            DiffStat {
                added: 2,
                removed: 0
            }
        );

        commit_all(&dir, "first", Some(&test_author())).unwrap();
        assert_eq!(file_diff(&dir, "note.md").unwrap(), DiffStat::default());

        std::fs::write(dir.join("note.md"), "one\nthree\nfour\n").unwrap();
        assert_eq!(
            file_diff(&dir, "note.md").unwrap(),
            DiffStat {
                added: 2,
                removed: 1
            }
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn pull_without_remote_reports_no_remote() {
        let dir = temp_repo("noremote");
        init(&dir).unwrap();
        assert_eq!(fetch(&dir).unwrap(), Outcome::NoRemote);
        assert_eq!(pull(&dir).unwrap(), Outcome::NoRemote);
        assert_eq!(push(&dir).unwrap(), Outcome::NoRemote);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn fetch_and_pull_fast_forward_remote_work() {
        let remote = temp_repo("ff-remote");
        run(&remote, &["init", "--bare"]).unwrap();

        let root = temp_repo("ff-local");
        init(&root).unwrap();
        std::fs::write(root.join("a.md"), "a\n").unwrap();
        commit_all(&root, "first", Some(&test_author())).unwrap();
        let url = remote.to_string_lossy().to_string();
        run(&root, &["remote", "add", "origin", &url]).unwrap();
        run(&root, &["push", "-u", "origin", "main"]).unwrap();

        let other = temp_repo("ff-other");
        run(&other, &["clone", "-b", "main", &url, "."]).unwrap();
        std::fs::write(other.join("b.md"), "b\n").unwrap();
        commit_all(&other, "second", Some(&test_author())).unwrap();
        run(&other, &["push"]).unwrap();

        assert_eq!(fetch(&root).unwrap(), Outcome::Updated);
        assert_eq!(pull(&root).unwrap(), Outcome::Updated);
        assert!(root.join("b.md").exists());

        std::fs::remove_dir_all(&root).unwrap();
        std::fs::remove_dir_all(&other).unwrap();
        std::fs::remove_dir_all(&remote).unwrap();
    }

    #[test]
    fn conflict_instructions_mention_files() {
        let text = conflict_instructions(&["notes/a.md".to_string()]);
        assert!(text.contains("notes/a.md"));
        assert!(text.contains("git status"));
    }
}
