//! A minimal PDF writer, enough to print notes.
//!
//! Text uses the base-14 Helvetica font (ASCII only). Display formulas are emitted as vector
//! paths from the same RaTeX display list the app paints: glyph outlines, fraction rules and
//! stretchy-delimiter shapes become ordinary PDF path operators, so PDF math is resolution
//! independent and needs no font embedding.

use crate::markdown::{self, Block};
use crate::math::{self, MathOptions, MathScene, MathStyle};

const PAGE_WIDTH: f32 = 612.0;
const PAGE_HEIGHT: f32 = 792.0;
const MARGIN: f32 = 72.0;
const LEADING: f32 = 14.0;
const LINES_PER_PAGE: usize = 46;
/// Largest size a display formula is set at on the page.
const MAX_MATH_SIZE: f32 = 18.0;
/// Font size for formulas set inside prose.
const INLINE_MATH_SIZE: f32 = 10.5;
/// Rough width of a Helvetica character at 11pt, for wrapping.
const CHAR_WIDTH: f32 = 5.4;
const TEXT_WIDTH: f32 = PAGE_WIDTH - MARGIN * 2.0;

/// One laid-out line on the page grid.
enum Line {
    Text {
        text: String,
        size: f32,
    },
    /// Prose with inline math, laid out as positioned runs.
    Rich {
        runs: Vec<Run>,
        size: f32,
    },
    /// A display formula, occupying `rows` grid rows.
    Math {
        scene: MathScene,
        rows: usize,
    },
    Blank,
}

enum Run {
    Text(String),
    Math(MathScene),
}

impl Line {
    fn rows(&self) -> usize {
        match self {
            Line::Math { rows, .. } => *rows,
            _ => 1,
        }
    }
}

/// Render markdown into a printable PDF.
pub fn from_markdown(src: &str, title: &str) -> String {
    let lines = layout(src, title);

    let mut pages: Vec<Vec<Line>> = Vec::new();
    let mut page: Vec<Line> = Vec::new();
    let mut used = 0usize;

    for line in lines {
        let rows = line.rows();
        if used + rows > LINES_PER_PAGE && !page.is_empty() {
            pages.push(std::mem::take(&mut page));
            used = 0;
        }
        used += rows;
        page.push(line);
    }
    if !page.is_empty() || pages.is_empty() {
        pages.push(page);
    }

    build(&pages)
}

/// Flatten the document into page-grid lines.
fn layout(src: &str, title: &str) -> Vec<Line> {
    let mut lines: Vec<Line> = Vec::new();

    if !title.trim().is_empty() {
        lines.push(Line::Text {
            text: title.to_string(),
            size: 20.0,
        });
        lines.push(Line::Blank);
    }

    for block in markdown::parse_blocks(src) {
        match block {
            Block::Heading { level, text } => {
                let size = match level {
                    1 => 18.0,
                    2 => 15.0,
                    3 => 13.0,
                    _ => 11.5,
                };
                lines.push(Line::Blank);
                push_rich(&mut lines, &text, size, "");
            }
            Block::Paragraph(text) => {
                push_rich(&mut lines, &text, 11.0, "");
                lines.push(Line::Blank);
            }
            Block::ListItem { depth, text, .. } => {
                let indent = "  ".repeat(depth);
                push_rich(&mut lines, &text, 11.0, &format!("{indent}- "));
            }
            Block::Code { text, .. } => {
                for line in text.lines() {
                    lines.push(Line::Text {
                        text: format!("  {line}"),
                        size: 9.0,
                    });
                }
                lines.push(Line::Blank);
            }
            Block::Quote(text) => {
                push_rich(&mut lines, &text, 11.0, "| ");
            }
            Block::Callout { title, text, .. } => {
                push_rich(&mut lines, &format!("! {title}"), 11.5, "");
                if !text.trim().is_empty() {
                    push_rich(&mut lines, &text, 11.0, "  ");
                }
                lines.push(Line::Blank);
            }
            Block::Rule => lines.push(Line::Text {
                text: "-".repeat(60),
                size: 11.0,
            }),
            Block::Math { tex } => {
                lines.push(Line::Blank);
                match math::typeset(
                    &tex,
                    &MathOptions {
                        style: MathStyle::Display,
                        color: [0.0, 0.0, 0.0, 1.0],
                    },
                ) {
                    Ok(scene) => {
                        let height = scene.total_height() as f32 * MAX_MATH_SIZE;
                        let rows = ((height / LEADING).ceil() as usize)
                            .clamp(1, LINES_PER_PAGE.saturating_sub(2));
                        lines.push(Line::Math { scene, rows });
                    }
                    // A formula we cannot typeset still has to print something readable.
                    Err(_) => {
                        for line in crate::math::to_unicode(&tex).lines() {
                            lines.push(Line::Text {
                                text: format!("    {line}"),
                                size: 11.5,
                            });
                        }
                    }
                }
                lines.push(Line::Blank);
            }
            Block::Embed { target } => lines.push(Line::Text {
                text: format!("[embed: {target}]"),
                size: 10.0,
            }),
            Block::Footnote { id, text } => {
                push_wrapped(&mut lines, &printable(&text), 9.5, &format!("[{id}] "));
            }
            Block::Table { headers, rows, .. } => {
                let headers: Vec<String> = headers.iter().map(|cell| printable(cell)).collect();
                push_wrapped(&mut lines, &headers.join(" | "), 10.5, "");
                lines.push(Line::Text {
                    text: "-".repeat(60),
                    size: 9.0,
                });
                for row in rows {
                    let row: Vec<String> = row.iter().map(|cell| printable(cell)).collect();
                    push_wrapped(&mut lines, &row.join(" | "), 10.0, "");
                }
                lines.push(Line::Blank);
            }
        }
    }

    lines
}

/// Wrap inline Markdown for print: words stay as text runs, formulas become vector scenes.
/// Approximate advance width of Helvetica text at `size` points.
///
/// The PDF text runs are positioned individually (math must be drawn between them), so the
/// estimate has to be close to the real font metrics or the gaps show. These are the standard
/// Helvetica AFM widths; anything outside ASCII falls back to the digit width.
fn helvetica_width(text: &str, size: f32) -> f32 {
    let units: u32 = text.chars().map(|ch| helvetica_unit(ch) as u32).sum();
    units as f32 * size / 1000.0
}

fn helvetica_unit(ch: char) -> u16 {
    match ch {
        ' ' => 278,
        '!' | ',' | '.' | '/' | ':' | ';' | '[' | ']' | '\\' => 278,
        '"' => 355,
        '#' | '$' | '0'..='9' | '=' | '<' | '>' | '_' => 556,
        '%' => 889,
        '&' => 667,
        '\'' => 191,
        '(' | ')' | '-' | 'r' => 333,
        '*' => 389,
        '+' => 584,
        '@' => 1015,
        'A' | 'B' | 'E' | 'K' | 'P' | 'S' | 'V' | 'X' | 'Y' => 667,
        'C' | 'D' | 'H' | 'N' | 'R' | 'U' | 'w' => 722,
        'F' | 'T' | 'Z' => 611,
        'G' | 'O' | 'Q' => 778,
        'I' => 278,
        'J' => 500,
        'L' => 556,
        'M' => 833,
        'W' => 944,
        'i' | 'j' | 'l' => 222,
        'a' | 'b' | 'd' | 'e' | 'g' | 'h' | 'n' | 'o' | 'p' | 'q' | 'u' => 556,
        'c' | 'k' | 's' | 'v' | 'x' | 'y' | 'z' => 500,
        'f' | 't' => 278,
        'm' => 833,
        '{' | '}' => 334,
        '|' => 260,
        '~' => 584,
        _ => 556,
    }
}

fn push_rich(lines: &mut Vec<Line>, text: &str, size: f32, prefix: &str) {
    let mut runs: Vec<Run> = Vec::new();
    let mut x = 0.0f32;

    if !prefix.is_empty() {
        x += helvetica_width(prefix, size);
        runs.push(Run::Text(prefix.to_string()));
    }

    for span in crate::inline::parse(text) {
        if let Some(tex) = span.math.as_deref() {
            if let Ok(scene) = math::typeset(
                tex,
                &MathOptions {
                    style: MathStyle::Inline,
                    color: [0.0, 0.0, 0.0, 1.0],
                },
            ) {
                let width = scene.width() as f32 * INLINE_MATH_SIZE;
                if x + width > TEXT_WIDTH && !runs.is_empty() {
                    lines.push(Line::Rich {
                        runs: std::mem::take(&mut runs),
                        size,
                    });
                    x = 0.0;
                }
                x += width;
                runs.push(Run::Math(scene));
                continue;
            }
        }

        for word in span.text.split_inclusive(' ') {
            if word.is_empty() {
                continue;
            }
            let width = helvetica_width(word, size);
            if x + width > TEXT_WIDTH && !runs.is_empty() && !word.trim().is_empty() {
                lines.push(Line::Rich {
                    runs: std::mem::take(&mut runs),
                    size,
                });
                x = 0.0;
                if word.trim().is_empty() {
                    continue;
                }
            }
            x += width;
            runs.push(Run::Text(word.to_string()));
        }
    }

    if !runs.is_empty() {
        lines.push(Line::Rich { runs, size });
    }
}

/// Strip inline Markdown for print. Math keeps its TeX source for now: inline PDF math is
/// not vectorized yet, and the source is more useful than a Unicode approximation that the
/// base-14 font would turn into `?`.
fn printable(text: &str) -> String {
    crate::inline::parse(text)
        .into_iter()
        .map(|span| match span.math {
            Some(tex) => format!("${tex}$"),
            None => span.text,
        })
        .collect::<String>()
        .replace('\n', " ")
}

fn push_wrapped(lines: &mut Vec<Line>, text: &str, size: f32, prefix: &str) {
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() {
        lines.push(Line::Text {
            text: prefix.trim_end().to_string(),
            size,
        });
        return;
    }

    let wrap = crate::wrap::wrap(
        &chars,
        |from, to| (to - from) as f32 * CHAR_WIDTH,
        TEXT_WIDTH,
    );

    for (index, (start, end)) in wrap.into_iter().enumerate() {
        let segment: String = chars[start..end].iter().collect();
        let text = if index == 0 {
            format!("{prefix}{segment}")
        } else {
            format!("{}{segment}", " ".repeat(prefix.chars().count()))
        };
        lines.push(Line::Text { text, size });
    }
}

fn build(pages: &[Vec<Line>]) -> String {
    let mut objects: Vec<String> = Vec::new();

    // 1: catalog, 2: pages tree, 3: font, then two objects per page.
    let page_count = pages.len();
    let kids: Vec<String> = (0..page_count)
        .map(|index| format!("{} 0 R", 4 + index * 2))
        .collect();

    objects.push("<< /Type /Catalog /Pages 2 0 R >>".to_string());
    objects.push(format!(
        "<< /Type /Pages /Kids [{}] /Count {} >>",
        kids.join(" "),
        page_count
    ));
    objects.push("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string());

    for (index, page) in pages.iter().enumerate() {
        let content_id = 5 + index * 2;
        objects.push(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {PAGE_WIDTH} {PAGE_HEIGHT}] \
             /Resources << /Font << /F1 3 0 R >> >> /Contents {content_id} 0 R >>"
        ));

        let stream = content_stream(page);
        objects.push(format!(
            "<< /Length {} >>\nstream\n{stream}endstream",
            stream.len()
        ));
    }

    let mut out = String::from("%PDF-1.4\n");
    let mut offsets = Vec::with_capacity(objects.len());

    for (index, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.push_str(&format!("{} 0 obj\n{body}\nendobj\n", index + 1));
    }

    let xref_offset = out.len();
    out.push_str(&format!("xref\n0 {}\n", objects.len() + 1));
    out.push_str("0000000000 65535 f \n");
    for offset in &offsets {
        out.push_str(&format!("{offset:010} 00000 n \n"));
    }
    out.push_str(&format!(
        "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_offset}\n%%EOF\n",
        objects.len() + 1
    ));

    out
}

fn content_stream(lines: &[Line]) -> String {
    let mut out = String::new();
    let top = PAGE_HEIGHT - MARGIN;
    let mut row = 0usize;

    for line in lines {
        match line {
            Line::Text { text, size } => {
                let y = top - row as f32 * LEADING;
                out.push_str(&format!(
                    "BT /F1 {size} Tf 1 0 0 1 {MARGIN} {y} Tm ({}) Tj ET\n",
                    escape(text)
                ));
                row += 1;
            }
            Line::Blank => row += 1,
            Line::Rich { runs, size } => {
                let y = top - row as f32 * LEADING;
                let mut x = MARGIN;
                for run in runs {
                    match run {
                        Run::Text(text) => {
                            if !text.is_empty() {
                                out.push_str(&format!(
                                    "BT /F1 {size} Tf 1 0 0 1 {x} {y} Tm ({}) Tj ET\n",
                                    escape(text)
                                ));
                                x += helvetica_width(text, *size);
                            }
                        }
                        Run::Math(scene) => {
                            draw_scene(&mut out, scene, x, y, INLINE_MATH_SIZE);
                            x += scene.width() as f32 * INLINE_MATH_SIZE;
                        }
                    }
                }
                row += 1;
            }
            Line::Math { scene, rows } => {
                draw_math(&mut out, scene, row, *rows);
                row += rows;
            }
        }
    }

    out
}

/// Place and paint one display formula inside its grid rows.
fn draw_math(out: &mut String, scene: &MathScene, row: usize, rows: usize) {
    let width_em = scene.width() as f32;
    let height_em = scene.height() as f32;
    let total_em = scene.total_height() as f32;
    if width_em <= 0.0 || total_em <= 0.0 {
        return;
    }

    let available_width = TEXT_WIDTH;
    let available_height = rows as f32 * LEADING - 2.0;
    let size = MAX_MATH_SIZE
        .min(available_width / width_em)
        .min(available_height / total_em)
        .max(4.0);

    let width = width_em * size;
    let height = height_em * size;
    let origin_x = MARGIN + (TEXT_WIDTH - width) / 2.0;
    let top = PAGE_HEIGHT - MARGIN - row as f32 * LEADING;
    let block_height = rows as f32 * LEADING;
    let content_top = top - (block_height - total_em * size) / 2.0;
    let baseline = content_top - height;

    draw_scene(out, scene, origin_x, baseline, size);
}

/// Paint a formula at a known baseline (used for both display and inline math).
fn draw_scene(out: &mut String, scene: &MathScene, origin_x: f32, baseline: f32, size: f32) {
    let height_em = scene.height() as f32;
    for item in &scene.display.items {
        match item {
            math::DisplayItem::GlyphPath {
                x,
                y,
                scale,
                font,
                char_code,
                color,
            } => {
                let Some(outline) = math::glyph_outline(font, *char_code) else {
                    continue;
                };
                if outline.units_per_em <= 0.0 {
                    continue;
                }
                let k = size * *scale as f32 / outline.units_per_em;
                let glyph_x = origin_x + *x as f32 * size;
                let glyph_y = baseline + (height_em - *y as f32) * size;
                draw_glyph(out, &outline, glyph_x, glyph_y, k, *color);
            }
            math::DisplayItem::Line {
                x,
                y,
                width,
                thickness,
                color,
                dashed,
            } => {
                set_fill_color(out, color);
                let left = origin_x + *x as f32 * size;
                let center = baseline + (height_em - *y as f32) * size;
                let t = (*thickness as f32 * size).max(0.4);
                if *dashed {
                    let dash = (4.0 * t).max(1.5);
                    let mut cursor = 0.0f32;
                    let total = *width as f32 * size;
                    while cursor < total {
                        let segment = dash.min(total - cursor);
                        out.push_str(&format!(
                            "{} {} {} {} re f\n",
                            num(left + cursor),
                            num(center - t / 2.0),
                            num(segment),
                            num(t)
                        ));
                        cursor += dash * 2.0;
                    }
                } else {
                    let w = (*width as f32 * size).max(0.1);
                    out.push_str(&format!(
                        "{} {} {} {} re f\n",
                        num(left),
                        num(center - t / 2.0),
                        num(w),
                        num(t)
                    ));
                }
            }
            math::DisplayItem::Rect {
                x,
                y,
                width,
                height,
                color,
            } => {
                set_fill_color(out, color);
                let left = origin_x + *x as f32 * size;
                let top = baseline + (height_em - *y as f32) * size;
                out.push_str(&format!(
                    "{} {} {} {} re f\n",
                    num(left),
                    num(top - *height as f32 * size),
                    num((*width as f32 * size).max(0.1)),
                    num((*height as f32 * size).max(0.1))
                ));
            }
            math::DisplayItem::Path {
                x,
                y,
                commands,
                fill,
                color,
            } => {
                let base_x = origin_x + *x as f32 * size;
                let base_y = baseline + (height_em - *y as f32) * size;
                set_fill_color(out, color);
                if *fill {
                    let mut cursor = (base_x, base_y);
                    for command in commands {
                        path_command(out, command, base_x, base_y, size, &mut cursor);
                    }
                    // Even-odd matches RaTeX's renderer for assembled delimiter shapes.
                    out.push_str("f*\n");
                } else {
                    out.push_str("1 w\n");
                    let mut cursor = (base_x, base_y);
                    for command in commands {
                        path_command(out, command, base_x, base_y, size, &mut cursor);
                    }
                    out.push_str("S\n");
                }
            }
        }
    }
}

fn draw_glyph(
    out: &mut String,
    outline: &math::GlyphOutline,
    origin_x: f32,
    origin_y: f32,
    scale: f32,
    color: math::Color,
) {
    set_fill_color(out, &color);
    let mut last: Option<(f32, f32)> = None;
    let mut started = false;

    let point = |x: f32, y: f32| (origin_x + x * scale, origin_y + y * scale);

    for curve in &outline.curves {
        let (start, end) = match curve {
            math::GlyphCurve::Line(a, b) => (a, b),
            math::GlyphCurve::Quad(a, _, c) => (a, c),
            math::GlyphCurve::Cubic(a, _, _, d) => (a, d),
        };
        let (sx, sy) = point(start.x, start.y);
        let needs_move = match last {
            None => true,
            Some((lx, ly)) => (lx - sx).abs() > 0.01 || (ly - sy).abs() > 0.01,
        };
        if needs_move {
            if started {
                out.push_str("h\n");
            }
            out.push_str(&format!("{} {} m\n", num(sx), num(sy)));
            started = true;
        }

        match curve {
            math::GlyphCurve::Line(_, b) => {
                let (x, y) = point(b.x, b.y);
                out.push_str(&format!("{} {} l\n", num(x), num(y)));
            }
            math::GlyphCurve::Quad(_, c, b) => {
                let (x, y) = point(b.x, b.y);
                let (cx, cy) = point(c.x, c.y);
                let (sx, sy) = point(start.x, start.y);
                let c1 = (sx + 2.0 / 3.0 * (cx - sx), sy + 2.0 / 3.0 * (cy - sy));
                let c2 = (x + 2.0 / 3.0 * (cx - x), y + 2.0 / 3.0 * (cy - y));
                out.push_str(&format!(
                    "{} {} {} {} {} {} c\n",
                    num(c1.0),
                    num(c1.1),
                    num(c2.0),
                    num(c2.1),
                    num(x),
                    num(y)
                ));
            }
            math::GlyphCurve::Cubic(_, c1, c2, b) => {
                let (x, y) = point(b.x, b.y);
                let (x1, y1) = point(c1.x, c1.y);
                let (x2, y2) = point(c2.x, c2.y);
                out.push_str(&format!(
                    "{} {} {} {} {} {} c\n",
                    num(x1),
                    num(y1),
                    num(x2),
                    num(y2),
                    num(x),
                    num(y)
                ));
            }
        }
        last = Some(point(end.x, end.y));
    }

    if started {
        out.push_str("h\nf\n");
    }
}

fn path_command(
    out: &mut String,
    command: &math::PathCommand,
    base_x: f32,
    base_y: f32,
    size: f32,
    cursor: &mut (f32, f32),
) {
    match command {
        math::PathCommand::MoveTo { x, y } => {
            let point = (base_x + *x as f32 * size, base_y - *y as f32 * size);
            *cursor = point;
            out.push_str(&format!("{} {} m\n", num(point.0), num(point.1)));
        }
        math::PathCommand::LineTo { x, y } => {
            let point = (base_x + *x as f32 * size, base_y - *y as f32 * size);
            *cursor = point;
            out.push_str(&format!("{} {} l\n", num(point.0), num(point.1)));
        }
        math::PathCommand::QuadTo { x1, y1, x, y } => {
            let (sx, sy) = *cursor;
            let (cx, cy) = (base_x + *x1 as f32 * size, base_y - *y1 as f32 * size);
            let (ex, ey) = (base_x + *x as f32 * size, base_y - *y as f32 * size);
            // PDF only has cubic curves: elevate the quadratic to a cubic.
            let c1 = (sx + 2.0 / 3.0 * (cx - sx), sy + 2.0 / 3.0 * (cy - sy));
            let c2 = (ex + 2.0 / 3.0 * (cx - ex), ey + 2.0 / 3.0 * (cy - ey));
            out.push_str(&format!(
                "{} {} {} {} {} {} c\n",
                num(c1.0),
                num(c1.1),
                num(c2.0),
                num(c2.1),
                num(ex),
                num(ey)
            ));
            *cursor = (ex, ey);
        }
        math::PathCommand::CubicTo {
            x1,
            y1,
            x2,
            y2,
            x,
            y,
        } => {
            let c1 = (base_x + *x1 as f32 * size, base_y - *y1 as f32 * size);
            let c2 = (base_x + *x2 as f32 * size, base_y - *y2 as f32 * size);
            let end = (base_x + *x as f32 * size, base_y - *y as f32 * size);
            out.push_str(&format!(
                "{} {} {} {} {} {} c\n",
                num(c1.0),
                num(c1.1),
                num(c2.0),
                num(c2.1),
                num(end.0),
                num(end.1)
            ));
            *cursor = end;
        }
        math::PathCommand::Close => out.push_str("h\n"),
    }
}

fn set_fill_color(out: &mut String, color: &math::Color) {
    out.push_str(&format!(
        "{} {} {} rg\n",
        num(color.r.clamp(0.0, 1.0)),
        num(color.g.clamp(0.0, 1.0)),
        num(color.b.clamp(0.0, 1.0))
    ));
}

/// PDF wants plain decimal numbers; three decimals is plenty at print sizes.
fn num(value: f32) -> String {
    let mut text = format!("{value:.3}");
    if text.contains('.') {
        while text.ends_with('0') {
            text.pop();
        }
        if text.ends_with('.') {
            text.pop();
        }
    }
    if text == "-0" {
        text = "0".to_string();
    }
    text
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '(' => out.push_str("\\("),
            ')' => out.push_str("\\)"),
            '\\' => out.push_str("\\\\"),
            '\u{2013}' | '\u{2014}' | '\u{2212}' => out.push('-'),
            '\u{2018}' | '\u{2019}' => out.push('\''),
            '\u{201c}' | '\u{201d}' => out.push('"'),
            '\u{2026}' => out.push_str("..."),
            '\u{2192}' => out.push_str("->"),
            c if c.is_ascii() => out.push(c),
            _ => out.push('?'),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn xref_offset(pdf: &str) -> usize {
        let marker = "startxref\n";
        let start = pdf.find(marker).unwrap() + marker.len();
        let end = pdf[start..].find('\n').unwrap() + start;
        pdf[start..end].trim().parse().unwrap()
    }

    #[test]
    fn writes_a_valid_looking_pdf() {
        let pdf = from_markdown("# Title\n\nHello world.\n", "My Note");

        assert!(pdf.starts_with("%PDF-1.4\n"));
        assert!(pdf.ends_with("%%EOF\n"));
        assert!(pdf.contains("My Note"));
        assert!(pdf.contains("(Hello ) Tj"));
        assert!(pdf.contains("(world.) Tj"));
        assert!(pdf.contains("/Type /Catalog"));
        assert!(pdf.contains("/BaseFont /Helvetica"));

        // The xref offset must point at the `xref` keyword.
        let offset = xref_offset(&pdf);
        assert_eq!(&pdf[offset..offset + 4], "xref");
    }

    #[test]
    fn escapes_pdf_string_delimiters() {
        let pdf = from_markdown("`a (b) \\ d`\n", "");
        assert!(pdf.contains("\\(b\\)"));
        assert!(pdf.contains("\\\\"));
    }

    #[test]
    fn paginates_long_documents() {
        let source = (0..200)
            .map(|index| format!("line {index}"))
            .collect::<Vec<_>>()
            .join("\n\n");

        let pdf = from_markdown(&source, "");

        let marker = "/Count ";
        let start = pdf.find(marker).unwrap() + marker.len();
        let end = pdf[start..].find(' ').unwrap() + start;
        let count: usize = pdf[start..end].parse().unwrap();

        assert!(count > 1, "expected multiple pages, got {count}");
        assert_eq!(pdf.matches("/Type /Page ").count(), count);
    }

    #[test]
    fn non_ascii_becomes_question_marks() {
        let pdf = from_markdown("caf\u{e9}\n", "");
        assert!(pdf.contains("(caf?) Tj"));
    }

    #[test]
    fn strips_inline_markdown_and_vectorizes_inline_math() {
        let pdf = from_markdown("**bold** [link](https://example.com) and $x^2$\n", "");
        assert!(pdf.contains("(bold) Tj"));
        assert!(pdf.contains("(link) Tj"));
        assert!(pdf.contains("(and ) Tj"));
        assert!(pdf.contains(" m\n"), "expected inline math glyph path");
        assert!(!pdf.contains("$x^2$"));
        assert!(!pdf.contains("[link]"));
    }

    #[test]
    fn display_math_is_written_as_vector_paths() {
        let pdf = from_markdown("$$\\frac{a}{b}$$\n", "");

        // Glyph outlines and the fraction bar are path operators, not text.
        assert!(pdf.contains(" m\n"), "expected moveto in math path");
        assert!(pdf.contains(" c\n"), "expected curve in glyph outline");
        assert!(pdf.contains(" re f\n"), "expected fraction rule rectangle");
        assert!(pdf.contains("f\n"), "expected a filled glyph outline");
        // The transliteration must not leak into the text stream.
        assert!(!pdf.contains("(a/b) Tj"));
    }

    #[test]
    fn display_math_uses_extra_rows_and_keeps_the_page_valid() {
        let pdf = from_markdown(
            "$$\\begin{aligned} a &= b \\\\ c &= d \\\\ e &= f \\end{aligned}$$\n",
            "",
        );
        let offset = xref_offset(&pdf);
        assert_eq!(&pdf[offset..offset + 4], "xref");
        assert!(pdf.contains(" m\n"));
    }
}
