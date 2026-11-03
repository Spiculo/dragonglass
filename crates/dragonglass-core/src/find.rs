//! In-note search and replace.
//!
//! The editor counts positions in **characters**, while the regex engine works in UTF-8
//! bytes. This module is the single place that converts between the two, so a match that
//! spans an emoji or an accented letter never splits a character.
//!
//! Plain-text mode escapes the pattern and treats the replacement literally. Regex mode
//! passes the pattern through and supports the regex crate's `$1` / `${name}` capture
//! expansion in the replacement (`$$` inserts a literal `$`).

use std::ops::Range;

use regex::{Regex, RegexBuilder};

/// How a pattern is interpreted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Options {
    /// Treat the pattern as a regular expression instead of literal text.
    pub regex: bool,
    /// Match case exactly. When false, matching is case-insensitive.
    pub case_sensitive: bool,
    /// Only match whole words (`\b` on both sides).
    pub whole_word: bool,
}

/// One match, as a character range into the searched text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Match {
    pub range: Range<usize>,
}

/// Build the regex used for a query. Returns the user-facing error message on failure.
fn compile(pattern: &str, options: Options) -> Result<Regex, String> {
    let body = if options.regex {
        pattern.to_string()
    } else {
        regex::escape(pattern)
    };
    let body = if options.whole_word {
        format!(r"\b(?:{body})\b")
    } else {
        body
    };

    RegexBuilder::new(&body)
        .case_insensitive(!options.case_sensitive)
        .build()
        .map_err(|error| {
            // The full regex error is multi-line and includes a caret diagram; the first
            // line is the useful part in the status bar.
            error
                .to_string()
                .lines()
                .next()
                .unwrap_or("invalid regular expression")
                .to_string()
        })
}

/// Convert a byte offset known to be on a character boundary into a character index.
fn char_index(text: &str, byte: usize) -> usize {
    if text.is_ascii() {
        byte
    } else {
        text[..byte].chars().count()
    }
}

/// All non-overlapping matches of `pattern` in `text`, as character ranges.
pub fn find_all(text: &str, pattern: &str, options: Options) -> Result<Vec<Match>, String> {
    if pattern.is_empty() {
        return Ok(Vec::new());
    }

    let regex = compile(pattern, options)?;
    Ok(regex
        .find_iter(text)
        .map(|found| Match {
            range: char_index(text, found.start())..char_index(text, found.end()),
        })
        .collect())
}

/// Replace every match, returning the new text and the number of replacements.
pub fn replace_all(
    text: &str,
    pattern: &str,
    replacement: &str,
    options: Options,
) -> Result<(String, usize), String> {
    if pattern.is_empty() {
        return Ok((text.to_string(), 0));
    }

    let regex = compile(pattern, options)?;
    let count = regex.find_iter(text).count();
    if count == 0 {
        return Ok((text.to_string(), 0));
    }
    let replaced = if options.regex {
        regex.replace_all(text, replacement).into_owned()
    } else {
        regex
            .replace_all(text, regex::NoExpand(replacement))
            .into_owned()
    };
    Ok((replaced, count))
}

/// The expanded replacement for one match starting at `target.start`.
///
/// Returns `None` when the pattern no longer matches there (for example after the buffer
/// changed), and an error when the pattern itself is invalid.
pub fn replacement_for(
    text: &str,
    target: &Match,
    pattern: &str,
    replacement: &str,
    options: Options,
) -> Result<Option<String>, String> {
    if pattern.is_empty() {
        return Ok(None);
    }

    let regex = compile(pattern, options)?;
    for captures in regex.captures_iter(text) {
        let Some(found) = captures.get(0) else {
            continue;
        };
        let start = char_index(text, found.start());
        if start != target.range.start {
            continue;
        }
        let end = char_index(text, found.end());
        if end != target.range.end {
            continue;
        }

        if !options.regex {
            return Ok(Some(replacement.to_string()));
        }
        let mut expanded = String::new();
        captures.expand(replacement, &mut expanded);
        return Ok(Some(expanded));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ranges(matches: &[Match]) -> Vec<Range<usize>> {
        matches.iter().map(|m| m.range.clone()).collect()
    }

    #[test]
    fn plain_text_is_literal_and_case_insensitive() {
        let found = find_all("One one ONE", "one", Options::default()).unwrap();
        assert_eq!(ranges(&found), vec![0..3, 4..7, 8..11]);
    }

    #[test]
    fn case_sensitive_matches_exactly() {
        let found = find_all(
            "One one ONE",
            "one",
            Options {
                case_sensitive: true,
                ..Options::default()
            },
        )
        .unwrap();
        assert_eq!(ranges(&found), vec![4..7]);
    }

    #[test]
    fn whole_word_does_not_match_inside_words() {
        let found = find_all(
            "one stone one",
            "one",
            Options {
                whole_word: true,
                ..Options::default()
            },
        )
        .unwrap();
        assert_eq!(ranges(&found), vec![0..3, 10..13]);
    }

    #[test]
    fn regex_captures_and_multibyte_offsets() {
        let text = "héllo 42, then 7";
        let found = find_all(
            text,
            r"(\d+)",
            Options {
                regex: true,
                ..Options::default()
            },
        )
        .unwrap();
        assert_eq!(ranges(&found), vec![6..8, 15..16]);
        let captured: String = text.chars().skip(6).take(2).collect();
        assert_eq!(captured, "42");
    }

    #[test]
    fn replacement_expands_captures() {
        let (out, count) = replace_all(
            "name: Ada; born 1815",
            r"name: (\w+); born (\d+)",
            "$1 ($2)",
            Options {
                regex: true,
                ..Options::default()
            },
        )
        .unwrap();
        assert_eq!(out, "Ada (1815)");
        assert_eq!(count, 1);
    }

    #[test]
    fn replace_one_returns_the_expanded_text() {
        let text = "a1 a22";
        let options = Options {
            regex: true,
            ..Options::default()
        };
        let found = find_all(text, r"a(\d+)", options).unwrap();
        let replaced = replacement_for(text, &found[1], r"a(\d+)", "n$1", options)
            .unwrap()
            .unwrap();
        assert_eq!(replaced, "n22");
    }

    #[test]
    fn invalid_regex_is_reported_without_panicking() {
        let error = find_all(
            "x",
            "(",
            Options {
                regex: true,
                ..Options::default()
            },
        )
        .unwrap_err();
        assert!(!error.is_empty());
    }

    #[test]
    fn empty_pattern_matches_nothing_and_replaces_nothing() {
        assert!(find_all("abc", "", Options::default()).unwrap().is_empty());
        let (out, count) = replace_all("abc", "", "x", Options::default()).unwrap();
        assert_eq!(out, "abc");
        assert_eq!(count, 0);
    }

    #[test]
    fn replacement_can_be_empty() {
        let (out, count) = replace_all("a-b-c", "-", "", Options::default()).unwrap();
        assert_eq!(out, "abc");
        assert_eq!(count, 2);
    }

    #[test]
    fn plain_replacement_treats_dollar_signs_literally() {
        let (out, count) = replace_all("cost", "cost", "$5", Options::default()).unwrap();
        assert_eq!(out, "$5");
        assert_eq!(count, 1);

        let options = Options {
            regex: true,
            ..Options::default()
        };
        let (out, _) = replace_all("cost 5", r"cost (\d+)", "$$$1", options).unwrap();
        assert_eq!(out, "$5");
    }
}
