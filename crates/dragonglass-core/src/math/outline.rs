//! Glyph outlines from the bundled KaTeX faces.
//!
//! The display list positions glyphs but does not carry their shapes: a `GlyphPath` item names
//! a KaTeX face and a character code. Both the GPUI painter and the PDF exporter need the
//! actual outline, so the lookup lives here, next to the engine, and is cached per face.

use std::cell::RefCell;
use std::collections::HashMap;

use ab_glyph::{Font, FontVec, OutlineCurve};
use ratex_font::FontId;

pub use ab_glyph::OutlineCurve as Curve;

/// One glyph's outline, in font units.
#[derive(Clone, Debug)]
pub struct GlyphOutline {
    pub units_per_em: f32,
    pub curves: Vec<OutlineCurve>,
}

thread_local! {
    /// Parsed faces, loaded once per thread. `None` records a face we failed to load so a
    /// broken/missing font does not retry on every frame.
    static FACES: RefCell<HashMap<FontId, Option<FontVec>>> = RefCell::new(HashMap::new());
}

/// KaTeX TTF file names, mirroring `ratex_font_loader`'s private map. Only the math faces
/// are listed: CJK/emoji fallbacks are system fonts and are not part of the bundled set.
fn file_name(font: FontId) -> Option<&'static str> {
    Some(match font {
        FontId::MainRegular => "KaTeX_Main-Regular.ttf",
        FontId::MainBold => "KaTeX_Main-Bold.ttf",
        FontId::MainItalic => "KaTeX_Main-Italic.ttf",
        FontId::MainBoldItalic => "KaTeX_Main-BoldItalic.ttf",
        FontId::MathItalic => "KaTeX_Math-Italic.ttf",
        FontId::MathBoldItalic => "KaTeX_Math-BoldItalic.ttf",
        FontId::AmsRegular => "KaTeX_AMS-Regular.ttf",
        FontId::CaligraphicRegular => "KaTeX_Caligraphic-Regular.ttf",
        FontId::FrakturRegular => "KaTeX_Fraktur-Regular.ttf",
        FontId::FrakturBold => "KaTeX_Fraktur-Bold.ttf",
        FontId::SansSerifRegular => "KaTeX_SansSerif-Regular.ttf",
        FontId::SansSerifBold => "KaTeX_SansSerif-Bold.ttf",
        FontId::SansSerifItalic => "KaTeX_SansSerif-Italic.ttf",
        FontId::ScriptRegular => "KaTeX_Script-Regular.ttf",
        FontId::TypewriterRegular => "KaTeX_Typewriter-Regular.ttf",
        FontId::Size1Regular => "KaTeX_Size1-Regular.ttf",
        FontId::Size2Regular => "KaTeX_Size2-Regular.ttf",
        FontId::Size3Regular => "KaTeX_Size3-Regular.ttf",
        FontId::Size4Regular => "KaTeX_Size4-Regular.ttf",
        FontId::CjkRegular | FontId::CjkFallback | FontId::EmojiFallback => return None,
    })
}

fn with_face<R>(font: FontId, f: impl FnOnce(&FontVec) -> R) -> Option<R> {
    FACES.with(|faces| {
        let mut faces = faces.borrow_mut();
        let entry = faces.entry(font).or_insert_with(|| {
            let name = file_name(font)?;
            let bytes = ratex_katex_fonts::ttf_bytes(name)?;
            FontVec::try_from_vec(bytes.into_owned()).ok()
        });
        entry.as_ref().map(f)
    })
}

/// Outline for the glyph a display-list item refers to.
///
/// `font` is the `font` field of `DisplayItem::GlyphPath` (e.g. `"Main-Regular"`) and
/// `char_code` is its `char_code`. Unknown faces fall back to `Main-Regular`, matching
/// RaTeX's own renderer.
pub fn glyph_outline(font: &str, char_code: u32) -> Option<GlyphOutline> {
    let font_id = FontId::parse(font).unwrap_or(FontId::MainRegular);
    let ch = ratex_font::katex_ttf_glyph_char(font_id, char_code);

    with_face(font_id, |face| {
        let glyph_id = face.glyph_id(ch);
        if glyph_id.0 == 0 {
            return None;
        }
        let outline = face.outline(glyph_id)?;
        Some(GlyphOutline {
            units_per_em: face.units_per_em().unwrap_or(1000.0),
            curves: outline.curves,
        })
    })
    .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outlines_a_math_glyph() {
        let outline = glyph_outline("Math-Italic", 'x' as u32).expect("x should have an outline");
        assert!(outline.units_per_em > 0.0);
        assert!(!outline.curves.is_empty(), "x should have curves");
    }

    #[test]
    fn outlines_a_size_delimiter() {
        // \left( uses the Size4 face; the glyph exists but its shape comes from that face.
        let outline = glyph_outline("Size4-Regular", '(' as u32);
        assert!(outline.is_some());
    }

    #[test]
    fn unknown_faces_fall_back_to_main() {
        let outline = glyph_outline("Not-A-Real-Face", 'a' as u32);
        assert!(outline.is_some());
    }

    #[test]
    fn cjk_faces_are_not_bundled() {
        assert!(glyph_outline("CjkRegular", '你' as u32).is_none());
    }
}
