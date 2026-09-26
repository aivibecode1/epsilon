//! M68: the stop-time verification nudge.
//!
//! `agent-contract.md` tells the model to run the project's checks after editing, and nothing
//! recorded whether it did. These scenarios drive the real loop: an edit with no check behind it
//! earns a nudge at the stop point (twice at most), an edit followed by a passing check does not,
//! and read-only work is never interrupted.

use std::sync::Arc;

use async_trait::async_trait;
use epsilon_contracts::content::UserContent;
use epsilon_contracts::message::{Message, UserMessage};
use epsilon_contracts::tool::{
    ExecutionMode, Tool, ToolContext, ToolDescriptor, ToolFailure, ToolInvocation, ToolOutput,
    ToolUpdateSink,
};
use epsilon_core::{AutoAllow, LoopLimits, ToolRegistry};
use epsilon_llm::{FauxProvider, FauxStep, FauxToolCall};

/// A tool that succeeds and echoes something harmless. Stands in for `write` / `edit` / `bash`
/// without touching a filesystem or a shell.
struct Answers {
    descriptor: ToolDescriptor,
    /// When set, the tool fails with this message instead of succeeding.
    fail_with: Option<String>,
}

#[async_trait]
impl Tool for Answers {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    async fn execute(
        &self,
        _call: ToolInvocation,
        _ctx: ToolContext,
        _on_update: ToolUpdateSink,
    ) -> Result<ToolOutput, ToolFailure> {
        match &self.fail_with {
            Some(message) => Err(ToolFailure::msg(message.clone())),
            None => Ok(ToolOutput::text("ok")),
        }
    }
}

fn tool(name: &str) -> Arc<dyn Tool> {
    Arc::new(Answers {
        descriptor: ToolDescriptor {
            name: name.into(),
            label: name.into(),
            description: "test fixture".into(),
            parameters_schema: serde_json::json!({"type": "object"}),
            execution_mode: ExecutionMode::Sequential,
        },
        fail_with: None,
    })
}

/// A `bash` whose commands fail — for "the check ran and did not pass".
fn failing_bash() -> Arc<dyn Tool> {
    Arc::new(Answers {
        descriptor: ToolDescriptor {
            name: "bash".into(),
            label: "Bash".into(),
            description: "test fixture".into(),
            parameters_schema: serde_json::json!({"type": "object"}),
            execution_mode: ExecutionMode::Sequential,
        },
        fail_with: Some("exit status: 101".into()),
    })
}

/// A `bash` that *delivers* a command which failed — the F05 shape.
///
/// `failing_bash()` above returns `Err(ToolFailure)`, so it exercises "the tool call itself
/// failed". Real shell work almost never looks like that: `BashTool` reports a command exiting
/// nonzero as a **successful** tool result whose `details.exit_code` carries the status, because
/// the tool did its job and the program it ran did not. That shape is pinned on the producing
/// side by `a_non_zero_exit_is_a_command_outcome_never_a_confinement_one`
/// (`crates/epsilon-tools/tests/builtin_tools.rs`), which asserts `details["exit_code"] == 7`,
/// `details["verdict"] == "exited:7"` and an `Ok` return. Until F05 this second shape was
/// untested, and it is the one a user's failing test suite actually produces.
struct DeliveredBash {
    descriptor: ToolDescriptor,
    exit_code: Option<i32>,
}

fn delivered_bash(exit_code: Option<i32>) -> Arc<dyn Tool> {
    Arc::new(DeliveredBash {
        descriptor: ToolDescriptor {
            name: "bash".into(),
            label: "Bash".into(),
            description: "test fixture".into(),
            parameters_schema: serde_json::json!({"type": "object"}),
            execution_mode: ExecutionMode::Sequential,
        },
        exit_code,
    })
}

#[async_trait]
impl Tool for DeliveredBash {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    async fn execute(
        &self,
        _call: ToolInvocation,
        _ctx: ToolContext,
        _on_update: ToolUpdateSink,
    ) -> Result<ToolOutput, ToolFailure> {
        Ok(ToolOutput {
            content: vec![epsilon_contracts::content::ContentBlock::Text {
                text: "running 3 tests".into(),
            }],
            details: serde_json::json!({
                "exit_code": self.exit_code,
                "duration_ms": 412,
                "truncated": false,
            }),
            // The real `BashTool` sets exactly these two: a shell run has no model usage of its
            // own and never asks the loop to stop (`shell.rs:350,356`).
            usage: None,
            terminate: false,
        })
    }
}

fn user(text: &str) -> Message {
    Message::User(UserMessage {
        id: epsilon_contracts::new_id("msg"),
        timestamp_ms: 0,
        content: vec![UserContent::Text {
            text: text.to_string(),
        }],
    })
}

fn call(id: &str, name: &str, args: serde_json::Value) -> FauxToolCall {
    FauxToolCall {
        id: id.into(),
        name: name.into(),
        arguments: args,
    }
}

fn agent_with(
    tools: Vec<Arc<dyn Tool>>,
    steps: Vec<FauxStep>,
    limits: LoopLimits,
) -> Arc<epsilon_core::Agent> {
    let mut registry = ToolRegistry::new();
    for t in tools {
        registry.register(t);
    }
    epsilon_core::Agent::new(
        limits,
        Arc::new(epsilon_llm::PanicGuard(FauxProvider::new(steps))),
        Arc::new(registry),
        Arc::new(AutoAllow),
        std::env::temp_dir(),
    )
}

fn nudges(agent: &epsilon_core::Agent) -> Vec<String> {
    agent
        .history()
        .iter()
        .filter_map(|m| match m {
            Message::User(u) => {
                let text = u
                    .content
                    .iter()
                    .filter_map(|c| match c {
                        UserContent::Text { text } => Some(text.clone()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                text.starts_with("[harness] you changed").then_some(text)
            }
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn an_edit_that_stops_without_a_check_is_nudged() {
    let steps = vec![
        FauxStep::ToolCalls {
            text: None,
            calls: vec![call(
                "tc_1",
                "edit",
                serde_json::json!({"path": "src/lib.rs"}),
            )],
        },
        // The model tries to stop here — no check has run.
        FauxStep::say("Done. I changed src/lib.rs."),
        // After the nudge, it checks.
        FauxStep::ToolCalls {
            text: None,
            calls: vec![call(
                "tc_2",
                "bash",
                serde_json::json!({"command": "cargo test"}),
            )],
        },
        FauxStep::say("Tests pass; src/lib.rs is verified."),
    ];
    let agent = agent_with(
        vec![tool("edit"), delivered_bash(Some(0))],
        // `tool("bash")` reported no exit status at all, which the corrected rule
        // reads as "unknown, so not a pass". These tests mean "a check that passed", so they
        // now use the shape a real passing `cargo test` produces.
        steps,
        LoopLimits {
            verification_nudges: 2,
            max_turns: 12,
            ..Default::default()
        },
    );
    agent.run(user("fix the bug")).await.expect("run");

    let n = nudges(&agent);
    assert_eq!(n.len(), 1, "exactly one nudge: {n:?}");
    assert!(n[0].contains("src/lib.rs"), "{}", n[0]);
    assert!(
        n[0].contains("no check has run"),
        "and it says what is missing: {}",
        n[0]
    );
}

#[tokio::test]
async fn a_passing_check_after_the_edit_ends_the_run_without_a_nudge() {
    let steps = vec![
        FauxStep::ToolCalls {
            text: None,
            calls: vec![call(
                "tc_1",
                "edit",
                serde_json::json!({"path": "src/lib.rs"}),
            )],
        },
        FauxStep::ToolCalls {
            text: None,
            calls: vec![call(
                "tc_2",
                "bash",
                serde_json::json!({"command": "cargo test"}),
            )],
        },
        FauxStep::say("Done, and the tests pass."),
    ];
    let agent = agent_with(
        vec![tool("edit"), delivered_bash(Some(0))],
        steps,
        LoopLimits::default(),
    );
    agent.run(user("fix the bug")).await.expect("run");
    assert!(
        nudges(&agent).is_empty(),
        "a verified edit must not be nagged about"
    );
}

#[tokio::test]
async fn a_check_that_did_not_pass_is_not_verification() {
    let steps = vec![
        FauxStep::ToolCalls {
            text: None,
            calls: vec![call(
                "tc_1",
                "edit",
                serde_json::json!({"path": "src/lib.rs"}),
            )],
        },
        FauxStep::ToolCalls {
            text: None,
            calls: vec![call(
                "tc_2",
                "bash",
                serde_json::json!({"command": "cargo test"}),
            )],
        },
        FauxStep::say("The tests fail but I am stopping anyway."),
    ];
    let agent = agent_with(
        vec![tool("edit"), failing_bash()],
        steps,
        LoopLimits {
            verification_nudges: 1,
            max_turns: 12,
            ..Default::default()
        },
    );
    agent.run(user("fix the bug")).await.expect("run");

    let n = nudges(&agent);
    assert_eq!(
        n.len(),
        1,
        "the failing check still leaves the edit unverified"
    );
    assert!(
        n[0].contains("none succeeding after the change"),
        "and the nudge says why: {}",
        n[0]
    );
}

#[tokio::test]
async fn a_delivered_check_that_exited_nonzero_is_not_verification() {
    // F05. Same scenario as the test above, but the check *succeeded as a tool call* and the
    // command it ran failed — which is what a real failing `cargo test` looks like. Before the
    // fix the loop read `!result.is_error`, saw a clean result, and stood down.
    let steps = vec![
        FauxStep::ToolCalls {
            text: None,
            calls: vec![call(
                "tc_1",
                "edit",
                serde_json::json!({"path": "src/lib.rs"}),
            )],
        },
        FauxStep::ToolCalls {
            text: None,
            calls: vec![call(
                "tc_2",
                "bash",
                serde_json::json!({"command": "cargo test"}),
            )],
        },
        FauxStep::say("The tests fail but I am stopping anyway."),
    ];
    let agent = agent_with(
        vec![tool("edit"), delivered_bash(Some(101))],
        steps,
        LoopLimits {
            verification_nudges: 1,
            max_turns: 12,
            ..Default::default()
        },
    );
    agent.run(user("fix the bug")).await.expect("run");

    let n = nudges(&agent);
    assert_eq!(
        n.len(),
        1,
        "a check that exited 101 is not a passing check, however cleanly it was delivered"
    );
    assert!(
        n[0].contains("none succeeding after the change"),
        "and the nudge says why: {}",
        n[0]
    );
}

#[tokio::test]
async fn a_delivered_check_that_exited_zero_is_verification() {
    // The positive control for the test above, and the reason the fix is not "nudge always".
    let steps = vec![
        FauxStep::ToolCalls {
            text: None,
            calls: vec![call(
                "tc_1",
                "edit",
                serde_json::json!({"path": "src/lib.rs"}),
            )],
        },
        FauxStep::ToolCalls {
            text: None,
            calls: vec![call(
                "tc_2",
                "bash",
                serde_json::json!({"command": "cargo test"}),
            )],
        },
        FauxStep::say("All green."),
    ];
    let agent = agent_with(
        vec![tool("edit"), delivered_bash(Some(0))],
        steps,
        LoopLimits {
            verification_nudges: 1,
            max_turns: 12,
            ..Default::default()
        },
    );
    agent.run(user("fix the bug")).await.expect("run");

    assert!(
        nudges(&agent).is_empty(),
        "a check that exited 0 leaves nothing standing behind the edit"
    );
}

#[tokio::test]
async fn a_check_with_no_reported_exit_code_is_not_verification() {
    // Signalled, or a tool that reports no status. Unknown must not read as clean.
    let steps = vec![
        FauxStep::ToolCalls {
            text: None,
            calls: vec![call(
                "tc_1",
                "edit",
                serde_json::json!({"path": "src/lib.rs"}),
            )],
        },
        FauxStep::ToolCalls {
            text: None,
            calls: vec![call(
                "tc_2",
                "bash",
                serde_json::json!({"command": "cargo test"}),
            )],
        },
        FauxStep::say("Done."),
    ];
    let agent = agent_with(
        vec![tool("edit"), delivered_bash(None)],
        steps,
        LoopLimits {
            verification_nudges: 1,
            max_turns: 12,
            ..Default::default()
        },
    );
    agent.run(user("fix the bug")).await.expect("run");

    assert_eq!(
        nudges(&agent).len(),
        1,
        "an unknown outcome stays unknown rather than counting as a pass"
    );
}

#[tokio::test]
async fn a_check_in_the_same_batch_before_an_edit_does_not_verify_it() {
    // Both calls are issued in ONE assistant turn. The old rule compared turn numbers, and
    // same-turn meant "at or after", so a check that ran first vouched for an edit that had not
    // happened yet when it ran.
    let steps = vec![
        FauxStep::ToolCalls {
            text: None,
            calls: vec![
                call("tc_1", "bash", serde_json::json!({"command": "cargo test"})),
                call("tc_2", "edit", serde_json::json!({"path": "src/lib.rs"})),
            ],
        },
        FauxStep::say("All green — the suite passed."),
    ];
    let agent = agent_with(
        vec![tool("edit"), delivered_bash(Some(0))],
        steps,
        LoopLimits {
            verification_nudges: 1,
            max_turns: 12,
            ..Default::default()
        },
    );
    agent.run(user("fix the bug")).await.expect("run");

    assert_eq!(
        nudges(&agent).len(),
        1,
        "a passing check that ran *before* the edit cannot verify it"
    );
}

#[tokio::test]
async fn read_only_work_is_never_nudged() {
    let steps = vec![
        FauxStep::ToolCalls {
            text: None,
            calls: vec![call(
                "tc_1",
                "read",
                serde_json::json!({"path": "src/lib.rs"}),
            )],
        },
        FauxStep::say("Here is what the file does."),
    ];
    let agent = agent_with(vec![tool("read")], steps, LoopLimits::default());
    agent.run(user("explain this file")).await.expect("run");
    assert!(nudges(&agent).is_empty());
}

#[tokio::test]
async fn the_nudge_gets_two_chances_and_then_stops() {
    // The model ignores both nudges and keeps saying it is done: the run ends (the harness does
    // not refuse to stop), still carrying the record.
    let steps = vec![
        FauxStep::ToolCalls {
            text: None,
            calls: vec![call(
                "tc_1",
                "edit",
                serde_json::json!({"path": "src/a.rs"}),
            )],
        },
        FauxStep::say("done"),
        FauxStep::say("done"),
        FauxStep::say("done"),
        FauxStep::say("done"),
    ];
    let agent = agent_with(
        vec![tool("edit")],
        steps,
        LoopLimits {
            verification_nudges: 2,
            max_turns: 12,
            ..Default::default()
        },
    );
    let outcome = agent.run(user("change it")).await.expect("run");
    assert_eq!(nudges(&agent).len(), 2, "at most two nudges, as configured");
    assert_eq!(
        outcome.reason,
        epsilon_contracts::agent_event::TurnEndReason::Completed,
        "the harness records and asks; it does not refuse to stop"
    );
}
