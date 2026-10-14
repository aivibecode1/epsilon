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

/// **A tool's output arriving while it runs.** The M152 fix, on screen.
///
/// The events are driven through `apply_with_scroll` — the real event loop's path — and
/// the frame is painted *between* the chunks, because a card that has never been drawn is
/// built fresh on the first frame with whatever body it already has. That is the whole
/// point: the defect this fixture exists to show was a cache that invalidated only
/// finished cards, so the running one kept the rows built when it opened.
#[test]
#[ignore = "a rendering fixture; run with --ignored --nocapture"]
fn frames_streaming_tool_output() {
    use epsilon_contracts::agent_event::AgentEvent;
    let (mut app, _a, _r) = test_app();
    app.ui.push_local_input("cargo build");
    // The header's `RunState` is fed by `AgentStart` (`app.rs:1767`). Without it the
    // frame captured a header reading `idle` above a card reading `pending` — two rows
    // of one screen describing different worlds, in a fixture named for a tool that is
    // running. The frame is evidence; it has to describe the state it is named for.
    app.ui.apply(&AgentEvent::AgentStart);
    app.ui.apply(&AgentEvent::MessageEnd {
        message: Box::new(Message::Assistant(AssistantMessage {
            id: "m-stream".into(),
            timestamp_ms: 0,
            provider: "faux".into(),
            model_id: "faux-1".into(),
            content: vec![epsilon_contracts::content::ContentBlock::ToolCall {
                id: "c1".into(),
                name: "bash".into(),
                arguments: serde_json::json!({ "command": "cargo build" }),
            }],
            stop_reason: StopReason::ToolUse,
            error_message: None,
            usage: Usage::default(),
        })),
    });
    super::apply_with_scroll(
        &mut app,
        &AgentEvent::ToolExecutionStart {
            tool_call_id: "c1".into(),
            tool_name: "bash".into(),
            args: serde_json::json!({ "command": "cargo build" }),
        },
    );
    shoot("streaming-tool-running", &mut app);
    for chunk in [
        "Compiling epsilon-core v0.1.0",
        "Compiling epsilon-tools v0.1.0",
    ] {
        super::apply_with_scroll(
            &mut app,
            &AgentEvent::ToolExecutionUpdate {
                tool_call_id: "c1".into(),
                tool_name: "bash".into(),
                partial: epsilon_contracts::tool::ToolPartial {
                    content: vec![epsilon_contracts::content::ContentBlock::Text {
                        text: chunk.into(),
                    }],
                    details: serde_json::json!({}),
                },
            },
        );
        shoot("streaming-tool-output", &mut app);
    }
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

/// **The search highlight, on a real frame (P-218).**
///
/// The panel counted matches for as long as it existed while the transcript
/// showed none of them. This is the picture that ends: both hits underlined, the
/// one you are on reversed, and every other row untouched — which is the only
/// way to see that the mark lands on the *word* and not on the row, and that the
/// live-edge padding above it is not dragged into the mark.
#[test]
#[ignore = "a rendering fixture; run with --ignored --nocapture"]
fn frames_the_search_marks_its_matches() {
    let (mut app, _a, _r) = test_app();
    for text in [
        "the Needle sits in the first row",
        "a row with nothing to find in it",
        "and a second NEEDLE down here",
    ] {
        app.ui.push_notice(text);
    }
    app.search_own_transcript("needle");
    shoot("search-marks", &mut app);

    // Stepped to the second hit: the two marks swap, so the frame shows the
    // feature is a state of the search rather than a property of the row.
    app.search_step(true);
    shoot("search-marks-second", &mut app);
}

/// **Halfwidth katakana wrapping a row the buffer can fill — the sound-mark rule.**
///
/// `unicode-width` reports `ﾞ` (U+FF9E) and `ﾟ` (U+FF9F) as zero-width, terminals
/// draw them as independent halfwidth cells, so ratatui's `CellWidth for str`
/// counts one column each. While `wrap.rs` spent the uncorrected rule the
/// wrapper believed it had columns ratatui did not give it, and the renderer
/// clipped the last character of every line — silently, in the direction where
/// we *under*-measure.
///
/// This frame is the witness that both ends now use one number. `shoot` renders it
/// at 100×30, 80×24 and 60×20 — the disagreement only shows when a mark lands near
/// a row boundary, so a single width would not find it — and `frame_text` states a
/// row's overspill rather than clipping it, so an over-long row cannot be hidden by
/// the presentation. `display_width_agrees_with_the_rule_ratatui_places_cells_by`
/// proves the measurement matches placement; this proves the *wrapped row* obeys it.
///
/// # How to read this frame, and the trap in doing so
///
/// Each katakana row must fill its frame **exactly**. The trap is that
/// `frame_text`'s own overspill guard counts `char`s while the bug is expressed in
/// *columns*, and the two are not interchangeable here — so verify by the identity
/// ratatui's `CellWidth for str` defines rather than by eyeballing:
///
/// ```text
/// cell_width = UnicodeWidthStr::width + count(ﾞ, ﾟ)
///            = (char_count - marks) + marks
///            = char_count
/// ```
///
/// because the marks are the only zero-width characters in these rows. So the row
/// is in budget exactly when its char count equals the frame width — which is what
/// the rows above do. I first measured this wrong (adding the marks to a char count
/// that already contained them) and read a correct frame as 119 columns wide; the
/// arithmetic, not the code, was the defect. Measure the number, not the intuition.
#[test]
#[ignore = "a rendering fixture; run with --ignored --nocapture"]
fn frames_halfwidth_katakana_wraps_within_the_column_the_buffer_has() {
    let (mut app, _a, _r) = test_app();
    app.ui.push_local_input("cargo check");
    app.ui.apply(&AgentEvent::MessageEnd {
        message: Box::new(Message::Assistant(AssistantMessage {
            id: "m-kana".into(),
            timestamp_ms: 0,
            provider: "faux".into(),
            model_id: "faux-1".into(),
            content: vec![epsilon_contracts::content::ContentBlock::ToolCall {
                id: "k1".into(),
                name: "bash".into(),
                arguments: serde_json::json!({ "command": "cat build.log" }),
            }],
            stop_reason: StopReason::ToolUse,
            error_message: None,
            usage: Usage::default(),
        })),
    });
    super::apply_with_scroll(
        &mut app,
        &AgentEvent::ToolExecutionStart {
            tool_call_id: "k1".into(),
            tool_name: "bash".into(),
            args: serde_json::json!({ "command": "cat build.log" }),
        },
    );
    // The rows go through `render_cache`, which is the real draw path into
    // `wrap_line` — a notice row would not have exercised it.
    super::apply_with_scroll(
        &mut app,
        &AgentEvent::ToolExecutionUpdate {
            tool_call_id: "k1".into(),
            tool_name: "bash".into(),
            partial: epsilon_contracts::tool::ToolPartial {
                content: vec![epsilon_contracts::content::ContentBlock::Text {
                    text: "\u{FF76}\u{FF9E}".repeat(40).to_string(),
                }],
                details: serde_json::json!({}),
            },
        },
    );
    shoot("katakana-sound-marks", &mut app);
}

/// **The file editor's own undo, on a frame the reader can check.**
///
/// Two things this makes visible that no state assertion can: that `Ctrl+Z` reaches
/// the *file* buffer (not the composer's, which has had undo since M16), and that the
/// rendered row is what was restored — a delta that lands mid-word is only obvious
/// when the word is drawn.
///
/// The first frame is the edit, the second is after `Ctrl+Z`. They must differ, and
/// the second must match the file as opened.
#[test]
#[ignore = "a rendering fixture; run with --ignored --nocapture"]
fn frames_the_file_editors_undo_restores_the_drawn_row() {
    let (mut app, _a, _r) = test_app();
    let dir = std::env::temp_dir().join(format!("epsilon-frame-undo-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("undo-demo.txt");
    std::fs::write(&path, "the quick brown fox\n").unwrap();

    // Open the viewer on that file: `open_viewer` refuses a directory and a binary,
    // so this is the real construction path rather than a hand-built struct.
    let viewer =
        crate::tree::open_viewer(dir.as_path(), "undo-demo.txt").expect("the demo file opens");
    app.open_viewer_tab(viewer);
    app.ui.push_local_input("undo-demo");
    {
        let v = app.active_viewer_mut().expect("the tab is active");
        assert!(v.start_edit(), "the file is small enough to edit");
        v.edit_col = v.edit_lines[0].chars().count();
        v.insert_char('!');
        v.clamp_cursor();
    }
    let edited = shoot("file-editor-undone-edit", &mut app);

    // Undo, and draw again: the row comes back to the file as opened.
    assert!(app.active_viewer_mut().expect("still open").undo());
    let restored = shoot("file-editor-undone-restored", &mut app);

    assert_ne!(
        edited, restored,
        "the two frames must differ — an undo that changes nothing is not an undo"
    );
    assert!(
        edited.contains("fox!"),
        "the edited frame carries the character that was typed: {edited}"
    );
    assert!(
        restored.contains("fox") && !restored.contains("fox!"),
        "the restored frame is the file as opened: {restored}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// **Find works in the file editor, not just over the opened file.**
///
/// `/` in edit mode searches the buffer being typed into, and `Enter` steps the
/// *caret* to the match — a highlighted line the caret cannot reach is a
/// decoration. Before this the key typed a literal slash into the text: the
/// edit-mode arm `return false`d before the find arm could run, and the hit list
/// was computed from `lines` (the file as opened), so it named text the user had
/// already changed.
#[test]
#[ignore = "a rendering fixture; run with --ignored --nocapture"]
fn frames_find_in_the_file_editor_moves_the_caret() {
    let (mut app, _a, _r) = test_app();
    let dir = std::env::temp_dir().join(format!("epsilon-frame-efind-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    std::fs::write(
        dir.join("find.txt"),
        "a needle here\nnothing\nand a needle again\n",
    )
    .unwrap();
    let viewer = crate::tree::open_viewer(dir.as_path(), "find.txt").expect("opens");
    app.open_viewer_tab(viewer);
    {
        let v = app.active_viewer_mut().expect("active");
        assert!(v.start_edit());
        v.find_open();
        v.find_query = "needle".into();
        v.find_recompute();
    }
    // Step to the second hit: the caret must land on that line, on the match.
    app.active_viewer_mut().expect("active").find_step(true, 24);
    let v = app.active_viewer().expect("active");
    assert_eq!(v.edit_row, 2, "the caret follows the hit");
    assert_eq!(v.edit_col, 6, "and lands on the match, not the line start");

    let frame = shoot("file-editor-find", &mut app);
    let _ = frame; // the frame is written for a reader; the assertions above are the proof.
    let _ = std::fs::remove_dir_all(&dir);
}

/// **The find line survives a body longer than the panel.**
///
/// The row budget is what this guards: the body was painted into `capacity` rows
/// and the find line, the replacement line and the hint row were appended after a
/// pad to `capacity`, so they sat past the edge of a `capacity + 1` area. Which of
/// them survived depended on how long the body was — a long file lost its query
/// line and a short one kept it, so the same code drew two different screens.
///
/// This one is *long on purpose*: the file has more lines than the panel holds, so
/// the query line is only on screen if the budget was reserved.
#[test]
#[ignore = "a rendering fixture; run with --ignored --nocapture"]
fn frames_the_find_line_survives_a_body_longer_than_the_panel() {
    let (mut app, _a, _r) = test_app();
    let dir = std::env::temp_dir().join(format!("epsilon-frame-efind2-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    // Far more lines than a 24-row panel can show, so the body fills the window.
    let long: String = (0..200)
        .map(|i| format!("line {i} of the body\n"))
        .collect();
    std::fs::write(dir.join("big.txt"), &long).unwrap();
    let viewer = crate::tree::open_viewer(dir.as_path(), "big.txt").expect("big opens");
    app.open_viewer_tab(viewer);
    {
        let v = app.active_viewer_mut().expect("active");
        // Edit mode, because the replacement input is only offered where a
        // replacement can be made — in the buffer, not in the file as opened.
        v.start_edit();
        v.find_open();
        v.find_query = "line 42".into();
        v.replace_toggle();
        v.replace_query = "with".into();
        v.find_recompute();
    }
    let frame = shoot("file-editor-find-long", &mut app);
    // Scoped to the panel: a row inside the box, identified by the gutter glyph
    // that only panel body rows carry. The earlier version checked for "esc close",
    // which also appears in the *application footer* outside the panel and so could
    // never fail — an assertion that can pass for the wrong reason is worse than none.
    let panel_row = |needle: &str| {
        frame
            .lines()
            .filter(|l| l.contains('\u{2502}'))
            .any(|l| l.contains(needle))
    };
    assert!(
        panel_row("/line 42"),
        "the query line must be drawn inside the panel even when the body fills it: {frame}"
    );
    // The replacement input the feature was withdrawn over. It is asserted here,
    // on the frame, because "it is in the data" is exactly the claim that has cost
    // this repository two features.
    assert!(
        panel_row(">with"),
        "the replacement line must be drawn too: {frame}"
    );
    // The hint row's wording differs by mode (edit mode names save, not scroll),
    // so what is checked is that *some* hint row is inside the panel — the
    // overrun that took this row out took the replacement line with it.
    assert!(
        panel_row("esc cancel"),
        "and the panel's own hint row: {frame}"
    );
    let _ = std::fs::remove_dir_all(&dir);
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
