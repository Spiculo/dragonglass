//! Theme palette, stored as a GPUI global so every view can read it.
//!
//! Obsidian community themes target a DOM and cannot work here, so Dragon Glass ships its
//! own palette with a dark and a light variant.

use gpui::{rgb, Global, Hsla};

#[derive(Clone, Copy, Debug)]
pub struct Theme {
    pub dark: bool,
    pub bg: Hsla,
    pub panel: Hsla,
    pub panel_alt: Hsla,
    pub header: Hsla,
    pub input_bg: Hsla,
    pub chip: Hsla,
    pub border: Hsla,
    pub text: Hsla,
    pub text_dim: Hsla,
    pub muted: Hsla,
    pub accent: Hsla,
    pub accent_text: Hsla,
    pub warn: Hsla,
    pub star: Hsla,
    pub code_bg: Hsla,
    pub code_text: Hsla,
    pub selection: Hsla,
}

fn h(hex: u32) -> Hsla {
    rgb(hex).into()
}

impl Theme {
    pub fn dark() -> Self {
        Self {
            dark: true,
            bg: h(0x1e1e1e),
            panel: h(0x181818),
            panel_alt: h(0x1b1b1b),
            header: h(0x252526),
            input_bg: h(0x232323),
            chip: h(0x2a2a2a),
            border: h(0x3a3a3a),
            text: h(0xe6e6e6),
            text_dim: h(0xc0c0c0),
            muted: h(0x8a8a8a),
            accent: h(0x2a3b57),
            accent_text: h(0xffffff),
            warn: h(0xe0b050),
            star: h(0xffd479),
            code_bg: h(0x111111),
            code_text: h(0xb8d4b8),
            selection: h(0x3b82f6).opacity(0.3),
        }
    }

    pub fn light() -> Self {
        Self {
            dark: false,
            bg: h(0xffffff),
            panel: h(0xf4f4f4),
            panel_alt: h(0xf7f7f7),
            header: h(0xeaeaea),
            input_bg: h(0xfbfbfb),
            chip: h(0xe9e9e9),
            border: h(0xd4d4d4),
            text: h(0x1f1f1f),
            text_dim: h(0x444444),
            muted: h(0x6f6f6f),
            accent: h(0xcfe0ff),
            accent_text: h(0x102a4a),
            warn: h(0xa86a00),
            star: h(0xb8860b),
            code_bg: h(0xf2f2f2),
            code_text: h(0x1f5f3f),
            selection: h(0x3b82f6).opacity(0.22),
        }
    }

    pub fn toggled(&self) -> Self {
        if self.dark {
            Self::light()
        } else {
            Self::dark()
        }
    }

    pub fn name(&self) -> &'static str {
        if self.dark {
            "dark"
        } else {
            "light"
        }
    }

    /// Build a theme from `key=RRGGBB` overrides on top of the dark or light base.
    pub fn from_colors(dark: bool, colors: &[(String, String)]) -> Self {
        let mut theme = if dark { Self::dark() } else { Self::light() };

        for (key, value) in colors {
            let Some(color) = parse_hex(value) else {
                continue;
            };
            match key.as_str() {
                "bg" => theme.bg = color,
                "panel" => theme.panel = color,
                "panel_alt" | "panel-alt" => theme.panel_alt = color,
                "header" => theme.header = color,
                "input_bg" | "input-bg" => theme.input_bg = color,
                "chip" => theme.chip = color,
                "border" => theme.border = color,
                "text" => theme.text = color,
                "text_dim" | "text-dim" => theme.text_dim = color,
                "muted" => theme.muted = color,
                "accent" => theme.accent = color,
                "accent_text" | "accent-text" => theme.accent_text = color,
                "warn" => theme.warn = color,
                "star" => theme.star = color,
                "code_bg" | "code-bg" => theme.code_bg = color,
                "code_text" | "code-text" => theme.code_text = color,
                "selection" => theme.selection = color,
                _ => {}
            }
        }

        theme
    }
}

/// Parse `RRGGBB` (with or without a leading `#`) into a colour.
pub fn parse_hex(value: &str) -> Option<Hsla> {
    let cleaned = value.trim().trim_start_matches('#');
    if cleaned.len() != 6 {
        return None;
    }
    let value = u32::from_str_radix(cleaned, 16).ok()?;
    Some(rgb(value).into())
}

impl Global for Theme {}
