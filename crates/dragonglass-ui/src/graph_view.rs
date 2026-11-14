//! Graph view: paints a link graph with edges, nodes and labels, and opens a note when a
//! node is clicked.

use dragonglass_core::graph::{self, Graph};
use gpui::{
    div, fill, point, prelude::*, px, relative, size, App, Bounds, Context, Element, ElementId,
    Entity, FocusHandle, Focusable, GlobalElementId, InspectorElementId, LayoutId, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad, PathBuilder, Pixels, Point, Render,
    ScrollDelta, ScrollWheelEvent, ShapedLine, SharedString, Style, TextRun, WeakEntity, Window,
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
    /// Magnification of the fitted layout.
    zoom: f32,
    /// Screen-space offset from the fitted layout, in pixels.
    pan: (f32, f32),
    /// An in-progress drag: start position, starting pan and whether it moved.
    drag: Option<(Point<Pixels>, (f32, f32), bool)>,
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
            zoom: 1.0,
            pan: (0.0, 0.0),
            drag: None,
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

    /// The virtual-canvas point under a position local to the graph bounds.
    fn scale(&self, local: Point<Pixels>) -> Option<(f32, f32)> {
        let bounds = self.last_bounds?;
        Some(inverse_point(
            (f32::from(local.x), f32::from(local.y)),
            (f32::from(bounds.size.width), f32::from(bounds.size.height)),
            self.zoom,
            self.pan,
        ))
    }

    fn reset_view(&mut self, cx: &mut Context<Self>) {
        self.zoom = 1.0;
        self.pan = (0.0, 0.0);
        cx.notify();
    }

    fn zoom_at(&mut self, local: Point<Pixels>, delta_lines: f32, cx: &mut Context<Self>) {
        let Some(bounds) = self.last_bounds else {
            return;
        };
        let size = (f32::from(bounds.size.width), f32::from(bounds.size.height));
        let (zoom, pan) = zoom_about(
            self.zoom,
            self.pan,
            (f32::from(local.x), f32::from(local.y)),
            size,
            delta_lines,
        );
        self.zoom = zoom;
        self.pan = pan;
        cx.notify();
    }

    fn on_scroll(
        &mut self,
        event: &ScrollWheelEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(bounds) = self.last_bounds else {
            return;
        };
        let Some(local) = bounds.localize(&event.position) else {
            return;
        };
        let lines = match event.delta {
            ScrollDelta::Lines(delta) => delta.y,
            ScrollDelta::Pixels(delta) => f32::from(delta.y) / 40.0,
        };
        self.zoom_at(local, lines, cx);
    }

    fn on_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((start, pan, moved)) = self.drag else {
            return;
        };
        let dx = f32::from(event.position.x - start.x);
        let dy = f32::from(event.position.y - start.y);
        let moved = moved || dx.abs() + dy.abs() > 3.0;
        if moved {
            self.pan = (pan.0 + dx, pan.1 + dy);
            self.drag = Some((start, pan, true));
            cx.notify();
        }
    }

    fn on_mouse_up(&mut self, event: &MouseUpEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some((_, _, moved)) = self.drag.take() else {
            return;
        };
        if !moved {
            self.open_node_at(event.position, window, cx);
        }
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus_handle);
        // A press starts a possible pan. A click without motion opens the node on release.
        self.drag = Some((event.position, self.pan, false));
        cx.notify();
    }

    fn open_node_at(
        &mut self,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(bounds) = self.last_bounds else {
            return;
        };
        let Some(local) = bounds.localize(&position) else {
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
            let _ = self
                .app
                .update(cx, |app, cx| app.open_note(&relative, window, cx));
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
                    .child("drag to pan · scroll to zoom · click a node to open · Esc to close")
                    .child(
                        div()
                            .id("graph-reset")
                            .px_2()
                            .rounded_sm()
                            .cursor_pointer()
                            .hover(move |style| style.bg(theme.hover))
                            .child("reset view")
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _event, _window, cx| {
                                    cx.stop_propagation();
                                    this.reset_view(cx);
                                }),
                            ),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.0))
                    .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
                    .on_mouse_move(cx.listener(Self::on_mouse_move))
                    .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
                    .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
                    .on_scroll_wheel(cx.listener(Self::on_scroll))
                    .child(GraphCanvas { view: cx.entity() }),
            )
    }
}

/// Uniform fit scale, so the graph is never stretched by the window's aspect ratio.
fn fit_scale(size: (f32, f32)) -> f32 {
    (size.0 / CANVAS.0).min(size.1 / CANVAS.1).max(0.01)
}

/// Map a canvas point to a screen point inside a `size` viewport, with the fitted layout
/// magnified by `zoom` and shifted by `pan` (both around the viewport centre).
fn forward_point(canvas: (f32, f32), size: (f32, f32), zoom: f32, pan: (f32, f32)) -> (f32, f32) {
    let scale = fit_scale(size) * zoom;
    (
        size.0 / 2.0 + pan.0 + (canvas.0 - CANVAS.0 / 2.0) * scale,
        size.1 / 2.0 + pan.1 + (canvas.1 - CANVAS.1 / 2.0) * scale,
    )
}

/// Inverse of [`forward_point`].
fn inverse_point(screen: (f32, f32), size: (f32, f32), zoom: f32, pan: (f32, f32)) -> (f32, f32) {
    let scale = (fit_scale(size) * zoom).max(0.0001);
    (
        (screen.0 - size.0 / 2.0 - pan.0) / scale + CANVAS.0 / 2.0,
        (screen.1 - size.1 / 2.0 - pan.1) / scale + CANVAS.1 / 2.0,
    )
}

/// Zoom by a wheel delta while keeping the canvas point under `pointer` fixed.
fn zoom_about(
    zoom: f32,
    pan: (f32, f32),
    pointer: (f32, f32),
    size: (f32, f32),
    delta_lines: f32,
) -> (f32, (f32, f32)) {
    let factor = (delta_lines * 0.1).exp();
    let new_zoom = (zoom * factor).clamp(0.35, 4.0);
    let anchor = inverse_point(pointer, size, zoom, pan);
    let scale = fit_scale(size) * new_zoom;
    let new_pan = (
        pointer.0 - size.0 / 2.0 - (anchor.0 - CANVAS.0 / 2.0) * scale,
        pointer.1 - size.1 / 2.0 - (anchor.1 - CANVAS.1 / 2.0) * scale,
    );
    (new_zoom, new_pan)
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
        let (zoom, pan) = {
            let view = self.view.read(cx);
            (view.zoom, view.pan)
        };
        let _ = (nw, nh);
        let to_screen = |px_: f32, py: f32| {
            let (x, y) = forward_point((px_, py), (nw, nh), zoom, pan);
            point(bounds.left() + px(x), bounds.top() + px(y))
        };

        // Edges first, so nodes sit on top. Plain links are solid; frontmatter relations are
        // dashed and tinted, so the relation graph reads differently from the link graph.
        let mut links = PathBuilder::stroke(px(1.0));
        let mut relations = PathBuilder::stroke(px(1.0)).dash_array(&[px(4.0), px(3.0)]);
        let mut drew_link = false;
        let mut drew_relation = false;
        for edge in &graph_edges {
            let (ax, ay) = canvas_position(edge.from);
            let (bx, by) = canvas_position(edge.to);
            let a = to_screen(ax, ay);
            let b = to_screen(bx, by);

            let (path, drew) = if edge.is_relation() {
                (&mut relations, &mut drew_relation)
            } else {
                (&mut links, &mut drew_link)
            };
            if !*drew {
                path.move_to(a);
                *drew = true;
            } else {
                path.line_to(a);
            }
            path.line_to(b);
        }
        if drew_link {
            if let Ok(path) = links.build() {
                window.paint_path(path, theme.border);
            }
        }
        if drew_relation {
            if let Ok(path) = relations.build() {
                window.paint_path(path, theme.accent_text);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pan_and_zoom_round_trip() {
        let size = (900.0, 600.0);
        let canvas = (250.0, 480.0);
        let screen = forward_point(canvas, size, 1.0, (0.0, 0.0));
        let back = inverse_point(screen, size, 1.0, (0.0, 0.0));
        assert!((back.0 - canvas.0).abs() < 0.01);
        assert!((back.1 - canvas.1).abs() < 0.01);
    }

    #[test]
    fn zoom_keeps_the_point_under_the_pointer_fixed() {
        let size = (900.0, 600.0);
        let pointer = (640.0, 180.0);
        let before = inverse_point(pointer, size, 1.0, (0.0, 0.0));
        let (zoom, pan) = zoom_about(1.0, (0.0, 0.0), pointer, size, 4.0);
        assert!(zoom > 1.0);
        let after = inverse_point(pointer, size, zoom, pan);
        assert!((before.0 - after.0).abs() < 0.01);
        assert!((before.1 - after.1).abs() < 0.01);
    }

    #[test]
    fn zoom_is_clamped_and_the_fit_is_uniform() {
        let (zoom, _) = zoom_about(1.0, (0.0, 0.0), (0.0, 0.0), (900.0, 600.0), 100.0);
        assert_eq!(zoom, 4.0);
        let (zoom, _) = zoom_about(1.0, (0.0, 0.0), (0.0, 0.0), (900.0, 600.0), -100.0);
        assert_eq!(zoom, 0.35);
        // A tall window letterboxes instead of stretching the graph.
        let scale = fit_scale((400.0, 900.0));
        assert!((scale - 0.4).abs() < 0.001);
    }
}
