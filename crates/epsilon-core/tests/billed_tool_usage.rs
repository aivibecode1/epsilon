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
