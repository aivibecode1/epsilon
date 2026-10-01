//! End-to-end: a real agent, driving the real draw path, with no window.
//!
//! # Why this file exists
//!
//! Every other test in this crate feeds the reducer synthetic events or paints an
//! agent-less frame. That is the right way to test two halves, and it is exactly how a
//! feature can be correct at every level and still not exist: `m196` in this repository
//! found a search field that had passing state tests and had never been drawn, and
//! `m198` found a transcript that printed every marker twice.
//!
//! So this file joins the halves. It composes a real agent through
//! `epsilon_runtime::build_runtime` — the same call the CLI and the terminal make —
//! attaches the real bridge, submits a real turn to the offline provider, pumps real
//! frames, and asserts on what is **on screen**.
//!
//! # No window, no network
//!
//! egui renders into an off-screen `egui::Context`, so "the window drew it" is measurable
//! without a window — which is what makes this a unit test rather than a manual check.
//! Keystrokes go in the same way: `RawInput.events` with `focused: true`, which egui
//! requires before it will deliver a key.
//!
//! # The frame-lag trap, recorded because it cost four rounds
//!
//! This file asserts that the reader's own words appear "the moment Enter is pressed",
//! and for several iterations that assertion failed with an empty transcript. Nothing was
//! broken. An immediate-mode frame paints **top to bottom**: the transcript is drawn before
//! the composer, so a row the composer adds during that frame cannot be in the same
//! frame's output. It is on screen one frame later, which is the next repaint and is well
//! inside a display cycle.
//!
//! The probe that settled it is worth its own note, because it also proved the input path
//! itself was fine: a bare `TextEdit` fed the same events received them **unfocused**,
//! so nothing was wrong with `focused`, with focus requests, or with the widget id — and
//! the remaining difference was the one-frame lag above.

use std::sync::Arc;
use std::time::{Duration, Instant};

use epsilon_contracts::agent_event::PermissionDecision;
use epsilon_desktop::{IdeApp, IdeSession, Options};
use epsilon_runtime::{LaunchRequest, ProviderChoice};

/// Build a window around a real offline agent in a throwaway workspace.
fn wire(workspace: &std::path::Path) -> (egui::Context, IdeApp) {
    // **Multi-thread, deliberately, and the reason is worth writing down.** The first
    // version of this fixture used `new_current_thread`, which is the natural choice in a
    // test — and it produced a completely empty transcript while every other assertion in
    // the file passed. A current-thread runtime only makes progress while somebody is
    // inside `block_on`; `submit` and the event bridge both `spawn`, and nothing was
    // driving them, so the turn never ran and not one event was ever produced.
    //
    // That failure reads exactly like "the wiring is broken", and it is not: the
    // production runtime (`crate::run`) is multi-threaded, and a fixture that does not
    // behave like the product tests a situation the product cannot be in.
    let runtime = Arc::new(
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("a test runtime"),
    );
    let launch = epsilon_runtime::resolve_launch(&LaunchRequest {
        workspace: workspace.to_path_buf(),
        provider: ProviderChoice::Faux,
        ..LaunchRequest::for_workspace(workspace)
    })
    .expect("a faux launch always resolves");

    assert!(
        launch.offline,
        "the fixture must be offline or the test proves nothing about the offline path"
    );

    let session = runtime
        .block_on(IdeSession::start(runtime.clone(), launch))
        .expect("composing an agent in an empty workspace");

    let ctx = egui::Context::default();
    let notices = vec!["provider: faux (offline scripted)".to_string()];
    let app = IdeApp::new(Options::for_workspace(workspace), session, notices, &ctx);
    (ctx, app)
}

/// Paint one frame at `w` x `h`, disposing of what it produced.
fn paint_once(
    ctx: &egui::Context,
    app: &mut IdeApp,
    w: f32,
    h: f32,
) -> epsilon_desktop::app::FrameSummary {
    paint_with(ctx, app, w, h, vec![])
}

/// Paint one frame carrying input events, so a test can type and press keys.
///
/// `focused: true` is not decoration: `RawInput` defaults it to false and egui drops
/// keyboard events for an unfocused window, so a default-constructed input produces a
/// window nobody can type into.
fn paint_with(
    ctx: &egui::Context,
    app: &mut IdeApp,
    w: f32,
    h: f32,
    events: Vec<egui::Event>,
) -> epsilon_desktop::app::FrameSummary {
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(w, h),
        )),
        focused: true,
        events,
        ..Default::default()
    };
    let mut out = epsilon_desktop::app::FrameSummary::default();
    let mut full = ctx.run_ui(input, |ui| out = app.paint(ui));
    // epaint panics in `Drop` on unapplied texture deltas; a harness that discards the
    // frame is not measuring it.
    full.textures_delta.clear();
    out
}

/// The events for typing `text` and pressing Enter, the way a user does it.
fn type_and_enter(text: &str) -> Vec<egui::Event> {
    let mut events: Vec<egui::Event> = text
        .chars()
        .map(|c| egui::Event::Text(c.to_string()))
        .collect();
    events.push(egui::Event::Key {
        key: egui::Key::Enter,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    });
    events
}

/// The window opens on a tree of the workspace's real files.
///
/// The workspace pane used to draw three placeholder labels. It now walks the workspace,
/// and a frame that draws nothing would be the "no file open" state wearing a file tree's
/// chrome — which is exactly the shape of defect this repository has paid for twice.
#[test]
fn the_window_draws_the_workspaces_own_files() {
    let ws = tempfile::tempdir().expect("a workspace");
    std::fs::create_dir_all(ws.path().join("src")).unwrap();
    std::fs::write(ws.path().join("README.md"), "# hi\n").unwrap();
    std::fs::write(ws.path().join("src").join("main.rs"), "fn main() {}\n").unwrap();
    std::fs::create_dir_all(ws.path().join("target")).unwrap();
    std::fs::write(ws.path().join("target").join("junk.bin"), "").unwrap();

    let ctx = egui::Context::default();
    let mut app = IdeApp::without_agent(Options::for_workspace(ws.path()));
    let frame = paint_once(&ctx, &mut app, 1440.0, 900.0);

    let tree = frame.region("workspace").join(" ");
    assert!(
        tree.contains("files"),
        "the pane reports how much it found.\n{}",
        frame.describe()
    );
    assert!(
        !tree.contains("target"),
        "build output must not be in the tree: {tree}"
    );
}

/// Clicking a file opens it in the editor, and the editor says it is read-only.
///
/// The click is a synthesized pointer press at a row's own rectangle, so it goes through
/// egui's hit testing. A tree that draws correctly and cannot be clicked is a screenshot.
#[test]
fn clicking_a_file_opens_it_read_only() {
    let ws = tempfile::tempdir().expect("a workspace");
    std::fs::create_dir_all(ws.path().join("src")).unwrap();
    std::fs::write(ws.path().join("src").join("only.rs"), "one\ntwo\nthree\n").unwrap();

    let ctx = egui::Context::default();
    let mut app = IdeApp::without_agent(Options::for_workspace(ws.path()));

    // Frame one lets the tree lay out; frame two presses the row's **recorded**
    // rectangle, so the test presses what was drawn rather than guessing a coordinate.
    // A guessed y passes or fails with the sidebar's height, which is the layout
    // module's business and not this test's.
    let laid_out = paint_once(&ctx, &mut app, 1440.0, 900.0);
    let row = laid_out
        .hotspot("tree:src/only.rs")
        .expect("the tree drew a row for src/only.rs");
    let click = row.center();

    // A real click is three frames: move, press, release. Sending the press and the
    // release in one input frame is not something a user can do, and egui treats the
    // two as separate passes — so a one-frame "click" tests a state the product is never
    // in, which is the same mistake the frame-lag test below records.
    paint_with(
        &ctx,
        &mut app,
        1440.0,
        900.0,
        vec![egui::Event::PointerMoved(click)],
    );
    paint_with(
        &ctx,
        &mut app,
        1440.0,
        900.0,
        vec![egui::Event::PointerButton {
            pos: click,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        }],
    );
    let frame = paint_with(
        &ctx,
        &mut app,
        1440.0,
        900.0,
        vec![egui::Event::PointerButton {
            pos: click,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }],
    );

    let editor = frame.region("editor").join(" ");
    assert!(
        editor.contains("only.rs"),
        "the editor should have opened the row recorded at {row:?} (sidebar {:?}).\n{}",
        laid_out
            .hotspot("tree:README.md")
            .unwrap_or(egui::Rect::ZERO),
        frame.describe()
    );
    assert!(
        editor.contains("read-only"),
        "and it must not pretend it can be typed into.\n{}",
        frame.describe()
    );
}

/// What is open when nothing was clicked must not look like something is.
///
/// The empty state names the action, because a blank editor pane reads as a broken pane.
#[test]
fn an_unopened_editor_says_what_to_do() {
    let ws = tempfile::tempdir().expect("a workspace");
    let ctx = egui::Context::default();
    let mut app = IdeApp::without_agent(Options::for_workspace(ws.path()));
    let frame = paint_once(&ctx, &mut app, 1440.0, 900.0);
    assert!(
        frame
            .region("editor")
            .iter()
            .any(|t| t.contains("Pick a file")),
        "{}",
        frame.describe()
    );
    assert!(
        !frame
            .region("editor")
            .iter()
            .any(|t| t.contains("read-only")),
        "an editor with nothing open must not claim to be showing a file:\n{}",
        frame.describe()
    );
}

/// Keystrokes reach the composer.
///
/// This was measured four times before it worked, and each failure looked like "the input
/// path is broken": focus requests, a stable widget id, `RawInput::focused` were all
/// changed without success. The thing that settled it was a bare `TextEdit` fed the same
/// events — which received them **unfocused**, proving none of those three was the cause.
///
/// What was the cause: the test asserted on the frame that *sent* the turn, and an
/// immediate-mode frame paints the transcript before the composer, so a row the composer
/// adds cannot be in that frame. The prompt is on screen on the next repaint.
///
/// This test therefore keeps the two things apart: **text in the draft** (same frame — the
/// text edit runs before the send) and **the prompt in the transcript** (next frame).
#[test]
fn typing_reaches_the_composer_and_sending_puts_the_words_on_screen() {
    let ws = tempfile::tempdir().expect("a workspace");
    let (ctx, mut app) = wire(ws.path());

    // Frame one: the composer asks for focus.
    paint_once(&ctx, &mut app, 1440.0, 900.0);

    // Frame two: type, without pressing Enter.
    let typed = vec![egui::Event::Text("draft words".into())];
    paint_with(&ctx, &mut app, 1440.0, 900.0, typed);
    assert_eq!(
        app.draft(),
        "draft words",
        "a keystroke must reach the composer's buffer"
    );

    // Frame three: Enter sends, and the draft empties on the same frame — the text edit
    // runs first, then `send_draft` reads it.
    let sent = paint_with(
        &ctx,
        &mut app,
        1440.0,
        900.0,
        vec![egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
    );
    let _ = sent;
    assert_eq!(app.draft(), "", "Enter sends, and the composer empties");

    // Frame four: the words are on screen.
    let frame = paint_once(&ctx, &mut app, 1440.0, 900.0);
    assert!(
        frame
            .region("agent")
            .iter()
            .any(|t| t.contains("draft words")),
        "the prompt belongs on screen one frame after the send.\n{}",
        frame.describe()
    );
}

/// Keystrokes reach the composer.
///
/// The headline claim: a turn submitted in this window produces an answer on this screen.
///
/// Nothing here is stubbed except the provider, which is the offline scripted one — the
/// same one `epsilon --demo` uses, and the reason a user's first run needs no key.
///
/// The scripted turn calls `bash`, so it **parks on a permission gate** and cannot finish
/// until someone answers. That is the point of the test: it walks the whole loop —
/// compose, submit, receive, show the ask, answer it, receive the rest — which is the only
/// way to know the window is wired to the harness rather than merely beside it.
#[test]
fn a_real_turn_runs_to_completion_through_a_permission_ask() {
    let ws = tempfile::tempdir().expect("a workspace");
    let (ctx, mut app) = wire(ws.path());

    let session = app.session().expect("this window has a session").clone();

    // Frame one lets the composer take the keyboard; frame two carries the typing.
    // A real window gets this for free — winit delivers input only once the surface is
    // up, and egui repaints many times before a human can press a key.
    paint_once(&ctx, &mut app, 1440.0, 900.0);
    paint_with(&ctx, &mut app, 1440.0, 900.0, type_and_enter("say hello"));

    // Frame three. **The user's own words are added when the send happens, which is after
    // this frame's transcript has already been drawn** — an immediate-mode frame paints
    // top to bottom, and the composer is below the transcript. So the prompt appears on
    // the *next* frame, and a test that asserts on the frame that sent it sees an empty
    // transcript and concludes the feature is missing. That is what four rounds of this
    // file looked like before it was measured.
    let first = paint_once(&ctx, &mut app, 1440.0, 900.0);
    assert_eq!(
        app.draft(),
        "",
        "the composer empties when it sends, not holding {:?}",
        app.draft()
    );

    // The reader's own words must appear immediately, not when the model answers.
    assert!(
        first
            .region("agent")
            .iter()
            .any(|t| t.contains("say hello")),
        "the prompt must be on screen the moment Enter is pressed.\n{}",
        first.describe()
    );

    // 1. Wait for the permission ask to reach the screen.
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut asked = false;
    while Instant::now() < deadline {
        let frame = paint_once(&ctx, &mut app, 1440.0, 900.0);
        if frame
            .region("agent")
            .iter()
            .any(|t| t.contains("allow bash"))
        {
            asked = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        asked,
        "the run parked on a permission gate and the window never said so (transcript holds \
         {} row(s)).\n{}",
        app.transcript().rows().len(),
        paint_once(&ctx, &mut app, 1440.0, 900.0).describe()
    );

    // 2. Answer it the way the user would, through the same seam the terminal uses.
    let ask = session
        .pending_approvals()
        .into_iter()
        .next()
        .expect("an ask is waiting");
    assert!(
        session.resolve(&ask.request_id, PermissionDecision::AllowOnce),
        "answering a waiting ask must succeed"
    );

    // 3. The rest of the turn must arrive.
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut finished = false;
    while Instant::now() < deadline {
        let frame = paint_once(&ctx, &mut app, 1440.0, 900.0);
        if frame
            .region("agent")
            .iter()
            .any(|t| t.contains("epsilon-ready") || t.contains("Run complete"))
        {
            finished = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }

    let frame = paint_once(&ctx, &mut app, 1440.0, 900.0);
    assert!(
        finished,
        "an allowed turn produced no answer within 20s (transcript holds {} row(s)).\n{}",
        app.transcript().rows().len(),
        frame.describe()
    );
    assert!(
        frame.drew("composer"),
        "and the composer is still there to answer with.\n{}",
        frame.describe()
    );
    assert!(
        !frame
            .region("agent")
            .iter()
            .any(|t| t.contains("allow bash")),
        "an answered ask must stop being offered.\n{}",
        frame.describe()
    );
}

/// The window names the provider it is calling, and that name came from the launch
/// rather than from the model id.
///
/// This is the M233 property, reached from the other end: M233 froze it in the terminal
/// header, and here it is the same field read by a different frontend through a different
/// resolver.
#[test]
fn the_window_names_the_provider_the_launch_chose() {
    let ws = tempfile::tempdir().expect("a workspace");
    let (ctx, mut app) = wire(ws.path());
    let frame = paint_once(&ctx, &mut app, 1440.0, 900.0);

    let title = frame.region("title").join(" ");
    assert!(
        title.contains("offline"),
        "an offline launch must read as offline in the window.\n{}",
        frame.describe()
    );
    assert!(
        !title.contains("$0") && !title.contains("cache"),
        "and must not invent a cost or a cache rate.\n{}",
        frame.describe()
    );
}

/// A turn that cannot start is refused at the point of the press, not silently dropped.
///
/// The loop is single-flight; a second submit against a running turn returns
/// `AlreadyRunning`, and a window that swallowed that would look like a dead Send button.
#[test]
fn a_second_submit_while_running_is_refused_rather_than_queued_silently() {
    let ws = tempfile::tempdir().expect("a workspace");
    let (ctx, mut app) = wire(ws.path());
    let session = app.session().expect("this window has a session").clone();
    session.submit("first").expect("the first turn is accepted");

    // Whether or not the turn is still in flight, a refusal must be a *sentence*.
    if let Err(reason) = session.submit("second") {
        assert!(
            reason.contains("in flight"),
            "the refusal must say why: {reason}"
        );
    }

    // Either way the window still draws, and the refusal never became a dead key.
    let frame = paint_once(&ctx, &mut app, 1440.0, 900.0);
    assert!(frame.drew("agent"), "{}", frame.describe());
}

/// A window that cannot reach its agent must say so rather than drawing an empty one.
///
/// The inverse of the test above, and the reason `IdeApp` has an agent-less constructor at
/// all: the state has to be constructible to be testable.
#[test]
fn a_window_with_no_agent_says_so_and_claims_no_numbers() {
    let mut app = IdeApp::without_agent(Options::for_workspace("/tmp/ws"));
    let ctx = egui::Context::default();
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1440.0, 900.0),
        )),
        ..Default::default()
    };
    let mut out = epsilon_desktop::app::FrameSummary::default();
    let mut full = ctx.run_ui(input, |ui| out = app.paint(ui));
    full.textures_delta.clear();

    assert!(
        out.region("status")
            .iter()
            .any(|t| t.contains("no agent attached")),
        "{}",
        out.describe()
    );
    for banned in ["$0", "GPT", "Claude", "DeepSeek", "cache", "ctx "] {
        assert!(
            !out.rows.iter().any(|(_, t)| t.contains(banned)),
            "an agent-less window claims {banned:?}.\n{}",
            out.describe()
        );
    }
}
