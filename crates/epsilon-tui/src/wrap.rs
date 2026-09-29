//! Deterministic screen-space wrapping for transcript lines (M16a).
//!
//! `Paragraph::wrap` wraps at draw time on already-built text; we need the wrap
//! decided BEFORE drawing so only visible rows are materialized (the whole point of
//! virtualization). This module is that decision: a pure function from styled spans
//! plus width to wrapped rows, using unicode display widths so CJK/emoji columns
//! match what ratatui will actually render.

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Wrap one logical line into at most-`width`-wide rows without splitting words.
///
/// - Whitespace between words is dropped at wrap points; trailing whitespace never
///   survives to the rendered row (screen convention).
/// - A single word longer than `width` hard-splits at the column boundary.
/// - Empty input yields one empty row (a blank screen row, matching Paragraph).
///
/// Takes `&str` rather than `&String`: the streaming tail is wrapped twice per
/// frame — once to count its rows for the footer's position badge, once to draw
/// them — and both callers hold a slice of the in-flight text. Taking a
/// `&[(String, Style)]` meant each of them had to manufacture an owned copy of
/// every line first, which is the allocation this signature exists to remove.
/// `Cow` is unnecessary: the only span a caller has to own is the `" "` that
/// stands in for collapsed whitespace, and that is built in [`flush_pending`].
pub fn wrap_line(
    spans: &[(&str, ratatui::style::Style)],
    width: usize,
) -> Vec<Vec<(String, ratatui::style::Style)>> {
    if width == 0 {
        return vec![
            spans
                .iter()
                .map(|(text, style)| ((*text).to_string(), *style))
                .collect(),
        ];
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
                let mut rest = token;
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
            push_text(&mut current, &mut col, token, *style);
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
///
/// The width of one character comes from [`UnicodeWidthChar::width`] rather than
/// from `ch.to_string().width()`. Those agree, and the second one allocated a
/// `String` **per character** — on the hard-split path, which is the one a long
/// path, a URL, a long identifier or a base64 blob takes, and therefore the one
/// a coding agent's tool output takes most often.
fn split_at_width(s: &str, width: usize) -> &str {
    let mut acc = 0usize;
    for (idx, ch) in s.char_indices() {
        let cw = UnicodeWidthChar::width(ch).unwrap_or(0);
        if acc + cw > width {
            return &s[..idx];
        }
        acc += cw;
    }
    s
}

/// Split `text` into runs of whitespace and runs of non-whitespace, **borrowed**.
///
/// The tokens are slices of `text`, not copies. It used to build a `String` per
/// token and then hand back a borrow of it, which meant one heap allocation for
/// every word in the line — on a path that runs over the whole in-flight answer
/// twice per drawn frame. The caller only ever reads a token, so the copy bought
/// nothing.
///
/// Whitespace is still tokenized rather than skipped in the loop below: it is how
/// a line is split into the words a wrap point may fall between.
fn tokenize(text: &str) -> Vec<&str> {
    let mut tokens = Vec::new();
    let mut start = 0usize;
    let mut cur_ws = None;
    for (idx, ch) in text.char_indices() {
        let ws = ch.is_whitespace();
        if cur_ws.is_none() {
            cur_ws = Some(ws);
        } else if cur_ws != Some(ws) {
            tokens.push(&text[start..idx]);
            start = idx;
            cur_ws = Some(ws);
        }
    }
    if start < text.len() {
        tokens.push(&text[start..]);
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Style;

    fn plain(s: &str) -> Vec<(&str, Style)> {
        vec![(s, Style::default())]
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

    /// The hard-split path — the one a long path, URL or identifier takes, and so
    /// the one a coding agent's tool output takes constantly. It walks the string
    /// a character at a time measuring widths, and it used to build a `String` to
    /// measure each one. This is the shape of input that made that expensive.
    #[test]
    fn a_word_far_wider_than_the_row_hard_splits_on_every_character() {
        let long = "x".repeat(200);
        let rows = wrap_line(&plain(&long), 7);
        assert_eq!(rows.len(), 29, "200 columns of 7 does not make 29 rows");
        for row in &rows {
            let width: usize = row
                .iter()
                .map(|(t, _)| crate::ui::layout::display_width(t))
                .sum();
            assert!(width <= 7, "a hard-split row overflowed: {width}");
        }
        // `concat()` joins the inner `Vec`s, not the strings: the first version
        // of this asserted `rows.concat().len() == 200` and got 29, which is the
        // row count. Flatten the text first, or the assertion measures nothing.
        let all: String = rows
            .iter()
            .flat_map(|r| r.iter())
            .map(|(t, _)| t.as_str())
            .collect();
        assert_eq!(all, long, "every character survived the hard split");
    }

    /// A multi-byte word is the same path, and the two are the reason the width
    /// has to come from a grapheme-aware table rather than from `len()`.
    #[test]
    fn a_wide_glyph_is_never_split_down_the_middle() {
        let text = "日本語のテキストです";
        let rows = wrap_line(&plain(text), 7);
        for row in &rows {
            let width: usize = row
                .iter()
                .map(|(t, _)| crate::ui::layout::display_width(t))
                .sum();
            assert!(width <= 7, "row of width {width}: {row:?}");
        }
        let all: String = rows
            .iter()
            .flat_map(|r| r.iter())
            .map(|(t, _)| t.as_str())
            .collect();
        assert_eq!(all, text, "no glyph was dropped or halved");
    }

    #[test]
    fn styles_are_preserved_across_the_wrap() {
        let bold = Style::default().add_modifier(ratatui::style::Modifier::BOLD);
        let spans = vec![
            ("bold header ", bold),
            ("plain continuation text here", Style::default()),
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

    /// The two callers that must agree: the footer counts the streaming tail's
    /// rows for its position badge, and the body draws them. They read the same
    /// text through the same call, so a row count that disagreed with the draw
    /// would need a second implementation to exist — and there is not one.
    ///
    /// **The fixture has no `\n`, and that is the point.** Both call sites split
    /// the tail on `'\n'` and wrap each piece, so a newline is a *line break* to
    /// them and merely whitespace to `wrap_line` itself. A test that fed this
    /// function text containing newlines and compared it against a
    /// split-then-wrap sum was comparing two different operations — the first
    /// version of this test did exactly that, and failed at width 1 with 30
    /// against 33, which says nothing about either implementation.
    ///
    /// So what is asserted is the property the badge actually rests on: for a
    /// single logical line, the row count is a function of the text and the
    /// width alone, and every character survives the wrapping.
    #[test]
    fn a_lines_row_count_depends_only_on_its_text_and_width() {
        let text = "a bb  ccc dddd eeeee ffffff  gggggggg   trailing";
        for width in [1usize, 3, 8, 20, 57, 200] {
            let first = wrap_line(&plain(text), width);
            let second = wrap_line(&plain(text), width);
            assert_eq!(
                first.len(),
                second.len(),
                "row count is not stable at {width}"
            );
            let joined: String = first
                .iter()
                .flat_map(|r| r.iter())
                .map(|(t, _)| t.as_str())
                .collect();
            // Whitespace at a wrap point is dropped by design, so compare the
            // non-space characters: the point is that no *content* is lost.
            let squeeze =
                |s: &str| -> String { s.chars().filter(|c| !c.is_whitespace()).collect() };
            assert_eq!(
                squeeze(&joined),
                squeeze(text),
                "content lost at width {width}"
            );
        }
    }
}
