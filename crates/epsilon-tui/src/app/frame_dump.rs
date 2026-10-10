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

/// The compaction digest, both states, at every width (plan 026 R2, round 106).
///
/// The slice's own frames: the collapsed row hiding the digest whole behind its
/// affordance, and the opened row showing it nested under the continuation
/// gutter. The `usage: None` arm is the legacy-session shape — a summary with no
/// bill is still a digest, and the row still opens.
#[test]
#[ignore = "a rendering fixture; run with --ignored --nocapture"]
fn frames_compaction_digest() {
    let (mut app, agent, _rx) = test_app();
    app.ui.push_local_input("compact");
    let summary = "The reader is porting the desktop's approval gate. The Enter                    routing defect is fixed and the honesty half is pinned. Nothing                    else is in flight.";
    app.ui.apply(&AgentEvent::CompactionFinished {
        usage: Some(Usage {
            input: 12_400,
            output: 310,
            cache_read: 8_900,
            cache_write: 0,
            total_tokens: 21_610,
            cache_reported: true,
        }),
        model_id: Some("test-model".into()),
        summary: summary.into(),
        tokens_before: 96_000,
    });
    shoot("compaction-collapsed", &mut app);

    use super::keys::handle_key;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    // The expand chord, through the real key path.
    handle_key(
        &mut app,
        &agent,
        KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL),
    );
    shoot("compaction-opened", &mut app);
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

/// **Tool cards, and a card that failed (M312).**
///
/// A tool call is the commonest thing in a transcript and the terminal has rendered one since
/// M63 — yet **no frame of one had ever been dumped here**, which is the same gap the desktop
/// turned out to have, in the other front end. One succeeded and one failed, because the failed
/// card's marker and colour are the two things a reader scans for.
///
/// The output arrives through `ToolExecutionUpdate`, which is the event that used to reach the
/// `_ => {}` arm and be dropped: a three-minute `cargo test` looked like a three-minute freeze
/// because nothing printed until the end.
#[test]
#[ignore = "a rendering fixture; run with --ignored --nocapture"]
fn frames_tool_cards() {
    // **The card is committed when the *assistant message* ends, not when the tool starts**
    // (`state.rs`, `ToolExecutionStart`: "The card was committed when the assistant message
    // ended"). The first version of this fixture sent tool events alone and the frame came out
    // with the prompt and twenty-four empty rows — which reads as "the terminal draws no tool
    // cards" and is not true. The first frame that looked like a defect turned out to be a
    // fixture that did not send what the product sends.
    let (mut app, _a, _r) = test_app();
    app.ui
        .push_local_input("run the suite and tell me what broke");
    let call = |id: &str, name: &str, args: serde_json::Value| {
        epsilon_contracts::content::ContentBlock::ToolCall {
            id: id.into(),
            name: name.into(),
            arguments: args,
        }
    };
    app.ui.apply(&AgentEvent::MessageEnd {
        message: Box::new(Message::Assistant(AssistantMessage {
            id: "m1".into(),
            timestamp_ms: 0,
            provider: "faux".into(),
            model_id: "faux-1".into(),
            content: vec![
                call("c1", "bash", serde_json::json!({ "command": "cargo test" })),
                call(
                    "c2",
                    "write",
                    serde_json::json!({ "path": "src/missing.rs" }),
                ),
            ],
            stop_reason: StopReason::ToolUse,
            error_message: None,
            usage: Usage::default(),
        })),
    });
    app.ui.apply(&AgentEvent::ToolExecutionStart {
        tool_call_id: "c1".into(),
        tool_name: "bash".into(),
        args: serde_json::json!({ "command": "cargo test -p epsilon-session" }),
    });
    app.ui.apply(&AgentEvent::ToolExecutionUpdate {
        tool_call_id: "c1".into(),
        tool_name: "bash".into(),
        partial: epsilon_contracts::tool::ToolPartial {
            content: vec![epsilon_contracts::content::ContentBlock::Text {
                text: "running 74 tests\ntest result: ok. 74 passed; 0 failed\n".into(),
            }],
            details: serde_json::json!({}),
        },
    });
    app.ui.apply(&AgentEvent::ToolExecutionEnd {
        tool_call_id: "c1".into(),
        tool_name: "bash".into(),
        is_error: false,
    });
    app.ui.apply(&AgentEvent::ToolExecutionStart {
        tool_call_id: "c2".into(),
        tool_name: "write".into(),
        args: serde_json::json!({ "path": "src/missing.rs" }),
    });
    app.ui.apply(&AgentEvent::ToolExecutionEnd {
        tool_call_id: "c2".into(),
        tool_name: "write".into(),
        is_error: true,
    });
    shoot("tool-cards", &mut app);
}

/// **The high-contrast palette, on the drawn frame (M312).**
///
/// That palette's claim is a set of numbers, and numbers are what every other check in this
/// repository reads — which is exactly how a palette can be right in a table and unreadable on a
/// screen. This draws it.
///
/// A terminal has no background of its own, so what the frame can show is the **foreground**
/// palette: body copy at pure white, the quiet roles legible rather than dimmed, and the outcomes
/// far apart in lightness. The page is the user's — which is the one thing a frame cannot supply,
/// and the reason the mode also exists on the desktop, where it paints black itself.
#[test]
#[ignore = "a rendering fixture; run with --ignored --nocapture"]
fn frames_high_contrast() {
    let (mut app, _a, _r) = test_app();
    app.theme
        .set_palette(crate::ui::theme::ThemeMode::HighContrast);
    app.ui
        .push_local_input("what does the contrast look like here?");
    for text in [
        "Pure white on black: 21:1.",
        "Quiet roles stay legible - the floor is not negotiable by a DIM attribute.",
        "failed gate: cargo test -p epsilon-session",
    ] {
        app.ui.push_notice(text);
    }
    app.input = "and the failed one?".into();
    app.cursor_chars = app.input.chars().count();
    shoot("high-contrast", &mut app);
}

/// **The command palette, open (M312).**
///
/// A full-screen overlay that `Ctrl`+`P` opens, with no rendered frame of it anywhere in the
/// repository — and the desktop side of this milestone found a header drawn 42 pixels off its own
/// layout the moment it was rendered, so "never looked at" is a finding waiting to happen rather
/// than a reassurance.
#[test]
#[ignore = "a rendering fixture; run with --ignored --nocapture"]
fn frames_command_palette() {
    let (mut app, _a, _r) = test_app();
    app.ui.push_local_input("type to filter");
    app.palette_open = true;
    shoot("palette-open", &mut app);
}

/// **The conversation's live edge, next to the composer (roadmap item 5).**
///
/// A transcript shorter than the viewport used to draw under the header and leave
/// the rest of the body blank, so the newest text sat as far from the input as the
/// screen allowed. The rule has three regimes and all three are drawn, because
/// the one that must *not* change — a window the reader scrolled back — is the
/// case a "make the body consistent" pass would break first:
///
/// 1. a short conversation, with the spare rows above it;
/// 2. a streaming answer, where the tail grows into the space;
/// 3. a scrolled-back window over a long transcript, still filled from the top.
#[test]
#[ignore = "a rendering fixture; run with --ignored --nocapture"]
fn frames_the_live_edge_sits_next_to_the_composer() {
    let (mut app, _a, _r) = test_app();
    app.ui.push_local_input("what changed?");
    app.ui.push_notice("here is the answer");
    app.input = "and the next thing?".into();
    app.cursor_chars = app.input.chars().count();
    shoot("live-edge-short", &mut app);

    let (mut app, _a, _r) = test_app();
    app.ui.push_local_input("run the tests");
    app.ui.apply(&AgentEvent::MessageUpdate {
        message: Box::new(Message::Assistant(AssistantMessage {
            id: "live-edge".into(),
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
        })),
    });
    shoot("live-edge-streaming", &mut app);

    let mut app = test_app().0;
    for index in 0..60 {
        app.ui.push_notice(format!("ROW-{index:03}"));
    }
    app.scroll_from_bottom = 5;
    shoot("live-edge-scrolled", &mut app);
}
