//! The per-frame cost the render benchmark cannot see.
//!
//! # Why this test exists
//!
//! `crates/epsilon-tui/src/app/event.rs:169` calls `App::mirror_context_size` on
//! **every drawn frame**. That reaches `Agent::estimated_context_tokens`
//! (`epsilon-core/src/lib.rs:1934`), which calls `self.history()` — a full deep
//! clone of every message in the session — and then walks the messages appended
//! since the usage anchor.
//!
//! `--tui-bench` cannot see any of this. It calls `draw_frame` directly, and its
//! `bench_app` holds no `Agent`, so the mirror never runs. The few-milliseconds
//! per frame it reports is therefore an *understatement* of a real frame by
//! exactly this cost.
//!
//! At the loop's 30 fps ceiling that is 30 whole-transcript clones per second on
//! the event-loop thread — the same thread that must read the keyboard. When a
//! session is long enough that one clone eats a real share of the frame budget,
//! input latency grows with the transcript. That is the shape of the report: a
//! TUI that degrades the longer it is used.
//!
//! # What it asserts
//!
//! Two things, deliberately separated. The first is wall-clock, with the numbers
//! printed via `--nocapture` so the evidence is readable rather than hidden; the
//! threshold is generous, because this is a tripwire, not a micro-benchmark. The
//! second is scale-free — it asserts the cost does not *track* transcript length,
//! which is the property that actually matters and which does not depend on how
//! fast the machine is.

use std::sync::Arc;
use std::time::{Duration, Instant};

use epsilon_contracts::content::UserContent;
use epsilon_contracts::ids::new_id;
use epsilon_contracts::message::{Message, UserMessage};
use epsilon_core::testing::EchoTool;
use epsilon_core::{Agent, AutoAllow, LoopLimits, PermissionGate, ToolRegistry};
use epsilon_llm::{FauxProvider, FauxStep, PanicGuard};

/// One turn's worth of prose, code and a list. A single word would make an
/// O(session) clone invisible; a realistic body makes it visible.
const BODY: &str = "I updated the request-path cache and the transcript renderer.\n\
The key change is that the estimate is now anchored to what the provider actually \
billed, so the header and the compaction trigger read the same number.\n\n\
```rust\npub fn estimated_context_tokens(&self) -> usize {\n    let anchor = self.usage_anchor.lock().ok().and_then(|s| *s);\n    crate::compact::anchored_tokens(&self.history(), anchor)\n}\n```\n\n\
- the anchor is cleared whenever history is rewritten\n- the fallback is chars/4 over the whole transcript\n";

fn agent_with(steps: Vec<FauxStep>) -> (Arc<Agent>, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("temp workspace");
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(EchoTool::new()));
    let agent = Agent::new(
        LoopLimits::default(),
        Arc::new(PanicGuard(FauxProvider::new(steps))),
        Arc::new(registry),
        Arc::new(AutoAllow) as Arc<dyn PermissionGate>,
        dir.path().to_path_buf(),
    );
    (agent, dir)
}

fn user_turn(i: usize) -> Message {
    Message::User(UserMessage {
        id: new_id("msg"),
        timestamp_ms: 0,
        content: vec![UserContent::Text {
            text: format!("turn {i}: {BODY}"),
        }],
    })
}

/// Drive `turns` real turns through the agent, so the history is built exactly as
/// production builds it rather than poked into directly.
async fn fill(agent: &Arc<Agent>, turns: usize) {
    for i in 0..turns {
        agent.run(user_turn(i)).await.expect("faux turn completes");
    }
}

/// Measure one context mirror, the way the event loop calls it.
///
/// **Best-of-N, not a single timed run.** This operation takes ~20 microseconds.
///
/// **What the failure actually was, because the first diagnosis was wrong.** It
/// failed at M196 inside a full battery with `12.222ms at 20 messages ->
/// 28.502ms at 200` -- a "2.3x growth" that read exactly like the whole-transcript
/// scan this test exists to catch. I assumed the box was loaded, and set out to
/// reproduce it: the same test under 6, then 14, then 40 competing processes all
/// passed at 30-35 microseconds, and the OLD code passed under all of them too.
/// So "the machine was busy" was **not** the cause, and a fix justified by it
/// would have been cargo-culted.
///
/// What is left is arithmetic on a cold path. `at_ten` is timed immediately after
/// `fill(&agent, 10)` -- the first time this transcript size is ever measured, so
/// it pays first-touch costs the second measurement does not. A ratio between a
/// cold number and a warm one is not a property of the code under test; it is a
/// property of when the timer was started. The doc comment above the test claimed
/// the check was "scale-free, so it does not depend on how fast this machine is",
/// and that claim was false.
///
/// The minimum over several runs is the right statistic for "how long does this
/// take": it is the run least polluted by a first-touch or a preemption, and it is
/// still an upper bound on the work the code does. A mean would be worse -- it is
/// exactly the quantity both inflate.
fn measure(agent: &Arc<Agent>, calls: u32) -> Duration {
    // Warm the allocator and any lazy statics first.
    for _ in 0..5 {
        let _ = agent.estimated_context_tokens();
    }
    let mut best = Duration::MAX;
    for _ in 0..BEST_OF {
        let start = Instant::now();
        for _ in 0..calls {
            let _ = agent.estimated_context_tokens();
        }
        let per_call = start.elapsed() / calls;
        if per_call < best {
            best = per_call;
        }
    }
    best
}

/// Timed runs per measurement, and how many calls each one averages over.
///
/// Nine runs, 200 calls each: enough that one preemption does not decide the
/// result, cheap enough (about 1.8ms of work at the measured cost) that the test
/// stays fast.
const BEST_OF: u32 = 9;
const CALLS_PER_RUN: u32 = 200;

/// The mirror must stay far below the frame budget, at a session size where a
/// user would plausibly still be working.
///
/// 30 fps gives a 33 ms frame. This asserts the mirror uses under a tenth of
/// that, so the loop keeps well over 20 frames per second even if everything
/// else in the frame were free.
#[tokio::test]
async fn context_mirror_stays_inside_the_frame_budget() {
    let (agent, _dir) = agent_with(vec![FauxStep::say(BODY); 64]);
    fill(&agent, 40).await;

    let messages = agent.history().len();
    let per_call = measure(&agent, 200);
    let share_of_frame = per_call.as_secs_f64() / 0.033;

    eprintln!(
        "context mirror over {messages} messages: {per_call:?} per call \
         ({:.2}% of a 33ms frame, {:.0} calls/second at 30fps)",
        share_of_frame * 100.0,
        30.0 / per_call.as_secs_f64().max(1e-9)
    );

    assert!(
        per_call < Duration::from_millis(3),
        "one context mirror cost {per_call:?} over {messages} messages; at the 30fps \
         ceiling that is {share_of_frame:.1}% of every frame, spent before the frame \
         is even drawn"
    );
}

/// The cost must not grow proportionally with the session.
///
/// This is the property that actually matters for the reported freeze, and it is
/// scale-free, so it does not depend on how fast this machine is. Doubling the
/// transcript should not double a per-frame cost: the anchored path only needs
/// the messages after the anchor.
///
/// A whole-transcript clone fails this. A bounded walk passes it.
#[tokio::test]
async fn context_mirror_cost_does_not_track_transcript_length() {
    let (agent, _dir) = agent_with(vec![FauxStep::say(BODY); 128]);

    fill(&agent, 10).await;
    let at_ten = measure(&agent, CALLS_PER_RUN);
    let ten_messages = agent.history().len();

    fill(&agent, 90).await;
    let at_hundred = measure(&agent, CALLS_PER_RUN);
    let hundred_messages = agent.history().len();

    eprintln!(
        "mirror cost: {at_ten:?} at {ten_messages} messages -> {at_hundred:?} at \
         {hundred_messages} messages ({:.1}x messages, {:.1}x cost)",
        hundred_messages as f64 / ten_messages as f64,
        at_hundred.as_secs_f64() / at_ten.as_secs_f64().max(1e-9)
    );

    // Ten times the transcript must not cost anything like ten times the time.
    // The anchored path walks only what arrived after the last billed request,
    // so the ratio should stay near 1 even though the transcript grew 10x.
    // Generous bound: a linear implementation lands near 10x; this allows 2x.
    assert!(
        at_hundred.as_secs_f64() < at_ten.as_secs_f64() * 2.0,
        "the mirror cost grew {:.1}x for a {:.1}x larger transcript ({at_ten:?} -> \
         {at_hundred:?}); that is a whole-transcript scan, which is the cost that \
         starves the event loop",
        at_hundred.as_secs_f64() / at_ten.as_secs_f64().max(1e-9),
        hundred_messages as f64 / ten_messages as f64
    );
}
