//! Theme palette, stored as a GPUI global so every view can read it.
//!
//! Obsidian community themes target a DOM and cannot work here, so Dragon Glass ships its
//! own palette with a dark and a light variant.

use gpui::{rgb, Global, Hsla};

#[derive(Clone, Copy, Debug, PartialEq)]
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
    /// Destructive or failed actions: save errors, lost links, conflicts.
    pub error: Hsla,
    pub star: Hsla,
    pub code_bg: Hsla,
    pub code_text: Hsla,
    pub code_keyword: Hsla,
    pub code_string: Hsla,
    pub code_function: Hsla,
    pub selection: Hsla,
    /// Background of the line containing the caret.
    pub current_line: Hsla,
    /// Background of a hovered list row.
    pub hover: Hsla,
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
            error: h(0xe05555),
            star: h(0xffd479),
            code_bg: h(0x111111),
            code_text: h(0xb8d4b8),
            code_keyword: h(0x569cd6),
            code_string: h(0xce9178),
            code_function: h(0xdcdcaa),
            selection: h(0x3b82f6).opacity(0.3),
            current_line: h(0x262626),
            hover: h(0x242424),
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
            error: h(0xb3261e),
            star: h(0xb8860b),
            code_bg: h(0xf2f2f2),
            code_text: h(0x1f5f3f),
            code_keyword: h(0x0000ff),
            code_string: h(0xa31515),
            code_function: h(0x795e26),
            selection: h(0x3b82f6).opacity(0.22),
            current_line: h(0xf0f1f3),
            hover: h(0xe8e8e8),
        }
    }

    /// Apply a user accent colour, choosing readable text on top of it.
    pub fn with_accent(&self, accent: Hsla) -> Self {
        let mut theme = *self;
        let rgba = gpui::Rgba::from(accent);
        let luminance = 0.299 * rgba.r + 0.587 * rgba.g + 0.114 * rgba.b;
        theme.accent = accent;
        theme.accent_text = if luminance > 0.6 {
            h(0x101010)
        } else {
            h(0xffffff)
        };
        theme
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

    /// Colour for a code token, shared by the reading view and the editor.
    pub fn code_color(&self, kind: dragonglass_core::syntax::CodeKind) -> Hsla {
        use dragonglass_core::syntax::CodeKind;
        match kind {
            CodeKind::Text => self.code_text,
            CodeKind::Keyword => self.code_keyword,
            CodeKind::String => self.code_string,
            CodeKind::Comment => self.muted,
            CodeKind::Number => self.warn,
            CodeKind::Function => self.code_function,
            CodeKind::Type => self.text_dim,
            CodeKind::Punctuation => self.muted,
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
                "error" => theme.error = color,
                "star" => theme.star = color,
                "code_bg" | "code-bg" => theme.code_bg = color,
                "code_text" | "code-text" => theme.code_text = color,
                "code_keyword" | "code-keyword" => theme.code_keyword = color,
                "code_string" | "code-string" => theme.code_string = color,
                "code_function" | "code-function" => theme.code_function = color,
                "selection" => theme.selection = color,
                "current_line" | "current-line" => theme.current_line = color,
                "hover" => theme.hover = color,
                _ => {}
            }
        }

        theme
    }
    /// Apply further `key=RRGGBB` overrides (used by snippets).
    pub fn with_overrides(&self, colors: &[(String, String)]) -> Self {
        let mut theme = *self;
        let base = if theme.dark {
            Self::dark()
        } else {
            Self::light()
        };

        // Rebuild from the base with the extra overrides so snippets layer predictably.
        let mut merged: Vec<(String, String)> = vec![
            ("bg".into(), hex(base.bg)),
            ("panel".into(), hex(base.panel)),
            ("panel_alt".into(), hex(base.panel_alt)),
            ("header".into(), hex(base.header)),
            ("input_bg".into(), hex(base.input_bg)),
            ("chip".into(), hex(base.chip)),
            ("border".into(), hex(base.border)),
            ("text".into(), hex(base.text)),
            ("text_dim".into(), hex(base.text_dim)),
            ("muted".into(), hex(base.muted)),
            ("accent".into(), hex(base.accent)),
            ("accent_text".into(), hex(base.accent_text)),
            ("warn".into(), hex(base.warn)),
            ("error".into(), hex(base.error)),
            ("star".into(), hex(base.star)),
            ("code_bg".into(), hex(base.code_bg)),
            ("code_text".into(), hex(base.code_text)),
            ("code_keyword".into(), hex(base.code_keyword)),
            ("code_string".into(), hex(base.code_string)),
            ("code_function".into(), hex(base.code_function)),
            ("hover".into(), hex(base.hover)),
        ];

        for (key, value) in colors {
            match merged.iter_mut().find(|(existing, _)| existing == key) {
                Some(existing) => existing.1 = value.clone(),
                None => merged.push((key.clone(), value.clone())),
            }
        }

        theme = Self::from_colors(theme.dark, &merged);
        theme
    }
}

/// `RRGGBB` for a colour, for round-tripping through the override list.
fn hex(color: Hsla) -> String {
    let rgba = gpui::Rgba::from(color);
    format!(
        "{:02x}{:02x}{:02x}",
        (rgba.r * 255.0).round() as u8,
        (rgba.g * 255.0).round() as u8,
        (rgba.b * 255.0).round() as u8
    )
}
pub fn parse_hex(value: &str) -> Option<Hsla> {
    let cleaned = value.trim().trim_start_matches('#');
    if cleaned.len() != 6 {
        return None;
    }
    let value = u32::from_str_radix(cleaned, 16).ok()?;
    Some(rgb(value).into())
}

impl Global for Theme {}
