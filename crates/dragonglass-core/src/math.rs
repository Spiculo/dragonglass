//! Math typesetting: a best-effort TeX-to-Unicode conversion.
//!
//! KaTeX is a JavaScript library with no native equivalent, so rather than pretend to
//! typeset TeX we transliterate it: commands become their Unicode symbols, sub/superscripts
//! become Unicode scripts where they exist, and `\frac`/`\sqrt` become readable forms. It is
//! not LaTeX, but `$x^2$` reads as `x²` in the app and in exported HTML.

use crate::markdown;

/// A styled run of a paragraph: plain text, a `==highlight==`, or inline math.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Span {
    Text(String),
    Highlight(String),
    Math(String),
}

/// Split text into spans of text, highlights and math (all delimiters removed).
pub fn spans(text: &str) -> Vec<Span> {
    let mut out = Vec::new();

    for (segment, is_math) in split_math(text) {
        if is_math {
            let rendered = to_unicode(&segment);
            if !rendered.is_empty() {
                out.push(Span::Math(rendered));
            }
            continue;
        }

        for (piece, is_highlight) in markdown::split_highlights(&segment) {
            if piece.is_empty() {
                continue;
            }
            out.push(if is_highlight {
                Span::Highlight(piece)
            } else {
                Span::Text(piece)
            });
        }
    }

    out
}

/// Split text into `(segment, is_math)` runs around `$…$` and `$$…$$`.
///
/// A `\$` is a literal dollar sign. Unterminated math is treated as plain text.
pub fn split_math(text: &str) -> Vec<(String, bool)> {
    let chars: Vec<char> = text.chars().collect();
    let mut out: Vec<(String, bool)> = Vec::new();
    let mut buffer = String::new();
    let mut in_math = false;
    let mut display = false;
    let mut i = 0;

    while i < chars.len() {
        if chars[i] == '\\' && chars.get(i + 1) == Some(&'$') {
            buffer.push('$');
            i += 2;
            continue;
        }

        if chars[i] == '$' {
            let double = chars.get(i + 1) == Some(&'$');
            let width = if double { 2 } else { 1 };

            if !in_math {
                if !buffer.is_empty() {
                    out.push((std::mem::take(&mut buffer), false));
                }
                in_math = true;
                display = double;
                i += width;
            } else if display == double {
                out.push((std::mem::take(&mut buffer), true));
                in_math = false;
                display = false;
                i += width;
            } else {
                // A mismatched delimiter is content.
                for _ in 0..width {
                    buffer.push('$');
                }
                i += width;
            }
            continue;
        }

        buffer.push(chars[i]);
        i += 1;
    }

    // Unterminated math is plain text, delimiter included.
    if in_math {
        let delimiter = if display { "$$" } else { "$" };
        buffer.insert_str(0, delimiter);
    }
    if !buffer.is_empty() {
        out.push((buffer, false));
    }

    // Merge adjacent runs of the same kind.
    let mut merged: Vec<(String, bool)> = Vec::new();
    for (segment, is_math) in out {
        match merged.last_mut() {
            Some((existing, kind)) if *kind == is_math => existing.push_str(&segment),
            _ => merged.push((segment, is_math)),
        }
    }

    merged
}

/// Convert a TeX fragment to readable Unicode.
pub fn to_unicode(tex: &str) -> String {
    let chars: Vec<char> = tex.chars().collect();
    let mut out = String::new();
    let mut i = 0;

    while i < chars.len() {
        match chars[i] {
            '\\' => {
                let start = i + 1;
                let mut end = start;
                while end < chars.len() && chars[end].is_ascii_alphabetic() {
                    end += 1;
                }

                if end == start {
                    // Punctuation command: `\,` `\;` `\!` `\{` `\}` …
                    match chars.get(start) {
                        Some(',') | Some(';') | Some(':') | Some(' ') => out.push(' '),
                        Some('{') => out.push('{'),
                        Some('}') => out.push('}'),
                        Some('!') | None | Some(_) => {}
                    }
                    i = start + 1;
                    continue;
                }

                let name: String = chars[start..end].iter().collect();
                i = end;

                match name.as_str() {
                    "frac" => {
                        let (numerator, next) = group(&chars, i);
                        let (denominator, after) = group(&chars, next);
                        out.push_str(&parenthesize(&to_unicode(&numerator)));
                        out.push('/');
                        out.push_str(&parenthesize(&to_unicode(&denominator)));
                        i = after;
                    }
                    "sqrt" => {
                        let (argument, next) = group(&chars, i);
                        out.push('√');
                        out.push_str(&parenthesize(&to_unicode(&argument)));
                        i = next;
                    }
                    "text" | "mathrm" | "mathbf" | "mathit" | "operatorname" => {
                        let (argument, next) = group(&chars, i);
                        out.push_str(&argument);
                        i = next;
                    }
                    "mathbb" => {
                        let (argument, next) = group(&chars, i);
                        for ch in argument.chars() {
                            out.push(double_struck(ch).unwrap_or(ch));
                        }
                        i = next;
                    }
                    "left" | "right" => {}
                    other => match symbol(other) {
                        Some(sym) => out.push_str(sym),
                        None => out.push_str(other),
                    },
                }
            }
            '^' | '_' => {
                let superscript = chars[i] == '^';
                let (fragment, next) = group(&chars, i + 1);
                out.push_str(&script(&to_unicode(&fragment), superscript));
                i = next;
            }
            '{' | '}' => i += 1,
            '~' => {
                out.push(' ');
                i += 1;
            }
            other => {
                out.push(other);
                i += 1;
            }
        }
    }

    out
}

/// Read a `{…}` group, or a single character when there is no brace.
fn group(chars: &[char], start: usize) -> (String, usize) {
    if start >= chars.len() {
        return (String::new(), start);
    }

    if chars[start] != '{' {
        return (chars[start].to_string(), start + 1);
    }

    let mut depth = 0;
    let mut end = start;
    while end < chars.len() {
        match chars[end] {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return (chars[start + 1..end].iter().collect(), end + 1);
                }
            }
            _ => {}
        }
        end += 1;
    }

    (chars[start + 1..].iter().collect(), chars.len())
}

/// Wrap multi-character fragments so `\frac{a+b}{2}` reads as `(a+b)/2`.
fn parenthesize(value: &str) -> String {
    if value.chars().count() > 1 {
        format!("({value})")
    } else {
        value.to_string()
    }
}

fn script(value: &str, superscript: bool) -> String {
    let mapped: Option<String> = value
        .chars()
        .map(|ch| {
            if superscript {
                superscript_char(ch)
            } else {
                subscript_char(ch)
            }
        })
        .collect();

    match mapped {
        Some(mapped) => mapped,
        None if value.chars().count() == 1 => {
            if superscript {
                format!("^{value}")
            } else {
                format!("_{value}")
            }
        }
        None => {
            if superscript {
                format!("^({value})")
            } else {
                format!("_({value})")
            }
        }
    }
}

fn superscript_char(ch: char) -> Option<char> {
    Some(match ch {
        '0' => '⁰',
        '1' => '¹',
        '2' => '²',
        '3' => '³',
        '4' => '⁴',
        '5' => '⁵',
        '6' => '⁶',
        '7' => '⁷',
        '8' => '⁸',
        '9' => '⁹',
        '+' => '⁺',
        '-' => '⁻',
        '=' => '⁼',
        '(' => '⁽',
        ')' => '⁾',
        'n' => 'ⁿ',
        'i' => 'ⁱ',
        _ => return None,
    })
}

fn subscript_char(ch: char) -> Option<char> {
    Some(match ch {
        '0' => '₀',
        '1' => '₁',
        '2' => '₂',
        '3' => '₃',
        '4' => '₄',
        '5' => '₅',
        '6' => '₆',
        '7' => '₇',
        '8' => '₈',
        '9' => '₉',
        '+' => '₊',
        '-' => '₋',
        '=' => '₌',
        '(' => '₍',
        ')' => '₎',
        'a' => 'ₐ',
        'e' => 'ₑ',
        'h' => 'ₕ',
        'i' => 'ᵢ',
        'j' => 'ⱼ',
        'k' => 'ₖ',
        'l' => 'ₗ',
        'm' => 'ₘ',
        'n' => 'ₙ',
        'o' => 'ₒ',
        'p' => 'ₚ',
        'r' => 'ᵣ',
        's' => 'ₛ',
        't' => 'ₜ',
        'u' => 'ᵤ',
        'v' => 'ᵥ',
        'x' => 'ₓ',
        _ => return None,
    })
}

fn double_struck(ch: char) -> Option<char> {
    Some(match ch {
        'R' => 'ℝ',
        'N' => 'ℕ',
        'Z' => 'ℤ',
        'Q' => 'ℚ',
        'C' => 'ℂ',
        _ => return None,
    })
}

fn symbol(name: &str) -> Option<&'static str> {
    Some(match name {
        "alpha" => "α",
        "beta" => "β",
        "gamma" => "γ",
        "delta" => "δ",
        "epsilon" => "ε",
        "varepsilon" => "ε",
        "zeta" => "ζ",
        "eta" => "η",
        "theta" => "θ",
        "iota" => "ι",
        "kappa" => "κ",
        "lambda" => "λ",
        "mu" => "μ",
        "nu" => "ν",
        "xi" => "ξ",
        "pi" => "π",
        "rho" => "ρ",
        "sigma" => "σ",
        "tau" => "τ",
        "upsilon" => "υ",
        "phi" => "φ",
        "chi" => "χ",
        "psi" => "ψ",
        "omega" => "ω",
        "Gamma" => "Γ",
        "Delta" => "Δ",
        "Theta" => "Θ",
        "Lambda" => "Λ",
        "Xi" => "Ξ",
        "Pi" => "Π",
        "Sigma" => "Σ",
        "Phi" => "Φ",
        "Psi" => "Ψ",
        "Omega" => "Ω",
        "times" => "×",
        "cdot" => "·",
        "div" => "÷",
        "pm" => "±",
        "mp" => "∓",
        "le" | "leq" => "≤",
        "ge" | "geq" => "≥",
        "ne" | "neq" => "≠",
        "approx" => "≈",
        "equiv" => "≡",
        "propto" => "∝",
        "infty" => "∞",
        "partial" => "∂",
        "nabla" => "∇",
        "sum" => "∑",
        "prod" => "∏",
        "int" => "∫",
        "to" | "rightarrow" => "→",
        "leftarrow" => "←",
        "Rightarrow" => "⇒",
        "leftrightarrow" => "↔",
        "forall" => "∀",
        "exists" => "∃",
        "in" => "∈",
        "notin" => "∉",
        "subset" => "⊂",
        "supset" => "⊃",
        "cup" => "∪",
        "cap" => "∩",
        "emptyset" => "∅",
        "angle" => "∠",
        "degree" => "°",
        "ldots" | "dots" => "…",
        "quad" | "qquad" => "  ",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_scripts() {
        assert_eq!(to_unicode("x^2"), "x²");
        assert_eq!(to_unicode("x^{10}"), "x¹⁰");
        assert_eq!(to_unicode("H_2O"), "H₂O");
        assert_eq!(to_unicode("a_n"), "aₙ");
    }

    #[test]
    fn converts_common_commands() {
        assert_eq!(to_unicode("\\alpha + \\beta"), "α + β");
        assert_eq!(to_unicode("a \\le b \\times c"), "a ≤ b × c");
        assert_eq!(to_unicode("\\mathbb{R}"), "ℝ");
        assert_eq!(to_unicode("\\Omega"), "Ω");
        assert_eq!(to_unicode("\\unknown"), "unknown");
    }

    #[test]
    fn converts_fractions_and_roots() {
        assert_eq!(to_unicode("\\frac{a}{b}"), "a/b");
        assert_eq!(to_unicode("\\frac{a+b}{2}"), "(a+b)/2");
        assert_eq!(to_unicode("\\sqrt{2}"), "√2");
        assert_eq!(to_unicode("\\sqrt{x+1}"), "√(x+1)");
        assert_eq!(to_unicode("\\sqrt{x}"), "√x");
    }

    #[test]
    fn splits_inline_and_display_math() {
        assert_eq!(
            split_math("a $x+y$ b"),
            vec![
                ("a ".to_string(), false),
                ("x+y".to_string(), true),
                (" b".to_string(), false),
            ]
        );
        assert_eq!(split_math("$$x^2$$"), vec![("x^2".to_string(), true)]);
        assert_eq!(
            split_math("cost \\$5"),
            vec![("cost $5".to_string(), false)]
        );
    }

    #[test]
    fn unterminated_math_is_plain_text() {
        assert_eq!(
            split_math("broken $x^2"),
            vec![("broken $x^2".to_string(), false)]
        );
    }

    #[test]
    fn builds_spans_for_text_highlight_and_math() {
        assert_eq!(
            spans("a ==b== $x^2$ c"),
            vec![
                Span::Text("a ".to_string()),
                Span::Highlight("b".to_string()),
                Span::Text(" ".to_string()),
                Span::Math("x²".to_string()),
                Span::Text(" c".to_string()),
            ]
        );
    }
}
