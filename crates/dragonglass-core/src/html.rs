//! Render notes to standalone HTML, for `dragon-glass export`.
//!
//! A small block-level renderer rather than a full CommonMark engine: it mirrors the same
//! block model the reading view uses, so exported output matches what the app shows.

use crate::assets::Attachments;
use crate::inline::{self, Inline};
use crate::markdown::{self, Block};
use crate::math;

/// Minimal CSS for exported pages.
pub const STYLE: &str = "\
body{font-family:system-ui,sans-serif;line-height:1.6;max-width:46rem;margin:2rem auto;padding:0 1rem;color:#1f1f1f}
h1,h2,h3,h4,h5,h6{line-height:1.25}
pre{background:#f4f4f4;padding:.75rem;border-radius:6px;overflow:auto}
blockquote{border-left:3px solid #d0d0d0;margin-left:0;padding-left:1rem;color:#555}
hr{border:0;border-top:1px solid #d0d0d0}
a{color:#1155cc}
mark{background:#fff2a8;padding:0 .1em}
code{background:#f4f4f4;padding:.1em .25em;border-radius:3px;font-size:.92em}
del{opacity:.75}
.math{background:#f4f4f4;padding:.1em .3em;border-radius:3px;font-family:ui-monospace,monospace}
.math-inline{background:none;padding:0;font-family:inherit}
.math-inline svg{vertical-align:middle}
.math-display{background:none;padding:0;font-family:inherit;display:block;text-align:center;margin:1rem 0}
.math-display svg{max-width:100%;height:auto}
table{border-collapse:collapse;width:100%;margin:1rem 0}
th,td{border:1px solid #d0d0d0;padding:.4rem .6rem;text-align:left}
th{background:#f4f4f4}
li.task{list-style:none;margin-left:-1.2em}
img{max-width:100%;height:auto}
.footnote{font-size:.9em;color:#555}
.fn{color:#1155cc;font-weight:600}
";

/// HTML-escape text.
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

/// Render the body of a document (no `<html>` wrapper).
pub fn render_body(src: &str) -> String {
    render_body_with(src, None, &mut Vec::new())
}

/// Render the body, resolving attachment embeds through `attachments`.
///
/// Vault-relative paths of the files the output points at are appended to `used`, so the
/// caller can copy them next to the exported HTML.
pub fn render_body_with(
    src: &str,
    attachments: Option<&Attachments>,
    used: &mut Vec<String>,
) -> String {
    let blocks = markdown::parse_blocks(src);
    let mut out = String::new();
    let mut index = 0;

    while index < blocks.len() {
        // Consecutive list items of the same kind share one list, the way the source wrote
        // them.
        if let Block::ListItem {
            ordered: list_ordered,
            ..
        } = &blocks[index]
        {
            let list_ordered = *list_ordered;
            let tag = if list_ordered { "ol" } else { "ul" };
            out.push_str(&format!("<{tag}>\n"));

            while index < blocks.len() {
                let Block::ListItem {
                    ordered,
                    depth,
                    marker: _,
                    checked,
                    text,
                } = blocks[index].clone()
                else {
                    break;
                };
                if ordered != list_ordered {
                    break;
                }

                let mut item = String::from("<li");
                if checked.is_some() {
                    item.push_str(" class=\"task\"");
                }
                if depth > 0 {
                    item.push_str(&format!(" style=\"margin-left:{}rem\"", depth as f32 * 1.2));
                }
                item.push('>');
                match checked {
                    Some(true) => item.push_str("<input type=\"checkbox\" checked disabled> "),
                    Some(false) => item.push_str("<input type=\"checkbox\" disabled> "),
                    None => {}
                }
                item.push_str(&inline(&text));
                item.push_str("</li>");
                out.push_str(&item);
                out.push('\n');
                index += 1;
            }

            out.push_str(&format!("</{tag}>\n"));
            continue;
        }

        out.push_str(&render_block(blocks[index].clone(), attachments, used));
        out.push('\n');
        index += 1;
    }

    out
}

fn render_block(block: Block, attachments: Option<&Attachments>, used: &mut Vec<String>) -> String {
    match block {
        Block::Heading { level, text } => {
            format!("<h{level}>{}</h{level}>", inline(&text))
        }
        Block::Paragraph(text) => format!("<p>{}</p>", inline(&text)),
        Block::ListItem {
            ordered,
            depth,
            marker: _,
            checked,
            text,
        } => {
            let tag = if ordered { "ol" } else { "ul" };
            let _ = depth;
            let checkbox = match checked {
                Some(true) => "<input type=\"checkbox\" checked disabled> ",
                Some(false) => "<input type=\"checkbox\" disabled> ",
                None => "",
            };
            format!("<{tag}><li>{checkbox}{}</li></{tag}>", inline(&text))
        }
        Block::Code { text, .. } => format!("<pre><code>{}</code></pre>", escape(&text)),
        Block::Quote(text) => format!("<blockquote>{}</blockquote>", inline(&text)),
        Block::Rule => "<hr>".to_string(),
        Block::Math { tex } => math_block_html(&tex),
        Block::Footnote { id, text } => format!(
            "<p class=\"footnote\"><sup>{}</sup> {}</p>",
            escape(&id),
            inline(&text)
        ),
        Block::Callout { kind, title, text } => {
            let body = if text.trim().is_empty() {
                String::new()
            } else {
                format!("<p>{}</p>", inline(&text))
            };
            format!(
                "<blockquote class=\"callout callout-{}\"><p><strong>{}</strong></p>{}</blockquote>",
                escape(&kind),
                escape(&title),
                body
            )
        }
        Block::Embed {
            target, width, alt, ..
        } => {
            if is_image(&target) {
                let resolved = attachments.and_then(|index| index.resolve(&target));
                let source = resolved.unwrap_or(target.as_str());
                if resolved.is_some() && !used.iter().any(|file| file == source) {
                    used.push(source.to_string());
                }
                let alt = alt.as_deref().unwrap_or(&target);
                let width = width
                    .map(|width| format!(" width=\"{width}\""))
                    .unwrap_or_default();
                format!(
                    "<p><img src=\"{}\" alt=\"{}\"{}></p>",
                    escape(source),
                    escape(alt),
                    width
                )
            } else {
                format!(
                    "<p><a href=\"{}.html\">[[{}]]</a></p>",
                    escape(&target),
                    escape(&target)
                )
            }
        }
        Block::Table {
            align,
            headers,
            rows,
        } => {
            let mut html = String::from("<table><thead><tr>");
            for (index, header) in headers.iter().enumerate() {
                let alignment = alignment_style(align.get(index));
                html.push_str(&format!("<th{alignment}>{}</th>", inline(header)));
            }
            html.push_str("</tr></thead><tbody>");
            for row in rows {
                html.push_str("<tr>");
                for (index, cell) in row.iter().enumerate() {
                    let alignment = alignment_style(align.get(index));
                    html.push_str(&format!("<td{alignment}>{}</td>", inline(cell)));
                }
                html.push_str("</tr>");
            }
            html.push_str("</tbody></table>");
            html
        }
    }
}

fn alignment_style(align: Option<&markdown::Align>) -> &'static str {
    match align {
        Some(markdown::Align::Center) => " style=\"text-align:center\"",
        Some(markdown::Align::Right) => " style=\"text-align:right\"",
        _ => "",
    }
}

/// Turn inline markdown into HTML: emphasis, code, links, highlights and math.
fn inline(text: &str) -> String {
    inline::parse(text).into_iter().map(render_span).collect()
}

/// Typeset `tex` and serialize it as a self-contained SVG (glyph outlines embedded).
/// Returns the SVG and the formula's depth in em, which inline HTML needs for baseline
/// alignment.
fn math_svg(tex: &str, style: math::MathStyle) -> Option<(String, f64)> {
    let scene = math::typeset(
        tex,
        &math::MathOptions {
            style,
            color: [0.0, 0.0, 0.0, 1.0],
        },
    )
    .ok()?;

    let options = ratex_svg::SvgOptions {
        font_size: if style == math::MathStyle::Display {
            18.0
        } else {
            16.0
        },
        padding: 0.0,
        stroke_width: 1.0,
        embed_glyphs: true,
        font_dir: String::new(),
    };

    Some((
        ratex_svg::render_to_svg(&scene.display, &options),
        scene.depth(),
    ))
}

fn math_block_html(tex: &str) -> String {
    match math_svg(tex, math::MathStyle::Display) {
        Some((svg, _)) => format!(
            "<div class=\"math math-display\" data-tex=\"{}\">{svg}</div>",
            escape(tex)
        ),
        None => format!("<p class=\"math\">{}</p>", escape(&math::to_unicode(tex))),
    }
}

fn math_inline_html(tex: &str) -> Option<String> {
    let (svg, depth) = math_svg(tex, math::MathStyle::Inline)?;
    Some(format!(
        "<span class=\"math math-inline\" data-tex=\"{}\" style=\"display:inline-block;vertical-align:-{depth:.3}em\">{svg}</span>",
        escape(tex)
    ))
}

fn render_span(span: Inline) -> String {
    let style = span.style;
    if style.footnote {
        return format!("<sup class=\"fn\">{}</sup>", escape(&span.text));
    }
    let mut html = if style.code {
        format!("<code>{}</code>", escape(&span.text))
    } else if style.math {
        match span.math.as_deref() {
            Some(tex) => math_inline_html(tex)
                .unwrap_or_else(|| format!("<span class=\"math\">{}</span>", escape(&span.text))),
            None => format!("<span class=\"math\">{}</span>", escape(&span.text)),
        }
    } else {
        escape(&span.text).replace('\n', "<br>")
    };

    if style.highlight {
        html = format!("<mark>{html}</mark>");
    }
    if style.strikethrough {
        html = format!("<del>{html}</del>");
    }
    if style.italic {
        html = format!("<em>{html}</em>");
    }
    if style.bold {
        html = format!("<strong>{html}</strong>");
    }
    if style.link {
        if let Some(href) = span.href {
            let href = if style.wiki {
                format!("{href}.html")
            } else {
                href
            };
            html = format!("<a href=\"{}\">{html}</a>", escape(&href));
        }
    }

    html
}

fn is_image(target: &str) -> bool {
    matches!(
        std::path::Path::new(target)
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .as_deref(),
        Some("png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "svg" | "avif" | "ico")
    )
}

/// Wrap rendered body in a complete HTML document.
pub fn document(title: &str, body: &str) -> String {
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\n\
         <title>{}</title>\n<style>{STYLE}</style>\n</head>\n<body>\n{body}\n</body>\n</html>\n",
        escape(title)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_html() {
        assert_eq!(
            escape("<a href=\"x\">&"),
            "&lt;a href=&quot;x&quot;&gt;&amp;"
        );
    }

    #[test]
    fn renders_headings_and_paragraphs() {
        let html = render_body("# Title\n\ntext <b>\n");
        assert!(html.contains("<h1>Title</h1>"));
        assert!(html.contains("<p>text &lt;b&gt;</p>"));
    }

    #[test]
    fn renders_code_verbatim_but_escaped() {
        let html = render_body("```\n<b>x</b>\n```\n");
        assert!(html.contains("<pre><code>&lt;b&gt;x&lt;/b&gt;\n</code></pre>"));
    }

    #[test]
    fn turns_wikilinks_into_anchors() {
        let html = render_body("see [[Other|Alias]] now");
        assert!(html.contains("<a href=\"Other.html\">Alias</a>"));
    }

    #[test]
    fn image_embed_becomes_img() {
        let html = render_body("![[shot.png]]\n");
        assert!(html.contains("<img src=\"shot.png\""));
    }

    #[test]
    fn an_image_embed_carries_width_and_alt() {
        let html = render_body("![[shot.png|320]]\n");
        assert!(html.contains("width=\"320\""), "{html}");
        assert!(html.contains("alt=\"shot.png\""), "{html}");

        let html = render_body("![A screenshot](shot.png)\n");
        assert!(html.contains("alt=\"A screenshot\""), "{html}");
    }

    #[test]
    fn bare_image_names_resolve_through_the_attachment_index() {
        let index = Attachments::from_paths(["attachments/shot.png".to_string()]);
        let mut used = Vec::new();
        let html = render_body_with("![[shot.png]]\n", Some(&index), &mut used);
        assert!(html.contains("<img src=\"attachments/shot.png\" alt=\"shot.png\">"));
        assert_eq!(used, vec!["attachments/shot.png".to_string()]);
    }

    #[test]
    fn unresolved_images_keep_their_target_and_are_not_collected() {
        let index = Attachments::from_paths(["attachments/shot.png".to_string()]);
        let mut used = Vec::new();
        let html = render_body_with("![[gone.png]]\n", Some(&index), &mut used);
        assert!(html.contains("<img src=\"gone.png\""));
        assert!(used.is_empty());
    }

    #[test]
    fn footnotes_render_as_superscripts_and_definitions() {
        let html = render_body("Text[^1]\n\n[^1]: Note.\n");
        assert!(html.contains("<sup class=\"fn\">1</sup>"));
        assert!(html.contains("class=\"footnote\""));
    }

    #[test]
    fn tables_render_with_headers_and_alignment() {
        let html = render_body("| Name | Value |\n| :--- | ---: |\n| a | 1 |\n");
        assert!(html.contains("<table>"));
        assert!(html.contains("<th>Name</th>"));
        assert!(html.contains("text-align:right"));
        assert!(html.contains("<td>a</td>"));
    }

    #[test]
    fn callouts_render_with_their_kind_and_title() {
        let html = render_body("> [!warning] Careful\n> mind the gap\n");
        assert!(html.contains("callout-warning"));
        assert!(html.contains("<strong>Careful</strong>"));
        assert!(html.contains("mind the gap"));
    }

    #[test]
    fn renders_inline_emphasis_code_and_strikethrough() {
        let html = render_body("**bold** *italic* `code` ~~gone~~ ==mark==\n");
        assert!(html.contains("<strong>bold</strong>"));
        assert!(html.contains("<em>italic</em>"));
        assert!(html.contains("<code>code</code>"));
        assert!(html.contains("<del>gone</del>"));
        assert!(html.contains("<mark>mark</mark>"));
    }

    #[test]
    fn renders_inline_math_and_links() {
        let html = render_body("see [docs](https://example.com) and $x^2$\n");
        assert!(html.contains("<a href=\"https://example.com\">docs</a>"));
        assert!(html.contains("class=\"math math-inline\""));
        assert!(html.contains("<svg"));
        assert!(!html.contains("x²"));
    }

    #[test]
    fn task_lists_render_checkboxes_in_one_list() {
        let html = render_body("- [ ] todo\n- [x] done\n");
        assert_eq!(html.matches("<ul>").count(), 1);
        assert!(html.contains("<input type=\"checkbox\" disabled> todo"));
        assert!(html.contains("<input type=\"checkbox\" checked disabled> done"));
    }

    #[test]
    fn ordered_and_unordered_lists_do_not_merge() {
        let html = render_body("- one\n1. first\n");
        assert!(html.contains("<ul>\n<li>one</li>\n</ul>"));
        assert!(html.contains("<ol>\n<li>first</li>\n</ol>"));
    }

    #[test]
    fn table_cells_render_inline_markup() {
        let html = render_body("| **a** | $x^2$ |\n| --- | --- |\n| *b* | c |\n");
        assert!(html.contains("<th><strong>a</strong></th>"));
        assert!(html.contains("<th><span class=\"math math-inline\""));
        assert!(html.contains("<svg"));
        assert!(html.contains("<td><em>b</em></td>"));
    }

    #[test]
    fn display_math_exports_as_standalone_svg() {
        let html = render_body("$$\\frac{a}{b}$$\n");
        assert!(html.contains("class=\"math math-display\""));
        assert!(html.contains("data-tex=\"\\frac{a}{b}\""));
        assert!(html.contains("<svg"));
    }

    #[test]
    fn document_wraps_and_titles() {
        let doc = document("My <Title>", "<p>x</p>");
        assert!(doc.starts_with("<!doctype html>"));
        assert!(doc.contains("<title>My &lt;Title&gt;</title>"));
    }
}
