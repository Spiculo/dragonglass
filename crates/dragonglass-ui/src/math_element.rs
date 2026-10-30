//! A GPUI element that paints a laid-out math formula.
//!
//! The layout engine (`dragonglass_core::math`) produces a display list of glyph outlines,
//! rules and paths in em units. This element scales that list to the requested font size and
//! paints it with GPUI primitives: quads for rules/rectangles and tessellated paths for glyphs
//! and stretchy delimiter shapes. Nothing here re-shapes text, so the geometry matches the
//! layout exactly at any zoom level.

use std::rc::Rc;

use gpui::{
    div, fill, point, prelude::*, px, rgba, AnyTooltip, AnyView, App, Bounds, Context, Element,
    ElementId, FillOptions, FillRule, GlobalElementId, Hsla, InspectorElementId, IntoElement,
    LayoutId, Path, PathBuilder, PathStyle, Pixels, Render, Style, Window,
};

use dragonglass_core::math::{self, DisplayItem, GlyphCurve, MathScene};

use crate::theme::Theme;

/// A formula, ready to paint.
pub struct MathElement {
    scene: Rc<MathScene>,
    font_size: f32,
    /// Colour for items that still carry the neutral layout colour (i.e. did not use an
    /// explicit `\color`). Themes can change without re-laying out.
    text_color: Hsla,
}

impl MathElement {
    pub fn new(scene: Rc<MathScene>, font_size: f32, text_color: Hsla) -> Self {
        Self {
            scene,
            font_size,
            text_color,
        }
    }

    fn pixel_size(&self) -> (Pixels, Pixels) {
        let fs = self.font_size;
        (
            px((self.scene.width() as f32 * fs).max(1.0)),
            px((self.scene.total_height() as f32 * fs).max(1.0)),
        )
    }
}

/// A small tooltip that shows the formula's TeX source.
struct MathTooltip {
    tex: String,
}

impl Render for MathTooltip {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.global::<Theme>();
        div()
            .px_2()
            .py_1()
            .rounded_md()
            .bg(theme.chip)
            .border_1()
            .border_color(theme.border)
            .text_xs()
            .text_color(theme.text)
            .child(format!("${}$", self.tex))
    }
}

/// Ask GPUI to show `tex` as a tooltip at the current mouse position.
///
/// Called from `prepaint`, which is where `Window::set_tooltip` is allowed. While the pointer
/// stays on a formula this is refreshed every frame, so moving between formulas updates the
/// tooltip immediately.
pub(crate) fn set_math_tooltip(
    window: &mut Window,
    cx: &mut App,
    tex: &str,
    bounds: Bounds<Pixels>,
) {
    let view = cx.new(|_| MathTooltip {
        tex: tex.to_string(),
    });
    let view: AnyView = view.into();
    window.set_tooltip(AnyTooltip {
        view,
        mouse_position: window.mouse_position(),
        check_visible_and_update: Rc::new(move |_, window, _| {
            bounds.contains(&window.mouse_position())
        }),
    });
}

impl IntoElement for MathElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

/// A paint instruction resolved to pixels for one frame.
pub enum PaintOp {
    Quad(Bounds<Pixels>, Hsla),
    Path(Path<Pixels>, Hsla),
}

impl Element for MathElement {
    type RequestLayoutState = ();
    type PrepaintState = Vec<PaintOp>;

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
    ) -> (LayoutId, Self::RequestLayoutState) {
        let (width, height) = self.pixel_size();
        let style = Style {
            size: gpui::Size {
                width: width.into(),
                height: height.into(),
            },
            ..Style::default()
        };
        (window.request_layout(style, None, cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _state: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Vec<PaintOp> {
        if bounds.contains(&window.mouse_position()) {
            set_math_tooltip(window, cx, &self.scene.source, bounds);
        }
        prepare(&self.scene, self.font_size, self.text_color, bounds.origin)
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _state: &mut Self::RequestLayoutState,
        ops: &mut Vec<PaintOp>,
        window: &mut Window,
        _cx: &mut App,
    ) {
        for op in ops.drain(..) {
            match op {
                PaintOp::Quad(bounds, color) => window.paint_quad(fill(bounds, color)),
                PaintOp::Path(path, color) => window.paint_path(path, color),
            }
        }
    }
}

/// Paint a formula directly at `origin` (top-left of its em box).
///
/// The inline flow layout uses this instead of embedding a `MathElement`, because it needs to
/// place the box on the text baseline itself.
pub(crate) fn paint_scene(
    window: &mut Window,
    scene: &MathScene,
    font_size: f32,
    text_color: Hsla,
    origin: gpui::Point<Pixels>,
) {
    for op in prepare(scene, font_size, text_color, origin) {
        match op {
            PaintOp::Quad(bounds, color) => window.paint_quad(fill(bounds, color)),
            PaintOp::Path(path, color) => window.paint_path(path, color),
        }
    }
}

/// Resolve every display-list item to pixels at `origin`.
fn prepare(
    scene: &MathScene,
    font_size: f32,
    text_color: Hsla,
    origin: gpui::Point<Pixels>,
) -> Vec<PaintOp> {
    let fs = font_size;
    let mut ops = Vec::with_capacity(scene.display.items.len());

    for item in &scene.display.items {
        match item {
            DisplayItem::Line {
                x,
                y,
                width,
                thickness,
                color,
                dashed,
            } => {
                let color = item_color(color, text_color);
                let thickness = (*thickness as f32 * fs).max(0.75);
                let left = origin.x + px(*x as f32 * fs);
                let top = origin.y + px(*y as f32 * fs) - px(thickness / 2.0);
                if *dashed {
                    // Matches RaTeX's rasterizer: 4·t dash, 4·t gap.
                    let dash = (4.0 * thickness).max(2.0);
                    let mut cursor = 0.0f32;
                    let total = *width as f32 * fs;
                    // Bound the loop for pathological geometry.
                    while cursor < total && ops.len() < 100_000 {
                        let segment = dash.min(total - cursor);
                        ops.push(PaintOp::Quad(
                            Bounds {
                                origin: point(left + px(cursor), top),
                                size: gpui::size(px(segment), px(thickness)),
                            },
                            color,
                        ));
                        cursor += dash * 2.0;
                    }
                } else {
                    ops.push(PaintOp::Quad(
                        Bounds {
                            origin: point(left, top),
                            size: gpui::size(px((*width as f32 * fs).max(0.1)), px(thickness)),
                        },
                        color,
                    ));
                }
            }
            DisplayItem::Rect {
                x,
                y,
                width,
                height,
                color,
            } => {
                ops.push(PaintOp::Quad(
                    Bounds {
                        origin: point(origin.x + px(*x as f32 * fs), origin.y + px(*y as f32 * fs)),
                        size: gpui::size(
                            px((*width as f32 * fs).max(0.1)),
                            px((*height as f32 * fs).max(0.1)),
                        ),
                    },
                    item_color(color, text_color),
                ));
            }
            DisplayItem::Path {
                x,
                y,
                commands,
                fill: filled,
                color,
            } => {
                let color = item_color(color, text_color);
                // Split at MoveTo: stretchy arrows are several subpaths whose windings would
                // otherwise cancel if tessellated as one path.
                let mut start = 0;
                for index in 1..=commands.len() {
                    let boundary = index == commands.len()
                        || matches!(
                            commands[index],
                            dragonglass_core::math::PathCommand::MoveTo { .. }
                        );
                    if !boundary {
                        continue;
                    }
                    if let Some(path) = build_path(
                        &commands[start..index],
                        *x as f32 * fs + f32::from(origin.x),
                        *y as f32 * fs + f32::from(origin.y),
                        fs,
                        *filled,
                    ) {
                        ops.push(PaintOp::Path(path, color));
                    }
                    start = index;
                }
            }
            DisplayItem::GlyphPath {
                x,
                y,
                scale,
                font,
                char_code,
                color,
            } => {
                let color = item_color(color, text_color);
                if let Some(outline) = math::glyph_outline(font, *char_code) {
                    if let Some(path) = build_glyph(
                        &outline,
                        f32::from(origin.x) + *x as f32 * fs,
                        f32::from(origin.y) + *y as f32 * fs,
                        fs * *scale as f32,
                    ) {
                        ops.push(PaintOp::Path(path, color));
                    }
                }
            }
        }
    }

    ops
}

/// The neutral layout colour stands in for "use the theme foreground".
fn item_color(color: &math::Color, text_color: Hsla) -> Hsla {
    if color.r <= 0.001 && color.g <= 0.001 && color.b <= 0.001 {
        return text_color;
    }
    let r = (color.r.clamp(0.0, 1.0) * 255.0).round() as u32;
    let g = (color.g.clamp(0.0, 1.0) * 255.0).round() as u32;
    let b = (color.b.clamp(0.0, 1.0) * 255.0).round() as u32;
    let a = (color.a.clamp(0.0, 1.0) * 255.0).round() as u32;
    rgba((r << 24) | (g << 16) | (b << 8) | a).into()
}

fn build_glyph(
    outline: &math::GlyphOutline,
    origin_x: f32,
    baseline_y: f32,
    em: f32,
) -> Option<Path<Pixels>> {
    if outline.units_per_em <= 0.0 || em <= 0.0 {
        return None;
    }
    let scale = em / outline.units_per_em;
    let mut builder = PathBuilder::fill();
    let mut last: Option<(f32, f32)> = None;
    let mut started = false;

    for curve in &outline.curves {
        let (start, end) = match curve {
            GlyphCurve::Line(a, b) => (a, b),
            GlyphCurve::Quad(a, _, c) => (a, c),
            GlyphCurve::Cubic(a, _, _, d) => (a, d),
        };
        let sx = origin_x + start.x * scale;
        let sy = baseline_y - start.y * scale;

        let needs_move = match last {
            None => true,
            Some((lx, ly)) => (lx - sx).abs() > 0.01 || (ly - sy).abs() > 0.01,
        };
        if needs_move {
            if started {
                builder.close();
            }
            builder.move_to(point(px(sx), px(sy)));
            started = true;
        }

        match curve {
            GlyphCurve::Line(_, b) => {
                builder.line_to(point(
                    px(origin_x + b.x * scale),
                    px(baseline_y - b.y * scale),
                ));
            }
            GlyphCurve::Quad(_, c, b) => {
                builder.curve_to(
                    point(px(origin_x + b.x * scale), px(baseline_y - b.y * scale)),
                    point(px(origin_x + c.x * scale), px(baseline_y - c.y * scale)),
                );
            }
            GlyphCurve::Cubic(_, c1, c2, b) => {
                // GPUI's path builder has no cubic segment; flatten the rare CFF curve.
                let (x0, y0) = (start.x, start.y);
                let (x3, y3) = (b.x, b.y);
                let steps = 8;
                for step in 1..=steps {
                    let t = step as f32 / steps as f32;
                    let mt = 1.0 - t;
                    let x = mt * mt * mt * x0
                        + 3.0 * mt * mt * t * c1.x
                        + 3.0 * mt * t * t * c2.x
                        + t * t * t * x3;
                    let y = mt * mt * mt * y0
                        + 3.0 * mt * mt * t * c1.y
                        + 3.0 * mt * t * t * c2.y
                        + t * t * t * y3;
                    builder.line_to(point(px(origin_x + x * scale), px(baseline_y - y * scale)));
                }
            }
        }
        last = Some((origin_x + end.x * scale, baseline_y - end.y * scale));
    }

    if started {
        builder.close();
    }
    builder.build().ok()
}

fn build_path(
    commands: &[dragonglass_core::math::PathCommand],
    origin_x: f32,
    origin_y: f32,
    em: f32,
    filled: bool,
) -> Option<Path<Pixels>> {
    use dragonglass_core::math::PathCommand;

    let mut builder = if filled {
        PathBuilder::fill().with_style(PathStyle::Fill(
            FillOptions::default().with_fill_rule(FillRule::EvenOdd),
        ))
    } else {
        PathBuilder::stroke(px(1.0))
    };

    let mut any = false;
    let mut cursor = (0.0f32, 0.0f32);
    let to_px = |x: f32, y: f32| point(px(origin_x + x * em), px(origin_y + y * em));

    for command in commands {
        match command {
            PathCommand::MoveTo { x, y } => {
                cursor = (*x as f32, *y as f32);
                builder.move_to(to_px(cursor.0, cursor.1));
                any = true;
            }
            PathCommand::LineTo { x, y } => {
                cursor = (*x as f32, *y as f32);
                builder.line_to(to_px(cursor.0, cursor.1));
            }
            PathCommand::CubicTo {
                x1,
                y1,
                x2,
                y2,
                x,
                y,
            } => {
                // Flatten the cubic into short lines; display-list cubics are only used by a
                // few large-delimiter shapes, so precision matters more than speed here.
                let (x0, y0) = cursor;
                let (x3, y3) = (*x as f32, *y as f32);
                let steps = 8;
                for step in 1..=steps {
                    let t = step as f32 / steps as f32;
                    let mt = 1.0 - t;
                    let cx = mt * mt * mt * x0
                        + 3.0 * mt * mt * t * (*x1 as f32)
                        + 3.0 * mt * t * t * (*x2 as f32)
                        + t * t * t * x3;
                    let cy = mt * mt * mt * y0
                        + 3.0 * mt * mt * t * (*y1 as f32)
                        + 3.0 * mt * t * t * (*y2 as f32)
                        + t * t * t * y3;
                    builder.line_to(to_px(cx, cy));
                }
                cursor = (x3, y3);
            }
            PathCommand::QuadTo { x1, y1, x, y } => {
                cursor = (*x as f32, *y as f32);
                builder.curve_to(to_px(cursor.0, cursor.1), to_px(*x1 as f32, *y1 as f32));
            }
            PathCommand::Close => builder.close(),
        }
    }

    if !any {
        return None;
    }
    builder.build().ok()
}
