# Graph view: dynamic nodes, no overlap, and pane clipping

Status: plan on branch `fix/graph-node-layout`, cut from `main`.
Scope: the desktop GPUI graph view. Android has no graph view yet.

## 1. Problems in the current implementation

### 1.1 Graph paint escapes the center pane

`GraphCanvas::paint` in `crates/dragonglass-ui/src/graph_view.rs` paints paths,
quads and shaped text directly into the window. A GPUI custom element may paint
outside its layout bounds unless it applies a content mask. The GPUI 0.2.2
element docs state this explicitly: elements are expected to stay in their
bounds, and `Window::with_content_mask` is the escape hatch for overlays.

The graph does not apply a content mask. Its parent chain does not clip either:
the center pane in `crates/dragonglass-ui/src/main.rs` uses `.relative()` but no
`.overflow_hidden()`. Pan, zoom, boundary nodes and long labels can therefore
paint over:

- the left file tree and sidebar,
- the outline and relations panels on the right,
- the tab bar and status bar above and below.

This is the z-order/clipping bug, not a layout bug. A node at the right edge of
the canvas is drawn at `center + radius`, and its label is drawn at
`center + radius + 3`. With no mask, both continue into the next panel.

### 1.2 Nodes can overlap

`graph::layout` in `crates/dragonglass-core/src/graph.rs` is one static
Fruchterman-Reingold pass:

- repulsion is `k * k / distance` and ignores node radius,
- attraction is `distance * distance / k`,
- every node is hard-clamped to `0..width` and `0..height`.

The equilibrium distance is not tied to `r_i + r_j`, so high-degree nodes (up to
16 px radius) can overlap. The hard clamp also stacks nodes against the edges
when the repulsion pushes them outward.

### 1.3 Labels always overlap

Every node gets a label at the same offset (right of the circle), for the first
60 nodes in vector order. There is no collision test against other labels, other
nodes or the pane edge. Long names and nodes close together produce unreadable
label piles.

### 1.4 The layout is static

`GraphView::set_graph` computes positions once. There is no simulation loop, no
node drag, no hover state and no reheat when the filter or depth changes. That
is the opposite of Obsidian's graph, where nodes move, settle and react to the
pointer.

### 1.5 Rebuilds jump

Any filter or depth change calls `rebuild`, which calls `set_graph` and computes
a new layout from scratch. Nodes jump to unrelated positions. `open_graph` also
keeps the previous zoom and pan, so the graph can reopen off-screen.

## 2. How Obsidian handles it

The Obsidian Help page for Graph view describes the user-facing contract:

- circles are notes, lines are internal links;
- a note with more references gets a bigger circle;
- hover highlights a note's connections, click opens the note;
- scroll zooms, drag pans;
- the settings panel has Filters, Groups, Display and Forces sections;
- Display controls node size, link thickness, arrows and a **text fade
  threshold** for note names;
- Forces controls **center force**, **repel force**, **link force** and **link
  distance**;
- a local graph adds a depth slider.

Source: <https://help.obsidian.md/plugins/graph>

The Obsidian Hub graph-view guide confirms the renderer: the graph is drawn on
a `<canvas>` with WebGL. CSS cannot affect nodes or links. Source:
<https://publish.obsidian.md/hub/04+-+Guides%2C+Workflows%2C+%26+Courses/Guides/Graph+view+customization>

Practical consequences for our design:

1. **Pane isolation.** A canvas inside a leaf is clipped to that leaf. The graph
   cannot paint over the file explorer or a side panel.
2. **Live force simulation.** Nodes settle over time, not in one batch. The
   forces are user-tunable. Reopening a graph can arrange it differently.
3. **Direct manipulation.** Nodes can be dragged. Other nodes move in response
   as the simulation reheats. Positions are not persisted by default; a plugin
   exists only because users want that.
4. **Label decluttering.** Names fade with zoom instead of always being drawn.
   Dense graphs can still show overlap, but the default view avoids most of it
   through repulsion and size scaling.
5. **Hover focus.** Hovering a node highlights its links and neighbors. This
   makes a dense graph readable without moving it.

## 3. Proposed design

### 3.1 Core: an incremental `GraphLayout`

Replace the one-shot `layout()` internals with a stateful layout that keeps
positions, velocities and an alpha (temperature). Keep the current free function
as a thin wrapper so headless callers and tests continue to work.

```rust
pub struct LayoutParams {
    pub width: f32,
    pub height: f32,
    pub padding: f32,
    pub iterations: usize,
    pub repel: f32,
    pub link_distance: f32,
    pub center_force: f32,
}

pub struct GraphLayout {
    positions: Vec<(f32, f32)>,
    velocities: Vec<(f32, f32)>,
    alpha: f32,
}

impl GraphLayout {
    pub fn new(graph: &Graph, params: LayoutParams) -> Self;
    pub fn step(&mut self, graph: &Graph, dt: f32) -> bool; // false when settled
    pub fn positions(&self) -> &[(f32, f32)];
    pub fn set_position(&mut self, node: usize, position: (f32, f32));
    pub fn reheat(&mut self, alpha: f32);
}
```

Move `node_radius(links)` from `graph_view.rs` into `graph.rs`. The layout and
the painter must use the same radius.

### 3.2 Collision and bounds

After the force pass in every step:

1. Run two or three collision passes. For each pair closer than
   `r_i + r_j + GAP`, push both nodes apart along the axis by half the
   penetration. Use the index order to break ties, so the result is
   deterministic.
2. Apply a soft boundary force when a node is within `padding + radius` of an
   edge. Clamp only as a last resort, and zero the outward velocity at the same
   time.
3. Stop when `alpha` is low and the largest displacement is below a threshold.

`MAX_NODES` is 80. With radii of 5–16 px in a 1000 × 600 canvas, collision
resolution has enough room to separate every node. Add a test for a dense
80-node graph and assert:

- every center stays inside `padding + radius .. edge - padding - radius`;
- every pair has `distance >= r_i + r_j` (within a small epsilon);
- two runs produce the same positions.

If a future graph cannot satisfy the constraint, the fallback is to grow the
virtual canvas and fit it uniformly, not to accept overlap silently.

### 3.3 Live simulation loop

`GraphView` owns a `GraphLayout` and a `frame_scheduled` flag.

- `set_graph` builds the layout and calls `cx.notify()`.
- `render` schedules `cx.on_next_frame` when the layout is active and no frame
  is already scheduled.
- The frame callback calls `step`, stores the new positions and calls
  `cx.notify()`. It reschedules only while `step` returns true.
- The loop stops when the layout settles. This respects the project rule
  "idle means idle": no timer runs when the graph is still.
- Guard the loop with the app's `show_graph` flag (or an explicit `active`
  flag). Closing the graph must stop the simulation immediately; a self
  rescheduling frame callback would otherwise keep running while hidden.

Use a bounded delta time (for example `1/120 .. 1/30` seconds) so a paused or
slow frame does not explode the simulation.

### 3.4 Direct manipulation

On mouse down:

1. Hit-test the node under the pointer in screen space.
2. If a node is hit, start a node drag: store the node id and the grab offset.
   Do not start a pan.
3. If no node is hit, start the existing pan gesture.

On mouse move:

- node drag: convert the pointer to canvas space, set the node position, zero
  its velocity, reheat the simulation, repaint;
- pan: keep the current behavior.

On mouse up:

- if a node was dragged, end the drag and pin that node until the next rebuild
  so it does not snap away immediately;
- if the press did not move, keep the existing click-to-open behavior;
- if the press panned, do nothing else.

Hover:

- track the hovered node on mouse move;
- highlight the hovered node, its edges and its direct neighbors;
- dim the rest;
- show the hovered label even when labels are otherwise hidden;
- set a pointer cursor over a node.

This is the behavior that makes Obsidian's graph feel alive. It also gives the
user a way to inspect a dense graph without changing the layout.

### 3.5 Label placement

Replace the unconditional "right of the node" label with a screen-space
placement pass:

1. Build occupied rectangles: every visible node circle, plus already accepted
   labels.
2. Sort candidates by degree (highest first), then by distance to the viewport
   center.
3. Try right, left, above and below each node with a 4 px gap.
4. Accept the first position whose label rectangle fits inside the pane and
   does not intersect an occupied rectangle.
5. Skip the label when no position fits.

Zoom behavior follows Obsidian's text fade threshold:

- below `LABEL_ZOOM` only the hovered node and its neighbors show labels;
- between `LABEL_ZOOM` and `LABEL_ZOOM + FADE` labels fade in;
- at high zoom all labels that fit are drawn.

The label pass is pure geometry and can be unit tested without a window:
`label_rects(nodes, positions, viewport, zoom) -> Vec<(usize, Rect)>`.

### 3.6 Pane clipping

Two layers of protection:

1. Add `.overflow_hidden()` to the graph view root and to the canvas container.
   This is the normal GPUI way to clip children.
2. In `GraphCanvas::paint`, wrap the edge, node and label painting in
   `window.with_content_mask(Some(ContentMask { bounds }), |window| { ... })`.
   A low-level custom element should not depend on every caller clipping it.

`paint_quad` and `paint_path` read the current content mask, and
`ShapedLine::paint` uses `paint_layer`, which intersects the mask. All three
will be clipped by the canvas bounds.

Keep hit-testing on the same `last_bounds`. A pointer outside the canvas cannot
hit a node because `bounds.localize` already rejects it.

### 3.7 Stable rebuilds and pan reset

- Keep a `HashMap<String, (f32, f32)>` of positions keyed by vault-relative
  path. On rebuild, seed matching nodes from the cache and place new nodes on a
  ring around the focus node. This removes the full-graph jump when a filter or
  depth changes.
- Reset zoom and pan when the graph is opened from the command palette or a
  workspace, unless a future workspace feature stores them. Obsidian reopens
  with a fresh view; our current code reopens with the last pan and can show an
  empty pane.
- Clamp pan so at least part of the graph stays visible. This is a safety net,
  not a substitute for clipping.

## 4. Work plan

### Phase 0 — Reproduce and baseline

- Add a debug vault with 80 notes, a few high-degree hubs and long names.
- Record the current bug: pan right/left until nodes paint over the tree and
  outline; zoom to 4× near an edge; open a dense graph and measure overlap.
- Run `cargo test`, `cargo fmt --all -- --check` and `cargo clippy
  --all-targets --all-features` on `main` to get a clean baseline.

### Phase 1 — Clip the graph (bug fix, smallest shippable change)

Files: `crates/dragonglass-ui/src/graph_view.rs`,
`crates/dragonglass-ui/src/main.rs`.

- Add `.overflow_hidden()` to the graph root and canvas container.
- Add the `ContentMask` wrapper in `GraphCanvas::paint`.
- Add a comment that explains why the custom element masks itself.
- Manual check: pan, zoom, resize and filter; no pixel crosses the graph pane.

### Phase 2 — Collision and bounds

Files: `crates/dragonglass-core/src/graph.rs`, `graph_view.rs`.

- Add `node_radius` to core and use it in both crates.
- Add `LayoutParams` and collision resolution.
- Replace hard clamping with a soft boundary plus a safety clamp.
- Add tests for no overlap, bounds and determinism.
- Keep the existing `layout()` signature as a wrapper.

### Phase 3 — Dynamic simulation and drag

Files: `graph_view.rs`, `graph.rs`.

- Add `GraphLayout` state and `step`.
- Add the `on_next_frame` loop with a settle stop.
- Add node hit-testing, node drag, reheat and pin-until-rebuild.
- Add hover state and the highlight/dim paint pass.
- Manual check: drag a hub node; neighbors move; the graph settles and stops
  repainting.

### Phase 4 — Labels

Files: `graph_view.rs`.

- Add the screen-space label placement function.
- Add zoom fade and hover override.
- Cache shaped lines per name and font size instead of shaping every frame.
- Unit test the placement function: no intersections, labels inside the pane,
  priority order, fallback to hidden.

### Phase 5 — Stability and polish

Files: `graph_view.rs`, `main.rs`, docs.

- Seed positions from the path cache on rebuild.
- Reset zoom/pan on open; clamp pan.
- Add a force settings popover only if it is still wanted after the dynamic
  layout lands (center, repel, link distance). This is the Obsidian Forces
  section, but it is not required for the bug fix.
- Update `PLAN.md`, `docs/qol-plan.md` and the README feature list.

## 5. Test matrix

Core unit tests:

| Test | Assertion |
|---|---|
| `layout_reserves_node_radius` | dense 80-node graph has no overlapping circles |
| `layout_keeps_nodes_in_bounds` | centers respect padding and radius |
| `layout_is_deterministic` | two runs return identical positions |
| `layout_stops_when_settled` | `step` returns false and does not move again |
| `layout_reheats_on_drag` | `set_position` + `reheat` makes `step` active |
| `label_placement_does_not_overlap` | no label rect intersects another label or node |
| `label_placement_stays_inside` | every accepted rect is inside the viewport |

Manual checks:

1. Open a graph, drag the canvas far left, right, up and down. Nothing paints
   over the tree, outline, relations, tab bar or status bar.
2. Zoom to 4× at each edge. Nodes and labels disappear at the pane edge.
3. Resize the window while the graph is panned. The clip follows the new bounds.
4. Open a dense 80-note graph. At rest, circles do not overlap. Labels either
   fit or are hidden.
5. Drag a node. It follows the pointer; the graph reacts and settles; CPU goes
   idle after settling.
6. Hover a node. Its edges and neighbors are highlighted and its label is
   visible.
7. Change the filter and depth. Existing nodes keep their approximate
   positions; no full-graph jump.
8. Close and reopen the graph. The view is reset and on-screen.
9. Click a node. The note opens, as before.
10. Esc closes the graph, as before.

Automated checks:

```sh
cargo test
cargo fmt --all -- --check
cargo clippy --all-targets --all-features
```

## 6. Decisions and recommendations

1. **Scope.** Do Phases 1–4 in this branch. Phase 5 force sliders can wait.
   Recommendation: ship the clipping fix first as its own commit, so it can be
   cherry-picked if the simulation work takes longer.
2. **Dragged node behavior.** Pin the dragged node until the next rebuild.
   Recommendation: pin; it matches "I put it there" expectations and avoids a
   snap-back.
3. **Positions across reopen.** Do not persist to disk. Reset the view on open,
   like Obsidian. Recommendation: keep the path cache only for the lifetime of
   the view.
4. **Labels.** Use zoom fade plus collision-aware placement. Recommendation:
   yes; always-on labels are the main source of visual overlap.
5. **Force settings UI.** Defer. Recommendation: expose the parameters in core
   now, add sliders only if users ask.
6. **Mobile.** Out of scope. The Android client has no graph view yet; the core
   layout will be reusable when it does.

## 7. Docs to update when the work lands

- `docs/graph-view.md` (this file): status, behavior and test notes.
- `PLAN.md`: graph view section, forces/collision and dynamic layout.
- `docs/qol-plan.md`: the graph bullet in the P2 list, plus a "Landed" note.
- `README.md`: the feature line that mentions the graph view.
