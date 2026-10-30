//! A small line diff, for comparing note versions and revisions.
//!
//! The algorithm is LCS with common prefix/suffix trimming. It is deterministic and
//! allocation-light for the documents this app edits; pathologically large comparisons
//! fall back to a coarse remove-all/add-all diff instead of burning memory.

/// One line in a diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub kind: LineKind,
    pub text: String,
    /// 1-based line number in the old document, when the line exists there.
    pub old_line: Option<usize>,
    /// 1-based line number in the new document, when the line exists there.
    pub new_line: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Context,
    Removed,
    Added,
}

impl LineKind {
    /// The one-character prefix used by diff views and unified diffs.
    pub fn marker(self) -> char {
        match self {
            LineKind::Context => ' ',
            LineKind::Removed => '-',
            LineKind::Added => '+',
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Diff {
    pub lines: Vec<Line>,
    pub added: usize,
    pub removed: usize,
    pub unchanged: usize,
}

impl Diff {
    pub fn is_empty(&self) -> bool {
        self.added == 0 && self.removed == 0
    }

    /// A compact `+a -r` summary.
    pub fn summary(&self) -> String {
        format!("+{} -{}", self.added, self.removed)
    }
}

/// Compare two documents line by line.
pub fn lines(old: &str, new: &str) -> Diff {
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();

    let mut prefix = 0usize;
    while prefix < old_lines.len()
        && prefix < new_lines.len()
        && old_lines[prefix] == new_lines[prefix]
    {
        prefix += 1;
    }

    let mut suffix = 0usize;
    while suffix < old_lines.len().saturating_sub(prefix)
        && suffix < new_lines.len().saturating_sub(prefix)
        && old_lines[old_lines.len() - 1 - suffix] == new_lines[new_lines.len() - 1 - suffix]
    {
        suffix += 1;
    }

    let old_mid = &old_lines[prefix..old_lines.len() - suffix];
    let new_mid = &new_lines[prefix..new_lines.len() - suffix];

    let mut ops: Vec<Op<'_>> = Vec::with_capacity(old_lines.len() + new_lines.len());
    for line in &old_lines[..prefix] {
        ops.push(Op::Equal(line));
    }
    ops.extend(middle(old_mid, new_mid));
    for line in &old_lines[old_lines.len() - suffix..] {
        ops.push(Op::Equal(line));
    }

    build(ops)
}

#[derive(Clone, Copy)]
enum Op<'a> {
    Equal(&'a str),
    Removed(&'a str),
    Added(&'a str),
}

fn middle<'a>(old: &[&'a str], new: &[&'a str]) -> Vec<Op<'a>> {
    if old.is_empty() && new.is_empty() {
        return Vec::new();
    }

    // Guard against O(n*m) memory on a huge rewrite.
    if old.len().saturating_mul(new.len()) > 1_000_000 {
        let mut ops = Vec::with_capacity(old.len() + new.len());
        ops.extend(old.iter().map(|line| Op::Removed(line)));
        ops.extend(new.iter().map(|line| Op::Added(line)));
        return ops;
    }

    let rows = old.len() + 1;
    let columns = new.len() + 1;
    let mut table = vec![0u32; rows * columns];

    for i in (0..old.len()).rev() {
        for j in (0..new.len()).rev() {
            table[i * columns + j] = if old[i] == new[j] {
                table[(i + 1) * columns + j + 1] + 1
            } else {
                table[(i + 1) * columns + j].max(table[i * columns + j + 1])
            };
        }
    }

    let mut ops = Vec::with_capacity(old.len() + new.len());
    let (mut i, mut j) = (0usize, 0usize);
    while i < old.len() && j < new.len() {
        if old[i] == new[j] {
            ops.push(Op::Equal(old[i]));
            i += 1;
            j += 1;
        } else if table[(i + 1) * columns + j] >= table[i * columns + j + 1] {
            ops.push(Op::Removed(old[i]));
            i += 1;
        } else {
            ops.push(Op::Added(new[j]));
            j += 1;
        }
    }
    ops.extend(old[i..].iter().map(|line| Op::Removed(line)));
    ops.extend(new[j..].iter().map(|line| Op::Added(line)));
    ops
}

fn build(ops: Vec<Op<'_>>) -> Diff {
    let mut diff = Diff::default();
    let mut old_line = 1usize;
    let mut new_line = 1usize;

    for op in ops {
        match op {
            Op::Equal(text) => {
                diff.lines.push(Line {
                    kind: LineKind::Context,
                    text: text.to_string(),
                    old_line: Some(old_line),
                    new_line: Some(new_line),
                });
                diff.unchanged += 1;
                old_line += 1;
                new_line += 1;
            }
            Op::Removed(text) => {
                diff.lines.push(Line {
                    kind: LineKind::Removed,
                    text: text.to_string(),
                    old_line: Some(old_line),
                    new_line: None,
                });
                diff.removed += 1;
                old_line += 1;
            }
            Op::Added(text) => {
                diff.lines.push(Line {
                    kind: LineKind::Added,
                    text: text.to_string(),
                    old_line: None,
                    new_line: Some(new_line),
                });
                diff.added += 1;
                new_line += 1;
            }
        }
    }

    diff
}

/// `(added, removed)` without retaining the full diff.
pub fn counts(old: &str, new: &str) -> (usize, usize) {
    let diff = lines(old, new);
    (diff.added, diff.removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_documents_have_no_changes() {
        let diff = lines("a\nb\n", "a\nb\n");
        assert!(diff.is_empty());
        assert_eq!(diff.unchanged, 2);
        assert!(diff.lines.iter().all(|line| line.kind == LineKind::Context));
    }

    #[test]
    fn detects_an_inserted_line() {
        let diff = lines("a\nc\n", "a\nb\nc\n");
        assert_eq!(diff.added, 1);
        assert_eq!(diff.removed, 0);
        let added = diff
            .lines
            .iter()
            .find(|line| line.kind == LineKind::Added)
            .unwrap();
        assert_eq!(added.text, "b");
        assert_eq!(added.new_line, Some(2));
        assert_eq!(added.old_line, None);
    }

    #[test]
    fn detects_a_removed_line() {
        let diff = lines("a\nb\nc\n", "a\nc\n");
        assert_eq!(diff.removed, 1);
        assert_eq!(diff.added, 0);
        assert_eq!(diff.lines[1].text, "b");
        assert_eq!(diff.lines[1].kind, LineKind::Removed);
    }

    #[test]
    fn detects_a_replacement_as_remove_plus_add() {
        let diff = lines("a\nold\nc\n", "a\nnew\nc\n");
        assert_eq!(diff.summary(), "+1 -1");
        assert_eq!(diff.lines[1].kind, LineKind::Removed);
        assert_eq!(diff.lines[2].kind, LineKind::Added);
    }

    #[test]
    fn line_numbers_track_both_sides() {
        let diff = lines("a\nb\nc\n", "a\nc\nd\n");
        let context = &diff.lines[0];
        assert_eq!((context.old_line, context.new_line), (Some(1), Some(1)));

        let removed = diff
            .lines
            .iter()
            .find(|line| line.text == "b" && line.kind == LineKind::Removed)
            .unwrap();
        assert_eq!(removed.old_line, Some(2));
        assert_eq!(removed.new_line, None);

        let added = diff
            .lines
            .iter()
            .find(|line| line.text == "d" && line.kind == LineKind::Added)
            .unwrap();
        assert_eq!(added.new_line, Some(3));
    }

    #[test]
    fn empty_inputs_are_handled() {
        assert_eq!(lines("", "").lines.len(), 0);
        let diff = lines("", "a\nb\n");
        assert_eq!(diff.added, 2);
        let diff = lines("a\n", "");
        assert_eq!(diff.removed, 1);
    }

    #[test]
    fn trailing_newline_differences_are_not_a_line_change() {
        assert!(lines("a\n", "a").is_empty());
    }

    #[test]
    fn a_huge_rewrite_falls_back_without_panicking() {
        let old: String = (0..1200).map(|i| format!("old {i}\n")).collect();
        let new: String = (0..1200).map(|i| format!("new {i}\n")).collect();
        let diff = lines(&old, &new);
        assert_eq!(diff.removed, 1200);
        assert_eq!(diff.added, 1200);
    }
}
