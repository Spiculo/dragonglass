//! Reading mode: render parsed markdown blocks as styled GPUI elements, including
//! embedded images and transcluded notes.
//!
//! Paragraph and list text goes through `StyledText` when it contains `==highlights==`, so
//! highlighted spans get a background while the text still wraps normally.

use std::path::{Path, PathBuf};

use dragonglass_core::markdown::{self, Block};
use gpui::{div, img, prelude::*, px, relative, AnyElement, IntoElement, StyledText, TextRun};

use crate::theme::Theme;

/// Guard against `![[a]]` → `a` → `![[a]]` cycles.
const MAX_EMBED_DEPTH: usize = 3;

pub fn reading_view(
    src: &str,
    vault_root: Option<&Path>,
    depth: usize,
    theme: Theme,
    template: &TextRun,
) -> impl IntoElement {
    let blocks = markdown::parse_blocks(src);
    let template = template.clone();

    div()
        .flex()
        .flex_col()
        .gap_3()
        .p_4()
        .text_color(theme.text)
        .children(
            blocks
                .into_iter()
                .map(move |block| render_block(block, vault_root, depth, theme, &template)),
        )
}

fn render_block(
    block: Block,
    vault_root: Option<&Path>,
    depth: usize,
    theme: Theme,
    template: &TextRun,
) -> AnyElement {
    match block {
        Block::Heading { level, text } => {
            let size = match level {
                1 => 30.0,
                2 => 24.0,
                3 => 20.0,
                _ => 17.0,
            };
            div()
                .text_size(px(size))
                .font_weight(gpui::FontWeight::BOLD)
                .child(text)
                .into_any_element()
        }
        Block::Paragraph(text) => rich_text(&text.replace('\n', " "), 15.0, theme, template),
        Block::ListItem {
            depth,
            marker,
            text,
            ..
        } => div()
            .flex()
            .flex_row()
            .gap_2()
            .pl(px(8.0 + depth as f32 * 16.0))
            .child(div().text_color(theme.muted).child(marker))
            .child(rich_text(&text, 15.0, theme, template))
            .into_any_element(),
        Block::Code { text, .. } => div()
            .bg(theme.code_bg)
            .p_3()
            .rounded_md()
            .text_size(px(13.0))
            .text_color(theme.code_text)
            .child(text)
            .into_any_element(),
        Block::Quote(text) => div()
            .border_l_1()
            .border_color(theme.border)
            .pl_3()
            .text_color(theme.muted)
            .child(text.replace('\n', " "))
            .into_any_element(),
        Block::Rule => div()
            .h(px(1.0))
            .w_full()
            .bg(theme.border)
            .into_any_element(),
        Block::Callout { kind, title, text } => {
            let accent = match kind.as_str() {
                "warning" | "caution" | "danger" | "error" => theme.warn,
                "tip" | "success" => theme.code_text,
                _ => theme.accent_text,
            };

            div()
                .border_l_1()
                .border_color(accent)
                .bg(theme.chip)
                .rounded_md()
                .p_3()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .font_weight(gpui::FontWeight::BOLD)
                        .text_color(accent)
                        .child(title),
                )
                .when(!text.trim().is_empty(), |d| {
                    d.child(rich_text(&text.replace('\n', " "), 15.0, theme, template))
                })
                .into_any_element()
        }
        Block::Embed { target } => render_embed(&target, vault_root, depth, theme, template),
    }
}

/// Plain text, unless it contains `==highlights==`.
fn rich_text(text: &str, size: f32, theme: Theme, template: &TextRun) -> AnyElement {
    let parts = markdown::split_highlights(text);
    let container = div().text_size(px(size)).line_height(relative(1.5));

    if !parts.iter().any(|(_, highlighted)| *highlighted) {
        return container.child(text.to_string()).into_any_element();
    }

    // The `==` delimiters are dropped, so render the rebuilt string.
    let display: String = parts.iter().map(|(segment, _)| segment.as_str()).collect();
    let runs: Vec<TextRun> = parts
        .iter()
        .filter(|(segment, _)| !segment.is_empty())
        .map(|(segment, highlighted)| TextRun {
            len: segment.len(),
            font: template.font.clone(),
            color: template.color,
            background_color: highlighted.then_some(theme.star),
            underline: None,
            strikethrough: None,
        })
        .collect();

    container
        .child(StyledText::new(display).with_runs(runs))
        .into_any_element()
}

fn render_embed(
    target: &str,
    vault_root: Option<&Path>,
    depth: usize,
    theme: Theme,
    template: &TextRun,
) -> AnyElement {
    let Some(root) = vault_root else {
        return placeholder(target, "no vault root", theme);
    };

    if is_image(target) {
        let path = root.join(target);
        return if path.exists() {
            img(path)
                .max_w(px(480.0))
                .max_h(px(360.0))
                .into_any_element()
        } else {
            placeholder(target, "image not found", theme)
        };
    }

    if depth >= MAX_EMBED_DEPTH {
        return placeholder(target, "embed depth limit", theme);
    }

    match std::fs::read_to_string(resolve_note(root, target)) {
        Ok(text) => div()
            .border_l_1()
            .border_color(theme.border)
            .pl_3()
            .child(reading_view(&text, Some(root), depth + 1, theme, template))
            .into_any_element(),
        Err(_) => placeholder(target, "note not found", theme),
    }
}

fn is_image(target: &str) -> bool {
    matches!(
        Path::new(target)
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .as_deref(),
        Some("png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "svg" | "avif" | "ico")
    )
}

fn resolve_note(root: &Path, target: &str) -> PathBuf {
    let candidate = root.join(target);
    if candidate.extension().is_some() {
        candidate
    } else {
        root.join(format!("{target}.md"))
    }
}

fn placeholder(target: &str, reason: &str, theme: Theme) -> AnyElement {
    div()
        .px_3()
        .py_2()
        .bg(theme.chip)
        .rounded_md()
        .text_sm()
        .text_color(theme.muted)
        .child(format!("embed: {target} ({reason})"))
        .into_any_element()
}
