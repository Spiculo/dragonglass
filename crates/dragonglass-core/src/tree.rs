//! Vault directory tree, for the sidebar.
//!
//! Built from `NoteRef`s, so it is pure and unit-testable.

use crate::NoteRef;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    Dir,
    Note,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeNode {
    /// Display name. For notes this is the file stem (no `.md`).
    pub name: String,
    /// Vault-relative path. Empty for the root.
    pub path: String,
    pub kind: NodeKind,
    pub children: Vec<TreeNode>,
}

impl TreeNode {
    fn root() -> Self {
        Self {
            name: String::new(),
            path: String::new(),
            kind: NodeKind::Dir,
            children: Vec::new(),
        }
    }
}

/// Build a directory tree from a sorted list of notes.
pub fn build(notes: &[NoteRef]) -> TreeNode {
    let mut root = TreeNode::root();
    for note in notes {
        insert(&mut root, note);
    }
    sort(&mut root);
    root
}

fn insert(root: &mut TreeNode, note: &NoteRef) {
    let parts: Vec<&str> = note.relative.split('/').collect();
    let mut node = root;
    let mut prefix = String::new();

    for (i, part) in parts.iter().enumerate() {
        if !prefix.is_empty() {
            prefix.push('/');
        }
        prefix.push_str(part);

        if i + 1 == parts.len() {
            node.children.push(TreeNode {
                name: strip_extension(part).to_string(),
                path: prefix.clone(),
                kind: NodeKind::Note,
                children: Vec::new(),
            });
        } else {
            let existing = node
                .children
                .iter()
                .position(|c| c.kind == NodeKind::Dir && c.name == *part);

            let idx = match existing {
                Some(idx) => idx,
                None => {
                    node.children.push(TreeNode {
                        name: (*part).to_string(),
                        path: prefix.clone(),
                        kind: NodeKind::Dir,
                        children: Vec::new(),
                    });
                    node.children.len() - 1
                }
            };
            node = &mut node.children[idx];
        }
    }
}

fn strip_extension(name: &str) -> &str {
    name.rsplit_once('.').map(|(stem, _)| stem).unwrap_or(name)
}

fn sort(node: &mut TreeNode) {
    node.children.sort_by(|a, b| {
        let rank = |kind: NodeKind| match kind {
            NodeKind::Dir => 0u8,
            NodeKind::Note => 1u8,
        };
        rank(a.kind)
            .cmp(&rank(b.kind))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    for child in &mut node.children {
        sort(child);
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn note(relative: &str) -> NoteRef {
        NoteRef {
            path: PathBuf::from(relative),
            relative: relative.to_string(),
        }
    }

    #[test]
    fn flat_notes_exclude_extension() {
        let tree = build(&[note("a.md"), note("b.md")]);
        let names: Vec<_> = tree.children.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["a", "b"]);
        assert!(tree.children.iter().all(|c| c.kind == NodeKind::Note));
    }

    #[test]
    fn nests_directories_and_sorts_dirs_first() {
        let tree = build(&[note("zeta.md"), note("dir/one.md"), note("alpha.md")]);
        let kinds: Vec<_> = tree.children.iter().map(|c| c.kind).collect();
        assert_eq!(kinds, vec![NodeKind::Dir, NodeKind::Note, NodeKind::Note]);
        assert_eq!(tree.children[0].name, "dir");
        assert_eq!(tree.children[0].children[0].name, "one");
        assert_eq!(tree.children[1].name, "alpha");
        assert_eq!(tree.children[2].name, "zeta");
    }

    #[test]
    fn shares_parent_directories() {
        let tree = build(&[note("a/x.md"), note("a/y.md")]);
        assert_eq!(tree.children.len(), 1);
        assert_eq!(tree.children[0].children.len(), 2);
    }

    #[test]
    fn preserves_vault_relative_paths() {
        let tree = build(&[note("a/b/c.md")]);
        let c = &tree.children[0].children[0].children[0];
        assert_eq!(c.path, "a/b/c.md");
        assert_eq!(c.kind, NodeKind::Note);
    }
}
