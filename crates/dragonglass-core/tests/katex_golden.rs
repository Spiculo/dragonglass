//! KaTeX reference-image comparison.
//!
//! The fixtures are KaTeX 0.16.45 renders from RaTeX's published golden suite
//! (`tests/golden/fixtures`, MIT). Our scene is rasterized with the same RaTeX renderer used
//! by the layout engine, then both images are cropped to their ink and compared as normalized
//! masks. This is deliberately a coarse ink-overlap check, not the authoritative
//! `ratex-ink-v2` score: it catches missing/wrong glyphs and broken layout without making the
//! test depend on the reference harness's canvas size.
//!
//! Regenerate fixtures by downloading the matching index from the RaTeX repository; formulas
//! are in the filenames' comments below.

use std::io::Cursor;
use std::path::Path;

use dragonglass_core::math::{self, Color, MathOptions, MathStyle};

const MASK: usize = 96;
const MIN_IOU: f64 = 0.55;

#[test]
fn katex_reference_ink_overlap() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/katex-fixtures");

    // (fixture, formula)
    let cases: &[(&str, &str)] = &[
        ("0015.png", "x_i"),
        ("0041.png", "\\alpha"),
        ("0309.png", "\\frac a b"),
        (
            "0629.png",
            "\\begin{pmatrix} a & b \\\\ c & d \\end{pmatrix}",
        ),
        ("0744.png", "\\sqrt[3]{x}"),
        ("0917.png", "\\hat{M}"),
        ("1057.png", "\\sum_{m=1}^{M} m"),
    ];

    for (fixture, tex) in cases {
        let scene = math::typeset(
            tex,
            &MathOptions {
                style: MathStyle::Display,
                color: [0.0, 0.0, 0.0, 1.0],
            },
        )
        .unwrap_or_else(|error| panic!("{tex}: {error}"));

        let rendered = ratex_render::render_to_png(
            &scene.display,
            &ratex_render::RenderOptions {
                font_size: 40.0,
                padding: 10.0,
                background_color: Color::WHITE,
                font_dir: String::new(),
                device_pixel_ratio: 1.0,
            },
        )
        .unwrap_or_else(|error| panic!("{tex}: render failed: {error}"));

        let reference = std::fs::read(root.join(fixture)).unwrap();
        let ours = normalize(&rendered);
        let theirs = normalize(&reference);
        let intersection = ours.iter().zip(&theirs).filter(|(a, b)| **a && **b).count();
        let union = ours.iter().zip(&theirs).filter(|(a, b)| **a || **b).count();
        let iou = intersection as f64 / union.max(1) as f64;
        eprintln!("{fixture} ({tex}): ink IoU {iou:.3}");

        assert!(
            iou >= MIN_IOU,
            "{fixture}: ink IoU {iou:.3} below {MIN_IOU}"
        );
    }
}

/// Decode a PNG, crop to its ink bounding box and resample to a fixed mask.
fn normalize(png: &[u8]) -> Vec<bool> {
    let (pixels, width, height, channels) = decode(png);
    let mut min_x = width;
    let mut min_y = height;
    let mut max_x = 0usize;
    let mut max_y = 0usize;
    let mut any = false;

    for y in 0..height {
        for x in 0..width {
            let index = (y * width + x) * channels;
            let r = pixels[index] as u16;
            let g = pixels[index + 1] as u16;
            let b = pixels[index + 2] as u16;
            if (r + g + b) / 3 < 200 {
                any = true;
                min_x = min_x.min(x);
                max_x = max_x.max(x);
                min_y = min_y.min(y);
                max_y = max_y.max(y);
            }
        }
    }

    if !any {
        return vec![false; MASK * MASK];
    }

    let crop_w = (max_x - min_x + 1).max(1);
    let crop_h = (max_y - min_y + 1).max(1);
    let mut mask = vec![false; MASK * MASK];

    for ty in 0..MASK {
        for tx in 0..MASK {
            let sx = min_x + tx * crop_w / MASK;
            let sy = min_y + ty * crop_h / MASK;
            let index = (sy * width + sx) * channels;
            let r = pixels[index] as u16;
            let g = pixels[index + 1] as u16;
            let b = pixels[index + 2] as u16;
            mask[ty * MASK + tx] = (r + g + b) / 3 < 200;
        }
    }
    mask
}

fn decode(png: &[u8]) -> (Vec<u8>, usize, usize, usize) {
    let mut decoder = png::Decoder::new(Cursor::new(png));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().expect("PNG header");
    let mut buffer = vec![0; reader.output_buffer_size().expect("PNG frame size")];
    let info = reader.next_frame(&mut buffer).expect("PNG data");
    buffer.truncate(info.buffer_size());
    let channels = match info.color_type {
        png::ColorType::Grayscale => 1,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        other => panic!("unsupported PNG colour type {other:?}"),
    };
    (buffer, info.width as usize, info.height as usize, channels)
}
