//! Reading mode: render parsed markdown blocks as styled GPUI elements, including
//! embedded images and transcluded notes.
//!
//! Inline text goes through `dragonglass_core::inline`, so emphasis, code, links, math
//! and highlights all render inside paragraphs, list items, headings and table cells.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::SystemTime;

use dragonglass_core::assets::Attachments;
use dragonglass_core::find as find_engine;
use dragonglass_core::frontmatter::{self, Property};
use dragonglass_core::inline;
use dragonglass_core::markdown::{self, Align, Block};
use dragonglass_core::math::{self, MathOptions, MathScene, MathStyle};
use dragonglass_core::shared;
use dragonglass_core::syntax;

use gpui::{
    anchored, div, fill, img, list, point, prelude::*, px, relative, AnyElement, App, Bounds,
    ClipboardItem, Corner, CursorStyle, DispatchPhase, Element, ElementId, Entity, FontStyle,
    GlobalElementId, Hitbox, HitboxBehavior, InspectorElementId, IntoElement, LayoutId,
    ListAlignment, ListState, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad,
    Pixels, Point, RenderImage, StrikethroughStyle, StyledText, TextLayout, TextRun,
    UnderlineStyle, Window,
};

use crate::flow::{FlowContent, FlowKind, FlowPiece, FlowText};
use crate::math_element::MathElement;
use crate::theme::Theme;

/// Guard against `![[a]]` â†’ `a` â†’ `![[a]]` cycles.
const MAX_EMBED_DEPTH: usize = 3;

/// Called when a link in the reading view is clicked. The target is either a vault-relative
/// note target or an `http(s)`/`mailto` URL; the app decides which.
pub type LinkOpener = Rc<dyn Fn(&str, &mut Window, &mut App)>;

/// Called when a heading's fold chevron is clicked. The index is the heading's block.
pub type FoldToggle = Rc<dyn Fn(usize, &mut Window, &mut App)>;

/// Typography for the reading view, stored as a GPUI global so open readers pick up a
/// change on the next frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReaderSettings {
    /// Body text size in points.
    pub body_size: f32,
    /// Maximum width of the reading column, in pixels.
    pub measure: f32,
}

impl Default for ReaderSettings {
    fn default() -> Self {
        Self {
            body_size: 15.0,
            measure: 780.0,
        }
    }
}

impl gpui::Global for ReaderSettings {}

/// Writes text to the clipboard for the reading view's copy buttons.
pub type CopyHandler = Rc<dyn Fn(&str, &mut Window, &mut App)>;

/// Opens an image in the reading lightbox.
pub type ImageOpener = Rc<dyn Fn(&str, &mut Window, &mut App)>;

/// Opens a non-image attachment with the operating system's default application.
pub type FileOpener = Rc<dyn Fn(&str, &mut Window, &mut App)>;

/// A point in the reading view: a block and a byte offset into its display text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ReadPoint {
    pub block: usize,
    pub offset: usize,
}

/// One in-page find hit: a block and a byte range in its display text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadingMatch {
    pub block: usize,
    pub range: Range<usize>,
}

/// The reading view's own find state. It searches block display text, so the note is never
/// touched and the mode never changes.
#[derive(Clone, Debug)]
struct ReadingFind {
    open: bool,
    pattern: String,
    options: find_engine::Options,
    matches: Vec<ReadingMatch>,
    current: usize,
    error: Option<String>,
    /// Top block when the bar opened, restored by Escape.
    return_block: usize,
}

impl Default for ReadingFind {
    fn default() -> Self {
        Self {
            open: false,
            pattern: String::new(),
            options: find_engine::Options {
                regex: false,
                case_sensitive: false,
                whole_word: false,
            },
            matches: Vec::new(),
            current: 0,
            error: None,
            return_block: 0,
        }
    }
}

/// A keyboard motion that extends the reading selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionMotion {
    Left,
    Right,
    Up,
    Down,
    LineStart,
    LineEnd,
    DocumentStart,
    DocumentEnd,
}

/// Character-level selection shared by every text piece in the reading view.
#[derive(Debug, Default)]
pub struct ReadSelection {
    pub(crate) anchor: Option<ReadPoint>,
    pub(crate) head: Option<ReadPoint>,
    /// Where the drag started, so a slightly shaky click still counts as a click.
    pub(crate) down: Option<Point<Pixels>>,
    pub(crate) dragging: bool,
    pub(crate) moved: bool,
}

impl ReadSelection {
    pub(crate) fn clear(&mut self) {
        self.anchor = None;
        self.head = None;
        self.down = None;
        self.dragging = false;
        self.moved = false;
    }

    /// The selection with its ends in document order.
    pub(crate) fn ordered(&self) -> Option<(ReadPoint, ReadPoint)> {
        let anchor = self.anchor?;
        let head = self.head?;
        if anchor == head {
            return None;
        }
        Some(if anchor <= head {
            (anchor, head)
        } else {
            (head, anchor)
        })
    }
}

/// Counts for the status bar while a reading-mode selection exists.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SelectionStats {
    pub words: usize,
    pub characters: usize,
    pub lines: usize,
}

/// Everything a block needs to render, threaded through the recursive embed renderer.
/// A parsed transcluded note, keyed by path and fragment and revalidated by mtime.
struct EmbeddedNote {
    modified: Option<SystemTime>,
    blocks: Rc<Vec<Block>>,
}

type EmbedCache = Rc<RefCell<HashMap<(PathBuf, Option<String>), EmbeddedNote>>>;

/// Typeset formulas, keyed by TeX source and style.
type MathScenes = HashMap<(String, MathStyle), Rc<MathScene>>;
/// Formulas typeset for embedded notes, filled lazily during render.
type EmbedMath = Rc<RefCell<MathScenes>>;

struct RenderCtx<'a> {
    vault_root: Option<&'a Path>,
    attachments: Option<&'a Attachments>,
    selection: &'a Rc<RefCell<ReadSelection>>,
    opener: Option<&'a LinkOpener>,
    /// Display formulas, typeset once per document version.
    math: &'a HashMap<(String, MathStyle), Rc<MathScene>>,
    /// Folded heading blocks in the current reading view.
    folded: &'a HashSet<usize>,
    /// Called with a heading's block index when its fold chevron is clicked.
    on_fold: Option<&'a FoldToggle>,
    /// Copies text to the clipboard (code fences have a copy button).
    copy: Option<&'a CopyHandler>,
    /// In-page find, so matches can be painted behind the text.
    find: Option<&'a ReadingFind>,
    /// Opens a clicked image in the lightbox.
    on_image: Option<&'a ImageOpener>,
    /// Opens a PDF/audio/video attachment with the OS.
    open_file: Option<&'a FileOpener>,
    /// Parsed embedded notes, cached by path and mtime.
    embeds: Option<&'a EmbedCache>,
    /// Formulas typeset for embedded notes, filled lazily.
    embed_math: Option<&'a EmbedMath>,
    /// Allocator for synthetic block ids inside embeds.
    next_block: Option<&'a Cell<usize>>,
    /// The reading list, so dragging past the viewport can scroll it.
    list_state: Option<&'a ListState>,
    /// How many blocks the list holds, to clamp drag auto-scroll.
    block_count: usize,
    /// All blocks, so a heading can tell whether it has a nested section.
    blocks: &'a [Block],
    depth: usize,
    theme: Theme,
    template: &'a TextRun,
    /// Body text size for this reader.
    body_size: f32,
    /// Maximum width of the reading column.
    measure: f32,
}

/// One place a footnote is referenced, used by the footnote picker.
#[derive(Clone)]
pub(crate) struct FootnoteRef {
    pub block: usize,
    pub snippet: String,
}

/// The small modal shown when a footnote marker is clicked: every reference to that footnote
/// in the current note, plus a jump to the definition.
pub(crate) struct FootnotePopup {
    title: String,
    refs: Rc<RefCell<HashMap<String, Vec<FootnoteRef>>>>,
    defs: Rc<RefCell<HashMap<String, usize>>>,
    list_state: ListState,
    /// Asking the reader to unfold whatever hides a block before scrolling to it.
    reveal: Rc<RefCell<Option<usize>>>,
    /// Window position of the reference that opened the popup, so it can sit beside it.
    anchor: Option<Point<Pixels>>,
    open: Option<(String, Vec<FootnoteRef>, Option<usize>)>,
}

impl FootnotePopup {
    fn new(
        refs: Rc<RefCell<HashMap<String, Vec<FootnoteRef>>>>,
        defs: Rc<RefCell<HashMap<String, usize>>>,
        list_state: ListState,
        reveal: Rc<RefCell<Option<usize>>>,
    ) -> Self {
        Self {
            title: "This note".to_string(),
            refs,
            defs,
            list_state,
            reveal,
            anchor: None,
            open: None,
        }
    }

    fn set_title(&mut self, title: String, cx: &mut Context<Self>) {
        if self.title != title {
            self.title = title;
            if self.open.is_some() {
                cx.notify();
            }
        }
    }

    /// Open the popup for `id`, anchored at the clicked reference in window coordinates.
    fn show_at(&mut self, id: &str, anchor: Option<Point<Pixels>>) {
        let refs = self.refs.borrow().get(id).cloned().unwrap_or_default();
        let definition = self.defs.borrow().get(id).copied();
        self.anchor = anchor;
        self.open = Some((id.to_string(), refs, definition));
    }

    fn anchor(&self) -> Option<Point<Pixels>> {
        self.anchor
    }

    fn close(&mut self) {
        self.open = None;
        self.anchor = None;
    }

    fn is_open(&self) -> bool {
        self.open.is_some()
    }
}

impl Render for FootnotePopup {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.global::<Theme>();
        let Some((id, refs, definition)) = &self.open else {
            return div().into_any_element();
        };

        let this = cx.entity().downgrade();
        let close = this.clone();
        let close_button = div()
            .id("footnote-close")
            .px_2()
            .cursor_pointer()
            .text_color(theme.muted)
            .hover(move |style| style.bg(theme.hover))
            .child("x")
            .on_mouse_down(MouseButton::Left, move |_ev, window, cx| {
                if let Some(this) = close.upgrade() {
                    this.update(cx, |this, cx| {
                        this.close();
                        cx.notify();
                    });
                }
                window.refresh();
            });

        let mut panel = div()
            .id("footnote-popup")
            .w(px(460.0))
            .max_h(px(320.0))
            .overflow_y_scroll()
            .bg(theme.panel)
            .border_1()
            .border_color(theme.border)
            .rounded_md()
            .p_3()
            .flex()
            .flex_col()
            .gap_2()
            .text_sm()
            .text_color(theme.text)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .font_weight(gpui::FontWeight::BOLD)
                            .child(format!("Footnote {id}")),
                    )
                    .child(close_button),
            );

        let title = self.title.clone();
        for (index, reference) in refs.iter().enumerate() {
            let block = reference.block;
            let snippet = reference.snippet.clone();
            let title = title.clone();
            let state = this.clone();
            let reveal = self.reveal.clone();
            let list_state = self.list_state.clone();
            panel = panel.child(
                div()
                    .id(ElementId::Name(format!("footnote-ref-{index}").into()))
                    .flex()
                    .flex_col()
                    .gap_1()
                    .px_2()
                    .py_1()
                    .rounded_sm()
                    .cursor_pointer()
                    .hover(move |style| style.bg(theme.hover))
                    .child(div().child(format!("Title: {title}")))
                    .child(div().text_xs().text_color(theme.muted).child(snippet))
                    .on_mouse_down(MouseButton::Left, move |_ev, window, cx| {
                        // Scroll now when the target is visible; the reader unfolds and
                        // scrolls again on the next render when a fold hides it.
                        list_state.scroll_to_reveal_item(block);
                        *reveal.borrow_mut() = Some(block);
                        if let Some(this) = state.upgrade() {
                            this.update(cx, |this, cx| {
                                this.close();
                                cx.notify();
                            });
                        }
                        window.refresh();
                    }),
            );
        }

        if let Some(block) = definition {
            let block = *block;
            let state = this.clone();
            let reveal = self.reveal.clone();
            let list_state = self.list_state.clone();
            panel = panel.child(
                div()
                    .id("footnote-definition")
                    .px_2()
                    .py_1()
                    .rounded_sm()
                    .cursor_pointer()
                    .text_color(theme.accent_text)
                    .hover(move |style| style.bg(theme.hover))
                    .child("Go to footnote definition")
                    .on_mouse_down(MouseButton::Left, move |_ev, window, cx| {
                        // Scroll now when the target is visible; the reader unfolds and
                        // scrolls again on the next render when a fold hides it.
                        list_state.scroll_to_reveal_item(block);
                        *reveal.borrow_mut() = Some(block);
                        if let Some(this) = state.upgrade() {
                            this.update(cx, |this, cx| {
                                this.close();
                                cx.notify();
                            });
                        }
                        window.refresh();
                    }),
            );
        }

        if refs.is_empty() && definition.is_none() {
            panel = panel.child(
                div()
                    .text_xs()
                    .text_color(theme.muted)
                    .child("No references found in this note."),
            );
        }

        panel.into_any_element()
    }
}

/// A display title for the footnote picker: frontmatter title, first heading, or a fallback.
fn note_title(properties: &[Property], blocks: &[Block]) -> String {
    properties
        .iter()
        .find(|property| property.key.eq_ignore_ascii_case("title"))
        .map(|property| property.value.display())
        .filter(|title| !title.trim().is_empty())
        .or_else(|| {
            blocks.iter().find_map(|block| match block {
                Block::Heading { text, .. } => Some(inline::plain(text)),
                _ => None,
            })
        })
        .unwrap_or_else(|| "This note".to_string())
}

/// The end of a paragraph, used as the context line in the footnote picker.
fn snippet(text: &str) -> String {
    let text = text.replace('\n', " ");
    let text = text.trim();
    let count = text.chars().count();
    if count <= 120 {
        text.to_string()
    } else {
        format!(
            "\u{2026}{}",
            text.chars().skip(count - 120).collect::<String>()
        )
    }
}

/// Parsed reading blocks plus the scroll state for a virtualized block list.
///
/// Blocks have different heights, so this uses GPUI's variable-height `List`: only the
/// blocks around the viewport are turned into elements. The parse result is cached per
/// document version, so scrolling never re-parses the note.
pub struct ReadingCache {
    version: (u64, u64),
    blocks: Rc<Vec<Block>>,
    list_state: ListState,
    /// File index for the current vault, built on first use and reused afterwards.
    attachments: Option<Rc<Attachments>>,
    attachments_root: Option<PathBuf>,
    /// Character-level selection, shared by every rendered text piece.
    selection: Rc<RefCell<ReadSelection>>,
    /// Frontmatter of the current document, shown above the reading view.
    properties: Vec<Property>,
    /// Display formulas, typeset when the document changes. Keyed by source and style: layout
    /// is in em, so one scene serves every font size and zoom level. `Rc` so the virtualized
    /// list closure can share it without cloning the map each frame.
    math: Rc<HashMap<(String, MathStyle), Rc<MathScene>>>,
    /// Collapsible metadata header; created on first render when an `App` is available.
    metadata: Option<Entity<crate::editor::MetadataView>>,
    /// Footnote references by id, for the marker picker.
    footnote_refs: Rc<RefCell<HashMap<String, Vec<FootnoteRef>>>>,
    /// Footnote definition blocks by id.
    footnote_defs: Rc<RefCell<HashMap<String, usize>>>,
    /// The picker overlay, created on first render.
    footnote_popup: Option<Entity<FootnotePopup>>,
    /// Heading blocks whose sections are folded.
    folded: HashSet<usize>,
    /// Source lines for each block, so the editor can map caret ↔ reader.
    spans: Rc<Vec<markdown::BlockSpan>>,
    /// False when shared-block expansion rewrote the text, which moves the spans away
    /// from the buffer's own line numbers.
    spans_valid: bool,
    /// In-page find over block display text. `Rc` so the virtualized list can borrow it
    /// without copying the match list every frame.
    find: Rc<ReadingFind>,
    /// Image shown in the lightbox overlay, if any.
    lightbox: Rc<RefCell<Option<PathBuf>>>,
    /// A block a popup asked the reader to reveal on the next render.
    reveal_block: Rc<RefCell<Option<usize>>>,
    /// Parsed transcluded notes, revalidated by modification time.
    embeds: EmbedCache,
    /// Formulas typeset for transcluded notes.
    embed_math: EmbedMath,
}

impl ReadingCache {
    pub fn new() -> Self {
        Self {
            version: (u64::MAX, u64::MAX),
            blocks: Rc::new(Vec::new()),
            list_state: ListState::new(0, ListAlignment::Top, px(600.0)),
            attachments: None,
            attachments_root: None,
            selection: Rc::new(RefCell::new(ReadSelection::default())),
            properties: Vec::new(),
            math: Rc::new(HashMap::new()),
            metadata: None,
            footnote_refs: Rc::new(RefCell::new(HashMap::new())),
            footnote_defs: Rc::new(RefCell::new(HashMap::new())),
            spans: Rc::new(Vec::new()),
            spans_valid: false,
            find: Rc::new(ReadingFind::default()),
            lightbox: Rc::new(RefCell::new(None)),
            reveal_block: Rc::new(RefCell::new(None)),
            embeds: Rc::new(RefCell::new(HashMap::new())),
            embed_math: Rc::new(RefCell::new(HashMap::new())),
            footnote_popup: None,
            folded: HashSet::new(),
        }
    }

    /// Fold or unfold the section under a heading block.
    pub fn toggle_fold(&mut self, index: usize) {
        if !self.folded.remove(&index) {
            self.folded.insert(index);
        }
        // Folded blocks collapse to zero height, so the list's cached item sizes are stale.
        let scroll = self.list_state.logical_scroll_top();
        let count = self.blocks.len();
        self.list_state.splice(0..count, count);
        self.list_state.scroll_to(scroll);
    }

    pub fn needs_update(&self, version: (u64, u64)) -> bool {
        self.version != version
    }

    /// Drop the attachment index, so the next render re-walks the vault. Called after a
    /// paste or drop files something new.
    pub fn invalidate_attachments(&mut self) {
        self.attachments = None;
        self.attachments_root = None;
    }

    /// Re-parse only when the document changed. A new document resets the scroll
    /// position; edits to the same document preserve it.
    pub fn update(&mut self, version: (u64, u64), source: &str, root: Option<&Path>) {
        if self.version == version {
            return;
        }

        // Folds are per document; an edit keeps them, a different note does not.
        if self.version.0 != version.0 {
            self.folded.clear();
        }

        let original = source.to_string();
        let source = match root {
            Some(root) => shared::expand(source, root),
            None => source.to_string(),
        };
        let spans_valid = source == original;
        let (blocks, spans) = markdown::parse_blocks_spanned(&source);
        self.properties = frontmatter::display_properties(&source);

        // Typeset every formula once per document version. Scenes are in em, so the painter
        // can scale them to any font size without laying out again.
        let mut math = HashMap::new();
        for block in &blocks {
            if let Block::Math { tex } = block {
                typeset_into(&mut math, tex, MathStyle::Display);
            }
            for text in block_texts(block) {
                for span in inline::parse(text) {
                    if let Some(tex) = span.math {
                        typeset_into(&mut math, &tex, MathStyle::Inline);
                    }
                }
            }
        }
        self.math = Rc::new(math);

        // Footnote occurrences for the marker picker: id → paragraphs that cite it, and
        // id → definition block.
        let mut footnote_refs: HashMap<String, Vec<FootnoteRef>> = HashMap::new();
        let mut footnote_defs: HashMap<String, usize> = HashMap::new();
        for (index, block) in blocks.iter().enumerate() {
            if let Block::Footnote { id, .. } = block {
                footnote_defs.insert(id.clone(), index);
            }
            let context = snippet(&block_display(block));
            for text in block_texts(block) {
                for span in inline::parse(text) {
                    if !span.style.footnote {
                        continue;
                    }
                    let id = span.text.trim().to_string();
                    let entry = footnote_refs.entry(id).or_default();
                    if !entry.iter().any(|reference| reference.block == index) {
                        entry.push(FootnoteRef {
                            block: index,
                            snippet: context.clone(),
                        });
                    }
                }
            }
        }
        *self.footnote_refs.borrow_mut() = footnote_refs;
        *self.footnote_defs.borrow_mut() = footnote_defs;

        if self.version.0 != version.0 {
            self.list_state.reset(blocks.len());
        } else {
            let old_count = self.blocks.len();
            self.list_state.splice(0..old_count, blocks.len());
        }

        self.selection.borrow_mut().clear();
        self.blocks = Rc::new(blocks);
        self.spans = Rc::new(spans);
        self.spans_valid = spans_valid;
        self.version = version;
    }

    /// The block that owns `line` (0-based), or the nearest block above it.
    pub fn block_for_line(&self, line: usize) -> Option<usize> {
        if !self.spans_valid {
            return None;
        }
        let mut fallback = None;
        for (index, span) in self.spans.iter().enumerate() {
            if span.first_line <= line && line <= span.last_line {
                return Some(index);
            }
            if span.first_line <= line {
                fallback = Some(index);
            }
        }
        fallback.or_else(|| (!self.blocks.is_empty()).then_some(0))
    }

    /// The first source line of a block, 0-based.
    pub fn block_start_line(&self, index: usize) -> Option<usize> {
        if !self.spans_valid {
            return None;
        }
        self.spans.get(index).map(|span| span.first_line)
    }

    /// The first source line of the top visible block.
    pub fn top_block_line(&self) -> Option<usize> {
        self.block_start_line(self.top_block())
    }

    /// Close the image lightbox. Returns whether one was open.
    pub fn close_lightbox(&mut self) -> bool {
        self.lightbox.borrow_mut().take().is_some()
    }

    pub fn find_open(&self) -> bool {
        self.find.open
    }

    pub fn find_pattern(&self) -> &str {
        &self.find.pattern
    }

    pub fn find_error(&self) -> Option<&str> {
        self.find.error.as_deref()
    }

    /// `(current, total)`, both 0 when there are no matches.
    pub fn find_position(&self) -> (usize, usize) {
        if self.find.matches.is_empty() {
            (0, 0)
        } else {
            (self.find.current + 1, self.find.matches.len())
        }
    }

    /// Start (or focus) in-page find, remembering where to return on Escape.
    pub fn open_find(&mut self) {
        let return_block = self.top_block();
        let find = Rc::make_mut(&mut self.find);
        find.return_block = return_block;
        find.open = true;
        self.recompute_find();
    }

    /// Close the bar and put the reader back where it was.
    pub fn close_find(&mut self) {
        let return_block = Rc::make_mut(&mut self.find).return_block;
        Rc::make_mut(&mut self.find).open = false;
        if !self.blocks.is_empty() {
            let target = return_block.min(self.blocks.len().saturating_sub(1));
            self.list_state.scroll_to_reveal_item(target);
        }
    }

    pub fn find_append(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        Rc::make_mut(&mut self.find).pattern.push_str(text);
        self.recompute_find();
    }

    pub fn find_backspace(&mut self) {
        Rc::make_mut(&mut self.find).pattern.pop();
        self.recompute_find();
    }

    pub fn find_toggle_regex(&mut self) {
        let find = Rc::make_mut(&mut self.find);
        find.options.regex = !find.options.regex;
        self.recompute_find();
    }

    pub fn find_toggle_case(&mut self) {
        let find = Rc::make_mut(&mut self.find);
        find.options.case_sensitive = !find.options.case_sensitive;
        self.recompute_find();
    }

    pub fn find_toggle_word(&mut self) {
        let find = Rc::make_mut(&mut self.find);
        find.options.whole_word = !find.options.whole_word;
        self.recompute_find();
    }

    pub fn find_options(&self) -> (bool, bool, bool) {
        (
            self.find.options.regex,
            self.find.options.case_sensitive,
            self.find.options.whole_word,
        )
    }

    pub fn find_next(&mut self, backwards: bool) {
        let block = {
            let find = Rc::make_mut(&mut self.find);
            if find.matches.is_empty() {
                return;
            }
            let len = find.matches.len();
            find.current = if backwards {
                (find.current + len - 1) % len
            } else {
                (find.current + 1) % len
            };
            find.matches[find.current].block
        };
        self.list_state.scroll_to_reveal_item(block);
    }

    fn recompute_find(&mut self) {
        let top = self.top_block();
        let find = Rc::make_mut(&mut self.find);
        find.matches.clear();
        find.current = 0;
        find.error = None;
        if find.pattern.is_empty() {
            return;
        }

        let options = find.options;
        let pattern = find.pattern.clone();
        'blocks: for (index, block) in self.blocks.iter().enumerate() {
            let text = block_display(block);
            match find_engine::find_all(&text, &pattern, options) {
                Ok(found) => {
                    for found in found {
                        find.matches.push(ReadingMatch {
                            block: index,
                            range: found.range,
                        });
                        if find.matches.len() >= 1000 {
                            break 'blocks;
                        }
                    }
                }
                Err(error) => {
                    find.error = Some(error);
                    find.matches.clear();
                    return;
                }
            }
        }

        if let Some(first) = find.matches.iter().position(|found| found.block >= top) {
            find.current = first;
        }
        if let Some(found) = find.matches.get(find.current) {
            let block = found.block;
            self.list_state.scroll_to_reveal_item(block);
        }
    }

    /// The selected reading text, with blocks joined by newlines.
    pub fn selected_text(&self) -> Option<String> {
        let (start, end) = self.selection.borrow().ordered()?;
        let mut out = String::new();

        for (index, block) in self.blocks.iter().enumerate() {
            if index < start.block || index > end.block {
                continue;
            }
            if let Block::Embed {
                target, fragment, ..
            } = block
            {
                // A transcluded note contributes its own display text to a copy.
                if let Some(embedded) = self.embedded_display_text(target, fragment.as_deref()) {
                    out.push_str(&embedded);
                }
            } else {
                let text = block_display(block);
                let from = if index == start.block {
                    start.offset.min(text.len())
                } else {
                    0
                };
                let to = if index == end.block {
                    end.offset.min(text.len())
                } else {
                    text.len()
                };
                if from < to {
                    out.push_str(&text[from..to]);
                }
            }
            if index < end.block {
                out.push('\n');
            }
        }

        (!out.is_empty()).then_some(out)
    }

    /// The display text of a transcluded note or section, for copying a selection that
    /// spans an embed.
    fn embedded_display_text(&self, target: &str, fragment: Option<&str>) -> Option<String> {
        let root = self.attachments_root.as_deref()?;
        let path = resolve_note(root, self.attachments.as_deref(), target);
        let key = (path.clone(), fragment.map(str::to_string));
        let cached = self
            .embeds
            .borrow()
            .get(&key)
            .map(|entry| entry.blocks.clone());
        let blocks = match cached {
            Some(blocks) => blocks,
            None => {
                let text = std::fs::read_to_string(&path).ok()?;
                let text = shared::expand(&text, root);
                let (mut blocks, spans) = markdown::parse_blocks_spanned(&text);
                blocks = match fragment {
                    Some(fragment) if fragment.starts_with('^') => {
                        vec![block_by_id(&blocks, &spans, &text, fragment)?]
                    }
                    Some(fragment) => section_blocks(blocks, fragment)?,
                    None => blocks,
                };
                Rc::new(blocks)
            }
        };
        let text = blocks
            .iter()
            .map(block_display)
            .collect::<Vec<_>>()
            .join("\n");
        Some(text)
    }

    /// Select the whole document, for Ctrl+A in reading mode.
    pub fn select_all(&mut self) {
        let Some(last) = self.blocks.len().checked_sub(1) else {
            return;
        };
        let end = block_display(&self.blocks[last]).len();
        let mut selection = self.selection.borrow_mut();
        selection.anchor = Some(ReadPoint {
            block: 0,
            offset: 0,
        });
        selection.head = Some(ReadPoint {
            block: last,
            offset: end,
        });
        selection.dragging = false;
        selection.moved = false;
    }

    pub fn selection_stats(&self) -> Option<SelectionStats> {
        let text = self.selected_text()?;
        Some(SelectionStats {
            words: text.split_whitespace().count(),
            characters: text.chars().count(),
            lines: text.lines().count().max(1),
        })
    }

    /// Test-only: install a selection from display-text offsets.
    #[cfg(test)]
    pub(crate) fn select_range(&mut self, anchor: ReadPoint, head: ReadPoint) {
        let mut selection = self.selection.borrow_mut();
        selection.anchor = Some(anchor);
        selection.head = Some(head);
        selection.dragging = false;
        selection.moved = true;
    }

    /// Drop the reading selection. Returns whether there was one to drop.
    pub fn clear_selection(&mut self) -> bool {
        let mut selection = self.selection.borrow_mut();
        let had = selection.ordered().is_some();
        if had {
            selection.clear();
        }
        had
    }

    /// Extend (or start) the reading selection with a keyboard motion.
    pub fn extend_selection(&mut self, motion: SelectionMotion) {
        let start = {
            let selection = self.selection.borrow();
            selection.head.or(selection.anchor)
        };
        let Some(start) = start else {
            return;
        };
        let target = self.move_point(start, motion);
        let mut selection = self.selection.borrow_mut();
        if selection.anchor.is_none() {
            selection.anchor = Some(start);
        }
        selection.head = Some(target);
        selection.dragging = false;
        selection.moved = true;
    }

    /// The next point in the requested direction, walking across block boundaries.
    fn move_point(&self, point: ReadPoint, motion: SelectionMotion) -> ReadPoint {
        let len = self.block_len(point.block);
        match motion {
            SelectionMotion::Left => {
                let text = self.block_text(point.block);
                if point.offset > 0 {
                    let offset = text.as_str()[..point.offset.min(text.len())]
                        .char_indices()
                        .next_back()
                        .map(|(index, _)| index)
                        .unwrap_or(0);
                    ReadPoint {
                        block: point.block,
                        offset,
                    }
                } else if let Some(block) = self.previous_block(point.block) {
                    ReadPoint {
                        block,
                        offset: self.block_len(block),
                    }
                } else {
                    point
                }
            }
            SelectionMotion::Right => {
                let text = self.block_text(point.block);
                if point.offset < len {
                    let offset = text.as_str()[point.offset.min(text.len())..]
                        .chars()
                        .next()
                        .map(|ch| point.offset + ch.len_utf8())
                        .unwrap_or(len);
                    ReadPoint {
                        block: point.block,
                        offset,
                    }
                } else if let Some(block) = self.next_block(point.block) {
                    ReadPoint { block, offset: 0 }
                } else {
                    point
                }
            }
            SelectionMotion::Up => {
                let block = self.previous_block(point.block).unwrap_or(point.block);
                ReadPoint {
                    block,
                    offset: point.offset.min(self.block_len(block)),
                }
            }
            SelectionMotion::Down => {
                let block = self.next_block(point.block).unwrap_or(point.block);
                ReadPoint {
                    block,
                    offset: point.offset.min(self.block_len(block)),
                }
            }
            SelectionMotion::LineStart => ReadPoint {
                block: point.block,
                offset: 0,
            },
            SelectionMotion::LineEnd => ReadPoint {
                block: point.block,
                offset: len,
            },
            SelectionMotion::DocumentStart => ReadPoint {
                block: 0,
                offset: 0,
            },
            SelectionMotion::DocumentEnd => {
                let block = self.blocks.len().saturating_sub(1);
                ReadPoint {
                    block,
                    offset: self.block_len(block),
                }
            }
        }
    }

    fn block_text(&self, block: usize) -> String {
        self.blocks
            .get(block)
            .map(block_display)
            .unwrap_or_default()
    }

    fn block_len(&self, block: usize) -> usize {
        self.blocks
            .get(block)
            .map(|block| block_display(block).len())
            .unwrap_or(0)
    }

    fn previous_block(&self, block: usize) -> Option<usize> {
        (0..block).rev().find(|index| self.block_len(*index) > 0)
    }

    fn next_block(&self, block: usize) -> Option<usize> {
        (block + 1..self.blocks.len()).find(|index| self.block_len(*index) > 0)
    }

    /// The block currently at the top of the reading view.
    pub fn top_block(&self) -> usize {
        if self.blocks.is_empty() {
            0
        } else {
            self.list_state.logical_scroll_top().item_ix
        }
    }

    /// Scroll the reading view by whole blocks, clamped to the document.
    pub fn scroll_blocks(&mut self, delta: isize) {
        if self.blocks.is_empty() {
            return;
        }
        let top = self.top_block() as isize;
        let last = self.blocks.len().saturating_sub(1) as isize;
        let target = (top + delta).clamp(0, last) as usize;
        self.list_state.scroll_to_reveal_item(target);
    }

    /// The nearest heading at or above the top visible block.
    pub fn current_heading(&self) -> Option<String> {
        if self.blocks.is_empty() {
            return None;
        }
        let top = self.top_block().min(self.blocks.len() - 1);
        self.blocks[..=top]
            .iter()
            .rev()
            .find_map(|block| match block {
                Block::Heading { text, .. } => Some(text.clone()),
                _ => None,
            })
    }

    /// How far down the document the viewport sits, from 0.0 to 1.0.
    pub fn progress(&self) -> f32 {
        if self.blocks.len() <= 1 {
            return 0.0;
        }
        (self.top_block() as f32 / (self.blocks.len() - 1) as f32).clamp(0.0, 1.0)
    }

    /// Scroll the reading view so `index` is visible. Restoring a session uses this.
    pub fn scroll_to_block(&mut self, index: usize) {
        if self.blocks.is_empty() {
            return;
        }
        self.list_state
            .scroll_to_reveal_item(index.min(self.blocks.len() - 1));
    }

    /// Close the footnote popup if it is open. Returns whether it was open, so Escape can
    /// consume the key before falling through to the panels.
    pub fn dismiss_popup(&mut self, cx: &mut App) -> bool {
        let Some(popup) = &self.footnote_popup else {
            return false;
        };
        if !popup.read(cx).is_open() {
            return false;
        }
        popup.update(cx, |popup, cx| {
            popup.close();
            cx.notify();
        });
        true
    }

    /// Scroll the reading view so the heading with this text is visible.
    pub fn scroll_to_heading(&self, text: &str) {
        if let Some(index) = self.blocks.iter().position(
            |block| matches!(block, Block::Heading { text: heading, .. } if heading == text),
        ) {
            self.list_state.scroll_to_reveal_item(index);
        }
    }

    /// Scroll to the nth heading in the document. Heading order matches the table of
    /// contents, so this works even when the heading text contains math or markup.
    pub fn scroll_to_heading_index(&self, heading_index: usize) -> bool {
        let Some((index, _)) = self
            .blocks
            .iter()
            .enumerate()
            .filter(|(_, block)| matches!(block, Block::Heading { .. }))
            .nth(heading_index)
        else {
            return false;
        };
        self.list_state.scroll_to_reveal_item(index);
        true
    }

    pub fn element(
        &mut self,
        vault_root: Option<PathBuf>,
        theme: Theme,
        template: TextRun,
        opener: Option<LinkOpener>,
        on_fold: Option<FoldToggle>,
        cx: &mut App,
    ) -> impl IntoElement {
        // A popup can ask for a block hidden by a fold: unfold, reflow, then scroll to it.
        if let Some(block) = self.reveal_block.borrow_mut().take() {
            let hidden = hidden_blocks(&self.blocks, &self.folded);
            if hidden.get(block).copied().unwrap_or(false) {
                self.folded.clear();
                let scroll = self.list_state.logical_scroll_top();
                let count = self.blocks.len();
                self.list_state.splice(0..count, count);
                self.list_state.scroll_to(scroll);
            }
            self.list_state.scroll_to_reveal_item(block);
        }

        if vault_root.as_deref() != self.attachments_root.as_deref() {
            self.attachments = vault_root
                .as_deref()
                .map(|root| Rc::new(Attachments::index(root)));
            self.attachments_root = vault_root.clone();
        }

        if self.footnote_popup.is_none() {
            let refs = self.footnote_refs.clone();
            let defs = self.footnote_defs.clone();
            let list_state = self.list_state.clone();
            let reveal = self.reveal_block.clone();
            self.footnote_popup =
                Some(cx.new(move |_| FootnotePopup::new(refs, defs, list_state, reveal)));
        }
        let title = note_title(&self.properties, self.blocks.as_slice());
        if let Some(popup) = &self.footnote_popup {
            popup.update(cx, |popup, cx| popup.set_title(title, cx));
        }

        // Footnote markers share the link path: `#fn:<id>` opens the picker, everything else
        // goes to the app's normal note/URL opener.
        let popup_for_links = self.footnote_popup.clone();
        let app_opener = opener.clone();
        let opener: Option<LinkOpener> = Some(Rc::new(move |target, window, cx| {
            if let Some(id) = target.strip_prefix("#fn:") {
                let anchor = window.mouse_position();
                if let Some(popup) = &popup_for_links {
                    let id = id.to_string();
                    popup.update(cx, |popup, cx| {
                        popup.show_at(&id, Some(anchor));
                        cx.notify();
                    });
                }
                window.refresh();
                return;
            }
            if let Some(opener) = &app_opener {
                opener(target, window, cx);
            }
        }));

        if self.metadata.is_none() {
            self.metadata = Some(cx.new(crate::editor::MetadataView::new));
        }
        let metadata = self.metadata.clone();
        if let Some(view) = &metadata {
            view.update(cx, |view, cx| {
                view.set(self.properties.clone(), theme, opener.clone(), cx);
            });
        }

        let reader = cx
            .try_global::<ReaderSettings>()
            .copied()
            .unwrap_or_default();
        let footnote_popup = self.footnote_popup.clone();
        let blocks = self.blocks.clone();
        let attachments = self.attachments.clone();
        let selection = self.selection.clone();
        let math = self.math.clone();
        let copy: CopyHandler = Rc::new(|text, window, cx| {
            cx.write_to_clipboard(ClipboardItem::new_string(text.to_string()));
            window.refresh();
        });
        let find = self.find.clone();
        let lightbox = self.lightbox.clone();
        let embeds = self.embeds.clone();
        let embed_math = self.embed_math.clone();
        let next_block = Cell::new(self.blocks.len());
        let on_image: ImageOpener = {
            let lightbox = lightbox.clone();
            Rc::new(move |path, window, _cx| {
                *lightbox.borrow_mut() = Some(PathBuf::from(path));
                window.refresh();
            })
        };
        let open_file: FileOpener = Rc::new(|path, window, cx| {
            cx.open_url(&format!("file://{path}"));
            window.refresh();
        });
        let hidden = Rc::new(hidden_blocks(self.blocks.as_slice(), &self.folded));
        let folded = self.folded.clone();
        let list_state = self.list_state.clone();
        div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .px_8()
            .py_6()
            .text_color(theme.text)
            .children(metadata)
            .child(
                div().flex_1().min_h(px(0.0)).child(
                    list(list_state.clone(), move |index, _window, _cx| {
                        let Some(block) = blocks.get(index).cloned() else {
                            return div().into_any_element();
                        };
                        if hidden.get(index).copied().unwrap_or(false) {
                            return div().w_full().h(px(0.0)).into_any_element();
                        }

                        let ctx = RenderCtx {
                            vault_root: vault_root.as_deref(),
                            attachments: attachments.as_deref(),
                            selection: &selection,
                            opener: opener.as_ref(),
                            math: &math,
                            folded: &folded,
                            on_fold: on_fold.as_ref(),
                            copy: Some(&copy),
                            find: Some(&find),
                            on_image: Some(&on_image),
                            open_file: Some(&open_file),
                            embeds: Some(&embeds),
                            embed_math: Some(&embed_math),
                            next_block: Some(&next_block),
                            list_state: Some(&list_state),
                            block_count: blocks.len(),
                            blocks: blocks.as_slice(),
                            depth: 0,
                            theme,
                            template: &template,
                            body_size: reader.body_size,
                            measure: reader.measure,
                        };

                        let mut item = div()
                            .w_full()
                            .max_w(px(reader.measure))
                            .mx_auto()
                            .px_2()
                            .pb_3();
                        // Display math is centred by the item itself: the list gives each
                        // item the full column width, but a shrink-to-fit child would
                        // otherwise sit at the left edge.
                        if matches!(block, Block::Math { .. }) {
                            item = item.flex().justify_center().py_3();
                        }
                        item.child(render_block(block, Some(index), &ctx))
                            .into_any_element()
                    })
                    .size_full(),
                ),
            )
            .children(
                footnote_popup
                    .clone()
                    .filter(|popup| popup.read(cx).is_open())
                    .map(|popup| {
                        let close = popup.clone();
                        div()
                            .id("footnote-backdrop")
                            .absolute()
                            .top_0()
                            .left_0()
                            .right_0()
                            .bottom_0()
                            .on_mouse_down(MouseButton::Left, move |_event, window, cx| {
                                // A click outside the popup dismisses it.
                                close.update(cx, |popup, cx| {
                                    popup.close();
                                    cx.notify();
                                });
                                window.refresh();
                            })
                            .into_any_element()
                    }),
            )
            .children(footnote_popup.map(|popup| {
                let anchor = popup
                    .read(cx)
                    .anchor()
                    .unwrap_or_else(|| point(px(24.0), px(96.0)));
                anchored()
                    .position(anchor)
                    .anchor(Corner::TopLeft)
                    .offset(point(px(8.0), px(12.0)))
                    .snap_to_window_with_margin(px(8.0))
                    .child(popup)
                    .into_any_element()
            }))
            .children(self.lightbox.borrow().as_ref().map(|path| {
                let close = lightbox.clone();
                div()
                    .id("lightbox")
                    .absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .bottom_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(gpui::hsla(0.0, 0.0, 0.0, 0.82))
                    .on_mouse_down(MouseButton::Left, move |_event, window, _cx| {
                        // Clicking the backdrop (not the image) closes the viewer.
                        *close.borrow_mut() = None;
                        window.refresh();
                    })
                    .child(
                        div()
                            .p_4()
                            .on_mouse_down(MouseButton::Left, |_event, _window, cx| {
                                cx.stop_propagation();
                            })
                            .child(img(path.clone()).max_w(px(1200.0)).max_h(px(900.0))),
                    )
                    .into_any_element()
            }))
    }
}

impl Default for ReadingCache {
    fn default() -> Self {
        Self::new()
    }
}

/// Mark the blocks hidden by folded headings.
///
/// A folded heading hides every following block up to the next heading of the same or a
/// higher level. Hidden blocks are still list items, but they render at zero height, so
/// list indices stay stable while a section opens and closes.
fn hidden_blocks(blocks: &[Block], folded: &HashSet<usize>) -> Vec<bool> {
    let mut hidden = vec![false; blocks.len()];
    let mut hide_until_level: Option<u8> = None;

    for (index, block) in blocks.iter().enumerate() {
        let heading_level = match block {
            Block::Heading { level, .. } => Some(*level),
            _ => None,
        };

        if let (Some(level), Some(limit)) = (heading_level, hide_until_level) {
            if level <= limit {
                hide_until_level = None;
            }
        }
        if hide_until_level.is_some() {
            hidden[index] = true;
            continue;
        }

        if let Some(level) = heading_level {
            if folded.contains(&index) {
                hide_until_level = Some(level);
            }
        }
    }

    hidden
}

/// A non-virtualized column, used for nested `![[note]]` embeds where a second scroll
/// container would be wrong.
/// Render a list of parsed blocks, giving each a synthetic id so links and selection work.
fn render_blocks(blocks: &Rc<Vec<Block>>, ctx: &RenderCtx) -> AnyElement {
    let template = ctx.template.clone();
    let nested = RenderCtx {
        vault_root: ctx.vault_root,
        on_fold: None,
        blocks: &[],
        depth: ctx.depth + 1,
        template: &template,
        ..*ctx
    };

    div()
        .flex()
        .flex_col()
        .gap_3()
        .children(blocks.iter().cloned().map(|block| {
            let id = ctx.next_block.map(|next| {
                let value = next.get();
                next.set(value + 1);
                value
            });
            render_block(block, id, &nested)
        }))
        .into_any_element()
}

/// The block whose last line carries `^id`, for `![[Note#^id]]`.
fn block_by_id(
    blocks: &[Block],
    spans: &[markdown::BlockSpan],
    source: &str,
    fragment: &str,
) -> Option<Block> {
    let id = fragment.trim().trim_start_matches('^').trim();
    if id.is_empty() {
        return None;
    }
    let marker = format!("^{id}");
    let line = source
        .lines()
        .position(|line| line.trim_end().ends_with(&marker))?;
    let index = spans
        .iter()
        .position(|span| span.first_line <= line && line <= span.last_line)?;
    blocks.get(index).cloned()
}

/// Only the blocks of one `#Heading` section, for `![[Note#Section]]`.
fn section_blocks(blocks: Vec<Block>, fragment: &str) -> Option<Vec<Block>> {
    let wanted = fragment
        .trim()
        .trim_start_matches('#')
        .trim()
        .to_lowercase();
    if wanted.is_empty() {
        return None;
    }
    let start = blocks.iter().position(|block| match block {
        Block::Heading { text, .. } => text.trim().to_lowercase() == wanted,
        _ => false,
    })?;
    let level = match &blocks[start] {
        Block::Heading { level, .. } => *level,
        _ => return None,
    };
    let mut end = blocks.len();
    for (index, block) in blocks.iter().enumerate().skip(start + 1) {
        if let Block::Heading { level: next, .. } = block {
            if *next <= level {
                end = index;
                break;
            }
        }
    }
    Some(blocks[start..end].to_vec())
}

fn render_block(block: Block, block_index: Option<usize>, ctx: &RenderCtx) -> AnyElement {
    let theme = ctx.theme;
    let template = ctx.template;
    match block {
        Block::Heading { level, text } => {
            let size = match level {
                1 => 30.0,
                2 => 24.0,
                3 => 20.0,
                _ => 17.0,
            };
            let foldable = block_index.is_some_and(|index| {
                matches!(
                    ctx.blocks.get(index + 1),
                    Some(Block::Heading { level: next, .. }) if *next > level
                )
            });
            let collapsed = block_index.is_some_and(|index| ctx.folded.contains(&index));

            let mut heading = div().flex().flex_row().items_center().gap_2().pb_1();
            if foldable {
                let mut chevron = div()
                    .w(px(14.0))
                    .flex_shrink_0()
                    .cursor_pointer()
                    .text_color(theme.muted)
                    .child(if collapsed { "\u{25B8}" } else { "\u{25BE}" });
                if let (Some(on_fold), Some(index)) = (ctx.on_fold, block_index) {
                    let on_fold = on_fold.clone();
                    chevron = chevron
                        .hover(move |style| style.text_color(theme.text))
                        .on_mouse_down(MouseButton::Left, move |_ev, window, cx| {
                            cx.stop_propagation();
                            on_fold(index, window, cx);
                        });
                }
                heading = heading.child(chevron);
            } else {
                heading = heading.child(div().w(px(14.0)).flex_shrink_0());
            }

            heading
                .child(div().flex_1().min_w(px(0.0)).child(rich_text_styled(
                    &text,
                    size,
                    block_index,
                    0,
                    ctx,
                    gpui::FontWeight::BOLD,
                    1.25,
                )))
                .into_any_element()
        }
        Block::Paragraph(text) => rich_text(&text, ctx.body_size, block_index, 0, ctx),
        Block::ListItem {
            depth,
            marker,
            checked,
            text,
            ..
        } => {
            let (marker, marker_color) = match checked {
                Some(true) => ("\u{2611}".to_string(), theme.code_text),
                Some(false) => ("\u{2610}".to_string(), theme.muted),
                None => (marker, theme.muted),
            };
            div()
                .flex()
                .flex_row()
                .items_start()
                // An explicit spacer instead of padding: padding on a flex row is added
                // outside the column width here, which pushed deep items past the wrap edge.
                .child(
                    div()
                        .w(px(8.0 + depth.min(8) as f32 * 16.0))
                        .flex_shrink_0(),
                )
                .child(
                    div()
                        .w(px(24.0))
                        .flex_shrink_0()
                        .text_color(marker_color)
                        .child(marker),
                )
                // The text must be the one flexible child: without `flex_1` its
                // max-content width becomes the row's width, the row outgrows the
                // reading column, and every later block loses the column's alignment.
                .child(div().flex_1().min_w(px(0.0)).child(rich_text(
                    &text,
                    ctx.body_size,
                    block_index,
                    0,
                    ctx,
                )))
                .into_any_element()
        }
        Block::Code { text, language } => code_block(&text, &language, block_index, ctx),
        Block::Quote(text) => {
            let mut muted = template.clone();
            muted.color = theme.muted;
            let muted_ctx = ctx.with_template(&muted);
            div()
                .border_l_1()
                .border_color(theme.border)
                .pl_3()
                .child(rich_text(&text, ctx.body_size, block_index, 0, &muted_ctx))
                .into_any_element()
        }
        Block::Rule => div()
            .h(px(1.0))
            .w_full()
            .bg(theme.border)
            .into_any_element(),
        Block::Math { tex } => render_math(&tex, ctx),
        Block::Footnote { id, text } => {
            // `[id] text` is the display form; selection offsets start after the marker.
            let base = id.len() + 3;
            let target = format!("#fn:{id}");
            let opener = ctx.opener.cloned();
            div()
                .flex()
                .flex_row()
                .items_start()
                .gap_2()
                .pt_1()
                .text_sm()
                .text_color(theme.muted)
                .child(
                    div()
                        .flex_shrink_0()
                        .font_weight(gpui::FontWeight::BOLD)
                        .text_color(theme.accent_text)
                        .cursor_pointer()
                        .child(format!("[{id}]"))
                        .on_mouse_down(MouseButton::Left, move |_ev, window, cx| {
                            if let Some(opener) = &opener {
                                opener(&target, window, cx);
                            }
                        }),
                )
                .child(div().flex_1().min_w(px(0.0)).child(rich_text(
                    &text,
                    13.0,
                    block_index,
                    base,
                    ctx,
                )))
                .into_any_element()
        }
        Block::Callout { kind, title, text } => {
            let accent = match kind.as_str() {
                "warning" | "caution" | "danger" | "error" => theme.warn,
                "tip" | "success" => theme.code_text,
                _ => theme.accent_text,
            };

            let mut title_template = template.clone();
            title_template.color = accent;
            let title_ctx = ctx.with_template(&title_template);
            let body_base = inline_display(&title).len() + 1;

            div()
                .border_l_1()
                .border_color(accent)
                .bg(theme.chip)
                .rounded_md()
                .p_3()
                .flex()
                .flex_col()
                .gap_1()
                .child(rich_text_styled(
                    &title,
                    ctx.body_size,
                    block_index,
                    0,
                    &title_ctx,
                    gpui::FontWeight::BOLD,
                    1.3,
                ))
                .when(!text.trim().is_empty(), |d| {
                    d.child(rich_text(&text, ctx.body_size, block_index, body_base, ctx))
                })
                .into_any_element()
        }
        Block::Table {
            align,
            headers,
            rows,
        } => render_table(&align, &headers, &rows, block_index, ctx),
        Block::Embed {
            target,
            width,
            alt,
            fragment,
        } => render_embed(&target, width, alt.as_deref(), fragment.as_deref(), ctx),
    }
}

impl<'a> RenderCtx<'a> {
    /// In-page find matches in one block, as `(range, is_current)`.
    fn find_matches(&self, block: Option<usize>) -> Vec<(Range<usize>, bool)> {
        let (Some(block), Some(find)) = (block, self.find) else {
            return Vec::new();
        };
        if !find.open || find.pattern.is_empty() {
            return Vec::new();
        }
        find.matches
            .iter()
            .enumerate()
            .filter(|(_, found)| found.block == block)
            .map(|(index, found)| (found.range.clone(), index == find.current))
            .collect()
    }

    /// A typeset scene: host formulas first, then lazily typeset embed formulas.
    fn math_scene(&self, tex: &str, style: MathStyle) -> Option<Rc<MathScene>> {
        let key = (tex.to_string(), style);
        if let Some(scene) = self.math.get(&key) {
            return Some(scene.clone());
        }
        let cache = self.embed_math?;
        if let Some(scene) = cache.borrow().get(&key) {
            return Some(scene.clone());
        }
        let scene = math::typeset(
            tex,
            &MathOptions {
                style,
                color: [0.0, 0.0, 0.0, 1.0],
            },
        )
        .ok()
        .map(Rc::new)?;
        cache.borrow_mut().insert(key, scene.clone());
        Some(scene)
    }

    fn with_template<'b>(&'b self, template: &'b TextRun) -> RenderCtx<'b> {
        RenderCtx {
            vault_root: self.vault_root,
            attachments: self.attachments,
            selection: self.selection,
            opener: self.opener,
            math: self.math,
            folded: self.folded,
            on_fold: self.on_fold,
            copy: self.copy,
            find: self.find,
            on_image: self.on_image,
            open_file: self.open_file,
            embeds: self.embeds,
            embed_math: self.embed_math,
            next_block: self.next_block,
            list_state: self.list_state,
            block_count: self.block_count,
            blocks: self.blocks,
            depth: self.depth,
            theme: self.theme,
            template,
            body_size: self.body_size,
            measure: self.measure,
        }
    }
}

/// Typeset one formula into the cache, unless it is already there.
fn typeset_into(
    cache: &mut HashMap<(String, MathStyle), Rc<MathScene>>,
    tex: &str,
    style: MathStyle,
) {
    let key = (tex.to_string(), style);
    if cache.contains_key(&key) {
        return;
    }
    let options = MathOptions {
        style,
        color: [0.0, 0.0, 0.0, 1.0],
    };
    if let Ok(scene) = math::typeset(tex, &options) {
        cache.insert(key, Rc::new(scene));
    }
}

/// The text-bearing parts of a block, so inline formulas can be typeset up front.
fn block_texts(block: &Block) -> Vec<&str> {
    match block {
        Block::Heading { text, .. } | Block::Paragraph(text) => vec![text],
        Block::ListItem { text, .. } => vec![text],
        Block::Quote(text) => vec![text],
        Block::Footnote { text, .. } => vec![text],
        Block::Callout { title, text, .. } => vec![title, text],
        Block::Table { headers, rows, .. } => {
            let mut out: Vec<&str> = headers.iter().map(String::as_str).collect();
            for row in rows {
                out.extend(row.iter().map(String::as_str));
            }
            out
        }
        Block::Code { .. } | Block::Rule | Block::Math { .. } | Block::Embed { .. } => Vec::new(),
    }
}

/// A display formula, centred and scaled down when it is wider than the reading column.
fn render_math(tex: &str, ctx: &RenderCtx) -> AnyElement {
    const FONT_SIZE: f32 = 17.0;
    // Usable width inside the reading column.
    let max_width: f32 = (ctx.measure - 44.0).max(320.0);

    let Some(scene) = ctx.math_scene(tex, MathStyle::Display) else {
        return math_error(tex, ctx.theme);
    };

    let width = scene.width() as f32 * FONT_SIZE;
    let scale = if width > max_width {
        (max_width / width).max(0.6)
    } else {
        1.0
    };

    MathElement::new(scene, FONT_SIZE * scale, ctx.theme.text).into_any_element()
}

/// Unsupported or malformed TeX: show the source, never a silent blank.
fn math_error(tex: &str, theme: Theme) -> AnyElement {
    div()
        .w_full()
        .flex()
        .justify_center()
        .py_2()
        .child(
            div()
                .px_2()
                .py_1()
                .rounded_md()
                .bg(theme.chip)
                .text_color(theme.warn)
                .text_sm()
                .child(format!("math: {tex}")),
        )
        .into_any_element()
}

/// Plain text, at the body weight and line height.
fn rich_text(
    text: &str,
    size: f32,
    block: Option<usize>,
    base: usize,
    ctx: &RenderCtx,
) -> AnyElement {
    rich_text_styled(text, size, block, base, ctx, gpui::FontWeight::NORMAL, 1.5)
}

/// Inline text â€” emphasis, code, links, math, highlights â€” at a given size, weight and
/// line height.
#[allow(clippy::too_many_arguments)]
fn rich_text_styled(
    text: &str,
    size: f32,
    block: Option<usize>,
    base: usize,
    ctx: &RenderCtx,
    weight: gpui::FontWeight,
    line_height: f32,
) -> AnyElement {
    let theme = ctx.theme;
    let template = ctx.template;
    let spans = inline::parse(text);
    let container = div().text_size(px(size)).line_height(relative(line_height));

    // Inline math cannot live inside one StyledText run, so paragraphs that contain it use a
    // wrapping row of word-sized text pieces and math boxes instead. Everything else keeps
    // the faster single-run path below.
    if spans
        .iter()
        .any(|span| (span.style.math && span.math.is_some()) || span.style.image)
    {
        let len = spans
            .iter()
            .map(|span| {
                span.math
                    .as_ref()
                    .map(|tex| tex.len() + 2)
                    .unwrap_or(span.text.len())
            })
            .sum();
        let content = flow_content(&spans, size, weight, line_height, ctx);
        return FlowText::new(
            content,
            block,
            base,
            len,
            ctx.selection.clone(),
            ctx.opener.cloned(),
            ctx.on_image.cloned(),
            ctx.theme,
        )
        .into_any_element();
    }

    let plain = spans
        .iter()
        .all(|span| span.style == inline::Style::default() && span.href.is_none());
    if plain && block.is_none() {
        let text: String = spans.iter().map(|span| span.text.as_str()).collect();
        return container.font_weight(weight).child(text).into_any_element();
    }

    let display: String = spans.iter().map(|span| span.text.as_str()).collect();
    let mut links: Vec<(Range<usize>, String)> = Vec::new();
    let mut offset = 0usize;
    let mut runs: Vec<TextRun> = Vec::with_capacity(spans.len());

    for span in &spans {
        let style = span.style;
        let mut font = if style.code {
            code_font(&template.font)
        } else {
            template.font.clone()
        };
        font.weight = if style.bold || style.footnote {
            gpui::FontWeight::BOLD
        } else {
            weight
        };
        if style.italic {
            font.style = FontStyle::Italic;
        }

        let color = if style.link || style.footnote {
            theme.accent_text
        } else if style.code || style.math {
            theme.code_text
        } else {
            template.color
        };

        if let Some(href) = &span.href {
            let end = offset + span.text.len();
            if end > offset {
                links.push((offset..end, href.clone()));
            }
        } else if style.footnote {
            let end = offset + span.text.len();
            if end > offset {
                links.push((offset..end, format!("#fn:{}", span.text.trim())));
            }
        }
        offset += span.text.len();

        if span.text.is_empty() {
            continue;
        }
        runs.push(TextRun {
            len: span.text.len(),
            font,
            color,
            background_color: if style.highlight {
                Some(theme.star)
            } else if style.code {
                Some(theme.code_bg)
            } else if style.math {
                Some(theme.chip)
            } else {
                None
            },
            underline: style.link.then_some(UnderlineStyle {
                thickness: px(1.0),
                color: Some(theme.accent_text),
                wavy: false,
            }),
            strikethrough: style.strikethrough.then_some(StrikethroughStyle {
                thickness: px(1.0),
                color: None,
            }),
        });
    }

    let styled = StyledText::new(display.clone()).with_runs(runs);

    match block {
        Some(block) => container
            .child(SelectableText {
                text: styled,
                display: Rc::new(display.clone()),
                block,
                base,
                len: display.len(),
                selection: ctx.selection.clone(),
                links,
                opener: ctx.opener.cloned(),
                list_state: ctx.list_state.cloned(),
                block_count: ctx.block_count,
                find: ctx
                    .find
                    .map(|_| ctx.find_matches(Some(block)))
                    .unwrap_or_default(),
            })
            .into_any_element(),
        None => container.child(styled).into_any_element(),
    }
}

/// Turn inline spans into measured flow pieces. Math spans become atomic boxes; the rest is
/// split at spaces so the flow layout can break lines between words.
fn flow_content(
    spans: &[inline::Inline],
    size: f32,
    weight: gpui::FontWeight,
    line_height: f32,
    ctx: &RenderCtx,
) -> Rc<FlowContent> {
    let theme = ctx.theme;
    let mut pieces = Vec::new();
    let mut offset = 0usize;

    for span in spans {
        if span.style.math {
            if let Some(tex) = &span.math {
                if let Some(scene) = ctx.math_scene(tex, MathStyle::Inline) {
                    // `block_display` copies math as `$tex$`; keep the piece range in sync.
                    let display_len = tex.len() + 2;
                    pieces.push(FlowPiece {
                        kind: FlowKind::Math {
                            scene,
                            color: theme.text,
                        },
                        offset,
                        len: display_len,
                    });
                    offset += display_len;
                    continue;
                }

                // Unsupported formula: show its source rather than a silent gap, and keep the
                // same copy length.
                let display = format!("${tex}$");
                pieces.push(FlowPiece {
                    kind: FlowKind::Text {
                        text: display.clone().into(),
                        font: ctx.template.font.clone(),
                        color: theme.warn,
                        background: Some(theme.chip),
                        underline: false,
                        strike: false,
                        href: None,
                    },
                    offset,
                    len: display.len(),
                });
                offset += display.len();
                continue;
            }
        }

        // An inline image: decode it once and place it as an atomic piece. The display text
        // stays the label, so selection and copy are unchanged.
        if span.style.image {
            if let (Some(root), Some(href)) = (ctx.vault_root, span.href.as_deref()) {
                if !href.starts_with("http://") && !href.starts_with("https://") {
                    if let Some(path) = resolve_file(root, ctx.attachments, href) {
                        if let Some((data, image_width, image_height)) = inline_image(&path) {
                            let height = (size * 2.4).max(12.0);
                            let width = if image_height > 0.0 {
                                height * (image_width / image_height)
                            } else {
                                height
                            };
                            pieces.push(FlowPiece {
                                kind: FlowKind::Image {
                                    data,
                                    width: px(width),
                                    height: px(height),
                                    href: span.href.clone(),
                                },
                                offset,
                                len: span.text.len(),
                            });
                            offset += span.text.len();
                            continue;
                        }
                    }
                }
            }
        }

        for chunk in span.text.split_inclusive(' ') {
            if chunk.is_empty() {
                continue;
            }
            let mut font = if span.style.code {
                code_font(&ctx.template.font)
            } else {
                ctx.template.font.clone()
            };
            font.weight = if span.style.bold || span.style.footnote {
                gpui::FontWeight::BOLD
            } else {
                weight
            };
            if span.style.italic {
                font.style = FontStyle::Italic;
            }
            let color = if span.style.link || span.style.footnote {
                theme.accent_text
            } else if span.style.code || span.style.math {
                theme.code_text
            } else {
                ctx.template.color
            };
            let background = if span.style.highlight {
                Some(theme.star)
            } else if span.style.code {
                Some(theme.code_bg)
            } else {
                None
            };
            let href = if span.style.footnote {
                Some(format!("#fn:{}", span.text.trim()))
            } else {
                span.href.clone()
            };
            pieces.push(FlowPiece {
                kind: FlowKind::Text {
                    text: chunk.to_string().into(),
                    font,
                    color,
                    background,
                    underline: span.style.link,
                    strike: span.style.strikethrough,
                    href,
                },
                offset,
                len: chunk.len(),
            });
            offset += chunk.len();
        }
    }

    Rc::new(FlowContent {
        pieces,
        font_size: size,
        line_height,
        link_color: theme.accent_text,
    })
}

/// A fenced code block, coloured by the small language lexer in `syntax`, with its
/// language shown above the code.
fn code_block(text: &str, language: &str, block: Option<usize>, ctx: &RenderCtx) -> AnyElement {
    let theme = ctx.theme;
    let font = code_font(&ctx.template.font);
    let content = text.strip_suffix('\n').unwrap_or(text);
    let lines: Vec<&str> = if content.is_empty() {
        vec![""]
    } else {
        content.split('\n').collect()
    };

    let mut rendered: Vec<AnyElement> = Vec::with_capacity(lines.len());
    let mut base = 0usize;
    for line in &lines {
        let spans = syntax::code_spans(line, language);
        let display: String = if spans.is_empty() {
            (*line).to_string()
        } else {
            spans.iter().map(|span| span.text.as_str()).collect()
        };

        let runs: Vec<TextRun> = if spans.is_empty() {
            vec![TextRun {
                len: display.len(),
                font: font.clone(),
                color: theme.code_text,
                background_color: None,
                underline: None,
                strikethrough: None,
            }]
        } else {
            spans
                .iter()
                .map(|span| TextRun {
                    len: span.text.len(),
                    font: font.clone(),
                    color: theme.code_color(span.kind),
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                })
                .collect()
        };

        let styled = StyledText::new(display.clone()).with_runs(runs);
        let piece: AnyElement = match block {
            Some(block) => SelectableText {
                text: styled,
                display: Rc::new(display.clone()),
                block,
                base,
                len: display.len(),
                selection: ctx.selection.clone(),
                links: Vec::new(),
                opener: ctx.opener.cloned(),
                list_state: ctx.list_state.cloned(),
                block_count: ctx.block_count,
                find: ctx
                    .find
                    .map(|_| ctx.find_matches(Some(block)))
                    .unwrap_or_default(),
            }
            .into_any_element(),
            None => div().whitespace_nowrap().child(styled).into_any_element(),
        };

        rendered.push(div().whitespace_nowrap().child(piece).into_any_element());
        base += display.len() + 1;
    }

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    language.hash(&mut hasher);
    let id = format!("reading-code-{:x}", hasher.finish());

    let language = language.trim();
    let copy_button = {
        let content = content.to_string();
        let copy = ctx.copy.cloned();
        div()
            .id(ElementId::Name(format!("{id}-copy").into()))
            .px_2()
            .py_1()
            .rounded_sm()
            .text_xs()
            .text_color(theme.muted)
            .bg(theme.chip)
            .cursor_pointer()
            .hover(move |style| style.bg(theme.hover).text_color(theme.text))
            .child("copy")
            .when_some(copy, |button, copy| {
                button.on_mouse_down(MouseButton::Left, move |_event, window, cx| {
                    copy(&content, window, cx);
                })
            })
    };

    div()
        .id(ElementId::Name(id.clone().into()))
        .w_full()
        .bg(theme.code_bg)
        .border_1()
        .border_color(theme.border)
        .rounded_md()
        .p_3()
        .text_size(px(13.0))
        .line_height(relative(1.45))
        .text_color(theme.code_text)
        .flex()
        .flex_col()
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .pb_2()
                .text_xs()
                .text_color(theme.muted)
                .child(language.to_string())
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(format!("{} lines", lines.len()))
                        .child(copy_button),
                ),
        )
        .child(
            div()
                .id(ElementId::Name(format!("{id}-scroll").into()))
                .flex()
                .flex_col()
                .overflow_x_scroll()
                .children(rendered),
        )
        .into_any_element()
}

fn code_font(base: &gpui::Font) -> gpui::Font {
    let mut font = base.clone();
    font.family = "Cascadia Mono".into();
    font.fallbacks = Some(gpui::FontFallbacks(std::sync::Arc::new(vec![
        "Consolas".to_string(),
        "DejaVu Sans Mono".to_string(),
        "Liberation Mono".to_string(),
        "Menlo".to_string(),
        "monospace".to_string(),
    ])));
    font
}

/// Byte range of the word around `index`, for double-click selection.
///
/// Letters, digits and `_` form a word. Clicking on the space after a word selects that
/// word; clicking between two non-word characters selects the character under the pointer,
/// so a double click always selects something visible.
fn word_range(text: &str, index: usize) -> Range<usize> {
    let index = index.min(text.len());
    let mut start = index;
    let mut end = index;

    while start > 0 {
        let Some((previous, ch)) = text[..start].char_indices().next_back() else {
            break;
        };
        if !is_word_char(ch) {
            break;
        }
        start = previous;
    }
    while end < text.len() {
        let ch = text[end..].chars().next().unwrap_or(' ');
        if !is_word_char(ch) {
            break;
        }
        end += ch.len_utf8();
    }

    if start == end {
        if let Some(ch) = text[end..].chars().next() {
            end += ch.len_utf8();
        } else if let Some((previous, _)) = text[..start].char_indices().next_back() {
            start = previous;
        }
    }

    start..end
}

fn is_word_char(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_'
}

/// A GFM pipe table, with the same inline math/highlight treatment as prose.
fn render_table(
    align: &[Align],
    headers: &[String],
    rows: &[Vec<String>],
    block: Option<usize>,
    ctx: &RenderCtx,
) -> AnyElement {
    let theme = ctx.theme;

    // Cell bases mirror `block_display`: cells joined by tabs, rows by newlines.
    let mut bases: Vec<Vec<usize>> = Vec::new();
    let mut offset = 0usize;
    for row in std::iter::once(headers).chain(rows.iter().map(Vec::as_slice)) {
        let mut row_bases = Vec::with_capacity(row.len());
        for (index, cell) in row.iter().enumerate() {
            row_bases.push(offset);
            offset += inline_display(cell).len();
            if index + 1 < row.len() {
                offset += 1;
            }
        }
        offset += 1;
        bases.push(row_bases);
    }

    let empty: Vec<usize> = Vec::new();
    let header = table_row(
        headers,
        align,
        block,
        bases.first().unwrap_or(&empty),
        ctx,
        true,
    );
    let body: Vec<AnyElement> = rows
        .iter()
        .enumerate()
        .map(|(index, row)| {
            table_row(
                row,
                align,
                block,
                bases.get(index + 1).unwrap_or(&empty),
                ctx,
                false,
            )
        })
        .collect();

    div()
        .w_full()
        .border_1()
        .border_color(theme.border)
        .rounded_md()
        .overflow_hidden()
        .flex()
        .flex_col()
        .child(header)
        .children(body)
        .into_any_element()
}

fn table_row(
    cells: &[String],
    align: &[Align],
    block: Option<usize>,
    bases: &[usize],
    ctx: &RenderCtx,
    header: bool,
) -> AnyElement {
    let theme = ctx.theme;
    let mut row = div()
        .w_full()
        .flex()
        .flex_row()
        .border_b_1()
        .border_color(theme.border);
    if header {
        row = row.bg(theme.panel_alt);
    }

    let rendered: Vec<AnyElement> = cells
        .iter()
        .enumerate()
        .map(|(index, cell)| {
            let mut cell_element = div()
                .flex_1()
                .min_w(px(0.0))
                .px_3()
                .py_2()
                .border_r_1()
                .border_color(theme.border);
            cell_element = match align.get(index) {
                Some(Align::Center) => cell_element.text_center(),
                Some(Align::Right) => cell_element.text_right(),
                _ => cell_element.text_left(),
            };
            cell_element
                .child(rich_text_styled(
                    cell,
                    14.0,
                    block,
                    bases.get(index).copied().unwrap_or(0),
                    ctx,
                    if header {
                        gpui::FontWeight::BOLD
                    } else {
                        gpui::FontWeight::NORMAL
                    },
                    1.4,
                ))
                .into_any_element()
        })
        .collect();

    row.children(rendered).into_any_element()
}

fn render_embed(
    target: &str,
    width: Option<u32>,
    alt: Option<&str>,
    fragment: Option<&str>,
    ctx: &RenderCtx,
) -> AnyElement {
    let theme = ctx.theme;
    let Some(root) = ctx.vault_root else {
        return placeholder(target, "no vault root", theme);
    };
    let attachments = ctx.attachments;

    if is_image(target) {
        if target.starts_with("http://") || target.starts_with("https://") {
            // The app is offline by default, so remote images are not fetched.
            return placeholder(target, "remote image (network is off)", theme);
        }
        return match resolve_file(root, attachments, target) {
            Some(path) => {
                let requested = width.map(|width| width as f32);
                let mut picture = img(path.clone()).max_w(px(requested.unwrap_or(480.0)));
                if requested.is_none() {
                    picture = picture.max_h(px(360.0));
                }
                let picture = picture.into_any_element();
                let on_image = ctx.on_image.cloned();
                let path_string = path.to_string_lossy().to_string();
                let caption = alt.map(str::to_string);
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .id(ElementId::Name(
                                format!("embed-image-{}", path_string).into(),
                            ))
                            .cursor_pointer()
                            .on_mouse_down(MouseButton::Left, move |_event, window, cx| {
                                if let Some(on_image) = &on_image {
                                    on_image(&path_string, window, cx);
                                }
                            })
                            .child(picture),
                    )
                    .when_some(caption, |d, caption| {
                        d.child(div().text_xs().text_color(theme.muted).child(caption))
                    })
                    .into_any_element()
            }
            None => placeholder(target, "image not found", theme),
        };
    }

    if is_attachment(target) {
        let Some(path) = resolve_file(root, attachments, target) else {
            return placeholder(target, "attachment not found", theme);
        };
        let kind = Path::new(target)
            .extension()
            .and_then(|extension| extension.to_str())
            .map(|extension| extension.to_ascii_uppercase())
            .unwrap_or_else(|| "FILE".to_string());
        let name = Path::new(target)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(target)
            .to_string();
        let open_file = ctx.open_file.cloned();
        let path_string = path.to_string_lossy().to_string();
        return div()
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .p_3()
            .rounded_md()
            .border_1()
            .border_color(theme.border)
            .bg(theme.panel_alt)
            .child(
                div()
                    .px_2()
                    .py_1()
                    .rounded_sm()
                    .bg(theme.chip)
                    .text_xs()
                    .text_color(theme.muted)
                    .child(kind),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .text_sm()
                    .text_color(theme.text)
                    .child(name),
            )
            .child(
                div()
                    .id(ElementId::Name(
                        format!("embed-open-{}", path_string).into(),
                    ))
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .bg(theme.chip)
                    .text_xs()
                    .text_color(theme.text_dim)
                    .cursor_pointer()
                    .hover(move |style| style.bg(theme.hover))
                    .child("Open")
                    .on_mouse_down(MouseButton::Left, move |_event, window, cx| {
                        if let Some(open_file) = &open_file {
                            open_file(&path_string, window, cx);
                        }
                    }),
            )
            .into_any_element();
    }

    if ctx.depth >= MAX_EMBED_DEPTH {
        return placeholder(target, "embed depth limit", theme);
    }

    let path = resolve_note(root, attachments, target);
    let Some(cache) = ctx.embeds else {
        return placeholder(target, "embed cache unavailable", theme);
    };
    let modified = std::fs::metadata(&path)
        .ok()
        .and_then(|metadata| metadata.modified().ok());
    let key = (path.clone(), fragment.map(str::to_string));

    let blocks = {
        let mut embeds = cache.borrow_mut();
        match embeds.get(&key) {
            Some(entry) if entry.modified == modified => entry.blocks.clone(),
            _ => {
                let Ok(text) = std::fs::read_to_string(&path) else {
                    return placeholder(target, "note not found", theme);
                };
                // Shared blocks are expanded once per parse, not per frame.
                let text = shared::expand(&text, root);
                let (parsed, spans) = markdown::parse_blocks_spanned(&text);
                let parsed = match fragment {
                    Some(fragment) if fragment.starts_with('^') => {
                        match block_by_id(&parsed, &spans, &text, fragment) {
                            Some(block) => vec![block],
                            None => {
                                return placeholder(target, "block not found", theme);
                            }
                        }
                    }
                    Some(fragment) => match section_blocks(parsed, fragment) {
                        Some(section) => section,
                        None => {
                            return placeholder(target, "section not found", theme);
                        }
                    },
                    None => parsed,
                };
                let blocks = Rc::new(parsed);
                embeds.insert(
                    key,
                    EmbeddedNote {
                        modified,
                        blocks: blocks.clone(),
                    },
                );
                blocks
            }
        }
    };

    div()
        .border_l_1()
        .border_color(theme.border)
        .pl_3()
        .child(render_blocks(&blocks, ctx))
        .into_any_element()
}

/// Resolve a link target to a file: the direct path first, then the vault-wide name index.
fn resolve_file(root: &Path, attachments: Option<&Attachments>, target: &str) -> Option<PathBuf> {
    let direct = root.join(target);
    if direct.is_file() {
        return Some(direct);
    }
    let resolved = attachments?.resolve(target)?;
    let path = root.join(resolved);
    path.is_file().then_some(path)
}

/// A decoded inline image: pixel data and intrinsic size.
type DecodedImage = Option<(Arc<RenderImage>, f32, f32)>;

thread_local! {
    /// Decoded inline images, keyed by path and modification time.
    static INLINE_IMAGES: RefCell<HashMap<(PathBuf, SystemTime), DecodedImage>> =
        RefCell::new(HashMap::new());
}

/// Decode an image for inline rendering. Returns `(data, width, height)` in image pixels.
fn inline_image(path: &Path) -> DecodedImage {
    let modified = std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()?;
    let key = (path.to_path_buf(), modified);
    INLINE_IMAGES.with(|cache| {
        if let Some(hit) = cache.borrow().get(&key) {
            return hit.clone();
        }
        let loaded = std::fs::read(path)
            .ok()
            .and_then(|bytes| image::load_from_memory(&bytes).ok())
            .map(|decoded| {
                let width = decoded.width() as f32;
                let height = decoded.height() as f32;
                let frame = image::Frame::new(decoded.into_rgba8());
                let data = Arc::new(RenderImage::new(smallvec::SmallVec::from_buf_and_len(
                    [frame],
                    1,
                )));
                (data, width, height)
            });
        cache.borrow_mut().insert(key, loaded.clone());
        loaded
    })
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

/// A non-note, non-image file: PDF, audio, video.
fn is_attachment(target: &str) -> bool {
    matches!(
        Path::new(target)
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .as_deref(),
        Some("pdf" | "mp3" | "wav" | "ogg" | "flac" | "mp4" | "webm" | "mov" | "m4a")
    )
}

fn resolve_note(root: &Path, attachments: Option<&Attachments>, target: &str) -> PathBuf {
    if let Some(path) = resolve_file(root, attachments, target) {
        return path;
    }
    let with_ext = format!("{target}.md");
    if let Some(path) = resolve_file(root, attachments, &with_ext) {
        return path;
    }
    root.join(with_ext)
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

/// The text a block contributes to a reading-mode selection.
/// Hide a trailing `^block-id` marker: it labels the block, it is not prose.
fn strip_block_id(text: String) -> String {
    if let Some(space) = text.rfind(' ') {
        let tail = &text[space + 1..];
        if tail.len() > 1
            && tail.starts_with('^')
            && tail[1..]
                .chars()
                .all(|ch| ch.is_alphanumeric() || ch == '-' || ch == '_')
        {
            return text[..space].to_string();
        }
    }
    text
}

fn block_display(block: &Block) -> String {
    match block {
        Block::Heading { text, .. } => inline_display(text),
        Block::Paragraph(text) | Block::Quote(text) | Block::ListItem { text, .. } => {
            strip_block_id(inline_display(text))
        }
        Block::Code { text, .. } => text.strip_suffix('\n').unwrap_or(text).to_string(),
        Block::Math { tex } => format!("$${tex}$$"),
        Block::Callout { title, text, .. } => {
            let mut out = inline_display(title);
            if !text.trim().is_empty() {
                out.push('\n');
                out.push_str(&inline_display(text));
            }
            out
        }
        Block::Table { headers, rows, .. } => {
            let mut out = String::new();
            let mut first_row = true;
            for row in std::iter::once(headers).chain(rows.iter()) {
                if !first_row {
                    out.push('\n');
                }
                first_row = false;
                for (index, cell) in row.iter().enumerate() {
                    if index > 0 {
                        out.push('\t');
                    }
                    out.push_str(&inline_display(cell));
                }
            }
            out
        }
        Block::Footnote { id, text } => format!("[{id}] {text}"),
        Block::Rule | Block::Embed { .. } => String::new(),
    }
}

/// The text `inline::parse` shows, with markdown removed. Math keeps its TeX source so
/// copying a selection preserves `$…$` for pasting back into a note.
fn inline_display(text: &str) -> String {
    inline::parse(text)
        .into_iter()
        .map(|span| match span.math {
            Some(tex) => format!("${tex}$"),
            None => span.text,
        })
        .collect()
}

/// A `StyledText` that can be drag-selected and whose links are clickable.
///
/// Reading blocks are laid out as separate elements, so each text piece reports its own
/// slice of a document-wide [`ReadSelection`]; `base` is the piece's byte offset within
/// its block, which lets a selection run across wrapped code lines and callout titles.
struct SelectableText {
    text: StyledText,
    /// The display text this piece was shaped from, for word and paragraph selection.
    display: Rc<String>,
    block: usize,
    base: usize,
    len: usize,
    selection: Rc<RefCell<ReadSelection>>,
    links: Vec<(Range<usize>, String)>,
    opener: Option<LinkOpener>,
    /// The reading list and its size, so dragging past the viewport can scroll.
    list_state: Option<ListState>,
    block_count: usize,
    /// In-page find matches in this piece, as `(range, is_current)`.
    find: Vec<(Range<usize>, bool)>,
}

impl IntoElement for SelectableText {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for SelectableText {
    type RequestLayoutState = ();
    type PrepaintState = Hitbox;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        self.text.request_layout(id, inspector_id, window, cx)
    }

    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Hitbox {
        self.text
            .prepaint(id, inspector_id, bounds, state, window, cx);
        window.insert_hitbox(bounds, HitboxBehavior::Normal)
    }

    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut (),
        hitbox: &mut Hitbox,
        window: &mut Window,
        cx: &mut App,
    ) {
        let theme = *cx.global::<Theme>();
        let layout = self.text.layout().clone();
        let (block, base, len) = (self.block, self.base, self.len);

        // Mouse listeners must be registered during paint, not prepaint.
        let down_selection = self.selection.clone();
        let down_layout = layout.clone();
        let down_text = self.display.clone();
        let down_list = self.list_state.clone();
        let down_blocks = self.block_count;
        window.on_mouse_event(
            move |event: &MouseDownEvent, phase, window: &mut Window, _cx: &mut App| {
                if phase != DispatchPhase::Bubble || event.button != MouseButton::Left {
                    return;
                }
                if !bounds.contains(&event.position) {
                    return;
                }
                let Ok(index) = down_layout.index_for_position(event.position) else {
                    return;
                };
                let index = index.min(len);
                let point = ReadPoint {
                    block,
                    offset: base + index,
                };
                let mut selection = down_selection.borrow_mut();

                // Shift-click extends from the existing anchor; a double click selects the
                // word under the pointer; a triple click selects the whole piece (the
                // paragraph, cell, callout line or code line).
                match event.click_count {
                    _ if event.modifiers.shift && selection.anchor.is_some() => {
                        selection.head = Some(point);
                        selection.dragging = true;
                        selection.moved = true;
                    }
                    2 => {
                        let word = word_range(down_text.as_str(), index);
                        selection.anchor = Some(ReadPoint {
                            block,
                            offset: base + word.start,
                        });
                        selection.head = Some(ReadPoint {
                            block,
                            offset: base + word.end,
                        });
                        selection.dragging = false;
                        selection.moved = true;
                        selection.down = None;
                    }
                    3 => {
                        selection.anchor = Some(ReadPoint {
                            block,
                            offset: base,
                        });
                        selection.head = Some(ReadPoint {
                            block,
                            offset: base + len,
                        });
                        selection.dragging = false;
                        selection.moved = true;
                        selection.down = None;
                    }
                    _ => {
                        selection.anchor = Some(point);
                        selection.head = Some(point);
                        selection.down = Some(event.position);
                        selection.dragging = true;
                        selection.moved = false;
                    }
                }
                drop(selection);

                // Keep the anchor block visible while dragging beyond the viewport.
                if event.position.y < bounds.top() || event.position.y > bounds.bottom() {
                    if let Some(list_state) = &down_list {
                        let target = if event.position.y < bounds.top() {
                            block.saturating_sub(1)
                        } else {
                            block + 1
                        };
                        list_state.scroll_to_reveal_item(target.min(down_blocks.saturating_sub(1)));
                    }
                }
                window.refresh();
            },
        );

        let move_selection = self.selection.clone();
        let move_layout = layout.clone();
        let move_list = self.list_state.clone();
        let move_blocks = self.block_count;
        window.on_mouse_event(
            move |event: &MouseMoveEvent, phase, window: &mut Window, _cx: &mut App| {
                if phase != DispatchPhase::Bubble {
                    return;
                }
                let mut selection = move_selection.borrow_mut();
                if !selection.dragging {
                    return;
                }
                // Dragging past the top or bottom scrolls the reading list one block per
                // move event; the pointer no longer needs to stay on text.
                if event.position.y < bounds.top() || event.position.y > bounds.bottom() {
                    if let Some(list_state) = &move_list {
                        let target = if event.position.y < bounds.top() {
                            block.saturating_sub(1)
                        } else {
                            block + 1
                        };
                        list_state.scroll_to_reveal_item(target.min(move_blocks.saturating_sub(1)));
                        selection.moved = true;
                        drop(selection);
                        window.refresh();
                    }
                    return;
                }
                // Vertical hit-testing only: dragging past the end of a line still
                // extends the selection to that line's end.
                let index = match move_layout.index_for_position(event.position) {
                    Ok(index) | Err(index) => index,
                };
                let point = ReadPoint {
                    block,
                    offset: base + index.min(len),
                };
                if selection.head != Some(point) {
                    selection.head = Some(point);
                    selection.moved = true;
                    drop(selection);
                    window.refresh();
                }
            },
        );

        let up_selection = self.selection.clone();
        let up_links = self.links.clone();
        let up_opener = self.opener.clone();
        window.on_mouse_event(
            move |event: &MouseUpEvent, phase, window: &mut Window, cx: &mut App| {
                if phase != DispatchPhase::Bubble {
                    return;
                }
                let mut selection = up_selection.borrow_mut();
                if !selection.dragging {
                    return;
                }
                selection.dragging = false;
                // A click with a little jitter is still a click on the link under it.
                let clicked = match selection.down {
                    Some(down) => (event.position - down).magnitude() < 4.0,
                    None => !selection.moved,
                };
                let head = selection.head;
                if clicked {
                    selection.clear();
                }
                drop(selection);

                if clicked {
                    if let Some(head) = head {
                        let local = head.offset.saturating_sub(base);
                        if let Some((_, target)) =
                            up_links.iter().find(|(range, _)| range.contains(&local))
                        {
                            if let Some(opener) = &up_opener {
                                opener(target, window, cx);
                            }
                        }
                    }
                }
                window.refresh();
            },
        );

        // In-page find highlights sit behind the text and under the selection.
        for (range, current) in &self.find {
            let from = range.start.max(base);
            let to = range.end.min(base + len);
            if from >= to {
                continue;
            }
            let color = if *current {
                theme.star.opacity(0.55)
            } else {
                theme.accent.opacity(0.22)
            };
            for quad in selection_quads(
                &layout,
                self.block,
                self.base,
                self.len,
                ReadPoint {
                    block: self.block,
                    offset: from,
                },
                ReadPoint {
                    block: self.block,
                    offset: to,
                },
                color,
            ) {
                window.paint_quad(quad);
            }
        }

        if let Some((start, end)) = self.selection.borrow().ordered() {
            for quad in selection_quads(
                &layout,
                self.block,
                self.base,
                self.len,
                start,
                end,
                theme.selection,
            ) {
                window.paint_quad(quad);
            }
        }

        if !self.links.is_empty() {
            if let Ok(index) = layout.index_for_position(window.mouse_position()) {
                if self.links.iter().any(|(range, _)| range.contains(&index)) {
                    window.set_cursor_style(CursorStyle::PointingHand, hitbox);
                }
            }
        }

        let mut prepaint = ();
        self.text
            .paint(id, inspector_id, bounds, state, &mut prepaint, window, cx)
    }
}

/// Highlight quads for the part of `[start, end)` that falls inside this text piece.
fn selection_quads(
    layout: &TextLayout,
    block: usize,
    base: usize,
    len: usize,
    start: ReadPoint,
    end: ReadPoint,
    color: gpui::Hsla,
) -> Vec<PaintQuad> {
    if start == end || end.block < block || start.block > block {
        return Vec::new();
    }

    let piece_end = base + len;
    let from = if start.block == block {
        start.offset.max(base)
    } else {
        base
    };
    let to = if end.block == block {
        end.offset.min(piece_end)
    } else {
        piece_end
    };
    if from >= to {
        return Vec::new();
    }

    let (Some(p0), Some(p1)) = (
        layout.position_for_index(from - base),
        layout.position_for_index(to - base),
    ) else {
        return Vec::new();
    };

    let height = layout.line_height();
    let bounds = layout.bounds();
    let mut quads = Vec::new();

    if f32::from(p0.y - p1.y).abs() < 0.5 {
        quads.push(fill(
            Bounds::from_corners(point(p0.x, p0.y), point(p1.x, p1.y + height)),
            color,
        ));
    } else {
        quads.push(fill(
            Bounds::from_corners(point(p0.x, p0.y), point(bounds.right(), p0.y + height)),
            color,
        ));
        let mut y = p0.y + height;
        while y + px(0.5) < p1.y {
            quads.push(fill(
                Bounds::from_corners(point(bounds.left(), y), point(bounds.right(), y + height)),
                color,
            ));
            y += height;
        }
        if p1.x > bounds.left() + px(0.5) {
            quads.push(fill(
                Bounds::from_corners(point(bounds.left(), p1.y), point(p1.x, p1.y + height)),
                color,
            ));
        }
    }

    quads
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reading_cache_reparses_only_when_the_version_changes() {
        let mut cache = ReadingCache::new();
        assert!(cache.needs_update((1, 0)));

        cache.update((1, 0), "# Title\n\nBody", None);
        assert!(!cache.needs_update((1, 0)));
        assert!(cache.blocks.len() >= 2);
        assert_eq!(cache.list_state.item_count(), cache.blocks.len());

        let before = cache.blocks.len();
        cache.update((1, 1), "# Title\n\nBody\n\nMore", None);
        assert!(cache.blocks.len() > before);
        assert_eq!(cache.list_state.item_count(), cache.blocks.len());
    }

    #[test]
    fn reading_cache_resets_for_a_new_document() {
        let mut cache = ReadingCache::new();
        cache.update((1, 4), "# Title\n\nBody", None);
        assert!(!cache.blocks.is_empty());

        cache.update((2, 0), "", None);
        assert!(cache.blocks.is_empty());
        assert_eq!(cache.list_state.item_count(), 0);
    }

    #[test]
    fn selection_spans_blocks_and_ignores_markup() {
        let mut cache = ReadingCache::new();
        cache.update((1, 0), "One **two**\n\n`Three` four\n", None);
        cache.select_all();

        assert_eq!(
            cache.selected_text().as_deref(),
            Some("One two\nThree four")
        );
        let stats = cache.selection_stats().unwrap();
        assert_eq!(stats.words, 4);
        assert_eq!(stats.lines, 2);
    }

    #[test]
    fn the_footnote_popup_remembers_and_clears_its_reference() {
        let refs = Rc::new(RefCell::new(HashMap::new()));
        let defs = Rc::new(RefCell::new(HashMap::new()));
        let mut popup = FootnotePopup::new(
            refs,
            defs,
            ListState::new(0, ListAlignment::Top, px(600.0)),
            Rc::new(RefCell::new(None)),
        );
        let anchor = point(px(120.0), px(80.0));
        popup.show_at("1", Some(anchor));
        assert!(popup.is_open());
        assert_eq!(popup.anchor(), Some(anchor));
        popup.close();
        assert!(!popup.is_open());
        assert_eq!(popup.anchor(), None);
    }

    #[test]
    fn footnote_references_are_grouped_for_the_picker() {
        let mut cache = ReadingCache::new();
        cache.update((1, 0), "One[^1].\n\nTwo[^1].\n\n[^1]: the note\n", None);

        let refs = cache.footnote_refs.borrow();
        let one = refs.get("1").expect("footnote 1 references");
        assert_eq!(one.len(), 2, "both paragraphs cite the same footnote");
        assert!(one[0].snippet.contains("One"));
        assert!(one[1].snippet.contains("Two"));

        let defs = cache.footnote_defs.borrow();
        assert!(defs.contains_key("1"));

        let mut popup = FootnotePopup::new(
            cache.footnote_refs.clone(),
            cache.footnote_defs.clone(),
            ListState::new(0, ListAlignment::Top, px(600.0)),
            Rc::new(RefCell::new(None)),
        );
        popup.show_at("1", None);
        let (id, options, definition) = popup.open.as_ref().expect("picker opened");
        assert_eq!(id, "1");
        assert_eq!(options.len(), 2);
        assert!(definition.is_some());
    }

    #[test]
    fn copying_preserves_tex_source() {
        let mut cache = ReadingCache::new();
        cache.update(
            (1, 0),
            "Energy $E = mc^2$ and $\\frac{a}{b}$.\n\n$$\\int_0^1 x\\,dx$$\n",
            None,
        );
        cache.select_all();

        let text = cache.selected_text().unwrap();
        assert!(text.contains("$E = mc^2$"), "got {text:?}");
        assert!(text.contains("$\\frac{a}{b}$"), "got {text:?}");
        assert!(text.contains("$$\\int_0^1 x\\,dx$$"), "got {text:?}");
        assert!(
            !text.contains("mc²"),
            "copy must not use the transliteration"
        );
    }

    #[test]
    fn inline_display_keeps_math_delimiters() {
        assert_eq!(inline_display("a $x^2$ b"), "a $x^2$ b");
    }

    #[test]
    fn selection_clears_when_the_document_changes() {
        let mut cache = ReadingCache::new();
        cache.update((1, 0), "Body\n", None);
        cache.select_all();
        assert!(cache.selected_text().is_some());

        cache.update((1, 1), "Body\n\nMore\n", None);
        assert_eq!(cache.selected_text(), None);
    }

    #[test]
    fn block_text_is_the_display_text() {
        let block = Block::Paragraph("**bold** and `code`".to_string());
        assert_eq!(block_display(&block), "bold and code");
    }

    #[test]
    fn table_text_joins_cells_with_tabs() {
        let block = Block::Table {
            align: Vec::new(),
            headers: vec!["a".to_string(), "b".to_string()],
            rows: vec![vec!["1".to_string(), "2".to_string()]],
        };
        assert_eq!(block_display(&block), "a\tb\n1\t2");
    }

    #[test]
    fn a_folded_reading_heading_hides_blocks_until_its_sibling() {
        let blocks = vec![
            Block::Heading {
                level: 1,
                text: "One".to_string(),
            },
            Block::Paragraph("body".to_string()),
            Block::Heading {
                level: 2,
                text: "Nested".to_string(),
            },
            Block::Paragraph("nested body".to_string()),
            Block::Heading {
                level: 1,
                text: "Two".to_string(),
            },
        ];
        let mut folded = HashSet::new();
        folded.insert(0);

        let hidden = hidden_blocks(&blocks, &folded);

        assert_eq!(hidden, vec![false, true, true, true, false]);
    }

    #[test]
    fn double_click_selects_the_word_under_the_pointer() {
        assert_eq!(word_range("hello world", 7), 6..11);
        assert_eq!(word_range("hello world", 0), 0..5);
        // On the space after a word, that word is selected.
        assert_eq!(word_range("hello world", 5), 0..5);
        // Punctuation still selects one visible character.
        assert_eq!(word_range("a  b", 2), 2..3);
        // Multi-byte text keeps byte boundaries.
        assert_eq!(word_range("café au lait", 1), 0..5);
        assert_eq!(word_range("café au lait", 6), 6..8);
    }

    #[test]
    fn keyboard_selection_moves_between_characters_and_blocks() {
        let mut cache = ReadingCache::new();
        cache.update((1, 1), "alpha\n\nbeta", None);
        {
            let mut selection = cache.selection.borrow_mut();
            selection.anchor = Some(ReadPoint {
                block: 1,
                offset: 0,
            });
            selection.head = Some(ReadPoint {
                block: 1,
                offset: 0,
            });
        }

        cache.extend_selection(SelectionMotion::Right);
        assert_eq!(
            cache.selection.borrow().head,
            Some(ReadPoint {
                block: 1,
                offset: 1
            })
        );
        cache.extend_selection(SelectionMotion::LineEnd);
        assert_eq!(
            cache.selection.borrow().head,
            Some(ReadPoint {
                block: 1,
                offset: 4
            })
        );
        assert_eq!(cache.selected_text().as_deref(), Some("beta"));

        // Walking left at the start of a block continues at the previous block's end.
        cache.extend_selection(SelectionMotion::LineStart);
        cache.extend_selection(SelectionMotion::Left);
        assert_eq!(
            cache.selection.borrow().head,
            Some(ReadPoint {
                block: 0,
                offset: 5
            })
        );
    }

    #[test]
    fn blocks_map_back_to_source_lines() {
        let mut cache = ReadingCache::new();
        cache.update((1, 1), "alpha\n\nbeta\n\ngamma", None);
        assert_eq!(cache.block_for_line(0), Some(0));
        assert_eq!(cache.block_for_line(2), Some(1));
        assert_eq!(cache.block_for_line(4), Some(2));
        assert_eq!(cache.block_start_line(1), Some(2));
        assert_eq!(cache.block_start_line(2), Some(4));
        // A blank line between blocks belongs to the block above it.
        assert_eq!(cache.block_for_line(3), Some(1));
        // Past the end is the last block.
        assert_eq!(cache.block_for_line(99), Some(2));
    }

    #[test]
    fn a_note_embed_section_stops_at_the_next_sibling_heading() {
        let (blocks, _) = markdown::parse_blocks_spanned("# One\na\n## Sub\nb\n# Two\nc\n");
        let section = section_blocks(blocks.clone(), "One").expect("section");
        assert_eq!(section.len(), 4, "{section:?}");
        assert!(matches!(section.last(), Some(Block::Paragraph(text)) if text == "b"));

        let section = section_blocks(blocks, "Sub").expect("section");
        assert_eq!(section.len(), 2);
        assert!(matches!(&section[0], Block::Heading { text, .. } if text == "Sub"));
    }

    #[test]
    fn a_note_embed_block_targets_the_block_with_its_id() {
        let text = "# One\nfirst ^a\n\nsecond ^b\n";
        let (blocks, spans) = markdown::parse_blocks_spanned(text);
        let block = block_by_id(&blocks, &spans, text, "^b").expect("block");
        assert_eq!(block_display(&block), "second");
        assert!(block_by_id(&blocks, &spans, text, "^gone").is_none());
    }

    #[test]
    fn a_missing_embed_section_is_reported() {
        let (blocks, _) = markdown::parse_blocks_spanned("# One\ntext\n");
        assert!(section_blocks(blocks, "Nope").is_none());
        assert!(section_blocks(markdown::parse_blocks("# One\ntext\n"), "").is_none());
    }

    #[test]
    fn an_in_page_find_counts_and_cycles_matches() {
        let mut cache = ReadingCache::new();
        cache.update((1, 1), "alpha beta\n\nbeta gamma", None);
        cache.open_find();
        cache.find_append("beta");

        let (current, total) = cache.find_position();
        assert_eq!(total, 2);
        assert_eq!(current, 1);
        assert!(cache.find.matches.iter().any(|found| found.block == 0));
        assert!(cache.find.matches.iter().any(|found| found.block == 1));

        cache.find_next(false);
        assert_eq!(cache.find_position().0, 2);
        // Wraps around.
        cache.find_next(false);
        assert_eq!(cache.find_position().0, 1);
        cache.find_next(true);
        assert_eq!(cache.find_position().0, 2);

        // Backspace follows the pattern.
        cache.find_backspace();
        cache.find_backspace();
        cache.find_backspace();
        cache.find_backspace();
        assert_eq!(cache.find_pattern(), "");
        assert_eq!(cache.find_position(), (0, 0));

        cache.close_find();
        assert!(!cache.find_open());
    }

    #[test]
    fn an_in_page_find_honours_regex_case_and_whole_word() {
        let mut cache = ReadingCache::new();
        cache.update((1, 1), "Beta beta betamax", None);
        cache.open_find();
        cache.find_append("beta");
        assert_eq!(cache.find_position().1, 3);

        cache.find_toggle_case();
        assert_eq!(cache.find_position().1, 2);

        cache.find_toggle_word();
        assert_eq!(cache.find_position().1, 1);

        // An invalid regex reports instead of panicking.
        cache.find_toggle_word();
        cache.find_toggle_regex();
        cache.find_append("(");
        assert!(cache.find_error().is_some());
    }

    #[test]
    fn an_in_page_find_never_touches_the_document() {
        let mut cache = ReadingCache::new();
        cache.update((1, 1), "alpha beta", None);
        let before: Vec<String> = cache.blocks.iter().map(block_display).collect();
        cache.open_find();
        cache.find_append("beta");
        cache.find_next(false);
        let after: Vec<String> = cache.blocks.iter().map(block_display).collect();
        assert_eq!(before, after);
    }

    #[test]
    fn the_reading_selection_can_be_cleared() {
        let mut cache = ReadingCache::new();
        cache.update((1, 1), "alpha beta", None);
        assert!(!cache.clear_selection());

        {
            let mut selection = cache.selection.borrow_mut();
            selection.anchor = Some(ReadPoint {
                block: 0,
                offset: 0,
            });
            selection.head = Some(ReadPoint {
                block: 0,
                offset: 5,
            });
        }
        assert!(cache.clear_selection());
        assert!(cache.selection.borrow().ordered().is_none());
    }
}
