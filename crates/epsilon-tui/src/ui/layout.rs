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
//! │                                                              │
//! ├──────────────────────────────────────────────────────────────┤  rule
//! │ ❯ composer                                                   │  1..=8
//! │ keys that work right now                     location · model│  footer
//! └──────────────────────────────────────────────────────────────┘
//! ```
//!
//! **One rule, not two.** It sits above the composer and means "everything below
//! this line takes keystrokes". There used to be a second one between the header
//! and the transcript, drawn by the same function in the same style, dividing
//! nothing the reader could name while costing a row of conversation at every
//! size — two of twenty-four rows at 80×24. One fold says where the transcript
//! stops; two say it twice.
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
/// Rows the chrome always owns: the header, the composer's rule, and the footer.
/// The footer is counted with them because it is never optional either.
///
/// This was 4 while the transcript had a rule of its own. Cutting that rule is
/// where the recovered row went; the constant moved with it so the arithmetic
/// here and in the tests cannot drift apart.
pub const CHROME_ROWS: u16 = 3;

/// The regions of one frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Regions {
    /// Status on the left, live metrics on the right.
    pub header: Rect,
    /// The conversation. Runs straight down to [`Regions::composer_rule`]: there
    /// is deliberately no second divider above it.
    pub transcript: Rect,
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
        // Chrome first: two one-row bands, plus the composer's own rows.
        let composer = composer_rows.clamp(1, Self::composer_cap(area.height));
        // `Min(1)` rather than `Fill`: with `Fill` a very short screen hands the
        // transcript a zero-height rectangle and the body silently disappears.
        let [header, transcript, composer_area, composer_rule, footer] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(composer),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .areas(area);
        Self {
            header,
            transcript,
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
            // header + transcript + composer + ONE rule + footer. The transcript
            // rule was removed deliberately: this is the arithmetic that proves
            // its row went to the transcript rather than going missing.
            let expected = 1 + r.transcript.height + r.composer.height + 1 + 1;
            assert_eq!(expected, h, "{w}x{h} left {expected} rows unaccounted");
            assert_eq!(r.header.y, 0);
            assert_eq!(r.transcript.y, r.header.bottom());
            assert_eq!(
                r.transcript.bottom(),
                r.composer.y,
                "{w}x{h}: the transcript must run into the composer with no rule between"
            );
            assert_eq!(
                r.composer_rule.y,
                r.composer.bottom(),
                "{w}x{h}: the one rule sits below the composer, not above it"
            );
            assert_eq!(r.footer.y + 1, h, "footer must sit on the last row");
            assert_eq!(r.header.width, w);
        }
    }

    #[test]
    fn cutting_the_transcript_rule_gave_its_row_to_the_transcript() {
        // The reviewable evidence that the removed rule was intentional rather
        // than lost: at every supported size the transcript is exactly one row
        // taller than a layout with two rules would have allowed, and the frame
        // still tiles completely.
        for (w, h) in [(44, 10), (80, 24), (100, 30), (160, 48)] {
            let r = Regions::split(area(w, h), 1);
            assert_eq!(
                r.transcript.height,
                h - CHROME_ROWS - r.composer.height,
                "{w}x{h}: transcript height no longer matches the chrome budget"
            );
        }
        assert_eq!(CHROME_ROWS, 3, "one header, one rule, one footer");
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
    fn the_floor_is_reported_rather_than_guessed() {
        assert!(Regions::too_small(area(43, 30)));
        assert!(Regions::too_small(area(100, 9)));
        assert!(!Regions::too_small(area(44, 10)));
        let message = too_small_message(30, 8);
        assert!(message.contains("44×10"), "{message}");
        assert!(message.contains("30×8"), "{message}");
    }

    /// The left half the footer composes: mode, badge, then the hints.
    ///
    /// The badge is the span the real caller protects, and it is the middle one:
    /// the mode is pushed before it and the hints are pushed after, so a count of
    /// "spans to keep whole" from either end would protect the wrong one. That is
    /// why the parameter is an index.
    fn badge_row(badge: &'static str) -> (Vec<ratatui::text::Span<'static>>, Option<usize>) {
        (
            vec![
                Span::raw("insert"),
                Span::raw(" "),
                Span::raw(badge),
                Span::raw(" · "),
                Span::raw("g jump to live"),
            ],
            Some(2),
        )
    }

    /// The whole row as text, and where its right half begins.
    ///
    /// The right half is cut from its far end too once the left has taken its
    /// share, so an assertion about *what the left kept* must not read the
    /// location's tail as if it were the hints'.
    fn row_parts(location: &'static str, badge: &'static str, width: usize) -> (String, String) {
        let (left, whole) = badge_row(badge);
        let line = side_by_side(
            left,
            vec![Span::raw(location)],
            width,
            Priority::Right,
            whole,
        );
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        let split = text.find(location).unwrap_or(text.len());
        (text[..split].to_string(), text[split..].to_string())
    }

    #[test]
    fn a_protected_span_is_kept_whole_or_dropped_whole() {
        // The defect this exists for, in the shape the footer produces it: a
        // *fraction* cut near its end keeps the numerator and loses the
        // denominator, so `^ 12/40` would render as `^ 12` — a position that is
        // wrong about the total rather than merely shorter. A badge whose size
        // does not fit whole is dropped, and what it did not use goes to the spans
        // before it rather than to a partial badge. The location is short here on
        // purpose: this is a test about the left half's trim, and a location that
        // eats the row would be testing the right half's.
        let location = "~/api · main";

        // 27 columns: the location takes its 12, the hints give way, and the
        // badge is **still whole** — which is what reserving its width before the
        // two halves are decided buys. Without the reservation the location's
        // share starved the left half down to the badge's own 7 columns and then
        // past them, for no reason a reader could see on the row.
        let (left, right) = row_parts(location, "^ 12/40", 27);
        assert_eq!(
            left, "insert ^ 12/40 ",
            "the badge must be kept whole: {left:?}"
        );
        assert!(right.starts_with(location), "{right:?}");

        // 18 columns: 6 of them left after the location and the gap, where
        // `insert ` (7) plus the badge (7) cannot fit. The badge is absent — and
        // absent, not `^ 12` — while the mode it outlived is still there to say
        // which keyboard the row belongs to.
        let (left, _) = row_parts(location, "^ 12/40", 18);
        assert!(
            !left.contains("12") && !left.contains('^'),
            "a partial badge is a wrong number, not a shorter one: {left:?}"
        );
        assert!(left.starts_with("insert"), "{left:?}");
    }
}

/// How many terminal columns a string occupies.
///
/// Not `chars().count()`. A braille spinner frame, a box-drawing rule and a
/// CJK glyph are each one `char`, but they occupy one, one and two columns
/// respectively. Measuring characters makes every derived width wrong by the
/// difference, which shows up as a row that overflows by a cell and wraps —
/// the exact class of bug this module exists to prevent.
pub fn display_width(text: &str) -> usize {
    use unicode_width::UnicodeWidthStr;
    text.width()
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
/// `whole_span` is the escape hatch for a value whose *fragments mean different
/// things* — the same reason a half-glyph is refused, one level up. The footer's
/// frozen badge is the case: `^ 12/40` cut to `^ 12` is not a shorter badge, it
/// is a **wrong number**, because the part that says "of how many" is what its
/// last characters carried. The span at that index is emitted entire or not at
/// all, and the spans around it cut as they always did; `None` — every caller but
/// the footer — changes nothing.
///
/// An index rather than a count because the index is what the caller actually
/// knows: the badge is one span in the middle of the left half, with the mode in
/// front of it and the hints behind, and how many of either there are is not the
/// caller's business.
pub fn side_by_side(
    left: Vec<ratatui::text::Span<'static>>,
    right: Vec<ratatui::text::Span<'static>>,
    width: usize,
    priority: Priority,
    whole_span: Option<usize>,
) -> ratatui::text::Line<'static> {
    use ratatui::text::{Line, Span};

    /// At least one column must separate the halves, or a value reads as part of
    /// the label beside it.
    const GAP: usize = 1;

    let measure = |spans: &[Span<'static>]| -> usize {
        spans.iter().map(|s| display_width(&s.content)).sum()
    };

    // Decide the two widths in *columns*. Both sides are measured the same way,
    // and every later budget is derived from the width actually emitted — never
    // from a width that was planned before a cut changed the text.
    let (lw, rw) = (measure(&left), measure(&right));
    let room = width.saturating_sub(GAP);
    // The protected span is reserved *before* the shares are decided, and that
    // ordering is the point rather than an implementation detail: the `Right`
    // formula below hands the left half whatever the location left over, so a
    // location can take the row down to nothing and the badge would be dropped
    // for want of a column while the location itself was short enough to give it
    // one. Reserving first makes the badge's fate depend on the width of the row,
    // not on which half happened to be measured first.
    let reserved = whole_span
        .and_then(|i| left.get(i))
        .map_or(0, |s| display_width(&s.content).min(room));
    let (mut lw, mut rw) = if lw + rw <= room {
        (lw, rw)
    } else {
        match priority {
            // The right half gives way first: it is the side written
            // most-important-last.
            Priority::Left => (lw.min(room), rw.min(room.saturating_sub(lw.min(room)))),
            Priority::Right => (lw.min(room.saturating_sub(rw.min(room))), rw.min(room)),
        }
    };
    if reserved > 0 {
        lw = lw.max(reserved);
        rw = rw.min(room.saturating_sub(lw));
    }

    let mut left = match priority {
        Priority::Left => cut_keeping_tail(left, lw),
        Priority::Right => cut_keeping_whole(left, lw, whole_span),
    };
    // Re-measure what survived. A span can be dropped whole, and `cut` never
    // emits a partial wide glyph, so the emitted width can be under budget.
    lw = measure(&left);
    rw = rw.min(room.saturating_sub(lw));
    let right = match priority {
        Priority::Left => cut_keeping_tail(right, rw),
        Priority::Right => cut(right, rw),
    };
    rw = measure(&right);

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
/// It exists because two width tables can disagree. Our measurements use the
/// workspace's `unicode-width`; ratatui places cells using the version it
/// depends on, and for a handful of code points the two differ by one column.
/// Rather than pin a table and hope, the row is measured once more on the way
/// out and trimmed to fit.
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

/// Cut styled spans to `width` columns, keeping the **last** characters.
///
/// For the half that is written least-important-first: trimming must remove the
/// leading fragments rather than the trailing ones.
pub fn cut_keeping_tail(
    spans: Vec<ratatui::text::Span<'static>>,
    width: usize,
) -> Vec<ratatui::text::Span<'static>> {
    use unicode_width::UnicodeWidthChar;

    let total: usize = spans
        .iter()
        .map(|s| crate::ui::layout::display_width(&s.content))
        .sum();
    if total <= width {
        return spans;
    }
    let mut skip = total - width;
    let mut out = Vec::new();
    for span in spans {
        if skip == 0 {
            out.push(span);
            continue;
        }
        let span_width = crate::ui::layout::display_width(&span.content);
        if skip >= span_width {
            skip -= span_width;
            continue;
        }
        let mut kept = String::new();
        let mut dropped = 0;
        for ch in span.content.chars() {
            if dropped < skip {
                dropped += ch.width().unwrap_or(0);
                continue;
            }
            kept.push(ch);
        }
        skip = 0;
        if !kept.is_empty() {
            out.push(ratatui::text::Span::styled(kept, span.style));
        }
    }
    out
}

/// Cut styled spans to at most `width` columns, keeping whole characters.
///
/// A wide glyph that does not fit is dropped rather than halved: half a CJK
/// character renders as a replacement box.
pub fn cut(
    spans: Vec<ratatui::text::Span<'static>>,
    width: usize,
) -> Vec<ratatui::text::Span<'static>> {
    use unicode_width::UnicodeWidthChar;

    let mut out = Vec::new();
    let mut left = width;
    for span in spans {
        if left == 0 {
            break;
        }
        let mut taken = String::new();
        let mut used = 0;
        for ch in span.content.chars() {
            let w = ch.width().unwrap_or(0);
            if used + w > left {
                break;
            }
            taken.push(ch);
            used += w;
        }
        left -= used;
        if !taken.is_empty() && used == crate::ui::layout::display_width(&span.content) {
            out.push(span);
        } else if !taken.is_empty() {
            out.push(ratatui::text::Span::styled(taken, span.style));
        }
    }
    out
}

/// [`cut`], except that the span at `whole` is emitted entire or not at all.
///
/// The one case it exists for is the footer's position badge, which is a
/// *fraction*: `^ 12/40` is a position and `^ 12` is a lie about one, while no
/// badge at all is merely less information. So the protected span is emitted only
/// when its full width fits the columns still free where it sits; otherwise it is
/// skipped and everything after it is offered the room — which is why a dropped
/// badge does not take the hints down with it.
///
/// "Where it sits" is the whole subtlety, and it is why this is a running budget
/// and not a reservation: asking for the badge's width up front would let it keep
/// its place on a row too narrow for the mode word in front of it, and a row that
/// reads `^ 12/40` with no mode says less than one that reads `insert` — the one
/// thing it dropped is the one thing that explained the badge.
fn cut_keeping_whole(
    spans: Vec<ratatui::text::Span<'static>>,
    width: usize,
    whole: Option<usize>,
) -> Vec<ratatui::text::Span<'static>> {
    let Some(index) = whole.filter(|i| *i < spans.len()) else {
        return cut(spans, width);
    };
    let mut spans = spans;
    let guarded = spans.remove(index);
    let guarded_width = display_width(&guarded.content);
    let before = cut(spans[..index].to_vec(), width.saturating_sub(guarded_width));
    if before.len() < index {
        // The value does not fit beside the spans that introduce it — the `insert`
        // that says which keyboard the row belongs to, and the blank between them.
        // It is dropped *whole* rather than shortened, and the room it did not use
        // goes to the spans after it rather than being spent on a prefix that
        // would read as a footer with its subject missing.
        return cut(spans, width);
    }
    let spent: usize = before.iter().map(|s| display_width(&s.content)).sum();
    let mut out = before;
    out.push(guarded);
    out.extend(cut(spans[index..].to_vec(), width - spent - guarded_width));
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
