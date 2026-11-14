//! Measured inline flow: shaped text runs plus atomic math boxes on one baseline.
//!
//! This replaces the earlier flex-wrap approximation. Layout runs once per available width
//! (inside GPUI's measured-layout pass), so painting is just drawing cached shaped lines and
//! math scenes. Selection maps screen positions back to the same display-text offsets the
//! rest of the reading view uses, including atomic selection of a formula.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    fill, point, px, App, Bounds, Corners, CursorStyle, DispatchPhase, Element, ElementId, Font,
    GlobalElementId, Hitbox, HitboxBehavior, Hsla, InspectorElementId, IntoElement, LayoutId,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, RenderImage, ShapedLine,
    SharedString, StrikethroughStyle, Style, TextRun, UnderlineStyle, Window,
};

use dragonglass_core::math::MathScene;

use crate::math_element;
use crate::reading::{ImageOpener, LinkOpener, ReadPoint, ReadSelection};
use crate::theme::Theme;

/// One inline piece, in display-text order.
pub(crate) struct FlowPiece {
    pub kind: FlowKind,
    /// Byte offset within the text passed to the rich renderer.
    pub offset: usize,
    /// Bytes this piece contributes to the block's display text.
    pub len: usize,
}

pub(crate) enum FlowKind {
    Text {
        text: SharedString,
        font: Font,
        color: Hsla,
        background: Option<Hsla>,
        underline: bool,
        strike: bool,
        href: Option<String>,
    },
    Math {
        scene: Rc<MathScene>,
        color: Hsla,
    },
    /// An inline image, painted at its intrinsic aspect ratio.
    Image {
        data: Arc<RenderImage>,
        width: Pixels,
        height: Pixels,
        href: Option<String>,
    },
}

pub(crate) struct FlowContent {
    pub pieces: Vec<FlowPiece>,
    pub font_size: f32,
    /// Line-height multiplier, matching the surrounding prose.
    pub line_height: f32,
    pub link_color: Hsla,
}

/// A paragraph with math in it.
pub(crate) struct FlowText {
    content: Rc<FlowContent>,
    layout: Rc<RefCell<Option<Rc<FlowLayout>>>>,
    block: Option<usize>,
    base: usize,
    len: usize,
    selection: Rc<RefCell<ReadSelection>>,
    opener: Option<LinkOpener>,
    on_image: Option<ImageOpener>,
    theme: Theme,
}

impl FlowText {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        content: Rc<FlowContent>,
        block: Option<usize>,
        base: usize,
        len: usize,
        selection: Rc<RefCell<ReadSelection>>,
        opener: Option<LinkOpener>,
        on_image: Option<ImageOpener>,
        theme: Theme,
    ) -> Self {
        Self {
            content,
            layout: Rc::new(RefCell::new(None)),
            block,
            base,
            len,
            selection,
            opener,
            on_image,
            theme,
        }
    }
}

impl IntoElement for FlowText {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

pub struct FlowLayout {
    lines: Vec<FlowLine>,
    height: Pixels,
}

struct FlowLine {
    pieces: Vec<PlacedPiece>,
    top: Pixels,
    ascent: Pixels,
    descent: Pixels,
}

enum PlacedPiece {
    Text {
        shaped: ShapedLine,
        x: Pixels,
        display_offset: usize,
        len: usize,
        href: Option<String>,
        color: Hsla,
    },
    Math {
        scene: Rc<MathScene>,
        x: Pixels,
        display_offset: usize,
        len: usize,
        color: Hsla,
    },
    Image {
        data: Arc<RenderImage>,
        x: Pixels,
        ascent: Pixels,
        width: Pixels,
        height: Pixels,
        display_offset: usize,
        len: usize,
        href: Option<String>,
    },
}

impl FlowLayout {
    fn build(content: &FlowContent, width: Pixels, window: &mut Window, _cx: &mut App) -> Self {
        let font_size = px(content.font_size);
        let line_height = px(content.font_size * content.line_height);
        let mut lines = Vec::new();
        let mut current: Vec<PlacedPiece> = Vec::new();
        let mut x = px(0.0);
        let mut ascent = px(0.0);
        let mut descent = px(0.0);
        let mut top = px(0.0);

        let flush = |current: &mut Vec<PlacedPiece>,
                     ascent: &mut Pixels,
                     descent: &mut Pixels,
                     top: &mut Pixels,
                     lines: &mut Vec<FlowLine>| {
            if current.is_empty() {
                return;
            }
            let height = (*ascent + *descent).max(line_height);
            let pieces = std::mem::take(current);
            lines.push(FlowLine {
                pieces,
                top: *top,
                ascent: *ascent,
                descent: *descent,
            });
            *top += height;
            *ascent = px(0.0);
            *descent = px(0.0);
        };

        for piece in &content.pieces {
            match &piece.kind {
                FlowKind::Text {
                    text,
                    font,
                    color,
                    background,
                    underline,
                    strike,
                    href,
                } => {
                    if text.trim().is_empty() && x == px(0.0) {
                        continue;
                    }
                    let run = TextRun {
                        len: text.len(),
                        font: font.clone(),
                        color: *color,
                        background_color: *background,
                        underline: underline.then_some(UnderlineStyle {
                            thickness: px(1.0),
                            color: Some(content.link_color),
                            wavy: false,
                        }),
                        strikethrough: strike.then_some(StrikethroughStyle {
                            thickness: px(1.0),
                            color: None,
                        }),
                    };
                    let shaped =
                        window
                            .text_system()
                            .shape_line(text.clone(), font_size, &[run], None);
                    let piece_width = shaped.width;
                    if x > px(0.0) && x + piece_width > width {
                        flush(
                            &mut current,
                            &mut ascent,
                            &mut descent,
                            &mut top,
                            &mut lines,
                        );
                        x = px(0.0);
                    }
                    ascent = ascent.max(shaped.ascent);
                    descent = descent.max(shaped.descent);
                    current.push(PlacedPiece::Text {
                        shaped,
                        x,
                        display_offset: piece.offset,
                        len: piece.len,
                        href: href.clone(),
                        color: *color,
                    });
                    x += piece_width;
                }
                FlowKind::Math { scene, color } => {
                    let math_width = px(scene.width() as f32 * content.font_size);
                    if x > px(0.0) && x + math_width > width {
                        flush(
                            &mut current,
                            &mut ascent,
                            &mut descent,
                            &mut top,
                            &mut lines,
                        );
                        x = px(0.0);
                    }
                    ascent = ascent.max(px(scene.height() as f32 * content.font_size));
                    descent = descent.max(px(scene.depth() as f32 * content.font_size));
                    current.push(PlacedPiece::Math {
                        scene: scene.clone(),
                        x,
                        display_offset: piece.offset,
                        len: piece.len,
                        color: *color,
                    });
                    x += math_width;
                }
                FlowKind::Image {
                    data,
                    width: requested_width,
                    height: requested_height,
                    href,
                } => {
                    let mut image_width = *requested_width;
                    let mut image_height = *requested_height;
                    if x > px(0.0) && x + image_width > width {
                        flush(
                            &mut current,
                            &mut ascent,
                            &mut descent,
                            &mut top,
                            &mut lines,
                        );
                        x = px(0.0);
                    }
                    // Never overflow the column: scale the picture down, keeping its ratio.
                    if image_width > width && image_width > px(0.0) {
                        let scale = width / image_width;
                        image_width = width;
                        image_height *= scale;
                    }
                    let image_ascent = image_height * 0.8;
                    ascent = ascent.max(image_ascent);
                    descent = descent.max(image_height - image_ascent);
                    current.push(PlacedPiece::Image {
                        data: data.clone(),
                        x,
                        ascent: image_ascent,
                        width: image_width,
                        height: image_height,
                        display_offset: piece.offset,
                        len: piece.len,
                        href: href.clone(),
                    });
                    x += image_width;
                }
            }
        }

        flush(
            &mut current,
            &mut ascent,
            &mut descent,
            &mut top,
            &mut lines,
        );

        Self { lines, height: top }
    }

    /// The display offset under a window position, clamped to the paragraph.
    fn point_at(
        &self,
        base: usize,
        len: usize,
        position: gpui::Point<Pixels>,
        font_size: f32,
    ) -> ReadPoint {
        let mut line = self.lines.last();
        for candidate in &self.lines {
            if position.y < candidate.top + candidate.ascent + candidate.descent {
                line = Some(candidate);
                break;
            }
        }
        let Some(line) = line else {
            return ReadPoint {
                block: 0,
                offset: base,
            };
        };

        let mut chosen: Option<&PlacedPiece> = None;
        for piece in &line.pieces {
            let width = match piece {
                PlacedPiece::Text { shaped, .. } => shaped.width,
                PlacedPiece::Math { scene, .. } => px(scene.width() as f32 * font_size),
                PlacedPiece::Image { width, .. } => *width,
            };
            if position.x <= piece_x(piece) + width {
                chosen = Some(piece);
                break;
            }
        }

        let offset = match chosen {
            Some(PlacedPiece::Text {
                shaped,
                x,
                display_offset,
                len: piece_len,
                ..
            }) => {
                let local = shaped
                    .index_for_x(position.x - *x)
                    .unwrap_or(*piece_len)
                    .min(*piece_len);
                display_offset + local
            }
            Some(PlacedPiece::Math {
                x,
                display_offset,
                len: piece_len,
                scene,
                ..
            }) => {
                let mid = *x + px(scene.width() as f32 * font_size) / 2.0;
                if position.x < mid {
                    *display_offset
                } else {
                    display_offset + piece_len
                }
            }
            Some(PlacedPiece::Image {
                x,
                width,
                display_offset,
                len: piece_len,
                ..
            }) => {
                if position.x < *x + *width / 2.0 {
                    *display_offset
                } else {
                    display_offset + piece_len
                }
            }
            None => line
                .pieces
                .last()
                .map(|piece| piece_display_offset(piece) + piece_display_len(piece))
                .unwrap_or(0),
        };

        ReadPoint {
            block: 0,
            offset: base + offset.min(len),
        }
    }
}

fn piece_x(piece: &PlacedPiece) -> Pixels {
    match piece {
        PlacedPiece::Text { x, .. }
        | PlacedPiece::Math { x, .. }
        | PlacedPiece::Image { x, .. } => *x,
    }
}

fn piece_display_offset(piece: &PlacedPiece) -> usize {
    match piece {
        PlacedPiece::Text { display_offset, .. }
        | PlacedPiece::Math { display_offset, .. }
        | PlacedPiece::Image { display_offset, .. } => *display_offset,
    }
}

fn piece_display_len(piece: &PlacedPiece) -> usize {
    match piece {
        PlacedPiece::Text { len, .. }
        | PlacedPiece::Math { len, .. }
        | PlacedPiece::Image { len, .. } => *len,
    }
}

impl Element for FlowText {
    type RequestLayoutState = ();
    type PrepaintState = (Rc<FlowLayout>, Hitbox);

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
        _cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let content = self.content.clone();
        let slot = self.layout.clone();
        let layout_id = window.request_measured_layout(
            Style::default(),
            move |known, available, window, cx| {
                let width = known.width.unwrap_or_else(|| match available.width {
                    gpui::AvailableSpace::Definite(width) => width,
                    _ => px(640.0),
                });
                let layout = Rc::new(FlowLayout::build(&content, width, window, cx));
                let size = gpui::size(width, layout.height);
                *slot.borrow_mut() = Some(layout);
                size
            },
        );
        (layout_id, ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _state: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let layout = self.layout.borrow().clone().unwrap_or_else(|| {
            Rc::new(FlowLayout {
                lines: Vec::new(),
                height: px(0.0),
            })
        });
        let mouse = window.mouse_position();
        if bounds.contains(&mouse) {
            if let Some((tex, local)) =
                layout.math_hit_at(mouse - bounds.origin, self.content.font_size)
            {
                let math_bounds = Bounds {
                    origin: point(
                        bounds.origin.x + local.origin.x,
                        bounds.origin.y + local.origin.y,
                    ),
                    size: local.size,
                };
                math_element::set_math_tooltip(window, cx, tex, math_bounds);
            }
        }
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        (layout, hitbox)
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _state: &mut Self::RequestLayoutState,
        state: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let (layout, hitbox) = state;
        let origin = bounds.origin;
        let theme = self.theme;
        let block = self.block;
        let base = self.base;
        let len = self.len;
        let font_size = self.content.font_size;

        // Mouse listeners are registered during paint.
        if let Some(block) = block {
            let selection = self.selection.clone();
            let down_layout = layout.clone();
            window.on_mouse_event(
                move |event: &MouseDownEvent, phase, window: &mut Window, _cx: &mut App| {
                    if phase != DispatchPhase::Bubble || event.button != MouseButton::Left {
                        return;
                    }
                    if !bounds.contains(&event.position) {
                        return;
                    }
                    let point = down_layout.point_at(base, len, event.position - origin, font_size);
                    let mut state = selection.borrow_mut();
                    state.anchor = Some(ReadPoint { block, ..point });
                    state.head = Some(ReadPoint { block, ..point });
                    state.down = Some(event.position);
                    state.dragging = true;
                    state.moved = false;
                    drop(state);
                    window.refresh();
                },
            );

            let selection = self.selection.clone();
            let drag_layout = layout.clone();
            window.on_mouse_event(
                move |event: &MouseMoveEvent, phase, window: &mut Window, _cx: &mut App| {
                    if phase != DispatchPhase::Bubble {
                        return;
                    }
                    let mut state = selection.borrow_mut();
                    if !state.dragging {
                        return;
                    }
                    if event.position.y < bounds.top() - px(2.0)
                        || event.position.y > bounds.bottom() + px(2.0)
                    {
                        return;
                    }
                    let point = drag_layout.point_at(base, len, event.position - origin, font_size);
                    let point = ReadPoint { block, ..point };
                    if state.head != Some(point) {
                        state.head = Some(point);
                        state.moved = true;
                        drop(state);
                        window.refresh();
                    }
                },
            );

            let selection = self.selection.clone();
            let opener = self.opener.clone();
            let on_image = self.on_image.clone();
            let up_layout = layout.clone();
            window.on_mouse_event(
                move |event: &MouseUpEvent, phase, window: &mut Window, cx: &mut App| {
                    if phase != DispatchPhase::Bubble {
                        return;
                    }
                    let mut state = selection.borrow_mut();
                    if !state.dragging {
                        return;
                    }
                    state.dragging = false;
                    let clicked = match state.down {
                        Some(down) => (event.position - down).magnitude() < 4.0,
                        None => !state.moved,
                    };
                    let head = state.head;
                    if clicked {
                        state.clear();
                    }
                    drop(state);

                    if clicked {
                        if let Some(head) = head {
                            let local = head.offset.saturating_sub(base);
                            if let Some(target) = up_layout.link_at(local) {
                                if let Some(opener) = &opener {
                                    opener(&target, window, cx);
                                }
                            } else if let Some(target) = up_layout.image_link_at(local, len) {
                                if let Some(on_image) = &on_image {
                                    on_image(&target, window, cx);
                                }
                            }
                        }
                    }
                    window.refresh();
                },
            );
        }

        // Backgrounds first (code spans, highlights), then selection, then ink.
        for line in &layout.lines {
            let baseline = origin.y + line.top + line.ascent;
            for piece in &line.pieces {
                if let PlacedPiece::Text { shaped, x, .. } = piece {
                    let _ = shaped.paint_background(
                        point(origin.x + *x, baseline - shaped.ascent),
                        shaped.ascent + shaped.descent,
                        window,
                        cx,
                    );
                }
            }
        }

        if let Some((start, end)) = self.selection.borrow().ordered() {
            for quad in flow_selection_quads(
                &layout,
                block,
                base,
                origin,
                start,
                end,
                theme.selection,
                self.content.font_size,
            ) {
                window.paint_quad(quad);
            }
        }

        for line in &layout.lines {
            let baseline = origin.y + line.top + line.ascent;
            for piece in &line.pieces {
                match piece {
                    PlacedPiece::Text {
                        shaped, x, color, ..
                    } => {
                        let text_origin = point(origin.x + *x, baseline - shaped.ascent);
                        let _ =
                            shaped.paint(text_origin, shaped.ascent + shaped.descent, window, cx);
                        let _ = color;
                    }
                    PlacedPiece::Math {
                        scene, x, color, ..
                    } => {
                        let math_origin = point(
                            origin.x + *x,
                            baseline - px(scene.height() as f32 * self.content.font_size),
                        );
                        math_element::paint_scene(
                            window,
                            scene,
                            self.content.font_size,
                            *color,
                            math_origin,
                        );
                    }
                    PlacedPiece::Image {
                        data,
                        x,
                        ascent,
                        width,
                        height,
                        ..
                    } => {
                        let image_bounds = Bounds {
                            origin: point(origin.x + *x, baseline - *ascent),
                            size: gpui::size(*width, *height),
                        };
                        let _ = window.paint_image(
                            image_bounds,
                            Corners::default(),
                            data.clone(),
                            0,
                            false,
                        );
                    }
                }
            }
        }

        // Pointer cursor over links.
        if let Some(link_hit) =
            layout.piece_at(base, len, window.mouse_position() - origin, font_size)
        {
            if bounds.contains(&window.mouse_position()) {
                let clickable = matches!(
                    link_hit,
                    PlacedPiece::Text { href: Some(_), .. }
                        | PlacedPiece::Image { href: Some(_), .. }
                );
                if clickable {
                    window.set_cursor_style(CursorStyle::PointingHand, hitbox);
                }
            }
        }
    }
}

impl FlowLayout {
    /// The href of an image piece whose display range contains `offset`.
    fn image_link_at(&self, offset: usize, len: usize) -> Option<String> {
        if offset > len {
            return None;
        }
        for line in &self.lines {
            for piece in &line.pieces {
                if let PlacedPiece::Image {
                    display_offset,
                    len: piece_len,
                    href: Some(href),
                    ..
                } = piece
                {
                    if offset >= *display_offset && offset <= display_offset + piece_len {
                        return Some(href.clone());
                    }
                }
            }
        }
        None
    }

    /// The TeX source and bounds (local to the paragraph) of a math box under `position`.
    fn math_hit_at(
        &self,
        position: gpui::Point<Pixels>,
        font_size: f32,
    ) -> Option<(&str, Bounds<Pixels>)> {
        let line = self.lines.iter().find(|line| {
            position.y >= line.top && position.y < line.top + line.ascent + line.descent
        })?;
        for piece in &line.pieces {
            if let PlacedPiece::Math { scene, x, .. } = piece {
                let width = px(scene.width() as f32 * font_size);
                if position.x >= *x && position.x <= *x + width {
                    return Some((
                        scene.source.as_str(),
                        Bounds {
                            origin: point(*x, line.top),
                            size: gpui::size(width, line.ascent + line.descent),
                        },
                    ));
                }
            }
        }
        None
    }

    fn link_at(&self, offset: usize) -> Option<String> {
        for line in &self.lines {
            for piece in &line.pieces {
                if let PlacedPiece::Text {
                    display_offset,
                    len,
                    href: Some(href),
                    ..
                } = piece
                {
                    if offset >= *display_offset && offset < display_offset + len {
                        return Some(href.clone());
                    }
                }
            }
        }
        None
    }

    fn piece_at(
        &self,
        base: usize,
        len: usize,
        position: gpui::Point<Pixels>,
        font_size: f32,
    ) -> Option<&PlacedPiece> {
        let mut line = self.lines.last();
        for candidate in &self.lines {
            if position.y < candidate.top + candidate.ascent + candidate.descent {
                line = Some(candidate);
                break;
            }
        }
        let line = line?;
        for piece in &line.pieces {
            let width = match piece {
                PlacedPiece::Text { shaped, .. } => shaped.width,
                PlacedPiece::Math { scene, .. } => px(scene.width() as f32 * font_size),
                PlacedPiece::Image { width, .. } => *width,
            };
            if position.x <= piece_x(piece) + width {
                return Some(piece);
            }
        }
        let _ = (base, len);
        line.pieces.last()
    }
}

fn flow_selection_quads(
    layout: &FlowLayout,
    block: Option<usize>,
    base: usize,
    origin: gpui::Point<Pixels>,
    start: ReadPoint,
    end: ReadPoint,
    color: Hsla,
    font_size: f32,
) -> Vec<gpui::PaintQuad> {
    let Some(block) = block else {
        return Vec::new();
    };
    if end.block < block || start.block > block {
        return Vec::new();
    }
    let from = if start.block < block { 0 } else { start.offset };
    let to = if end.block > block {
        usize::MAX
    } else {
        end.offset
    };
    if from >= to {
        return Vec::new();
    }

    let mut quads = Vec::new();
    for line in &layout.lines {
        let top = origin.y + line.top;
        let height = (line.ascent + line.descent).max(px(font_size * 1.5));
        for piece in &line.pieces {
            let p_start = base + piece_display_offset(piece);
            let p_end = p_start + piece_display_len(piece);
            let a = from.max(p_start);
            let b = to.min(p_end);
            if a >= b {
                continue;
            }
            let (x0, x1) = match piece {
                PlacedPiece::Text {
                    shaped,
                    x,
                    display_offset,
                    len,
                    ..
                } => {
                    let local0 = a - (base + display_offset);
                    let local1 = (b - (base + display_offset)).min(*len);
                    let x0 = *x + shaped.x_for_index(local0.min(*len));
                    let x1 = if local1 >= shaped.len() {
                        *x + shaped.width
                    } else {
                        *x + shaped.x_for_index(local1)
                    };
                    (x0, x1)
                }
                PlacedPiece::Math { x, scene, .. } => {
                    (*x, *x + px(scene.width() as f32 * font_size))
                }
                PlacedPiece::Image { x, width, .. } => (*x, *x + *width),
            };
            if x1 > x0 {
                quads.push(fill(
                    Bounds {
                        origin: point(origin.x + x0, top),
                        size: gpui::size(x1 - x0, height),
                    },
                    color,
                ));
            }
        }
    }
    quads
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reading::{ReadPoint, ReadingCache};
    use dragonglass_core::math::{self, MathOptions, MathStyle};

    #[test]
    fn math_hit_test_returns_the_tex_source() {
        let scene = Rc::new(
            math::typeset(
                "x^2",
                &MathOptions {
                    style: MathStyle::Inline,
                    color: [0.0, 0.0, 0.0, 1.0],
                },
            )
            .unwrap(),
        );
        let layout = FlowLayout {
            lines: vec![FlowLine {
                pieces: vec![PlacedPiece::Math {
                    scene,
                    x: px(10.0),
                    display_offset: 0,
                    len: 5,
                    color: gpui::black(),
                }],
                top: px(0.0),
                ascent: px(10.0),
                descent: px(2.0),
            }],
            height: px(12.0),
        };

        assert_eq!(
            layout
                .math_hit_at(point(px(20.0), px(5.0)), 16.0)
                .map(|(tex, _)| tex),
            Some("x^2")
        );
        assert_eq!(
            layout
                .math_hit_at(point(px(5.0), px(5.0)), 16.0)
                .map(|(tex, _)| tex),
            None
        );
    }

    #[test]
    fn dragging_across_a_math_box_selects_its_tex_source() {
        let scene = Rc::new(
            math::typeset(
                "x^2",
                &MathOptions {
                    style: MathStyle::Inline,
                    color: [0.0, 0.0, 0.0, 1.0],
                },
            )
            .unwrap(),
        );
        let width = px(scene.width() as f32 * 16.0);
        let layout = FlowLayout {
            lines: vec![FlowLine {
                pieces: vec![PlacedPiece::Math {
                    scene,
                    x: px(0.0),
                    display_offset: 0,
                    len: 5,
                    color: gpui::black(),
                }],
                top: px(0.0),
                ascent: px(10.0),
                descent: px(2.0),
            }],
            height: px(12.0),
        };

        let start = layout.point_at(0, 5, point(px(1.0), px(5.0)), 16.0);
        let end = layout.point_at(0, 5, point(width - px(1.0), px(5.0)), 16.0);
        assert_eq!((start.offset, end.offset), (0, 5));

        let mut cache = ReadingCache::new();
        cache.update((1, 0), "$x^2$\n", None);
        cache.select_range(
            ReadPoint {
                block: 0,
                offset: start.offset,
            },
            ReadPoint {
                block: 0,
                offset: end.offset,
            },
        );
        assert_eq!(cache.selected_text().as_deref(), Some("$x^2$"));
    }
}
