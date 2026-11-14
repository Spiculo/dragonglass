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

/// Radius of a node in canvas units. A note with more links gets a bigger circle.
pub fn node_radius(links: usize) -> f32 {
    (5.0 + (links as f32).sqrt() * 2.0).min(16.0)
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

/// Tuning for the force-directed graph layout.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayoutParams {
    /// Width of the virtual canvas.
    pub width: f32,
    /// Height of the virtual canvas.
    pub height: f32,
    /// Empty space kept between a node edge and the canvas edge.
    pub padding: f32,
    /// Number of simulation steps used by [`layout`] and [`GraphLayout::settle`].
    pub iterations: usize,
    /// Strength of the pairwise repulsion.
    pub repel: f32,
    /// Strength of the spring along an edge.
    pub link_force: f32,
    /// Target length of an edge. Zero asks the layout to derive it from the graph size.
    pub link_distance: f32,
    /// Pull toward the canvas centre. This keeps disconnected parts together.
    pub center_force: f32,
    /// Extra space between two node circles.
    pub collision_gap: f32,
}

impl Default for LayoutParams {
    fn default() -> Self {
        Self {
            width: 1000.0,
            height: 600.0,
            padding: 8.0,
            iterations: 120,
            repel: 1.0,
            link_force: 0.06,
            link_distance: 0.0,
            center_force: 0.015,
            collision_gap: 6.0,
        }
    }
}

impl LayoutParams {
    pub fn new(width: f32, height: f32, iterations: usize) -> Self {
        Self {
            width,
            height,
            iterations,
            ..Self::default()
        }
    }

    /// The natural distance between two linked nodes.
    pub fn k(&self, n: usize) -> f32 {
        let area = (self.width.max(1.0) * self.height.max(1.0)) as f32;
        (area / n.max(1) as f32).sqrt().clamp(60.0, 160.0)
    }

    fn resolved_link_distance(&self, n: usize) -> f32 {
        if self.link_distance > 0.0 {
            self.link_distance
        } else {
            self.k(n) * 0.8
        }
    }
}

/// A deterministic, incremental force-directed layout.
///
/// The UI can call [`step`](Self::step) once per frame to animate the graph. Headless
/// callers use [`layout`] instead, which settles the same simulation synchronously.
#[derive(Debug, Clone)]
pub struct GraphLayout {
    positions: Vec<(f32, f32)>,
    pinned: Vec<bool>,
    alpha: f32,
    decay: f32,
    params: LayoutParams,
    steps: usize,
}

impl GraphLayout {
    /// Start a layout from a deterministic ring. Call [`step`](Self::step) to animate it.
    pub fn new(graph: &Graph, params: LayoutParams) -> Self {
        let n = graph.nodes.len();
        let positions = initial_positions(graph, &params);
        let decay = 0.02f32.powf(1.0 / params.iterations.max(1) as f32);
        Self {
            positions,
            pinned: vec![false; n],
            alpha: 1.0,
            decay,
            params,
            steps: 0,
        }
    }

    pub fn positions(&self) -> &[(f32, f32)] {
        &self.positions
    }

    pub fn params(&self) -> LayoutParams {
        self.params
    }

    /// True while the simulation still wants frames.
    pub fn is_active(&self) -> bool {
        self.alpha > 0.02
    }

    /// Raise the temperature again. Dragging a node or rebuilding the graph uses this.
    pub fn reheat(&mut self, alpha: f32) {
        self.alpha = self.alpha.max(alpha.clamp(0.0, 1.0));
    }

    /// Pin a node so forces cannot move it. The UI pins a node after the user drags it.
    pub fn pin(&mut self, node: usize, pinned: bool) {
        if let Some(slot) = self.pinned.get_mut(node) {
            *slot = pinned;
        }
    }

    /// Move a node directly. Used by the pointer drag.
    pub fn set_position(&mut self, node: usize, position: (f32, f32)) {
        if let Some(slot) = self.positions.get_mut(node) {
            *slot = position;
        }
    }

    /// Advance the simulation by one frame. Returns true while another frame is useful.
    pub fn step(&mut self, graph: &Graph, _dt: f32) -> bool {
        let n = self.positions.len().min(graph.nodes.len());
        if n == 0 || !self.is_active() {
            return false;
        }

        let k = self.params.k(n);
        let link_distance = self.params.resolved_link_distance(n);
        let center = (self.params.width / 2.0, self.params.height / 2.0);
        let mut disp = vec![(0.0f32, 0.0f32); n];

        // Repulsion between every pair. The factor keeps the force in the same
        // range as the springs, so a single alpha controls the whole step.
        for i in 0..n {
            for j in (i + 1)..n {
                let dx = self.positions[i].0 - self.positions[j].0;
                let dy = self.positions[i].1 - self.positions[j].1;
                let distance = (dx * dx + dy * dy).sqrt().max(0.5);
                let force = self.params.repel * k * k / distance * 0.025;
                let (ux, uy) = (dx / distance, dy / distance);
                disp[i].0 += ux * force;
                disp[i].1 += uy * force;
                disp[j].0 -= ux * force;
                disp[j].1 -= uy * force;
            }
        }

        // Springs along edges. They pull toward `link_distance` and push apart
        // when an edge is too short, which keeps clusters from collapsing.
        for edge in &graph.edges {
            if edge.from >= n || edge.to >= n {
                continue;
            }
            let dx = self.positions[edge.from].0 - self.positions[edge.to].0;
            let dy = self.positions[edge.from].1 - self.positions[edge.to].1;
            let distance = (dx * dx + dy * dy).sqrt().max(0.5);
            let force = (distance - link_distance) * self.params.link_force;
            let (ux, uy) = (dx / distance, dy / distance);
            disp[edge.from].0 -= ux * force;
            disp[edge.from].1 -= uy * force;
            disp[edge.to].0 += ux * force;
            disp[edge.to].1 += uy * force;
        }

        // A weak centre pull and soft walls keep disconnected notes on screen.
        for (i, (x, y)) in self.positions.iter().enumerate() {
            let radius = node_radius(graph.nodes[i].links) + self.params.padding;
            disp[i].0 += (center.0 - x) * self.params.center_force;
            disp[i].1 += (center.1 - y) * self.params.center_force;
            if *x < radius {
                disp[i].0 += (radius - x) * 0.2;
            }
            if *x > self.params.width - radius {
                disp[i].0 -= (x - (self.params.width - radius)) * 0.2;
            }
            if *y < radius {
                disp[i].1 += (radius - y) * 0.2;
            }
            if *y > self.params.height - radius {
                disp[i].1 -= (y - (self.params.height - radius)) * 0.2;
            }
        }

        // The step shrinks with alpha, so the graph slows down instead of
        // oscillating forever.
        let max_step = (14.0 * self.alpha).max(0.4);
        for i in 0..n {
            if self.pinned[i] {
                continue;
            }
            let (dx, dy) = disp[i];
            let length = (dx * dx + dy * dy).sqrt();
            if length > 0.001 {
                let step = length.min(max_step);
                self.positions[i].0 += dx / length * step;
                self.positions[i].1 += dy / length * step;
            }
        }

        self.resolve_collisions(graph, 2);
        self.clamp_to_bounds(graph);
        self.alpha *= self.decay;
        self.steps += 1;
        self.is_active()
    }

    /// Run the simulation to rest. Used by the headless [`layout`] entry point.
    pub fn settle(&mut self, graph: &Graph) {
        for _ in 0..self.params.iterations.max(1) {
            if !self.step(graph, 1.0 / 60.0) {
                break;
            }
        }
        // Forces can leave a tiny residual overlap at an edge. A few direct
        // separation passes make the final positions usable for hit-testing.
        for _ in 0..6 {
            self.resolve_collisions(graph, 2);
            self.clamp_to_bounds(graph);
        }
        self.alpha = 0.0;
    }

    fn resolve_collisions(&mut self, graph: &Graph, passes: usize) {
        let n = self.positions.len().min(graph.nodes.len());
        const EPSILON: f32 = 0.001;

        for _ in 0..passes {
            for i in 0..n {
                for j in (i + 1)..n {
                    let min_distance = node_radius(graph.nodes[i].links)
                        + node_radius(graph.nodes[j].links)
                        + self.params.collision_gap;
                    let dx = self.positions[i].0 - self.positions[j].0;
                    let dy = self.positions[i].1 - self.positions[j].1;
                    let distance = (dx * dx + dy * dy).sqrt();
                    if distance >= min_distance {
                        continue;
                    }

                    let (ux, uy) = if distance > EPSILON {
                        (dx / distance, dy / distance)
                    } else {
                        // Two nodes exactly on top of each other: pick a stable
                        // direction from their indices.
                        let angle = std::f32::consts::TAU * ((i * 37 + j * 17) % n.max(1)) as f32
                            / n.max(1) as f32;
                        (angle.cos(), angle.sin())
                    };
                    let overlap = min_distance - distance;
                    let (wi, wj) = match (self.pinned[i], self.pinned[j]) {
                        (false, false) => (0.5, 0.5),
                        (true, false) => (0.0, 1.0),
                        (false, true) => (1.0, 0.0),
                        (true, true) => (0.0, 0.0),
                    };
                    self.positions[i].0 += ux * overlap * wi;
                    self.positions[i].1 += uy * overlap * wi;
                    self.positions[j].0 -= ux * overlap * wj;
                    self.positions[j].1 -= uy * overlap * wj;
                }
            }
            self.clamp_to_bounds(graph);
        }
    }

    fn clamp_to_bounds(&mut self, graph: &Graph) {
        let n = self.positions.len().min(graph.nodes.len());
        for i in 0..n {
            let radius = node_radius(graph.nodes[i].links) + self.params.padding;
            let (x, y) = self.positions[i];
            self.positions[i].0 = clamp_axis(x, radius, self.params.width);
            self.positions[i].1 = clamp_axis(y, radius, self.params.height);
        }
    }
}

fn clamp_axis(value: f32, margin: f32, length: f32) -> f32 {
    let low = margin;
    let high = length - margin;
    if low <= high {
        value.clamp(low, high)
    } else {
        length / 2.0
    }
}

fn initial_positions(graph: &Graph, params: &LayoutParams) -> Vec<(f32, f32)> {
    let n = graph.nodes.len();
    if n == 0 {
        return Vec::new();
    }

    let center = (params.width / 2.0, params.height / 2.0);
    if n == 1 {
        return vec![center];
    }

    let max_radius = graph
        .nodes
        .iter()
        .map(|node| node_radius(node.links))
        .fold(0.0f32, f32::max);
    let ring =
        ((params.width.min(params.height) / 2.0) - max_radius - params.padding).max(10.0) * 0.75;

    (0..n)
        .map(|i| {
            let angle = std::f32::consts::TAU * (i as f32) / (n as f32);
            // A small deterministic jitter stops symmetric nodes from stacking.
            let jitter = 1.0 + ((i * 2654435761usize % 1000) as f32 / 1000.0 - 0.5) * 0.1;
            (
                center.0 + ring * angle.cos() * jitter,
                center.1 + ring * angle.sin() * jitter,
            )
        })
        .collect()
}

/// Deterministic force-directed layout inside `width` x `height`.
///
/// Use [`GraphLayout`] directly when the caller wants to animate the simulation.
pub fn layout(graph: &Graph, width: f32, height: f32, iterations: usize) -> Vec<(f32, f32)> {
    let mut layout = GraphLayout::new(graph, LayoutParams::new(width, height, iterations));
    layout.settle(graph);
    layout.positions
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

    fn dense_graph(n: usize) -> Graph {
        let nodes = (0..n)
            .map(|id| Node {
                id,
                relative: format!("note-{id}.md"),
                links: id % 9,
            })
            .collect();
        let mut edges = Vec::new();
        for i in 0..n {
            edges.push(Edge {
                from: i,
                to: (i + 1) % n,
                kind: EdgeKind::Link,
            });
            if i % 3 == 0 {
                edges.push(Edge {
                    from: i,
                    to: (i + 7) % n,
                    kind: EdgeKind::Embed,
                });
            }
        }
        Graph { nodes, edges }
    }

    #[test]
    fn layout_does_not_overlap_node_circles() {
        let graph = dense_graph(80);
        let params = LayoutParams::new(1000.0, 600.0, 240);
        let mut layout = GraphLayout::new(&graph, params);
        layout.settle(&graph);

        let positions = layout.positions();
        assert_eq!(positions.len(), graph.nodes.len());
        for i in 0..positions.len() {
            for j in (i + 1)..positions.len() {
                let dx = positions[i].0 - positions[j].0;
                let dy = positions[i].1 - positions[j].1;
                let distance = (dx * dx + dy * dy).sqrt();
                let min = node_radius(graph.nodes[i].links)
                    + node_radius(graph.nodes[j].links)
                    + params.collision_gap;
                assert!(
                    distance + 0.01 >= min,
                    "nodes {i} and {j} overlap: distance {distance}, minimum {min}"
                );
            }
        }
    }

    #[test]
    fn layout_keeps_every_circle_inside_the_canvas() {
        let graph = dense_graph(80);
        let params = LayoutParams::new(1000.0, 600.0, 240);
        let mut layout = GraphLayout::new(&graph, params);
        layout.settle(&graph);

        for (id, (x, y)) in layout.positions().iter().enumerate() {
            let radius = node_radius(graph.nodes[id].links) + params.padding;
            assert!((*x >= radius - 0.01 && *x <= params.width - radius + 0.01));
            assert!((*y >= radius - 0.01 && *y <= params.height - radius + 0.01));
        }
    }

    #[test]
    fn layout_stops_after_settling() {
        let graph = dense_graph(20);
        let mut layout = GraphLayout::new(&graph, LayoutParams::new(800.0, 500.0, 60));
        let mut steps = 0;
        while layout.step(&graph, 1.0 / 60.0) {
            steps += 1;
            assert!(steps < 500, "layout did not settle");
        }
        assert!(!layout.is_active());
        assert!(!layout.step(&graph, 1.0 / 60.0));
    }

    #[test]
    fn a_pinned_node_stays_where_the_pointer_left_it() {
        let graph = dense_graph(20);
        let mut layout = GraphLayout::new(&graph, LayoutParams::new(800.0, 500.0, 120));
        layout.pin(0, true);
        layout.set_position(0, (400.0, 250.0));
        layout.reheat(1.0);
        for _ in 0..12 {
            layout.step(&graph, 1.0 / 60.0);
        }
        assert_eq!(layout.positions()[0], (400.0, 250.0));
    }
}
