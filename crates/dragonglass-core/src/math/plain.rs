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
///
/// This is a transliteration, not a typesetter: it turns commands into the symbols they
/// stand for, keeps fractions and roots readable with parentheses, and writes sub- and
/// superscripts with Unicode scripts where the font has them. `\\` becomes a line break so
/// aligned environments can be drawn as separate lines.
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
                    // Punctuation command: `\,` `\;` `\!` `\{` `\}` `\\` …
                    match chars.get(start) {
                        Some(',') | Some(';') | Some(':') | Some(' ') => out.push(' '),
                        Some('%') => out.push('%'),
                        Some('&') => out.push('&'),
                        Some('#') => out.push('#'),
                        Some('_') => out.push('_'),
                        Some('$') => out.push('$'),
                        Some('{') => out.push('{'),
                        Some('}') => out.push('}'),
                        Some('|') => out.push('‖'),
                        Some('\\') => out.push('\n'),
                        Some('(') | Some(')') | Some('[') | Some(']') => {}
                        Some('!') | None => {}
                        Some(other) => out.push(*other),
                    }
                    i = start + 1;
                    continue;
                }

                let name: String = chars[start..end].iter().collect();
                i = end;

                match name.as_str() {
                    "frac" | "dfrac" | "tfrac" => {
                        let (numerator, next) = group(&chars, i);
                        let (denominator, after) = group(&chars, next);
                        out.push_str(&parenthesize(&to_unicode(&numerator)));
                        out.push('/');
                        out.push_str(&parenthesize(&to_unicode(&denominator)));
                        i = after;
                    }
                    "binom" => {
                        let (top, next) = group(&chars, i);
                        let (bottom, after) = group(&chars, next);
                        out.push_str(&format!(
                            "C({}, {})",
                            to_unicode(&top).trim(),
                            to_unicode(&bottom).trim()
                        ));
                        i = after;
                    }
                    "sqrt" => {
                        let (index, next) = bracket(&chars, i);
                        let (argument, after) = group(&chars, next);
                        let root = to_unicode(&argument);
                        match index.filter(|index| !index.trim().is_empty()) {
                            Some(index) => {
                                let index = to_unicode(&index);
                                out.push_str(&format!("{}({root})", script(index.trim(), true)))
                            }
                            None => {
                                out.push('√');
                                out.push_str(&parenthesize(&root));
                            }
                        }
                        i = after;
                    }
                    "text" | "textrm" | "textnormal" | "mathrm" | "mathbf" | "mathit"
                    | "mathsf" | "mathtt" | "mathcal" | "mathfrak" | "operatorname"
                    | "boldsymbol" | "bm" | "mbox" | "textbf" | "textit" | "boxed" => {
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
                    "hat" | "widehat" => {
                        let (argument, next) = group(&chars, i);
                        out.push_str(&accent(&to_unicode(&argument), '\u{0302}'));
                        i = next;
                    }
                    "bar" | "overline" => {
                        let (argument, next) = group(&chars, i);
                        out.push_str(&accent(&to_unicode(&argument), '\u{0305}'));
                        i = next;
                    }
                    "tilde" | "widetilde" => {
                        let (argument, next) = group(&chars, i);
                        out.push_str(&accent(&to_unicode(&argument), '\u{0303}'));
                        i = next;
                    }
                    "vec" => {
                        let (argument, next) = group(&chars, i);
                        out.push_str(&accent(&to_unicode(&argument), '\u{20D7}'));
                        i = next;
                    }
                    "dot" => {
                        let (argument, next) = group(&chars, i);
                        out.push_str(&accent(&to_unicode(&argument), '\u{0307}'));
                        i = next;
                    }
                    "ddot" => {
                        let (argument, next) = group(&chars, i);
                        out.push_str(&accent(&to_unicode(&argument), '\u{0308}'));
                        i = next;
                    }
                    "check" => {
                        let (argument, next) = group(&chars, i);
                        out.push_str(&accent(&to_unicode(&argument), '\u{030C}'));
                        i = next;
                    }
                    "underbrace" | "overbrace" => {
                        let (argument, next) = group(&chars, i);
                        out.push_str(&to_unicode(&argument));
                        i = next;
                    }
                    "phantom" | "hphantom" | "vphantom" => {
                        let (_, next) = group(&chars, i);
                        i = next;
                    }
                    "left" | "right" | "big" | "Big" | "bigl" | "bigr" | "Bigl" | "Bigr"
                    | "bigg" | "Bigg" | "biggl" | "biggr" | "Biggl" | "Biggr" | "middle" => {
                        // Delimiters are ordinary characters; `\left.` is an invisible one.
                        if chars.get(i) == Some(&'.') {
                            i += 1;
                        }
                    }
                    "begin" | "end" => {
                        let (environment, next) = group(&chars, i);
                        i = next;
                        // `array` takes a column specification after the environment name.
                        if environment.trim() == "array" && chars.get(i) == Some(&'{') {
                            let (_, after) = group(&chars, i);
                            i = after;
                        }
                    }
                    "color" => {
                        let (_, next) = group(&chars, i);
                        i = next;
                    }
                    "textcolor" => {
                        let (_, next) = group(&chars, i);
                        let (argument, after) = group(&chars, next);
                        out.push_str(&argument);
                        i = after;
                    }
                    "pmod" => {
                        let (argument, next) = group(&chars, i);
                        out.push_str(&format!(" (mod {})", to_unicode(&argument).trim()));
                        i = next;
                    }
                    "bmod" | "mod" => out.push_str(" mod "),
                    "not" => {
                        // `\not\in` should read ∉ rather than ¬∈.
                        if chars.get(i) == Some(&'\\') {
                            let inner_start = i + 1;
                            let mut inner_end = inner_start;
                            while inner_end < chars.len() && chars[inner_end].is_ascii_alphabetic()
                            {
                                inner_end += 1;
                            }
                            let inner: String = chars[inner_start..inner_end].iter().collect();
                            let negated = format!("not{inner}");
                            if let Some(sym) = symbol(&negated) {
                                out.push_str(sym);
                                i = inner_end;
                                continue;
                            }
                        }
                        out.push('¬');
                    }
                    "xrightarrow" | "xleftarrow" => {
                        let (_, next) = bracket(&chars, i);
                        let (argument, after) = group(&chars, next);
                        out.push(if name == "xrightarrow" { '→' } else { '←' });
                        let label = to_unicode(&argument);
                        if !label.trim().is_empty() {
                            out.push_str(&format!("({})", label.trim()));
                        }
                        i = after;
                    }
                    "limits" | "nolimits" | "displaystyle" | "textstyle" | "scriptstyle"
                    | "scriptscriptstyle" | "mathstrut" | "!" => {}
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
            '&' => {
                // Column separator in aligned/matrix environments.
                out.push_str("  ");
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

/// Read an optional `[…]` argument. Returns it, unparsed, and the index after it.
fn bracket(chars: &[char], start: usize) -> (Option<String>, usize) {
    if chars.get(start) != Some(&'[') {
        return (None, start);
    }

    let mut depth = 0;
    let mut end = start;
    while end < chars.len() {
        match chars[end] {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    return (Some(chars[start + 1..end].iter().collect()), end + 1);
                }
            }
            _ => {}
        }
        end += 1;
    }

    (None, start)
}

/// Apply a combining mark to every visible character of `value`.
fn accent(value: &str, mark: char) -> String {
    let mut out = String::with_capacity(value.len() * 2);
    for ch in value.chars() {
        out.push(ch);
        if !ch.is_whitespace() {
            out.push(mark);
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
    let (open, close) = if superscript {
        ('⁽', '⁾')
    } else {
        ('₍', '₎')
    };

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
            // No Unicode script for every character (e.g. `1/p`): keep the readable
            // characters inside script parentheses instead of an ASCII `_(...)` tail.
            let mut out = String::new();
            out.push(open);
            for ch in value.chars() {
                let mapped = if superscript {
                    superscript_char(ch)
                } else {
                    subscript_char(ch)
                };
                out.push(mapped.unwrap_or(ch));
            }
            out.push(close);
            out
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
        "vartheta" => "ϑ",
        "iota" => "ι",
        "kappa" => "κ",
        "lambda" => "λ",
        "mu" => "μ",
        "nu" => "ν",
        "xi" => "ξ",
        "pi" => "π",
        "varpi" => "ϖ",
        "rho" => "ρ",
        "varrho" => "ϱ",
        "sigma" => "σ",
        "varsigma" => "ς",
        "tau" => "τ",
        "upsilon" => "υ",
        "phi" => "φ",
        "varphi" => "ϕ",
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
        "Upsilon" => "Υ",
        "Phi" => "Φ",
        "Psi" => "Ψ",
        "Omega" => "Ω",
        "times" => "×",
        "cdot" => "·",
        "div" => "÷",
        "pm" => "±",
        "mp" => "∓",
        "ast" => "∗",
        "star" => "⋆",
        "circ" => "∘",
        "bullet" => "∙",
        "oplus" => "⊕",
        "ominus" => "⊖",
        "otimes" => "⊗",
        "oslash" => "⊘",
        "odot" => "⊙",
        "bigoplus" => "⨁",
        "bigotimes" => "⨂",
        "rtimes" => "⋊",
        "ltimes" => "⋉",
        "sqcup" => "⊔",
        "sqcap" => "⊓",
        "le" | "leq" => "≤",
        "ge" | "geq" => "≥",
        "ne" | "neq" => "≠",
        "approx" => "≈",
        "equiv" => "≡",
        "cong" => "≅",
        "sim" => "∼",
        "simeq" => "≃",
        "propto" => "∝",
        "lesssim" => "≲",
        "gtrsim" => "≳",
        "ll" => "≪",
        "gg" => "≫",
        "prec" => "≺",
        "succ" => "≻",
        "preceq" => "≼",
        "succeq" => "≽",
        "infty" => "∞",
        "partial" => "∂",
        "nabla" => "∇",
        "sum" => "∑",
        "prod" => "∏",
        "coprod" => "∐",
        "int" => "∫",
        "iint" => "∬",
        "iiint" => "∭",
        "oint" => "∮",
        "to" | "rightarrow" => "→",
        "leftarrow" | "gets" => "←",
        "leftrightarrow" => "↔",
        "Rightarrow" | "implies" => "⇒",
        "Leftarrow" => "⇐",
        "Leftrightarrow" | "iff" => "⇔",
        "longrightarrow" | "Longrightarrow" => "⟶",
        "longleftarrow" | "Longleftarrow" => "⟵",
        "Longleftrightarrow" => "⟺",
        "mapsto" => "↦",
        "uparrow" => "↑",
        "downarrow" => "↓",
        "rightharpoonup" => "⇀",
        "leftharpoonup" => "↼",
        "forall" => "∀",
        "exists" => "∃",
        "nexists" => "∄",
        "neg" | "lnot" => "¬",
        "in" => "∈",
        "notin" => "∉",
        "ni" => "∋",
        "subset" => "⊂",
        "supset" => "⊃",
        "subseteq" => "⊆",
        "supseteq" => "⊇",
        "subsetneq" => "⊊",
        "supsetneq" => "⊋",
        "cup" => "∪",
        "cap" => "∩",
        "setminus" => "∖",
        "emptyset" | "varnothing" => "∅",
        "wedge" | "land" => "∧",
        "vee" | "lor" => "∨",
        "angle" => "∠",
        "perp" => "⊥",
        "parallel" => "∥",
        "top" => "⊤",
        "bot" => "⊥",
        "therefore" => "∴",
        "because" => "∵",
        "degree" => "°",
        "prime" => "′",
        "ldots" | "dots" => "…",
        "cdots" => "⋯",
        "vdots" => "⋮",
        "ddots" => "⋱",
        "lfloor" => "⌊",
        "rfloor" => "⌋",
        "lceil" => "⌈",
        "rceil" => "⌉",
        "langle" => "⟨",
        "rangle" => "⟩",
        "lbrace" => "{",
        "rbrace" => "}",
        "lvert" => "|",
        "rvert" => "|",
        "lVert" => "‖",
        "rVert" => "‖",
        "vert" => "|",
        "Vert" => "‖",
        "aleph" => "ℵ",
        "hbar" => "ℏ",
        "ell" => "ℓ",
        "Re" => "ℜ",
        "Im" => "ℑ",
        "wp" => "℘",
        "quad" | "qquad" => "  ",
        "colon" => ":",
        "dagger" => "†",
        "ddagger" => "‡",
        "S" => "§",
        "P" => "¶",
        "copyright" => "©",
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
        assert_eq!(to_unicode("\\sqrt[3]{x}"), "³(x)");
        assert_eq!(to_unicode("\\binom{n}{k}"), "C(n, k)");
    }

    #[test]
    fn converts_delimiters_and_functions() {
        assert_eq!(to_unicode("\\lceil x \\rceil"), "⌈ x ⌉");
        assert_eq!(to_unicode("\\lfloor n/2 \\rfloor"), "⌊ n/2 ⌋");
        assert_eq!(to_unicode("\\langle x, y \\rangle"), "⟨ x, y ⟩");
        assert_eq!(to_unicode("\\log_2 n"), "log₂ n");
        assert_eq!(to_unicode("\\log_{1/p} n"), "log₍₁/ₚ₎ n");
        assert_eq!(to_unicode("\\mathcal{O}(n \\log n)"), "O(n log n)");
        assert_eq!(to_unicode("\\left(\\frac{a}{b}\\right)"), "(a/b)");
        assert_eq!(to_unicode("\\left. x \\right|"), " x |");
    }

    #[test]
    fn converts_accents_and_escaped_punctuation() {
        assert_eq!(to_unicode("\\hat{x}"), "x\u{0302}");
        assert_eq!(to_unicode("\\overline{AB}"), "A\u{0305}B\u{0305}");
        assert_eq!(to_unicode("\\vec{v}"), "v\u{20D7}");
        assert_eq!(to_unicode("50\\%"), "50%");
        assert_eq!(to_unicode("\\$5"), "$5");
    }

    #[test]
    fn environments_become_lines() {
        assert_eq!(
            to_unicode("\\begin{aligned} a &= b \\\\ c &= d \\end{aligned}"),
            " a   = b \n c   = d "
        );
        assert_eq!(to_unicode("\\boxed{x}"), "x");
        assert_eq!(to_unicode("\\underbrace{a+b}_{n}"), "a+bₙ");
        assert_eq!(to_unicode("n \\pmod{2}"), "n  (mod 2)");
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
