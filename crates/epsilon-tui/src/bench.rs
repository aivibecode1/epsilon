//! `--tui-bench <n>`: headless rendering benchmark (M16a).
//!
//! Builds `n` synthetic transcript messages, then drives the REAL draw pipeline
//! (`app::draw_frame`) against a ratatui [`TestBackend`] and reports wall-clock per
//! frame. This is the honesty meter for the M16a efficiency work: numbers come from
//! this module, before AND after the render-cache change, and land in the dev log.

use std::time::Instant;

use epsilon_contracts::agent_event::AgentEvent;
use epsilon_contracts::ids::new_id;
use epsilon_contracts::message::{AssistantMessage, Message, StopReason, ToolResultMessage, Usage};
use ratatui::Terminal;
use ratatui::backend::TestBackend;

use crate::state::{LineKind, UiState};

/// Frames timed after warmup. High enough to amortize scheduler noise; low enough
/// that the whole bench stays under ~10s even on slow pipelines.
const FRAMES: usize = 300;

/// Terminal size used for every timed frame.
const COLS: u16 = 110;
const ROWS: u16 = 40;

fn assistant_msg(text: &str) -> AssistantMessage {
    AssistantMessage {
        id: new_id("msg"),
        timestamp_ms: 0,
        provider: "faux".into(),
        model_id: "faux-1".into(),
        content: vec![epsilon_contracts::content::ContentBlock::text(
            text.to_string(),
        )],
        stop_reason: StopReason::EndTurn,
        error_message: None,
        usage: Usage {
            input: 1200,
            output: 340,
            cache_read: 9000,
            cache_write: 1500,
            ..Default::default()
        },
    }
}

fn tool_result(name: &str) -> Message {
    Message::ToolResult(ToolResultMessage {
        id: new_id("msg"),
        timestamp_ms: 0,
        tool_call_id: format!("tc_{name}"),
        tool_name: name.to_string(),
        content: vec![epsilon_contracts::content::ContentBlock::text(format!(
            "{name}: ok — 42 lines written to src/example_{name}.rs"
        ))],
        details: serde_json::json!({}),
        usage: None,
        is_error: false,
    })
}

/// A long paragraph so markdown re-parsing cost is visible in the BEFORE numbers.
const MD_PARAGRAPH: &str = "## Section\n\nThe **cache** prefix must stay byte-stable \
because provider pricing treats it differently — see `ADR-008` for the tier contract and \
the *volatility* guard tests.\n\n- first item with some prose attached\n- second item\n\
```rust\nfn main() { println!(\"hi\"); }\n```\n";

/// Populate a [`UiState`] with `n` assistant messages + interleaved tool results by
/// replaying synthetic [`AgentEvent`]s through the normal reducer.
pub fn build_synthetic_transcript(ui: &mut UiState, n: usize) {
    ui.apply(&AgentEvent::AgentStart);
    for i in 0..n {
        let mut msg = assistant_msg(&format!("Working on step {i} of the plan. {MD_PARAGRAPH}"));
        if i % 3 == 0 {
            msg.content
                .push(epsilon_contracts::content::ContentBlock::ToolCall {
                    id: format!("tc_bench_{i}"),
                    name: "edit".into(),
                    arguments: serde_json::json!({ "path": "src/lib.rs", "change": i }),
                });
        }
        ui.apply(&AgentEvent::MessageEnd {
            message: Box::new(Message::Assistant(msg)),
        });
        ui.apply(&AgentEvent::MessageEnd {
            message: Box::new(tool_result("edit")),
        });
    }
    ui.apply(&AgentEvent::AgentEnd { messages: vec![] });
}

/// Run the benchmark: returns printable report lines (already formatted).
///
/// The caller owns printing; keeping this sync makes timing deterministic and lets
/// tests assert on the shape without capturing stdout.
pub fn run(n_messages: usize) -> Vec<String> {
    let mut ui = UiState::new();
    let t_build = Instant::now();
    build_synthetic_transcript(&mut ui, n_messages);
    let build_ms = t_build.elapsed().as_millis();

    let kinds: Vec<LineKind> = ui.lines().iter().map(|l| l.kind).collect();
    let md_rows = kinds.iter().filter(|k| **k == LineKind::Assistant).count();

    // One App whose caches persist across frames — the production shape.
    let mut app = crate::app::bench_app(ui);

    let backend = TestBackend::new(COLS, ROWS);
    let mut terminal = match Terminal::new(backend) {
        Ok(t) => t,
        Err(e) => return vec![format!("bench aborted: cannot build terminal: {e}")],
    };

    // Warmup (allocator, first render of every cached line, font tables).
    let warm = Instant::now();
    for _ in 0..5 {
        let _ = terminal.draw(|f| crate::app::draw_frame(f, &mut app));
    }
    let warm_us = warm.elapsed().as_micros();

    let start = Instant::now();
    for _ in 0..FRAMES {
        let _ = terminal
            .draw(|f| crate::app::draw_frame(f, &mut app))
            .unwrap_or_else(|e| panic!("draw failed mid-bench: {e}"));
    }
    let total_us = start.elapsed().as_micros().max(1);
    let per_frame = total_us / FRAMES as u128;
    let fps = 1_000_000.0 / total_us as f64 * FRAMES as f64;

    vec![
        format!(
            "messages={n_messages} rows={kinds_len}",
            kinds_len = kinds.len()
        ),
        format!("of which markdown rows: {md_rows}"),
        format!("transcript build time: {build_ms}ms"),
        format!("warmup (5 draws): {warm_us}µs"),
        format!("timed draws: {FRAMES} × ({COLS}x{ROWS})"),
        format!("total: {total_us}µs → {per_frame}µs/frame (~{fps:.0} fps equivalent)"),
        format!(
            "frame budget at TICK_MS tick: 120000µs — per-frame share: {}%",
            per_frame as f64 / 1200.0
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synthetic_transcript_produces_expected_shape() {
        let mut ui = UiState::new();
        build_synthetic_transcript(&mut ui, 3);
        // run-start + per message (assistant + thinking? no — assistant+tool) +
        // tool-call rows on every 3rd message + run-end.
        // 3 msgs → start, 3×(assistant), 1×tool-call (i=0 only), 3×tool, end = 9.
        assert_eq!(ui.lines().len(), 9);
        assert!(ui.lines().iter().any(|l| l.md.is_some()));
    }

    #[test]
    fn bench_report_has_numbers() {
        let report = run(4);
        assert!(report.len() >= 6);
        assert!(report[0].contains("messages=4"));
        assert!(report.iter().any(|l| l.contains("µs/frame")));
    }
}
