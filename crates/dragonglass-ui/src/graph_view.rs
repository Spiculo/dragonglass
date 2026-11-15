//! Graph view: paints a link graph with edges, nodes and labels, and opens a note when a
//! node is clicked.
//!
//! The layout is a small live force simulation. Nodes repel each other, edges pull, and a
//! collision pass keeps the circles apart. The simulation stops when it settles, so an idle
//! graph does not keep repainting.

use std::collections::{HashMap, HashSet};

use dragonglass_core::graph::{self, Graph, GraphLayout, LayoutParams};
use gpui::{
    div, fill, outline, point, prelude::*, px, relative, size, AnyElement, App, Bounds,
    ContentMask, Context, Corners, Element, ElementId, Entity, FocusHandle, Focusable,
    GlobalElementId, Hsla, InspectorElementId, LayoutId, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, PathBuilder, Pixels, Point, Render, ScrollDelta,
    ScrollWheelEvent, ShapedLine, SharedString, Style, TextRun, WeakEntity, Window,
};

use crate::theme::Theme;

/// The virtual canvas the layout is computed in.
const CANVAS: (f32, f32) = (1000.0, 600.0);
/// How many hops of the active note to show.
const DEPTH: usize = 1;
const MAX_NODES: usize = 80;
/// Simulation steps. The UI spends these over frames; the headless wrapper spends them at once.
const LAYOUT_STEPS: usize = 120;
const LABEL_SIZE: f32 = 11.0;
const LABEL_LINE_HEIGHT: f32 = 12.0;
const LABEL_GAP: f32 = 4.0;
const LABEL_ZOOM: f32 = 0.7;
const LABEL_FADE: f32 = 0.35;
const LABEL_ALPHA_STEPS: u8 = 8;
const NODE_HIT_PADDING: f32 = 3.0;
/// Keep at least this much of the graph on screen when the user pans.
const PAN_MARGIN: f32 = 48.0;
/// Extra space around the node circles when measuring the graph bounds.
const GRAPH_PADDING: f32 = 8.0;

pub struct GraphView {
    graph: Graph,
    layout: GraphLayout,
    params: LayoutParams,
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
    /// An in-progress canvas pan: start position, starting pan and whether it moved.
    drag: Option<PanDrag>,
    /// An in-progress node drag.
    node_drag: Option<NodeDrag>,
    hovered: Option<usize>,
    frame_scheduled: bool,
    /// Last positions by vault-relative path, so filters do not make the graph jump.
    positions: HashMap<String, (f32, f32)>,
    /// Shaped labels, keyed by name and quantized alpha.
    label_cache: HashMap<(String, u8), ShapedLine>,
    label_color: Option<Hsla>,
    show_forces: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ForceField {
    Center,
    Repel,
    Link,
    Distance,
}

impl ForceField {
    const ALL: [ForceField; 4] = [
        ForceField::Center,
        ForceField::Repel,
        ForceField::Link,
        ForceField::Distance,
    ];

    fn key(self) -> &'static str {
        match self {
            ForceField::Center => "center",
            ForceField::Repel => "repel",
            ForceField::Link => "link",
            ForceField::Distance => "distance",
        }
    }

    fn label(self) -> &'static str {
        match self {
            ForceField::Center => "centre",
            ForceField::Repel => "repel",
            ForceField::Link => "link",
            ForceField::Distance => "distance",
        }
    }

    fn value(self, params: &LayoutParams) -> f32 {
        match self {
            ForceField::Center => params.center_force,
            ForceField::Repel => params.repel,
            ForceField::Link => params.link_force,
            ForceField::Distance => params.link_distance,
        }
    }

    fn set(self, params: &mut LayoutParams, value: f32) {
        let (min, max, _) = self.range();
        let value = value.clamp(min, max);
        match self {
            ForceField::Center => params.center_force = value,
            ForceField::Repel => params.repel = value,
            ForceField::Link => params.link_force = value,
            ForceField::Distance => params.link_distance = value,
        }
    }

    fn range(self) -> (f32, f32, f32) {
        match self {
            ForceField::Center => (0.0, 0.10, 0.005),
            ForceField::Repel => (0.0, 0.10, 0.005),
            ForceField::Link => (0.0, 0.20, 0.01),
            ForceField::Distance => (0.0, 200.0, 10.0),
        }
    }

    fn display(self, params: &LayoutParams) -> String {
        let value = self.value(params);
        match self {
            ForceField::Distance if value <= 0.0 => "auto".to_string(),
            ForceField::Distance => format!("{value:.0}"),
            _ => format!("{value:.3}"),
        }
    }
}

#[derive(Clone, Copy)]
struct PanDrag {
    start: Point<Pixels>,
    pan: (f32, f32),
    moved: bool,
}

#[derive(Clone, Copy)]
struct NodeDrag {
    node: usize,
    start: Point<Pixels>,
    offset: (f32, f32),
    moved: bool,
}

impl GraphView {
    pub fn new(app: WeakEntity<crate::DragonGlassApp>, cx: &mut Context<Self>) -> Self {
        let empty = Graph::default();
        let params = LayoutParams::new(CANVAS.0, CANVAS.1, LAYOUT_STEPS);
        let layout = GraphLayout::new(&empty, params);
        Self {
            graph: empty,
            layout,
            params,
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
            node_drag: None,
            hovered: None,
            frame_scheduled: false,
            positions: HashMap::new(),
            label_cache: HashMap::new(),
            label_color: None,
            show_forces: false,
        }
    }

    /// Replace the displayed graph and start a new layout from the last positions.
    pub fn set_graph(&mut self, graph: Graph) {
        let previous = self.snapshot_positions();
        self.graph = graph;
        self.params.width = CANVAS.0;
        self.params.height = CANVAS.1;
        self.params.iterations = LAYOUT_STEPS;

        let mut layout = GraphLayout::new(&self.graph, self.params);
        let mut seeded = 0;
        for (id, node) in self.graph.nodes.iter().enumerate() {
            if let Some(position) = previous.get(&node.relative) {
                layout.set_position(id, *position);
                seeded += 1;
            }
        }

        self.layout = layout;
        self.positions = previous;
        self.hovered = None;
        self.drag = None;
        self.node_drag = None;
        self.label_cache.clear();
        if self.graph.nodes.len() > 1 {
            // A graph made only of known nodes needs a gentle nudge; a graph with
            // new nodes starts from the ring and gets the full simulation.
            self.layout.reheat(if seeded == self.graph.nodes.len() {
                0.35
            } else {
                1.0
            });
        }
    }

    /// Show the whole graph. Filters and depth rebuild from `full`.
    pub fn show_global(&mut self, full: &Graph) {
        self.full = full.clone();
        self.focus = None;
        self.rebuild();
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

    fn adjust_force(&mut self, field: ForceField, direction: f32, cx: &mut Context<Self>) {
        let (min, max, step) = field.range();
        let current = field.value(&self.params);
        let next = if field == ForceField::Distance && current <= 0.0 {
            if direction > 0.0 {
                80.0
            } else {
                0.0
            }
        } else {
            current + direction * step
        };
        field.set(&mut self.params, next.clamp(min, max));
        self.apply_forces(cx);
    }

    fn reset_forces(&mut self, cx: &mut Context<Self>) {
        self.params = LayoutParams::new(CANVAS.0, CANVAS.1, LAYOUT_STEPS);
        self.apply_forces(cx);
    }

    /// Rebuild the simulation with the current force settings, keeping node positions.
    fn apply_forces(&mut self, cx: &mut Context<Self>) {
        let current = self.snapshot_positions();
        self.params.width = CANVAS.0;
        self.params.height = CANVAS.1;
        self.params.iterations = LAYOUT_STEPS;

        let mut layout = GraphLayout::new(&self.graph, self.params);
        for (id, node) in self.graph.nodes.iter().enumerate() {
            if let Some(position) = current.get(&node.relative) {
                layout.set_position(id, *position);
            }
        }
        self.layout = layout;
        self.label_cache.clear();
        if self.graph.nodes.len() > 1 {
            self.layout.reheat(0.8);
        }
        cx.notify();
    }

    fn force_control(&self, field: ForceField, theme: Theme, cx: &mut Context<Self>) -> AnyElement {
        let value = field.display(&self.params);
        let button = |label: &'static str, id: String, direction: f32| {
            div()
                .id(ElementId::Name(id.into()))
                .w(px(18.0))
                .h(px(18.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded_sm()
                .cursor_pointer()
                .bg(theme.chip)
                .text_color(theme.text)
                .hover(move |style| style.bg(theme.hover))
                .child(label)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _event, _window, cx| {
                        cx.stop_propagation();
                        this.adjust_force(field, direction, cx);
                    }),
                )
                .into_any_element()
        };

        div()
            .flex()
            .flex_row()
            .items_center()
            .gap_1()
            .child(field.label())
            .child(button("−", format!("force-{}-minus", field.key()), -1.0))
            .child(
                div()
                    .min_w(px(42.0))
                    .text_center()
                    .text_color(theme.text)
                    .child(value),
            )
            .child(button("+", format!("force-{}-plus", field.key()), 1.0))
            .into_any_element()
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

    fn snapshot_positions(&self) -> HashMap<String, (f32, f32)> {
        let mut positions = self.positions.clone();
        for (id, node) in self.graph.nodes.iter().enumerate() {
            if let Some(position) = self.layout.positions().get(id) {
                positions.insert(node.relative.clone(), *position);
            }
        }
        positions
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

    fn canvas_point(&self, position: Point<Pixels>) -> Option<(f32, f32)> {
        let bounds = self.last_bounds?;
        let local = bounds.localize(&position)?;
        self.scale(local)
    }

    /// The node under a window position, if any.
    fn node_at(&self, position: Point<Pixels>) -> Option<usize> {
        let bounds = self.last_bounds?;
        let local = bounds.localize(&position)?;
        let size = (f32::from(bounds.size.width), f32::from(bounds.size.height));
        let scale = fit_scale(size) * self.zoom;
        let pointer = (f32::from(local.x), f32::from(local.y));

        let mut best: Option<(usize, f32)> = None;
        for (id, (x, y)) in self.layout.positions().iter().enumerate() {
            let Some(node) = self.graph.nodes.get(id) else {
                continue;
            };
            let (sx, sy) = forward_point((*x, *y), size, self.zoom, self.pan);
            let dx = sx - pointer.0;
            let dy = sy - pointer.1;
            let distance = (dx * dx + dy * dy).sqrt();
            let reach = graph::node_radius(node.links) * scale + NODE_HIT_PADDING;
            if distance <= reach && best.map(|(_, d)| distance < d).unwrap_or(true) {
                best = Some((id, distance));
            }
        }
        best.map(|(id, _)| id)
    }

    fn set_hover(&mut self, hovered: Option<usize>, cx: &mut Context<Self>) {
        if self.hovered != hovered {
            self.hovered = hovered;
            cx.notify();
        }
    }

    pub fn reset_view(&mut self, cx: &mut Context<Self>) {
        self.zoom = 1.0;
        self.pan = (0.0, 0.0);
        self.clamp_pan();
        cx.notify();
    }

    fn clamp_pan(&mut self) {
        let Some(view_bounds) = self.last_bounds else {
            return;
        };
        let Some(graph_bounds) = graph_bounds(&self.graph, self.layout.positions(), GRAPH_PADDING)
        else {
            return;
        };
        let size = (
            f32::from(view_bounds.size.width),
            f32::from(view_bounds.size.height),
        );
        self.pan = clamp_pan_to_bounds(self.pan, size, self.zoom, graph_bounds);
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
        self.clamp_pan();
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
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(drag) = self.node_drag {
            let dx = f32::from(event.position.x - drag.start.x);
            let dy = f32::from(event.position.y - drag.start.y);
            let moved = drag.moved || dx.abs() + dy.abs() > 3.0;
            if moved {
                if let Some((x, y)) = self.canvas_point(event.position) {
                    self.layout
                        .set_position(drag.node, (x + drag.offset.0, y + drag.offset.1));
                    self.layout.reheat(0.85);
                }
                self.node_drag = Some(NodeDrag {
                    moved: true,
                    ..drag
                });
                self.schedule_frame(window, cx);
                cx.notify();
            }
            return;
        }

        if let Some(PanDrag { start, pan, moved }) = self.drag {
            let dx = f32::from(event.position.x - start.x);
            let dy = f32::from(event.position.y - start.y);
            let moved = moved || dx.abs() + dy.abs() > 3.0;
            if moved {
                self.pan = (pan.0 + dx, pan.1 + dy);
                self.clamp_pan();
                self.drag = Some(PanDrag {
                    start,
                    pan,
                    moved: true,
                });
                cx.notify();
            }
            return;
        }

        let hovered = self.node_at(event.position);
        self.set_hover(hovered, cx);
    }

    fn on_mouse_up(&mut self, event: &MouseUpEvent, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(drag) = self.node_drag.take() {
            if drag.moved {
                // Keep the node where the user put it until the graph is rebuilt.
                self.layout.pin(drag.node, true);
                self.layout.reheat(0.6);
            } else {
                // A click that did not move the node should not leave it pinned.
                self.layout.pin(drag.node, false);
                self.open_node_at(event.position, window, cx);
            }
            self.schedule_frame(window, cx);
            cx.notify();
            return;
        }

        let Some(PanDrag { moved, .. }) = self.drag.take() else {
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

        if let Some(node) = self.node_at(event.position) {
            let offset = self
                .canvas_point(event.position)
                .and_then(|pointer| {
                    self.layout
                        .positions()
                        .get(node)
                        .map(|position| (position.0 - pointer.0, position.1 - pointer.1))
                })
                .unwrap_or((0.0, 0.0));
            self.node_drag = Some(NodeDrag {
                node,
                start: event.position,
                offset,
                moved: false,
            });
            self.drag = None;
            self.layout.pin(node, true);
            self.set_hover(Some(node), cx);
        } else {
            // A press on empty space starts a possible pan. A click without motion
            // opens the node on release.
            self.drag = Some(PanDrag {
                start: event.position,
                pan: self.pan,
                moved: false,
            });
            self.node_drag = None;
        }
        cx.notify();
    }

    fn open_node_at(
        &mut self,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = self.node_at(position) else {
            return;
        };
        let relative = self.graph.nodes[id].relative.clone();
        let _ = self
            .app
            .update(cx, |app, cx| app.open_note(&relative, window, cx));
        cx.notify();
    }

    fn is_visible(&self, cx: &App) -> bool {
        self.app
            .upgrade()
            .is_some_and(|app| app.read(cx).show_graph)
    }

    fn schedule_frame(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.frame_scheduled || !self.layout.is_active() {
            return;
        }
        self.frame_scheduled = true;
        cx.on_next_frame(window, |this, window, cx| {
            this.frame_scheduled = false;
            if !this.is_visible(cx) {
                return;
            }
            let active = this.layout.step(&this.graph, 1.0 / 60.0);
            if active {
                this.schedule_frame(window, cx);
            }
            cx.notify();
        });
    }
}

/// Bounds of the drawn graph in canvas units, including node radius and padding.
fn graph_bounds(
    graph: &Graph,
    positions: &[(f32, f32)],
    padding: f32,
) -> Option<(f32, f32, f32, f32)> {
    let mut min_x = f32::MAX;
    let mut min_y = f32::MAX;
    let mut max_x = f32::MIN;
    let mut max_y = f32::MIN;

    for (id, node) in graph.nodes.iter().enumerate() {
        let Some((x, y)) = positions.get(id) else {
            continue;
        };
        let radius = graph::node_radius(node.links) + padding;
        min_x = min_x.min(x - radius);
        min_y = min_y.min(y - radius);
        max_x = max_x.max(x + radius);
        max_y = max_y.max(y + radius);
    }

    (min_x != f32::MAX).then_some((min_x, min_y, max_x, max_y))
}

/// Keep at least `PAN_MARGIN` of the graph visible after a pan or zoom.
fn clamp_pan_to_bounds(
    pan: (f32, f32),
    size: (f32, f32),
    zoom: f32,
    graph: (f32, f32, f32, f32),
) -> (f32, f32) {
    let scale = fit_scale(size) * zoom;
    let (min_x, min_y, max_x, max_y) = graph;
    let margin_x = PAN_MARGIN.min(size.0 / 3.0);
    let margin_y = PAN_MARGIN.min(size.1 / 3.0);

    let left = size.0 / 2.0 + pan.0 + (min_x - CANVAS.0 / 2.0) * scale;
    let right = size.0 / 2.0 + pan.0 + (max_x - CANVAS.0 / 2.0) * scale;
    let top = size.1 / 2.0 + pan.1 + (min_y - CANVAS.1 / 2.0) * scale;
    let bottom = size.1 / 2.0 + pan.1 + (max_y - CANVAS.1 / 2.0) * scale;

    let mut pan = pan;
    if right < margin_x {
        pan.0 += margin_x - right;
    } else if left > size.0 - margin_x {
        pan.0 -= left - (size.0 - margin_x);
    }
    if bottom < margin_y {
        pan.1 += margin_y - bottom;
    } else if top > size.1 - margin_y {
        pan.1 -= top - (size.1 - margin_y);
    }
    pan
}

impl Focusable for GraphView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for GraphView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.global::<Theme>();
        let count = self.graph.nodes.len();
        let edges = self.graph.edges.len();

        self.schedule_frame(window, cx);

        let force_controls: Vec<_> = ForceField::ALL
            .into_iter()
            .map(|field| self.force_control(field, theme, cx))
            .collect();
        let force_panel = self.show_forces.then(|| {
            div()
                .flex()
                .flex_row()
                .flex_wrap()
                .items_center()
                .gap_3()
                .px_3()
                .py_1()
                .bg(theme.input_bg)
                .border_b_1()
                .border_color(theme.border)
                .text_xs()
                .text_color(theme.muted)
                .children(force_controls)
                .child(
                    div()
                        .id("forces-reset")
                        .px_2()
                        .rounded_sm()
                        .cursor_pointer()
                        .hover(move |style| style.bg(theme.hover))
                        .child("reset forces")
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _event, _window, cx| {
                                cx.stop_propagation();
                                this.reset_forces(cx);
                            }),
                        ),
                )
                .into_any_element()
        });

        div()
            .key_context("Graph")
            .track_focus(&self.focus_handle)
            .flex()
            .flex_col()
            .size_full()
            .overflow_hidden()
            .bg(theme.bg)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_3()
                    .px_3()
                    .py_1()
                    .bg(theme.panel_alt)
                    .text_xs()
                    .text_color(theme.muted)
                    .child(
                        div()
                            .flex_shrink_0()
                            .child(format!(
                                "graph: {count} notes / {edges} links · depth {}",
                                self.depth
                            )),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .truncate()
                            .child("drag nodes or canvas · scroll to zoom · click a node to open · Esc to close"),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .id("graph-forces")
                                    .px_2()
                                    .rounded_sm()
                                    .cursor_pointer()
                                    .hover(move |style| style.bg(theme.hover))
                                    .child(if self.show_forces { "forces ✓" } else { "forces" })
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(|this, _event, _window, cx| {
                                            cx.stop_propagation();
                                            this.show_forces = !this.show_forces;
                                            cx.notify();
                                        }),
                                    ),
                            )
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
                    ),
            )
            .children(force_panel)
            .child(
                div()
                    .id("graph-canvas")
                    .flex_1()
                    .min_h(px(0.0))
                    .overflow_hidden()
                    .on_hover(cx.listener(|this, hovered: &bool, _window, cx| {
                        if !*hovered {
                            this.set_hover(None, cx);
                        }
                    }))
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

fn display_name(relative: &str) -> String {
    relative
        .rsplit('/')
        .next()
        .unwrap_or(relative)
        .trim_end_matches(".md")
        .to_string()
}

#[derive(Clone, Debug)]
struct LabelCandidate {
    node: usize,
    width: f32,
    height: f32,
    center: (f32, f32),
    radius: f32,
    priority: usize,
}

#[derive(Clone, Copy, Debug)]
struct LabelPlacement {
    node: usize,
    x: f32,
    y: f32,
}

fn place_labels(candidates: &[LabelCandidate], viewport: (f32, f32)) -> Vec<LabelPlacement> {
    let mut order: Vec<usize> = (0..candidates.len()).collect();
    order.sort_by(|a, b| {
        let a = &candidates[*a];
        let b = &candidates[*b];
        b.priority
            .cmp(&a.priority)
            .then_with(|| {
                let da = distance_to_center(a, viewport);
                let db = distance_to_center(b, viewport);
                da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| a.node.cmp(&b.node))
    });

    let mut occupied: Vec<(f32, f32, f32, f32)> = candidates
        .iter()
        .map(|candidate| {
            (
                candidate.center.0 - candidate.radius,
                candidate.center.1 - candidate.radius,
                candidate.radius * 2.0,
                candidate.radius * 2.0,
            )
        })
        .collect();
    let mut placed = Vec::new();

    for index in order {
        let candidate = &candidates[index];
        let offsets = [
            (
                candidate.center.0 + candidate.radius + LABEL_GAP,
                candidate.center.1 - candidate.height / 2.0,
            ),
            (
                candidate.center.0 - candidate.radius - LABEL_GAP - candidate.width,
                candidate.center.1 - candidate.height / 2.0,
            ),
            (
                candidate.center.0 - candidate.width / 2.0,
                candidate.center.1 + candidate.radius + LABEL_GAP,
            ),
            (
                candidate.center.0 - candidate.width / 2.0,
                candidate.center.1 - candidate.radius - LABEL_GAP - candidate.height,
            ),
        ];

        for (x, y) in offsets {
            if x < 0.0
                || y < 0.0
                || x + candidate.width > viewport.0
                || y + candidate.height > viewport.1
            {
                continue;
            }
            let rect = (x, y, candidate.width, candidate.height);
            if occupied
                .iter()
                .any(|occupied| rects_intersect(rect, *occupied))
            {
                continue;
            }
            occupied.push(rect);
            placed.push(LabelPlacement {
                node: candidate.node,
                x,
                y,
            });
            break;
        }
    }

    placed
}

fn distance_to_center(candidate: &LabelCandidate, viewport: (f32, f32)) -> f32 {
    let dx = candidate.center.0 - viewport.0 / 2.0;
    let dy = candidate.center.1 - viewport.1 / 2.0;
    dx * dx + dy * dy
}

fn rects_intersect(a: (f32, f32, f32, f32), b: (f32, f32, f32, f32)) -> bool {
    const PAD: f32 = 2.0;
    a.0 < b.0 + b.2 + PAD && b.0 < a.0 + a.2 + PAD && a.1 < b.1 + b.3 + PAD && b.1 < a.1 + a.3 + PAD
}

struct GraphCanvas {
    view: Entity<GraphView>,
}

struct GraphPrepaint {
    graph: Graph,
    centers: Vec<Point<Pixels>>,
    radii: Vec<f32>,
    labels: Vec<PaintedLabel>,
    hovered: Option<usize>,
    highlight: HashSet<usize>,
    focus: Option<usize>,
}

struct PaintedLabel {
    line: ShapedLine,
    origin: Point<Pixels>,
}

impl IntoElement for GraphCanvas {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for GraphCanvas {
    type RequestLayoutState = ();
    type PrepaintState = GraphPrepaint;

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
        bounds: Bounds<Pixels>,
        _state: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> GraphPrepaint {
        self.view.update(cx, |view, _| {
            view.last_bounds = Some(bounds);
            view.clamp_pan();
        });

        let theme = *cx.global::<Theme>();
        let (graph, positions, zoom, pan, hovered, focus) = {
            let view = self.view.read(cx);
            (
                view.graph.clone(),
                view.layout.positions().to_vec(),
                view.zoom,
                view.pan,
                view.hovered,
                view.focus.clone(),
            )
        };

        let size = (f32::from(bounds.size.width), f32::from(bounds.size.height));
        let scale = fit_scale(size) * zoom;
        let centers: Vec<Point<Pixels>> = positions
            .iter()
            .map(|(x, y)| {
                let (x, y) = forward_point((*x, *y), size, zoom, pan);
                point(bounds.left() + px(x), bounds.top() + px(y))
            })
            .collect();
        let radii: Vec<f32> = graph
            .nodes
            .iter()
            .map(|node| graph::node_radius(node.links) * scale)
            .collect();
        let focus = focus
            .as_deref()
            .and_then(|relative| graph.index_of(relative));

        let highlight: HashSet<usize> = hovered
            .map(|node| {
                let mut set = HashSet::from([node]);
                set.extend(graph.neighbors(node));
                set
            })
            .unwrap_or_default();

        // Collect labels that are visible. Names fade in with zoom; hovering a node
        // always reveals its own label and the labels of its neighbors.
        let mut candidates = Vec::new();
        for (id, node) in graph.nodes.iter().enumerate() {
            let Some(center) = centers.get(id).copied() else {
                continue;
            };
            let radius = radii.get(id).copied().unwrap_or(0.0);
            let local_center = (
                f32::from(center.x - bounds.left()),
                f32::from(center.y - bounds.top()),
            );
            if local_center.0 + radius < 0.0
                || local_center.1 + radius < 0.0
                || local_center.0 - radius > size.0
                || local_center.1 - radius > size.1
            {
                continue;
            }

            let active = hovered == Some(id) || highlight.contains(&id);
            let alpha = if active {
                1.0
            } else {
                ((zoom - LABEL_ZOOM) / LABEL_FADE).clamp(0.0, 1.0)
            };
            if alpha <= 0.0 {
                continue;
            }

            candidates.push(LabelCandidate {
                node: id,
                width: 0.0,
                height: LABEL_LINE_HEIGHT,
                center: local_center,
                radius,
                priority: if hovered == Some(id) {
                    1_000_000
                } else if highlight.contains(&id) {
                    500_000
                } else {
                    0
                } + node.links,
            });
        }

        // Shape each visible name once per alpha bucket. The colour is baked into
        // the shaped line, so the cache is keyed by the text and alpha bucket.
        self.view.update(cx, |view, _| {
            if view.label_color != Some(theme.text_dim) {
                view.label_cache.clear();
                view.label_color = Some(theme.text_dim);
            }
        });

        let keys: Vec<(String, u8)> = candidates
            .iter()
            .map(|candidate| {
                let name = display_name(&graph.nodes[candidate.node].relative);
                let alpha =
                    if hovered == Some(candidate.node) || highlight.contains(&candidate.node) {
                        1.0
                    } else {
                        ((zoom - LABEL_ZOOM) / LABEL_FADE).clamp(0.0, 1.0)
                    };
                // A visible label must not round down to a zero-alpha cache entry.
                (
                    name,
                    (alpha * f32::from(LABEL_ALPHA_STEPS)).round().max(1.0) as u8,
                )
            })
            .collect();

        let cached: Vec<Option<ShapedLine>> = {
            let view = self.view.read(cx);
            keys.iter()
                .map(|key| view.label_cache.get(key).cloned())
                .collect()
        };

        let text_style = window.text_style();
        let font = text_style.font();
        let mut lines = Vec::with_capacity(candidates.len());
        let mut new_entries = Vec::new();
        for (index, candidate) in candidates.iter_mut().enumerate() {
            let key = keys[index].clone();
            let alpha = (f32::from(key.1) / f32::from(LABEL_ALPHA_STEPS)).clamp(0.0, 1.0);
            let line = if let Some(line) = cached[index].clone() {
                line
            } else {
                let name = &key.0;
                let run = TextRun {
                    len: name.len(),
                    font: font.clone(),
                    color: theme.text_dim.opacity(alpha),
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                };
                let line = window.text_system().shape_line(
                    SharedString::from(name.clone()),
                    px(LABEL_SIZE),
                    &[run],
                    None,
                );
                new_entries.push((key, line.clone()));
                line
            };
            candidate.width = f32::from(line.width);
            lines.push(line);
        }
        if !new_entries.is_empty() {
            self.view.update(cx, |view, _| {
                for (key, line) in new_entries {
                    view.label_cache.insert(key, line);
                }
            });
        }

        let placements = place_labels(&candidates, size);
        let labels = placements
            .into_iter()
            .filter_map(|placement| {
                let index = candidates
                    .iter()
                    .position(|candidate| candidate.node == placement.node)?;
                Some(PaintedLabel {
                    line: lines.get(index)?.clone(),
                    origin: point(
                        bounds.left() + px(placement.x),
                        bounds.top() + px(placement.y),
                    ),
                })
            })
            .collect();

        GraphPrepaint {
            graph,
            centers,
            radii,
            labels,
            hovered,
            highlight,
            focus,
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _state: &mut (),
        prepaint: &mut GraphPrepaint,
        window: &mut Window,
        cx: &mut App,
    ) {
        // This is a custom element, so it must clip itself. Without this mask,
        // pan and zoom can paint nodes and labels over the sidebar and panels.
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            let theme = *cx.global::<Theme>();
            let graph = &prepaint.graph;
            let centers = &prepaint.centers;
            let radii = &prepaint.radii;
            let hovered = prepaint.hovered;

            // Edges first, so nodes sit on top. Plain links are solid; frontmatter
            // relations are dashed and tinted, so the relation graph reads differently.
            if let Some(hovered) = hovered {
                let mut dim_links = PathBuilder::stroke(px(1.0));
                let mut dim_relations =
                    PathBuilder::stroke(px(1.0)).dash_array(&[px(4.0), px(3.0)]);
                let mut hi_links = PathBuilder::stroke(px(1.5));
                let mut hi_relations = PathBuilder::stroke(px(1.5)).dash_array(&[px(4.0), px(3.0)]);
                let mut drew_dim_links = false;
                let mut drew_dim_relations = false;
                let mut drew_hi_links = false;
                let mut drew_hi_relations = false;

                for edge in &graph.edges {
                    let Some(a) = centers.get(edge.from) else {
                        continue;
                    };
                    let Some(b) = centers.get(edge.to) else {
                        continue;
                    };
                    let incident = edge.from == hovered || edge.to == hovered;
                    let (path, drew) = match (edge.is_relation(), incident) {
                        (false, true) => (&mut hi_links, &mut drew_hi_links),
                        (true, true) => (&mut hi_relations, &mut drew_hi_relations),
                        (false, false) => (&mut dim_links, &mut drew_dim_links),
                        (true, false) => (&mut dim_relations, &mut drew_dim_relations),
                    };
                    if !*drew {
                        path.move_to(*a);
                        *drew = true;
                    } else {
                        path.line_to(*a);
                    }
                    path.line_to(*b);
                }

                paint_paths(
                    window,
                    dim_links,
                    drew_dim_links,
                    theme.border.opacity(0.25),
                );
                paint_paths(
                    window,
                    dim_relations,
                    drew_dim_relations,
                    theme.accent_text.opacity(0.25),
                );
                paint_paths(window, hi_links, drew_hi_links, theme.text);
                paint_paths(window, hi_relations, drew_hi_relations, theme.accent_text);
            } else {
                let mut links = PathBuilder::stroke(px(1.0));
                let mut relations = PathBuilder::stroke(px(1.0)).dash_array(&[px(4.0), px(3.0)]);
                let mut drew_link = false;
                let mut drew_relation = false;
                for edge in &graph.edges {
                    let Some(a) = centers.get(edge.from) else {
                        continue;
                    };
                    let Some(b) = centers.get(edge.to) else {
                        continue;
                    };
                    let (path, drew) = if edge.is_relation() {
                        (&mut relations, &mut drew_relation)
                    } else {
                        (&mut links, &mut drew_link)
                    };
                    if !*drew {
                        path.move_to(*a);
                        *drew = true;
                    } else {
                        path.line_to(*a);
                    }
                    path.line_to(*b);
                }
                paint_paths(window, links, drew_link, theme.border);
                paint_paths(window, relations, drew_relation, theme.accent_text);
            }

            // Nodes.
            for (id, node) in graph.nodes.iter().enumerate() {
                let Some(center) = centers.get(id).copied() else {
                    continue;
                };
                let radius = radii.get(id).copied().unwrap_or(0.0);
                let mut color = if prepaint.focus == Some(id) {
                    theme.accent
                } else {
                    folder_color(graph::folder_of(&node.relative), theme.dark)
                };
                if hovered.is_some() && !prepaint.highlight.contains(&id) {
                    color = color.opacity(0.3);
                }

                if hovered == Some(id) {
                    window.paint_quad(
                        fill(
                            Bounds::new(
                                point(center.x - px(radius + 5.0), center.y - px(radius + 5.0)),
                                size(px(radius * 2.0 + 10.0), px(radius * 2.0 + 10.0)),
                            ),
                            theme.text.opacity(0.18),
                        )
                        .corner_radii(Corners::all(px(radius + 5.0))),
                    );
                }

                window.paint_quad(
                    fill(
                        Bounds::new(
                            point(center.x - px(radius), center.y - px(radius)),
                            size(px(radius * 2.0), px(radius * 2.0)),
                        ),
                        color,
                    )
                    .corner_radii(Corners::all(px(radius))),
                );

                if hovered == Some(id) {
                    window.paint_quad(
                        outline(
                            Bounds::new(
                                point(center.x - px(radius + 1.5), center.y - px(radius + 1.5)),
                                size(px(radius * 2.0 + 3.0), px(radius * 2.0 + 3.0)),
                            ),
                            theme.text,
                            gpui::BorderStyle::default(),
                        )
                        .corner_radii(Corners::all(px(radius + 1.5))),
                    );
                }
            }

            // Labels last, so they stay readable over edges and circles.
            for label in &prepaint.labels {
                let _ = label
                    .line
                    .paint(label.origin, px(LABEL_LINE_HEIGHT), window, cx);
            }
        });
    }
}

fn paint_paths(
    window: &mut Window,
    path: PathBuilder,
    has_path: bool,
    color: impl Into<gpui::Background>,
) {
    if has_path {
        if let Ok(path) = path.build() {
            window.paint_path(path, color);
        }
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

    fn candidate(node: usize, center: (f32, f32), radius: f32, priority: usize) -> LabelCandidate {
        LabelCandidate {
            node,
            width: 60.0,
            height: LABEL_LINE_HEIGHT,
            center,
            radius,
            priority,
        }
    }

    #[test]
    fn labels_do_not_overlap_nodes_or_each_other() {
        let candidates = vec![
            candidate(0, (100.0, 100.0), 10.0, 10),
            candidate(1, (120.0, 100.0), 10.0, 9),
            candidate(2, (500.0, 300.0), 12.0, 8),
        ];
        let placed = place_labels(&candidates, (800.0, 600.0));
        assert!(!placed.is_empty());
        for (index, placement) in placed.iter().enumerate() {
            let rect = (placement.x, placement.y, 60.0, LABEL_LINE_HEIGHT);
            assert!(rect.0 >= 0.0 && rect.1 >= 0.0);
            assert!(rect.0 + rect.2 <= 800.0);
            assert!(rect.1 + rect.3 <= 600.0);
            for other in placed.iter().skip(index + 1) {
                let other_rect = (other.x, other.y, 60.0, LABEL_LINE_HEIGHT);
                assert!(!rects_intersect(rect, other_rect));
            }
        }
    }

    #[test]
    fn labels_are_hidden_when_the_pane_is_full() {
        let candidates = vec![candidate(0, (40.0, 40.0), 10.0, 1)];
        let placed = place_labels(&candidates, (40.0, 40.0));
        assert!(placed.is_empty());
    }

    #[test]
    fn labels_are_placed_in_priority_order() {
        let candidates = vec![
            candidate(0, (100.0, 100.0), 10.0, 1),
            candidate(1, (105.0, 100.0), 10.0, 100),
        ];
        let placed = place_labels(&candidates, (800.0, 600.0));
        assert_eq!(placed.first().map(|placement| placement.node), Some(1));
    }

    #[test]
    fn graph_bounds_include_the_node_radius_and_padding() {
        let graph = Graph {
            nodes: vec![
                dragonglass_core::graph::Node {
                    id: 0,
                    relative: "a.md".to_string(),
                    links: 0,
                },
                dragonglass_core::graph::Node {
                    id: 1,
                    relative: "b.md".to_string(),
                    links: 4,
                },
            ],
            edges: Vec::new(),
        };
        let bounds = graph_bounds(&graph, &[(100.0, 100.0), (200.0, 150.0)], 8.0).unwrap();
        assert!(bounds.0 <= 100.0 - 13.0);
        assert!(bounds.1 <= 100.0 - 13.0);
        assert!(bounds.2 >= 200.0 + 13.0);
        assert!(bounds.3 >= 150.0 + 13.0);
    }

    #[test]
    fn pan_clamp_keeps_a_graph_with_a_large_pan_visible() {
        let size = (900.0, 600.0);
        let graph = (400.0, 250.0, 600.0, 350.0);
        let (x, y) = clamp_pan_to_bounds((2000.0, 2000.0), size, 1.0, graph);
        let left = size.0 / 2.0 + x + (graph.0 - CANVAS.0 / 2.0) * fit_scale(size);
        let top = size.1 / 2.0 + y + (graph.1 - CANVAS.1 / 2.0) * fit_scale(size);
        assert!(left <= size.0 - PAN_MARGIN.min(size.0 / 3.0));
        assert!(top <= size.1 - PAN_MARGIN.min(size.1 / 3.0));
    }

    #[test]
    fn pan_clamp_leaves_a_centred_graph_alone() {
        let size = (900.0, 600.0);
        let graph = (400.0, 250.0, 600.0, 350.0);
        assert_eq!(
            clamp_pan_to_bounds((0.0, 0.0), size, 1.0, graph),
            (0.0, 0.0)
        );
    }

    #[test]
    fn force_fields_clamp_to_their_ranges() {
        let mut params = LayoutParams::new(1000.0, 600.0, 120);
        ForceField::Center.set(&mut params, 1.0);
        assert_eq!(params.center_force, 0.10);
        ForceField::Repel.set(&mut params, -1.0);
        assert_eq!(params.repel, 0.0);
        ForceField::Link.set(&mut params, 0.055);
        assert_eq!(params.link_force, 0.055);
        ForceField::Distance.set(&mut params, 500.0);
        assert_eq!(params.link_distance, 200.0);
    }

    #[test]
    fn link_distance_displays_auto_until_it_is_set() {
        let params = LayoutParams::new(1000.0, 600.0, 120);
        assert_eq!(ForceField::Distance.display(&params), "auto");
        let params = LayoutParams {
            link_distance: 80.0,
            ..params
        };
        assert_eq!(ForceField::Distance.display(&params), "80");
    }
}
