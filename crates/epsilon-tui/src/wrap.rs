//! Deterministic screen-space wrapping for transcript lines (M16a).
//!
//! `Paragraph::wrap` wraps at draw time on already-built text; we need the wrap
//! decided BEFORE drawing so only visible rows are materialized (the whole point of
//! virtualization). This module is that decision: a pure function from styled spans
//! plus width to wrapped rows, using unicode display widths so CJK/emoji columns
//! match what ratatui will actually render.

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

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
        let w = crate::ui::layout::display_width(grapheme);
        if used + w > text_budget {
            break;
        }
        out.push_str(grapheme);
        used += w;
    }
    out.push_str(mark);
    out
}

/// The longest prefix of `s` occupying at most `width` screen columns, cut on
/// **grapheme** boundaries.
///
/// # Why this exists rather than a `char` walk
///
/// Four functions in this crate used to cut text by walking `char`s and summing
/// ``UnicodeWidthChar::width``, and every one of them was wrong in the same way.
/// `unicode-width` implements *string-level* rules that disagree with the sum of its
/// per-character ones for exactly the characters a coding agent's output is full of:
///
/// | sequence | `UnicodeWidthStr::width` | sum of `UnicodeWidthChar::width` |
/// |---|---|---|
/// | `👨‍👩‍👧‍👦` (family, ZWJ) | 2 | 8 |
/// | `👍🏽` (skin tone) | 2 | 4 |
/// | `❤️` (VS16) | 2 | 1 |
/// | `1️⃣` (keycap) | 2 | 1 |
///
/// Those numbers are measured, not recalled. The consequence is that a cut decided with
/// one rule and measured with the other lands mid-cluster: a row came back ending in an
/// orphaned ZWJ with the next row starting at `👩`, which on most terminals renders as a
/// *different emoji* — one person instead of four — and on some joins across the row
/// boundary. It is width-dependent, so the same content was correct at 8 columns and
/// corrupt at 6.
///
/// A grapheme is the unit a terminal advances by, so it is the only unit a cut may fall
/// on. `unicode-width`'s per-grapheme answer is what [`UnicodeWidthStr::width]` gives for
/// that grapheme, which is the string rule applied at the right granularity.
///
/// The prefix may be **empty** — a 2-column grapheme does not fit a 1-column row, and
/// there is no honest non-empty answer. Callers that loop must not treat an empty return
/// as progress; see [`wrap_line`]'s hard-split arm for the one that had to learn that.
pub fn prefix_at_width(s: &str, width: usize) -> &str {
    let mut acc = 0usize;
    for (idx, grapheme) in s.grapheme_indices(true) {
        let w = crate::ui::layout::display_width(grapheme);
        if w == 0 {
            // A bare combining mark carries no column and belongs to whatever follows,
            // so it never triggers the cut.
            continue;
        }
        if acc + w > width {
            return &s[..idx];
        }
        acc += w;
    }
    s
}

/// The longest **suffix** of `s` occupying at most `width` screen columns, cut on
/// grapheme boundaries.
///
/// The mirror of [`prefix_at_width`], for the half of a status row that is written
/// least-important-first: trimming there removes the leading fragments, so what survives
/// is a suffix. Same rule — a grapheme is kept whole or not at all, and the result is
/// never over budget. It may be empty, for the same reason.
///
/// The walk drops whole graphemes from the front until the remainder fits, and stops at
/// the first point where it does: that is the *longest* suffix within budget. An earlier
/// version walked forward accumulating and returned whatever offset it happened to be
/// holding when the sum passed the budget, which produced a suffix wider than asked for
/// — measured `"ix👨‍👩‍👧‍👦👨‍👩‍👧‍👦"` for a budget of 4.
pub fn suffix_at_width(s: &str, width: usize) -> &str {
    let total = crate::ui::layout::display_width(s);
    if total <= width {
        return s;
    }
    let mut dropped = 0usize;
    let mut start = s.len();
    for (idx, grapheme) in s.grapheme_indices(true) {
        dropped += crate::ui::layout::display_width(grapheme);
        if total - dropped <= width {
            // The remainder now fits, so the suffix starts after this grapheme. This is
            // the longest one that does, because dropping any less would leave it over.
            start = idx + grapheme.len();
            break;
        }
    }
    &s[start..]
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
/// stands in for collapsed whitespace, and that is built in ``flush_pending``.
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
            let w = crate::ui::layout::display_width(token);
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
                while crate::ui::layout::display_width(rest) > width {
                    // **The cut must always advance, and this is the arm where it did
                    // not.** `split_at_width` returns the empty string when the row is
                    // narrower than the first grapheme — every wide glyph at width 1 —
                    // and an empty cut with the condition still true never terminates.
                    // That is not hypothetical: `app/panels.rs:1057` floors the slash
                    // popup's description budget at one column with `.max(1)`, so a
                    // description whose first token starts with a CJK character or an
                    // emoji hung the draw thread. Measured, not reasoned: a probe run on
                    // the old code hung past 60 seconds and had to be killed.
                    //
                    // The recovery emits the grapheme that does not fit on a row of its
                    // own rather than dropping it (a lost character is silent data loss)
                    // or halving it (half a glyph is a box). It overhangs by one column,
                    // which the renderer clips at the edge — the same trade a single wide
                    // glyph in a too-narrow row already makes everywhere else.
                    let cut = split_at_width(rest, width);
                    let cut = if cut.is_empty() {
                        rest.graphemes(true).next().unwrap_or(rest)
                    } else {
                        cut
                    };
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

/// Split `s` into an initial segment of at most `width` display columns, **on grapheme
/// boundaries**.
///
/// The width of one grapheme comes from [`UnicodeWidthStr::width] applied to that grapheme,
/// which is the string rule at the right granularity — `UnicodeWidthChar::width` sums to 8
/// for a family emoji whose real width is 2, so a cut decided with it lands mid-cluster.
/// See [`prefix_at_width`] for the measured table.
///
/// The result may be empty when the row is narrower than the first grapheme. That is the
/// case [`wrap_line`]'s hard-split arm must not read as progress.
fn split_at_width(s: &str, width: usize) -> &str {
    prefix_at_width(s, width)
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

    // ---------------------------------------------------------------------------------------------
    // Grapheme-cluster integrity, and the loop that never terminated
    //
    // Every fixture below is a character a coding agent's output actually contains: a ZWJ
    // family emoji, a skin-tone modifier, a variation selector, a keycap. The measured
    // disagreement that makes them dangerous is on `prefix_at_width` -- the per-character
    // width sum and the per-string width differ by six columns for a family emoji.
    // ---------------------------------------------------------------------------------------------

    /// A ZWJ family emoji: one grapheme, two columns, four characters summing to eight.
    const FAMILY: &str = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}\u{200D}\u{1F466}";
    /// A raised hand with a skin-tone modifier: one grapheme, two columns.
    const THUMB: &str = "\u{1F44D}\u{1F3FD}";
    /// A heart with a variation selector: one grapheme, two columns.
    const HEART: &str = "\u{2764}\u{FE0F}";
    /// Keycap 1: one grapheme, two columns.
    const KEYCAP: &str = "\u{0031}\u{FE0F}\u{20E3}";

    fn emoji_tokens() -> Vec<&'static str> {
        vec![FAMILY, THUMB, HEART, KEYCAP]
    }

    /// Do `rows` segment `text` into **whole** graphemes, in order?
    ///
    /// This is the property that matters, and it has to be stated this way round. The
    /// obvious-looking check — "does a row end in a zero-width character" — is wrong in the
    /// other direction: `❤️` is a *complete* grapheme whose last character is the
    /// zero-width variation selector, so that check reports every heart as broken. And
    /// measuring the concatenated rows is wrong too, because concatenating re-joins a
    /// cluster that was cut at a row boundary and reports nothing at all — which is what the
    /// first version of this helper did.
    ///
    /// So: walk the input's graphemes once, and consume exactly as many as each row holds.
    /// A row that ends inside one grapheme desynchronises the walk and is caught.
    fn rows_are_whole_graphemes(rows: &[Vec<(String, Style)>], text: &str) -> Option<usize> {
        use unicode_segmentation::UnicodeSegmentation;
        let mut want = text.graphemes(true);
        for (i, row) in rows.iter().enumerate() {
            let row_text: String = row.iter().map(|(t, _)| t.as_str()).collect();
            for held in row_text.graphemes(true) {
                match want.next() {
                    Some(g) if g == held => {}
                    // Either the row holds a grapheme the input does not have (a
                    // re-ordered or duplicated one), or it holds half of one and the walk
                    // has already moved past its base.
                    _ => return Some(i),
                }
            }
        }
        // And nothing may be left over: a dropped grapheme is silent data loss.
        if want.next().is_some() {
            return rows.len().checked_sub(1);
        }
        None
    }

    /// **The defect: a hard split cut an emoji ZWJ sequence at a row boundary.**
    ///
    /// Measured on the old code, at widths 2 through 7 and 10 out of 1 through 10:
    /// `row0='👨‍'`, `row0='👨‍👩‍👧‍'`. Widths 8 and 9 happened to be clean, which is what
    /// makes this class so easy to miss -- the same content was correct at one terminal
    /// width and corrupt at another.
    ///
    /// The rendering consequence is not cosmetic. A row ending in an orphaned ZWJ draws as
    /// a *different emoji* (one person where the text said a family of four), and on some
    /// terminals the joiner combines with the next row's first cell.
    #[test]
    fn a_hard_split_never_cuts_an_emoji_cluster_at_a_row_boundary() {
        for emoji in emoji_tokens() {
            let text = format!("{emoji}{emoji}{emoji}{emoji}{emoji}tail");
            for width in 1..=12 {
                let rows = wrap_line(&plain(&text), width);
                if let Some(broken_row) = rows_are_whole_graphemes(&rows, &text) {
                    let shown: Vec<String> = rows
                        .iter()
                        .map(|r| r.iter().map(|(t, _)| t.as_str()).collect())
                        .collect();
                    panic!(
                        "{emoji:?} at width {width} did not segment into whole graphemes \
                         (row {broken_row} is the first to desynchronise): {shown:?}"
                    );
                }
                for row in &rows {
                    let w: usize = row
                        .iter()
                        .map(|(t, _)| crate::ui::layout::display_width(t))
                        .sum();
                    if width == 1 {
                        // **The one deliberate overhang.** A 2-column cluster cannot fit a
                        // 1-column row, and the alternatives are dropping it (silent data
                        // loss) or halving it (a box). It is emitted whole on its own row
                        // and the renderer clips it at the edge, exactly as it already does
                        // for a wide glyph in a too-narrow row anywhere else. The bound is
                        // the grapheme's own width, so the overhang is one column and never
                        // accumulates.
                        assert!(
                            w <= 2,
                            "{emoji:?} at width 1: a row of {w} columns is more than one \
                             grapheme wide, so this is not the documented overhang"
                        );
                    } else {
                        assert!(
                            w <= width,
                            "{emoji:?} at width {width}: a row of {w} columns over a \
                             {width} budget"
                        );
                    }
                }
            }
        }
    }

    /// **The same property for a wide script**, which is the case that was already half
    /// right: CJK graphemes are two columns each and one `char` each, so the old arithmetic
    /// only failed when the first glyph was wider than the row.
    #[test]
    fn a_hard_split_never_leaves_half_a_wide_glyph() {
        let text =
            "\u{65E5}\u{672C}\u{8A9E}\u{306E}\u{30C6}\u{30AD}\u{30B9}\u{30C8}\u{3067}\u{3059}";
        for width in 1..=9 {
            let rows = wrap_line(&plain(text), width);
            let all: String = rows
                .iter()
                .flat_map(|r| r.iter())
                .map(|(t, _)| t.as_str())
                .collect();
            assert_eq!(
                all, text,
                "width {width} dropped or halved a glyph: {all:?}"
            );
        }
    }

    /// **The hang: the hard split did not terminate when the row was narrower than the
    /// first grapheme.**
    ///
    /// `split_at_width` returned the empty string -- a 2-column grapheme does not fit a
    /// 1-column row -- and the loop advanced by `cut.len()`. Zero advance with the condition
    /// still true is not a slow path, it is no path: a probe run on the old code hung past
    /// 60 seconds and had to be killed.
    ///
    /// **This was reachable, not theoretical.** `app/panels.rs:1057` computes the slash
    /// popup's description budget as `inner_width.saturating_sub(1 + 1 + label_col).max(1)`
    /// -- floored at one column on purpose -- and line 1096 passes it straight to
    /// `wrap_line`. A narrow terminal with a long command head, and a description whose
    /// first token starts with a CJK character or an emoji, froze the draw thread.
    #[test]
    fn a_row_narrower_than_its_first_grapheme_still_makes_progress() {
        for emoji in emoji_tokens() {
            // One token, wider than the row, so this is the hard-split arm.
            let text = format!("{emoji}{emoji}{emoji}tail");
            let rows = wrap_line(&plain(&text), 1);
            assert!(!rows.is_empty(), "{emoji:?} at width 1 produced no rows");
            let all: String = rows
                .iter()
                .flat_map(|r| r.iter())
                .map(|(t, _)| t.as_str())
                .collect();
            assert_eq!(
                all, text,
                "{emoji:?} at width 1 lost content while escaping the loop"
            );
        }
        // And the CJK case that was the original trigger.
        let rows = wrap_line(&plain("\u{65E5}\u{672C}"), 1);
        let all: String = rows
            .iter()
            .flat_map(|r| r.iter())
            .map(|(t, _)| t.as_str())
            .collect();
        assert_eq!(all, "\u{65E5}\u{672C}", "width 1 lost a CJK glyph");
    }

    /// **The two width rules, measured where the cut decisions are made.**
    ///
    /// Not a documentation test: `prefix_at_width` is the one function all four cut paths
    /// now share, so a regression here is a regression in every one of them.
    #[test]
    fn a_cut_uses_the_string_width_rule_not_the_sum_of_characters() {
        use unicode_width::UnicodeWidthChar;
        // The family emoji is one grapheme and two columns; its characters sum to eight.
        assert_eq!(FAMILY.width(), 2, "the string rule");
        assert_eq!(
            FAMILY
                .chars()
                .map(|c| UnicodeWidthChar::width(c).unwrap_or(0))
                .sum::<usize>(),
            8,
            "the character rule -- the one that was in use"
        );
        // A budget of two therefore keeps the whole cluster, and a budget of one keeps
        // none of it rather than half of it.
        assert_eq!(prefix_at_width(FAMILY, 2), FAMILY);
        assert_eq!(
            prefix_at_width(FAMILY, 1),
            "",
            "no honest non-empty cut exists"
        );
        // Three clusters: the prefix keeps whole clusters and stops at the boundary.
        let three = format!("{FAMILY}{FAMILY}{FAMILY}");
        assert_eq!(prefix_at_width(&three, 4), format!("{FAMILY}{FAMILY}"));
        assert_eq!(
            prefix_at_width(&three, 3),
            FAMILY,
            "2 fits in 3, the next 2 does not"
        );
    }

    /// The suffix half, which is what a status row's least-important-first half uses.
    #[test]
    fn a_tail_cut_keeps_whole_clusters_from_the_end() {
        let text = format!("prefix{FAMILY}{FAMILY}");
        // Four columns of tail is exactly the two clusters.
        assert_eq!(suffix_at_width(&text, 4), format!("{FAMILY}{FAMILY}"));
        assert_eq!(suffix_at_width(&text, 3), FAMILY);
        assert_eq!(
            suffix_at_width(&text, 1),
            "",
            "a cluster is kept whole or not at all"
        );
        assert_eq!(suffix_at_width(&text, 100), text);
    }

    /// A combining sequence and a skin-tone modifier are each one grapheme, so a cut may
    /// not separate the mark from the base it modifies.
    #[test]
    fn a_cut_never_separates_a_modifier_from_its_base() {
        let combining = "cafe\u{301} au lait";
        for width in 1..=combining.len() {
            let kept = prefix_at_width(combining, width);
            assert!(
                !kept.ends_with('e'),
                "width {width} cut after the base letter and dropped its accent: {kept:?}"
            );
        }
        let thumbs = format!("{THUMB}{THUMB}{THUMB}");
        for width in 1..=6 {
            let kept = prefix_at_width(&thumbs, width);
            assert_eq!(
                kept.graphemes(true).count() * 2,
                crate::ui::layout::display_width(kept),
                "width {width} left part of a skin-tone sequence: {kept:?}"
            );
        }
    }
}
