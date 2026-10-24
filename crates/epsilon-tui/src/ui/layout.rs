//! The screen's spatial model: where each region sits, and how it degrades.
//!
//! A terminal UI is legible because the reader builds a map of it once and then
//! trusts that map. That means the regions have to be decided in **one** place,
//! in a fixed order, and every draw has to rebuild from `frame.area()` — never
//! from coordinates remembered last frame, which go stale the moment the user
//! drags a window edge.
//!
//! ```text
//! ┌──────────────────────────────────────────────────────────────┐
//! │ status ···················· metrics (model, tokens, cache…)  │  header
//! │                                                              │
//! │ transcript — conversation, tool cards, notices               │  body (fills)
//! │                                                              │  ┐
//! ├──────────────────────────────────────────────────────────────┤  ┘ one blank row
//! │ ❯ composer                                                   │  1..=8
//! │ keys that work right now                     location · model│  footer
//! └──────────────────────────────────────────────────────────────┘
//! ```
//!
//! **One rule, not two.** It sits above the composer and means "everything below
//! this line takes keystrokes". There used to be a second one between the header
//! and the transcript, drawn by the same function in this same style, dividing
//! nothing the reader could name while costing a row of conversation at every
//! size — two of twenty-four rows at 80×24. One fold says where the transcript
//! stops; two say it twice.
//!
//! **The blank above the composer is not that rule brought back.** The rule
//! divided with a line; the blank *is* the row `push_line` already puts between
//! every block, so the last answer ends the way each block before it did. It
//! exists because with the transcript running straight into the composer, the
//! composer read as one more transcript row — its prompt glyph is shared with a
//! submitted turn on purpose, so nothing on screen said where the conversation
//! stopped and the input line looked like the answer's last line.
//!
//! Two invariants keep this honest as the window narrows:
//!
//! * **Chrome yields before content.** The composer gives up rows first, then
//!   the footer's optional half, then the header's metrics. The transcript is
//!   the last thing to lose space, because it is the only region the user came
//!   for.
//! * **Below the floor, say so.** A layout squeezed under [`MIN_WIDTH`] ×
//!   [`MIN_HEIGHT`] cannot show a conversation; it shows one centered line
//!   instead of a mangled frame.

use ratatui::layout::{Constraint, Layout, Rect};

/// Below this width the layout stops trying to be a layout.
pub const MIN_WIDTH: u16 = 44;
/// Below this height the layout stops trying to be a layout.
pub const MIN_HEIGHT: u16 = 10;
/// The composer never grows past this many text rows.
pub const COMPOSER_MAX_ROWS: u16 = 8;
/// The composer never takes more than this fraction of the screen.
pub const COMPOSER_MAX_SHARE: u16 = 3;
/// Rows the chrome always owns: the header, the blank that ends the
/// conversation, the composer's rule, and the footer. The footer is counted
/// with them because it is never optional either.
///
/// This was 3 while the transcript ran straight into the composer, and 4 while
/// the transcript had a rule of its own; the second rule is gone and the blank
/// is not it. A row came back because a bare join made the composer read as one
/// more transcript row — the prompt glyph is shared with a submitted turn on
/// purpose (`composer.rs`), so nothing on screen said where the conversation
/// stopped. The constant sits here so the arithmetic and the tests cannot drift
/// apart.
pub const CHROME_ROWS: u16 = 4;

/// The regions of one frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Regions {
    /// Status on the left, live metrics on the right.
    pub header: Rect,
    /// The conversation. Ends one row above [`Regions::composer`]: the row
    /// between them is [`Regions::gap`].
    pub transcript: Rect,
    /// The blank row that ends the conversation.
    ///
    /// Drawn blank rather than as a rule because the transcript's own block
    /// language is a blank row (`push_line`), so the last answer ends the way
    /// every block before it did instead of touching the input. A rule here is
    /// the second divider the layout still declines to draw.
    pub gap: Rect,
    /// Where the user types.
    pub composer: Rect,
    /// The divider between the composer and the key hints — and, read from the
    /// other side, the line that says where the transcript stops.
    pub composer_rule: Rect,
    /// Keys that work right now, and where the session lives.
    pub footer: Rect,
}

impl Regions {
    /// Split a frame.
    ///
    /// `composer_rows` is the number of *text* rows the draft needs, already
    /// wrapped by the caller: the layout decides how much of that it can afford,
    /// because only the layout knows the screen height.
    pub fn split(area: Rect, composer_rows: u16) -> Self {
        // Chrome first: the header, the blank that ends the conversation, the
        // composer's rule and the footer, plus the composer's own rows.
        let composer = composer_rows.clamp(1, Self::composer_cap(area.height));
        // `Min(1)` rather than `Fill`: with `Fill` a very short screen hands the
        // transcript a zero-height rectangle and the body silently disappears.
        let [
            header,
            transcript,
            gap,
            composer_area,
            composer_rule,
            footer,
        ] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
            Constraint::Length(composer),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .areas(area);
        Self {
            header,
            transcript,
            gap,
            composer: composer_area,
            composer_rule,
            footer,
        }
    }

    /// How many composer rows this screen can afford.
    ///
    /// A third of the height is the ceiling from the design brief; the absolute
    /// cap keeps a paste from turning the composer into the whole screen.
    pub fn composer_cap(height: u16) -> u16 {
        let share = (height / COMPOSER_MAX_SHARE).max(1);
        COMPOSER_MAX_ROWS.min(share)
    }

    /// Whether the screen is too small to show a conversation at all.
    pub fn too_small(area: Rect) -> bool {
        area.width < MIN_WIDTH || area.height < MIN_HEIGHT
    }
}

/// The message shown when the terminal cannot hold the layout.
pub fn too_small_message(width: u16, height: u16) -> String {
    format!("terminal too small — widen to {MIN_WIDTH}×{MIN_HEIGHT} (now {width}×{height})")
}

/// The narrowest a docked project explorer is useful at.
pub const EXPLORER_MIN_WIDTH: u16 = 24;
/// The widest the explorer may grow before it starts crowding the workspace.
pub const EXPLORER_MAX_WIDTH: u16 = 38;
/// The explorer's preferred share of the frame, in percent.
pub const EXPLORER_SHARE_PERCENT: u16 = 30;
/// One blank column between the workspace and the explorer's frame.
pub const EXPLORER_GAP: u16 = 1;

/// The width a docked explorer asks for at a given frame width.
///
/// Derived from the frame rather than fixed, like every other region: a sidebar
/// that keeps 38 columns on a 72-column terminal is half the application.
pub fn explorer_width(frame_width: u16) -> u16 {
    (frame_width * EXPLORER_SHARE_PERCENT / 100).clamp(EXPLORER_MIN_WIDTH, EXPLORER_MAX_WIDTH)
}

/// Where the workspace and the docked project explorer sit.
///
/// The explorer is on the **right**, because the agent transcript is the primary
/// reading surface and the eye returns to it after a glance at the tree. It
/// docks only when the workspace can still keep [`MIN_WIDTH`] columns; below
/// that the caller draws the explorer as a focused full-area panel instead of
/// squeezing both surfaces into an unreadable frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Explorer {
    /// The workspace: header, transcript, composer and footer.
    pub main: Rect,
    /// The docked explorer, or `None` when it is hidden or does not fit.
    pub tree: Option<Rect>,
}

impl Explorer {
    /// Split `area` for a visible or hidden explorer.
    pub fn split(area: Rect, visible: bool) -> Self {
        if !visible {
            return Self {
                main: area,
                tree: None,
            };
        }
        let width = explorer_width(area.width);
        if area.width < width + EXPLORER_GAP + MIN_WIDTH {
            return Self {
                main: area,
                tree: None,
            };
        }
        let [main, gap, tree] = Layout::horizontal([
            Constraint::Min(MIN_WIDTH),
            Constraint::Length(EXPLORER_GAP),
            Constraint::Length(width),
        ])
        .areas(area);
        // The gap is a real, blank column rather than a second border: two frames
        // touching read as one thick line and make the panel boundary ambiguous.
        debug_assert_eq!(gap.width, EXPLORER_GAP);
        Self {
            main,
            tree: Some(tree),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::text::Span;

    fn area(width: u16, height: u16) -> Rect {
        Rect::new(0, 0, width, height)
    }

    #[test]
    fn regions_tile_the_frame_without_gaps_or_overlap() {
        for (w, h) in [(44, 10), (80, 24), (100, 30), (160, 48)] {
            let r = Regions::split(area(w, h), 1);
            // header + transcript + the ending blank + composer + ONE rule +
            // footer. The transcript *rule* was removed deliberately; the blank
            // is a different thing and this is the arithmetic that proves its
            // row is accounted for rather than going missing.
            let expected = 1 + r.transcript.height + r.gap.height + r.composer.height + 1 + 1;
            assert_eq!(expected, h, "{w}x{h} left {expected} rows unaccounted");
            assert_eq!(r.header.y, 0);
            assert_eq!(r.transcript.y, r.header.bottom());
            assert_eq!(
                r.gap.y,
                r.transcript.bottom(),
                "{w}x{h}: the blank ends the conversation where the transcript stops"
            );
            assert_eq!(
                r.gap.bottom(),
                r.composer.y,
                "{w}x{h}: the composer must start below the ending blank"
            );
            assert_eq!(
                r.composer_rule.y,
                r.composer.bottom(),
                "{w}x{h}: the one rule sits below the composer, not above it"
            );
            assert_eq!(r.footer.y + 1, h, "footer must sit on the last row");
            assert_eq!(r.gap.height, 1, "the ending blank is exactly one row");
            assert_eq!(r.header.width, w);
            assert_eq!(r.gap.width, w);
        }
    }

    #[test]
    fn the_blank_that_ends_the_conversation_is_one_row() {
        // Two rows would be the same trade the removed rule made — a taller
        // chrome budget paid out of the conversation — and zero rows is the join
        // this band exists to end. One row is the middle the frames rely on.
        for (w, h) in [(44, 10), (80, 24), (100, 30), (160, 48)] {
            for composer_rows in [1u16, 3, 8] {
                let r = Regions::split(area(w, h), composer_rows);
                assert_eq!(
                    r.gap.height, 1,
                    "{w}x{h} with {composer_rows} composer rows"
                );
                assert!(
                    r.transcript.height >= 1,
                    "{w}x{h} with {composer_rows} composer rows gave the transcript {}",
                    r.transcript.height
                );
            }
        }
    }

    #[test]
    fn the_chrome_budget_is_spent_on_header_blank_rule_and_footer() {
        // The reviewable evidence that the arithmetic here and the frame agree:
        // the transcript gets exactly the rows the chrome budget leaves it, at
        // every supported size, whatever the composer asked for.
        for (w, h) in [(44, 10), (80, 24), (100, 30), (160, 48)] {
            let r = Regions::split(area(w, h), 1);
            assert_eq!(
                r.transcript.height,
                h - CHROME_ROWS - r.composer.height,
                "{w}x{h}: transcript height no longer matches the chrome budget"
            );
        }
        assert_eq!(CHROME_ROWS, 4, "header, ending blank, rule, footer");
    }

    #[test]
    fn the_transcript_keeps_the_space_it_needs_at_every_supported_size() {
        for (w, h) in [(44, 10), (60, 20), (80, 24), (120, 30)] {
            let r = Regions::split(area(w, h), 8);
            assert!(
                r.transcript.height >= 1,
                "{w}x{h} gave the transcript {} rows",
                r.transcript.height
            );
        }
    }

    #[test]
    fn the_composer_never_takes_the_screen() {
        // A pasted document must not push the conversation off the screen.
        let r = Regions::split(area(100, 30), 400);
        assert!(r.composer.height <= COMPOSER_MAX_ROWS);
        assert!(
            r.transcript.height >= 30 - CHROME_ROWS - COMPOSER_MAX_ROWS,
            "transcript lost rows to a paste: {}",
            r.transcript.height
        );
        // Nor may it take a third of a short screen.
        let small = Regions::split(area(80, 12), 400);
        assert!(
            small.composer.height <= 12 / COMPOSER_MAX_SHARE,
            "{}",
            small.composer.height
        );
    }

    #[test]
    fn the_docked_explorer_stays_on_the_right_and_preserves_the_workspace_floor() {
        for width in [100u16, 120, 160, 200] {
            let split = Explorer::split(area(width, 30), true);
            let tree = split.tree.expect("a wide frame docks the explorer");
            assert_eq!(
                tree.right(),
                width,
                "{width}: explorer must touch the right edge"
            );
            assert!(
                tree.x > split.main.right(),
                "{width}: explorer must sit right of the workspace"
            );
            assert_eq!(
                tree.x - split.main.right(),
                EXPLORER_GAP,
                "{width}: exactly one gutter column"
            );
            assert!(
                split.main.width >= MIN_WIDTH,
                "{width}: workspace fell below its floor: {}",
                split.main.width
            );
            assert!(
                (EXPLORER_MIN_WIDTH..=EXPLORER_MAX_WIDTH).contains(&tree.width),
                "{width}: explorer width {} outside its band",
                tree.width
            );
        }
    }

    #[test]
    fn a_hidden_or_narrow_explorer_does_not_take_columns_from_the_workspace() {
        assert_eq!(
            Explorer::split(area(120, 30), false),
            Explorer {
                main: area(120, 30),
                tree: None
            }
        );
        for width in [44u16, 60, 68] {
            let split = Explorer::split(area(width, 24), true);
            assert_eq!(split.main, area(width, 24), "{width}: no room to dock");
            assert!(
                split.tree.is_none(),
                "{width}: must not squeeze the workspace"
            );
        }
    }

    #[test]
    fn the_floor_is_reported_rather_than_guessed() {
        assert!(Regions::too_small(area(43, 30)));
        assert!(Regions::too_small(area(100, 9)));
        assert!(!Regions::too_small(area(44, 10)));
        let message = too_small_message(30, 8);
        assert!(message.contains("44×10"), "{message}");
        assert!(message.contains("30×8"), "{message}");
    }

    /// One half of a status row, built the way a caller builds it.
    ///
    /// The parts are separate spans on purpose: both real callers emit a value and
    /// its separators as different spans, and a test that merged them into one
    /// string would pass against a layout that cuts inside a value.
    fn halves(
        location: &'static str,
    ) -> (
        Vec<ratatui::text::Span<'static>>,
        Vec<ratatui::text::Span<'static>>,
    ) {
        (
            vec![
                Span::raw("insert"),
                Span::raw(" "),
                Span::raw("^ 12/40"),
                Span::raw(" · "),
                Span::raw("g jump to live"),
            ],
            vec![Span::raw(location)],
        )
    }

    #[test]
    fn the_right_half_keeps_its_whole_columns_and_the_left_yields() {
        // `Priority::Right`: the location is a safety signal, so the hints are the
        // half that gives way. What this pins is the *arithmetic* — the row is
        // exactly as wide as it was given, and the gap is never spent — because
        // the policy about which parts survive belongs to the caller that knows
        // what the parts mean (see `footer::draw` and `header::metric_spans`).
        let location = "~/api · main";
        for width in [18usize, 27, 40, 80] {
            let (left, right) = halves(location);
            let line = side_by_side(left, right, width, Priority::Right);
            let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
            assert!(
                display_width(&text) <= width,
                "width {width} overflowed: {text:?}"
            );
            if let Some(at) = text.find(location) {
                assert!(at > 0, "width {width}: nothing survived on the left");
                assert_eq!(
                    &text[at - 1..at],
                    " ",
                    "width {width}: the halves touch: {text:?}"
                );
            }
        }
    }

    /// **The status rows cut on grapheme boundaries now, and they did not.**
    ///
    /// `cut` and `cut_keeping_tail` walked `char`s summing `UnicodeWidthChar::width`, which
    /// is the arithmetic that splits an emoji ZWJ sequence — the per-character sum for a
    /// family emoji is eight against its real two. A header or footer carrying a CJK file
    /// name, or a model name with an emoji in it, could therefore come apart mid-cluster.
    ///
    /// The property asserted here is the one that makes the row *correct* rather than merely
    /// present: every grapheme in the survivor is a whole grapheme from the input, in order,
    /// and the survivor is never wider than the budget it was given.
    #[test]
    fn a_status_row_cut_keeps_whole_grapheme_clusters_and_stays_in_budget() {
        use super::{cut, cut_keeping_tail};
        use unicode_segmentation::UnicodeSegmentation;
        use unicode_width::UnicodeWidthStr;

        let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}\u{200D}\u{1F466}";
        let content = format!("{family}{family}\u{65E5}\u{672C}\u{8A9E} tail");
        let span = Span::raw(content.clone());
        let graphemes: Vec<&str> = content.graphemes(true).collect();
        let widest = UnicodeWidthStr::width(content.as_str());

        for width in 1..=widest {
            let head: String = cut(vec![span.clone()], width)
                .iter()
                .map(|s| s.content.to_string())
                .collect();
            // The head must be a **prefix** of the input's grapheme sequence: whole
            // graphemes, in order, and nothing invented. Comparing against the whole
            // sequence instead of a prefix of it reports every non-empty cut as a defect.
            let head_graphemes: Vec<&str> = head.graphemes(true).collect();
            assert!(
                head_graphemes.is_empty() || graphemes.starts_with(&head_graphemes),
                "width {width}: the head cut through a cluster: {head:?}"
            );
            assert!(
                display_width(&head) <= width,
                "width {width}: the head is {} columns over a {width} budget",
                display_width(&head)
            );

            let tail: String = cut_keeping_tail(vec![span.clone()], width)
                .iter()
                .map(|s| s.content.to_string())
                .collect();
            // The tail keeps the *last* whole clusters, so it must be a suffix of the
            // grapheme sequence rather than an arbitrary subset of it.
            let kept_tail: Vec<&str> = tail.graphemes(true).collect();
            let is_suffix = kept_tail.is_empty()
                || (kept_tail.len() <= graphemes.len() && graphemes.ends_with(&kept_tail[..]));
            assert!(
                is_suffix,
                "width {width}: the tail cut through a cluster: {tail:?}"
            );
            assert!(
                display_width(&tail) <= width,
                "width {width}: the tail is {} columns over a {width} budget",
                display_width(&tail)
            );
        }
    }

    /// **`display_width` is ratatui's measurement, not `unicode-width`'s.**
    ///
    /// `unicode-width` reports the halfwidth katakana sound marks `ﾞ` (U+FF9E) and
    /// `ﾟ` (U+FF9F) as zero-width because they carry `Grapheme_Extend`; terminals
    /// draw them as independent halfwidth cells, so `ratatui-core`'s `CellWidth
    /// for str` adds one column per occurrence (it cites microsoft/terminal#18087).
    ///
    /// This test asserts the two **agree** rather than asserting a number, because
    /// the claim is "we measure what the renderer will place" and that claim is
    /// checkable directly. A hard-coded `2` would pass while the two rules drifted
    /// apart the next time ratatui is bumped.
    ///
    /// The consequence of the old disagreement was not cosmetic: we measured a row
    /// one column narrower than ratatui made it, so the wrap budget overspent and
    /// the renderer clipped the last character — on every line of a Japanese log.
    #[test]
    fn display_width_agrees_with_the_rule_ratatui_places_cells_by() {
        use ratatui::buffer::CellWidth;
        use unicode_width::UnicodeWidthStr;
        // The case that differed: `unicode-width` says 1, ratatui says 2.
        let pair = "\u{FF76}\u{FF9E}";
        assert_eq!(UnicodeWidthStr::width(pair), 1, "the uncorrected rule");
        assert_eq!(
            display_width(pair),
            2,
            "and the rule that matches placement"
        );
        assert_eq!(
            pair.cell_width() as usize,
            display_width(pair),
            "our measurement must be the renderer's"
        );
        // Both marks, and an orphaned one on its own.
        for sample in [
            "\u{FF76}\u{FF9E}",
            "\u{FF9E}",
            "\u{FF76}\u{FF9E}\u{FF9F}",
            "\u{30AB}\u{FF9E}\u{30AD}\u{FF9E}",
            "\u{65E5}\u{672C}\u{8A9E}",
            "plain ascii",
            "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}\u{200D}\u{1F466}",
        ] {
            assert_eq!(
                sample.cell_width() as usize,
                display_width(sample),
                "{sample:?} measures differently here than it is placed"
            );
        }
    }

    /// **The wrap budget now spends what the renderer has.**
    ///
    /// The observable half of the change above: a transcript row of halfwidth
    /// katakana wraps to a width the buffer can actually fill. With the uncorrected
    /// rule the wrapper believed it had a column to spare that did not exist, and
    /// the last character was clipped at the frame's edge — silently, because a
    /// one-column clip mid-character is exactly what the renderer is trusted to do.
    #[test]
    fn a_row_of_halfwidth_katakana_wraps_to_a_width_the_buffer_can_fill() {
        use crate::wrap::wrap_line;
        use ratatui::buffer::CellWidth;
        use ratatui::style::Style;
        // ｶﾞﾞﾞﾞ... — one column per mark under the old rule, two under ratatui's.
        let text = "\u{FF76}\u{FF9E}".repeat(20);
        for width in 1..=12 {
            let rows = wrap_line(&[(text.as_str(), Style::default())], width);
            for row in &rows {
                let joined: String = row.iter().map(|(t, _)| t.as_str()).collect();
                // Width 1 is the one deliberate exception, and it is the same exception
                // every 2-column cluster has: a cluster wider than the row is emitted
                // whole rather than cut in half, so the row overhangs by one column.
                // See `split_at_width`'s progress guarantee.
                let ceiling = width.max(2);
                // **Measured with ratatui's rule, not ours.** An earlier version of this
                // test asserted `display_width(&joined) <= width`, which passes under
                // *both* rules when the wrapper and the assertion share a unit — so it
                // proved nothing. The claim under test is "the buffer can fill what the
                // wrapper produced", and `cell_width` is the only function that knows
                // that. Verified by control: reverting `display_width` to the uncorrected
                // rule fails this assertion at width 2 (four columns) rather than passing.
                assert!(
                    joined.cell_width() as usize <= ceiling,
                    "width {width}: a row of {} columns, which the buffer cannot place",
                    joined.cell_width()
                );
            }
        }
    }
}

/// How many terminal columns a string occupies.
///
/// Not `chars().count()`. A braille spinner frame, a box-drawing rule and a
/// CJK glyph are each one `char`, but they occupy one, one and two columns
/// respectively. Measuring characters makes every derived width wrong by the
/// difference, which shows up as a row that overflows by a cell and wraps —
/// the exact class of bug this module exists to prevent.
///
/// # Why this is not plain `unicode-width`
///
/// Because the number has to be the one the renderer will act on, and the
/// renderer is ratatui. `ratatui-core`'s `CellWidth for str` (`buffer/cell_width.rs`)
/// is `unicode-width` **plus one column per halfwidth katakana sound mark**:
/// `unicode-width` reports `ﾞ` (U+FF9E) and `ﾟ` (U+FF9F) as zero-width because
/// they carry `Grapheme_Extend`, but terminals draw them as independent
/// halfwidth cells, so ratatui corrects for it — citing microsoft/terminal#18087
/// and a Ruby reline fix for the same class.
///
/// Measuring with the uncorrected rule therefore disagreed with placement by one
/// column per mark, in the direction that matters: we thought a row was one
/// column narrower than ratatui made it, so a wrap budget spent one column too
/// much and the renderer clipped the last character. A Japanese log line
/// carrying these marks hit it on every line.
///
/// So this function applies the same correction, and the test asserts the two
/// agree rather than asserting a number that would drift when ratatui is bumped.
/// `clamp_line`'s backstop still exists for the status rows; this is what makes
/// the *wrap budget* correct too.
pub fn display_width(text: &str) -> usize {
    use unicode_width::UnicodeWidthStr;
    // Ratatui's rule is `unicode-width` plus a column per halfwidth sound mark.
    // The single-byte special case in its implementation returns 1 for ASCII,
    // which `UnicodeWidthStr::width` already gives, so only the correction is
    // reproduced here.
    text.width() + count_halfwidth_sound_marks(text)
}

/// How many halfwidth katakana sound marks a string carries.
///
/// The same set, for the same reason, as `ratatui_core::buffer::cell_width`'s
/// private helper — kept local because that one is private, and because a
/// dependency on ratatui's *internal* correction would be a dependency on a
/// non-public API. The test pins the agreement instead.
fn count_halfwidth_sound_marks(text: &str) -> usize {
    text.chars()
        .filter(|c| matches!(*c, '\u{FF9E}' | '\u{FF9F}'))
        .count()
}

/// The gap [`side_by_side`] keeps between a row's two halves.
///
/// A module constant rather than a local one so a caller that needs to reason
/// about the same row — the header, which has to decide what fits before it
/// builds anything — reads the number instead of restating it.
///
/// **One, and the reason it is not two is worth recording.** Two columns would
/// read as a clearer separation, and at 80 the header does gain from it — but
/// measured against the same row one column narrower, it costs the footer a
/// whole hint, and the footer's hints are the ones a reader depends on: at 80
/// columns a truncated-file viewer lost `e edit unavailable`, the statement
/// that a bound key does not act (invariant 7). A hint is dropped whole or not
/// at all, so one column of gap is exactly one hint. `side_by_side` cannot
/// know which of those is worth more — which is why this is a module constant
/// a caller reads, and why the trade was made here rather than inside the
/// layout. Two is the right answer the day the footer's own set gets shorter;
/// until then the footer keeps the column.
pub const HALF_GAP: usize = 1;

/// How many columns one half of a row may occupy, given what the other half
/// already needs.
///
/// This is the one piece of [`side_by_side`]'s arithmetic a caller has to
/// reproduce, and both status rows need it: each has a half whose parts mean
/// different things (`R0 W0 CH n/a` is one gauge; `pageup scroll back` is one
/// hint) and therefore has to drop whole parts until the remainder fits. Neither
/// can know the budget without knowing the gap.
///
/// The alternative — calling [`side_by_side`] and inspecting what survived — is
/// how a part ends up cut into rather than dropped, because by the time the
/// layout has run, the half-part has already been made.
pub fn room_beside(other_half: &[ratatui::text::Span<'static>], width: usize) -> usize {
    let taken: usize = other_half.iter().map(|s| display_width(&s.content)).sum();
    width.saturating_sub(taken).saturating_sub(HALF_GAP)
}

/// One row with left-aligned and right-aligned content.
///
/// Status bars and footers are the same problem twice: put two groups of styled
/// spans on one row, keep them apart, and never let either push the row past the
/// terminal's width. Doing it with two different bodies of arithmetic is how a
/// UI ends up with one bar that wraps and another that silently drops a field.
///
/// `priority` decides the loser when both cannot fit, and the loser is trimmed
/// **from its far end** — the end touching the other half gives way first. That
/// is deliberate: a status row is written most-important-last, so trimming the
/// near end keeps the model name and drops the token arithmetic, rather than
/// keeping `12k` and losing the one fragment that says which model produced it.
///
/// Neither side is ever half-drawn: trimming happens at a span boundary, so a
/// styled value cannot come apart mid-glyph.
///
/// **A fragment inside a span is the caller's problem, and the caller's job to
/// prevent.** This function cannot know that `R0 W0 CH n/a` is one measurement or
/// that `pageup scroll back` is one hint; all it sees is spans. Both status rows
/// therefore budget their own halves with [`room_beside`] and drop whole parts
/// before calling here, so what this returns is a backstop for the case where two
/// width tables disagree — see [`clamp_line`] — rather than the mechanism that
/// decides what a narrow terminal keeps.
pub fn side_by_side(
    left: Vec<ratatui::text::Span<'static>>,
    right: Vec<ratatui::text::Span<'static>>,
    width: usize,
    priority: Priority,
) -> ratatui::text::Line<'static> {
    use ratatui::text::{Line, Span};

    let measure = |spans: &[Span<'static>]| -> usize {
        spans.iter().map(|s| display_width(&s.content)).sum()
    };

    // Decide the two widths in *columns*. Both sides are measured the same way,
    // and every later budget is derived from the width actually emitted — never
    // from a width that was planned before a cut changed the text.
    let (lw, rw) = (measure(&left), measure(&right));
    let room = width.saturating_sub(HALF_GAP);
    let (mut lw, rw) = if lw + rw <= room {
        (lw, rw)
    } else {
        match priority {
            // The right half gives way first: it is the side written
            // most-important-last.
            Priority::Left => (lw.min(room), rw.min(room.saturating_sub(lw.min(room)))),
            Priority::Right => (lw.min(room.saturating_sub(rw.min(room))), rw.min(room)),
        }
    };

    let mut left = match priority {
        Priority::Left => cut_keeping_tail(left, lw),
        Priority::Right => cut(left, lw),
    };
    // Re-measure what survived. A span can be dropped whole, and `cut` never
    // emits a partial wide glyph, so the emitted width can be under budget.
    lw = measure(&left);
    let rw = rw.min(room.saturating_sub(lw));
    let right = match priority {
        Priority::Left => cut_keeping_tail(right, rw),
        Priority::Right => cut(right, rw),
    };
    let rw = measure(&right);

    if right.is_empty() {
        // Nothing on the right: leave the rest of the row untouched rather than
        // writing blanks. The terminal already has blank cells there, and a row
        // of trailing spaces only makes a captured screen look like a bug.
        return Line::from(left);
    }
    let pad = width.saturating_sub(lw + rw);
    left.push(Span::raw(" ".repeat(pad)));
    left.extend(right);
    debug_assert_eq!(
        measure(&left),
        lw + pad + rw,
        "a status row must account for every column it emits"
    );
    Line::from(left)
}

/// Which half of a row survives when both cannot fit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Priority {
    /// The left half matters more: measurements yield to run state.
    Left,
    /// The right half matters more: key hints yield to the workspace location.
    Right,
}

/// Guarantee a line occupies at most `width` columns.
///
/// A status row must never wrap: wrapping pushes the next row down and a
/// one-line bar becomes two. This is the backstop that makes that structural,
/// rather than a property of arithmetic that has to stay correct in every branch.
///
/// It exists because two width rules can disagree. Ours is `unicode-width`; ratatui
/// places cells with its own `CellWidth for str`, which is `unicode-width` **plus one
/// column per halfwidth katakana sound mark** (`ﾞ` U+FF9E, `ﾟ` U+FF9F) — characters
/// `unicode-width` calls zero-width because they carry `Grapheme_Extend`, but which
/// terminals render as an independent halfwidth cell. ratatui cites
/// microsoft/terminal#18087 for that correction. So our measurement of `ｶﾞ` is one
/// column while the buffer advances two, and the row is trimmed once more on the way out.
///
/// **An earlier version of this comment gave a different and false reason**: that the two
/// crates depended on different *versions* of `unicode-width`. Measured against
/// `Cargo.lock`, `ratatui-core 0.1.2` and `ratatui-widgets 0.3.2` both resolve to
/// `unicode-width 0.2.2`, the same crate the workspace pins; the second copy in the lock
/// (`0.1.14`) is pulled by `uuid` and `vt100`, neither of which is in the render graph.
/// The disagreement is real, it is not a version skew, and it is worth knowing where it
/// actually lives — because the transcript's wrap budget in `wrap::wrap_line` measures
/// with our rule and ratatui then places with its own, so the same one-column error is
/// live there and this backstop does not cover it.
pub fn clamp_line(
    line: ratatui::text::Line<'static>,
    width: usize,
) -> ratatui::text::Line<'static> {
    let total: usize = line.spans.iter().map(|s| display_width(&s.content)).sum();
    if total <= width {
        return line;
    }
    ratatui::text::Line::from(cut_keeping_tail(line.spans, width))
}

/// Cut styled spans to `width` columns, keeping the **last** whole **grapheme** clusters.
///
/// For the half that is written least-important-first: trimming must remove the leading
/// fragments rather than the trailing ones.
///
/// The arithmetic drops whole graphemes until at least the excess is gone, so the survivor
/// is under budget rather than over it. The version this replaced walked `char`s and could
/// drop *fewer* columns than it had to -- a zero-width character in the run consumed the
/// skip budget without freeing a column, and the row came out wider than asked for.
/// [`crate::wrap::suffix_at_width`] carries the measured table that makes a grapheme the
/// unit a cut may fall on.
pub fn cut_keeping_tail(
    spans: Vec<ratatui::text::Span<'static>>,
    width: usize,
) -> Vec<ratatui::text::Span<'static>> {
    let total: usize = spans
        .iter()
        .map(|s| crate::ui::layout::display_width(&s.content))
        .sum();
    if total <= width {
        return spans;
    }
    // Columns that must go. Dropping whole graphemes may remove more than this, which is
    // what keeps the result under budget rather than over it.
    let mut excess = total - width;
    let mut out = Vec::new();
    for span in spans {
        if excess == 0 {
            out.push(span);
            continue;
        }
        let span_width = crate::ui::layout::display_width(&span.content);
        let keep = span_width.saturating_sub(excess);
        let kept = crate::wrap::suffix_at_width(&span.content, keep);
        let kept_width = crate::ui::layout::display_width(kept);
        excess = excess.saturating_sub(span_width - kept_width);
        if !kept.is_empty() {
            out.push(ratatui::text::Span::styled(kept.to_string(), span.style));
        }
    }
    out
}

/// Cut styled spans to at most `width` columns, keeping whole **grapheme** clusters.
///
/// A grapheme that does not fit is dropped rather than halved: half of a CJK character or
/// half of an emoji ZWJ sequence renders as a replacement box, and a row ending in an
/// orphaned zero-width joiner renders as a *different emoji* from the one in the text.
/// The measured reason lives on [`crate::wrap::prefix_at_width`] -- the per-character width
/// sum and the per-string width disagree by six columns for a family emoji, so a cut
/// decided with one and measured with the other landed mid-cluster at most row widths.
///
/// Under budget is the normal outcome at odd widths: a 2-column grapheme leaves one column
/// unused when three were available.
pub fn cut(
    spans: Vec<ratatui::text::Span<'static>>,
    width: usize,
) -> Vec<ratatui::text::Span<'static>> {
    let mut out = Vec::new();
    let mut left = width;
    for span in spans {
        if left == 0 {
            break;
        }
        // The span is kept whole when nothing was cut, so an untouched styled value does
        // not pay for a re-allocation.
        let kept = crate::wrap::prefix_at_width(&span.content, left);
        if kept.is_empty() {
            // This span's first grapheme is wider than what is left of the row. A later
            // span may still carry narrow glyphs, so this is not the end of the row.
            continue;
        }
        let used = crate::ui::layout::display_width(kept);
        if kept.len() == span.content.len() {
            out.push(span);
        } else {
            out.push(ratatui::text::Span::styled(kept.to_string(), span.style));
        }
        left -= used;
    }
    out
}

/// Read one screen row back as text.
///
/// Test helper, and a deliberate one. `Cell::symbol()` returns the whole
/// grapheme the cell holds, and ratatui coalesces a run of identical adjacent
/// cells into one cell whose symbol repeats — so a row containing `100%` can
/// come back with a single cell whose symbol is `%%`. Appending symbols
/// therefore doubles repeated characters, and taking only the first character
/// silently drops the rest. This walks the row and appends each cell's content
/// in full, which is what a screenshot of the same frame would show.
#[cfg(test)]
pub fn row_text(buffer: &ratatui::buffer::Buffer, y: u16, width: u16) -> String {
    let mut out = String::new();
    let mut x = 0u16;
    while x < width {
        let cell = &buffer[(x, y)];
        let symbol = cell.symbol();
        let span = crate::ui::layout::display_width(symbol).max(1) as u16;
        out.push_str(symbol);
        x += span;
    }
    out
}

/// Read the whole screen back as one string, for `contains` assertions.
#[cfg(test)]
pub fn screen_text(buffer: &ratatui::buffer::Buffer) -> String {
    let area = buffer.area;
    (0..area.height)
        .map(|y| row_text(buffer, y, area.width))
        .collect::<Vec<_>>()
        .join("\n")
}
