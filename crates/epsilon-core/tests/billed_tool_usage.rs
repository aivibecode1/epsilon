//! A tool that bills its own provider call must reach the session's numbers (M229, ledger A11).
//!
//! The `task` tool is the only tool that reports `usage` today, and a subagent's tokens are the
//! parent's spend: the child runs real provider calls on the parent's account. Before this, a
//! completed task's usage was **stored on the task entry and persisted in the tool result, and
//! aggregated nowhere visible** — the provider stream sums only assistant messages, so the turn
//! total, `session_usage()` and `session_usage_by_model()` all under-reported by exactly the
//! child's cost, while `/task list` printed turns and a preview without a number.
//!
//! This file pins the *accounting rule* rather than the task machinery: any tool that returns a
//! bill, with or without a model, must move the turn's total, and one that names the model it ran
//! on must also be filed under it. Stating it once is what lets a second billing tool inherit it.
//!
//! Offline and deterministic: the faux provider bills every assistant turn 100 in / 50 out, and
//! this fixture's own bill is the numbers below.

use std::sync::Arc;

use epsilon_contracts::content::{ContentBlock, UserContent};
use epsilon_contracts::message::{Message, Usage, UserMessage};
use epsilon_contracts::tool::{
    ExecutionMode, Tool, ToolContext, ToolDescriptor, ToolFailure, ToolInvocation, ToolOutput,
    ToolUpdateSink,
};
use epsilon_core::{Agent, AutoAllow, LoopLimits, ToolRegistry};
use epsilon_llm::{FauxProvider, FauxStep, PanicGuard};

/// What this fixture's delegated call bills — deliberately **larger than the faux provider's own
/// turn** (100/50), so "the total includes it" cannot be satisfied by the parent's two turns. A
/// test that passes against the broken build because its threshold was too low is the failure mode
/// this number exists to prevent.
const DELEGATED_BILL: (u64, u64) = (7_000, 3_000);

/// A tool that did its own provider work and says so, optionally naming the model it billed to.
struct BillingTool {
    descriptor: ToolDescriptor,
    /// The model the call was billed to, as a tool reports it in `details`.
    billed_model: Option<&'static str>,
}

#[async_trait::async_trait]
impl Tool for BillingTool {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    async fn execute(
        &self,
        _call: ToolInvocation,
        _ctx: ToolContext,
        _on_update: ToolUpdateSink,
    ) -> Result<ToolOutput, ToolFailure> {
        let details = match self.billed_model {
            Some(m) => serde_json::json!({ "model": m }),
            None => serde_json::json!({}),
        };
        Ok(ToolOutput {
            content: vec![ContentBlock::text("delegated work done")],
            details,
            usage: Some(Usage {
                input: DELEGATED_BILL.0,
                output: DELEGATED_BILL.1,
                total_tokens: DELEGATED_BILL.0 + DELEGATED_BILL.1,
                ..Default::default()
            }),
            terminate: false,
        })
    }
}

fn billing_tool(name: &str, billed_model: Option<&'static str>) -> Arc<dyn Tool> {
    Arc::new(BillingTool {
        descriptor: ToolDescriptor {
            name: name.into(),
            label: "Scout".into(),
            description: "does delegated work and reports what it cost".into(),
            parameters_schema: serde_json::json!({ "type": "object" }),
            execution_mode: ExecutionMode::Sequential,
        },
        billed_model,
    })
}

fn agent_with_tool(steps: Vec<FauxStep>, tool: Arc<dyn Tool>) -> Arc<Agent> {
    let mut registry = ToolRegistry::new();
    registry.register(tool);
    Agent::new(
        LoopLimits::default(),
        Arc::new(PanicGuard(FauxProvider::new(steps))),
        Arc::new(registry),
        Arc::new(AutoAllow),
        std::env::temp_dir(),
    )
}

fn user_msg(text: &str) -> Message {
    Message::User(UserMessage {
        id: epsilon_contracts::new_id("msg"),
        timestamp_ms: 0,
        content: vec![UserContent::Text {
            text: text.to_string(),
        }],
    })
}

/// A billed tool call is part of the turn's spend, and it is filed under the model it names.
///
/// Both halves are asserted because either alone would pass against a broken build: a total that
/// counts the bill but files it under nothing, and a map that has the row but a total that omits it,
/// are the two ways this can be half-done.
#[tokio::test]
async fn a_tool_that_bills_its_own_provider_call_is_in_the_turns_total() {
    let agent = agent_with_tool(
        vec![
            FauxStep::call("t1", "scout", serde_json::json!({})),
            FauxStep::say("done"),
        ],
        billing_tool("scout", Some("cheap-1")),
    );

    agent
        .run(user_msg("send a scout"))
        .await
        .expect("the turn runs");

    let total = agent.session_usage();
    assert!(
        total.input >= DELEGATED_BILL.0 && total.output >= DELEGATED_BILL.1,
        "the tool's own provider call is the session's spend too: {total:?}"
    );

    let by_model = agent.session_usage_by_model();
    let child = by_model.get("cheap-1").unwrap_or_else(|| {
        panic!("the delegated call is billed to cheap-1, so that row exists: {by_model:?}")
    });
    assert!(
        child.input >= DELEGATED_BILL.0,
        "and it is filed under the model it names: {child:?}"
    );
}

/// A bill with no model cannot be priced per-model — but it is still **spent**, so the turn's total
/// must count it. The alternative is M141's defect in a new place: inventing a rate to fill a row.
#[tokio::test]
async fn a_bill_with_no_model_still_counts_toward_the_total() {
    let agent = agent_with_tool(
        vec![
            FauxStep::call("t1", "anon", serde_json::json!({})),
            FauxStep::say("done"),
        ],
        billing_tool("anon", None),
    );

    agent
        .run(user_msg("send an unattributed worker"))
        .await
        .expect("the turn runs");

    assert!(
        agent.session_usage().input >= DELEGATED_BILL.0,
        "an unattributed bill is still spend: {:?}",
        agent.session_usage()
    );
    assert!(
        !agent.session_usage_by_model().contains_key(""),
        "and it is not filed under an empty model name: {:?}",
        agent.session_usage_by_model()
    );
}

/// **A compaction's summarizer call is the session's spend, in the numbers a reader can see.**
///
/// The bill reaches the durable `Compaction` entry (M145/F22) and the `CompactionFinished`
/// event (so the live UI could rebuild it) — and **neither in-memory total**. The session
/// totals are folded at exactly two points, both inside the run loop (`lib.rs:2686-2688`
/// and `:2352-2354`), and a compaction happens *outside* that loop: it is the summarizer's
/// own aux call, which the provider stream never sums. So the summarizer — the call that
/// resends the whole context, and the most expensive one in a long session — was free in
/// every number the desktop's cost panel and `get_session_stats` read.
///
/// The TUI hid the hole: it re-derives its figures from `usage_accounts()` on hydration
/// (`state.rs:1701-1708`), so the terminal was right and the window was wrong, which is the
/// shape of a defect that survives because one front end happens to recover.
///
/// **Both halves are asserted** for the reason the tool-billing test states: a total that
/// counts the bill but files it under nothing, and a map that has the row while the total
/// omits it, are the two ways this is half-done.
#[tokio::test]
async fn a_compaction_bill_is_in_the_sessions_totals() {
    use epsilon_core::testing::SlowProvider;
    use epsilon_llm::FauxStep;
    use epsilon_session::SessionStore;

    let dir = tempfile::tempdir().unwrap();
    let store = SessionStore::open(dir.path().join("session.jsonl")).unwrap();
    // An `echo` tool: its arguments become a large tool result, which is what pushes the
    // estimated context over the threshold. The crate's own fixture, not a second copy.
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(epsilon_core::testing::EchoTool::new()));

    // A summarizer that bills far more than the faux provider's own 100/50 turn, so
    // "the total includes it" cannot be satisfied by the main loop's turns.
    let summarizer = Arc::new(SlowProvider::new(
        FauxProvider::new(vec![
            FauxStep::say("## Objective\nsummarized state"),
            FauxStep::say("final answer"),
        ]),
        0,
    ));
    let provider = Arc::new(epsilon_llm::PanicGuard(SlowProvider::new(
        FauxProvider::new(vec![
            FauxStep::call(
                "tc_1",
                "echo",
                serde_json::json!({ "text": "x".repeat(8000) }),
            ),
            FauxStep::say("done"),
        ]),
        0,
    )));

    let agent = Agent::new(
        LoopLimits {
            compaction_threshold_tokens: 1500,
            compaction_tail_tokens: 300,
            max_turns: 8,
            ..Default::default()
        },
        provider,
        Arc::new(registry),
        Arc::new(epsilon_core::AutoAllow),
        dir.path().to_path_buf(),
    );
    agent.set_summarizer(summarizer);
    agent.attach_session_store(Arc::new(std::sync::Mutex::new(store)));

    let outcome = agent.run(user_msg("long task")).await.expect("the run");
    assert!(
        !matches!(
            outcome.reason,
            epsilon_contracts::agent_event::TurnEndReason::Error { .. }
        ),
        "the run must not fail: {outcome:?}"
    );

    // The compaction really happened, or this test pins nothing. Written as a scan rather
    // than a nested `matches!` — the shape of the assertion is not the point and a paren
    // maze in a fixture is how a test ends up testing its own braces.
    let summarized = agent.history().iter().any(|m| {
        let epsilon_contracts::message::Message::User(u) = m else {
            return false;
        };
        u.content.iter().any(|c| {
            matches!(
                c,
                epsilon_contracts::content::UserContent::Text { text }
                    if text.contains("<compaction_summary>")
            )
        })
    });
    assert!(
        summarized,
        "the history must open with the compaction summary, or no summarizer ran"
    );

    let total = agent.session_usage();
    assert!(
        total.input >= MAIN_LOOP_BILL.0 + SUMMARIZER_BILL.0
            && total.output >= MAIN_LOOP_BILL.1 + SUMMARIZER_BILL.1,
        "the summarizer's call is the session's spend too: total {total:?}, the run's own \
         turns bill {MAIN_LOOP_BILL:?} and the summarizer's call {SUMMARIZER_BILL:?}"
    );

    let by_model = agent.session_usage_by_model();
    let summarizer_row = by_model.get("faux-1").unwrap_or_else(|| {
        panic!("the summarizer ran as faux-1, so that row exists: {by_model:?}")
    });
    assert!(
        summarizer_row.input >= MAIN_LOOP_BILL.0 + SUMMARIZER_BILL.0,
        "and it is filed under the model that answered: {summarizer_row:?}"
    );
}

/// What the summarizer's own call bills.
///
/// **The faux provider's fixed bill, not an invented figure** (`epsilon-llm/src/faux.rs:167-172`:
/// every response is 100 in / 50 out). The main loop runs two turns here — the `echo` call and
/// the final reply — so a broken build reports 200/100 and a fixed one 300/150. The assertion
/// below is on that arithmetic rather than on a threshold, because a number a test did not
/// measure is how the first version of this test came to assert 9000/4000 against a provider
/// that bills 100/50.
const SUMMARIZER_BILL: (u64, u64) = (100, 50);

/// The main loop's own spend in this fixture: two faux responses, each billed at the
/// provider's fixed rate.
const MAIN_LOOP_BILL: (u64, u64) = (200, 100);
