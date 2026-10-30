//! The link graph over a vault, plus a deterministic force-directed layout.
//!
//! Kept headless and testable: the UI only paints what this produces.

use std::collections::{HashMap, HashSet, VecDeque};

use crate::{Note, NoteRef};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    pub id: usize,
    pub relative: String,
    /// Resolved outgoing links.
    pub links: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum EdgeKind {
    /// A plain `[[wikilink]]`.
    Link,
    /// A `![[embed]]`.
    Embed,
    /// A frontmatter relation (`up`, `prev`, `related`, …), carrying its label.
    Relation(&'static str),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Edge {
    pub from: usize,
    pub to: usize,
    pub kind: EdgeKind,
}

impl Edge {
    pub fn is_relation(self) -> bool {
        matches!(self.kind, EdgeKind::Relation(_))
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Graph {
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
}

impl Graph {
    pub fn neighbors(&self, id: usize) -> Vec<usize> {
        let mut out = Vec::new();
        for edge in &self.edges {
            if edge.from == id {
                out.push(edge.to);
            }
            if edge.to == id {
                out.push(edge.from);
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }

    /// Index of the node for a vault-relative path.
    pub fn index_of(&self, relative: &str) -> Option<usize> {
        self.nodes.iter().position(|node| node.relative == relative)
    }
}

/// Build the graph, resolving `[[links]]` to notes by path, stem, frontmatter name or alias.
pub fn build(notes: &[NoteRef]) -> Graph {
    let index = crate::resolve::NoteIndex::new(notes);

    let mut nodes: Vec<Node> = notes
        .iter()
        .enumerate()
        .map(|(id, note)| Node {
            id,
            relative: note.relative.clone(),
            links: 0,
        })
        .collect();

    let mut edges = Vec::new();
    let mut seen = HashSet::new();

    for (from, note_ref) in notes.iter().enumerate() {
        let Ok(note) = Note::load(&note_ref.path) else {
            continue;
        };
        for link in note.links() {
            if link.target.is_empty() {
                continue;
            }
            let Some(to) = index.resolve(&link.target) else {
                continue;
            };
            if seen.insert((from, to)) {
                edges.push(Edge {
                    from,
                    to,
                    kind: if link.embed {
                        EdgeKind::Embed
                    } else {
                        EdgeKind::Link
                    },
                });
                nodes[from].links += 1;
            }
        }
    }

    Graph { nodes, edges }
}

/// Keep only nodes whose path contains `query`, and the edges between them.
pub fn filter(graph: &Graph, query: &str) -> Graph {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return graph.clone();
    }

    let mut remap: HashMap<usize, usize> = HashMap::new();
    let mut nodes = Vec::new();

    for node in &graph.nodes {
        if node.relative.to_lowercase().contains(&needle) {
            remap.insert(node.id, nodes.len());
            nodes.push(Node {
                id: nodes.len(),
                relative: node.relative.clone(),
                links: node.links,
            });
        }
    }

    let edges = graph
        .edges
        .iter()
        .filter_map(|edge| {
            Some(Edge {
                from: *remap.get(&edge.from)?,
                to: *remap.get(&edge.to)?,
                kind: edge.kind,
            })
        })
        .collect();

    Graph { nodes, edges }
}

/// The top-level folder of a vault-relative path, or `""` for a root note.
pub fn folder_of(relative: &str) -> &str {
    match relative.split_once('/') {
        Some((folder, _)) => folder,
        None => "",
    }
}

/// The subgraph within `depth` hops of `root`, capped at `max_nodes`.
pub fn local(graph: &Graph, root: usize, depth: usize, max_nodes: usize) -> Graph {
    if graph.nodes.is_empty() || root >= graph.nodes.len() {
        return Graph::default();
    }

    let mut order = Vec::new();
    let mut seen = HashSet::new();
    let mut queue = VecDeque::new();

    seen.insert(root);
    queue.push_back((root, 0usize));
    order.push(root);

    while let Some((node, level)) = queue.pop_front() {
        if level >= depth || order.len() >= max_nodes {
            continue;
        }
        for neighbor in graph.neighbors(node) {
            if order.len() >= max_nodes {
                break;
            }
            if seen.insert(neighbor) {
                order.push(neighbor);
                queue.push_back((neighbor, level + 1));
            }
        }
    }

    let mut remap = HashMap::new();
    for (new_id, old_id) in order.iter().enumerate() {
        remap.insert(*old_id, new_id);
    }

    let nodes = order
        .iter()
        .enumerate()
        .map(|(new_id, old_id)| Node {
            id: new_id,
            relative: graph.nodes[*old_id].relative.clone(),
            links: graph.nodes[*old_id].links,
        })
        .collect();

    let edges = graph
        .edges
        .iter()
        .filter_map(|edge| {
            let from = *remap.get(&edge.from)?;
            let to = *remap.get(&edge.to)?;
            Some(Edge {
                from,
                to,
                kind: edge.kind,
            })
        })
        .collect();

    Graph { nodes, edges }
}

/// Deterministic Fruchterman-Reingold layout inside `width` x `height`.
pub fn layout(graph: &Graph, width: f32, height: f32, iterations: usize) -> Vec<(f32, f32)> {
    let n = graph.nodes.len();
    if n == 0 {
        return Vec::new();
    }

    let center = (width / 2.0, height / 2.0);
    let radius = (width.min(height) / 2.0) * 0.7;

    let mut pos: Vec<(f32, f32)> = (0..n)
        .map(|i| {
            let angle = std::f32::consts::TAU * (i as f32) / (n as f32);
            // A small deterministic jitter stops symmetric nodes from stacking.
            let jitter = 1.0 + ((i * 2654435761usize % 1000) as f32 / 1000.0 - 0.5) * 0.1;
            (
                center.0 + radius * angle.cos() * jitter,
                center.1 + radius * angle.sin() * jitter,
            )
        })
        .collect();

    if n == 1 {
        return pos;
    }

    let area = width * height;
    let k = (area / n as f32).sqrt();
    let mut temperature = radius * 0.5;
    let cooling = if iterations > 0 {
        temperature / (iterations as f32 + 1.0)
    } else {
        0.0
    };

    for _ in 0..iterations {
        let mut disp = vec![(0.0f32, 0.0f32); n];

        // Repulsion between every pair.
        for i in 0..n {
            for j in (i + 1)..n {
                let dx = pos[i].0 - pos[j].0;
                let dy = pos[i].1 - pos[j].1;
                let distance = (dx * dx + dy * dy).sqrt().max(0.01);
                let force = k * k / distance;
                let (ux, uy) = (dx / distance, dy / distance);
                disp[i].0 += ux * force;
                disp[i].1 += uy * force;
                disp[j].0 -= ux * force;
                disp[j].1 -= uy * force;
            }
        }

        // Attraction along edges.
        for edge in &graph.edges {
            let dx = pos[edge.from].0 - pos[edge.to].0;
            let dy = pos[edge.from].1 - pos[edge.to].1;
            let distance = (dx * dx + dy * dy).sqrt().max(0.01);
            let force = distance * distance / k;
            let (ux, uy) = (dx / distance, dy / distance);
            disp[edge.from].0 -= ux * force;
            disp[edge.from].1 -= uy * force;
            disp[edge.to].0 += ux * force;
            disp[edge.to].1 += uy * force;
        }

        for i in 0..n {
            let (dx, dy) = disp[i];
            let length = (dx * dx + dy * dy).sqrt().max(0.01);
            let step = length.min(temperature);
            pos[i].0 += dx / length * step;
            pos[i].1 += dy / length * step;
            pos[i].0 = pos[i].0.clamp(0.0, width);
            pos[i].1 = pos[i].1.clamp(0.0, height);
        }

        temperature = (temperature - cooling).max(0.0);
    }

    pos
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vault(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "dragon-glass-graph-{tag}-{}-{:?}",
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
    fn resolves_links_between_notes() {
        let dir = vault("build");
        let notes = vec![
            note(&dir, "a.md", "[[b]] and [[c]]\n"),
            note(&dir, "b.md", "[[a]]\n"),
            note(&dir, "c.md", "no links\n"),
        ];

        let graph = build(&notes);
        assert_eq!(graph.nodes.len(), 3);
        assert_eq!(graph.edges.len(), 3);
        assert_eq!(graph.nodes[0].links, 2);
        assert_eq!(graph.neighbors(0), vec![1, 2]);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn edges_remember_whether_they_were_links_or_embeds() {
        let dir = vault("kinds");
        let notes = vec![
            note(&dir, "a.md", "[[b]] and ![[c]]\n"),
            note(&dir, "b.md", "b\n"),
            note(&dir, "c.md", "c\n"),
        ];

        let graph = build(&notes);
        assert_eq!(graph.edges[0].kind, EdgeKind::Link);
        assert_eq!(graph.edges[1].kind, EdgeKind::Embed);
        assert!(!graph.edges[0].is_relation());

        let filtered = filter(&graph, "md");
        assert_eq!(filtered.edges.len(), 2);
        assert!(filtered
            .edges
            .iter()
            .all(|edge| matches!(edge.kind, EdgeKind::Link | EdgeKind::Embed)));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn local_subgraph_is_bounded_and_connected() {
        let dir = vault("local");
        let notes = vec![
            note(&dir, "a.md", "[[b]]\n"),
            note(&dir, "b.md", "[[c]]\n"),
            note(&dir, "c.md", "[[d]]\n"),
            note(&dir, "d.md", "end\n"),
        ];
        let graph = build(&notes);

        let local = super::local(&graph, 0, 1, 10);
        assert_eq!(local.nodes.len(), 2);
        assert_eq!(local.edges.len(), 1);

        let local = super::local(&graph, 0, 3, 10);
        assert_eq!(local.nodes.len(), 4);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn filters_by_path_substring() {
        let dir = vault("filter");
        let notes = vec![
            note(&dir, "projects/a.md", "[[b]]\n"),
            note(&dir, "projects/b.md", "[[a]]\n"),
            note(&dir, "other/c.md", "[[a]]\n"),
        ];
        let graph = build(&notes);

        let filtered = filter(&graph, "projects");
        assert_eq!(filtered.nodes.len(), 2);
        // a -> b and b -> a both survive; c -> a does not.
        assert_eq!(filtered.edges.len(), 2);
        assert_eq!(filtered.nodes[0].relative, "projects/a.md");

        assert_eq!(filter(&graph, "  ").nodes.len(), 3);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn folder_of_uses_the_first_component() {
        assert_eq!(folder_of("projects/deep/a.md"), "projects");
        assert_eq!(folder_of("a.md"), "");
    }

    #[test]
    fn layout_is_deterministic_and_in_bounds() {
        let dir = vault("layout");
        let notes = vec![
            note(&dir, "a.md", "[[b]]\n"),
            note(&dir, "b.md", "[[c]]\n"),
            note(&dir, "c.md", "[[a]]\n"),
        ];
        let graph = build(&notes);

        let first = layout(&graph, 400.0, 300.0, 50);
        let second = layout(&graph, 400.0, 300.0, 50);
        assert_eq!(first, second);

        for (x, y) in first {
            assert!(x.is_finite() && y.is_finite());
            assert!((0.0..=400.0).contains(&x));
            assert!((0.0..=300.0).contains(&y));
        }

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
