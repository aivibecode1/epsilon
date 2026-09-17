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
        vec![tool("edit"), tool("bash")],
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
        vec![tool("edit"), tool("bash")],
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
