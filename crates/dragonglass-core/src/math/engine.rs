//! RaTeX adapter: TeX source â†’ display list.
//!
//! Everything in this file is the *only* place that talks to the `ratex-*` crates. The
//! public types are Dragon Glass types, so a RaTeX upgrade (the 0.1.x line moves fast) or a
//! replacement engine only touches this module.

use ratex_layout::{layout, to_display_list, validate_layout, LayoutOptions};
use ratex_parser::parse;
use ratex_types::{Color, DisplayList, MathStyle as RaTeXStyle};

/// How a formula participates in the surrounding text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MathStyle {
    /// `$â€¦$`: text style, scripts at script size, operators compact.
    Inline,
    /// `$$â€¦$$`: display style, limits above/below, larger operators.
    Display,
}

impl MathStyle {
    fn ratex(self) -> RaTeXStyle {
        match self {
            MathStyle::Inline => RaTeXStyle::Text,
            MathStyle::Display => RaTeXStyle::Display,
        }
    }
}

/// Options that affect the layout itself.
#[derive(Clone, Copy, Debug)]
pub struct MathOptions {
    pub style: MathStyle,
    /// Base colour recorded in the display list. Explicit `\color` wins; the UI also
    /// recolours items that still carry this base colour when the theme changes without
    /// re-laying out.
    pub color: [f32; 4],
}

impl Default for MathOptions {
    fn default() -> Self {
        Self {
            style: MathStyle::Inline,
            color: [1.0, 1.0, 1.0, 1.0],
        }
    }
}

/// A laid-out formula.
///
/// Coordinates are em units in a top-left origin, with the baseline at `y = height`
/// (the convention of `ratex_layout::to_display_list`). Multiply by the UI font size to
/// get pixels. The scene is font-size independent, so one scene serves every zoom level.
#[derive(Clone, Debug)]
pub struct MathScene {
    pub display: DisplayList,
    /// The original TeX, kept for selection, tooltips and error reporting.
    pub source: String,
}

impl MathScene {
    pub fn width(&self) -> f64 {
        self.display.width
    }

    pub fn height(&self) -> f64 {
        self.display.height
    }

    pub fn depth(&self) -> f64 {
        self.display.depth
    }

    /// `height + depth`: the full vertical extent of the box.
    pub fn total_height(&self) -> f64 {
        self.display.height + self.display.depth
    }

    pub fn is_empty(&self) -> bool {
        self.display.items.is_empty()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum MathError {
    #[error("unsupported LaTeX: {0}")]
    Parse(String),
    #[error("formula is too large to lay out")]
    TooLarge,
}

/// Typeset `tex` (without the surrounding `$`/`$$` delimiters).
///
/// Layout runs entirely in em and is independent of the display size; pass the style and
/// base colour here and scale the result when painting.
pub fn typeset(tex: &str, options: &MathOptions) -> Result<MathScene, MathError> {
    let source = tex.trim();
    let nodes = parse(source).map_err(|error| MathError::Parse(error.to_string()))?;

    let [r, g, b, a] = options.color;
    let layout_options = LayoutOptions {
        style: options.style.ratex(),
        color: Color::new(r, g, b, a),
        ..LayoutOptions::default()
    };

    let root = layout(&nodes, &layout_options);
    validate_layout(&root).map_err(|_| MathError::TooLarge)?;
    let display = to_display_list(&root);

    // Defensive: a layout that produced non-finite geometry must never reach the painter.
    if !display.width.is_finite()
        || !display.height.is_finite()
        || !display.depth.is_finite()
        || display.width < 0.0
        || display.height < 0.0
        || display.depth < 0.0
    {
        return Err(MathError::TooLarge);
    }

    Ok(MathScene {
        display,
        source: source.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratex_types::display_item::DisplayItem;

    fn scene(tex: &str, style: MathStyle) -> MathScene {
        typeset(
            tex,
            &MathOptions {
                style,
                color: [1.0, 1.0, 1.0, 1.0],
            },
        )
        .unwrap_or_else(|error| panic!("{tex:?} failed to typeset: {error}"))
    }

    #[test]
    fn typesets_a_fraction() {
        let scene = scene("\\frac{a}{b}", MathStyle::Display);
        assert_eq!(scene.source, "\\frac{a}{b}");
        assert!(!scene.is_empty());
        assert!(scene.width() > 0.0);
        assert!(scene.total_height() > 0.0);
        // A fraction always draws a bar.
        assert!(
            scene
                .display
                .items
                .iter()
                .any(|item| matches!(item, DisplayItem::Line { .. })),
            "expected a fraction rule in {:?}",
            scene.display.items
        );
    }

    #[test]
    fn display_style_is_taller_for_limits() {
        let inline = scene("\\sum_{k=1}^{n} k", MathStyle::Inline);
        let display = scene("\\sum_{k=1}^{n} k", MathStyle::Display);
        assert!(
            display.total_height() > inline.total_height(),
            "display {} should be taller than inline {}",
            display.total_height(),
            inline.total_height()
        );
    }

    #[test]
    fn a_matrix_produces_many_items() {
        let scene = scene(
            "\\begin{pmatrix} a & b \\\\ c & d \\end{pmatrix}",
            MathStyle::Display,
        );
        assert!(
            scene.display.items.len() > 4,
            "expected several glyphs, got {}",
            scene.display.items.len()
        );
    }

    #[test]
    fn unknown_commands_are_errors_not_graphic_text() {
        let result = typeset("\\definitelynotacommand{x}", &MathOptions::default());
        assert!(matches!(result, Err(MathError::Parse(_))));
    }

    #[test]
    fn malformed_input_never_panics() {
        for input in ["\\frac{1}{", "\\begin{pmatrix}", "$x$", "\\", "\\\\"] {
            let _ = typeset(input, &MathOptions::default());
        }
    }

    #[test]
    fn radical_and_tall_delimiters_produce_paths() {
        let sqrt = scene("\\sqrt{x+1}", MathStyle::Display);
        assert!(
            sqrt.display
                .items
                .iter()
                .any(|item| matches!(item, DisplayItem::Line { .. })),
            "expected the radical's vinculum"
        );

        let fraction = scene("\\frac{a}{b}", MathStyle::Display);
        let delimited = scene("\\left(\\frac{a}{b}\\right)", MathStyle::Display);
        assert!(delimited.width() > fraction.width());
        assert!(delimited.total_height() > fraction.total_height());
    }

    #[test]
    fn truncating_real_formulas_never_panics() {
        use std::path::Path;

        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-vault");
        if !root.is_dir() {
            eprintln!("test-vault not present; skipping truncation test");
            return;
        }

        let options = MathOptions {
            style: MathStyle::Display,
            color: [0.0, 0.0, 0.0, 1.0],
        };
        let mut checked = 0usize;

        'files: for entry in walkdir::WalkDir::new(&root)
            .into_iter()
            .filter_map(Result::ok)
        {
            if !entry.file_type().is_file()
                || entry.path().extension().and_then(|ext| ext.to_str()) != Some("md")
            {
                continue;
            }
            let Ok(source) = std::fs::read_to_string(entry.path()) else {
                continue;
            };
            for block in crate::markdown::parse_blocks(&source) {
                let crate::markdown::Block::Math { tex } = block else {
                    continue;
                };
                let chars: Vec<char> = tex.chars().collect();
                let step = (chars.len() / 8).max(1);
                for end in (1..chars.len()).step_by(step) {
                    let prefix: String = chars[..end].iter().collect();
                    let _ = typeset(&prefix, &options);
                }
                checked += 1;
                if checked >= 40 {
                    break 'files;
                }
            }
        }

        assert!(checked > 0, "no formulas exercised");
    }

    /// Local acceptance corpus: every display formula in the test vault must typeset.
    /// The vault is git-ignored, so CI without it skips this test.
    #[test]
    fn typesets_every_display_formula_in_the_test_vault() {
        use std::path::Path;

        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-vault");
        if !root.is_dir() {
            eprintln!("test-vault not present; skipping corpus test");
            return;
        }

        let mut total = 0usize;
        let mut failures = Vec::new();
        for entry in walkdir::WalkDir::new(&root)
            .into_iter()
            .filter_map(Result::ok)
        {
            if !entry.file_type().is_file() {
                continue;
            }
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("md") {
                continue;
            }
            let Ok(source) = std::fs::read_to_string(path) else {
                continue;
            };
            for block in crate::markdown::parse_blocks(&source) {
                let crate::markdown::Block::Math { tex } = block else {
                    continue;
                };
                total += 1;
                if let Err(error) = typeset(
                    &tex,
                    &MathOptions {
                        style: MathStyle::Display,
                        color: [0.0, 0.0, 0.0, 1.0],
                    },
                ) {
                    failures.push(format!("{}: {error} :: {tex}", path.display()));
                }
            }
        }

        assert!(total > 100, "expected a math corpus, found {total}");
        assert!(
            failures.is_empty(),
            "{} of {total} display formulas failed:\n{}",
            failures.len(),
            failures.join("\n")
        );
    }

    /// Geometry snapshots: a layout regression (spacing, delimiter growth, fraction metrics)
    /// changes these numbers. Values are in em and rounded to three decimals.
    #[test]
    fn golden_geometry() {
        let cases: &[(&str, MathStyle, f64, f64, f64)] = &[
            ("\\frac{a}{b}", MathStyle::Display, 0.769, 1.108, 0.686),
            ("\\sqrt{x+1}", MathStyle::Display, 3.127, 0.914, 0.126),
            ("\\sum_{k=1}^{n} k", MathStyle::Display, 2.163, 1.731, 1.382),
            (
                "\\left(\\frac{a}{b}\\right)",
                MathStyle::Display,
                1.963,
                1.150,
                0.686,
            ),
            (
                "\\begin{pmatrix} a & b \\\\ c & d \\end{pmatrix}",
                MathStyle::Display,
                3.521,
                1.450,
                0.950,
            ),
            ("E = mc^2", MathStyle::Inline, 3.840, 0.814, 0.000),
            ("\\alpha + \\beta", MathStyle::Inline, 2.484, 0.694, 0.194),
            (
                "\\int_0^1 x^2\\,dx",
                MathStyle::Display,
                3.797,
                1.565,
                0.911,
            ),
            (
                "\\begin{aligned} a &= b \\\\ c &= d \\end{aligned}",
                MathStyle::Display,
                2.382,
                1.600,
                1.100,
            ),
            (
                "\\mathcal{O}(n \\log n)",
                MathStyle::Inline,
                4.413,
                0.750,
                0.250,
            ),
        ];

        for (tex, style, width, height, depth) in cases {
            let scene = scene(tex, *style);
            let close = |a: f64, b: f64| (a - b).abs() < 0.01;
            assert!(
                close(scene.width(), *width)
                    && close(scene.height(), *height)
                    && close(scene.depth(), *depth),
                "{tex}: got ({:.3}, {:.3}, {:.3}), expected ({width}, {height}, {depth})",
                scene.width(),
                scene.height(),
                scene.depth()
            );
        }
    }

    /// A loose budget: this is a regression guard against accidental per-frame typesetting,
    /// not a benchmark. Debug builds are slow; release is an order of magnitude faster.
    #[test]
    fn typesetting_budget() {
        let options = MathOptions {
            style: MathStyle::Inline,
            color: [0.0, 0.0, 0.0, 1.0],
        };
        let start = std::time::Instant::now();
        for _ in 0..100 {
            for tex in [
                "O(n \\log n)",
                "\\frac{a}{b}",
                "\\sqrt{x+1}",
                "\\sum_{k=1}^{n} k",
                "\\alpha + \\beta",
            ] {
                let _ = typeset(tex, &options).unwrap();
            }
        }
        let elapsed = start.elapsed();
        eprintln!("500 inline formulas in {elapsed:?}");
        assert!(
            elapsed < std::time::Duration::from_secs(5),
            "typesetting 500 inline formulas took {elapsed:?}"
        );
    }
}
