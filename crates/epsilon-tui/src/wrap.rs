//! Deterministic screen-space wrapping for transcript lines (M16a).
//!
//! `Paragraph::wrap` wraps at draw time on already-built text; we need the wrap
//! decided BEFORE drawing so only visible rows are materialized (the whole point of
//! virtualization). This module is that decision: a pure function from styled spans
//! plus width to wrapped rows, using unicode display widths so CJK/emoji columns
//! match what ratatui will actually render.

use unicode_width::UnicodeWidthStr;

/// Wrap one logical line into at most-`width`-wide rows without splitting words.
///
/// - Whitespace between words is dropped at wrap points; trailing whitespace never
///   survives to the rendered row (screen convention).
/// - A single word longer than `width` hard-splits at the column boundary.
/// - Empty input yields one empty row (a blank screen row, matching Paragraph).
pub fn wrap_line(
    spans: &[(String, ratatui::style::Style)],
    width: usize,
) -> Vec<Vec<(String, ratatui::style::Style)>> {
    if width == 0 {
        return vec![spans.to_vec()];
    }
    let mut rows: Vec<Vec<(String, ratatui::style::Style)>> = Vec::new();
    let mut current: Vec<(String, ratatui::style::Style)> = Vec::new();
    let mut col = 0usize;
    // Pending whitespace + the style of the span it came from; emitted only when a
    // word actually follows (trailing whitespace never renders).
    let mut pending_ws: Option<(String, ratatui::style::Style)> = None;

    for (text, style) in spans {
        for token in tokenize(text) {
            let is_ws = token.chars().all(char::is_whitespace);
            if is_ws {
                // One space is all a wrap point or separator ever needs.
                if pending_ws.is_none() {
                    pending_ws = Some((" ".to_string(), *style));
                }
                continue;
            }
            let w = token.width();
            if w > width {
                flush_pending(&mut current, &mut pending_ws, &mut col);
                let mut rest = token.as_str();
                while rest.width() > width {
                    let cut = split_at_width(rest, width);
                    push_text(&mut current, &mut col, cut, *style);
                    rest = &rest[cut.len()..];
                    if !rest.is_empty() {
                        rows.push(std::mem::take(&mut current));
                        col = 0;
                    }
                }
                if !rest.is_empty() {
                    push_text(&mut current, &mut col, rest, *style);
                }
                continue;
            }
            if col + w > width {
                rows.push(std::mem::take(&mut current));
                col = 0;
                pending_ws = None; // the wrap eats the gap
            } else {
                flush_pending(&mut current, &mut pending_ws, &mut col);
            }
            push_text(&mut current, &mut col, &token, *style);
        }
    }
    if !current.is_empty() || rows.is_empty() {
        rows.push(current);
    }
    rows
}

fn flush_pending(
    current: &mut Vec<(String, ratatui::style::Style)>,
    pending_ws: &mut Option<(String, ratatui::style::Style)>,
    col: &mut usize,
) {
    let Some((ws, ws_style)) = pending_ws.take() else {
        return;
    };
    *col += UnicodeWidthStr::width(ws.as_str());
    match current.last_mut() {
        // Whitespace belongs to the span it followed; merge when styles match.
        Some((last_text, last_style)) if *last_style == ws_style => last_text.push_str(&ws),
        _ => current.push((ws, ws_style)),
    }
}

fn push_text(
    current: &mut Vec<(String, ratatui::style::Style)>,
    col: &mut usize,
    token: &str,
    style: ratatui::style::Style,
) {
    if let Some((last_text, last_style)) = current.last_mut()
        && *last_style == style
    {
        last_text.push_str(token);
    } else {
        current.push((token.to_string(), style));
    }
    *col += UnicodeWidthStr::width(token);
}

/// Split `s` into an initial segment of exactly display-width `width`.
fn split_at_width(s: &str, width: usize) -> &str {
    let mut acc = 0usize;
    for (idx, ch) in s.char_indices() {
        let cw = ch.to_string().width();
        if acc + cw > width {
            return &s[..idx];
        }
        acc += cw;
    }
    s
}

fn tokenize(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut cur = String::new();
    let mut cur_ws = None;
    for ch in text.chars() {
        let ws = ch.is_whitespace();
        if cur_ws.is_none() {
            cur_ws = Some(ws);
            cur.push(ch);
            continue;
        }
        if cur_ws == Some(ws) {
            cur.push(ch);
        } else {
            tokens.push(std::mem::take(&mut cur));
            cur.push(ch);
            cur_ws = Some(ws);
        }
    }
    if !cur.is_empty() {
        tokens.push(cur);
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Style;

    fn plain(s: &str) -> Vec<(String, Style)> {
        vec![(s.to_string(), Style::default())]
    }

    fn render(rows: &[Vec<(String, Style)>]) -> Vec<String> {
        rows.iter()
            .map(|r| r.iter().map(|(t, _)| t.as_str()).collect())
            .collect()
    }

    #[test]
    fn short_lines_pass_through_as_one_row() {
        let rows = wrap_line(&plain("hello"), 40);
        assert_eq!(render(&rows), vec!["hello"]);
    }

    #[test]
    fn wraps_at_word_boundaries_not_mid_word() {
        let rows = wrap_line(&plain("alpha beta gamma delta"), 11);
        assert_eq!(render(&rows), vec!["alpha beta", "gamma delta"]);
    }

    #[test]
    fn oversized_words_hard_split() {
        let rows = wrap_line(&plain("abcdefghijklmnop"), 6);
        assert_eq!(render(&rows), vec!["abcdef", "ghijkl", "mnop"]);
    }

    #[test]
    fn styles_are_preserved_across_the_wrap() {
        let bold = Style::default().add_modifier(ratatui::style::Modifier::BOLD);
        let spans = vec![
            ("bold header ".to_string(), bold),
            ("plain continuation text here".to_string(), Style::default()),
        ];
        let rows = wrap_line(&spans, 12);
        // 12-col rows: "bold header" fills row 1 exactly; the remaining words
        // wrap one-per-row where they don't pair within 12 columns.
        assert_eq!(
            rows,
            vec![
                vec![("bold header".to_string(), bold)],
                vec![("plain".to_string(), Style::default())],
                vec![("continuation".to_string(), Style::default())],
                vec![("text here".to_string(), Style::default())],
            ]
        );
    }

    #[test]
    fn wide_chars_count_double_width() {
        // Each CJK glyph is 2 columns; 4 glyphs = 8 columns.
        let rows = wrap_line(&plain("你好世界"), 5);
        assert_eq!(render(&rows), vec!["你好", "世界"]);
    }

    #[test]
    fn empty_input_yields_one_blank_row() {
        assert_eq!(wrap_line(&plain(""), 10).len(), 1);
        assert_eq!(wrap_line(&[], 10).len(), 1);
    }

    #[test]
    fn zero_width_disables_wrapping() {
        let rows = wrap_line(&plain("a b c"), 0);
        assert_eq!(render(&rows).len(), 1);
    }
}
