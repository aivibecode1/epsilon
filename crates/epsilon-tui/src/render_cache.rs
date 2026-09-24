//! The transcript render cache (M16a): markdown parse + wrap ONCE per line, never
//! per frame.
//!
//! Draw path before M16a: every frame re-ran pulldown-cmark on EVERY assistant row
//! and cloned every string, then Paragraph re-wrapped everything — O(transcript)
//! work at 8+ fps that got worse as sessions grew.
//!
//! Draw path now: each [`crate::state::TranscriptLine`] renders to wrapped screen
//! rows exactly once per width (keyed by its monotonic id); the transcript's row
//! offsets are prefix sums over cached heights; the draw picks the visible window
//! with a binary search and clones ONLY those rows. Cost per frame is O(visible),
//! independent of transcript length.

use std::collections::HashMap;

use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::markdown;
use crate::state::{LineKind, TranscriptLine};
use crate::ui::theme::Theme;
use crate::wrap;

/// One logical transcript line rendered into `width`-wide screen rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedLine {
    /// The role the rows came from. Carried alongside them because the draw path
    /// decorates per kind and a wrapped logical line yields *several* rows that
    /// all share one kind — the row offset table cannot recover it.
    pub kind: LineKind,
    /// Wrapped rows (already prefixed/styled); total height = `rows.len()`.
    pub rows: Vec<Vec<(String, Style)>>,
}

impl RenderedLine {
    fn build(line: &TranscriptLine, width: usize, expand_outputs: bool, theme: &Theme) -> Self {
        // Each LOGICAL line wraps independently (headings/fences never merge across
        // paragraphs); their wrapped rows concatenate into this line's screen rows.
        let mut rows = Vec::new();
        for logical in logical_lines(line, expand_outputs, theme) {
            rows.extend(wrap::wrap_line(&logical, width));
        }
        if rows.is_empty() {
            rows.push(Vec::new());
        }
        Self {
            kind: line.kind,
            rows,
        }
    }
}

/// The styled logical lines of one transcript row BEFORE wrapping (prefix + body).
/// Markdown rows expand here — once per line id, never per frame.
fn logical_lines(
    line: &TranscriptLine,
    expand_outputs: bool,
    theme: &Theme,
) -> Vec<Vec<(String, Style)>> {
    // Expandable rows show their full payload when toggled (M13 behavior preserved).
    if expand_outputs && let Some(full) = line.full.as_ref() {
        return vec![vec![(full.clone(), style_for(line.kind, theme))]];
    }
    if line.code {
        return vec![vec![(line.text.clone(), theme.code_block())]];
    }
    if let Some(md) = line.md.as_ref() {
        return markdown::md_to_lines(md, theme)
            .into_iter()
            .map(|l| {
                l.spans
                    .into_iter()
                    .map(|s| (s.content.to_string(), s.style))
                    .collect()
            })
            .collect();
    }
    let marker = row_marker(line.kind, theme);
    if marker.is_empty() {
        vec![vec![(line.text.clone(), style_for(line.kind, theme))]]
    } else {
        vec![vec![
            (marker, style_for(line.kind, theme)),
            (line.text.clone(), style_for(line.kind, theme)),
        ]]
    }
}

/// The marker that introduces a row's own text, inside the gutter.
///
/// Not every role gets one: assistant prose is what the page is *for* and reads
/// better undecorated, and the reasoning rule is already carried by the gutter.
/// Every marker here (and the absence of one) comes from [`Theme`], so a console
/// with a legacy code page gets characters it can actually draw.
///
/// The user row is the one prefix the cache owns. `state::push_local_input` used
/// to store its own `"> "` as part of the text, which rendered as `❯ > hello` —
/// two markers, four columns, and a `>` sitting where the `▎` gutter belongs.
fn row_marker(kind: LineKind, theme: &Theme) -> String {
    let glyphs = &theme.glyphs;
    match kind {
        LineKind::User => format!("{} ", glyphs.prompt),
        // A pending call needs its own marker, not the result's tick: the reader
        // has to tell "about to run" from "ran" without reading the text, and on
        // a terminal with no surfaces the glyph is all there is.
        LineKind::ToolCall => format!("{} ", glyphs.tool_call),
        LineKind::ToolResultOk => format!("{} ", glyphs.tick),
        LineKind::ToolResultErr => format!("{} ", glyphs.cross),
        // A notice is not the same event as a tool call and must not share its
        // marker: with `NO_COLOR` or 16 colours the two would be the same pixels.
        LineKind::System => format!("{} ", glyphs.bullet),
        LineKind::Assistant | LineKind::Thinking => String::new(),
    }
}

/// The body style for a transcript row.
///
/// The gutter and any band come from `ui::transcript::decorate`; this is only the
/// text. Both read the same theme, so a row cannot end up half-styled.
fn style_for(kind: LineKind, theme: &Theme) -> Style {
    match kind {
        LineKind::User => theme.user(),
        LineKind::Assistant => theme.assistant(),
        LineKind::Thinking => theme.thinking(),
        LineKind::ToolCall => theme.warn(),
        LineKind::ToolResultOk => theme.dim(),
        LineKind::ToolResultErr => theme.danger(),
        LineKind::System => theme.system(),
    }
}

/// Width-keyed cache: id → rendered rows, plus prefix sums for O(log n) windowing.
///
/// Eviction is structural: entries whose id no longer exists in the live transcript
/// (`/clear`) drop on the next reconcile; entries for surviving ids stay valid across
/// frames because ids are content-stable.
pub struct RenderCache {
    width: usize,
    expand_outputs: bool,
    rendered: HashMap<u64, RenderedLine>,
    /// Parallel to `UiState.lines` after `reconcile`: cumulative row counts.
    offsets: Vec<usize>,
    total_rows: usize,
    reconciled_ids: Vec<u64>,
}

impl Default for RenderCache {
    fn default() -> Self {
        Self::new()
    }
}

impl RenderCache {
    /// Empty cache; first draw builds everything once.
    pub fn new() -> Self {
        Self {
            width: 0,
            expand_outputs: false,
            rendered: HashMap::new(),
            offsets: Vec::new(),
            total_rows: 0,
            reconciled_ids: Vec::new(),
        }
    }

    /// Bring cache bookkeeping in line with the current transcript state.
    ///
    /// Renders any unseen lines (the only markdown/wrap work in the system) and
    /// maintains the offset table incrementally: the common id-prefix with the last
    /// reconcile is reused untouched (transcripts are append-only except `/clear`),
    /// so an idle session's steady-state reconcile cost is O(1), independent of
    /// transcript length.
    pub fn reconcile(
        &mut self,
        lines: &[TranscriptLine],
        width: usize,
        expand_outputs: bool,
        theme: &Theme,
    ) {
        let invalidate_all = width != self.width || expand_outputs != self.expand_outputs;
        if invalidate_all {
            self.rendered.clear();
            self.offsets.clear();
            self.reconciled_ids.clear();
            // offsets[0] is the base (line 0 starts at screen row 0); it must exist
            // before the incremental append below.
            self.offsets.push(0);
            self.width = width;
            self.expand_outputs = expand_outputs;
        }

        // Reuse the common prefix of already-reconciled lines.
        let reuse = self
            .reconciled_ids
            .iter()
            .zip(lines.iter())
            .take_while(|(cached, line)| **cached == line.id)
            .count();
        self.reconciled_ids.truncate(reuse);
        self.offsets.truncate(reuse + 1);

        let mut total = self.offsets.last().copied().unwrap_or(0);
        for line in &lines[reuse..] {
            self.rendered
                .entry(line.id)
                .or_insert_with(|| RenderedLine::build(line, width, expand_outputs, theme));
            let h = self.rendered[&line.id].rows.len();
            total += h;
            self.reconciled_ids.push(line.id);
            self.offsets.push(total);
        }

        // Drop entries for lines removed by `/clear`.
        let live: std::collections::HashSet<u64> = self.reconciled_ids.iter().copied().collect();
        self.rendered.retain(|id, _| live.contains(id));
        self.total_rows = total;
    }

    /// Total number of screen rows in the transcript (after the last reconcile).
    pub fn total_rows(&self) -> usize {
        self.total_rows
    }

    /// Materialize screen rows `[skip, skip + visible)` as ratatui `Line`s.
    ///
    /// Only this window is cloned out of the cache — scrolling through a huge session
    /// costs the same as a tiny one.
    ///
    /// The draw path wants the kinds as well
    /// ([`Self::visible_rows_with_kinds`]); this plain form is for callers that
    /// only need the pixels.
    pub fn visible_rows(&self, skip: usize, visible: usize) -> Vec<Line<'static>> {
        self.visible_rows_with_kinds(skip, visible)
            .into_iter()
            .map(|(_kind, line)| line)
            .collect()
    }

    /// The same window, each row paired with the [`LineKind`] it came from.
    ///
    /// The draw path needs the kind to pick the row's gutter and band, and the
    /// kind is not recoverable from a materialized `Line`. It is carried out of
    /// the cache rather than re-derived from `ui.lines()` by index, because a
    /// logical line wraps into *several* screen rows: row index and line index
    /// stop agreeing the moment anything wraps.
    ///
    /// Decoration deliberately stays outside this call, so the cache key does
    /// not have to gain the theme (see [`RenderCache::reconcile`]). The body
    /// styles here do come from the theme — they always have — but only the
    /// *wrap width* and the expand toggle change the cached structure, and
    /// neither of those is a theme property.
    pub fn visible_rows_with_kinds(
        &self,
        skip: usize,
        visible: usize,
    ) -> Vec<(LineKind, Line<'static>)> {
        let mut out = Vec::with_capacity(visible.min(self.total_rows));
        if visible == 0 || self.reconciled_ids.is_empty() {
            return out;
        }
        // offsets[i] = first screen row of logical line i (offsets[0] = 0).
        // Find the last line whose top is <= skip: that's the one containing row `skip`.
        let start_line_idx = match self.offsets.binary_search(&skip) {
            Ok(i) => i,
            Err(i) => i.saturating_sub(1),
        };
        let end = skip + visible;
        for (i, &id) in self.reconciled_ids.iter().enumerate().skip(start_line_idx) {
            let rendered = &self.rendered[&id];
            let line_top = self.offsets[i];
            if line_top >= end {
                break;
            }
            let from = skip.saturating_sub(line_top);
            let to = rendered.rows.len().min(end - line_top);
            for row in &rendered.rows[from..to] {
                let spans: Vec<Span<'static>> = row
                    .iter()
                    .map(|(t, s)| Span::styled(t.clone(), *s))
                    .collect();
                out.push((rendered.kind, Line::from(spans)));
            }
        }
        out
    }

    /// Number of distinct rendered lines held (bench/diagnostics).
    pub fn len(&self) -> usize {
        self.rendered.len()
    }

    /// True when nothing is cached.
    pub fn is_empty(&self) -> bool {
        self.rendered.is_empty()
    }
}

/// Convenience: full pipeline for tests and benches — reconcile then take a window.
pub fn snapshot_rows(
    lines: &[TranscriptLine],
    width: usize,
    expand_outputs: bool,
    skip: usize,
    visible: usize,
    theme: &Theme,
) -> Vec<Line<'static>> {
    snapshot_rows_with_kinds(lines, width, expand_outputs, skip, visible, theme)
        .into_iter()
        .map(|(_, line)| line)
        .collect()
}

/// [`snapshot_rows`], keeping the kind of the row each line came from.
pub fn snapshot_rows_with_kinds(
    lines: &[TranscriptLine],
    width: usize,
    expand_outputs: bool,
    skip: usize,
    visible: usize,
    theme: &Theme,
) -> Vec<(LineKind, Line<'static>)> {
    let mut cache = RenderCache::new();
    cache.reconcile(lines, width, expand_outputs, theme);
    cache.visible_rows_with_kinds(skip, visible)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One theme for the suite, in the Unicode repertoire production uses by
    /// default. It is also what decides each row's leading marker, via
    /// [`row_marker`] — the rows carry a `LineKind` and the marker table turns it
    /// into a glyph, so a change to that vocabulary moves these assertions.
    fn theme() -> Theme {
        Theme::new(
            crate::ui::theme::ColorMode::Ansi16,
            crate::ui::theme::UNICODE,
        )
    }

    fn plain(kind: LineKind, text: impl Into<String>) -> TranscriptLine {
        TranscriptLine::plain(kind, text)
    }

    #[test]
    fn assistant_markdown_renders_once_and_stays_cached() {
        let lines = vec![TranscriptLine::assistant_md("# Title\n\nbody text")];
        let mut cache = RenderCache::new();
        cache.reconcile(&lines, 80, false, &theme());
        assert_eq!(cache.len(), 1);
        // Same state again: no new renders (len unchanged), offsets stable.
        cache.reconcile(&lines, 80, false, &theme());
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.total_rows(), 2, "heading + paragraph");
    }

    #[test]
    fn visible_window_matches_full_render_in_the_middle() {
        let mut lines: Vec<TranscriptLine> = Vec::new();
        for i in 0..50 {
            lines.push(plain(LineKind::System, format!("system line {i}")));
        }
        let mut cache = RenderCache::new();
        cache.reconcile(&lines, 80, false, &theme());
        let mid = cache.visible_rows(20, 5);
        assert_eq!(mid.len(), 5);
        let text_of = |l: &Line<'_>| {
            l.spans
                .iter()
                .map(|s| s.content.to_string())
                .collect::<String>()
        };
        assert_eq!(text_of(&mid[0]), "• system line 20");
        assert_eq!(text_of(&mid[4]), "• system line 24");
    }

    #[test]
    fn window_crossing_a_wrapped_line_stitches_both_halves() {
        let long = "word ".repeat(40).trim_end().to_string(); // wraps at narrow widths
        let lines = vec![
            plain(LineKind::System, "before"),
            TranscriptLine::expandable(LineKind::ToolResultOk, long.clone(), long.clone()),
            plain(LineKind::System, "after"),
        ];
        let mut cache = RenderCache::new();
        cache.reconcile(&lines, 30, false, &theme());
        assert!(cache.total_rows() > 3, "tool line must wrap");
        // Window starting AT the tool line: its first wrapped row carries the ✓
        // mark (prefix and body are one logical line), later rows are continuations.
        let rows = cache.visible_rows(1, 2);
        assert_eq!(rows.len(), 2);
        let text_of = |l: &Line<'_>| {
            l.spans
                .iter()
                .map(|s| s.content.to_string())
                .collect::<String>()
        };
        let first = text_of(&rows[0]);
        assert!(
            first.starts_with("✓ "),
            "mark rides on the first wrapped row: {first:?}"
        );
        assert!(
            text_of(&rows[1]).starts_with("word"),
            "continuation row has no mark"
        );
    }

    #[test]
    fn clear_then_reappend_drops_stale_entries() {
        let mut lines = vec![plain(LineKind::System, "one")];
        let mut cache = RenderCache::new();
        cache.reconcile(&lines, 80, false, &theme());
        lines = vec![plain(LineKind::System, "fresh")];
        cache.reconcile(&lines, 80, false, &theme());
        assert_eq!(cache.len(), 1, "stale id evicted");
        assert_eq!(cache.total_rows(), 1);
    }

    #[test]
    fn width_change_invalidates_everything() {
        let lines = vec![plain(LineKind::System, "some text")];
        let mut cache = RenderCache::new();
        cache.reconcile(&lines, 80, false, &theme());
        let before = cache.len();
        cache.reconcile(&lines, 40, false, &theme());
        assert_eq!(before, 1);
        assert_eq!(cache.len(), 1, "rebuilt once at the new width");
    }

    #[test]
    fn expand_toggle_rerenders_tool_cards_only_in_place() {
        let long = "x".repeat(200);
        let lines = vec![TranscriptLine::expandable(
            LineKind::ToolResultOk,
            format!("bash → {}", &long[..40]),
            format!("bash → {long}"),
        )];
        let mut cache = RenderCache::new();
        cache.reconcile(&lines, 60, false, &theme());
        let collapsed_rows = cache.total_rows();
        cache.reconcile(&lines, 60, true, &theme());
        assert!(
            cache.total_rows() > collapsed_rows,
            "expanded payload must produce more rows"
        );
    }

    #[test]
    fn a_user_row_carries_one_marker_not_two() {
        // The regression this pins: `state::push_local_input` used to store
        // `"> hello"` and the cache prepend `"❯ "`, so the row rendered as
        // `❯ > hello` — two markers, four columns, and a `>` sitting where the
        // `▎` gutter belongs. The text is now stored marker-free, so the single
        // surviving marker is the theme's prompt glyph.
        //
        // The stored text is built by `push_local_input` itself, not typed in
        // here: a fixture with the `"> "` written out by hand would keep passing
        // after the real one came back.
        let mut ui = crate::state::UiState::new();
        ui.push_local_input("hello");
        let user = ui.lines().first().expect("the echo").clone();
        assert_eq!(
            user.text, "hello",
            "the marker is the renderer's job; storing one doubles it"
        );
        let lines = vec![
            user,
            TranscriptLine::expandable(
                LineKind::ToolResultErr,
                "bash → boom".to_string(),
                "bash → boom".to_string(),
            ),
            TranscriptLine::plain(LineKind::ToolCall, "tool-call bash {ls}"),
            TranscriptLine::plain(LineKind::System, "run finished"),
        ];
        let rows = snapshot_rows(&lines, 80, false, 0, 10, &theme());
        let text_of = |l: &Line<'_>| {
            l.spans
                .iter()
                .map(|s| s.content.to_string())
                .collect::<String>()
        };
        assert_eq!(text_of(&rows[0]), "❯ hello");
        assert_eq!(text_of(&rows[1]), "✗ bash → boom");
        assert_eq!(text_of(&rows[2]), "▸ tool-call bash {ls}");
        assert_eq!(text_of(&rows[3]), "• run finished");
    }

    #[test]
    fn a_tool_call_and_a_notice_are_distinguishable_without_colour() {
        // With `NO_COLOR` and the ASCII repertoire — the least capable terminal we
        // support — a pending call and a lifecycle notice must not be the same
        // pixels. They shared `glyphs.bullet` before this, in *every* mode.
        let theme = Theme::new(crate::ui::theme::ColorMode::None, crate::ui::theme::ASCII);
        let lines = vec![
            TranscriptLine::plain(LineKind::ToolCall, "tool-call bash {cargo test}"),
            TranscriptLine::plain(LineKind::System, "run finished"),
        ];
        let rows = snapshot_rows_with_kinds(&lines, 80, false, 0, 10, &theme);
        assert_eq!(rows.len(), 2);
        // Each row must come back still knowing what it is: without the kind the
        // draw path cannot decorate, which is the defect this whole change fixes.
        assert_eq!(rows[0].0, LineKind::ToolCall);
        assert_eq!(rows[1].0, LineKind::System);
        let text_of = |(_, l): &(LineKind, Line<'_>)| {
            l.spans
                .iter()
                .map(|s| s.content.to_string())
                .collect::<String>()
        };
        let call = text_of(&rows[0]);
        let notice = text_of(&rows[1]);
        assert_eq!(call, "\\ tool-call bash {cargo test}");
        assert_eq!(notice, "* run finished");
        assert_ne!(
            crate::ui::transcript::row_style(&theme, LineKind::ToolCall).0,
            crate::ui::transcript::row_style(&theme, LineKind::System).0,
            "the gutter glyphs must differ when colour cannot carry the role"
        );
    }
}
