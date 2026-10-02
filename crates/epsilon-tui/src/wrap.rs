//! Deterministic screen-space wrapping for transcript lines (M16a).
//!
//! `Paragraph::wrap` wraps at draw time on already-built text; we need the wrap
//! decided BEFORE drawing so only visible rows are materialized (the whole point of
//! virtualization). This module is that decision: a pure function from styled spans
//! plus width to wrapped rows, using unicode display widths so CJK/emoji columns
//! match what ratatui will actually render.

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Shorten `text` to at most `budget` **screen columns**, marking a cut with an ellipsis.
///
/// # Why not `.chars().take(n)`
///
/// Three previews did exactly that, and it is wrong in two independent ways.
///
/// **The budget was in the wrong unit.** A hundred CJK characters are two hundred columns,
/// so a "100-character" preview overflowed the pane it was built to fit — the opposite of
/// what a truncation is for, and invisible until someone names a file in Japanese.
///
/// **It split grapheme clusters.** Taking 100 `char`s can cut a combining sequence or leave
/// half a wide glyph, which is how a row ends in a broken box in CP437.
///
/// Both are already solved elsewhere in this crate — [`wrap_line`] has measured display
/// width since M16a — so this is the same rule applied where it was missing, not a new
/// policy.
pub fn fit_width(text: &str, budget: usize) -> String {
    fit_width_with(text, budget, "…")
}

/// [`fit_width`] with the truncation mark supplied by the caller.
///
/// The mark is a parameter because it is a **repertoire** decision, not a cosmetic one:
/// `…` is one column on a modern terminal and **not representable at all in CP437**,
/// where the ASCII repertoire passes `...` instead (`ui::theme::Glyphs`, M219/A22). That
/// difference is three cells against one, and a budget that reserved the wrong one came
/// back two columns wide and cost the footer a hint at M220 — so the mark is *measured*,
/// never assumed.
///
/// The budget must leave room for the mark beside some text; a cut that spends the whole
/// budget on the mark says less than no mark, so an impossible budget gets the mark alone.
pub fn fit_width_with(text: &str, budget: usize, mark: &str) -> String {
    if budget == 0 {
        return String::new();
    }
    if UnicodeWidthStr::width(text) <= budget {
        return text.to_string();
    }
    let mark_width = UnicodeWidthStr::width(mark);
    if mark_width >= budget {
        return mark.chars().take(budget).collect();
    }
    let text_budget = budget - mark_width;
    let mut out = String::new();
    let mut used = 0usize;
    for grapheme in UnicodeSegmentation::graphemes(text, true) {
        let w = UnicodeWidthStr::width(grapheme);
        if used + w > text_budget {
            break;
        }
        out.push_str(grapheme);
        used += w;
    }
    out.push_str(mark);
    out
}

#[cfg(test)]
mod fit_width_tests {
    use super::fit_width;

    #[test]
    fn short_text_is_returned_untouched() {
        assert_eq!(fit_width("hello", 10), "hello");
        assert_eq!(fit_width("", 10), "");
    }

    /// The defect this replaced: a 100-*character* budget is a 200-*column* row in CJK, so
    /// the preview overflowed the pane it was built to fit.
    #[test]
    fn a_wide_script_is_cut_to_the_column_budget_not_the_character_count() {
        // 120 ideographs: 240 columns against a 100-column budget.
        let wide = "鯨".repeat(120);
        let cut = fit_width(&wide, 100);
        assert!(
            unicode_width::UnicodeWidthStr::width(cut.as_str()) <= 100,
            "the preview must fit its budget, got {} columns",
            unicode_width::UnicodeWidthStr::width(cut.as_str())
        );
        assert!(cut.ends_with('…'), "and must say it was cut");
    }

    /// A cut must not leave half a glyph or a split combining sequence behind.
    #[test]
    fn a_cut_never_splits_a_grapheme_cluster() {
        // "e" + combining acute: one grapheme, one column.
        let combining = "cafe\u{301} au lait, and then some more text";
        let cut = fit_width(combining, 6);
        assert!(
            !cut.ends_with('e'),
            "cut after the base letter and dropped its accent: {cut:?}"
        );
        // An emoji with a skin-tone modifier is one grapheme and two columns.
        let emoji = "\u{1F44D}\u{1F3FD} ".repeat(20);
        let cut = fit_width(&emoji, 10);
        assert!(
            !cut.contains('\u{FFFD}'),
            "a cut must not leave half a glyph behind: {cut:?}"
        );
        assert!(
            unicode_width::UnicodeWidthStr::width(cut.as_str()) <= 10,
            "and must still fit the budget: {cut:?}"
        );
        // Every remaining grapheme is whole — a skin-tone modifier is never orphaned from
        // the hand it modifies.
        assert!(
            cut.trim_end_matches('…')
                .trim_end()
                .ends_with("\u{1F44D}\u{1F3FD}"),
            "the last kept emoji must keep its modifier: {cut:?}"
        );
    }

    /// A budget too small for text *and* a mark still produces something readable.
    #[test]
    fn a_tiny_budget_still_returns_something() {
        assert_eq!(fit_width("abcdef", 0), "");
        let one = fit_width("abcdef", 1);
        assert!(
            unicode_width::UnicodeWidthStr::width(one.as_str()) <= 1,
            "a one-column budget must not produce a wider row: {one:?}"
        );
    }

    /// The ASCII repertoire's mark is `...` — **three** cells against `…`'s one — so a
    /// caller that reserved one and wrote three came back two columns wide. That is not
    /// hypothetical: `format_path` lost the footer's `g jump to live` hint to exactly
    /// this at M220, and every card headline in the transcript pays the same toll if the
    /// mark is assumed rather than measured.
    #[test]
    fn the_ascii_mark_costs_its_own_three_columns() {
        use super::fit_width_with;
        let cut = fit_width_with("abcdefghij", 6, "...");
        assert_eq!(cut, "abc...");
        assert_eq!(unicode_width::UnicodeWidthStr::width(cut.as_str()), 6);
        // The same budget with the one-column mark keeps two more characters, because
        // it can afford them.
        assert_eq!(fit_width_with("abcdefghij", 6, "…"), "abcde…");
    }
}

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
                // A word wider than the whole row cannot share a row with
                // anything. It used to be appended to whatever the row already
                // held — the pending space, and whatever word preceded it — so
                // each chunk was measured against the full budget while being
                // *drawn* into the leftover, and the row came out as wide as the
                // text before it plus `width`. At width 1 a row read `I w`.
                //
                // Starting the row here rather than flushing into it is also
                // what makes the trailing space disappear: a wrap point eats the
                // gap, exactly as the word path below does.
                if col > 0 {
                    rows.push(std::mem::take(&mut current));
                    col = 0;
                }
                pending_ws = None;
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
            // The fit test has to include the separator that is about to be
            // flushed, and that is the whole of a defect that silently ate a
            // character at 60 columns: `col + w` fits, the space is emitted
            // anyway, and the row lands one column over the budget. The
            // transcript then clips it — and a one-column clip mid-word turns
            // "then" into "the", on the user's own message, with no error.
            //
            // Measured: at body width 58 this produced a 59-column row, which
            // `decorate`'s two-column gutter pushed to 61 in a 60-column frame.
            // The old `a_lines_row_count_depends_only_on_its_text_and_width`
            // property test passed throughout, because it asserted that no
            // *content* was lost and none was — the loss is a width, downstream.
            let sep = pending_ws
                .as_ref()
                .map_or(0, |(ws, _)| UnicodeWidthStr::width(ws.as_str()));
            if col + sep + w > width {
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

    /// **No row may be wider than the width it was asked for.** The other property
    /// test in this file asserts that wrapping loses no *content*, and it passed
    /// for years while the wrapper emitted rows one column over budget. Both are
    /// true at once: nothing is dropped, and something is pushed off the edge.
    ///
    /// That is the shape the defect took. The transcript wraps to `width`, prefixes
    /// a two-column gutter, and the renderer clips at the edge — so an over-wide row
    /// loses its *last character* and nothing complains. A user's own message came
    /// back as `…in the workspace and the` / `read the layout module…`, which is a
    /// different sentence from the one they typed. The row-count property test could
    /// not see it, because the row count was always right.
    ///
    /// The sweep is deliberate about where it asserts: the hard-split path already
    /// had a width assertion, and this one is about the *word* path, so a fixture of
    /// ordinary prose at every width from 1 to its own length is enough to fail on
    /// the separator alone.
    #[test]
    fn no_row_is_wider_than_the_width_it_was_given() {
        let text = "I will start by listing the files in the workspace and then read the \
                    layout module to understand how the frame is composed before making \
                    any changes to it.";
        for width in 1..=text.len() {
            let rows = wrap_line(&plain(text), width);
            for (i, row) in rows.iter().enumerate() {
                let w: usize = row
                    .iter()
                    .map(|(t, _)| crate::ui::layout::display_width(t))
                    .sum();
                assert!(w <= width, "row {i} is {w} wide, budget {width}: {row:?}");
            }
        }
    }

    /// The same invariant across **styled** spans, where the separator is a span
    /// of its own and `flush_pending` can decline to merge it.
    ///
    /// The first version of the width sweep above passed with a single span, so it
    /// could not have caught a bug that only appears when the space arrives from a
    /// different style than the word after it — which is what a rendered markdown
    /// row is.
    #[test]
    fn no_row_is_wider_than_the_width_across_styled_spans() {
        let bold = Style::default().add_modifier(ratatui::style::Modifier::BOLD);
        let spans = vec![
            ("plain words that go on ", Style::default()),
            ("and on", bold),
            (" until the row is full of them", Style::default()),
        ];
        for width in 1..=60 {
            for row in wrap_line(&spans, width) {
                let w: usize = row
                    .iter()
                    .map(|(t, _)| crate::ui::layout::display_width(t))
                    .sum();
                assert!(w <= width, "row of {w} columns at width {width}: {row:?}");
            }
        }
    }
}
