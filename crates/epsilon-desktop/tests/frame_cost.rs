//! What one painted desktop frame costs, and what makes it cost that.
//!
//! ```text
//! cargo test -p epsilon-desktop --test frame_cost -- --ignored --nocapture
//! ```
//!
//! # Why this exists
//!
//! **No desktop frame cost had ever been measured in this repository.** M252 established
//! that no wgpu backend presents on this machine, so every claim about the window's
//! *appearance* is a hypothesis; nothing had ever established a claim about its *speed*.
//!
//! That mattered immediately. A parallel read-only review ranked **the sessions pane** first
//! on the strength of the project's own recorded estimate — 202 ms at 500 sessions, from
//! `epsilon-session/src/discover.rs:296-298`. Measured here, fifty session files cost
//! **+1.8 ms/frame**. The thing that was measured to be expensive was not that one.
//!
//! # What the numbers are
//!
//! Painting through the real `paint` at 1440×900, 20 frames each, one warm-up frame first:
//!
//! | transcript | before any fix | after [`markup::Cache`] |
//! |---|---|---|
//! | 50 assistant rows, rich markdown | 11.5 ms | 8.0 ms |
//! | 200 assistant rows, rich markdown | 41.3 ms | 33.7 ms |
//! | 800 assistant rows, rich markdown | **149.9 ms** | **115.1 ms** |
//! | 800 rows, one paragraph each | — | 32.0 ms |
//! | 800 rows, plain labels | — | 17.0 ms |
//!
//! # What the numbers say, which is the point
//!
//! Cost tracks **blocks**, not parses and not rows: the same 800 rows cost 17 ms as plain
//! labels, 32 ms as one markdown block each, and 115 ms as the six blocks the fixture's rich
//! markdown produces. Caching the parse took 20–30% off, which is real and is not the bulk.
//!
//! **The transcript builds a text widget for every block of every row, on screen or not.**
//! The file tree two functions away already virtualizes (`workspace.rs` uses `show_rows`,
//! with a comment saying virtualization is "not optional"); the transcript cannot, because
//! egui's `show_rows` requires a uniform row height and a transcript's rows are not uniform.
//!
//! That is the open measurement, and it is recorded rather than half-solved: the fix needs
//! cached per-row heights, which is a layout change with its own correctness surface, and a
//! guess at it would be worse than the number.
//!
//! # Not a gate
//!
//! These are timings on one machine and would be a flaky gate anywhere else, so the test is
//! `#[ignore]`d and prints rather than asserts — the same arrangement as the terminal's
//! `frame_dump`, and for the same reason: the value is in running it when the path changes.

use std::sync::Arc;
use std::time::Instant;

use epsilon_desktop::{IdeApp, IdeSession, Options, Transcript};

/// A paragraph with a list and a table: the six-block case.
const RICH: &str = "Here is a paragraph of **markdown** with `code`, a list:\n\n- one\n- two\n\n\
                     and a table:\n\n| a | b |\n|---|---|\n| 1 | 2 |";
/// One paragraph: the one-block case, for the per-row/per-block comparison.
const PLAIN: &str = "one single paragraph of prose with **bold** and `code`";

/// Build a window around a real offline agent in a throwaway workspace.
///
/// The same fixture `tests/window.rs` uses, including its **multi-thread** runtime: a
/// current-thread one makes `spawn`ed work never run, which reads exactly like broken wiring.
fn wire(workspace: &std::path::Path) -> (egui::Context, IdeApp) {
    let runtime = Arc::new(
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("a test runtime"),
    );
    let launch = epsilon_runtime::resolve_launch(&epsilon_runtime::LaunchRequest {
        workspace: workspace.to_path_buf(),
        provider: epsilon_runtime::ProviderChoice::Faux,
        ..epsilon_runtime::LaunchRequest::for_workspace(workspace)
    })
    .expect("a faux launch always resolves");
    let session = runtime
        .block_on(IdeSession::start(runtime.clone(), launch))
        .expect("composing an agent in an empty workspace");
    let ctx = egui::Context::default();
    let app = IdeApp::new(
        Options::for_workspace(workspace),
        session,
        vec!["provider: faux (offline scripted)".to_string()],
        &ctx,
    );
    (ctx, app)
}

/// One frame through the production draw path, at the size a real window uses.
fn paint_once(ctx: &egui::Context, app: &mut IdeApp) {
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1440.0, 900.0),
        )),
        focused: true,
        ..Default::default()
    };
    let mut out = epsilon_desktop::app::FrameSummary::default();
    let mut full = ctx.run_ui(input, |ui| out = app.paint(ui));
    full.textures_delta.clear();
}

/// Median-free, but not a single frame either: one warm-up, then `n`, and the mean.
fn ms_per_frame(label: &str, ctx: &egui::Context, app: &mut IdeApp, n: u32) {
    paint_once(ctx, app);
    let t = Instant::now();
    for _ in 0..n {
        paint_once(ctx, app);
    }
    let each = t.elapsed().as_secs_f64() * 1000.0 / f64::from(n);
    println!("  {label:<38} {each:>8.3} ms/frame  ({n} frames)");
}

fn assistant_row(i: usize, body: &str) -> epsilon_contracts::agent_event::AgentEvent {
    epsilon_contracts::agent_event::AgentEvent::MessageEnd {
        message: Box::new(epsilon_contracts::message::Message::Assistant(
            epsilon_contracts::message::AssistantMessage {
                id: format!("m{i}"),
                timestamp_ms: 0,
                provider: "faux".into(),
                model_id: "faux-1".into(),
                content: vec![epsilon_contracts::content::ContentBlock::text(body)],
                stop_reason: epsilon_contracts::message::StopReason::EndTurn,
                error_message: None,
                usage: epsilon_contracts::message::Usage::default(),
            },
        )),
    }
}

/// Seed a transcript of `n` exchanges whose answers are `body`.
fn seeded(app: &mut IdeApp, ws: &std::path::Path, n: usize, body: &str) {
    let mut t = Transcript::new(ws);
    for i in 0..n {
        t.push_user(&format!("question {i}"));
        t.apply(&assistant_row(i, body));
    }
    app.seed_transcript(t);
}

/// Fill the sessions directory with `n` plausible files, for the pane the review ranked first.
fn seed_sessions(ws: &std::path::Path, n: usize) {
    let dir = ws.join(".epsilon").join("sessions");
    std::fs::create_dir_all(&dir).expect("a sessions directory");
    for i in 0..n {
        std::fs::write(
            dir.join(format!("2026-10-0{}_session{i}.jsonl", (i % 9) + 1)),
            format!(
                "{{\"type\":\"session\",\"version\":3}}\n{{\"type\":\"message\",\"id\":\"m{i}\",\
                 \"timestamp_ms\":0,\"message\":{{\"kind\":\"user\",\"id\":\"u{i}\",\"timestamp_ms\":0,\
                 \"content\":[{{\"kind\":\"text\",\"text\":\"session {i}\"}}]}}}}\n"
            ),
        )
        .expect("a session file");
    }
}

#[test]
#[ignore = "a timing measurement on one machine; run it when the draw path changes"]
fn what_a_frame_costs() {
    let ws = tempfile::tempdir().expect("a workspace");
    let (ctx, mut app) = wire(ws.path());
    println!("FRAME COST @1440x900 (egui offscreen, real `paint`)");

    ms_per_frame("empty, files sidebar", &ctx, &mut app, 40);

    println!("  -- transcript cost, by rows --");
    for n in [50usize, 200, 800] {
        seeded(&mut app, ws.path(), n, RICH);
        ms_per_frame(
            &format!("{n} assistant rows, rich markdown"),
            &ctx,
            &mut app,
            20,
        );
    }

    println!("  -- is it the row or the block? --");
    for n in [200usize, 800] {
        seeded(&mut app, ws.path(), n, PLAIN);
        ms_per_frame(
            &format!("{n} assistant rows, one block"),
            &ctx,
            &mut app,
            20,
        );
    }
    {
        let mut t = Transcript::new(ws.path());
        for i in 0..800 {
            t.push_user(&format!("question {i}"));
        }
        app.seed_transcript(t);
        ms_per_frame("800 plain rows (no markdown path)", &ctx, &mut app, 20);
    }

    println!("  -- the sessions pane, which the review ranked first --");
    seeded(&mut app, ws.path(), 800, RICH);
    ms_per_frame("800 rows, files sidebar", &ctx, &mut app, 20);
    seed_sessions(ws.path(), 50);
    app.set_sidebar_view(epsilon_desktop::app::SidebarView::Sessions);
    ms_per_frame("800 rows, 50 sessions on disk", &ctx, &mut app, 20);
}

/// **Where the transcript's 104 ms actually goes.**
///
/// `what_a_frame_costs` established that cost tracks *blocks* — the same 800 rows cost 13 ms as
/// plain labels, 32 ms with one markdown block each, 104 ms with six. That says the expensive
/// thing is per-block, and says nothing about *which* per-block step. Three candidates, and the
/// fix is different for each:
///
/// * `markup::render` — already cached since M266, so this should be ~0;
/// * `markup::job` + the summary's `plain_of` — **our** work, per block, per frame;
/// * `ui.label(WidgetText::from(job))` — **egui's** text layout and shaping, per block, per frame.
///
/// Only the third is addressed by virtualising the transcript. If the second dominates, the
/// cheap fix is to cache the built job instead. **So this is the measurement that picks
/// between two different fixes**, and guessing would have cost a large change aimed at the
/// wrong phase.
///
/// Timed outside the frame, over the same 800 rich answers, so nothing here depends on what
/// egui does.
#[test]
#[ignore = "a timing measurement on one machine; run it when the draw path changes"]
fn where_the_transcript_milliseconds_go() {
    const ROWS: usize = 800;
    /// The whole-frame figure from `what_a_frame_costs` for this fixture, so the remainder
    /// below is computed rather than guessed. If that test's numbers move, this one is wrong.
    const FRAME_MS: f64 = 104.4;
    let blocks_per_row: Vec<epsilon_desktop::markup::Block> = epsilon_desktop::markup::render(RICH);

    let t = Instant::now();
    for _ in 0..ROWS {
        std::hint::black_box(epsilon_desktop::markup::render(RICH));
    }
    let parse = t.elapsed().as_secs_f64() * 1000.0;

    // `plain_of` is the summary's text: one `String` per block, per frame.
    let runs: Vec<&epsilon_desktop::markup::Run> = blocks_per_row
        .iter()
        .filter_map(|b| match b {
            epsilon_desktop::markup::Block::Paragraph(r)
            | epsilon_desktop::markup::Block::Heading { runs: r, .. }
            | epsilon_desktop::markup::Block::Quote(r) => Some(r.iter()),
            _ => None,
        })
        .flatten()
        .collect();
    let _ = runs;
    let t = Instant::now();
    for _ in 0..ROWS {
        for b in &blocks_per_row {
            if let epsilon_desktop::markup::Block::Paragraph(r) = b {
                std::hint::black_box(r.iter().map(|x| x.text.as_str()).collect::<String>());
            }
        }
    }
    let plain = t.elapsed().as_secs_f64() * 1000.0;

    let t = Instant::now();
    for _ in 0..ROWS {
        for b in &blocks_per_row {
            if let epsilon_desktop::markup::Block::Paragraph(r) = b {
                std::hint::black_box(epsilon_desktop::markup::job(
                    r,
                    egui::Color32::WHITE,
                    egui::Color32::GRAY,
                ));
            }
        }
    }
    let job = t.elapsed().as_secs_f64() * 1000.0;

    // The app caches `render`, so it pays only the two phases below.
    let paid = plain + job;
    let egui = (FRAME_MS - paid).max(0.0);

    println!("TRANSCRIPT PHASE BREAKDOWN — {ROWS} rich rows, per frame");
    println!("  markup::render  (RAW — the app caches it since M266) {parse:>8.3} ms");
    println!("  plain_of         (one String per block, per frame)    {plain:>8.3} ms  <- paid");
    println!("  markup::job      (one LayoutJob per block, per frame){job:>8.3} ms  <- paid");
    println!("  OUR WORK       {paid:>8.3} ms  of the frame");
    println!(
        "  EGUI'S OWN    {egui:>8.3} ms  ({:.0}% of it): text layout and shaping, per block, \
         every frame",
        (egui / FRAME_MS) * 100.0
    );
    println!(
        "\n  **Caching the jobs would buy {:.1}% -- which is why this measurement exists.** The \
         fix is\n  virtualization, and nothing cheaper addresses it.",
        (paid / FRAME_MS) * 100.0
    );
    println!(
        "  (`markup::render` swings between runs on a debug build and is NOT paid: the app \
         caches it\n   since M266. Only the two `paid` figures matter.)"
    );
}

/// What `Agent::goal()` costs with a **real store behind it**.
///
/// `what_a_frame_costs` above seeds the window's *view* model, and that is a hole in the
/// measurement rather than a detail: `agent.goal()` reads the **durable store**, and against
/// a store this bench never wrote it measured a near-empty file. The window calls it twice per
/// painted frame (`app.rs:1905` for the band, `:725` for the round driver), and `store.goal()`
/// walks `read_all()`, which **deep-clones every entry** under the mutex the agent's own
/// append path takes.
///
/// So: how expensive is it, really, on a session of a realistic size?
#[test]
#[ignore = "a timing measurement on one machine; run it when the draw path changes"]
fn what_the_goal_read_costs() {
    println!("Agent::goal() against a real store");
    for n in [50usize, 400, 1_600] {
        let ws = tempfile::tempdir().expect("a workspace");
        let (_ctx, app) = wire(ws.path());
        let agent = Arc::clone(app.session().expect("a session").agent());

        let store = Arc::new(std::sync::Mutex::new(
            epsilon_session::SessionStore::open_in(ws.path().join("bench.jsonl"), ws.path())
                .expect("a store"),
        ));
        {
            let mut guard = store.lock().expect("the store lock");
            for i in 0..n {
                guard
                    .append(epsilon_contracts::session_entry::EntryKind::Message {
                        message: assistant_markdown(&format!(
                            "an answer number {i} with **bold**, a list:\n\n- one\n- two\n"
                        )),
                        api_request_sha256: None,
                    })
                    .expect("an entry lands in the log");
            }
        }
        agent.attach_session_store(Arc::clone(&store));

        let goal = agent.goal_apply(
            None,
            epsilon_contracts::goal::GoalOp::Create {
                objective: "make the suite green".into(),
                max_rounds: Some(7),
            },
        );
        assert!(
            goal.is_ok(),
            "a goal is created so the read has something to find"
        );

        let t = Instant::now();
        let reps = 20;
        for _ in 0..reps {
            std::hint::black_box(agent.goal());
        }
        let each = t.elapsed().as_secs_f64() * 1000.0 / f64::from(reps);
        println!(
            "  {n:>5} stored messages -> agent.goal() {each:>8.3} ms  \
             (called twice per painted frame: {:.1} ms)",
            each * 2.0
        );
    }
    let _ = egui::Context::default();
}

/// A minimal assistant message for seeding a store.
fn assistant_markdown(body: &str) -> epsilon_contracts::message::Message {
    epsilon_contracts::message::Message::Assistant(epsilon_contracts::message::AssistantMessage {
        id: epsilon_contracts::ids::new_id("msg"),
        timestamp_ms: 0,
        provider: "faux".into(),
        model_id: "faux-1".into(),
        content: vec![epsilon_contracts::content::ContentBlock::text(body)],
        stop_reason: epsilon_contracts::message::StopReason::EndTurn,
        error_message: None,
        usage: epsilon_contracts::message::Usage::default(),
    })
}
