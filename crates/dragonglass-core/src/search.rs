//! Note lookup: fuzzy quick-switcher matching and full-text search.
//!
//! Full-text search scans on demand rather than maintaining an index. That keeps the app
//! at ~0% CPU when idle and avoids a rebuild step; an index can come later if scanning
//! proves too slow on very large vaults.

use crate::{Note, NoteRef};

/// A fuzzy match score. Higher is better; `None` means the query did not match.
pub fn fuzzy_score(query: &str, target: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }

    let query: Vec<char> = query.to_lowercase().chars().collect();
    let target: Vec<char> = target.to_lowercase().chars().collect();

    let mut score = 0;
    let mut last_match: Option<usize> = None;
    let mut search_from = 0usize;

    for &qc in &query {
        // Prefer the next occurrence that starts a word; fall back to the first occurrence.
        let mut word_start = None;
        let mut first = None;
        for ti in search_from..target.len() {
            if target[ti] == qc {
                if first.is_none() {
                    first = Some(ti);
                }
                if is_word_start(&target, ti) {
                    word_start = Some(ti);
                    break;
                }
            }
        }

        let ti = word_start.or(first)?;

        score += 1;
        // Contiguity dominates: an adjacent run beats scattering across word starts.
        if last_match == Some(ti.wrapping_sub(1)) {
            score += 8;
        }
        if is_word_start(&target, ti) {
            score += 5;
        }
        last_match = Some(ti);
        search_from = ti + 1;
    }

    // Prefer shorter paths when scores are otherwise equal.
    Some(score - (target.len() as i32) / 8)
}

fn is_word_start(target: &[char], index: usize) -> bool {
    index == 0
        || matches!(
            target.get(index - 1),
            Some('/') | Some(' ') | Some('-') | Some('_') | Some('.')
        )
}

/// Notes whose path fuzzy-matches `query`, best first.
pub fn fuzzy_notes(notes: &[NoteRef], query: &str, limit: usize) -> Vec<NoteRef> {
    let mut scored: Vec<(i32, &NoteRef)> = notes
        .iter()
        .filter_map(|note| fuzzy_score(query, note.path_no_ext()).map(|score| (score, note)))
        .collect();

    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.relative.cmp(&b.1.relative)));
    scored.truncate(limit);
    scored.into_iter().map(|(_, note)| note.clone()).collect()
}

/// A single full-text hit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextMatch {
    pub relative: String,
    /// 1-based line number.
    pub line_number: usize,
    pub line: String,
}

/// Case-insensitive substring search across all notes, capped at `limit` hits.
pub fn full_text(notes: &[NoteRef], query: &str, limit: usize) -> Vec<TextMatch> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return Vec::new();
    }

    let mut matches = Vec::new();
    for note in notes {
        let Ok(loaded) = Note::load(&note.path) else {
            continue;
        };
        for (index, line) in loaded.source.lines().enumerate() {
            if line.to_lowercase().contains(&needle) {
                matches.push(TextMatch {
                    relative: note.relative.clone(),
                    line_number: index + 1,
                    line: truncate(line.trim(), 120),
                });
                if matches.len() >= limit {
                    return matches;
                }
            }
        }
    }

    matches
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max).collect();
    out.push('\u{2026}');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn refs(relatives: &[&str]) -> Vec<NoteRef> {
        relatives
            .iter()
            .map(|r| NoteRef {
                path: PathBuf::from(r),
                relative: (*r).to_string(),
            })
            .collect()
    }

    #[test]
    fn fuzzy_prefers_contiguous_and_word_starts() {
        let a = fuzzy_score("sw", "notes/switch-notes").unwrap();
        let b = fuzzy_score("sw", "notes/slow-write").unwrap();
        assert!(a > b, "adjacent word-start should outrank scattered");
    }

    #[test]
    fn fuzzy_is_case_insensitive() {
        assert!(fuzzy_score("index", "Notes/INDEX").is_some());
    }

    #[test]
    fn fuzzy_rejects_missing_characters() {
        assert!(fuzzy_score("zzz", "notes/index").is_none());
    }

    #[test]
    fn fuzzy_notes_orders_by_score_then_path() {
        let notes = refs(&["a/Index.md", "Index.md", "Other.md"]);
        let hits = fuzzy_notes(&notes, "index", 10);
        assert_eq!(hits.len(), 2);
        // "Other" does not match; shortest path wins on the tie-break.
        assert_eq!(hits[0].relative, "Index.md");
    }

    #[test]
    fn full_text_finds_lines_across_notes() {
        let dir = std::env::temp_dir().join(format!(
            "dragon-glass-search-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        let a = dir.join("a.md");
        let b = dir.join("b.md");
        std::fs::write(&a, "hello world\nsecond line\n").unwrap();
        std::fs::write(&b, "no match here\nWorld again\n").unwrap();

        let notes = vec![
            NoteRef {
                path: a,
                relative: "a.md".into(),
            },
            NoteRef {
                path: b,
                relative: "b.md".into(),
            },
        ];

        let hits = full_text(&notes, "world", 10);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].relative, "a.md");
        assert_eq!(hits[0].line_number, 1);
        assert_eq!(hits[0].line, "hello world");
        assert_eq!(hits[1].relative, "b.md");
        assert_eq!(hits[1].line_number, 2);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn full_text_empty_query_is_empty() {
        assert!(full_text(&refs(&["a.md"]), "   ", 10).is_empty());
    }
}
