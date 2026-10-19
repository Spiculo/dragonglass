//! Greedy word wrapping, font-agnostic: the caller supplies text widths.
//!
//! Kept in the headless engine so it can be unit-tested without a GPU or a font.

/// Split `chars` into rows no wider than `max_width`, returning `(start, end)` character
/// ranges. Breaks at the last space that fits, otherwise hard-breaks; always progresses.
///
/// `width_of(from, to)` must return the width of `chars[from..to]`.
pub fn wrap(
    chars: &[char],
    width_of: impl Fn(usize, usize) -> f32,
    max_width: f32,
) -> Vec<(usize, usize)> {
    let len = chars.len();
    if len == 0 {
        return vec![(0, 0)];
    }
    if !max_width.is_finite() || max_width <= 0.0 {
        return vec![(0, len)];
    }

    let mut rows = Vec::new();
    let mut start = 0;

    while start < len {
        // Leading spaces belong to the previous break, not to the next row.
        while start < len && chars[start] == ' ' {
            start += 1;
        }
        if start >= len {
            break;
        }

        // Largest end (exclusive) that fits.
        let mut end = start + 1;
        while end <= len && width_of(start, end) <= max_width {
            end += 1;
        }
        // If the loop ran past the end, everything remaining fits.
        let limit = if end > len {
            len
        } else {
            end.saturating_sub(1).max(start + 1)
        };

        if limit >= len {
            rows.push((start, len));
            break;
        }

        // Prefer the last space at or before the limit; a space exactly at the limit ends
        // the row there, and one before it ends the row just before the space.
        let mut break_at = limit;
        let mut candidate = limit;
        while candidate > start {
            if chars[candidate] == ' ' {
                break_at = candidate;
                break;
            }
            candidate -= 1;
        }

        rows.push((start, break_at));
        start = break_at;
    }

    if rows.is_empty() {
        rows.push((0, len));
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every character is 8 units wide, so limits are predictable.
    fn mono(text: &str) -> Vec<char> {
        text.chars().collect()
    }

    fn wrap_chars(chars: &[char], columns: f32) -> Vec<String> {
        wrap(chars, |from, to| (to - from) as f32 * 8.0, columns * 8.0)
            .into_iter()
            .map(|(a, b)| chars[a..b].iter().collect::<String>())
            .collect()
    }

    #[test]
    fn empty_line_is_one_empty_row() {
        assert_eq!(wrap(&[], |_, _| 0.0, 100.0), vec![(0, 0)]);
    }

    #[test]
    fn short_line_is_one_row() {
        assert_eq!(wrap_chars(&mono("hello"), 20.0), vec!["hello"]);
    }

    #[test]
    fn breaks_at_spaces() {
        assert_eq!(
            wrap_chars(&mono("hello world again"), 11.0),
            vec!["hello world", "again"]
        );
    }

    #[test]
    fn hard_breaks_a_long_word() {
        let rows = wrap_chars(&mono("abcdefghij"), 4.0);
        assert_eq!(rows, vec!["abcd", "efgh", "ij"]);
    }

    #[test]
    fn skips_the_space_it_broke_on() {
        assert_eq!(wrap_chars(&mono("ab cd"), 3.0), vec!["ab", "cd"]);
    }

    #[test]
    fn always_progresses() {
        let rows = wrap(&mono("word"), |_, _| 1000.0, 1.0);
        assert_eq!(rows, vec![(0, 1), (1, 2), (2, 3), (3, 4)]);
    }

    #[test]
    fn non_finite_width_keeps_one_row() {
        assert_eq!(
            wrap(&mono("hello"), |_, _| 0.0, f32::INFINITY),
            vec![(0, 5)]
        );
    }
}
