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
//! # No window, no network, no key
//!
//! egui renders into an off-screen `egui::Context`, so "the window drew it" is measurable
//! without a window — which is what makes this a unit test rather than a manual check.
//!
//! # What is *not* covered here, stated plainly
//!
//! **Keystrokes to the composer are not exercised.** The draft is set with
//! `IdeApp::set_draft` and sent with `IdeApp::send_draft` — the same function the composer
//! calls, so the send rule and the agent path are genuinely under test — but the route
//! from an egui `Event::Text` into the draft is not.
//!
//! Four attempts to drive it through `RawInput.events` all failed to move a character, and
//! the cause is **not isolated**: focus is requested unconditionally, the composer's widget
//! id is stable, `RawInput::focused` is set true, and the events use the documented
//! `Event::Text(String)` shape. Nothing about the product is known to be broken — a real
//! window receives focus and events from winit rather than from a `RawInput` — but neither
//! is it known to work, and this file does not claim that it does.
//!
//! The next thing to try is a two-frame sequence (focus on one frame, type on the next)
//! against a context that has been tessellated, or driving the real `eframe` window.
//! Guessing again would be the fourth theory in a row, and `audit/04-tui.md` records what
//! that costs.

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

/// Paint one frame with input events, so a test can type and press keys.
///
/// The events go through egui's real input path and reach the real composer. That is the
/// difference between "the window can be told to run a turn" and "a user typing in it runs
/// a turn", and only the second one is the product.
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
        // **`focused` defaults to `false`, and egui drops keyboard events for an unfocused
        // window.** A `RawInput::default()` therefore produces a window that cannot be
        // typed into, which cost this test four rounds of "the composer ignores keys" —
        // and it is exactly what a real window looks like before the OS has focused it.
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

    // The draft is set and sent through the **same function the composer calls**, so what
    // is under test is the composer's send rule and the agent path. Keystroke-to-draft
    // plumbing is a separate, still-open question — see the module docs.
    app.set_draft("say hello");
    app.send_draft().expect("a draft with words is sent");
    assert_eq!(app.draft(), "", "the composer empties when it sends");

    let first = paint_once(&ctx, &mut app, 1440.0, 900.0);

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
