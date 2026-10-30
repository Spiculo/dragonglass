# Math typesetting (LaTeX) — spec

**Status:** Proposed
**Date:** 2026-10-30
**Owner:** Felipe
**Applies to:** `dragonglass-core`, `dragonglass-ui`, `dragonglass-cli`
**Replaces:** the Unicode transliteration in `crates/dragonglass-core/src/math.rs`

---

## Implementation status (2026-10-30)

| Milestone | Status | Evidence |
|---|---|---|
| **M0 — Spike** | **Done** | `ratex-*` pinned in `crates/dragonglass-core/Cargo.toml`; `math::engine` typesets fractions, matrices, display limits; unit tests pass |
| **M1 — Display math** | **Done** | `dragonglass-ui/src/math_element.rs` paints the display list (glyph outlines, rules, paths); `Block::Math` uses it; screenshot shows stacked fractions, `\sum` limits, growing delimiters, `aligned`; all display formulas in `test-vault` typeset (corpus test) |
| **M2 — Inline math** | **Done** | Measured inline flow (`dragonglass-ui/src/flow.rs`): shaped text runs + atomic math boxes, real line breaking, baseline alignment, selection/copy across formulas (Ctrl+A and the drag mapping both yield the `$…$` TeX source), and a hover tooltip with the source (`flow::math_source_at`, `MathElement` prepaint) |
| **M3 — Export parity** | **Done** | HTML export emits self-contained SVG per formula (`math_svg`); PDF export draws both display and inline formulas as vector glyph-outline/rule paths (`pdf::draw_scene`), with Helvetica metrics used for text runs so inline math lands in the right place. Rasterized `pdftoppm` checks show correct stacking, delimiters and prose spacing |
| **M4 — Hardening** | **Done** | Corpus parse test, truncation fuzz, exact geometry snapshots, checked-in SVG goldens, a typesetting budget, and a KaTeX 0.16.45 reference-image ink-overlap test (`tests/katex_golden.rs`, 7 fixtures, IoU 0.61–0.83) |

Inline math now uses the measured flow engine in `dragonglass-ui/src/flow.rs`: each word is
shaped with GPUI's text system, formulas are atomic boxes placed on the same baseline, line
breaking uses measured widths, and selection maps back to the paragraph's display text.
Selection includes formulas atomically, copying preserves the original `$…$` TeX source, and
hovering a formula shows that source in a tooltip (both display blocks and inline boxes).

---

## 1. Summary

Dragon Glass renders LaTeX math as Unicode text today (`\frac{a}{b}` → `a/b`,
`\lceil x \rceil` → `⌈ x ⌉`). That is a readability aid, not typesetting: fractions are not
stacked, radicals do not stretch, delimiters do not grow, and anything the symbol table does
not know leaks into the document as a command name. In a vault that is mostly technical
writing this is a core-quality problem, not a cosmetic one.

This spec replaces the transliterator with a **real native typesetting pipeline**:

```
TeX source
  → ratex-parser   (KaTeX-compatible AST, macro expansion, mhchem, environments)
  → ratex-layout   (KaTeX layout rules, stretchy delimiters, display list)
  → DisplayList    (absolute glyph outlines, rules, paths, in em)
  → GPUI painter   (vector paths + quads, cached; no JS, no images, no WebView)
```

The engine is **RaTeX** (`ratex-*` crates, MIT), a pure-Rust KaTeX port that produces a
platform-neutral display list. Dragon Glass keeps its own thin wrapper (`math::typeset`) and
its own GPUI backend so the UI stays native and the vault format stays plain text.

**Headline decision:** adopt RaTeX rather than writing a TeX layout engine, and paint its
display list directly in GPUI rather than rasterizing to images.

---

## 2. Goals

1. **Correct math**, not approximate math: stacked fractions, radicals with rules, growing
   delimiters, operators with limits, matrices/`cases`/`aligned`, accents, fonts, spacing.
2. **Inline math** in paragraphs, headings, list items, table cells, quotes and callouts, with
   the math box sitting on the same baseline as the surrounding text.
3. **Display math** centred in reading view, scaled down when wider than the text column.
4. **Offline and native.** No JS runtime, no WebView, no network at render time.
5. **One engine everywhere:** the app, HTML export and PDF export consume the same layout
   output, so a formula cannot look different in the three places.
6. **No data loss and no hard failures.** Malformed or unsupported TeX must never prevent
   reading, editing or saving a note; it degrades to a visible, copyable fallback.
7. **Fast and memory-bounded.** Re-render on scroll must be a cache lookup, not a layout.

## 3. Non-goals

- Executing LaTeX macros that write files, include images or run shell commands.
- `\includegraphics`, `\htmlClass`, `\htmlStyle` and other DOM/trust extensions.
- A TeX document compiler (page breaking, counters, cross-references, bibliography styles).
- Math in the **source editor** (live preview while typing). Source mode keeps syntax
  highlighting; math renders in reading view and the split preview.
- Editable WYSIWYG math (MathQuill-style). A source editor for formulas is a later feature.
- Mermaid / diagrams (separate, explicitly out of scope for v1).

---

## 4. Current state and why it fails

`math::to_unicode` transliterates:

| Input | Today | Expected |
|---|---|---|
| `\frac{a+b}{2}` | `(a+b)/2` | stacked fraction |
| `\sqrt{x+1}` | `√(x+1)` | radical with vinculum over the radicand |
| `\sum_{k=1}^{n} k` | `∑ₖ₌₁ⁿ k` | ∑ with limits above/below (display) |
| `\left(\frac{a}{b}\right)` | `(a/b)` | parentheses that grow to the fraction |
| `\begin{pmatrix} a & b \\ c & d \end{pmatrix}` | `a  b \n c  d` | matrix with brackets and centred columns |
| `\lceil x \rceil` | `⌈ x ⌉` (Unicode, cannot grow) | true delimiters, stretchy |
| `\unknowncmd{x}` | `unknowncmdx` | not displayed as garbage; parse error UI |

The real vault used for testing (`test-vault`, 106 notes) contains **3,122 inline and 427
display formulas**. They exercise `\frac`, `\sqrt`, `\sum`, `\int`, `\mathcal`, `\mathbb`,
`\operatorname`, `\left…\right`, `aligned`, `cases`, `bmatrix`, `\pmod`, accents, `\text`,
`\tag` and `\ce`. Transliteration cannot cover this set credibly.

---

## 5. Requirements

### 5.1 Functional

- **FR1** Inline `$…$`, display `$$…$$`, `\[…\]`, `\(…\)`; `\$` is a literal dollar.
- **FR2** KaTeX-compatible syntax for the commands and environments in §11. Unknown
  commands produce an inline error badge, not silently mangled text.
- **FR3** Inline math is an atomic inline box: breaks allowed before/after it, never inside
  it; baseline-aligned within 0.5 device px of the text baseline.
- **FR4** Display math is centred, with `\tag`/numbering rendered on the right when present.
- **FR5** Theme colours apply (text, muted, accent); explicit `\color{…}` wins.
- **FR6** Selection and copy preserve the **TeX source** (`$E = mc^2$`), not the Unicode
  approximation. Selecting across a formula takes it atomically.
- **FR7** Hovering a formula shows its source in the status bar/tooltip (discoverability and
  a quick check that the renderer saw the intended TeX).
- **FR8** HTML export emits self-contained inline SVG per formula (vector, embedded glyph
  outlines, `data-tex` + `aria-label` attributes and a `<title>` with the TeX).
- **FR9** PDF export emits vector paths for the same display list; no font embedding needed.
- **FR10** Zoom/DPI: vector output at any scale; no pixelation when the UI scales.
- **FR11** Copying a note as Markdown keeps the original source untouched.

### 5.2 Quality / performance budgets

| Metric | Target |
|---|---|
| Cold layout, typical inline formula | ≤ 5 ms |
| Cold layout, complex display formula | ≤ 20 ms |
| Warm render (cache hit) | ≤ 0.2 ms per formula |
| Scroll of a 100-formula note | no layout work, only cached scenes painted |
| Math cache memory, 1,000-formula document | ≤ 32 MB |
| Reading-mode frame time while scrolling | stays within the existing 60 fps budget |
| Baseline alignment error | ≤ 0.5 px |
| Panic on arbitrary TeX | zero (fuzz/truncation tests) |
| Parse success on the test-vault corpus | 100% of 3,122 + 427 formulas |

### 5.3 Platforms

Windows and Linux first (the existing targets), macOS when the app targets it. The painter is
GPU-agnostic GPUI path/quads, so it inherits GPUI's platform support.

---

## 6. Options considered

| Option | Verdict |
|---|---|
| **Keep transliteration, grow the symbol table** | Cannot scale fractions, radicals or delimiters. Rejected as an end state; retained only as a text fallback for copy/accessibility. |
| **`katex` / `katex-gdef-v8` crates** | Embed a JavaScript engine (QuickJS/V8) to run KaTeX. Adds a JS runtime, startup cost and a large native dep; the `katex` crate only wires QuickJS on GNU toolchains, so MSVC Windows builds are awkward. Contradicts the "not Electron" premise. Rejected. |
| **`katex-rs` (pure Rust port)** | No JS, but its output is HTML/DOM-oriented (web-sys/phf, no font-metrics or display-list dependency), so a GPUI integration would still need a layout layer. Kept as a fallback if RaTeX stalls. |
| **`ratex-gpui` crate** | Closest fit, but depends on `gpui-pre 0.3`; Dragon Glass is on `gpui 0.2.2`. It rasterizes to an image, which loses zoom crispness and selection. Use as a reference implementation, not a dependency. |
| **Rasterize with `ratex-render` (PNG)** | Trivial to integrate but blurry when scaled, adds image decode cost, and inline baseline alignment is a bitmap offset. Rejected as the primary path; kept as a per-formula fallback. |
| **`ratex-*` layout + own GPUI vector painter** | **Chosen.** KaTeX-compatible parsing/layout for free, native crisp output, one display list feeding app/HTML/PDF. |

---

## 7. Architecture

### 7.1 Crate boundaries

```
dragonglass-core
  math/
    mod.rs        public API: typeset(), MathScene, MathError, MathStyle
    engine.rs     RaTeX wrapper (parse → layout → display list)
    plain.rs      existing to_unicode(), renamed plain_text(); copy/fallback only
    outline.rs    glyph outlines from the bundled KaTeX faces (UI painter + PDF export)
    export.rs     DisplayList → SVG/PDF emitters (feature-gated, see §13)

dragonglass-ui
  math_element.rs  GPUI Element that paints a MathScene (vector, cached)
  flow.rs          inline flow layout: text runs + atomic math boxes
  reading.rs       display math block, paragraphs via flow
  preview_window.rs  (unchanged; reading cache already shared)
```

Core remains headless and unit-testable: no `gpui` dependency, no fonts inside the engine
wrapper. `MathScene` is the boundary.

### 7.2 Core API

```rust
// crates/dragonglass-core/src/math/mod.rs

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MathStyle {
    Inline,   // text style: small operators, scripts at script size
    Display,  // display style: limits above/below, larger operators
}

#[derive(Clone, Copy, Debug)]
pub struct MathOptions {
    pub style: MathStyle,
    /// Base colour (RGBA) recorded in the display list; explicit `\color` overrides it.
    pub color: [f32; 4],
}

#[derive(Clone)]
pub struct MathScene {
    /// Re-exported as `math::DisplayList` so the RaTeX dependency can be swapped without
    /// touching every call site.
    pub display: DisplayList, // em units, origin top-left, baseline y = height
    pub source: String,       // original TeX, for copy/hover/errors
}

#[derive(Debug, thiserror::Error)]
pub enum MathError {
    #[error("unsupported LaTeX: {0}")]
    Parse(String),
    #[error("formula is too large to lay out")]
    TooLarge,
}

pub fn typeset(tex: &str, options: &MathOptions) -> Result<MathScene, MathError>;
```

Coordinate system (from RaTeX): x right, y down, origin at the top-left of the bounding box,
**baseline at `y = height`**. All lengths in em; multiply by the font size to get px. Total
inline box = `width × (height + depth)`.

**The scene is font-size independent.** Layout is entirely in em, so one scene serves 14 px,
18 px and 200% zoom; only the paint transform changes. This is why the cache key has no size
in it.

### 7.3 Pipeline and caching

```
document version ──► ReadingCache::update()
                       ├─ markdown::parse_blocks()
                       ├─ for each Block::Math        → typeset(Display)
                       └─ for each inline math span   → typeset(Inline)
                     → Rc<Vec<Block>> + HashMap<span-id, Rc<MathScene>>
render(width) ──► flow::layout(text runs + scenes, width)  [per width change]
                └─ paint cached scenes / shaped text
```

Typesetting happens when the **document changes**, not in `render`. `render` only does flow
layout when the content width changes and paints cached geometry. This mirrors the existing
`ReadingCache`/`heading` caching strategy and keeps scrolling free of math work.

Cache key: `(tex, style, color)`; the scene is size-independent, so resizing the window or
changing theme only affects prepared paint geometry. LRU bounded by bytes (scenes are small;
budget 32 MB). Clear on vault switch.

Embedded/transcluded notes are read and parsed inside `render_embed` today, not in the
document cache. Their formulas still go through the same `MathCache`, so the first paint of an
embed pays layout once and later frames hit the cache; as a follow-up, fold embedded file
versions into the document version so an external edit refreshes scenes without a full reload.

---

## 8. Fonts

RaTeX bundles KaTeX's TTF faces (`ratex-katex-fonts`, 19 faces: Main-Regular, Math-Italic,
Size1–Size4, AMS, Caligraphic, SansSerif, Typewriter, Script, Fraktur, …).

- Depend on the bundled font crate; do **not** read from the system or require a KaTeX install.
- Font files are **SIL OFL 1.1**; the code is MIT. Add `THIRD_PARTY_NOTICES.md` and ship the
  licences in `packaging/` for Windows, `.deb`, Flatpak and AppImage. GPL-3.0 is compatible.
- Font bytes are parsed once with `ab_glyph`/`ttf-parser`. Outlines are cached by
  `(FontId, char_code)` in font units; transforms happen at paint time.
- `ratex-font-loader` already ships an outline cache, but it is public-surface thin; the
  `math::outline` module owns glyph lookup so both the GPUI painter and the PDF writer use the
  same outlines. `ratex_font::katex_ttf_glyph_char(font_id, char_code)` maps the display-list
  `char_code` to the character to look up in that face (this is how RaTeX's own rasterizer
  does it).
- CJK/emoji inside `\text{…}`: out of scope for the first delivery; fall back to the system
  font via GPUI text (see §12) or render the whole formula as a raster fallback.

Bundled font weight: ~1–2 MB, acceptable for the installer.

---

## 9. GPUI painter

A custom `Element` (not a `div`) so layout and painting have exact control.

```rust
pub struct MathElement {
    scene: Rc<MathScene>,
    font_size: Pixels,
    /// Paint colour for items whose colour equals the layout default.
    color: Hsla,
}

impl Element for MathElement {
    // request_layout: size = (width*fs, (height+depth)*fs)
    // paint: emit quads/paths at origin
}
```

### 9.1 Display-list mapping

| Item | GPUI primitive | Notes |
|---|---|---|
| `Line { x, y, width, thickness }` | `window.paint_quad(quad(..))` | `y` is the centre; thickness ≥ 1 device px; dashed lines become a run of quads (4·t dash, 2·t gap, matching RaTeX). |
| `Rect { x, y, width, height }` | `paint_quad` | `\colorbox` backgrounds. |
| `Path { x, y, commands, fill }` | `gpui::Path` + `paint_path` | Scale commands by `em`, translate by `(x,y)`. Split at every `MoveTo` and paint each subpath separately (stretchy arrows are assembled from oppositely wound components; a single path cancels). GPUI exposes no fill-rule knob; separate primitives reproduce RaTeX's visual result. Validate `\left(…\right)`, `\xrightarrow`, `\sqrt`. |
| `GlyphPath { x, y, scale, font, char_code }` | outline → `gpui::Path` + `paint_path` | `scale` multiplies the em size for that glyph. `math::outline` returns the curves (font units); the UI flips y, scales by `font_size*scale/units_per_em` and translates. Cache raw outlines; build prepared paths once per scene. |

Why outlines instead of GPUI text shaping: the layout engine has already decided every glyph
position and size (including scalings for script/size styles). Re-shaping the character through
the platform text stack would re-introduce kerning/advance differences and break alignment with
fraction bars and radicals. Outlines give pixel-faithful KaTeX geometry at any scale.

Hairlines: GPUI quads snap to device pixels, so fraction bars and radical rules must be checked
at 125%/150% DPI. If a quad disappears or doubles, paint that rule as a 1-device-pixel path
instead of a quad.

### 9.2 Prepared geometry

`MathScene` is EM-based and reusable. The UI builds a `PreparedMath` once per
`(scene, font_size, color)`:

```rust
pub struct PreparedMath {
    size: Size<Pixels>,
    ops: Vec<MathOp>, // Quad(..) | Path(gpui::Path<Pixels>)
}
```

Painting a frame is then a loop over `ops`. This keeps scroll frames allocation-free apart from
the temporary element.

### 9.3 Baseline placement

- **Display block:** box top-left at the block's top; centre horizontally.
- **Inline:** the flow engine places the box so `box_top + height*font_size == baseline`.
  The flow records `ascent = height*fs`, `descent = depth*fs`, and expands the line box to
  `max(text_ascent, math_ascent) + max(text_descent, math_descent)`.

### 9.4 Width overflow

Display formulas wider than the content column are scaled down at paint time (a smaller
effective font size), with a floor of 60%; below that, the block scrolls horizontally. No
re-layout is needed. Inline formulas wider than the column are allowed to overflow visually (no
mid-formula break), consistent with KaTeX in a browser.

---

## 10. Inline math: the flow engine

This is the only genuinely new UI machinery. Today a paragraph is a single `StyledText` with
runs; an atomic, baseline-aligned box cannot live inside it. We add a small flow layout for
rich paragraphs.

```rust
pub enum FlowItem {
    Text { text: String, style: InlineStyle, href: Option<String> },
    Math { scene: Rc<MathScene> },   // atomic
    LineBreak,
}

pub struct FlowLine {
    items: Vec<PlacedItem>,          // x offset + shaped line / math scene
    ascent: Pixels,
    descent: Pixels,
}
```

Algorithm:

1. Split text into words at spaces (keep trailing spaces for wrapping decisions); math is a
   single atom; inline code keeps its own spaces and may break only between them, like today.
2. Shape text words with `Window::text_system()` at the paragraph font/size; measure.
3. Greedy line breaking at spaces (the app is left-aligned, not justified).
4. Line height = max of text line height and each math box's ascent/descent.
5. Emit shaped lines + math ops in paint order.

Scope of replacement: paragraphs, headings, list items, table cells, quotes, callouts,
footnote bodies, and transcluded/embedded note paragraphs. Display blocks stay in
`render_block`.

Selection/copy:

- Each paragraph exposes `copy_text()` = concatenation of text and `$source$`.
- Byte offsets map to `(item, offset)`; a math item accepts the whole range or nothing.
- The existing `SelectableText` element becomes `SelectableFlow`; drag selection, Ctrl+A,
  Ctrl+C and the status-bar selection stats keep working.
- Clicking a math box does not move the caret in reading mode (there is no caret); it can
  open a "copy TeX" context action later.

Risk control: keep the current `StyledText` renderer behind a temporary feature flag until the
flow engine passes the reading-view regression checklist (tables, links, highlights,
strikethrough, footnotes, code spans, alt text).

---

## 11. Syntax scope

Supported by RaTeX and therefore in scope from day one:

- Fractions, binomials, roots (incl. `\sqrt[n]`), sub/superscripts, primes.
- Sums, products, integrals, limits, `\operatorname`, `\text`, `\mathrm`, `\mathbf`,
  `\mathit`, `\mathcal`, `\mathbb`, `\mathfrak`, `\mathtt`, `\boldsymbol`.
- Delimiters: `\left…\middle…\right`, `\big…\Bigg`, `\{`, `\langle`, `\lvert`, and the
  stretchy forms (parens, brackets, braces, bars, angle brackets, arrows).
- Accents: `\hat`, `\bar`, `\overline`, `\tilde`, `\vec`, `\dot`, `\ddot`, `\check`.
- Matrices/arrays, `cases`, `aligned`, `alignedat`, `gathered`, `split`, `smallmatrix`.
- Numbered environments and `\tag`, `\nonumber`/`\notag` (KaTeX-style).
- Colour: `\color`, `\textcolor`, `\colorbox`.
- Extensions: `\ce`/`\pu` (mhchem), `\slashed`, `\centernot`, `prooftree` (bonus).
- Escapes: `\%`, `\&`, `\$`, `\#`, `\_`, `\{`, `\}`, `\\`, `\,`, `\;`, `\quad`.

Delimiters in Markdown: `$…$`, `$$…$$` (block), `\[…\]`, `\(…\)`. The existing rule that an
opening `$` must be followed by a non-space and a closing `$` preceded by a non-space is kept
to avoid turning prices into math.

Out of scope (documented in the UI as unsupported, not silently dropped):
`\includegraphics`, `\htmlClass`/`\htmlData`/`\htmlId`/`\htmlStyle`, `\input`, `\include`,
`\write18`, and document-level counters.

---

## 12. Errors, fallback and accessibility

- **Parse/layout error:** render the source in a muted monospace span with a small warning
  marker (e.g. `⚠`), tooltip = error. The note remains editable and saveable.
- **Renderer fallback:** if a specific glyph has no outline, try the system font via GPUI text;
  if a complex display environment still fails to paint, rasterize that one formula with
  `ratex-render` at the current DPR and draw it as an image (cached). Never panic in `render`.
- **Hover:** show the TeX source in the status bar (and a tooltip), so a wrong render can be
  diagnosed immediately.
- **Copy:** source form (see §10). Add "Copy as LaTeX" and "Copy as Unicode" to the command
  palette later.
- **Plain-text consumers:** the outline pane and search snippets use `plain_text(tex)` (the
  existing transliteration) so `\frac{a}{b}` reads `a/b` rather than raw TeX. Hover still shows
  the source (above). This is the only remaining use of the old converter in the reading path.
- **Alt text:** exported HTML sets `data-tex` and an `aria-label`/`<title>` with the TeX; the UI
  element exposes the same string as its accessibility label when GPUI supports it. Full MathML
  is a later enhancement.

---

## 13. Export parity

### HTML (`crates/dragonglass-core/src/html.rs`)

- Inline: `<span class="math math-inline" data-tex="...">{svg}</span>`
- Display: `<div class="math math-display" data-tex="...">{svg}</div>`
- SVG comes from `ratex_svg::render_to_svg` with `SvgOptions { embed_glyphs: true, .. }`
  (requires the `embed-fonts`/`standalone` feature), so exported pages need no CSS,
  webfonts, JS or network. This also fixes math in exports on machines without KaTeX fonts.
- The exporter already copies attachments; math is self-contained. HTML-escape `data-tex`
  (`&`, `<`, `>`, `"`) so pathological TeX cannot break the page.

### PDF (`crates/dragonglass-core/src/pdf.rs`)

Display formulas are drawn from the same display list into the page content stream: glyph
outlines become cubic paths, rules become rectangles, and stretchy-delimiter shapes become
filled/stroked paths (even-odd for assembled shapes). No font embedding is needed because
glyphs are outlines. A display formula occupies as many grid rows as its height needs, so
pagination stays correct. Inline formulas use the same renderer: prose is laid out as
positioned Helvetica runs (using the font's AFM advance widths) and each math box is drawn at
the next run position on the text baseline, so paragraphs mix text and vector math.

### CLI

`dragon-glass export` and `dragon-glass pdf` use the same core functions; add a `--math=none`
escape hatch that falls back to `plain_text` if a vault has pathological formulas.

---

## 14. Testing

### 14.1 Core

- **Command matrix:** one test per command/environment in §11 asserting non-empty layout,
  finite dimensions and expected ascent/descent ordering (e.g. display `\sum` is taller than
  inline `\sum`).
- **Geometry snapshots:** width/height/depth for a curated set of formulas in
  `math::engine::tests::golden_geometry`; a spacing or metrics change fails the test.
- **Corpus:** every display formula in `test-vault` typesets (local test; skipped without the
  vault). Inline formulas are typeset into the reading cache and exercised by the UI.
- **Robustness:** truncate real formulas at multiple byte boundaries; `typeset` must return
  `Err` or a scene, never panic.
- **Golden images (vector):** `tests/math_golden.rs` renders a fixed set through the same
  display list → SVG path used by HTML export and compares against checked-in files in
  `crates/dragonglass-core/tests/math-golden/`. Regenerate intentionally with
  `UPDATE_MATH_GOLDEN=1`.
- **Performance budget:** 500 inline formulas must typeset well under 5 s in a debug build;
  the assertion is a regression guard, not a benchmark.
- **KaTeX reference images:** `tests/katex_golden.rs` compares our render against seven
  KaTeX 0.16.45 fixtures from RaTeX's published golden suite (scripts, Greek, fraction, matrix,
  radical, accent, sum). Both images are cropped to their ink bounding box and resampled to a
  96×96 mask, then scored by IoU (threshold 0.55; observed 0.61–0.83). This is a coarse
  ink-overlap check; the authoritative `ratex-ink-v2` harness is external.

### 14.2 UI

- Unit-test the display-list → `MathOp` mapper: positions, scale, subpath splitting, colour.
- Flow layout unit tests: line breaks around math, ascent/descent, atomicity, copy text.
- Manual visual checklist (screenshot diffs) for: inline in heading/list/table/quote/callout,
  display with `\tag`, wide matrix, `\left(...\right)` taller than a line, dark/light theme,
  125%/150% DPI.

### 14.3 Acceptance gate

- 100% of the vault corpus typesets.
- Sample visual diff passes; no glyph spills outside its line box.
- Inline baseline error ≤ 0.5 px.
- Warm scroll of the "Advanced Data Structures" article (60+ formulas) shows no layout spike.
- HTML export opens offline with math visible; PDF math is vector at 400% zoom.
- Malformed note (`$ \frac{1}{$`) still opens, edits and saves; the error is visible.

---

## 15. Rollout plan

| Phase | Work | Exit criteria | Est. |
|---|---|---|---|
| **M0 — Spike** | Add pinned `ratex-*` deps, typeset one formula in a core test, dump the display list; confirm the font crate and outline APIs | A unit test prints/validates a `DisplayList` for `\frac` and `\sum` | 0.5 d |
| **M1 — Display math** | `math::typeset`, `MathElement`, glyph/path/line painter, prepared-scene cache, replace `Block::Math`; fonts + notices | Display math in reading view and preview window; wide-formula scaling; goldens for 20 formulas | 3 d |
| **M2 — Inline flow** | `flow.rs` (measure, break, baseline), `SelectableFlow`, migrate paragraphs/headings/lists/tables/quotes/callouts; keep old path behind a flag until parity | Inline math everywhere in reading view, selection/copy works; baseline tests pass | 5–7 d |
| **M3 — Export parity** | `ratex-svg` in HTML export; outline emission in PDF; CLI flags | Exported HTML offline-correct; PDF vector math | 3 d |
| **M4 — Hardening** | Corpus + truncation tests, visual diff harness, perf pass, error UI, hover status, accessibility label, docs | Acceptance gate §14.3 | 3–5 d |

Total ≈ 3–4 weeks of solo work; M1 alone is a shippable improvement and should land first.
No vault-format or packaging changes beyond the font notices.

---

## 16. Risks and mitigations

| Risk | Mitigation |
|---|---|
| RaTeX is 0.1.x and iterates fast (24 releases in ~7 months) | Pin exact versions (`=0.1.15`), wrap behind `math::typeset`, keep the wrapper small; vendor the crates (MIT) if a release breaks us. |
| GPUI's `paint_path` has no fill-rule/stroke controls | Split subpaths at `MoveTo`; validate against goldens; fall back to a raster scene for the specific formula if a shape is wrong. |
| Inline flow regresses selection, tables or links | Feature-flag the old renderer; expand the reading-view regression checklist before switching; land M2 behind an env var for dogfooding. |
| Font licensing/packaging mistakes | OFL notice + licence files in `packaging/`; CI check that the notices are present. |
| Layout cost on very long notes | Typeset in `ReadingCache::update`, virtualized blocks only, LRU by bytes; measure with the corpus. |
| CJK/emoji inside `\text{}` | System-font fallback or raster fallback; documented limitation until RaTeX's unicode fallback is wired to GPUI. |
| KaTeX visual drift | Golden-image tests against KaTeX references on the same corpus; track the RaTeX support table. |

---

## 17. Open questions

1. Do we want **equation numbering across a document** (continuous counters), or per-formula
   numbering? RaTeX numbers within one formula today.
2. Should inline math be **selectable atomically** (planned) or expanded to its source in the
   copy buffer only when the selection fully covers it? (Obsidian copies `$…$`; match that.)
3. Do we expose a **"math font" setting** (KaTeX vs. a system math font later), or keep one
   bundled face for fidelity?
4. Do we upstream the GPUI vector backend to `ratex-gpui` (currently raster on `gpui-pre 0.3`)
   to reduce our maintenance surface? Worth revisiting if we move to GPUI 0.3.
5. Is `\ce`/`\pu` wanted in v1? In RaTeX 0.1.15 mhchem is always compiled into
   `ratex-parser` (no feature flag), so it is available at no integration cost; the only
   question is whether to advertise it.

---

## Appendix A — Display list reference (RaTeX 0.1.15)

```rust
pub struct DisplayList { items: Vec<DisplayItem>, width: f64, height: f64, depth: f64 }

pub enum DisplayItem {
    GlyphPath { x, y, scale, font: String, char_code: u32, color: Color },
    Line { x, y, width, thickness, color, dashed: bool },
    Rect { x, y, width, height, color },
    Path { x, y, commands: Vec<PathCommand>, fill: bool, color },
}

pub enum PathCommand { MoveTo{x,y}, LineTo{x,y}, CubicTo{x1,y1,x2,y2,x,y}, QuadTo{x1,y1,x,y}, Close }
```

All coordinates are f64 em units in a top-left origin, baseline at `y = height`.

## Appendix B — Dependency sketch

```toml
# crates/dragonglass-core/Cargo.toml
ratex-types       = "=0.1.15"
ratex-parser      = "=0.1.15"          # mhchem is built in for 0.1.15
ratex-layout      = "=0.1.15"
ratex-font        = "=0.1.15"
ratex-font-loader = { version = "=0.1.15", features = ["embed-fonts"] }
ratex-svg         = { version = "=0.1.15", features = ["embed-fonts"] }   # HTML export
ab_glyph          = "0.2"              # glyph outlines for the painter
```

Verified against the published 0.1.15 metadata: `ratex-font-loader` exposes `embed-fonts`
(which pulls `ratex-katex-fonts`), and `ratex-svg`'s `embed-fonts` implies `standalone`, which
is what enables outline glyphs in the SVG. `ratex-*` is MIT; the bundled fonts are OFL 1.1.

## Appendix C — File map

| File | Change |
|---|---|
| `crates/dragonglass-core/src/math.rs` | Split into `math/{mod,engine,plain,outline,export}.rs`; `to_unicode` → `plain::to_unicode` |
| `crates/dragonglass-core/src/inline.rs` | Keep the raw TeX on math spans instead of storing transliterated text |
| `crates/dragonglass-core/src/markdown.rs` | `Block::Math` unchanged; add `\[…\]`/`\(…\)` recognition |
| `crates/dragonglass-core/src/html.rs` | Inline SVG per formula |
| `crates/dragonglass-core/src/pdf.rs` | Emit display-list paths into the content stream |
| `crates/dragonglass-ui/src/math_element.rs` | New GPUI element + prepared-scene cache |
| `crates/dragonglass-ui/src/flow.rs` | New inline flow layout + selection mapping |
| `crates/dragonglass-ui/src/reading.rs` | Display math uses `MathElement`; rich blocks use `Flow` |
| `crates/dragonglass-ui/src/editor.rs` | Reading-mode render delegates to `reading` (already does); preview uses the same |
| `packaging/**` | Font licence notices |
| `PLAN.md` | Tech-stack row and gap list point at this spec |
