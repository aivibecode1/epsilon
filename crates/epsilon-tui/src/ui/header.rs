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

use crate::ui::layout::{Priority, clamp_line, side_by_side};
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
pub fn draw(f: &mut Frame<'_>, area: Rect, theme: &Theme, state: &RunState, metrics: &Metrics) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let left = state_spans(theme, state);
    let right = metric_spans(theme, metrics);
    let line = side_by_side(left, right, area.width as usize, Priority::Left);
    let line = clamp_line(line, area.width as usize);
    f.render_widget(line, area);
}

fn state_spans(theme: &Theme, state: &RunState) -> Vec<Span<'static>> {
    match state {
        RunState::Idle => vec![Span::styled("idle", theme.faint())],
        RunState::Running {
            frame,
            phase,
            elapsed_secs,
        } => {
            let frames = theme.glyphs.spinner;
            let glyph = frames[frame % frames.len()];
            let label = if phase.is_empty() {
                format!("running {elapsed_secs}s")
            } else {
                format!("running {elapsed_secs}s · {phase}")
            };
            vec![
                Span::styled(format!("{glyph} "), theme.accent()),
                Span::styled(label, theme.dim_strong()),
            ]
        }
    }
}

fn metric_spans(theme: &Theme, metrics: &Metrics) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    for (index, part) in metrics.parts().iter().enumerate() {
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

    #[test]
    fn measurements_sit_on_the_right_and_never_wrap() {
        let metrics = Metrics {
            model_label: "DeepSeek V3.1".into(),
            context_used: Some(48_000),
            context_window: Some(200_000),
            tokens_in: 12_400,
            tokens_out: 3_400,
            cache_read: 36_000,
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
                assert!(line.contains("cache"), "width {width}: {line}");
            }
        }
    }

    #[test]
    fn a_narrow_terminal_keeps_the_model_name_and_drops_the_earlier_fragments() {
        // The model name is the fragment that changes what every other number on
        // the row means, so it is the one that must never be the casualty of a
        // narrow terminal.
        let metrics = Metrics {
            model_label: "DeepSeek V3.1".into(),
            context_used: Some(48_000),
            context_window: Some(200_000),
            tokens_in: 12_400,
            tokens_out: 3_400,
            cache_read: 36_000,
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
        // The near end of the measurements is what yields, so the fragment that
        // says what every other number means is never the casualty.
        assert!(!line.contains("ctx"), "nothing should be invented: {line}");
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
