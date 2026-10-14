//! The header: run state on the left, live measurements on the right.
//!
//! ```text
//! ⠹ running 12s · reading src/main.rs       24%/200k · ↑12k ↓3.4k · cache 75% · $0.012 · DeepSeek V3.1
//! ```
//!
//! The header answers "what is happening" and "what has it cost" in one row, and
//! it is the only place those numbers appear. A status bar that repeats the
//! model name, or that prints a percentage in two places with different
//! rounding, teaches the reader to distrust all of it.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::Span;

use crate::ui::layout::{HALF_GAP, Priority, clamp_line, display_width, room_beside, side_by_side};
use crate::ui::metrics::{MetricRole, Metrics};
use crate::ui::theme::Theme;

/// What the run is doing, as the header states it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunState {
    /// Nothing in flight.
    Idle,
    /// Work in flight, with the spinner frame index and the word for the phase.
    Running {
        /// Which spinner frame to draw; the caller advances it.
        frame: usize,
        /// The phase in words (`streaming`, `running bash`, …).
        phase: String,
        /// Seconds since the run started.
        elapsed_secs: u64,
    },
}

impl RunState {
    /// The word the header shows, without decoration.
    pub fn label(&self) -> String {
        match self {
            Self::Idle => "idle".to_string(),
            Self::Running {
                phase,
                elapsed_secs,
                ..
            } => {
                if phase.is_empty() {
                    format!("running {elapsed_secs}s")
                } else {
                    format!("running {elapsed_secs}s · {phase}")
                }
            }
        }
    }
}

/// Draw the header row.
///
/// When the terminal is too narrow for both halves, the measurements yield first
/// and the run state is truncated last: "is it still working" outranks "how many
/// tokens". [`side_by_side`] enforces that, and guarantees the row occupies
/// exactly `area.width` cells so nothing wraps onto the next line.
///
/// **The phase is budgeted here, before either half is joined.** The phase is the
/// only fragment on this row with no length of its own — it is the tool's status
/// line, in the tool's words — so it is the only one that has to be cut to fit.
/// It used to be emitted whole and the right half was cut to whatever was left,
/// which cut *through* the model name: at 80 columns a long `bash: Compiling
/// epsilon-tools v0.1.0` turned `Faux (offline scripted)` into `ed)`, because
/// [`side_by_side`] trims the half that yields from its far end and the model is
/// one span. Budgeting the phase against the smallest right half there is
/// ([`metric_spans`] built against the phase-less run state) keeps every
/// measurement whole, and the phase is what pays for it.
pub fn draw(f: &mut Frame<'_>, area: Rect, theme: &Theme, state: &RunState, metrics: &Metrics) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let width = area.width as usize;
    // The run state is built first because it is the half that does not yield:
    // "is it still working" outranks "how many tokens". The phase is therefore
    // budgeted against the **least** the measurements can be — their pinned tail
    // — rather than against what they happen to want today.
    let floor = state_spans_without_phase(theme, state);
    let left = state_spans(theme, state, &floor, phase_budget(width, &floor, metrics));
    let right = metric_spans(theme, metrics, room_beside(&left, width));
    let line = side_by_side(left, right, width, Priority::Left);
    let line = clamp_line(line, width);
    f.render_widget(line, area);
}

/// The run state's own words, without the phase: the words that are always there.
fn state_spans_without_phase(theme: &Theme, state: &RunState) -> Vec<Span<'static>> {
    match state {
        RunState::Idle => vec![Span::styled("idle", theme.faint())],
        RunState::Running {
            frame,
            elapsed_secs,
            ..
        } => {
            let frames = theme.glyphs.spinner;
            let glyph = frames[frame % frames.len()];
            vec![
                Span::styled(format!("{glyph} "), theme.accent()),
                Span::styled(format!("running {elapsed_secs}s"), theme.dim_strong()),
            ]
        }
    }
}

/// Columns the phase may occupy: what the fixed halves leave over, once the
/// measurements have been reduced to the tail they never drop.
///
/// The other order — measuring the phase against the measurements' *full* width
/// — starved it: at 80 columns the measurements want 80 columns of their own, so
/// a 35-column tool status got a budget of zero and vanished from the row
/// entirely. The measurements can always be dropped down to their pinned parts
/// ([`crate::ui::metrics::MetricPart::pinned`]), so that is the width the phase
/// is budgeted against, and [`metric_spans`] then drops whatever is left over.
fn phase_budget(width: usize, floor: &[Span<'static>], metrics: &Metrics) -> usize {
    let taken: usize = floor.iter().map(|s| display_width(&s.content)).sum();
    let parts = metrics.parts();
    let pinned: Vec<&crate::ui::metrics::MetricPart> = parts.iter().filter(|p| p.pinned).collect();
    let pinned_width: usize = pinned.iter().map(|p| display_width(&p.text)).sum::<usize>()
        + 3 * pinned.len().saturating_sub(1);
    // ` · ` joins the phase to the run state, and it is spent even when the
    // phase behind it is cut to a mark — a separator with nothing after it is
    // noise, so the budget has to cover it before the phase gets a column.
    //
    // **Below the width where the pinned tail fits beside the run state, the run
    // state owns the row instead.** `running 0s` is 12 columns and the pinned
    // model-plus-qualifier is 48, so at 60 columns they cannot share one, and
    // budgeting the phase against a tail that has nowhere to go gives it zero —
    // which loses the phase entirely while the layout's backstop then slices
    // the model name to squeeze the rest in: the defect this whole change is
    // about, one width band down. So below that width the phase takes what the
    // run state leaves and the measurements give way in whole parts.
    if width < taken + HALF_GAP + pinned_width {
        return width.saturating_sub(HALF_GAP + taken + 3);
    }
    width
        .saturating_sub(taken)
        .saturating_sub(pinned_width)
        .saturating_sub(HALF_GAP)
        .saturating_sub(3)
}

/// The run state as drawn, with the phase cut to what the row can spare.
///
/// `floor` is [`state_spans_without_phase`] for the same state, so the budget is
/// measured against the text that is about to be emitted rather than a plan.
fn state_spans(
    theme: &Theme,
    state: &RunState,
    floor: &[Span<'static>],
    budget: usize,
) -> Vec<Span<'static>> {
    let RunState::Running { phase, .. } = state else {
        return floor.to_vec();
    };
    if phase.is_empty() || budget == 0 {
        return floor.to_vec();
    }
    let shown = crate::wrap::fit_width_with(phase, budget, theme.glyphs.ellipsis);
    if shown.is_empty() {
        return floor.to_vec();
    }
    let mut spans = floor.to_vec();
    spans.push(Span::styled(format!(" · {shown}"), theme.dim_strong()));
    spans
}

/// The measurements, dropped whole until the rest fit in `room`.
///
/// **A measurement is dropped or kept; it is never cut.** The right half used to
/// be trimmed by `side_by_side`, which cuts a span from its far end one *column*
/// at a time, and the header's spans are whole gauges. At 60 columns it produced
///
/// ```text
/// idle 0 ↓0 · R0 W0 CH n/a · Faux (offline scripted) · offline
/// ```
///
/// where 100 columns produces `idle … ↑0 ↓0 · …`. The `↑` is the marker that
/// makes the first number mean *input*; without it the row reads `idle 0 ↓0`,
/// and a bare `0` is a value rather than the loss of a label. That is the M196
/// class restated: **a partial badge is worse than a missing one, because a
/// wrong number is not noticed the way an absent one is.** Truncating `R12k` to
/// `12k` is the same lie with different digits.
///
/// **The tail yields, and the model with it is pinned.** [`Metrics::parts`] is
/// written in the order of questions — how much room is left, what has been spent
/// on tokens, whether the cache is working, what it cost, which model — so the
/// *last* measurement is the least important one and is what goes first. The parts
/// marked [`MetricPart::pinned`] are the model and the `offline` qualifier behind
/// it, because the model is the fragment that changes what every other number
/// means.
///
/// The distinction is not academic; it is what the first version of this function
/// got wrong. Dropping from the *front* keeps the tail, so a 60-column frame — the
/// width this whole change is about — rendered
///
/// ```text
/// idle        R0 W0 CH n/a · Faux (offline scripted) · offline
/// ```
///
/// which lost `ctx ?/1.0M` (how much room is left, the number a reader is watching)
/// and `↑0 ↓0`, and kept `R0 W0 CH n/a` — a gauge that says *nothing* here, because
/// this provider cannot report a cache rate at all. Two rules were in conflict and
/// the wrong one won: "drop the leading fragment first" is a rule about *layout*,
/// and the order of *importance* is the one in [`Metrics::parts`].
///
/// The second version kept the right rule and found the pin by
/// [`MetricRole::Model`], which is not the model's role in an offline session — it
/// is `Warn` there, so the pin silently stopped matching in the one row long enough
/// to need it, and the frame read `… · R0 W0 CH n/a · Faux (offline scripted)` with
/// `offline` dropped and the model next. That is why the pin is a field on the part
/// and not an inference from how it is drawn.
fn metric_spans(theme: &Theme, metrics: &Metrics, room: usize) -> Vec<Span<'static>> {
    // The parts are joined by a three-column separator, so a set of `n` costs
    // `sum(widths) + 3 * (n - 1)`. The count is recomputed after every drop
    // rather than decremented, so the budget can never drift from the text.
    let parts = metrics.parts();
    let width_of = |kept: &[&crate::ui::metrics::MetricPart]| -> usize {
        let sum: usize = kept.iter().map(|p| display_width(&p.text)).sum();
        sum + 3 * kept.len().saturating_sub(1)
    };

    let mut keep: Vec<&crate::ui::metrics::MetricPart> = parts.iter().collect();
    // Drop the least important measurement: the last part that is not pinned.
    // Re-measured every step, so the budget can never drift from the text the loop
    // is about to emit.
    while width_of(&keep) > room {
        match keep.iter().rposition(|p| !p.pinned) {
            Some(i) => {
                keep.remove(i);
            }
            // Only the pinned tail is left and it still does not fit. Emit it
            // anyway: the layout's clamp will cut the model name, and a truncated
            // model name is more useful than a row that admits no measurement at
            // all. This is a width the product never draws — the frame reports
            // itself too small below 44 columns — so it exists to keep the
            // arithmetic total rather than to be seen.
            None => break,
        }
    }
    // Nothing survived the budget — an empty right half would leave a bare `idle`
    // and lose the one fact a reader always wants. Keep the last part.
    if keep.is_empty() {
        keep = parts.last().into_iter().collect();
    }

    let mut spans = Vec::with_capacity(keep.len() * 2);
    for (index, part) in keep.iter().enumerate() {
        if index > 0 {
            spans.push(Span::styled(" · ", theme.faint()));
        }
        spans.push(Span::styled(
            part.text.clone(),
            match part.role {
                MetricRole::Model => theme.user(),
                MetricRole::Value => theme.dim(),
                MetricRole::Warn => theme.warn(),
                MetricRole::Danger => theme.danger(),
                MetricRole::Label => theme.faint(),
            },
        ));
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn render(width: u16, state: &RunState, metrics: &Metrics) -> String {
        let theme = Theme::new(crate::ui::theme::ColorMode::Ansi16, crate::ui::theme::ASCII);
        let mut terminal = Terminal::new(TestBackend::new(width, 1)).expect("test terminal");
        terminal
            .draw(|f| draw(f, f.area(), &theme, state, metrics))
            .expect("draw");
        crate::ui::layout::row_text(terminal.backend().buffer(), 0, width)
            .trim_end()
            .to_string()
    }

    #[test]
    fn idle_and_running_read_differently() {
        let metrics = Metrics::default();
        let idle = render(100, &RunState::Idle, &metrics);
        assert!(idle.starts_with("idle"), "{idle}");
        let running = render(
            100,
            &RunState::Running {
                frame: 0,
                phase: "streaming".into(),
                elapsed_secs: 12,
            },
            &metrics,
        );
        assert!(running.contains("running 12s"), "{running}");
        assert!(running.contains("streaming"), "{running}");
    }

    /// **`/help` says the thinking level is in the status bar, so it is (M312).**
    ///
    /// The guide line reads *"thinking: Ctrl+T cycles · current level in the status bar"*, and
    /// the row printed no level — so the reader was told where to look and found nothing, and
    /// `Ctrl`+`T` looked broken. This is the third claim found in this session pointing at
    /// nothing, after `/theme`'s `high-contrast` palette and the `dropped` counter.
    ///
    /// **The level travels with the model and is spelled pi's way** (`off` is a word, every
    /// other level is the bare name), because that is the reference's own rule read from
    /// `components/footer.ts:233-243` rather than invented here. The model part is `pinned`, so
    /// the level is what survives on the narrowest row this product draws.
    ///
    /// Both spellings are asserted, because the failure mode is silence: a level that renders as
    /// nothing is indistinguishable from a level that was never set.
    #[test]
    fn the_thinking_level_rides_with_the_model_and_is_spelled_out_when_off() {
        let base = Metrics {
            model_label: "DeepSeek V3.1".into(),
            context_used: Some(48_000),
            context_window: Some(200_000),
            ..Metrics::default()
        };

        let off = render(
            140,
            &RunState::Idle,
            &Metrics {
                thinking: Some("off"),
                ..base.clone()
            },
        );
        assert!(
            off.contains("thinking off"),
            "`off` must be spelled: a bare `off` on a header row reads as a fragment of \
             something else.\n{off}"
        );

        let high = render(
            140,
            &RunState::Idle,
            &Metrics {
                thinking: Some("high"),
                ..base.clone()
            },
        );
        assert!(
            high.contains("DeepSeek V3.1 • high"),
            "a level in force is the bare name, beside the model — pi's wording, not ours.\n{high}"
        );

        // **No level, no claim.** A window with no session must not print one, or it describes
        // a turn that is not happening.
        let none = render(
            140,
            &RunState::Idle,
            &Metrics {
                thinking: None,
                ..base
            },
        );
        assert!(
            none.contains("DeepSeek V3.1") && !none.contains("thinking"),
            "the model alone, and nothing invented beside it.\n{none}"
        );

        // **It survives the narrowest row the product draws**, because it is on the pinned part
        // — a level that appears only when the terminal is wide is a level nobody reads.
        let narrow = render(
            60,
            &RunState::Idle,
            &Metrics {
                thinking: Some("high"),
                ..Metrics {
                    model_label: "DeepSeek V3.1".into(),
                    context_used: Some(48_000),
                    context_window: Some(200_000),
                    ..Metrics::default()
                }
            },
        );
        assert!(
            narrow.contains("high"),
            "the level must survive the budget the gauges yield to.\n{narrow}"
        );
    }

    #[test]
    fn measurements_sit_on_the_right_and_never_wrap() {
        let metrics = Metrics {
            model_label: "DeepSeek V3.1".into(),
            context_used: Some(48_000),
            context_window: Some(200_000),
            tokens_in: 12_400,
            tokens_out: 3_400,
            cache_read_session: 36_000,
            cache_read_last: 36_000,
            cost_micro_usd: Some(12_345),
            ..Metrics::default()
        };
        for width in [60u16, 80, 100, 140] {
            let line = render(
                width,
                &RunState::Running {
                    frame: 3,
                    phase: "reading".into(),
                    elapsed_secs: 4,
                },
                &metrics,
            );
            // The row may be shorter than the terminal (unwritten cells are
            // blank) but must never be longer, because that wraps.
            assert!(
                crate::ui::layout::display_width(&line) <= width as usize,
                "width {width} produced {} cells: {line:?}",
                crate::ui::layout::display_width(&line)
            );
            if width >= 100 {
                // At 100 columns there is room for everything, so the fragment
                // order is testable. Below that the right half is cut, and which
                // fragment survives is [`side_by_side`]'s business — asserted
                // there, by width breakpoint.
                assert!(line.contains("DeepSeek"), "width {width}: {line}");
                // The cache fragment is identified by its counters, not by the word
                // "cache" — the row reads `R12k W2.0k CH 75%`, and a test that pinned
                // the old label would pass against a row that had lost its rate.
                assert!(line.contains("CH"), "width {width}: {line}");
            }
        }
    }

    /// A gauge is dropped or kept — never cut in half.
    ///
    /// This is the defect the header had at 60 columns, and the reason it is
    /// worth a test that asserts on the *rendered row* rather than on the parts
    /// list: the parts were always correct. `metric_spans` emitted every gauge,
    /// `Metrics::parts` returned all six, and the loss happened in the layout's
    /// column-at-a-time trim. A test on either of those passes against the broken
    /// build, which is the whole reason the assertion is written against the
    /// frame.
    ///
    /// The markers are named individually rather than asserted as a set, because
    /// the failure mode is *silent absence*: at 60 columns the row read
    /// `idle 0 ↓0 · …` and every other test in this file was satisfied.
    #[test]
    fn a_narrow_terminal_drops_whole_gauges_rather_than_cutting_one() {
        let metrics = Metrics {
            model_label: "Faux (offline scripted)".into(),
            context_used: None,
            context_window: Some(1_000_000),
            tokens_in: 0,
            tokens_out: 0,
            cache_read_session: 0,
            cache_write_session: 0,
            ..Metrics::default()
        };
        for width in [40u16, 46, 52, 60, 72, 80, 100] {
            let line = render(width, &RunState::Idle, &metrics);
            assert!(
                !line.contains("0 ↓0") || line.contains("↑0 ↓0"),
                "width {width} printed a token gauge with no ↑ marker: {line:?}"
            );
            // `R0 W0 CH n/a` is one gauge, not three, and it is the first to go.
            assert!(
                !(line.contains("R0") && !line.contains("W0")),
                "width {width} printed a cache gauge with only some of its fields: {line:?}"
            );
            assert!(
                crate::ui::layout::display_width(&line) <= width as usize,
                "width {width} overflowed: {line:?}"
            );
        }
    }

    /// The pinned tail survives even when the model is drawn as a warning.
    ///
    /// The second version of the whole-gauge rule found the pin by
    /// [`MetricRole::Model`] — and an offline session draws the model name with
    /// `Warn`, because "no provider is configured" is something to notice. So the
    /// pin silently stopped matching in exactly the row long enough to need it: the
    /// rendered 60-column row read `… · R0 W0 CH n/a · Faux (offline scripted)`,
    /// with `offline` dropped and the model next in line. The parts list was
    /// correct throughout and every other assertion in this file passed; only the
    /// frame showed it.
    #[test]
    fn an_offline_session_keeps_the_model_and_its_qualifier_when_the_row_narrows() {
        let metrics = Metrics {
            model_label: "Faux (offline scripted)".into(),
            offline: true,
            context_window: Some(1_000_000),
            ..Metrics::default()
        };
        for width in [44u16, 52, 60, 72, 100] {
            let line = render(width, &RunState::Idle, &metrics);
            assert!(
                line.contains("Faux (offline scripted)"),
                "width {width} lost the model name: {line:?}"
            );
            assert!(
                line.ends_with("offline"),
                "width {width} lost the qualifier that says no provider is \
                 configured, which is what makes every other number on the row \
                 mean something different: {line:?}"
            );
            assert!(
                crate::ui::layout::display_width(&line) <= width as usize,
                "width {width} overflowed: {line:?}"
            );
        }
    }

    /// The tail of the measurements yields first, and the context gauge outlives
    /// the cache rate.
    ///
    /// The model name is the fragment that changes what every other number on the
    /// row means, so it is never the casualty of a narrow terminal. What goes
    /// before it is decided by [`Metrics::parts`]' order of questions, and the
    /// property this pins is the one the first version got backwards: dropping the
    /// *front* kept the tail, so a 60-column frame lost `ctx ?/1.0M` — how much room
    /// is left — and kept `R0 W0 CH n/a`, which says nothing at all when the
    /// provider cannot report a cache rate.
    #[test]
    fn a_narrow_terminal_keeps_the_model_and_the_context_gauge() {
        let metrics = Metrics {
            model_label: "DeepSeek V3.1".into(),
            context_used: Some(48_000),
            context_window: Some(200_000),
            tokens_in: 12_400,
            tokens_out: 3_400,
            cache_read_session: 36_000,
            cache_read_last: 36_000,
            cost_micro_usd: Some(12_345),
            ..Metrics::default()
        };
        let line = render(
            64,
            &RunState::Running {
                frame: 0,
                phase: "reading".into(),
                elapsed_secs: 4,
            },
            &metrics,
        );
        assert!(line.contains("DeepSeek"), "model name trimmed away: {line}");
        assert!(
            line.contains("running"),
            "run state must survive too: {line}"
        );
        // What a reader watches survives; the cache rate and the cost are what the
        // row spends its columns on instead.
        assert!(
            line.contains("ctx 24%/200k"),
            "the context gauge is the last measurement to go: {line}"
        );
        assert!(
            !line.contains("CH"),
            "the cache rate should have gone: {line}"
        );
        assert!(!line.contains('$'), "the cost should have gone: {line}");
        assert!(
            crate::ui::layout::display_width(&line) <= 64,
            "{} cells: {line:?}",
            crate::ui::layout::display_width(&line)
        );
    }

    #[test]
    fn a_narrow_terminal_spends_its_columns_on_the_run_state() {
        let metrics = Metrics {
            model_label: "DeepSeek V3.1".into(),
            context_used: Some(48_000),
            context_window: Some(200_000),
            ..Metrics::default()
        };
        for width in [16u16, 20, 24] {
            let line = render(
                width,
                &RunState::Running {
                    frame: 1,
                    phase: "streaming".into(),
                    elapsed_secs: 2,
                },
                &metrics,
            );
            assert!(
                crate::ui::layout::display_width(&line) <= width as usize,
                "width {width} overflowed: {line:?}"
            );
            // Whatever survives is the run state: the measurements are the half
            // that yields, and the tail of the state ("how long, doing what") is
            // worth more than its head.
            assert!(
                line.contains("streaming") || line.contains("2s") || line.contains("running"),
                "width {width} lost the run state: {line:?}"
            );
            assert!(
                !line.contains('%'),
                "width {width} kept measurements it had no room for: {line:?}"
            );
        }
    }
}
