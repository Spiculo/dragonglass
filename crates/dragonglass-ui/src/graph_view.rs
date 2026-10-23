//! Graph view: paints a link graph with edges, nodes and labels, and opens a note when a
//! node is clicked.

use dragonglass_core::graph::{self, Graph};
use gpui::{
    div, fill, point, prelude::*, px, relative, size, App, Bounds, Context, Element, ElementId,
    Entity, FocusHandle, Focusable, GlobalElementId, InspectorElementId, LayoutId, MouseButton,
    MouseDownEvent, PaintQuad, PathBuilder, Pixels, Point, Render, ShapedLine, SharedString, Style,
    TextRun, WeakEntity, Window,
};

use crate::theme::Theme;

/// The virtual canvas the layout is computed in.
const CANVAS: (f32, f32) = (1000.0, 600.0);
/// How many hops of the active note to show.
const DEPTH: usize = 1;
const MAX_NODES: usize = 80;

pub struct GraphView {
    graph: Graph,
    positions: Vec<(f32, f32)>,
    /// The whole vault graph, before the local window and filter are applied.
    full: Graph,
    focus: Option<String>,
    filter: String,
    depth: usize,
    app: WeakEntity<crate::DragonGlassApp>,
    focus_handle: FocusHandle,
    last_bounds: Option<Bounds<Pixels>>,
}

impl GraphView {
    pub fn new(app: WeakEntity<crate::DragonGlassApp>, cx: &mut Context<Self>) -> Self {
        Self {
            graph: Graph::default(),
            positions: Vec::new(),
            full: Graph::default(),
            focus: None,
            filter: String::new(),
            depth: DEPTH,
            app,
            focus_handle: cx.focus_handle(),
            last_bounds: None,
        }
    }

    /// Replace the displayed graph and recompute the layout.
    pub fn set_graph(&mut self, graph: Graph) {
        self.positions = graph::layout(&graph, CANVAS.0, CANVAS.1, 120);
        self.graph = graph;
    }

    /// The local graph around a vault-relative note.
    pub fn show_local(&mut self, full: &Graph, relative: &str) {
        self.full = full.clone();
        self.focus = Some(relative.to_string());
        self.rebuild();
    }

    /// Restrict the graph to nodes whose path contains `filter`.
    pub fn set_filter(&mut self, filter: &str) {
        self.filter = filter.to_string();
        self.rebuild();
    }

    /// Hops of the active note to show (1–3).
    pub fn set_depth(&mut self, depth: usize) {
        self.depth = depth.clamp(1, 3);
        self.rebuild();
    }

    pub fn depth(&self) -> usize {
        self.depth
    }

    fn rebuild(&mut self) {
        let base = match self
            .focus
            .as_ref()
            .and_then(|relative| self.full.index_of(relative))
        {
            Some(id) => graph::local(&self.full, id, self.depth, MAX_NODES),
            None => self.full.clone(),
        };

        let filtered = graph::filter(&base, &self.filter);
        self.set_graph(filtered);
    }

    fn node_radius(links: usize) -> f32 {
        (5.0 + (links as f32).sqrt() * 2.0).min(16.0)
    }

    fn scale(&self, local: Point<Pixels>) -> Option<(f32, f32)> {
        let bounds = self.last_bounds?;
        let x = f32::from(local.x) / f32::from(bounds.size.width) * CANVAS.0;
        let y = f32::from(local.y) / f32::from(bounds.size.height) * CANVAS.1;
        Some((x, y))
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus_handle);

        let Some(bounds) = self.last_bounds else {
            return;
        };
        let Some(local) = bounds.localize(&event.position) else {
            return;
        };
        let Some((x, y)) = self.scale(local) else {
            return;
        };

        let mut best: Option<(usize, f32)> = None;
        for (id, (px_, py)) in self.positions.iter().enumerate() {
            let dx = px_ - x;
            let dy = py - y;
            let distance = (dx * dx + dy * dy).sqrt();
            let reach = Self::node_radius(self.graph.nodes[id].links) + 6.0;
            if distance <= reach && best.map(|(_, d)| distance < d).unwrap_or(true) {
                best = Some((id, distance));
            }
        }

        if let Some((id, _)) = best {
            let relative = self.graph.nodes[id].relative.clone();
            let _ = self.app.update(cx, |app, cx| app.open_note(&relative, cx));
            cx.notify();
        }
    }
}

impl Focusable for GraphView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for GraphView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.global::<Theme>();
        let count = self.graph.nodes.len();
        let edges = self.graph.edges.len();

        div()
            .key_context("Graph")
            .track_focus(&self.focus_handle)
            .flex()
            .flex_col()
            .size_full()
            .bg(theme.bg)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .justify_between()
                    .px_3()
                    .py_1()
                    .bg(theme.panel_alt)
                    .text_xs()
                    .text_color(theme.muted)
                    .child(format!(
                        "graph: {count} notes / {edges} links · depth {}",
                        self.depth
                    ))
                    .child("click a node to open · Esc to close · colours group by folder"),
            )
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.0))
                    .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
                    .child(GraphCanvas { view: cx.entity() }),
            )
    }
}

/// A stable colour per top-level folder, so related notes group visually.
fn folder_color(folder: &str, dark: bool) -> gpui::Hsla {
    let hash = folder.bytes().fold(7u32, |acc, byte| {
        acc.wrapping_mul(31).wrapping_add(u32::from(byte))
    });
    let hue = (hash % 360) as f32 / 360.0;
    gpui::hsla(hue, 0.55, if dark { 0.62 } else { 0.45 }, 1.0)
}

struct GraphCanvas {
    view: Entity<GraphView>,
}

impl IntoElement for GraphCanvas {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for GraphCanvas {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.0).into();
        style.size.height = relative(1.0).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _state: &mut (),
        _window: &mut Window,
        _cx: &mut App,
    ) {
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _state: &mut (),
        _prepaint: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        let theme = *cx.global::<Theme>();
        let (nw, nh) = (f32::from(bounds.size.width), f32::from(bounds.size.height));

        let (graph_edges, graph_nodes, positions) = {
            let view = self.view.read(cx);
            (
                view.graph.edges.clone(),
                view.graph.nodes.clone(),
                view.positions.clone(),
            )
        };
        let canvas_position = |id: usize| positions.get(id).copied().unwrap_or((0.0, 0.0));
        let to_screen = |px_: f32, py: f32| {
            point(
                bounds.left() + px(nw * px_ / CANVAS.0),
                bounds.top() + px(nh * py / CANVAS.1),
            )
        };

        // Edges first, so nodes sit on top.
        let mut edges = PathBuilder::stroke(px(1.0));
        let mut drew_edge = false;
        for edge in &graph_edges {
            let (ax, ay) = canvas_position(edge.from);
            let (bx, by) = canvas_position(edge.to);
            let a = to_screen(ax, ay);
            let b = to_screen(bx, by);
            if !drew_edge {
                edges.move_to(a);
                drew_edge = true;
            } else {
                edges.line_to(a);
            }
            edges.line_to(b);
        }
        if drew_edge {
            if let Ok(path) = edges.build() {
                window.paint_path(path, theme.border);
            }
        }

        // Nodes.
        let mut node_quads: Vec<PaintQuad> = Vec::new();
        for (id, node) in graph_nodes.iter().enumerate() {
            let (px_, py) = canvas_position(id);
            let center = to_screen(px_, py);
            let radius = GraphView::node_radius(node.links);
            let folder = graph::folder_of(&node.relative);
            let color = if id == 0 {
                theme.accent
            } else {
                folder_color(folder, theme.dark)
            };
            node_quads.push(fill(
                Bounds::new(
                    point(center.x - px(radius), center.y - px(radius)),
                    size(px(radius * 2.0), px(radius * 2.0)),
                ),
                color,
            ));
        }
        for quad in node_quads {
            window.paint_quad(quad);
        }

        // Labels.
        let text_style = window.text_style();
        let font = text_style.font();
        let line_height = px(12.0);
        let mut labels: Vec<ShapedLine> = Vec::new();
        let mut label_origins: Vec<Point<Pixels>> = Vec::new();
        for (id, node) in graph_nodes.iter().take(60).enumerate() {
            let (px_, py) = canvas_position(id);
            let center = to_screen(px_, py);
            let radius = GraphView::node_radius(node.links);
            let name = node
                .relative
                .rsplit('/')
                .next()
                .unwrap_or(&node.relative)
                .trim_end_matches(".md")
                .to_string();

            let run = TextRun {
                len: name.len(),
                font: font.clone(),
                color: theme.text_dim,
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            let shaped =
                window
                    .text_system()
                    .shape_line(SharedString::from(name), px(11.0), &[run], None);
            labels.push(shaped);
            label_origins.push(point(
                center.x + px(radius + 3.0),
                center.y - line_height / 2.0,
            ));
        }
        for (line, origin) in labels.iter().zip(label_origins) {
            let _ = line.paint(origin, line_height, window, cx);
        }

        self.view.update(cx, |view, _cx| {
            view.last_bounds = Some(bounds);
        });
    }
}
