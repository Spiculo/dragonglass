//! Golden-image (vector) regression test for the layout engine.
//!
//! Each case is rendered through the same display list → SVG path used by HTML export. The
//! files in `tests/math-golden/` are checked in; regenerate them with
//! `UPDATE_MATH_GOLDEN=1 cargo test -p dragonglass-core --test math_golden`.
//!
//! This is not a KaTeX screenshot diff (that needs an external reference corpus): it pins the
//! layout + glyph-outline output so spacing, delimiter growth and fraction metrics cannot
//! regress silently.

use dragonglass_core::math::{self, MathOptions, MathStyle};

#[test]
fn svg_goldens_match() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/math-golden");
    let update = std::env::var("UPDATE_MATH_GOLDEN").is_ok();

    let cases: &[(&str, &str, MathStyle)] = &[
        ("fraction", "\\frac{a+b}{2}", MathStyle::Display),
        ("radical", "\\sqrt{x+1}", MathStyle::Display),
        ("sum", "\\sum_{k=1}^{n} k", MathStyle::Display),
        (
            "delimiters",
            "\\left(\\frac{a}{b}\\right)",
            MathStyle::Display,
        ),
        (
            "matrix",
            "\\begin{pmatrix} a & b \\\\ c & d \\end{pmatrix}",
            MathStyle::Display,
        ),
        ("inline_scripts", "E = mc^2", MathStyle::Inline),
    ];

    for (name, tex, style) in cases {
        let scene = math::typeset(
            tex,
            &MathOptions {
                style: *style,
                color: [0.0, 0.0, 0.0, 1.0],
            },
        )
        .unwrap_or_else(|error| panic!("{name}: {error}"));

        let options = ratex_svg::SvgOptions {
            font_size: 20.0,
            padding: 2.0,
            stroke_width: 1.0,
            embed_glyphs: true,
            font_dir: String::new(),
        };
        let svg = ratex_svg::render_to_svg(&scene.display, &options);

        let path = root.join(format!("{name}.svg"));
        if update {
            std::fs::create_dir_all(&root).unwrap();
            std::fs::write(&path, &svg).unwrap();
            continue;
        }

        let expected = std::fs::read_to_string(&path).unwrap_or_else(|_| {
            panic!(
                "missing golden {} (run with UPDATE_MATH_GOLDEN=1)",
                path.display()
            )
        });
        assert!(
            svg == expected,
            "golden mismatch for {name}: regenerate with UPDATE_MATH_GOLDEN=1 if the change is intentional"
        );
    }
}
