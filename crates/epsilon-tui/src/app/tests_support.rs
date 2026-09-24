//! Rendered-frame helpers shared by the test modules.
//!
//! Two things live here rather than in each test file, because getting either
//! wrong makes a test lie rather than fail.
//!
//! # Reading a screen back
//!
//! `Cell::symbol()` returns the whole grapheme a cell holds, and ratatui
//! *coalesces* a run of identical adjacent cells into one cell whose symbol
//! repeats. Appending symbols therefore doubles repeated characters — a row
//! containing `100%` comes back as `100%%` — and a frame with a bug in it can
//! look fine while a correct one looks broken. [`row_text`] appends each cell's
//! symbol once and steps by its display width, which is what a screenshot of the
//! same frame would show.
//!
//! # Building an app
//!
//! [`test_app`] composes the same objects the CLI composes, in the same order,
//! because a test that builds a different graph is testing a different program.
//! The receivers are returned rather than dropped: dropping the sender is how the
//! run loop learns to stop, and a test that holds them keeps the wiring live.

use std::sync::Arc;

use epsilon_contracts::agent_event::PermissionRequest;
use epsilon_safety::{ApprovalUi, ChannelApprovalUi, RulesetGate};

use super::*;

/// The receiving ends of the app's channels, held so the senders stay alive.
pub type TestReceivers = (
    tokio::sync::mpsc::UnboundedReceiver<(String, Result<Vec<String>, String>)>,
    tokio::sync::mpsc::UnboundedReceiver<String>,
);

/// An app wired like the product's, for tests that need the real structure.
pub fn test_app() -> (App, Arc<Agent>, TestReceivers) {
    let (ftx, frx) = tokio::sync::mpsc::unbounded_channel();
    let (ntx, nrx) = tokio::sync::mpsc::unbounded_channel();
    let (otx, _orx) = tokio::sync::mpsc::unbounded_channel();
    let approvals = Arc::new(ChannelApprovalUi::new());
    let gate = Arc::new(RulesetGate::new(
        Arc::clone(&approvals) as Arc<dyn epsilon_safety::ApprovalUi>
    ));
    let agent = Agent::new(
        epsilon_core::LoopLimits::default(),
        Arc::new(epsilon_llm::PanicGuard(epsilon_llm::FauxProvider::new(
            vec![],
        ))),
        Arc::new(epsilon_core::ToolRegistry::new()),
        gate,
        std::env::temp_dir(),
    );
    (App::new("faux-1".into(), ftx, ntx, otx), agent, (frx, nrx))
}

/// One screen row as text: every cell's symbol once, stepping by its width.
pub fn row_text(buffer: &ratatui::buffer::Buffer, y: u16, width: u16) -> String {
    let mut out = String::new();
    let mut x = 0u16;
    while x < width {
        let symbol = buffer[(x, y)].symbol();
        out.push_str(symbol);
        x += crate::ui::layout::display_width(symbol).max(1) as u16;
    }
    out
}

/// Draw one frame with the production draw path and read the screen back.
///
/// Uses [`super::draw`] — the same function the event loop calls — so a widget
/// that renders correctly in isolation but cannot be assembled into a frame is
/// caught here rather than by a user.
pub fn render_rows(app: &mut App, width: u16, height: u16) -> Vec<String> {
    render_buffer(app, width, height).1
}

/// The same draw, but keep the buffer.
///
/// Style is not visible in the text: a filled band, a gutter glyph's colour and a
/// row's modifiers live on the cells. A test that only reads the text back cannot
/// tell a decorated frame from an undecorated one, which is how the transcript's
/// whole decoration system stayed dead code behind a green suite.
pub fn render_buffer(
    app: &mut App,
    width: u16,
    height: u16,
) -> (ratatui::buffer::Buffer, Vec<String>) {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("test terminal");
    terminal
        .draw(|frame| draw(frame, app))
        .expect("the draw path must not panic");
    let buffer = terminal.backend().buffer().clone();
    let rows = (0..height).map(|y| row_text(&buffer, y, width)).collect();
    (buffer, rows)
}

/// Park one ask on the shared queue and hand it to the app, exactly like the run
/// loop does.
///
/// The requester future is spawned and deliberately leaked: dropping it would
/// resolve the ask to Deny, and a test that wanted the *dialog* would then be
/// testing a state the app never reaches. The returned handle is the answer, so
/// a test that does care about the decision can await it.
pub async fn park_ask(
    app: &mut App,
    approvals: &ChannelApprovalUi,
    id: &str,
) -> tokio::task::JoinHandle<epsilon_contracts::agent_event::PermissionDecision> {
    let requester = approvals.clone();
    let id = id.to_string();
    let response = tokio::spawn(async move {
        requester
            .ask(PermissionRequest {
                request_id: id,
                capability: "bash".into(),
                pattern: "rm -rf build".into(),
                detail: serde_json::json!({ "command": "rm -rf build" }),
            })
            .await
    });
    app.pending_ask = approvals.next_ask().await;
    response
}
