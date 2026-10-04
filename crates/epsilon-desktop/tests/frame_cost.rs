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
//! Painting through the real `paint` at 1440×900, 20 frames each, one warm-up frame first.
//!
//! **M285 virtualised the transcript.** The "after virtualisation" column was measured on this
//! machine (2026-10-04, `CARGO_BUILD_JOBS=1`, `CARGO_PROFILE_DEV_DEBUG=0`,
//! `CARGO_PROFILE_TEST_DEBUG=2`) — **with the accessibility tree switched off**, which was a
//! hole in the instrument rather than a property of the frame. Read that column as "the
//! transcript fix, measured by an instrument that could not see node-building"; the next
//! section is what it measures now that it can.
//!
//! | transcript | before any fix | after `markup::Cache` | after virtualisation (M285) |
//! |---|---|---|---|
//! | 50 assistant rows, rich markdown | 11.5 ms | 8.0 ms | **1.9 – 2.2 ms** |
//! | 200 assistant rows, rich markdown | 41.3 ms | 33.7 ms | **2.0 – 2.3 ms** |
//! | 800 assistant rows, rich markdown | **149.9 ms** | **115.1 ms** | **2.1 – 2.7 ms** |
//! | 800 rows, one paragraph each | — | 32.0 ms | **2.2 – 2.6 ms** |
//! | 800 rows, plain labels | — | 17.0 ms | **1.9 – 2.2 ms** |
//! | empty window + file sidebar | — | 0.94 ms | **0.9 – 1.0 ms** |
//!
//! **The third column is the result, and the shape of it is the point:** the transcript cost is
//! now *flat in row count*. 50 rows and 800 rows cost the same, because the frame is
//! O(rows on screen) rather than O(rows). A 128 ms control measured the same day became 2.2 ms.
//!
//! **The `FRAME_MS` constant in `where_the_transcript_milliseconds_go` below is now
//! historical**, and so is the phase breakdown it feeds: with the transcript virtualised, the
//! phase it apportions is no longer 96% of the frame. Read it as a record of why virtualisation
//! was chosen, not as a current statement about where the time goes.
//!
//! **The earlier figures were not reproducible on this machine on 2026-10-04** — a control of
//! the *unvirtualised* code measured 127.9 ms where the table says 104.4 ms. That is a
//! build/machine difference rather than a code difference, which is exactly why the before/after
//! pair above is the trustworthy measurement and any single historical figure is not.
//!
//! # What this measures now — and what it still does not
//!
//! **Since M288 every frame this file paints runs with the accessibility tree switched on.**
//! It did not before, and that was not a smaller measurement but **a different question**:
//! `Context::accesskit_node_builder` returns `None` unless the pass state exists
//! (`egui-0.36.2/src/context.rs:600-601`), and only `Context::enable_accesskit` creates it
//! (`egui-0.36.2/src/context.rs:520-533`). With the tree off, every `name_node` and
//! `name_region` call in `IdeApp::paint` wrote into nothing, so **building nodes was in no
//! number this file ever printed.** That is how M286's transcript-row naming came out
//! "unmeasurable" here — 3.927 ms → 4.004 ms, both figures really saying "no change to the
//! part that was invisible" — while an interleaved A/B with the tree on found the two fully
//! overlapping at the noise floor (ADR-022, `docs/design/epsilon-decisions.md:1782-1789`).
//!
//! eframe turns the tree on when the platform asks for the initial tree
//! (`eframe-0.36.2/src/native/winit_integration.rs:180`) and never turns it off, so
//! [`paint_once`] does the same. One flag, and nothing else, separates the two columns below.
//!
//! | frame | AccessKit **off** — the old instrument | AccessKit **on** — what the window runs |
//! |---|---|---|
//! | empty window + file sidebar | 1.18 ms | **1.51 – 1.89 ms** |
//! | 50 assistant rows, rich markdown | 2.37 ms | **4.36 – 4.90 ms** |
//! | 200 assistant rows, rich markdown | 2.74 ms | **4.84 – 5.48 ms** |
//! | 800 assistant rows, rich markdown | 3.04 ms | **4.76 – 5.05 ms** |
//! | 200 assistant rows, one paragraph each | 2.87 ms | **4.97 – 5.23 ms** |
//! | 800 assistant rows, one paragraph each | 3.38 ms | **4.87 – 5.39 ms** |
//! | 800 rows, plain labels | 2.58 ms | **4.44 – 4.49 ms** |
//! | 800 rows, files sidebar | 2.81 ms | **4.40 – 4.73 ms** |
//! | 800 rows, 50 sessions on disk | 3.03 ms | **4.12 – 4.57 ms** |
//!
//! The off column is **one** run, taken before this change and not repeatable now that the flag
//! is on. The on column is the **range of two** runs of this test taken the same afternoon,
//! because even that moved by ~0.5 ms for identical bytes.
//!
//! **Those pairs do not settle anything on their own, and a single pair would settle nothing.**
//! This machine moved the same 800-row figure between 2.6 and 4.0 ms for one unchanged build
//! (ADR-022), and the table's own third column does not reproduce either — which is exactly why
//! the honest form of this measurement is **interleaved**, alternating the two settings inside
//! one process, ten rounds of ten frames each, order swapped every round so drift cannot always
//! land on the same column:
//!
//! | fixture | AccessKit on | AccessKit off | delta |
//! |---|---|---|---|
//! | empty window, files sidebar | 1.788 ms (1.509 – 2.087) | 1.360 ms (1.147 – 1.692) | **+0.43 ms (+31.5 %)** |
//! | 800 assistant rows, rich markdown | 5.342 ms (5.079 – 5.763) | 3.858 ms (3.566 – 4.344) | **+1.48 ms (+38.5 %)** |
//!
//! **Node-building is not free, and the two ranges do not overlap at 800 rows.** All twenty
//! paired rounds came out positive — twenty for twenty, which is not what noise looks like — so
//! this is a cost and not a wobble. **That is the finding this instrument was blind to**, and it
//! is unwelcome: a frame that publishes accessible names costs about a third more than a frame
//! that does not, and the old benchmark could not have said so however long it was run.
//!
//! What the delta is made of is *not* broken out here, and the honest guess is that it is both
//! halves: egui's per-node bookkeeping (an `IdMap` insert, a parent search, a `Node` clone per
//! widget) and our own `name_region` / `name_node` string work, which is computed either way and
//! merely discarded today. Splitting it needs its own probe and this file does not claim the
//! split.
//!
//! Note what the transcript fix still means in spite of this: **the cost is bounded by what is
//! on screen, not by how much has been said.** 800 rows and 200 rows cost the same here, and the
//! row count stopped mattering in M285. Adding a third of a frame back did not make it grow
//! again.
//!
//! **What it now measures** — at 1440×900, offscreen, through the real `IdeApp::paint`:
//!
//! * egui's whole pass over the window: layout, clipping, text shaping, the app's own
//!   per-row work — **plus** the AccessKit node pass: a node per registered widget, the tree
//!   assembled, and every accessible **name** the window writes into it. The last part is
//!   what M286 added and what the old instrument was structurally unable to see.
//! * the mean over `n` frames of a **steady-state** frame: comparable between runs, and
//!   deliberately not a worst case.
//!
//! **What it still does not measure**, which is worth as much as the list above:
//!
//! * **It is not a screen reader, and it does not check that a name is *right*.** A frame
//!   that attaches a perfectly-formed label to the wrong node passes everything below. Names
//!   are asserted by the in-crate a11y tests (`Harness::node_named`,
//!   `crates/epsilon-desktop/src/app.rs`), which read the same tree and then check its
//!   contents. This file only checks that a tree exists, that it holds more than the window
//!   root, and that at least one node carries a name.
//! * **The tree goes nowhere.** `accesskit_update` is built and then thrown away. The
//!   platform adapter's half — diffing an update against the tree the OS already holds, and
//!   the UIA/AX calls behind it — is where accessibility cost actually grows with node count,
//!   and none of it is in these numbers.
//! * **No input.** The `RawInput` carries no pointer, no wheel, no keys, and **no advancing
//!   clock**: every timed frame is the same frame, and egui's animations never move. A frame
//!   that follows a keystroke or a scroll does more work, and this cannot say by how much.
//!   **Keys now include the zoom** — `paint` binds `Ctrl`+`+`/`-`/`0` through egui's own
//!   `gui_zoom` (`crates/epsilon-desktop/src/app.rs:2146-2163`) — so "no input" no longer means
//!   "no scale" by accident. It means the factor stays at **1.0**, and
//!   `the_frame_carries_the_accessibility_tree_the_window_publishes` **asserts** that rather than
//!   assuming it, which is what keeps these numbers comparable to the M285 column taken before
//!   the zoom was reachable at all.
//! * **No GPU.** M252 established that no backend presents on this machine. Shapes are
//!   tessellated inside the pass; rasterisation, upload and present are not measured at all.
//! * **One pixel ratio.** `pixels_per_point` is 1.0, so glyph rasterisation at HiDPI and any
//!   text scale above 1.0 sit outside it.
//! * **Not the product's frame budget.** No winit, no vsync, no pacing: this is work per
//!   frame, not the rate a window achieves.
//! * **The agent is offline.** [`wire`] composes the faux provider, so nothing streams into
//!   the transcript while frames are timed.
//!
//! # What the numbers said, which is the point
//!
//! Cost tracked **blocks**, not parses and not rows: the same 800 rows cost 17 ms as plain
//! labels, 32 ms as one markdown block each, and 115 ms as the six blocks the fixture's rich
//! markdown produces. Caching the parse took 20–30% off, which is real and was not the bulk —
//! and virtualising removed the rest, because the blocks were being shaped whether or not the
//! row was on screen.
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
//! # Not a gate — except the one that has to be
//!
//! The three timings are on one machine and would be a flaky gate anywhere else, so they are
//! `#[ignore]`d and print rather than assert — the same arrangement as the terminal's
//! `frame_dump`, and for the same reason: the value is in running it when the path changes.
//! **None of the three is weakened or skipped**; they are the record of why virtualisation
//! was chosen, and they still print what they printed.
//!
//! **The fourth test is not `#[ignore]`d, and that is the exception rather than the rule.**
//! `the_frame_carries_the_accessibility_tree_the_window_publishes` costs one frame and asserts
//! nothing about time, and it is the only thing in this file that would have caught the
//! instrument going blind: a benchmark that cannot see the tree cannot fail when the tree
//! stops being built, because it never knew the tree was there.

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

/// One frame through the production draw path, at the size a real window uses,
/// **with the accessibility tree switched on**.
///
/// The `enable_accesskit` is the whole point of the change, and it is here rather than in the
/// fixture because a frame measured without it is **a frame the product does not draw**:
/// eframe turns the tree on when the platform asks for the initial tree
/// (`eframe-0.36.2/src/native/winit_integration.rs:180`) and never turns it off again, and
/// `Context::enable_accesskit` is documented to do the same for every future frame
/// (`egui-0.36.2/src/context.rs:3700`).
///
/// Until M288 this helper painted with the tree off, which is not a smaller measurement but a
/// **different question**: `Context::accesskit_node_builder` returns `None` unless the pass
/// state exists (`egui-0.36.2/src/context.rs:600-601`) and only `enable_accesskit` creates it
/// (`egui-0.36.2/src/context.rs:520-533`), so every `name_node` and `name_region` call in
/// `IdeApp::paint` was writing nothing and the cost of building nodes was never in any number
/// below. That is the gap ADR-022 records, and it is why M286's transcript-row naming could not
/// be measured here.
///
/// The `FullOutput` is **returned rather than dropped**, so a caller can read
/// `platform_output.accesskit_update` — the same `accesskit::TreeUpdate` the platform adapter
/// would receive — and check the frame really carried a tree. [`ms_per_frame`] does that on its
/// warm-up; [`the_frame_carries_the_accessibility_tree_the_window_publishes`] is the gate.
fn paint_once(ctx: &egui::Context, app: &mut IdeApp) -> egui::FullOutput {
    ctx.enable_accesskit();
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
    full
}

/// **The control that keeps this instrument honest.** A frame with the tree off produces no
/// update at all, and a frame whose tree holds only the window root node means egui built no
/// widget nodes — either way the numbers timed around it would be describing a screen no user
/// sees, and the only symptom would be a column that stops moving.
///
/// Fails loudly rather than printing, because a benchmark that reports a cost it cannot see is
/// exactly the failure `AGENTS.md` §5 names: *something reports success while the thing you
/// wanted did not happen*.
fn assert_frame_carries_an_accessibility_tree(full: &egui::FullOutput) {
    let tree = full.platform_output.accesskit_update.as_ref().expect(
        "egui produced no AccessKit tree update: this frame was painted with the \
             accessibility tree switched off, so it is a frame the product does not draw and \
             every number measured from it answers a different question.",
    );
    assert!(
        tree.nodes.len() > 1,
        "the tree holds only the window root node ({}), so egui built no widget nodes. The \
         instrument is measuring a frame with no screen-reader surface on it.",
        tree.nodes.len()
    );
}

/// Median-free, but not a single frame either: one warm-up, then `n`, and the mean.
///
/// The warm-up is **asserted, not merely discarded**: it is the frame that proves the timed
/// ones are frames with a tree, since a reader who finds a number that stopped tracking the
/// code has to be able to ask the benchmark whether it was still looking at the right thing.
fn ms_per_frame(label: &str, ctx: &egui::Context, app: &mut IdeApp, n: u32) {
    assert_frame_carries_an_accessibility_tree(&paint_once(ctx, app));
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

/// **The instrument cannot go blind again.**
///
/// The blindness was invisible in every number this file printed: with the accessibility tree
/// off, each frame still painted, still cost about the same, and the benchmark still reported
/// `ok`. Nothing in a timing can tell you what it failed to look at, so this is a test — and it
/// is the **only test here that is not `#[ignore]`d**, because it asserts rather than times and
/// costs one frame.
///
/// **It reads the frame, not a field on the app.** `accesskit_update` is the
/// `accesskit::TreeUpdate` egui hands the platform adapter
/// (`egui-0.36.2/src/context.rs:2710`), so a pass in which the app computed the perfect name
/// and never attached it still fails here — which is the same reason the in-crate a11y tests
/// read it, in `Harness::a11y_frame` (`crates/epsilon-desktop/src/app.rs`). Named rather than
/// line-cited because that file is written by someone else and its line numbers move.
///
/// **Negative control, run on 2026-10-04 and it failed for the right reason.** With the
/// `ctx.enable_accesskit()` line in [`paint_once`] commented out, this test panics with
/// *"egui produced no AccessKit tree update: this frame was painted with the accessibility tree
/// switched off…"* at `frame_cost.rs:223` — and nothing else in the file changes behaviour,
/// because `paint_once` is the only thing that ever switched it on.
///
/// The second control — tree enabled, `IdeApp::paint` emitting nothing — is **not** run, and
/// this test does not claim it would pass. Proving it means stubbing the draw path, which is a
/// file this test does not own; the `tree.nodes.len() > 1` assertion is therefore argued rather
/// than demonstrated, and stated as such here rather than left for a reader to assume.
#[test]
fn the_frame_carries_the_accessibility_tree_the_window_publishes() {
    let ws = tempfile::tempdir().expect("a workspace");
    let (ctx, mut app) = wire(ws.path());
    let full = paint_once(&ctx, &mut app);
    assert_frame_carries_an_accessibility_tree(&full);

    let tree = full
        .platform_output
        .accesskit_update
        .as_ref()
        .expect("checked by assert_frame_carries_an_accessibility_tree");
    // Only nodes with a **name**, because a name is what a screen reader announces and a node
    // carrying its text as a bare value announces nothing about what it is. Same rule as
    // `named_nodes` in the in-crate harness.
    let named: Vec<&str> = tree.nodes.iter().filter_map(|(_, n)| n.label()).collect();
    assert!(
        !named.is_empty(),
        "the tree holds {} nodes and not one of them is named, so this frame gives a screen \
         reader nothing to announce.\nthe roles it did carry: {:?}",
        tree.nodes.len(),
        tree.nodes.iter().map(|(_, n)| n.role()).collect::<Vec<_>>()
    );

    // **Zoom, because the "no input" claim stopped being free.** `paint` binds `Ctrl`+`+`/`-`/`0`
    // through egui's own `gui_zoom` before it reads the scale (`IdeApp::paint`,
    // `crates/epsilon-desktop/src/app.rs:2146-2163`), so a frame *can* now be drawn at a scale
    // other than 1.0. This input carries no key events, so the factor must still be 1.0 — and
    // pinning it here is what keeps the tables in the header comparable to the M285 column,
    // which was taken before the zoom was reachable at all. Asserted rather than reasoned about,
    // because that is the kind of claim that rots quietly.
    assert_eq!(
        ctx.zoom_factor(),
        1.0,
        "the timed frame is not at zoom 1.0, so it is not the frame the tables above describe \
         and the numbers are no longer comparable to the M285 column."
    );
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
    ///
    /// **Historical, and kept as a record.** This is the *unvirtualised* frame, measured when
    /// egui's per-block shaping was 95% of it and the phase breakdown below apportioned the
    /// cost accordingly. Since M285 the transcript is virtualised and the frame is a couple of
    /// milliseconds at 800 rows, so the percentage this prints is no longer a statement about
    /// where the time goes. The module header records why the fix was chosen; this constant is
    /// left as the number the decision was made against, and the comment in the header says
    /// so.
    ///
    /// **It also predates M288**, and so is a frame with the accessibility tree switched off —
    /// the "EGUI'S OWN" remainder below is therefore "egui's own work *including node
    /// building*", not "egui's own text shaping". Nothing this test prints changed when
    /// `paint_once` was fixed, because nothing here paints; what changed is how the printed
    /// remainder should be read, and the only honest move for it is to call it historical too.
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
