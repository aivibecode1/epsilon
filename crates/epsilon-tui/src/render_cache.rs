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

/// Everything a rendered row depends on besides its own text.
///
/// This is the cache key, in one place. It used to be a bare `expand_outputs`
/// boolean, which was complete while expansion was one global toggle; per-card
/// expansion means the *set* of open cards is part of the answer now, so it has to
/// be represented here rather than being read from the app at render time — a row
/// cached under one set of open cards and served under another is the one bug this
/// type exists to make impossible.
#[derive(Debug, Clone, Copy)]
pub struct RenderState<'a> {
    /// The global "expand all" toggle (`app.tools.expand`). It means *expand all*,
    /// not "the only way to expand": a card the user opened individually stays
    /// open when this is off.
    pub expand_all: bool,
    /// Ids of cards the user opened individually; ids the user closed individually
    /// while [`Self::expand_all`] was on live in [`Self::collapsed`].
    pub expanded: &'a [(u64, bool)],
    /// The key that opens a card, resolved from the live binding table. `None`
    /// when the binding does not resolve, and then the affordance promises nothing.
    pub more_key: Option<&'a str>,
}

impl RenderState<'_> {
    /// Whether an individual card has been opened or closed by hand.
    pub fn explicit(&self, id: u64) -> Option<bool> {
        self.expanded
            .iter()
            .find(|(card_id, _)| *card_id == id)
            .map(|(_, open)| *open)
    }

    /// Whether one card renders its whole body: the user's own decision if they
    /// made one, the global toggle otherwise.
    pub fn is_open(&self, id: u64, has_body: bool) -> bool {
        if !has_body {
            return false;
        }
        self.explicit(id).unwrap_or(self.expand_all)
    }

    /// A short description of the key, for cache-invalidation errors and tests.
    pub fn key(&self) -> (bool, Vec<(u64, bool)>) {
        (self.expand_all, self.expanded.to_vec())
    }
}

impl RenderedLine {
    fn build(line: &TranscriptLine, width: usize, state: &RenderState<'_>, theme: &Theme) -> Self {
        // Each LOGICAL line wraps independently (headings/fences never merge across
        // paragraphs); their wrapped rows concatenate into this line's screen rows.
        //
        // A row that already fits is not re-wrapped, and the reason is not
        // performance. [`logical_lines`] hands back rows that a renderer has already
        // cut *and padded* to the width — a tool card's diff rows are the case —
        // and `wrap::wrap_line` is not idempotent on those: it drops trailing
        // whitespace and collapses a whitespace run to one space, which is right for
        // prose ("screen convention", `wrap.rs`) and wrong for a row whose alignment
        // *is* the padding. The split diff is that row: two halves each padded to its
        // own column, and re-wrapping collapsed the left half's 56 spaces to one — a
        // 138-cell row became 25 cells, the right half landed 13 columns after the
        // left instead of 69, and the blank half that says "this line has no
        // counterpart" disappeared. No 80-column diff test could see it, because
        // only a 120-column body takes the split branch.
        let mut rows = Vec::new();
        for logical in logical_lines(line, state, theme, width) {
            if crate::ui::card::row_width(&logical) <= width {
                rows.push(logical);
            } else {
                rows.extend(wrap::wrap_line(&logical, width));
            }
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
/// Markdown rows and tool cards expand here — once per line id, never per frame.
fn logical_lines(
    line: &TranscriptLine,
    state: &RenderState<'_>,
    theme: &Theme,
    width: usize,
) -> Vec<Vec<(String, Style)>> {
    // A tool card is a card: a title line carrying name, status and duration, then
    // a body capped with an explicit "open it" affordance. It may never take the
    // `full` path below — `full` for an edit result is the raw diff text, and
    // handing a diff to the markdown parser is exactly the defect this replaces.
    if let Some(card) = line.card.as_ref() {
        let open = state.is_open(line.id, card.has_body());
        return crate::ui::card::rows(
            card,
            open,
            width,
            crate::ui::card::BODY_ROWS,
            theme,
            state.more_key,
        );
    }
    // Expandable rows show their full payload when toggled (M13 behavior preserved).
    if state.expand_all
        && let Some(full) = line.full.as_ref()
    {
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

/// The exact state a cached row was rendered under.
///
/// Not `RenderState` itself: that borrows the open-card list and the resolved key
/// string, and a cache cannot hold either. This is the owned snapshot kept for the
/// comparison that decides whether anything has to be rebuilt.
#[derive(Debug, Clone, PartialEq, Eq)]
struct CacheKey {
    expand_all: bool,
    open_cards: Vec<(u64, bool)>,
    more_key: Option<String>,
}

impl CacheKey {
    fn of(state: &RenderState<'_>) -> Self {
        Self {
            expand_all: state.expand_all,
            open_cards: state.expanded.to_vec(),
            more_key: state.more_key.map(str::to_string),
        }
    }
}

/// Width-keyed cache: id → rendered rows, plus prefix sums for O(log n) windowing.
///
/// Eviction is structural: entries whose id no longer exists in the live transcript
/// (`/clear`) drop on the next reconcile; entries for surviving ids stay valid across
/// frames because ids are content-stable.
///
/// The **render state** is the other half of the key. Three things invalidate a
/// cached row: the width, a change to the open/closed set, and a line whose content
/// changed. The first two are detected wholesale (a resize, or an expansion toggle
/// from the key handler, both of which are rare); the third is detected when the
/// caller says which lines changed, which is how a card's status going from
/// `pending` to `ok` re-renders that card and nothing else.
pub struct RenderCache {
    width: usize,
    /// The state every cached row was rendered under. `None` before the first
    /// reconcile, which is what makes "no state yet" distinguishable from "the
    /// default state" rather than equal to it.
    state: Option<CacheKey>,
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
            state: None,
            rendered: HashMap::new(),
            offsets: Vec::new(),
            total_rows: 0,
            reconciled_ids: Vec::new(),
        }
    }

    /// Bring cache bookkeeping in line with the current transcript state.
    ///
    /// Renders any unseen lines (the only markdown/card/wrap work in the system) and
    /// maintains the offset table incrementally: the common id-prefix with the last
    /// reconcile is reused untouched (transcripts are append-only except `/clear`
    /// and except a card changing status in place), so an idle session's
    /// steady-state reconcile cost is O(1), independent of transcript length.
    ///
    /// `restale` names the line ids whose content changed under a stable id — the
    /// card whose result just landed, in practice. Only those entries are dropped,
    /// so a busy run does not rebuild the conversation every time a tool finishes.
    pub fn reconcile(
        &mut self,
        lines: &[TranscriptLine],
        width: usize,
        state: &RenderState<'_>,
        theme: &Theme,
        restale: &[u64],
    ) {
        let key = CacheKey::of(state);
        let invalidate_all = width != self.width || self.state.as_ref() != Some(&key);
        if invalidate_all {
            self.rendered.clear();
            self.offsets.clear();
            self.reconciled_ids.clear();
            // offsets[0] is the base (line 0 starts at screen row 0); it must exist
            // before the incremental append below.
            self.offsets.push(0);
            self.width = width;
            self.state = Some(key);
        }
        if !restale.is_empty() {
            for id in restale {
                self.rendered.remove(id);
            }
        }

        // Reuse the common prefix of already-reconciled lines — but only while the
        // rows are still there. An id whose entry was just dropped by `restale` is
        // still in `reconciled_ids`, and counting it as reusable is how a card whose
        // result landed would keep its old rows for ever (or, worse, vanish: the
        // `retain` below drops entries with no rows, and nothing would rebuild them
        // because the reuse count had already skipped past them).
        let reuse = self
            .reconciled_ids
            .iter()
            .zip(lines.iter())
            .take_while(|(cached, line)| {
                **cached == line.id && self.rendered.contains_key(&line.id)
            })
            .count();
        self.reconciled_ids.truncate(reuse);
        self.offsets.truncate(reuse + 1);

        let mut total = self.offsets.last().copied().unwrap_or(0);
        for line in &lines[reuse..] {
            self.rendered
                .entry(line.id)
                .or_insert_with(|| RenderedLine::build(line, width, state, theme));
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
///
/// `expand_outputs` is the *global* toggle; a test that wants one card open passes
/// it through [`RenderState`] directly instead.
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
    let state = RenderState {
        expand_all: expand_outputs,
        expanded: &[],
        more_key: None,
    };
    cache.reconcile(lines, width, &state, theme, &[]);
    cache.visible_rows_with_kinds(skip, visible)
}

/// [`snapshot_rows`] under an explicit render state: the shape a test needs to
/// open one card and leave the others collapsed.
pub fn snapshot_rows_in(
    lines: &[TranscriptLine],
    width: usize,
    state: &RenderState<'_>,
    skip: usize,
    visible: usize,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let mut cache = RenderCache::new();
    cache.reconcile(lines, width, state, theme, &[]);
    cache.visible_rows(skip, visible)
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

    /// The default render state: global expansion off, no per-card decisions.
    fn state(expand_all: bool) -> RenderState<'static> {
        RenderState {
            expand_all,
            expanded: &[],
            more_key: Some("ctrl+x"),
        }
    }

    /// One card, committed through the real reducer so its title is the one a
    /// session would draw rather than a fixture that agrees with the test.
    fn edit_card(diff: &str, expanded: bool) -> (TranscriptLine, RenderState<'static>) {
        let mut ui = crate::state::UiState::new();
        ui.apply(&crate::state::agent_event_edit_result(diff, false));
        let line = ui.lines().first().expect("the card").clone();
        let open: &'static [(u64, bool)] = Box::leak(vec![(line.id, expanded)].into_boxed_slice());
        (
            line,
            RenderState {
                expand_all: false,
                expanded: open,
                more_key: Some("ctrl+x"),
            },
        )
    }

    fn text_of(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|s| s.content.to_string())
            .collect::<String>()
    }

    /// A unified diff of `n` changed lines, for the cap tests.
    fn long_diff(n: usize) -> String {
        let mut out = format!("@@ -1,{n} +1,{n} @@\n");
        for i in 0..n {
            out.push_str(&format!("-old line {i}\n"));
        }
        for i in 0..n {
            out.push_str(&format!("+new line {i}\n"));
        }
        out
    }

    #[test]
    fn assistant_markdown_renders_once_and_stays_cached() {
        let lines = vec![TranscriptLine::assistant_md("# Title\n\nbody text")];
        let mut cache = RenderCache::new();
        cache.reconcile(&lines, 80, &state(false), &theme(), &[]);
        assert_eq!(cache.len(), 1);
        // Same state again: no new renders (len unchanged), offsets stable.
        cache.reconcile(&lines, 80, &state(false), &theme(), &[]);
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
        cache.reconcile(&lines, 80, &state(false), &theme(), &[]);
        let mid = cache.visible_rows(20, 5);
        assert_eq!(mid.len(), 5);
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
        cache.reconcile(&lines, 30, &state(false), &theme(), &[]);
        assert!(cache.total_rows() > 3, "tool line must wrap");
        // Window starting AT the tool line: its first wrapped row carries the ✓
        // mark (prefix and body are one logical line), later rows are continuations.
        let rows = cache.visible_rows(1, 2);
        assert_eq!(rows.len(), 2);
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
        cache.reconcile(&lines, 80, &state(false), &theme(), &[]);
        lines = vec![plain(LineKind::System, "fresh")];
        cache.reconcile(&lines, 80, &state(false), &theme(), &[]);
        assert_eq!(cache.len(), 1, "stale id evicted");
        assert_eq!(cache.total_rows(), 1);
    }

    #[test]
    fn width_change_invalidates_everything() {
        let lines = vec![plain(LineKind::System, "some text")];
        let mut cache = RenderCache::new();
        cache.reconcile(&lines, 80, &state(false), &theme(), &[]);
        let before = cache.len();
        cache.reconcile(&lines, 40, &state(false), &theme(), &[]);
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
        cache.reconcile(&lines, 60, &state(false), &theme(), &[]);
        let collapsed_rows = cache.total_rows();
        cache.reconcile(&lines, 60, &state(true), &theme(), &[]);
        assert!(
            cache.total_rows() > collapsed_rows,
            "expanded payload must produce more rows"
        );
    }

    /// M126: a changed open/closed set is part of the cache key.
    ///
    /// The negative control for the whole per-card feature: if `RenderState` were
    /// not compared, the cache would keep serving the collapsed rows of a card the
    /// user has opened, and the frame would look like the key did nothing.
    ///
    /// The diff is deliberately longer than [`crate::ui::card::BODY_ROWS`], so the
    /// open and collapsed bodies differ by the *cap* rather than by nothing.
    #[test]
    fn changing_which_cards_are_open_invalidates_the_cache() {
        let long_diff = long_diff(13);
        let (line, _) = edit_card(&long_diff, false);
        let id = line.id;
        let mut cache = RenderCache::new();
        cache.reconcile(
            std::slice::from_ref(&line),
            60,
            &state(false),
            &theme(),
            &[],
        );
        let collapsed = cache.total_rows();
        cache.reconcile(
            std::slice::from_ref(&line),
            60,
            &RenderState {
                expand_all: false,
                expanded: &[(id, true)],
                more_key: Some("ctrl+x"),
            },
            &theme(),
            &[],
        );
        let opened = cache.total_rows();
        assert_eq!(
            collapsed,
            crate::ui::card::BODY_ROWS + 2,
            "the collapsed card is the cap plus its title and its affordance"
        );
        assert!(
            opened > collapsed,
            "an opened card must render more rows than a capped one: {collapsed} -> {opened}"
        );
    }

    /// M126: a card whose status moved is rebuilt when it is named, and only then.
    #[test]
    fn a_restaled_card_is_rebuilt_when_named_and_kept_when_not() {
        let diff = "@@ -1,1 +1,1 @@\n-old\n+new\n";
        let (line, _) = edit_card(diff, false);
        let mut cache = RenderCache::new();
        cache.reconcile(
            std::slice::from_ref(&line),
            40,
            &state(false),
            &theme(),
            &[],
        );
        let rows = |cache: &RenderCache| {
            cache
                .rendered
                .get(&line.id)
                .expect("the card was rendered")
                .rows
                .iter()
                .map(|r| r.iter().map(|(t, _)| t.as_str()).collect::<String>())
                .collect::<Vec<_>>()
        };
        let cached = rows(&cache);

        // Whatever the id, an un-named card keeps the very same rows: this is what
        // makes a long transcript O(1) to reconcile per frame. Compared by the
        // rows rather than by the whole entry, because a rebuild re-reads the clock
        // for the card's duration and two builds of the same card can differ by a
        // millisecond of wall time.
        cache.reconcile(
            std::slice::from_ref(&line),
            40,
            &state(false),
            &theme(),
            &[],
        );
        assert_eq!(rows(&cache), cached, "an un-named card must not be rebuilt");

        // Naming it drops the entry, so the next read rebuilds from the card.
        let before = cache.len();
        cache.reconcile(
            std::slice::from_ref(&line),
            40,
            &state(false),
            &theme(),
            &[line.id],
        );
        assert_eq!(rows(&cache), cached, "rebuilt from the same card");
        assert_eq!(cache.len(), before, "nothing else was touched");
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
        let call = text_of(&rows[0].1);
        let notice = text_of(&rows[1].1);
        assert_eq!(call, "\\ tool-call bash {cargo test}");
        assert_eq!(notice, "* run finished");
        assert_ne!(
            crate::ui::transcript::row_style(&theme, LineKind::ToolCall).0,
            crate::ui::transcript::row_style(&theme, LineKind::System).0,
            "the gutter glyphs must differ when colour cannot carry the role"
        );
    }

    // ---- M126: the card path -------------------------------------------------

    /// A tool call's rows never come from `text`: the title is rebuilt from the
    /// card, so a stale `text` cannot leak into the frame.
    #[test]
    fn a_tool_card_renders_its_title_and_body_not_its_text_field() {
        let (line, _) = edit_card("@@ -1,1 +1,1 @@\n-old\n+new\n", false);
        assert!(
            line.text.starts_with("edit → "),
            "the legacy text field stays populated for non-card consumers"
        );
        let rows = snapshot_rows(std::slice::from_ref(&line), 60, false, 0, 20, &theme());
        let joined = rows.iter().map(text_of).collect::<Vec<_>>().join("\n");
        assert!(
            joined.contains("edit edited a.rs"),
            "the title must carry the name and the status: {joined}"
        );
        assert!(
            // The pre-M153 fallback was a row reading `edit → <output>`. The headline may
            // legitimately contain the output's first line, so the assertion has to be about
            // the *form*: an arrow-prefixed row, not the presence of the words.
            !joined.contains("edit → "),
            "the card must not fall back to the `name → body` row: {joined}"
        );
        // The sign gutter is what survives `NO_COLOR`, so it is asserted on the
        // text rather than on the colour: `+`/`-` then the line number.
        assert!(
            joined.contains("- 1 old") && joined.contains("+ 1 new"),
            "the diff body must be drawn with its sign gutter and line numbers: {joined}"
        );
    }
}
