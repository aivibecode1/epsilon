//! M70: the interrupted-sequence repair, through the real loop.
//!
//! A run can end between the assistant's tool-call block and its results — an abort, a crash, a
//! `Ctrl+C` at the wrong moment. The transcript is then structurally invalid for the next
//! request: providers validate that every tool call has a result, so the error arrives one
//! request later, far from its cause, as a 400 nobody can explain.
//!
//! `epsilon_session::repair::repair_on_open` closes that shape for a *reopened* file. These
//! scenarios cover the same shape in a live process, which is what `run()` has to survive.

use std::sync::Arc;

use epsilon_contracts::content::{ContentBlock, UserContent};
use epsilon_contracts::message::{
    AssistantMessage, Message, StopReason, ToolResultMessage, Usage, UserMessage,
};
use epsilon_core::{AutoAllow, LoopLimits, ToolRegistry};
use epsilon_llm::{FauxProvider, FauxStep};

fn user(text: &str) -> Message {
    Message::User(UserMessage {
        id: epsilon_contracts::new_id("msg"),
        timestamp_ms: 0,
        content: vec![UserContent::Text {
            text: text.to_string(),
        }],
    })
}

/// An assistant turn that asked for two tools and never got their results.
fn assistant_with_dangling_calls() -> Message {
    Message::Assistant(AssistantMessage {
        id: epsilon_contracts::new_id("msg"),
        timestamp_ms: 0,
        provider: "faux".into(),
        model_id: "faux-1".into(),
        content: vec![
            ContentBlock::ToolCall {
                id: "call_a".into(),
                name: "bash".into(),
                arguments: serde_json::json!({"command": "cargo test"}),
            },
            ContentBlock::ToolCall {
                id: "call_b".into(),
                name: "read".into(),
                arguments: serde_json::json!({"path": "src/lib.rs"}),
            },
        ],
        stop_reason: StopReason::ToolUse,
        error_message: None,
        usage: Usage::default(),
    })
}

fn agent() -> Arc<epsilon_core::Agent> {
    epsilon_core::Agent::new(
        LoopLimits::default(),
        Arc::new(epsilon_llm::PanicGuard(FauxProvider::new(vec![
            FauxStep::say("carrying on"),
        ]))),
        Arc::new(ToolRegistry::new()),
        Arc::new(AutoAllow),
        std::env::temp_dir(),
    )
}

/// Every tool call in the transcript has exactly one result.
fn every_call_has_a_result(history: &[Message]) -> Result<(), String> {
    let mut open: Vec<String> = Vec::new();
    for m in history {
        match m {
            Message::Assistant(a) => {
                for (id, _, _) in a.tool_calls() {
                    open.push(id.to_string());
                }
            }
            Message::ToolResult(t) => open.retain(|id| id != &t.tool_call_id),
            _ => {}
        }
    }
    if open.is_empty() {
        Ok(())
    } else {
        Err(format!("unanswered tool calls: {open:?}"))
    }
}

#[tokio::test]
async fn a_run_that_starts_on_a_dangling_call_closes_it_before_the_next_request() {
    let agent = agent();
    // The state an interrupted run leaves behind: a user turn, an assistant turn asking for two
    // tools, and no results (the process died, or the user aborted).
    agent.restore_history(vec![
        user("make the tests pass"),
        assistant_with_dangling_calls(),
    ]);

    agent.run(user("what happened?")).await.expect("run");

    let history = agent.history();
    every_call_has_a_result(&history).expect("the transcript is valid after the repair");

    let synthesized: Vec<&ToolResultMessage> = history
        .iter()
        .filter_map(|m| match m {
            Message::ToolResult(t) if t.tool_name == "bash" || t.tool_name == "read" => Some(t),
            _ => None,
        })
        .collect();
    assert_eq!(synthesized.len(), 2, "both calls were closed");
    for t in &synthesized {
        assert!(
            t.is_error,
            "a call that never ran is reported as an error, not a success"
        );
        let text = t
            .content
            .iter()
            .filter_map(|c| match c {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            text.contains("never executed"),
            "and it says what happened: {text}"
        );
        assert_eq!(
            t.details["interrupted"], true,
            "with a machine-readable marker for frontends"
        );
    }
    // The results are paired with the calls they answer, in order.
    let ids: Vec<&str> = synthesized
        .iter()
        .map(|t| t.tool_call_id.as_str())
        .collect();
    assert_eq!(ids, vec!["call_a", "call_b"]);
}

#[tokio::test]
async fn a_healthy_transcript_is_left_alone() {
    let agent = agent();
    // The same shape, but with results — nothing to repair, and nothing may be added.
    let mut answered = assistant_with_dangling_calls();
    let Message::Assistant(a) = &mut answered else {
        unreachable!()
    };
    let results: Vec<Message> = a
        .tool_calls()
        .map(|(id, name, _)| {
            Message::ToolResult(ToolResultMessage {
                id: epsilon_contracts::new_id("msg"),
                timestamp_ms: 0,
                tool_call_id: id.to_string(),
                tool_name: name.to_string(),
                content: vec![ContentBlock::text("all good")],
                details: serde_json::json!({}),
                usage: None,
                is_error: false,
            })
        })
        .collect();
    let mut history = vec![user("make the tests pass"), answered];
    history.extend(results);
    agent.restore_history(history.clone());

    let before = history
        .iter()
        .filter(|m| matches!(m, Message::ToolResult(_)))
        .count();
    agent.run(user("thanks")).await.expect("run");

    let after = agent.history();
    let results = after
        .iter()
        .filter(|m| matches!(m, Message::ToolResult(_)))
        .count();
    assert_eq!(
        results, before,
        "no result was invented for calls that already had one: {after:#?}"
    );
    every_call_has_a_result(&after).expect("still valid");
}

#[tokio::test]
async fn a_second_run_does_not_close_the_same_calls_twice() {
    let agent = agent();
    agent.restore_history(vec![
        user("make the tests pass"),
        assistant_with_dangling_calls(),
    ]);

    agent.run(user("first")).await.expect("run");
    let after_first = agent.history().len();
    agent.run(user("second")).await.expect("run");

    // The second run must not add another pair of results for calls already answered.
    let results = agent
        .history()
        .iter()
        .filter(|m| matches!(m, Message::ToolResult(_)))
        .count();
    assert_eq!(results, 2, "the repair is idempotent");
    assert!(
        agent.history().len() > after_first,
        "the run itself did happen"
    );
}
