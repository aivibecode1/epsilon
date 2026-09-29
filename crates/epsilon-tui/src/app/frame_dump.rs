//! Render the real frame and read it.
//!
//! # Why this module exists
//!
//! At M198 the transcript painted every marker **twice** — `• • text`, `▸ ▸ bash`,
//! `✓ ✓ …` — because the gutter glyph and the body marker were the same character.
//! Nothing found it. It is not in the defect ledger, and the module that causes it
//! carried about 450 lines of documentation explaining how the gutter system works.
//! One `TestBackend` screenshot found it in a minute.
//!
//! That is the general shape of the failure this guards against: **a feature can be
//! correct at every level you test and still not exist on screen.** State-level
//! assertions are the cheapest tests to write and the least likely to notice that a
//! glyph is doubled, a badge is half-drawn at 60 columns, or a panel is painted
//! inside the wrong `if` arm. Two features were in exactly that state during the
//! M196 review — the `/tree` search field was drawn inside the *model picker's*
//! renderer, and the label editor inside `if app.auth_prompt_open` — and both had
//! passing state tests and had never once been rendered.
//!
//! So this is not a debugging aid left over from one milestone. It is the measuring
//! instrument the rest of the TUI work is done with, and it is here to stay.
//!
//! # How to use it
//!
//! ```text
//! cargo test -p epsilon-tui frame_dump -- --ignored --nocapture
//! ```
//!
//! Set `EPSILON_FRAME_DIR` to also write each frame to a file, which is how a
//! before/after pair ends up in `audit/evidence/` as something a reviewer can read
//! rather than take on trust:
//!
//! ```text
//! EPSILON_FRAME_DIR=audit/evidence/frames cargo test -p epsilon-tui frame_dump -- --ignored
//! ```
//!
//! The tests are `#[ignore]`d, which is not a way of keeping them out of sight: it
//! is the only way to have a rendering fixture in a suite whose whole value depends
//! on being readable. A dump test that ran on every battery would print a screen per
//! scenario and teach everyone to scroll past frames. The cost of running it is one
//! command, and the cost of *not* running it is the M198 class of defect.

use epsilon_contracts::agent_event::AgentEvent;
use epsilon_contracts::message::{AssistantMessage, Message, StopReason, Usage};

use super::tests_support::{render_rows, test_app};
use super::*;

/// The widths a person actually uses.
///
/// 100x30 and 80x24 are ordinary terminals. **60 is in the set because it is where
/// the layout breaks** — the M196 review found a footer badge rendering as a
/// *partial* badge at widths 26–31, which is worse than a missing one, because a
/// wrong number is not noticed the way an absent one is.
pub const WIDTHS: [(u16, u16); 3] = [(100, 30), (80, 24), (60, 20)];

/// Render one frame as a bordered text screen, and return it.
///
/// The border is not decoration: a row of text with no frame edge makes a
/// full-width row indistinguishable from a truncated one, which is exactly the
/// class of defect a dump exists to catch.
pub fn frame_text(name: &str, app: &mut App, w: u16, h: u16) -> String {
    let rows = render_rows(app, w, h);
    let rule = format!("+{}+", "-".repeat(w as usize));
    let mut out = format!("\n=== {name} @ {w}x{h} ===\n{rule}\n");
    for r in &rows {
        // A row can come back longer than the width if a writer ignored its budget.
        // Truncating hides that, so the overspill is stated rather than clipped.
        let taken: String = r.chars().take(w as usize).collect();
        let overflow = r.chars().count().saturating_sub(w as usize);
        if overflow > 0 {
            out.push_str(&format!(
                "!! row overflowed the width by {overflow} chars: {r:?}\n"
            ));
        }
        out.push_str(&format!("|{:<width$}|\n", taken, width = w as usize));
    }
    out.push_str(&rule);
    out.push('\n');

    if let Ok(dir) = std::env::var("EPSILON_FRAME_DIR") {
        // Relative paths resolve against the *package* root, because that is the
        // working directory a `cargo test` process is given — so a relative value
        // quietly lands in `crates/epsilon-tui/audit/…` instead of the repository.
        // Refusing is the honest answer, and the resolved path is printed on every
        // write so a dump can never be misplaced without saying so.
        let base = std::path::Path::new(&dir);
        if base.is_relative() {
            eprintln!(
                "frame dump not written: EPSILON_FRAME_DIR={dir} is relative, and a test \
                 process runs in the package root. Pass an absolute path."
            );
        } else {
            let _ = std::fs::create_dir_all(base);
            let path = base.join(format!("{name}.{w}x{h}.txt"));
            match std::fs::write(&path, &out) {
                Ok(()) => eprintln!("frame dump written: {}", path.display()),
                Err(e) => eprintln!("frame dump could not be written to {}: {e}", path.display()),
            }
        }
    }
    out
}

/// Print one frame at every width in [`WIDTHS`], and return what was printed.
pub fn shoot(name: &str, app: &mut App) -> String {
    let mut all = String::new();
    for (w, h) in WIDTHS {
        all.push_str(&frame_text(name, app, w, h));
    }
    print!("{all}");
    all
}

#[test]
#[ignore = "a rendering fixture; run with --ignored --nocapture"]
fn frames_idle_and_empty() {
    // A brand-new session: the first thing a user ever sees, and the frame with the
    // least code behind it, which is why it is the easiest one to get wrong.
    let (mut app, _a, _r) = test_app();
    shoot("brand-new", &mut app);

    let (mut app, _a, _r) = test_app();
    app.ui.push_local_input("explain the layout module");
    for text in [
        "The layout module owns the spatial model.",
        "It splits the frame into a header, a transcript, a composer and a footer.",
        "and it decides what happens when the terminal is too small.",
    ] {
        app.ui.push_notice(text);
    }
    app.input = "and the composer?".into();
    app.cursor_chars = app.input.chars().count();
    shoot("idle-with-content", &mut app);
}

#[test]
#[ignore = "a rendering fixture; run with --ignored --nocapture"]
fn frames_streaming() {
    let (mut app, _a, _r) = test_app();
    app.ui.push_local_input("run the tests");
    app.ui
        .push_notice("context compacted: 1,633 tokens dropped, summary 210 tokens");
    let msg = Message::Assistant(AssistantMessage {
        id: "frame-dump".into(),
        timestamp_ms: 0,
        provider: "faux".into(),
        model_id: "faux-1".into(),
        content: vec![epsilon_contracts::content::ContentBlock::text(
            "I will start by listing the files in the workspace and then read the layout module to understand how the frame is composed before making any changes to it."
                .to_string(),
        )],
        stop_reason: StopReason::EndTurn,
        error_message: None,
        usage: Usage::default(),
    });
    app.ui.apply(&AgentEvent::MessageUpdate {
        message: Box::new(msg),
    });
    app.input = "while you work - what is the plan".into();
    app.cursor_chars = app.input.chars().count();
    shoot("streaming", &mut app);
}

#[test]
#[ignore = "a rendering fixture; run with --ignored --nocapture"]
fn frames_permission_dialog() {
    use epsilon_safety::ApprovalUi;

    let (mut app, _a, _r) = test_app();
    app.ui.push_local_input("delete the build directory");
    let approvals = std::sync::Arc::new(epsilon_safety::ChannelApprovalUi::new());
    let h = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let ap2 = approvals.clone();
    h.spawn(async move {
        ap2.ask(epsilon_contracts::agent_event::PermissionRequest {
            request_id: "ask-1".into(),
            capability: "bash".into(),
            pattern: "rm -rf target".into(),
            detail: serde_json::json!({ "command": "rm -rf target" }),
        })
        .await
    });
    app.pending_ask = h.block_on(approvals.next_ask());
    shoot("permission", &mut app);
}
