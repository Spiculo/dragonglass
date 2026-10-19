//! Render notes to standalone HTML, for `dragon-glass export`.
//!
//! A small block-level renderer rather than a full CommonMark engine: it mirrors the same
//! block model the reading view uses, so exported output matches what the app shows.

use crate::markdown::{self, Block};
use crate::math::{self, Span};
use crate::wikilink;

/// Minimal CSS for exported pages.
pub const STYLE: &str = "\
body{font-family:system-ui,sans-serif;line-height:1.6;max-width:46rem;margin:2rem auto;padding:0 1rem;color:#1f1f1f}
h1,h2,h3{line-height:1.25}
pre{background:#f4f4f4;padding:.75rem;border-radius:6px;overflow:auto}
blockquote{border-left:3px solid #d0d0d0;margin-left:0;padding-left:1rem;color:#555}
hr{border:0;border-top:1px solid #d0d0d0}
a{color:#1155cc}
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
    let mut out = String::new();
    for block in markdown::parse_blocks(src) {
        out.push_str(&render_block(block));
        out.push('\n');
    }
    out
}

fn render_block(block: Block) -> String {
    match block {
        Block::Heading { level, text } => {
            format!("<h{level}>{}</h{level}>", inline(&text))
        }
        Block::Paragraph(text) => format!("<p>{}</p>", inline(&text.replace('\n', " "))),
        Block::ListItem {
            ordered,
            depth,
            marker: _,
            text,
        } => {
            let tag = if ordered { "ol" } else { "ul" };
            let _ = depth;
            format!("<{tag}><li>{}</li></{tag}>", inline(&text))
        }
        Block::Code { text, .. } => format!("<pre><code>{}</code></pre>", escape(&text)),
        Block::Quote(text) => format!(
            "<blockquote>{}</blockquote>",
            inline(&text.replace('\n', " "))
        ),
        Block::Rule => "<hr>".to_string(),
        Block::Math { tex } => format!("<p class=\"math\">{}</p>", escape(&math::to_unicode(&tex))),
        Block::Callout { kind, title, text } => {
            let body = if text.trim().is_empty() {
                String::new()
            } else {
                format!("<p>{}</p>", inline(&text.replace('\n', " ")))
            };
            format!(
                "<blockquote class=\"callout callout-{}\"><p><strong>{}</strong></p>{}</blockquote>",
                escape(&kind),
                escape(&title),
                body
            )
        }
        Block::Embed { target } => {
            if is_image(&target) {
                format!(
                    "<p><img src=\"{}\" alt=\"{}\"></p>",
                    escape(&target),
                    escape(&target)
                )
            } else {
                format!(
                    "<p><a href=\"{}.html\">[[{}]]</a></p>",
                    escape(&target),
                    escape(&target)
                )
            }
        }
    }
}

/// Turn `[[wikilinks]]` into links, highlights into `<mark>`, math into spans, and escape
/// everything else.
fn inline(text: &str) -> String {
    math::spans(text)
        .into_iter()
        .map(|span| match span {
            Span::Text(text) => linked(&text),
            Span::Highlight(text) => format!("<mark>{}</mark>", escape(&text)),
            Span::Math(text) => format!("<span class=\"math\">{}</span>", escape(&text)),
        })
        .collect()
}

fn linked(text: &str) -> String {
    if !text.contains("[[") {
        return escape(text);
    }

    let mut out = String::new();
    let mut last = 0;

    for link in wikilink::parse(text) {
        let Some(start) = text[last..].find("[[") else {
            break;
        };
        let start = last + start;
        let Some(end) = text[start..].find("]]") else {
            break;
        };
        let end = start + end + 2;

        out.push_str(&escape(&text[last..start]));

        let label = link.alias.clone().unwrap_or_else(|| link.target.clone());
        if link.target.is_empty() {
            out.push_str(&escape(&format!("[[{}]]", label)));
        } else {
            out.push_str(&format!(
                "<a href=\"{}.html\">{}</a>",
                escape(&link.target),
                escape(&label)
            ));
        }

        last = end;
    }

    out.push_str(&escape(&text[last..]));
    out
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
    fn callouts_render_with_their_kind_and_title() {
        let html = render_body("> [!warning] Careful\n> mind the gap\n");
        assert!(html.contains("callout-warning"));
        assert!(html.contains("<strong>Careful</strong>"));
        assert!(html.contains("mind the gap"));
    }

    #[test]
    fn document_wraps_and_titles() {
        let doc = document("My <Title>", "<p>x</p>");
        assert!(doc.starts_with("<!doctype html>"));
        assert!(doc.contains("<title>My &lt;Title&gt;</title>"));
    }
}
