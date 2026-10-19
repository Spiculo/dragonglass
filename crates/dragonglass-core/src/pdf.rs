//! A minimal PDF writer, enough to print notes.
//!
//! No dependency: PDF is a text format, so a single-page-font document is a few hundred
//! lines of bookkeeping. Uses the base-14 Helvetica font, so text is ASCII-only (anything
//! else becomes `?`).

use crate::markdown::{self, Block};

const PAGE_WIDTH: f32 = 612.0;
const PAGE_HEIGHT: f32 = 792.0;
const MARGIN: f32 = 72.0;
const LEADING: f32 = 14.0;
const LINES_PER_PAGE: usize = 46;
/// Rough width of a Helvetica character at 11pt, for wrapping.
const CHAR_WIDTH: f32 = 5.4;
const TEXT_WIDTH: f32 = PAGE_WIDTH - MARGIN * 2.0;

/// Render markdown into a printable PDF.
pub fn from_markdown(src: &str, title: &str) -> String {
    let lines = layout(src, title);

    let mut pages: Vec<Vec<(String, f32)>> = Vec::new();
    if lines.is_empty() {
        pages.push(Vec::new());
    } else {
        for chunk in lines.chunks(LINES_PER_PAGE) {
            pages.push(chunk.to_vec());
        }
    }

    build(&pages)
}

/// Flatten the document into `(text, font size)` lines.
fn layout(src: &str, title: &str) -> Vec<(String, f32)> {
    let mut lines: Vec<(String, f32)> = Vec::new();

    if !title.trim().is_empty() {
        lines.push((title.to_string(), 20.0));
        lines.push((String::new(), 11.0));
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
                lines.push((String::new(), 11.0));
                lines.push((text, size));
            }
            Block::Paragraph(text) => {
                push_wrapped(&mut lines, &text.replace('\n', " "), 11.0, "");
                lines.push((String::new(), 11.0));
            }
            Block::ListItem { depth, text, .. } => {
                let indent = "  ".repeat(depth);
                push_wrapped(&mut lines, &text, 11.0, &format!("{indent}- "));
            }
            Block::Code { text, .. } => {
                for line in text.lines() {
                    lines.push((format!("  {line}"), 9.0));
                }
                lines.push((String::new(), 11.0));
            }
            Block::Quote(text) => {
                push_wrapped(&mut lines, &text.replace('\n', " "), 11.0, "| ");
            }
            Block::Callout { title, text, .. } => {
                push_wrapped(&mut lines, &format!("! {title}"), 11.5, "");
                if !text.trim().is_empty() {
                    push_wrapped(&mut lines, &text.replace('\n', " "), 11.0, "  ");
                }
                lines.push((String::new(), 11.0));
            }
            Block::Rule => lines.push(("-".repeat(60), 11.0)),
            Block::Embed { target } => lines.push((format!("[embed: {target}]"), 10.0)),
        }
    }

    lines
}

fn push_wrapped(lines: &mut Vec<(String, f32)>, text: &str, size: f32, prefix: &str) {
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() {
        lines.push((prefix.trim_end().to_string(), size));
        return;
    }

    let wrap = crate::wrap::wrap(
        &chars,
        |from, to| (to - from) as f32 * CHAR_WIDTH,
        TEXT_WIDTH,
    );

    for (index, (start, end)) in wrap.into_iter().enumerate() {
        let segment: String = chars[start..end].iter().collect();
        if index == 0 {
            lines.push((format!("{prefix}{segment}"), size));
        } else {
            lines.push((
                format!("{}{segment}", " ".repeat(prefix.chars().count())),
                size,
            ));
        }
    }
}

fn build(pages: &[Vec<(String, f32)>]) -> String {
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

fn content_stream(lines: &[(String, f32)]) -> String {
    let mut out = String::from("BT\n");
    let mut size = 11.0f32;
    out.push_str(&format!(
        "/F1 {size} Tf\n{LEADING} TL\n{MARGIN} {} Td\n",
        PAGE_HEIGHT - MARGIN
    ));

    for (index, (text, line_size)) in lines.iter().enumerate() {
        if (line_size - size).abs() > 0.01 {
            size = *line_size;
            out.push_str(&format!("/F1 {size} Tf\n"));
        }
        out.push_str(&format!("({}) Tj\n", escape(text)));
        if index + 1 < lines.len() {
            out.push_str("T*\n");
        }
    }

    out.push_str("ET\n");
    out
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '(' => out.push_str("\\("),
            ')' => out.push_str("\\)"),
            '\\' => out.push_str("\\\\"),
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
        assert!(pdf.contains("(Hello world.) Tj"));
        assert!(pdf.contains("/Type /Catalog"));
        assert!(pdf.contains("/BaseFont /Helvetica"));

        // The xref offset must point at the `xref` keyword.
        let offset = xref_offset(&pdf);
        assert_eq!(&pdf[offset..offset + 4], "xref");
    }

    #[test]
    fn escapes_pdf_string_delimiters() {
        let pdf = from_markdown("a (b) c \\ d\n", "");
        assert!(pdf.contains("a \\(b\\) c \\\\ d"));
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
}
