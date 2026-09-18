//! Minimal LaTeX → Unicode for terminal rendering (pi `latex.ts` parity).

//!

//! Full `latex.ts` is ~1380 LOC with matrix/aligned/cases layout nodes. This module ports
//! the full 217-symbol table plus `\\frac`/`\\sqrt`/`^`/`_` and plain-text fallbacks for
//! `\\\\`/`&`/`\\begin{…}` (matrix/aligned/cases render as linear joins, ponytail deferred
//! until a transcript needs `pmatrix` fidelity). Unsupported constructs fall back to the raw
//! source inside the math delimiters.

use std::collections::HashMap;

use std::sync::OnceLock;

fn symbols() -> &'static HashMap<&'static str, &'static str> {
    static MAP: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();

    MAP.get_or_init(|| {
        let mut m = HashMap::new();

        // Greek + common operators — verbatim from pi SYMBOLS (217 entries, full parity)

        let pairs: &[(&str, &str)] = &[
            ("alpha", "α"),
            ("beta", "β"),
            ("gamma", "γ"),
            ("delta", "δ"),
            ("epsilon", "ϵ"),
            ("varepsilon", "ε"),
            ("zeta", "ζ"),
            ("eta", "η"),
            ("theta", "θ"),
            ("vartheta", "ϑ"),
            ("iota", "ι"),
            ("kappa", "κ"),
            ("varkappa", "ϰ"),
            ("lambda", "λ"),
            ("mu", "μ"),
            ("nu", "ν"),
            ("xi", "ξ"),
            ("pi", "π"),
            ("varpi", "ϖ"),
            ("rho", "ρ"),
            ("varrho", "ϱ"),
            ("sigma", "σ"),
            ("varsigma", "ς"),
            ("tau", "τ"),
            ("upsilon", "υ"),
            ("phi", "ϕ"),
            ("varphi", "φ"),
            ("chi", "χ"),
            ("psi", "ψ"),
            ("omega", "ω"),
            ("Gamma", "Γ"),
            ("Delta", "Δ"),
            ("Theta", "Θ"),
            ("Lambda", "Λ"),
            ("Xi", "Ξ"),
            ("Pi", "Π"),
            ("Sigma", "Σ"),
            ("Upsilon", "Υ"),
            ("Phi", "Φ"),
            ("Psi", "Ψ"),
            ("Omega", "Ω"),
            ("pm", "±"),
            ("mp", "∓"),
            ("times", "×"),
            ("div", "÷"),
            ("cdot", "·"),
            ("ast", "∗"),
            ("star", "⋆"),
            ("circ", "∘"),
            ("bullet", "•"),
            ("oplus", "⊕"),
            ("ominus", "⊖"),
            ("otimes", "⊗"),
            ("oslash", "⊘"),
            ("odot", "⊙"),
            ("bigcirc", "○"),
            ("dagger", "†"),
            ("ddagger", "‡"),
            ("amalg", "⨿"),
            ("uplus", "⊎"),
            ("sqcap", "⊓"),
            ("sqcup", "⊔"),
            ("triangleleft", "◁"),
            ("triangleright", "▷"),
            ("wr", "≀"),
            ("cap", "∩"),
            ("cup", "∪"),
            ("bigcap", "⋂"),
            ("bigcup", "⋃"),
            ("bigwedge", "⋀"),
            ("bigvee", "⋁"),
            ("bigsqcup", "⨆"),
            ("biguplus", "⨄"),
            ("bigoplus", "⨁"),
            ("bigotimes", "⨂"),
            ("bigodot", "⨀"),
            ("setminus", "∖"),
            ("in", "∈"),
            ("notin", "∉"),
            ("ni", "∋"),
            ("subset", "⊂"),
            ("supset", "⊃"),
            ("subseteq", "⊆"),
            ("supseteq", "⊇"),
            ("sqsubset", "⊏"),
            ("sqsupset", "⊐"),
            ("sqsubseteq", "⊑"),
            ("sqsupseteq", "⊒"),
            ("prec", "≺"),
            ("preceq", "≼"),
            ("succ", "≻"),
            ("succeq", "≽"),
            ("ll", "≪"),
            ("gg", "≫"),
            ("le", "≤"),
            ("leq", "≤"),
            ("leqslant", "≤"),
            ("ge", "≥"),
            ("geq", "≥"),
            ("geqslant", "≥"),
            ("ne", "≠"),
            ("neq", "≠"),
            ("equiv", "≡"),
            ("approx", "≈"),
            ("sim", "∼"),
            ("simeq", "≃"),
            ("cong", "≅"),
            ("asymp", "≍"),
            ("doteq", "≐"),
            ("propto", "∝"),
            ("parallel", "∥"),
            ("perp", "⊥"),
            ("mid", "∣"),
            ("vdash", "⊢"),
            ("dashv", "⊣"),
            ("models", "⊨"),
            ("Vdash", "⊩"),
            ("Vvdash", "⊪"),
            ("nvdash", "⊬"),
            ("nvDash", "⊭"),
            ("forall", "∀"),
            ("exists", "∃"),
            ("nexists", "∄"),
            ("neg", "¬"),
            ("land", "∧"),
            ("wedge", "∧"),
            ("lor", "∨"),
            ("vee", "∨"),
            ("to", "→"),
            ("rightarrow", "→"),
            ("longrightarrow", "→"),
            ("leftarrow", "←"),
            ("longleftarrow", "←"),
            ("gets", "←"),
            ("leftrightarrow", "↔"),
            ("longleftrightarrow", "↔"),
            ("hookleftarrow", "↩"),
            ("hookrightarrow", "↪"),
            ("twoheadleftarrow", "↞"),
            ("twoheadrightarrow", "↠"),
            ("leftharpoonup", "↼"),
            ("leftharpoondown", "↽"),
            ("rightharpoonup", "⇀"),
            ("rightharpoondown", "⇁"),
            ("rightleftharpoons", "⇌"),
            ("leftrightharpoons", "⇋"),
            ("nearrow", "↗"),
            ("searrow", "↘"),
            ("swarrow", "↙"),
            ("nwarrow", "↖"),
            ("rightsquigarrow", "⇝"),
            ("leadsto", "⇝"),
            ("Rightarrow", "⇒"),
            ("Longrightarrow", "⇒"),
            ("Leftarrow", "⇐"),
            ("Longleftarrow", "⇐"),
            ("Leftrightarrow", "⇔"),
            ("Longleftrightarrow", "⇔"),
            ("implies", "⇒"),
            ("iff", "⇔"),
            ("mapsto", "↦"),
            ("longmapsto", "↦"),
            ("uparrow", "↑"),
            ("downarrow", "↓"),
            ("partial", "∂"),
            ("nabla", "∇"),
            ("int", "∫"),
            ("iint", "∬"),
            ("iiint", "∭"),
            ("oint", "∮"),
            ("sum", "∑"),
            ("prod", "∏"),
            ("coprod", "∐"),
            ("infty", "∞"),
            ("emptyset", "∅"),
            ("varnothing", "∅"),
            ("angle", "∠"),
            ("therefore", "∴"),
            ("because", "∵"),
            ("aleph", "ℵ"),
            ("beth", "ℶ"),
            ("gimel", "ℷ"),
            ("daleth", "ℸ"),
            ("top", "⊤"),
            ("bot", "⊥"),
            ("triangle", "△"),
            ("square", "□"),
            ("lozenge", "◊"),
            ("checkmark", "✓"),
            ("complement", "∁"),
            ("wp", "℘"),
            ("prime", "′"),
            ("ldots", "…"),
            ("dots", "…"),
            ("cdots", "⋯"),
            ("vdots", "⋮"),
            ("ddots", "⋱"),
            ("ell", "ℓ"),
            ("hbar", "ℏ"),
            ("Im", "ℑ"),
            ("Re", "ℜ"),
            ("langle", "⟨"),
            ("rangle", "⟩"),
            ("vert", "|"),
            ("lvert", "|"),
            ("rvert", "|"),
            ("Vert", "‖"),
            ("lVert", "‖"),
            ("rVert", "‖"),
            ("lbrace", "{"),
            ("rbrace", "}"),
            ("backslash", "\\"),
            ("lfloor", "⌊"),
            ("rfloor", "⌋"),
            ("lceil", "⌈"),
            ("rceil", "⌉"),
            ("colon", ":"),
        ];

        for (k, v) in pairs {
            m.insert(*k, *v);
        }

        m
    })
}

/// Render a LaTeX math fragment (inside `$…$` / `\(…\)` / `\[…\]` / `$$…$$`) to terminal unicode.
///
/// Returns `None` only for an empty input; unknown commands are passed through with the
/// backslash stripped so the transcript stays readable instead of vanishing. ponytail: no
/// layout nodes — `\\`, `&`, and `\begin{matrix}` become plain whitespace/joins.
pub fn render_latex(source: &str) -> Option<String> {
    let s = source.trim();

    if s.is_empty() {
        return None;
    }

    // Fast path: no backslash / ^ / _ → just return trimmed source (still useful to strip)

    // but still apply symbol replacement if a backslash exists.

    let mut out = String::with_capacity(s.len());

    let chars: Vec<char> = s.chars().collect();

    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];

        if c == '\\' {
            // parse command name

            let start = i + 1;

            if start >= chars.len() {
                i += 1;

                continue;
            }

            let nc = chars[start];

            if nc.is_ascii_alphabetic() {
                let mut j = start;

                while j < chars.len() && chars[j].is_ascii_alphabetic() {
                    j += 1;
                }

                let name: String = chars[start..j].iter().collect();

                // special forms

                if name == "frac" {
                    // \frac{num}{den} → num/den (ponytail: no stacking)

                    let (num, after_num) = parse_braced(&chars, j);

                    let (den, after_den) = parse_braced(&chars, after_num);

                    if let (Some(n), Some(d)) = (num, den) {
                        let rn = render_latex(&n).unwrap_or(n);

                        let rd = render_latex(&d).unwrap_or(d);

                        // ponytail: simple inline fraction

                        let simple_num = rn.chars().all(|ch| ch.is_alphanumeric() || ch == '.');

                        let simple_den = rd.chars().all(|ch| ch.is_ascii_digit() || ch == '.')
                            || rd.chars().count() == 1;

                        if simple_num && simple_den {
                            out.push_str(&format!("{}/{}", rn, rd));
                        } else {
                            out.push_str(&format!("({})/({})", rn, rd));
                        }
                    } else {
                        out.push_str(&format!("\\{}", name));
                    }

                    i = after_den;

                    continue;
                } else if name == "sqrt" {
                    let (inner, after) = parse_braced(&chars, j);

                    if let Some(v) = inner {
                        let rv = render_latex(&v).unwrap_or(v);

                        let bare = rv.chars().all(|ch| ch.is_alphanumeric() || ch == '.');

                        if bare {
                            out.push_str(&format!("√{}", rv));
                        } else {
                            out.push_str(&format!("√({})", rv));
                        }
                    } else {
                        out.push_str("\\sqrt");
                    }

                    i = after;

                    continue;
                } else if name == "text"
                    || name == "mathrm"
                    || name == "mathbf"
                    || name == "mathit"
                    || name == "mathbb"
                    || name == "mathcal"
                {
                    let (inner, after) = parse_braced(&chars, j);

                    if let Some(v) = inner {
                        out.push_str(&render_latex(&v).unwrap_or(v));

                        i = after;

                        continue;
                    }

                    i = j;

                    continue;
                } else if [
                    "displaystyle",
                    "limits",
                    "nolimits",
                    "scriptstyle",
                    "quad",
                    "qquad",
                    "enspace",
                    "thinspace",
                    "medspace",
                ]
                .contains(&name.as_str())
                {
                    i = j; // ignored spacing/style

                    continue;
                } else if let Some(sym) = symbols().get(name.as_str()) {
                    out.push_str(sym);
                } else {
                    // unknown command → keep name without backslash so text remains legible

                    out.push_str(&name);
                }

                i = j;
            } else {
                // single-char escape like \{, \}, \\, \%, etc.

                match nc {
                    '{' => out.push('{'),

                    '}' => out.push('}'),

                    '\\' => out.push('\n'), // pi splits rows on \\

                    '%' => out.push('%'),

                    '$' => out.push('$'),

                    '&' => out.push('&'),

                    '_' | '^' => out.push(nc),

                    _ => out.push(nc),
                }

                i += 2;
            }
        } else if c == '^' || c == '_' {
            // super/subscript — use unicode when the braced content is 1 char or mappable

            let is_sup = c == '^';

            let (inner, after) = if i + 1 < chars.len() && chars[i + 1] == '{' {
                let (v, a) = parse_braced(&chars, i + 1);

                (v, a)
            } else if i + 1 < chars.len() {
                (Some(chars[i + 1].to_string()), i + 2)
            } else {
                (None, i + 1)
            };

            if let Some(v) = inner {
                let rendered = render_sup_sub(&v, is_sup);

                out.push_str(&rendered);
            }

            i = after;
        } else if c == '&' {
            out.push_str(" │ ");

            i += 1;
        } else {
            out.push(c);

            i += 1;
        }
    }

    let trimmed = out.trim().to_string();

    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

fn parse_braced(chars: &[char], start: usize) -> (Option<String>, usize) {
    // start points at '{' or after command; skip whitespace, expect '{'

    let mut p = start;

    while p < chars.len() && chars[p].is_whitespace() {
        p += 1;
    }

    if p >= chars.len() || chars[p] != '{' {
        return (None, p);
    }

    p += 1;

    let mut depth = 1usize;

    let begin = p;

    while p < chars.len() {
        if chars[p] == '\\' {
            p += 2;

            continue;
        }

        if chars[p] == '{' {
            depth += 1;
        }

        if chars[p] == '}' {
            depth -= 1;

            if depth == 0 {
                let s: String = chars[begin..p].iter().collect();

                return (Some(s), p + 1);
            }
        }

        p += 1;
    }

    (None, p)
}

fn render_sup_sub(s: &str, is_sup: bool) -> String {
    let t = s.trim();

    if is_sup {
        if let Some(u) = to_superscript(t) {
            return u;
        }
    } else if let Some(u) = to_subscript(t) {
        return u;
    }

    let prefix = if is_sup { "^" } else { "_" };

    if t.chars().count() == 1 || (!is_sup && t.chars().all(|c| c.is_ascii_alphanumeric())) {
        format!("{}{}", prefix, t)
    } else {
        format!("{}({})", prefix, t)
    }
}

fn to_superscript(s: &str) -> Option<String> {
    const MAP: &[(&str, &str)] = &[
        ("0", "⁰"),
        ("1", "¹"),
        ("2", "²"),
        ("3", "³"),
        ("4", "⁴"),
        ("5", "⁵"),
        ("6", "⁶"),
        ("7", "⁷"),
        ("8", "⁸"),
        ("9", "⁹"),
        ("+", "⁺"),
        ("-", "⁻"),
        ("=", "⁼"),
        ("(", "⁽"),
        (")", "⁾"),
        ("a", "ᵃ"),
        ("b", "ᵇ"),
        ("c", "ᶜ"),
        ("d", "ᵈ"),
        ("e", "ᵉ"),
        ("f", "ᶠ"),
        ("g", "ᵍ"),
        ("h", "ʰ"),
        ("i", "ⁱ"),
        ("j", "ʲ"),
        ("k", "ᵏ"),
        ("l", "ˡ"),
        ("m", "ᵐ"),
        ("n", "ⁿ"),
        ("o", "ᵒ"),
        ("p", "ᵖ"),
        ("r", "ʳ"),
        ("s", "ˢ"),
        ("t", "ᵗ"),
        ("u", "ᵘ"),
        ("v", "ᵛ"),
        ("w", "ʷ"),
        ("x", "ˣ"),
        ("y", "ʸ"),
        ("z", "ᶻ"),
    ];

    let mut out = String::new();

    for ch in s.chars() {
        let k = ch.to_string();

        if let Some((_, v)) = MAP.iter().find(|(a, _)| *a == k) {
            out.push_str(v);
        } else {
            return None;
        }
    }

    Some(out)
}

fn to_subscript(s: &str) -> Option<String> {
    const MAP: &[(&str, &str)] = &[
        ("0", "₀"),
        ("1", "₁"),
        ("2", "₂"),
        ("3", "₃"),
        ("4", "₄"),
        ("5", "₅"),
        ("6", "₆"),
        ("7", "₇"),
        ("8", "₈"),
        ("9", "₉"),
        ("+", "₊"),
        ("-", "₋"),
        ("=", "₌"),
        ("(", "₍"),
        (")", "₎"),
        ("a", "ₐ"),
        ("e", "ₑ"),
        ("h", "ₕ"),
        ("i", "ᵢ"),
        ("j", "ⱼ"),
        ("k", "ₖ"),
        ("l", "ₗ"),
        ("m", "ₘ"),
        ("n", "ₙ"),
        ("o", "ₒ"),
        ("p", "ₚ"),
        ("r", "ᵣ"),
        ("s", "ₛ"),
        ("t", "ₜ"),
        ("u", "ᵤ"),
        ("v", "ᵥ"),
        ("x", "ₓ"),
    ];

    let mut out = String::new();

    for ch in s.chars() {
        let k = ch.to_string().to_lowercase();

        if let Some((_, v)) = MAP.iter().find(|(a, _)| *a == k) {
            out.push_str(v);
        } else {
            return None;
        }
    }

    Some(out)
}

/// Replace every math span (`$…$`, `$$…$$`, `\(…\)`, `\[…\]`) in `text` with its rendered unicode.
///
/// Non-math text is untouched. Unrenderable spans fall back to their inner text without delimiters.
/// This is the hook `markdown.rs` calls before `pulldown_cmark`.
pub fn preprocess_math(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();

    let mut out = String::with_capacity(text.len());

    let mut i = 0;

    while i < chars.len() {
        if i + 1 < chars.len()
            && chars[i] == '$'
            && chars[i + 1] == '$'
            && let Some(end) = find_closing(&chars, i + 2, &['$', '$'])
        {
            let inner: String = chars[i + 2..end].iter().collect();

            let rendered = render_latex(&inner).unwrap_or_else(|| inner.trim().to_string());

            out.push_str(&rendered);

            i = end + 2;

            continue;
        }

        if i + 1 < chars.len()
            && chars[i] == '\\'
            && chars[i + 1] == '['
            && let Some(end) = find_seq(&chars, i + 2, &['\\', ']'])
        {
            let inner: String = chars[i + 2..end].iter().collect();

            let rendered = render_latex(&inner).unwrap_or_else(|| inner.trim().to_string());

            // display math on its own line

            if !out.ends_with('\n') && !out.is_empty() {
                out.push('\n');
            }

            out.push_str(&rendered);

            if end + 2 < chars.len() {
                out.push('\n');
            }

            i = end + 2;

            continue;
        }

        if i + 1 < chars.len()
            && chars[i] == '\\'
            && chars[i + 1] == '('
            && let Some(end) = find_seq(&chars, i + 2, &['\\', ')'])
        {
            let inner: String = chars[i + 2..end].iter().collect();

            let rendered = render_latex(&inner).unwrap_or_else(|| inner.trim().to_string());

            out.push_str(&rendered);

            i = end + 2;

            continue;
        }

        if chars[i] == '$' {
            // single $…$ — must not be $$ and must not be `$ ` (pi guards against /`$ ` and `/`$` cases)

            if i + 1 < chars.len() && chars[i + 1] == ' ' {
                out.push(chars[i]);

                i += 1;

                continue;
            }

            if let Some(end) = find_single_dollar(&chars, i + 1) {
                let inner: String = chars[i + 1..end].iter().collect();

                if inner.contains('\n') || inner.is_empty() {
                    out.push(chars[i]);

                    i += 1;

                    continue;
                }

                // pi guards: /$VAR/ and VAR$ etc. — treat `$` adjacent to alphanum/_ on both sides as not math

                let before_ok = i == 0 || !is_word_char(chars[i - 1]);

                let after_ok = end + 1 >= chars.len() || !is_word_char(chars[end + 1]);

                // also guard `` ` `` inside like pi

                if inner.contains('`') {
                    out.push(chars[i]);

                    i += 1;

                    continue;
                }

                let rendered = render_latex(&inner).unwrap_or_else(|| inner.trim().to_string());

                // keep surrounding char guard simple: if both sides word, don't treat as math

                if !before_ok || !after_ok {
                    out.push(chars[i]);

                    i += 1;

                    continue;
                }

                out.push_str(&rendered);

                i = end + 1;

                continue;
            }
        }

        out.push(chars[i]);

        i += 1;
    }

    out
}

fn is_word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

fn find_closing(chars: &[char], start: usize, delim: &[char]) -> Option<usize> {
    let mut p = start;

    while p + delim.len() <= chars.len() {
        if chars[p..p + delim.len()] == *delim {
            // check not escaped

            let mut bs = 0;

            let mut q = p;

            while q > 0 && chars[q - 1] == '\\' {
                bs += 1;

                q -= 1;
            }

            if bs % 2 == 0 {
                return Some(p);
            }
        }

        p += 1;
    }

    None
}

fn find_seq(chars: &[char], start: usize, seq: &[char]) -> Option<usize> {
    find_closing(chars, start, seq)
}

fn find_single_dollar(chars: &[char], start: usize) -> Option<usize> {
    let mut p = start;

    while p < chars.len() {
        if chars[p] == '$' {
            let mut bs = 0;

            let mut q = p;

            while q > 0 && chars[q - 1] == '\\' {
                bs += 1;

                q -= 1;
            }

            if bs % 2 == 0 {
                // also skip $$ case

                if p + 1 < chars.len() && chars[p + 1] == '$' {
                    p += 2;

                    continue;
                }

                return Some(p);
            }
        }

        p += 1;
    }

    None
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]

    fn symbols_render() {
        assert_eq!(render_latex(r"\alpha").unwrap(), "α");

        assert_eq!(render_latex(r"\in").unwrap(), "∈");

        assert_eq!(render_latex(r"\Rightarrow").unwrap(), "⇒");

        assert_eq!(render_latex(r"x \in \mathbb{R}").unwrap(), "x ∈ R");
    }

    #[test]

    fn frac_and_sqrt() {
        assert_eq!(render_latex(r"\frac{1}{2}").unwrap(), "1/2");

        assert_eq!(render_latex(r"\frac{x+1}{y}").unwrap(), "(x+1)/(y)");

        assert_eq!(render_latex(r"\sqrt{x}").unwrap(), "√x");

        assert_eq!(render_latex(r"\sqrt{x+1}").unwrap(), "√(x+1)");
    }

    #[test]

    fn sup_sub() {
        assert_eq!(render_latex(r"x^2").unwrap(), "x²");

        assert_eq!(render_latex(r"x_{i}").unwrap(), "xᵢ");

        assert_eq!(render_latex(r"a^{10}").unwrap(), "a¹⁰");
    }

    #[test]

    fn preprocess_inline_and_display() {
        let s = r"Let $x \in \mathbb{R}$ and $$\alpha + \beta = \gamma$$ then \(a^2\)";

        let out = preprocess_math(s);

        assert!(out.contains("x ∈ R"), "{out}");

        assert!(out.contains("α + β = γ"), "{out}");

        assert!(out.contains("a²"), "{out}");

        assert!(!out.contains('$'), "delimiters stripped: {out}");

        assert!(!out.contains(r"\alpha"), "{out}");
    }

    #[test]

    fn dollar_guards() {
        // pi parity: `$ ` not math, `$VAR` env var not math

        assert_eq!(preprocess_math("$ hello"), "$ hello");

        assert_eq!(preprocess_math("cost $5"), "cost $5");
    }
}
