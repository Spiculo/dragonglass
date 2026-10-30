//! Math typesetting.
//!
//! Two layers live here:
//!
//! - [`engine`] wraps [RaTeX](https://github.com/erweixin/RaTeX), a pure-Rust KaTeX port.
//!   It parses TeX and lays it out into a platform-neutral display list of glyph outlines,
//!   rules and paths, in em units. This is what the app, HTML export and PDF export use.
//! - [`plain`] is the old Unicode transliterator. It is no longer used to display math; it
//!   survives as the plain-text form for the outline pane, search snippets and as an export
//!   escape hatch when a formula cannot be typeset.
//!
//! [`MathScene`] is the boundary: the UI never sees RaTeX types.

mod engine;
mod outline;
mod plain;

pub use engine::{typeset, MathError, MathOptions, MathScene, MathStyle};
pub use outline::{glyph_outline, Curve as GlyphCurve, GlyphOutline};
pub use plain::{spans, split_math, to_unicode};

// Re-export the display-list types so the rest of the app (and the GPUI painter) can consume
// them without depending on RaTeX directly.
pub use ratex_types::color::Color;
pub use ratex_types::display_item::{DisplayItem, DisplayList};
pub use ratex_types::path_command::PathCommand;
