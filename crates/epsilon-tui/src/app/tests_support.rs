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
    let mut app = App::new(
        "faux-1".into(),
        ftx,
        ntx,
        otx,
        // Today's behavior, pinned: tests run against the launch cwd
        // unless they override `workspace_root` afterwards (most do).
        // Production passes the CLI-resolved workspace instead.
        std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")),
    );
    // Pinned, not loaded: `App::new` reads the saved default from global
    // config, which is the developer's own on a dev machine — a test that
    // inherited it would order and mark rows by whoever ran it. Tests that
    // want a default set `app.default_model` explicitly.
    app.default_model = None;
    // Every config write from this fixture goes to its own directory. The
    // explorer's visibility now persists on F6, so without this a binding test
    // would rewrite the developer's `[tui] explorer`; the same guard covers
    // favourites, presets and thinking. Tests that need the real resolver path
    // set this to `None` explicitly.
    app.config_dir_override = Some(tempfile::tempdir().expect("test config dir").keep());
    (app, agent, (frx, nrx))
}

/// An app wired like the product's, with a **scripted** faux provider.
///
/// [`test_app`] hands the agent `FauxProvider::new(vec![])` — an empty script —
/// so the very first turn falls back to "out of scripted steps". That is the
/// right default for a fixture nobody asked to converse with, and it is why
/// every `run_one_turn` in the suite produces exactly one assistant row of
/// disclaimer text and nothing else.
///
/// A test that needs to *look at* a conversation needs one, so this hands over
/// the script. Same composition, same order, same `PanicGuard` wrapper — the
/// only difference from [`test_app`] is what the provider has been told to say,
/// which is the whole point and the only thing changed.
pub fn test_app_with_faux(steps: Vec<epsilon_llm::FauxStep>) -> (App, Arc<Agent>, TestReceivers) {
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
            steps,
        ))),
        Arc::new(epsilon_core::ToolRegistry::new()),
        gate,
        std::env::temp_dir(),
    );
    let mut app = App::new(
        "faux-1".into(),
        ftx,
        ntx,
        otx,
        std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")),
    );
    app.default_model = None;
    app.config_dir_override = Some(tempfile::tempdir().expect("test config dir").keep());
    (app, agent, (frx, nrx))
}

/// An app whose agent has a **durable session file**, plus the `TempDir` that owns it.
///
/// The goal lives in the session log and nowhere else, so a `/goal` test on an agent with no
/// store would exercise the refusal path — a real case, with its own test — instead of the
/// behaviour it means to check.
/// An app whose store sits where a real launch puts it: the sessions root, the per-working-copy
/// directory under it, and a named file inside that.
///
/// **M222: this fixture used to plant `<temp>/session.jsonl`** — not in a sessions tree at all —
/// so every line of code that reasons about *where* a session lives was exercised only by accident,
/// and the one place that derives the root from the directory name (A17's notice) was never
/// exercised at all. A fixture that misrepresents production is how a layout rule goes untested.
pub fn test_app_with_store() -> (App, Arc<Agent>, TestReceivers, tempfile::TempDir) {
    let (mut app, agent, rx) = test_app();
    let dir = tempfile::tempdir().expect("a temporary workspace");
    app.workspace_root = dir.path().to_path_buf();
    let store = open_workspace_session(&app.workspace_root, "session.jsonl");
    agent.attach_session_store(Arc::new(std::sync::Mutex::new(store)));
    (app, agent, rx, dir)
}

/// The file a launch in `workspace` would write, named `name` — the sessions root, the per-cwd
/// directory, and the file inside it, with pi's naming rule applied by the caller.
///
/// The directory is created; the store's header lands in it on open.
pub fn open_workspace_session(
    workspace: &std::path::Path,
    name: &str,
) -> epsilon_session::SessionStore {
    let sessions = workspace.join(".epsilon").join("sessions");
    let per_cwd = epsilon_session::time::session_dir_for(&sessions, workspace);
    std::fs::create_dir_all(&per_cwd).expect("the per-workspace session directory");
    epsilon_session::SessionStore::open_in(per_cwd.join(name), workspace).expect("a session opens")
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

/// An app wired like the product's, **with the `bash` tool registered**, plus the
/// approval channel a `bash` call is gated behind.
///
/// [`test_app`] deliberately hands back an empty registry, which is what most
/// tests want. It is the wrong fixture for the `!` shell escape, for TWO reasons,
/// and the second is the one that bites:
///
/// 1. With no `bash` registered, `exec::run_local` answers "no tool named bash",
///    and a test that only asserted on the notice text would accept that as a
///    command having run.
/// 2. With a `bash` registered and nobody answering the gate, `run_local` blocks
///    on an approval prompt forever, and the test sees no output at all. Which is
///    correct product behaviour — `!` goes through the capability gate, so a user
///    can run something the agent could not.
///
/// So the channel comes back too, for a test to answer on, the way a user does.
pub fn test_app_with_bash() -> (App, Arc<Agent>, TestReceivers, ChannelApprovalUi) {
    let (ftx, frx) = tokio::sync::mpsc::unbounded_channel();
    let (ntx, nrx) = tokio::sync::mpsc::unbounded_channel();
    let (otx, _orx) = tokio::sync::mpsc::unbounded_channel();
    let approvals = Arc::new(ChannelApprovalUi::new());
    let gate = Arc::new(RulesetGate::new(
        Arc::clone(&approvals) as Arc<dyn epsilon_safety::ApprovalUi>
    ));
    let mut registry = epsilon_core::ToolRegistry::new();
    registry.register(Arc::new(epsilon_tools::BashTool::new()));
    let agent = Agent::new(
        epsilon_core::LoopLimits::default(),
        Arc::new(epsilon_llm::PanicGuard(epsilon_llm::FauxProvider::new(
            vec![],
        ))),
        Arc::new(registry),
        gate,
        std::env::temp_dir(),
    );
    // A CLONE, not a second `new()`: every clone of `ChannelApprovalUi` feeds the
    // same queue, so the test's answers reach the gate the tool is blocked on. A
    // fresh instance would be a different queue, and the command would hang there
    // too — with the test unable to see why.
    (
        App::new(
            "faux-1".into(),
            ftx,
            ntx,
            otx,
            std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")),
        ),
        agent,
        (frx, nrx),
        (*approvals).clone(),
    )
}

/// Park a model question on a local hub and seat it in this app, returning
/// the waiter's handle. Shared by the overlay tests (ask.rs) and the footer
/// tests (view.rs) so the two cannot drift into different parkings: a local
/// hub and a per-app slot mean parallel tests never touch each other's
/// overlays *or* pendings.
pub async fn park_question(
    app: &mut App,
    prompts: Vec<epsilon_contracts::question::QuestionPrompt>,
) -> (
    std::sync::Arc<epsilon_runtime::QuestionHub>,
    tokio::task::JoinHandle<
        Result<
            Vec<epsilon_contracts::question::QuestionAnswer>,
            epsilon_contracts::question::QuestionError,
        >,
    >,
) {
    let hub = std::sync::Arc::new(epsilon_runtime::QuestionHub::new());
    hub.set_operator_present(true);
    let waiter = tokio::spawn({
        let hub = std::sync::Arc::clone(&hub);
        async move {
            use epsilon_contracts::question::QuestionService;
            hub.ask("s".into(), prompts).await
        }
    });
    while hub.pending_count() == 0 {
        tokio::task::yield_now().await;
    }
    super::ask::seat_for_test(app, hub.take_pending().expect("the parked question"));
    (hub, waiter)
}
