//! Editorial block model: split a note into movable blocks and edit them.
//!
//! Blocks are the units an editor wants to rearrange — a heading, a paragraph, a list, a
//! fenced code block, a quote, a callout, math, a rule or a standalone embed. Every
//! operation is a pure text transformation so the same code drives the UI, the CLI and
//! tests, and the user's file stays plain markdown.

use crate::frontmatter;

/// What a block is, for labels and icons in the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockKind {
    Heading,
    Paragraph,
    List,
    Code,
    Quote,
    Callout,
    Math,
    Rule,
    Embed,
}

impl BlockKind {
    pub fn name(self) -> &'static str {
        match self {
            BlockKind::Heading => "heading",
            BlockKind::Paragraph => "paragraph",
            BlockKind::List => "list",
            BlockKind::Code => "code",
            BlockKind::Quote => "quote",
            BlockKind::Callout => "callout",
            BlockKind::Math => "math",
            BlockKind::Rule => "rule",
            BlockKind::Embed => "embed",
        }
    }
}

/// A block's half-open line range (`end` is exclusive) in the source document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub kind: BlockKind,
    pub start: usize,
    pub end: usize,
}

impl Block {
    pub fn lines(&self) -> usize {
        self.end.saturating_sub(self.start)
    }
}

/// The result of an edit: the new document and the line the caret should land on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockEdit {
    pub text: String,
    pub line: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChunkKind {
    Frontmatter,
    Blank,
    Block(BlockKind),
}

#[derive(Debug, Clone)]
struct Chunk {
    kind: ChunkKind,
    text: String,
}

impl Chunk {
    fn is_block(&self) -> bool {
        matches!(self.kind, ChunkKind::Block(_))
    }

    fn kind(&self) -> Option<BlockKind> {
        match self.kind {
            ChunkKind::Block(kind) => Some(kind),
            _ => None,
        }
    }
}

/// Every block in `src`, in document order. Frontmatter and blank lines are skipped but
/// still counted, so `start`/`end` are source line numbers.
pub fn parse(src: &str) -> Vec<Block> {
    let chunks = chunks(src);
    let mut blocks = Vec::new();
    let mut line = 0usize;

    for chunk in &chunks {
        let count = chunk.text.matches('\n').count();
        if let Some(kind) = chunk.kind() {
            blocks.push(Block {
                kind,
                start: line,
                end: line + count + usize::from(!chunk.text.ends_with('\n')),
            });
        }
        line += count;
        // A chunk that does not end in a newline is the last line of the document.
        if !chunk.text.ends_with('\n') {
            // Nothing follows, but keep the accumulator honest if it ever does.
        }
    }

    blocks
}

/// The block containing `line` (0-based), or the closest block to it.
pub fn block_at(src: &str, line: usize) -> Option<Block> {
    let blocks = parse(src);
    if blocks.is_empty() {
        return None;
    }

    if let Some(block) = blocks
        .iter()
        .find(|block| line >= block.start && line < block.end)
    {
        return Some(block.clone());
    }

    // A blank line between blocks: prefer the block above, then the one below.
    blocks
        .iter()
        .rev()
        .find(|block| block.end <= line)
        .or_else(|| blocks.iter().find(|block| block.start > line))
        .cloned()
}

/// Move the block containing `line` up one position.
pub fn move_up(src: &str, line: usize) -> Option<BlockEdit> {
    move_block(src, line, true)
}

/// Move the block containing `line` down one position.
pub fn move_down(src: &str, line: usize) -> Option<BlockEdit> {
    move_block(src, line, false)
}

fn move_block(src: &str, line: usize, up: bool) -> Option<BlockEdit> {
    let mut chunks = chunks(src);
    let current = block_chunk_for_line(&chunks, line)?;

    let neighbour = if up {
        (0..current).rev().find(|index| chunks[*index].is_block())
    } else {
        (current + 1..chunks.len()).find(|index| chunks[*index].is_block())
    }?;

    chunks.swap(current, neighbour);
    let text = join(&chunks);

    Some(BlockEdit {
        line: line_of_chunk(&chunks, neighbour),
        text,
    })
}

/// Insert a copy of the block containing `line` directly beneath it.
pub fn duplicate(src: &str, line: usize) -> Option<BlockEdit> {
    let mut chunks = chunks(src);
    let current = block_chunk_for_line(&chunks, line)?;
    let copy = chunks[current].clone();

    chunks.insert(current + 1, copy);
    let text = join(&chunks);

    Some(BlockEdit {
        line: line_of_chunk(&chunks, current + 1),
        text,
    })
}

/// Remove the block containing `line` and one adjacent blank separator.
pub fn delete(src: &str, line: usize) -> Option<BlockEdit> {
    let mut chunks = chunks(src);
    let current = block_chunk_for_line(&chunks, line)?;

    chunks.remove(current);
    if matches!(
        chunks.get(current).map(|chunk| chunk.kind),
        Some(ChunkKind::Blank)
    ) {
        chunks.remove(current);
    } else if current > 0
        && matches!(
            chunks.get(current - 1).map(|chunk| chunk.kind),
            Some(ChunkKind::Blank)
        )
    {
        chunks.remove(current - 1);
    }

    let text = join(&chunks);
    let line = line_of_chunk(&chunks, current.min(chunks.len().saturating_sub(1)));

    Some(BlockEdit { text, line })
}

/// The text of a block, without its trailing separator newline.
pub fn text_of(src: &str, block: &Block) -> String {
    let lines: Vec<&str> = src.lines().collect();
    let start = block.start.min(lines.len());
    let end = block.end.min(lines.len());
    lines[start..end].join("\n")
}

/// A short one-line label for the block, for palettes and outlines.
pub fn label(block: &Block, src: &str) -> String {
    let text = text_of(src, block);
    let first = text.lines().next().unwrap_or("").trim();
    let cleaned = first.trim_start_matches(['#', '>', '-', '*', '+']).trim();
    let mut label: String = cleaned.chars().take(60).collect();
    if cleaned.chars().count() > 60 {
        label.push('…');
    }
    if label.is_empty() {
        block.kind.name().to_string()
    } else {
        label
    }
}

fn block_chunk_for_line(chunks: &[Chunk], line: usize) -> Option<usize> {
    let mut at = 0usize;
    let mut fallback: Option<usize> = None;

    for (index, chunk) in chunks.iter().enumerate() {
        let count = chunk.text.matches('\n').count();
        let end = at + count + usize::from(!chunk.text.ends_with('\n'));

        if chunk.is_block() {
            if line >= at && line < end {
                return Some(index);
            }
            if at <= line {
                fallback = Some(index);
            }
        }
        at = end;
    }

    // The caret is on a blank line or past the end: use the nearest block above.
    fallback
}

fn line_of_chunk(chunks: &[Chunk], index: usize) -> usize {
    let mut line = 0usize;
    for chunk in chunks.iter().take(index) {
        line += chunk.text.matches('\n').count();
        if !chunk.text.ends_with('\n') {
            line += 1;
        }
    }
    line
}

fn join(chunks: &[Chunk]) -> String {
    let mut out = String::new();
    for (index, chunk) in chunks.iter().enumerate() {
        out.push_str(&chunk.text);
        let last = index + 1 == chunks.len();
        if !last && !chunk.text.is_empty() && !chunk.text.ends_with('\n') {
            out.push('\n');
        }
    }
    out
}

fn chunks(src: &str) -> Vec<Chunk> {
    let lines: Vec<&str> = src.split_inclusive('\n').collect();
    if lines.is_empty() {
        return Vec::new();
    }

    let mut chunks = Vec::new();
    let mut i = 0usize;

    // Frontmatter is a fixed chunk: never moved, never edited by block commands.
    if let Some(end) = frontmatter_end(&lines) {
        chunks.push(Chunk {
            kind: ChunkKind::Frontmatter,
            text: lines[..end].concat(),
        });
        i = end;
    }

    while i < lines.len() {
        let line = content(lines[i]);
        let trimmed = line.trim();

        if trimmed.is_empty() {
            chunks.push(Chunk {
                kind: ChunkKind::Blank,
                text: lines[i].to_string(),
            });
            i += 1;
            continue;
        }

        if is_fence(trimmed) {
            let start = i;
            i += 1;
            while i < lines.len() && !is_fence(content(lines[i]).trim()) {
                i += 1;
            }
            if i < lines.len() {
                i += 1;
            }
            chunks.push(Chunk {
                kind: ChunkKind::Block(BlockKind::Code),
                text: lines[start..i].concat(),
            });
            continue;
        }

        if trimmed == "$$" {
            let start = i;
            i += 1;
            while i < lines.len() && content(lines[i]).trim() != "$$" {
                i += 1;
            }
            if i < lines.len() {
                i += 1;
            }
            chunks.push(Chunk {
                kind: ChunkKind::Block(BlockKind::Math),
                text: lines[start..i].concat(),
            });
            continue;
        }

        if is_single_line_math(trimmed) {
            chunks.push(Chunk {
                kind: ChunkKind::Block(BlockKind::Math),
                text: lines[i].to_string(),
            });
            i += 1;
            continue;
        }

        if heading_level(trimmed).is_some() {
            chunks.push(Chunk {
                kind: ChunkKind::Block(BlockKind::Heading),
                text: lines[i].to_string(),
            });
            i += 1;
            continue;
        }

        if is_rule(trimmed) {
            chunks.push(Chunk {
                kind: ChunkKind::Block(BlockKind::Rule),
                text: lines[i].to_string(),
            });
            i += 1;
            continue;
        }

        if trimmed.starts_with('>') {
            let start = i;
            let mut callout = false;
            while i < lines.len() && content(lines[i]).trim_start().starts_with('>') {
                let inner = content(lines[i])
                    .trim_start()
                    .trim_start_matches('>')
                    .trim_start();
                callout |= inner.starts_with("[!");
                i += 1;
            }
            chunks.push(Chunk {
                kind: ChunkKind::Block(if callout {
                    BlockKind::Callout
                } else {
                    BlockKind::Quote
                }),
                text: lines[start..i].concat(),
            });
            continue;
        }

        if is_list_item(line) {
            let start = i;
            i += 1;
            while i < lines.len() {
                let candidate = content(lines[i]);
                if candidate.trim().is_empty() {
                    break;
                }
                let indented = candidate.starts_with(' ') || candidate.starts_with('\t');
                if is_list_item(candidate) || indented {
                    i += 1;
                } else {
                    break;
                }
            }
            chunks.push(Chunk {
                kind: ChunkKind::Block(BlockKind::List),
                text: lines[start..i].concat(),
            });
            continue;
        }

        // A paragraph runs until a blank line or the start of another block construct.
        let start = i;
        i += 1;
        while i < lines.len() {
            let candidate = content(lines[i]);
            if candidate.trim().is_empty() || starts_block(candidate) {
                break;
            }
            i += 1;
        }
        let text = lines[start..i].concat();
        let kind = if i == start + 1 && is_embed(text.trim()) {
            BlockKind::Embed
        } else {
            BlockKind::Paragraph
        };
        chunks.push(Chunk {
            kind: ChunkKind::Block(kind),
            text,
        });
    }

    chunks
}

/// Number of lines (including the closing fence) consumed by leading frontmatter.
fn frontmatter_end(lines: &[&str]) -> Option<usize> {
    if lines.is_empty() {
        return None;
    }
    let first = content(lines[0]).trim_start_matches('\u{feff}').trim_end();
    if first != "---" {
        return None;
    }
    for (index, line) in lines.iter().enumerate().skip(1) {
        let trimmed = content(line).trim_end();
        if trimmed == "---" || trimmed == "..." {
            return Some(index + 1);
        }
    }
    None
}

fn content(line: &str) -> &str {
    line.trim_end_matches('\n').trim_end_matches('\r')
}

fn starts_block(line: &str) -> bool {
    let trimmed = line.trim();
    is_fence(trimmed)
        || trimmed == "$$"
        || is_single_line_math(trimmed)
        || heading_level(trimmed).is_some()
        || is_rule(trimmed)
        || trimmed.starts_with('>')
        || is_list_item(line)
}

fn is_fence(trimmed: &str) -> bool {
    trimmed.starts_with("```") || trimmed.starts_with("~~~")
}

fn is_single_line_math(trimmed: &str) -> bool {
    trimmed.len() > 4 && trimmed.starts_with("$$") && trimmed.ends_with("$$")
}

fn heading_level(trimmed: &str) -> Option<usize> {
    let level = trimmed.chars().take_while(|c| *c == '#').count();
    if level == 0 || level > 6 {
        return None;
    }
    let rest = &trimmed[level..];
    if rest.is_empty() || rest.starts_with(' ') {
        Some(level)
    } else {
        None
    }
}

fn is_rule(trimmed: &str) -> bool {
    let compact: String = trimmed.chars().filter(|c| !c.is_whitespace()).collect();
    compact.len() >= 3
        && (compact.chars().all(|c| c == '-')
            || compact.chars().all(|c| c == '*')
            || compact.chars().all(|c| c == '_'))
}

fn is_list_item(line: &str) -> bool {
    let trimmed = line.trim_start();
    if ["- ", "* ", "+ "]
        .iter()
        .any(|prefix| trimmed.starts_with(prefix))
    {
        return true;
    }

    let digits = trimmed.chars().take_while(|c| c.is_ascii_digit()).count();
    digits > 0 && trimmed[digits..].starts_with(". ")
}

fn is_embed(trimmed: &str) -> bool {
    (trimmed.starts_with("![[") && trimmed.ends_with("]]"))
        || (trimmed.starts_with("![") && trimmed.ends_with(')')) && trimmed.contains("](")
}

/// Frontmatter detection is shared with the rest of the crate; kept here so this module's
/// rule is explicit: a note's leading `---` block is never treated as movable content.
pub fn has_frontmatter(src: &str) -> bool {
    frontmatter::split(src).frontmatter.is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_headings_paragraphs_lists_and_code() {
        let src = "# T\n\npara one\npara two\n\n- a\n- b\n\n```rust\nfn x() {}\n```\n";
        let blocks = parse(src);
        let kinds: Vec<BlockKind> = blocks.iter().map(|block| block.kind).collect();
        assert_eq!(
            kinds,
            vec![
                BlockKind::Heading,
                BlockKind::Paragraph,
                BlockKind::List,
                BlockKind::Code
            ]
        );
        assert_eq!(blocks[0].start, 0);
        assert_eq!(blocks[1].start, 2);
        assert_eq!(blocks[1].end, 4);
        assert_eq!(blocks[3].start, 8);
    }

    #[test]
    fn frontmatter_is_not_a_movable_block() {
        let src = "---\ntitle: T\n---\n# Body\n";
        let blocks = parse(src);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].kind, BlockKind::Heading);
        assert_eq!(blocks[0].start, 3);
        assert!(has_frontmatter(src));
    }

    #[test]
    fn moves_a_paragraph_up_preserving_blank_separator() {
        let src = "# T\n\nfirst\n\nsecond\n";
        let edited = move_up(src, 4).expect("block moves");
        assert_eq!(edited.text, "# T\n\nsecond\n\nfirst\n");
        assert_eq!(edited.line, 2);
    }

    #[test]
    fn moves_a_paragraph_down_preserving_blank_separator() {
        let src = "# T\n\nfirst\n\nsecond\n";
        let edited = move_down(src, 2).expect("block moves");
        assert_eq!(edited.text, "# T\n\nsecond\n\nfirst\n");
        assert_eq!(edited.line, 4);
    }

    #[test]
    fn first_block_cannot_move_up_and_last_cannot_move_down() {
        let src = "# T\n\ntext\n";
        assert!(move_up(src, 0).is_none());
        assert!(move_down(src, 2).is_none());
    }

    #[test]
    fn a_caret_on_a_blank_line_uses_the_block_above() {
        let src = "# T\n\nbody\n";
        let block = block_at(src, 1).unwrap();
        assert_eq!(block.kind, BlockKind::Heading);
    }

    #[test]
    fn duplicate_inserts_the_block_beneath_it() {
        let src = "# T\n\nbody\n";
        let edited = duplicate(src, 2).unwrap();
        assert_eq!(edited.text, "# T\n\nbody\nbody\n");
        assert_eq!(edited.line, 3);
    }

    #[test]
    fn delete_removes_the_block_and_one_separator() {
        let src = "# T\n\nbody\n\ntail\n";
        let edited = delete(src, 2).unwrap();
        assert_eq!(edited.text, "# T\n\ntail\n");
        assert_eq!(edited.line, 2);
    }

    #[test]
    fn code_and_math_fences_are_single_blocks() {
        let src = "```\n# not a heading\n\nstill code\n```\n\n$$\na=b\n$$\n";
        let blocks = parse(src);
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].kind, BlockKind::Code);
        assert_eq!(blocks[1].kind, BlockKind::Math);
    }

    #[test]
    fn list_continuation_lines_stay_with_their_item() {
        let src = "- one\n  continued\n- two\n\nafter\n";
        let blocks = parse(src);
        assert_eq!(blocks[0].kind, BlockKind::List);
        assert_eq!(blocks[0].lines(), 3);
    }

    #[test]
    fn moving_without_a_trailing_newline_stays_separated() {
        let src = "a\n\nb";
        let edited = move_up(src, 2).unwrap();
        assert_eq!(edited.text, "b\n\na\n");
    }

    #[test]
    fn labels_are_short_and_human() {
        let src = "## A heading that is actually quite long and should be shortened for menus\n";
        let block = &parse(src)[0];
        let label = label(block, src);
        assert!(label.starts_with("A heading"));
        assert!(label.chars().count() <= 61);
    }
}
