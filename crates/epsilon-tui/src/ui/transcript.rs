//! How the conversation is presented.
//!
//! The difference between a terminal UI that looks considered and one that looks
//! like a 1980s BBS is almost never the widget toolkit. It is this:
//!
//! | Dated | Considered |
//! |---|---|
//! | A box around every message | One filled band, or a two-column gutter |
//! | `user:` / `assistant:` labels in caps | Position, glyph, and colour carry the role |
//! | Tool calls as JSON on one long line | A card: name, status, duration, then the detail |
//! | Every row the same weight | Speaker bright, payload dim, metadata faint |
//!
//! So: the **gutter** is the primary device. Two columns on the left say who is
//! speaking and, for tools, whether the call succeeded. Colour reinforces it but
//! never carries it alone — a reader on a 16-colour terminal, or with `NO_COLOR`
//! set, must still be able to follow the conversation.

use crate::ui::theme::Theme;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::state::LineKind;

/// Columns reserved at the left of every transcript row.
pub const GUTTER: usize = 2;

/// The glyph and body style for one transcript row.
///
/// Returns `(gutter_glyph, gutter_style, body_style, band)`, where `band` is the
/// background fill (if the terminal has colour to spare).
///
/// # The `has_surfaces` fallback
///
/// `Theme::has_surfaces` promises that a widget relying on a band for legibility
/// falls back to a gutter glyph when the band is unavailable. This is where that
/// promise is kept: off truecolor the two text roles — the answer and the
/// reasoning trace behind it — take ``Glyphs::assistant`` and
/// ``Glyphs::thinking`` instead of the blank gutter they use when a band already
/// says what the row is.
///
/// Without it the pair collapsed on exactly the terminals the promise names. A
/// band is truecolor-only, so nothing else separates them: an assistant row and
/// a thinking row rendered the same two spaces of gutter and the same plain
/// wrapped prose under them. With `NO_COLOR` the styles collapse as well —
/// `Emphasis::Plain` is `Style::default()` there — so on a 16-colour or
/// colourless terminal, "here is the answer" and "here is the model talking to
/// itself" were the same pixels. That is the one collapse the skill singles out
/// by name ("reasoning traces must not become indistinguishable from answers").
pub fn row_style(theme: &Theme, kind: LineKind) -> (String, Style, Style, Style) {
    let glyphs = &theme.glyphs;
    // A band is available only on truecolor (`Theme::surface`), so this is the
    // same question the band itself asks.
    let bands = theme.has_surfaces();
    match kind {
        // The user's own words: a filled band plus a caret glyph, because a
        // question and an answer must be distinguishable at a glance when
        // scrolling back through a long session.
        LineKind::User => (
            glyphs.cursor_bar.to_string(),
            theme.user(),
            theme.text(),
            theme.surface_user(),
        ),
        // The model's prose is the default surface: no marker, no band, while the
        // band is there to say so. See the fallback note above for the other half.
        LineKind::Assistant => (
            if bands {
                " ".to_string()
            } else {
                glyphs.assistant.to_string()
            },
            theme.assistant(),
            theme.assistant(),
            Style::default(),
        ),
        // Reasoning is background material and says so.
        LineKind::Thinking => (
            if bands {
                glyphs.vertical.to_string()
            } else {
                glyphs.thinking.to_string()
            },
            theme.faint(),
            theme.thinking(),
            Style::default(),
        ),
        LineKind::ToolCall => (
            glyphs.tool_call.to_string(),
            theme.warn(),
            theme.warn(),
            theme.surface_pending(),
        ),
        LineKind::ToolResultOk => (
            glyphs.tick.to_string(),
            theme.success(),
            theme.dim(),
            theme.surface_ok(),
        ),
        LineKind::ToolResultErr => (
            glyphs.cross.to_string(),
            theme.danger(),
            theme.danger(),
            theme.surface_err(),
        ),
        // A lifecycle notice is not a tool call. They shared `glyphs.bullet`, so
        // on a terminal with no surfaces — or with `NO_COLOR` set — a pending call
        // and a notice were the same pixels. The bullet is the notice's now; the
        // call owns `glyphs.tool_call`, and the row's own marker repeats it in the
        // body position so the distinction survives at a glance.
        LineKind::System => (
            glyphs.bullet.to_string(),
            theme.faint(),
            theme.faint(),
            Style::default(),
        ),
        // A failure is the one notice that must not be quiet. It takes the
        // cross the tool-error row already owns — the same glyph for the same
        // meaning, which is the point — with the danger colour and the error
        // band. On a terminal with no surfaces, the glyph alone carries it, so
        // the distinction survives `NO_COLOR` and a 16-colour palette alike.
        //
        // Without this, `error: 500 from provider` was drawn with the faintest
        // style on the page, which is the emphasis of something skippable.
        LineKind::SystemErr => (
            glyphs.cross.to_string(),
            theme.danger(),
            theme.danger(),
            theme.surface_err(),
        ),
        // A separator is not content: no glyph (a gutter mark on an empty
        // row is noise), no band, no body style to inherit.
        LineKind::Blank => (
            String::new(),
            Style::default(),
            Style::default(),
            Style::default(),
        ),
        // A card's body nests under its title: the continuation rule, not
        // the result's tick. No band (the title carries the card's state).
        LineKind::CardBody => (
            glyphs.vertical.to_string(),
            theme.dim(),
            theme.dim(),
            Style::default(),
        ),
    }
}

/// Wrap one already-rendered transcript row in its gutter.
///
/// The render cache holds the *body* (markdown expanded, wrapped, styled); this
/// adds the two-column gutter and, where the terminal supports it, the band that
/// runs to the right edge. Doing it here rather than in the cache keeps the cache
/// independent of the theme, so switching colour mode does not invalidate it.
///
/// `width` is the **whole row**, not the body: a band has to reach the terminal
/// edge to read as a block of speech rather than as a highlight on the text, so
/// the fill is `width - gutter - body`. Callers therefore wrap the body to
/// `width - GUTTER` and pass the frame width here; wrapping to `width` and then
/// prefixing two columns is how a transcript row ends up over the edge and wraps.
///
/// This is the production path: `app::draw_transcript` runs every visible row
/// through it. It used to be called only by this module's tests, which is exactly
/// how the gutter and the bands came to be missing from the screen while the
/// suite reported them covered.
pub fn decorate(
    theme: &Theme,
    kind: LineKind,
    body: Vec<Span<'static>>,
    width: usize,
) -> Line<'static> {
    let (glyph, gutter_style, _body_style, band) = row_style(theme, kind);
    let mut spans = Vec::with_capacity(body.len() + 2);
    // Measured, not `format!("{glyph:<GUTTER$}")`: a width counted in `char`s is
    // wrong the moment a repertoire holds a double-width glyph, and the gutter is
    // the one column every row of the transcript aligns on.
    let glyph_width = crate::ui::layout::display_width(&glyph);
    spans.push(Span::styled(
        format!("{glyph}{}", " ".repeat(GUTTER.saturating_sub(glyph_width))),
        gutter_style,
    ));
    spans.extend(body);
    if band != Style::default() {
        // The band has to reach the right edge, or it reads as a highlight on
        // the text rather than as a block of speech.
        let used: usize = spans
            .iter()
            .map(|s| crate::ui::layout::display_width(&s.content))
            .sum();
        if used < width {
            spans.push(Span::styled(" ".repeat(width - used), band));
        }
        // Re-apply the band to every span so the fill is continuous behind the
        // body's own colours.
        for span in spans.iter_mut() {
            span.style = span.style.patch(band);
        }
    }
    Line::from(spans)
}

/// Mark every occurrence of `query` in a transcript row's body spans.
///
/// The search panel has always named the query and counted the matches ("3 of 7")
/// while the transcript showed none of them: a reader had to take the count on
/// trust and scroll for themselves, and the row they were standing on looked
/// exactly like the six others. Pi marks them (`tui-renderer.ts:26-29`), so this
/// does too.
///
/// `current` is the hit the search is on — bold **and** reversed, so it survives
/// a terminal that honours neither colour nor underline, and still points at one
/// hit rather than at all of them.
///
/// The matching is case-insensitive and runs over the spans *as drawn*, after
/// wrapping and markdown expansion. That ordering is deliberate: a query the
/// reader typed matches what they can see, not what the source file said, and it
/// cannot split a word across two spans because a match never spans a span
/// boundary — a miss in that one case is priced at nothing next to highlighting
/// across a styled fragment boundary.
pub fn highlight_matches(
    body: Vec<Span<'static>>,
    query: &str,
    hit: Style,
    current: bool,
) -> Vec<Span<'static>> {
    if query.is_empty() {
        return body;
    }
    // The attributes are the point, and they differ by design: a hit is
    // underlined, the hit you are on is reversed (the bold already comes from
    // `search_strong`'s emphasis, which is bold when there is no colour at all).
    // Two statements, two marks — or "where am I" and "where are they" are the
    // same pixels and the reader has to count.
    let hit = if current {
        hit.add_modifier(Modifier::REVERSED)
    } else {
        hit.add_modifier(Modifier::UNDERLINED)
    };
    let needle: Vec<char> = query.to_lowercase().chars().collect();
    let mut out = Vec::with_capacity(body.len());
    for span in body {
        let (haystack, starts) = fold_case(&span.content);
        if haystack.is_empty() {
            out.push(span);
            continue;
        }
        // `at` is a **char** index into the folded copy, which is how `find_run`
        // counts. A folded copy indexed by bytes would make the two disagree for
        // a character whose lowercase is longer than itself (U+0130 folds to
        // two), and a byte offset read as a char index is an off-by-one that
        // marks the wrong letters.
        let mut at = 0usize;
        let mut pushed = false;
        while let Some(found) = find_run(&haystack[at..], &needle) {
            let start_c = at + found;
            let end_c = start_c + needle.len();
            // Back to byte offsets in the ORIGINAL string: `fold_case` records
            // where every folded character came from, so a match is cut on the
            // original's char boundaries rather than on the folded copy's.
            let start_b = starts[start_c];
            let end_b = starts[end_c - 1]
                + span.content[start_b..]
                    .chars()
                    .next()
                    .map(char::len_utf8)
                    .unwrap_or(0);
            if start_b > at {
                out.push(Span::styled(
                    span.content[at..start_b].to_string(),
                    span.style,
                ));
            }
            out.push(Span::styled(
                span.content[start_b..end_b].to_string(),
                span.style.patch(hit),
            ));
            at = end_c;
            pushed = true;
        }
        if !pushed {
            out.push(span);
        } else if at < span.content.len() {
            out.push(Span::styled(span.content[at..].to_string(), span.style));
        }
    }
    out
}

/// The char index of `needle` inside `haystack`, if it is there.
fn find_run(haystack: &[char], needle: &[char]) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// The string lowercased char by char, plus the byte offset each folded
/// character came from — so a match found in the folded copy can be cut out of
/// the original without ever landing inside a character.
fn fold_case(text: &str) -> (Vec<char>, Vec<usize>) {
    let mut folded = Vec::with_capacity(text.len());
    let mut starts = Vec::with_capacity(text.len());
    for (offset, ch) in text.char_indices() {
        for lower in ch.to_lowercase() {
            folded.push(lower);
            starts.push(offset);
        }
    }
    (folded, starts)
}

/// The first-run screen.
///
/// A blank transcript on launch is the single worst first impression a TUI can
/// make: the user cannot tell whether the program is working, waiting, or broken.
/// This says what the harness is and what to type — and it disappears the moment
/// there is a real conversation to show.
///
/// **What it deliberately does not do is list the keybindings.** It used to, and that was
/// the design failure worth recording: the footer already renders the *live* key set for the
/// current state, three lines below, so the block above was a second, staler copy of the same
/// information. Two copies of a help listing, one of them wrong within seconds of the first
/// keystroke, reads as noise and pushes the composer off the screen.
///
/// **It does not restate the header either, and that is the second version of the
/// same mistake.** It carried a `model`/`context` panel — real values, correctly
/// derived — two rows below a header that already prints both, on a screen with
/// twenty blank rows underneath them. Two surfaces stating one fact do not
/// reinforce each other; they make the reader decide which to believe, and they
/// degrade on different rules (the header drops whole gauges as the row narrows,
/// the panel never did). The header is the owner because it states them in *every*
/// state, including after the first message lands and this panel is gone.
///
/// So the space goes to what no other row says: what to do, and the three prefixes
/// the composer reads. One left edge, and a body weight for the instruction rather
/// than the same faint style as the decoration around it.
pub fn welcome(theme: &Theme, session_label: &str) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();
    // A plain, wide-tracked wordmark. Not ASCII art: art breaks on a narrow
    // terminal and reads as noise on a re-read.
    let tracked = "EPSILON"
        .chars()
        .map(|c| c.to_string())
        .collect::<Vec<_>>()
        .join(" ");
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(tracked, theme.accent())));
    lines.push(Line::from(Span::styled(
        "a coding agent in your terminal",
        theme.dim(),
    )));
    lines.push(Line::from(""));
    // The one instruction on the screen, and the only sentence drawn in a body
    // weight. An instruction rendered in the same style as the data above it is
    // not a call to action, it is another datum.
    lines.push(Line::from(vec![
        Span::styled("Describe a task", theme.dim_strong()),
        Span::styled(", then press ", theme.dim()),
        Span::styled("enter", theme.dim_strong()),
        Span::styled(".", theme.dim()),
    ]));
    // The three prefixes the composer reads (`app.rs` dispatches all three), which
    // no other row on screen states.
    lines.push(Line::from(vec![
        Span::styled("@", theme.dim_strong()),
        Span::styled(" files · ", theme.faint()),
        Span::styled("!", theme.dim_strong()),
        Span::styled(" shell · ", theme.faint()),
        Span::styled("/", theme.dim_strong()),
        Span::styled(" commands", theme.faint()),
    ]));
    // The one fact here that the header cannot state. An empty label means
    // "unnamed", and the row is dropped rather than printed blank.
    if !session_label.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled("session  ", theme.faint()),
            Span::styled(session_label.to_string(), theme.dim()),
        ]));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::theme::{ASCII, ColorMode, UNICODE};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    /// What the highlight did, as the reader would see it: the spans' text and
    /// whether each carries the mark. The style itself is the caller's to choose,
    /// so the tests assert the *shape* — which runs are hit runs — and the frame
    /// test in `app/golden.rs` asserts the style.
    fn runs(body: Vec<Span<'static>>, query: &str) -> Vec<(String, bool)> {
        let theme = Theme::new(ColorMode::Truecolor, UNICODE);
        let hit = theme.search();
        highlight_matches(body, query, hit, false)
            .into_iter()
            .map(|s| {
                let marked = s
                    .style
                    .add_modifier
                    .contains(ratatui::style::Modifier::UNDERLINED);
                (s.content.to_string(), marked)
            })
            .collect()
    }

    #[test]
    fn a_hit_is_marked_and_the_rest_of_the_row_is_not() {
        let spans = vec![Span::styled("plain needle tail", Style::default())];
        assert_eq!(
            runs(spans, "needle"),
            vec![
                ("plain ".to_string(), false),
                ("needle".to_string(), true),
                (" tail".to_string(), false),
            ]
        );
    }

    #[test]
    fn matching_ignores_case_and_repeats_without_overlap() {
        let spans = vec![Span::styled(
            "Needle then needle then NEEDLE",
            Style::default(),
        )];
        let out = runs(spans, "needle");
        assert_eq!(
            out.iter().filter(|(_, m)| *m).count(),
            3,
            "all three, whatever their case: {out:?}"
        );
        assert_eq!(
            out.iter().map(|(t, _)| t.as_str()).collect::<String>(),
            "Needle then needle then NEEDLE",
            "and the text is unchanged: {out:?}"
        );
    }

    #[test]
    fn a_match_split_across_two_styled_spans_is_priced_not_marked() {
        // The row is styled per fragment, and a run that starts in one span and
        // ends in the next is not found: matching inside a span keeps the
        // fragment's own style on the marked run, and matching across one would
        // mean re-styling a neighbour's text to draw a hit. The price is a miss
        // on a word the styling happened to cut in half — which the reader still
        // sees, unmarked, and can find by searching for a shorter fragment.
        let spans = vec![
            Span::styled("nee", Style::default()),
            Span::styled("dle here", Style::default()),
        ];
        assert_eq!(
            runs(spans, "needle"),
            vec![("nee".to_string(), false), ("dle here".to_string(), false)],
            "nothing marked, and nothing corrupted"
        );
    }

    #[test]
    fn characters_whose_lowercase_is_not_one_char_survive_the_fold() {
        // U+0130 lowercases to two characters, so the folded copy is longer than
        // the original and a byte offset taken from it would land inside a
        // character. The row must come back whole, not panic.
        let spans = vec![Span::styled("İ needle İ", Style::default())];
        let out = runs(spans, "needle");
        assert_eq!(
            out.iter().map(|(t, _)| t.as_str()).collect::<String>(),
            "İ needle İ",
            "the text round-trips: {out:?}"
        );
        assert!(out.iter().any(|(t, m)| t == "needle" && *m), "{out:?}");
    }

    #[test]
    fn no_match_and_no_query_leave_the_row_exactly_as_it_was() {
        let spans = vec![Span::styled("nothing to find", Style::default())];
        assert_eq!(
            runs(spans.clone(), "elsewhere"),
            vec![("nothing to find".to_string(), false)]
        );
        assert_eq!(
            runs(spans, ""),
            vec![("nothing to find".to_string(), false)],
            "a blank query is not a query that matches everywhere"
        );
    }

    #[test]
    fn a_hit_span_keeps_the_style_it_was_drawn_with() {
        // The mark is *added to* the row's own style, not substituted for it: a
        // hit inside a thinking trace stays dim as well as underlined.
        let dim = Style::default().add_modifier(ratatui::style::Modifier::DIM);
        let spans = vec![Span::styled("reasoning needle here", dim)];
        let theme = Theme::new(ColorMode::Truecolor, UNICODE);
        let out = highlight_matches(spans, "needle", theme.search(), false);
        let marked = out.iter().find(|s| s.content.contains("needle")).unwrap();
        assert!(
            marked
                .style
                .add_modifier
                .contains(ratatui::style::Modifier::DIM),
            "the row's own emphasis survives the mark: {:?}",
            marked.style
        );
        assert!(
            marked
                .style
                .add_modifier
                .contains(ratatui::style::Modifier::UNDERLINED),
            "and the mark is there: {:?}",
            marked.style
        );
        assert_eq!(marked.style.fg, Some(theme.search().fg.unwrap()));
    }

    fn screen(lines: Vec<Line<'static>>, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("test terminal");
        terminal
            .draw(|f| {
                f.render_widget(ratatui::widgets::Paragraph::new(lines), f.area());
            })
            .expect("draw");
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol().chars().next().unwrap_or(' '))
            .collect()
    }

    #[test]
    fn every_role_is_distinguishable_without_colour() {
        // The acid test: colour off, ASCII glyphs. Each role must still produce a
        // different left margin.
        let theme = Theme::new(ColorMode::None, ASCII);
        let mut gutters = Vec::new();
        for kind in [
            LineKind::User,
            LineKind::Assistant,
            LineKind::Thinking,
            LineKind::ToolCall,
            LineKind::ToolResultOk,
            LineKind::ToolResultErr,
            LineKind::System,
            // P-058: a failure notice. It shares the cross with the failed tool
            // result **on purpose** — same glyph, same meaning — so the pair this
            // loop must now catch is failure-notice against plain notice, which
            // is the pair that was indistinguishable before this row existed.
            LineKind::SystemErr,
            LineKind::Blank,
            LineKind::CardBody,
        ] {
            let (glyph, _, _, _) = row_style(&theme, kind);
            gutters.push((kind, glyph));
        }
        // Every role has a marker of its own here, including the two that have
        // none while a band can carry the distinction: with no colour and no
        // band, a marker is the only thing left. The call and the notice are the
        // pair that used to share `*`, and the answer and the reasoning trace are
        // the pair that used to share a blank gutter.
        assert_eq!(gutters[0].1, ">"); // user
        assert_eq!(gutters[1].1, "\""); // assistant prose: the band's fallback
        assert_eq!(gutters[2].1, "="); // thinking: the band's fallback
        assert_eq!(gutters[3].1, "\\"); // tool call
        assert_eq!(gutters[4].1, "+"); // ok
        assert_eq!(gutters[5].1, "x"); // error
        assert_eq!(gutters[6].1, "*"); // system notice
        assert_eq!(gutters[7].1, "x"); // failure notice
        assert_ne!(
            gutters[7].1, gutters[6].1,
            "a failure and an ordinary notice must not be the same pixels: the \
             failure is the one that needs the emphasis"
        );
        assert_eq!(gutters[8].1, ""); // blank separator: no gutter mark
        assert_eq!(gutters[9].1, "|"); // card body: the continuation rule
        // The pair the presentation pass is about: a title and its output
        // must never share a gutter again.
        assert_ne!(gutters[3].1, gutters[8].1);
        assert_ne!(gutters[3].1, gutters[6].1);
        // The pair this whole change is about, called out by name so a failure
        // here says which two roles collapsed rather than just "some pair did".
        assert_ne!(
            gutters[1].1, gutters[2].1,
            "the answer and the reasoning trace must not be the same pixels"
        );
        // **And the pair this test used to assert was equal.** It named the collision
        // ("the pair that used to share `*`") and then pinned both roles to `"*"` —
        // so the one assertion in a test called *every role is distinguishable*
        // that would have caught the 2026-10-04 report was written backwards. A named
        // past collision is a claim that it cannot recur, and asserting equality is
        // the opposite of that claim.
        assert_ne!(
            gutters[1].1, gutters[6].1,
            "the model's answer and a harness notice must not be the same pixels: \
             a reader cannot tell a reply from a status line without colour"
        );
    }

    #[test]
    fn the_bandless_gutter_is_the_fallback_the_theme_documents() {
        // `Theme::has_surfaces` promises that a widget relying on a band falls
        // back to a gutter glyph. The promise is only worth anything if it is
        // observable on the pair that needs it: the answer and the reasoning
        // trace are identical rows on a truecolor terminal *except* for the band,
        // so if the marker is not the fallback they are identical everywhere.
        let rich = Theme::new(ColorMode::Truecolor, UNICODE);
        let plain = Theme::new(ColorMode::Ansi16, ASCII);
        assert!(rich.has_surfaces());
        assert!(!plain.has_surfaces());

        let assistant_rich = row_style(&rich, LineKind::Assistant).0;
        let assistant_plain = row_style(&plain, LineKind::Assistant).0;
        let thinking_rich = row_style(&rich, LineKind::Thinking).0;
        let thinking_plain = row_style(&plain, LineKind::Thinking).0;

        assert_ne!(
            assistant_plain, assistant_rich,
            "the fallback must be a different glyph, not the same one"
        );
        assert_ne!(thinking_plain, thinking_rich);
        for (name, plain_glyph) in [
            ("assistant", &assistant_plain),
            ("thinking", &thinking_plain),
        ] {
            assert!(
                !plain_glyph.trim().is_empty(),
                "{name} has no marker without a band: {plain_glyph:?}"
            );
        }
        // And the fallback is what actually reaches a screen, through `decorate`
        // — the function the draw path calls.
        let line = decorate(&plain, LineKind::Assistant, vec![Span::raw("body")], 40);
        let head = line.spans.first().expect("gutter span").content.clone();
        assert!(
            head.starts_with(assistant_plain.as_str()),
            "decorate drew {head:?}, not the fallback marker"
        );
    }

    #[test]
    fn the_gutter_is_exactly_two_columns_for_every_role() {
        for theme in [
            Theme::new(ColorMode::Truecolor, UNICODE),
            Theme::new(ColorMode::None, ASCII),
        ] {
            for kind in [
                LineKind::User,
                LineKind::Assistant,
                LineKind::Thinking,
                LineKind::ToolCall,
                LineKind::ToolResultOk,
                LineKind::ToolResultErr,
                LineKind::System,
                LineKind::CardBody,
            ] {
                let line = decorate(&theme, kind, vec![Span::raw("body")], 40);
                let first = line.spans.first().expect("gutter span");
                assert_eq!(
                    crate::ui::layout::display_width(&first.content),
                    GUTTER,
                    "gutter width drifted for {kind:?}"
                );
                // Body text starts at exactly GUTTER for every role, which is
                // what makes a conversation read as two aligned columns.
                let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
                let body: String = text.chars().skip(GUTTER).collect();
                assert!(body.starts_with("body"), "{kind:?}: {text:?}");
            }
        }
    }

    #[test]
    fn a_banded_row_fills_to_the_right_edge() {
        let theme = Theme::new(ColorMode::Truecolor, UNICODE);
        let line = decorate(&theme, LineKind::User, vec![Span::raw("hi")], 20);
        let total: usize = line
            .spans
            .iter()
            .map(|s| crate::ui::layout::display_width(&s.content))
            .sum();
        assert_eq!(total, 20, "band did not reach the edge");
    }

    #[test]
    fn a_plain_row_is_not_padded_with_invisible_fill() {
        // Only banded roles pad: padding every row would write cells for nothing
        // and make `NO_COLOR` output differ from coloured output for no reason.
        let theme = Theme::new(ColorMode::None, ASCII);
        let line = decorate(&theme, LineKind::Assistant, vec![Span::raw("hi")], 20);
        let total: usize = line
            .spans
            .iter()
            .map(|s| crate::ui::layout::display_width(&s.content))
            .sum();
        assert_eq!(total, GUTTER + 2);
    }

    #[test]
    fn the_welcome_screen_is_not_a_blank_void() {
        let theme = Theme::new(ColorMode::Ansi16, ASCII);
        let text = screen(welcome(&theme, ""), 80, 20);
        assert!(text.contains("E P S I L O N"), "{text}");
        assert!(text.contains("Describe a task"), "{text}");
        assert!(text.contains("press enter"), "{text}");
        // The three prefixes the composer reads, which no other row states.
        for prefix in ["@ files", "! shell", "/ commands"] {
            assert!(text.contains(prefix), "missing {prefix:?}: {text}");
        }
    }

    /// **The welcome panel does not restate the header.**
    ///
    /// It used to: `model Faux (offline scripted)` and `?/1.0M` appeared in the
    /// header *and* in the body two rows below, on a screen with twenty blank rows
    /// under them. The first thing a new user saw spent its whole budget repeating
    /// itself, and the two copies degrade on different rules — the header drops
    /// whole gauges as the row narrows and the panel never did.
    ///
    /// The header owns the model and the context because it states them in every
    /// state; this asserts the panel stays out of it, by rendering with a metrics
    /// value whose *label* the panel could not have invented and checking the panel
    /// never prints it.
    #[test]
    fn the_welcome_screen_does_not_restate_the_header() {
        let theme = Theme::new(ColorMode::Ansi16, ASCII);
        let text = screen(welcome(&theme, ""), 80, 20);
        for restated in ["model", "context", "Faux"] {
            assert!(
                !text.contains(restated),
                "the panel repeats the header's {restated:?}: {text}"
            );
        }
    }

    /// The footer already renders the live key set for the current state. A welcome screen
    /// that repeats it produces two help listings, the upper one stale within seconds of the
    /// first keystroke, and pushes the composer off a short window. This is the regression test
    /// for the design change, not a formatting preference.
    #[test]
    fn the_welcome_screen_does_not_repeat_the_footers_keybindings() {
        let theme = Theme::new(ColorMode::Ansi16, ASCII);
        let text = screen(welcome(&theme, ""), 80, 20);
        for key in ["ctrl-p", "pageup scroll", "command palette"] {
            assert!(
                !text.contains(key),
                "the welcome screen must not restate the footer's live keys ({key:?}): {text}"
            );
        }
    }

    /// A session label is optional; an empty one drops the row instead of printing a blank
    /// field, because a row reading "session" with nothing after it looks like a bug.
    #[test]
    fn a_session_label_appears_only_when_there_is_one() {
        let theme = Theme::new(ColorMode::Ansi16, ASCII);
        let named = screen(welcome(&theme, "refactor-rates"), 80, 20);
        assert!(named.contains("refactor-rates"), "{named}");
        let anon = screen(welcome(&theme, ""), 80, 20);
        assert!(!anon.contains("session"), "{anon}");
    }

    /// Every row of the panel shares one left edge.
    ///
    /// It used to have three: the wordmark and the tagline at column 1, then
    /// `model`/`context` at column 3, then the instruction at column 3 in the same
    /// style as the data above it. Three edges and no hierarchy is a wall of text
    /// that happens to be aligned; the panel now reads top-down as identity,
    /// instruction, affordances, session.
    #[test]
    fn every_welcome_row_shares_one_left_edge() {
        let theme = Theme::new(ColorMode::Ansi16, ASCII);
        let flat = screen(welcome(&theme, "cli"), 80, 20);
        // `screen` returns the cells in row-major order with no separators, which
        // is what every other test in this file wants; this one is about rows, so
        // it slices them back out.
        let rows: Vec<String> = flat
            .chars()
            .collect::<Vec<_>>()
            .chunks(80)
            .map(|c| c.iter().collect::<String>())
            .collect();
        let mut checked = 0;
        for row in rows.iter().filter(|r| !r.trim().is_empty()) {
            checked += 1;
            assert_eq!(
                row.len(),
                row.trim_start().len(),
                "a welcome row is indented away from the others: {row:?}"
            );
        }
        assert!(checked >= 4, "the panel lost rows: {rows:#?}");
    }
}
