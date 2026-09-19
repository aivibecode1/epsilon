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
//! ├──────────────────────────────────────────────────────────────┤  rule
//! │                                                              │
//! │ transcript — conversation, tool cards, notices               │  body (fills)
//! │                                                              │
//! ├──────────────────────────────────────────────────────────────┤  rule
//! │ ❯ composer                                                   │  1..=8
//! ├──────────────────────────────────────────────────────────────┤  rule
//! │ keys that work right now                     location · model│  footer
//! └──────────────────────────────────────────────────────────────┘
//! ```
//!
//! Two rules keep this honest as the window narrows:
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
/// Rows the chrome always owns: the header, and the two rules. The footer is
/// counted with them because it is never optional either.
pub const CHROME_ROWS: u16 = 4;

/// The regions of one frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Regions {
    /// Status on the left, live metrics on the right.
    pub header: Rect,
    /// The conversation.
    pub transcript: Rect,
    /// The divider between the transcript and the composer.
    pub transcript_rule: Rect,
    /// Where the user types.
    pub composer: Rect,
    /// The divider between the composer and the key hints.
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
        // Chrome first: three one-row bands, plus the composer's own rows.
        let composer = composer_rows.clamp(1, Self::composer_cap(area.height));
        // `Min(1)` rather than `Fill`: with `Fill` a very short screen hands the
        // transcript a zero-height rectangle and the body silently disappears.
        let [header, transcript, transcript_rule, composer_area, composer_rule, footer] =
            Layout::vertical([
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
            transcript_rule,
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

    fn area(width: u16, height: u16) -> Rect {
        Rect::new(0, 0, width, height)
    }

    #[test]
    fn regions_tile_the_frame_without_gaps_or_overlap() {
        for (w, h) in [(44, 10), (80, 24), (100, 30), (160, 48)] {
            let r = Regions::split(area(w, h), 1);
            let expected = 1 + r.transcript.height + 1 + r.composer.height + 1 + 1;
            assert_eq!(expected, h, "{w}x{h} left {expected} rows unaccounted");
            assert_eq!(r.header.y, 0);
            assert_eq!(r.transcript.y, 1);
            assert_eq!(r.footer.y + 1, h, "footer must sit on the last row");
            assert_eq!(r.header.width, w);
        }
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
pub fn side_by_side(
    left: Vec<ratatui::text::Span<'static>>,
    right: Vec<ratatui::text::Span<'static>>,
    width: usize,
    priority: Priority,
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

    let mut left = match priority {
        Priority::Left => cut_keeping_tail(left, lw),
        Priority::Right => cut(left, lw),
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
    let total: usize = line
        .spans
        .iter()
        .map(|s| display_width(&s.content))
        .sum();
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
