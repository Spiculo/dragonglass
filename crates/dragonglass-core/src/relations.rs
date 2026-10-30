//! Typed document relations: links, embeds, frontmatter relationships and shared tags.
//!
//! The link graph in [`crate::graph`] answers "what points at what". This module answers
//! the editorial question "how is this document related?" — a note can declare a parent,
//! previous/next companion, a series entry, a related reading item, or a superseded note.
//! Relationships are read from ordinary frontmatter keys, so they travel with the vault and
//! remain editable in any text editor.

use std::collections::{HashMap, HashSet};

use crate::frontmatter::{self, PropertyValue};
use crate::{graph, wikilink, Note, NoteRef};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RelationKind {
    Link,
    Embed,
    Parent,
    Child,
    Previous,
    Next,
    Related,
    Series,
    Supersedes,
    SharedTag,
}

impl RelationKind {
    pub fn label(self) -> &'static str {
        match self {
            RelationKind::Link => "link",
            RelationKind::Embed => "embed",
            RelationKind::Parent => "parent",
            RelationKind::Child => "child",
            RelationKind::Previous => "previous",
            RelationKind::Next => "next",
            RelationKind::Related => "related",
            RelationKind::Series => "series",
            RelationKind::Supersedes => "supersedes",
            RelationKind::SharedTag => "shared tag",
        }
    }

    fn inverse(self) -> Self {
        match self {
            RelationKind::Parent => RelationKind::Child,
            RelationKind::Child => RelationKind::Parent,
            RelationKind::Previous => RelationKind::Next,
            RelationKind::Next => RelationKind::Previous,
            other => other,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Outgoing,
    Incoming,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Relation {
    pub kind: RelationKind,
    pub direction: Direction,
    /// Vault-relative path of the other note (or the tag for `SharedTag`).
    pub target: String,
    /// Alias, heading fragment or frontmatter key that explains the relation.
    pub label: Option<String>,
    /// 1-based source line for links; absent for frontmatter relations.
    pub line: Option<usize>,
}

/// Editorial relations for one note, aligned with `NoteRef` order in [`build`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NoteRelations {
    pub relative: String,
    pub outgoing: Vec<Relation>,
    pub incoming: Vec<Relation>,
    /// Tags this note shares with other notes, with those notes sorted by path.
    pub shared_tags: Vec<(String, Vec<String>)>,
}

impl NoteRelations {
    pub fn is_empty(&self) -> bool {
        self.outgoing.is_empty() && self.incoming.is_empty() && self.shared_tags.is_empty()
    }
}

/// Build relations for every note, in the same order as `notes`.
pub fn build(notes: &[NoteRef]) -> Vec<NoteRelations> {
    let resolver = crate::resolve::NoteIndex::new(notes);
    let mut result: Vec<NoteRelations> = notes
        .iter()
        .map(|note| NoteRelations {
            relative: note.relative.clone(),
            ..NoteRelations::default()
        })
        .collect();

    let mut tag_map: HashMap<String, Vec<usize>> = HashMap::new();

    for (from, note_ref) in notes.iter().enumerate() {
        let Ok(note) = Note::load(&note_ref.path) else {
            continue;
        };

        for (target, label, line, embed) in links_with_lines(&note.source) {
            let Some(to) = resolver.resolve(&target) else {
                continue;
            };
            push_unique(
                &mut result[from].outgoing,
                Relation {
                    kind: if embed {
                        RelationKind::Embed
                    } else {
                        RelationKind::Link
                    },
                    direction: Direction::Outgoing,
                    target: notes[to].relative.clone(),
                    label,
                    line: Some(line),
                },
            );
        }

        for (kind, target) in semantic_relations(&note.source) {
            let Some(to) = resolver.resolve(&target) else {
                continue;
            };
            push_unique(
                &mut result[from].outgoing,
                Relation {
                    kind,
                    direction: Direction::Outgoing,
                    target: notes[to].relative.clone(),
                    label: None,
                    line: None,
                },
            );
        }

        for tag in note.tags() {
            tag_map.entry(tag).or_default().push(from);
        }
    }

    // Add reverse edges for every outgoing relation we found.
    for from in 0..result.len() {
        let outgoing = result[from].outgoing.clone();
        for relation in outgoing {
            let Some(to) = resolver.resolve(&relation.target) else {
                continue;
            };
            if to == from {
                continue;
            }
            push_unique(
                &mut result[to].incoming,
                Relation {
                    kind: relation.kind.inverse(),
                    direction: Direction::Incoming,
                    target: notes[from].relative.clone(),
                    label: relation.label.clone(),
                    line: None,
                },
            );
        }
    }

    // Shared tags: tag -> other notes carrying it.
    for (index, relations) in result.iter_mut().enumerate() {
        let relative = &relations.relative;
        let Some(note_ref) = notes.iter().find(|note| &note.relative == relative) else {
            continue;
        };
        let Ok(note) = Note::load(&note_ref.path) else {
            continue;
        };

        for tag in note.tags() {
            let Some(indices) = tag_map.get(&tag) else {
                continue;
            };
            let others: Vec<String> = indices
                .iter()
                .filter(|other| **other != index)
                .map(|other| notes[*other].relative.clone())
                .collect();
            let mut others = others;
            others.sort();
            others.dedup();
            if !others.is_empty() {
                relations.shared_tags.push((tag, others));
            }
        }

        relations.shared_tags.sort_by(|a, b| a.0.cmp(&b.0));
        relations
            .outgoing
            .sort_by(|a, b| a.kind.cmp(&b.kind).then_with(|| a.target.cmp(&b.target)));
        relations
            .incoming
            .sort_by(|a, b| a.kind.cmp(&b.kind).then_with(|| a.target.cmp(&b.target)));
    }

    result
}

/// Look up one note's relations by path.
pub fn for_note<'a>(relations: &'a [NoteRelations], relative: &str) -> Option<&'a NoteRelations> {
    relations.iter().find(|entry| entry.relative == relative)
}

/// A link graph enriched with semantic frontmatter relations.
///
/// The graph view can paint this exactly like the plain link graph, but it answers "what
/// documents belong together" rather than only "what links where".
pub fn graph(notes: &[NoteRef]) -> graph::Graph {
    let mut graph = graph::build(notes);
    let resolver = crate::resolve::NoteIndex::new(notes);
    let mut seen: HashSet<(usize, usize)> = graph
        .edges
        .iter()
        .map(|edge| (edge.from, edge.to))
        .collect();

    for (from, note_ref) in notes.iter().enumerate() {
        let Ok(note) = Note::load(&note_ref.path) else {
            continue;
        };
        for (kind, target) in semantic_relations(&note.source) {
            let Some(to) = resolver.resolve(&target) else {
                continue;
            };
            if from == to || !seen.insert((from, to)) {
                continue;
            }
            graph.edges.push(graph::Edge {
                from,
                to,
                kind: graph::EdgeKind::Relation(kind.label()),
            });
            if let Some(node) = graph.nodes.get_mut(from) {
                node.links += 1;
            }
        }
    }

    graph
}

/// The semantic keys this module understands, lowercased.
pub fn relation_key(key: &str) -> Option<RelationKind> {
    match key.trim().to_ascii_lowercase().as_str() {
        "up" | "parent" | "parents" => Some(RelationKind::Parent),
        "down" | "child" | "children" => Some(RelationKind::Child),
        "prev" | "previous" | "prior" => Some(RelationKind::Previous),
        "next" | "next-note" | "next_note" => Some(RelationKind::Next),
        "related" | "relations" | "see-also" | "see_also" | "seealso" => {
            Some(RelationKind::Related)
        }
        "series" => Some(RelationKind::Series),
        "supersedes" | "replaces" | "replaced" => Some(RelationKind::Supersedes),
        _ => None,
    }
}

fn semantic_relations(src: &str) -> Vec<(RelationKind, String)> {
    let properties = frontmatter::properties(src).unwrap_or_default();
    let mut out = Vec::new();

    for property in properties {
        let Some(kind) = relation_key(&property.key) else {
            continue;
        };
        match property.value {
            PropertyValue::Scalar(value) => {
                if let Some(target) = clean_target(&value) {
                    out.push((kind, target));
                }
            }
            PropertyValue::List(items) => {
                for item in items {
                    if let Some(target) = clean_target(&item) {
                        out.push((kind, target));
                    }
                }
            }
            PropertyValue::Raw(_) => {}
        }
    }

    out
}

pub(crate) fn clean_target(raw: &str) -> Option<String> {
    let mut target = raw.trim();
    if target.is_empty() {
        return None;
    }
    if let Some(rest) = target.strip_prefix("[[") {
        target = rest.split("]]").next().unwrap_or(rest);
    }
    let target = target.split('|').next().unwrap_or(target);
    let target = target.split('#').next().unwrap_or(target);
    let target = target.trim().trim_end_matches(".md").trim();
    if target.is_empty() {
        None
    } else {
        Some(target.to_string())
    }
}

/// Wikilinks with their 1-based source lines, skipping frontmatter and fenced code.
fn links_with_lines(src: &str) -> Vec<(String, Option<String>, usize, bool)> {
    let body = frontmatter::split(src).body;
    let offset = src[..src.len().saturating_sub(body.len())]
        .matches('\n')
        .count();

    let mut out = Vec::new();
    let mut in_fence = false;
    for (index, line) in body.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        for link in wikilink::parse(line) {
            if link.target.is_empty() {
                continue;
            }
            let label = link
                .alias
                .clone()
                .or_else(|| link.heading.clone())
                .or_else(|| link.block.as_ref().map(|block| format!("^{block}")));
            out.push((link.target, label, offset + index + 1, link.embed));
        }
    }
    out
}

fn push_unique(list: &mut Vec<Relation>, relation: Relation) {
    if list
        .iter()
        .any(|existing| existing.kind == relation.kind && existing.target == relation.target)
    {
        return;
    }
    list.push(relation);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vault(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "dragon-glass-relations-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn note(dir: &std::path::Path, name: &str, body: &str) -> NoteRef {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&path, body).unwrap();
        NoteRef {
            path,
            relative: name.to_string(),
        }
    }

    #[test]
    fn links_and_embeds_are_typed_and_reversed() {
        let dir = vault("links");
        let notes = vec![
            note(&dir, "a.md", "see [[b|Bee]] and ![[c]]\n"),
            note(&dir, "b.md", "[[a]]\n"),
            note(&dir, "c.md", "end\n"),
        ];
        let relations = build(&notes);

        let a = for_note(&relations, "a.md").unwrap();
        assert!(a.outgoing.iter().any(|r| r.kind == RelationKind::Link
            && r.target == "b.md"
            && r.label.as_deref() == Some("Bee")));
        assert!(a
            .outgoing
            .iter()
            .any(|r| r.kind == RelationKind::Embed && r.target == "c.md"));

        let b = for_note(&relations, "b.md").unwrap();
        assert_eq!(b.incoming.len(), 1);
        assert_eq!(b.incoming[0].target, "a.md");
        assert_eq!(b.incoming[0].kind, RelationKind::Link);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn frontmatter_relations_are_parsed_and_inverted() {
        let dir = vault("semantic");
        let notes = vec![
            note(
                &dir,
                "chapter-2.md",
                "---\nup: chapter-1\nprev: chapter-1\nnext: chapter-3\nrelated: [themes]\n---\nbody\n",
            ),
            note(&dir, "chapter-1.md", "intro\n"),
            note(&dir, "chapter-3.md", "later\n"),
            note(&dir, "themes.md", "themes\n"),
        ];
        let relations = build(&notes);

        let chapter2 = for_note(&relations, "chapter-2.md").unwrap();
        assert!(chapter2
            .outgoing
            .iter()
            .any(|r| r.kind == RelationKind::Parent && r.target == "chapter-1.md"));
        assert!(chapter2
            .outgoing
            .iter()
            .any(|r| r.kind == RelationKind::Next && r.target == "chapter-3.md"));
        assert!(chapter2
            .outgoing
            .iter()
            .any(|r| r.kind == RelationKind::Related && r.target == "themes.md"));

        let chapter1 = for_note(&relations, "chapter-1.md").unwrap();
        assert!(chapter1
            .incoming
            .iter()
            .any(|r| r.kind == RelationKind::Child && r.target == "chapter-2.md"));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn shared_tags_list_other_notes() {
        let dir = vault("tags");
        let notes = vec![
            note(&dir, "a.md", "#alpha shared\n"),
            note(&dir, "b.md", "#alpha\n"),
            note(&dir, "c.md", "#beta\n"),
        ];
        let relations = build(&notes);
        let a = for_note(&relations, "a.md").unwrap();
        assert_eq!(
            a.shared_tags,
            vec![("alpha".to_string(), vec!["b.md".to_string()])]
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn semantic_relations_extend_the_graph() {
        let dir = vault("graph");
        let notes = vec![
            note(&dir, "a.md", "---\nrelated: b\n---\nno links\n"),
            note(&dir, "b.md", "b\n"),
        ];
        let plain = graph::build(&notes);
        assert!(plain.edges.is_empty());

        let enriched = graph(&notes);
        assert_eq!(enriched.edges.len(), 1);
        assert_eq!(enriched.edges[0].from, 0);
        assert_eq!(enriched.edges[0].to, 1);
        assert_eq!(
            enriched.edges[0].kind,
            graph::EdgeKind::Relation(RelationKind::Related.label())
        );
        assert!(enriched.edges[0].is_relation());
        assert_eq!(enriched.nodes[0].links, 1);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn relation_keys_are_recognised() {
        assert_eq!(relation_key("Up"), Some(RelationKind::Parent));
        assert_eq!(relation_key("see-also"), Some(RelationKind::Related));
        assert_eq!(relation_key("tags"), None);
    }

    #[test]
    fn unresolved_targets_are_ignored() {
        let dir = vault("unresolved");
        let notes = vec![note(&dir, "a.md", "[[missing]]\n---\n")];
        let relations = build(&notes);
        assert!(relations[0].outgoing.is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
