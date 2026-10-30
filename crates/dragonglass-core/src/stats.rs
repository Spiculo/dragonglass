//! Document statistics for the status bar: words, sentences and reading time.

use crate::frontmatter;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stats {
    pub words: usize,
    pub characters: usize,
    pub sentences: usize,
    pub paragraphs: usize,
    pub reading_minutes: usize,
}

/// Count the body of a note. Frontmatter is metadata, not prose, so it is excluded.
pub fn compute(src: &str) -> Stats {
    let body = frontmatter::split(src).body;
    let text = crate::markdown::strip_comments(body);

    let words: Vec<&str> = text.split_whitespace().collect();
    let characters = words.iter().map(|word| word.chars().count()).sum();
    let sentences = count_sentences(&text);

    let mut paragraphs = 0usize;
    let mut in_paragraph = false;
    let mut in_fence = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
            in_paragraph = false;
            continue;
        }
        if in_fence {
            continue;
        }
        if trimmed.is_empty() || trimmed.starts_with('#') {
            in_paragraph = false;
            continue;
        }
        if !in_paragraph {
            paragraphs += 1;
            in_paragraph = true;
        }
    }

    let reading_minutes = if words.is_empty() {
        0
    } else {
        words.len().div_ceil(200)
    };

    Stats {
        words: words.len(),
        characters,
        sentences,
        paragraphs,
        reading_minutes,
    }
}

fn count_sentences(text: &str) -> usize {
    let mut count = 0usize;
    let mut has_content = false;

    for ch in text.chars() {
        if ch.is_alphanumeric() {
            has_content = true;
        }
        if matches!(ch, '.' | '!' | '?') && has_content {
            count += 1;
            has_content = false;
        }
    }
    if has_content {
        count += 1;
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_words_and_reading_time() {
        let src = format!("# T\n\n{}\n", "word ".repeat(450));
        let stats = compute(&src);
        assert_eq!(stats.words, 452);
        assert_eq!(stats.reading_minutes, 3);
        assert_eq!(stats.paragraphs, 1);
    }

    #[test]
    fn frontmatter_is_not_prose() {
        let stats = compute("---\ntitle: not counted\n---\nOne two three.\n");
        assert_eq!(stats.words, 3);
    }

    #[test]
    fn counts_sentences() {
        let stats = compute("One. Two! Three?\nNo terminator");
        assert_eq!(stats.sentences, 4);
    }

    #[test]
    fn empty_documents_have_zero_minutes() {
        let stats = compute("");
        assert_eq!(stats.words, 0);
        assert_eq!(stats.reading_minutes, 0);
        assert_eq!(stats.paragraphs, 0);
    }

    #[test]
    fn comments_are_ignored() {
        let stats = compute("kept %% hidden words %%\n");
        assert_eq!(stats.words, 1);
    }
}
