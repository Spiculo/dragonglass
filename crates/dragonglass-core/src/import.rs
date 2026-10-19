//! Importers for other note tools, producing markdown notes.
//!
//! Supported so far: Evernote `.enex` exports and Roam JSON exports. Both funnel through
//! the small HTML-to-markdown converter below.

use crate::Result;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedNote {
    pub title: String,
    pub markdown: String,
}

/// Evernote `.enex` export: one `<note>` per entry, content as HTML inside CDATA.
pub fn evernote_enex(xml: &str) -> Vec<ImportedNote> {
    let mut notes = Vec::new();
    let mut rest = xml;

    while let Some(start) = rest.find("<note>") {
        let after = &rest[start + "<note>".len()..];
        let Some(end) = after.find("</note>") else {
            break;
        };
        let body = &after[..end];
        rest = &after[end + "</note>".len()..];

        let title = tag(body, "title")
            .map(|value| decode_entities(&value))
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "Untitled".to_string());

        let content = tag(body, "content").unwrap_or_default();
        let content = strip_cdata(&content);
        let markdown = html_to_markdown(&content);

        notes.push(ImportedNote {
            title: title.clone(),
            markdown: format!("# {title}\n\n{}", markdown.trim()),
        });
    }

    notes
}

/// Roam JSON export: `[{ "title": ..., "children": [{ "string": ..., "children": [...] }] }]`.
pub fn roam_pages(json: &str) -> Result<Vec<ImportedNote>> {
    let value: serde_json::Value = serde_json::from_str(json)?;
    let mut notes = Vec::new();

    let pages = match value {
        serde_json::Value::Array(pages) => pages,
        other => vec![other],
    };

    for page in pages {
        let title = page
            .get("title")
            .and_then(|value| value.as_str())
            .unwrap_or("Untitled")
            .to_string();

        let mut markdown = String::new();
        if let Some(children) = page.get("children").and_then(|value| value.as_array()) {
            render_roam_blocks(children, 0, &mut markdown);
        }

        notes.push(ImportedNote {
            title: title.clone(),
            markdown: format!("# {title}\n\n{}", markdown.trim()),
        });
    }

    Ok(notes)
}

fn render_roam_blocks(blocks: &[serde_json::Value], depth: usize, out: &mut String) {
    let indent = "  ".repeat(depth);

    for block in blocks {
        let text = block
            .get("string")
            .and_then(|value| value.as_str())
            .unwrap_or("")
            .trim();
        if !text.is_empty() {
            out.push_str(&format!("{indent}- {text}\n"));
        }
        if let Some(children) = block.get("children").and_then(|value| value.as_array()) {
            render_roam_blocks(children, depth + 1, out);
        }
    }
    if depth == 0 {
        out.push('\n');
    }
}

/// A deliberately small HTML-to-markdown converter covering the tags note exports use.
pub fn html_to_markdown(html: &str) -> String {
    let mut out = String::new();
    let mut rest = html;
    let mut links: Vec<String> = Vec::new();
    let mut in_pre = false;

    while let Some(open) = rest.find('<') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let Some(close) = after.find('>') else {
            out.push_str(&rest[open..]);
            break;
        };

        let tag_text = &after[..close];
        rest = &after[close + 1..];

        let lower = tag_text.trim().to_ascii_lowercase();
        let closing = lower.starts_with('/');
        let name: String = lower
            .trim_start_matches('/')
            .split_whitespace()
            .next()
            .unwrap_or("")
            .trim_end_matches('/')
            .to_string();

        match name.as_str() {
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                if !closing {
                    out.push_str("\n\n");
                    let level = name[1..].parse::<usize>().unwrap_or(1).min(6);
                    out.push_str(&"#".repeat(level));
                    out.push(' ');
                } else {
                    out.push('\n');
                }
            }
            "p" | "div" | "section" | "table" | "tr" => {
                if closing {
                    out.push_str("\n\n");
                }
            }
            "br" => out.push('\n'),
            "strong" | "b" => out.push_str("**"),
            "em" | "i" => out.push('*'),
            "code" => {
                if !in_pre {
                    out.push('`');
                }
            }
            "pre" => {
                in_pre = !closing;
                out.push_str("\n```\n");
                if !closing {
                    // `pre` is a block: keep its raw text on its own lines.
                }
            }
            "blockquote" => out.push_str(if closing { "\n" } else { "\n> " }),
            "li" => {
                if !closing {
                    out.push_str("\n- ");
                }
            }
            "ul" | "ol" => {
                if closing {
                    out.push_str("\n\n");
                }
            }
            "a" => {
                if closing {
                    let href = links.pop().unwrap_or_default();
                    out.push_str(&format!("]({href})"));
                } else if let Some(href) = attr(tag_text, "href") {
                    links.push(href);
                    out.push('[');
                }
            }
            "img" => {
                let src = attr(tag_text, "src").unwrap_or_default();
                let alt = attr(tag_text, "alt").unwrap_or_default();
                out.push_str(&format!("![{alt}]({src})"));
            }
            "hr" => out.push_str("\n---\n"),
            _ => {}
        }
    }

    out.push_str(rest);
    decode_entities(&out)
}

fn tag(source: &str, name: &str) -> Option<String> {
    let open = format!("<{name}>");
    let close = format!("</{name}>");
    let start = source.find(&open)? + open.len();
    let end = source[start..].find(&close)? + start;
    Some(source[start..end].to_string())
}

fn strip_cdata(value: &str) -> String {
    let trimmed = value.trim();
    let inner = trimmed
        .strip_prefix("<![CDATA[")
        .unwrap_or(trimmed)
        .strip_suffix("]]>")
        .unwrap_or(trimmed);
    inner.to_string()
}

fn attr(tag_text: &str, name: &str) -> Option<String> {
    let lower = tag_text.to_ascii_lowercase();
    let index = lower.find(&format!("{name}="))?;
    let after = tag_text[index + name.len() + 1..].trim_start();

    if let Some(rest) = after.strip_prefix('"') {
        rest.split('"').next().map(str::to_string)
    } else if let Some(rest) = after.strip_prefix('\'') {
        rest.split('\'').next().map(str::to_string)
    } else {
        after
            .split_whitespace()
            .next()
            .map(|value| value.trim_end_matches('/').to_string())
    }
}

fn decode_entities(value: &str) -> String {
    value
        .replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_basic_html() {
        let markdown =
            html_to_markdown("<h2>Title</h2><p>Some <b>bold</b> text</p><ul><li>one</li></ul>");
        assert!(markdown.contains("## Title"));
        assert!(markdown.contains("Some **bold** text"));
        assert!(markdown.contains("- one"));
    }

    #[test]
    fn converts_links_images_and_entities() {
        let markdown = html_to_markdown(
            r#"<a href="https://x.test">x &amp; y</a><img src="p.png" alt="pic">"#,
        );
        assert!(markdown.contains("[x & y](https://x.test)"));
        assert!(markdown.contains("![pic](p.png)"));
    }

    #[test]
    fn parses_an_enex_note() {
        let xml = r#"<?xml version="1.0"?>
<en-export>
<note><title>Groceries</title>
<content><![CDATA[<?xml version="1.0"?><en-note><h1>Groceries</h1><p>milk</p></en-note>]]></content>
</note>
</en-export>"#;

        let notes = evernote_enex(xml);
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].title, "Groceries");
        assert!(notes[0].markdown.starts_with("# Groceries"));
        assert!(notes[0].markdown.contains("milk"));
    }

    #[test]
    fn parses_a_roam_page_with_nesting() {
        let json = r#"[{"title":"Page","children":[
            {"string":"first","children":[{"string":"nested","children":[]}]},
            {"string":"second","children":[]}
        ]}]"#;

        let notes = roam_pages(json).unwrap();
        assert_eq!(notes.len(), 1);
        assert!(notes[0].markdown.contains("- first"));
        assert!(notes[0].markdown.contains("  - nested"));
        assert!(notes[0].markdown.contains("- second"));
    }

    #[test]
    fn rejects_invalid_roam_json() {
        assert!(roam_pages("not json").is_err());
    }
}
