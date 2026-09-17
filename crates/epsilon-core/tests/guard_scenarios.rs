//! M67: guards for runs that look busy and go nowhere.
//!
//! The doom-loop brake catches the same *call* repeated. These two failure modes look nothing
//! like that:
//!
//! - **No progress**: every call differs, every result is the same. A stuck agent retries with
//!   `rg`, then `grep`, then `find`, gets the same "no such file" three ways, and burns the budget.
//! - **Empty turn**: the model finishes a run by saying nothing (or by repeating one line until
//!   it hits the token limit). The loop reads that as success.
//!
//! Both are asserted through the real loop: the guard has to reach the model, not just a log.

use std::sync::Arc;

use epsilon_contracts::agent_event::{AgentEvent, TurnEndReason};
use epsilon_contracts::content::UserContent;
use epsilon_contracts::message::{Message, UserMessage};
use epsilon_core::{AutoAllow, LoopLimits, ToolRegistry};
use epsilon_llm::{FauxProvider, FauxStep, FauxToolCall};

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

fn agent_with(steps: Vec<FauxStep>, limits: LoopLimits) -> Arc<epsilon_core::Agent> {
    epsilon_core::Agent::new(
        limits,
        Arc::new(epsilon_llm::PanicGuard(FauxProvider::new(steps))),
        Arc::new(ToolRegistry::new()),
        Arc::new(AutoAllow),
        std::env::temp_dir(),
    )
}

fn tool_results(agent: &epsilon_core::Agent) -> Vec<String> {
    agent
        .history()
        .iter()
        .filter_map(|m| match m {
            Message::ToolResult(t) => Some(
                t.content
                    .iter()
                    .filter_map(|c| match c {
                        epsilon_contracts::content::ContentBlock::Text { text } => {
                            Some(text.clone())
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
            _ => None,
        })
        .collect()
}

/// Five different commands, all failing with the same words — the shape the doom-loop brake
/// cannot see, because no two calls are identical.
#[tokio::test]
async fn different_commands_with_the_same_failure_trip_the_stuck_run_guard() {
    // Three DIFFERENT calls (different arguments), one identical failure — the doom-loop brake
    // counts call identity and would never fire here.
    let steps = vec![
        FauxStep::ToolCalls {
            text: None,
            calls: vec![call("tc_1", "fail", serde_json::json!({"n": 1}))],
        },
        FauxStep::ToolCalls {
            text: None,
            calls: vec![call("tc_2", "fail", serde_json::json!({"n": 2}))],
        },
        FauxStep::ToolCalls {
            text: None,
            calls: vec![call("tc_3", "fail", serde_json::json!({"n": 3}))],
        },
        // A fourth attempt: the warn rung has already spoken, and now the block rung asks —
        // the check runs before a call, so it can only see results that already arrived.
        FauxStep::ToolCalls {
            text: None,
            calls: vec![call("tc_4", "fail", serde_json::json!({"n": 4}))],
        },
        FauxStep::say("I could not get past that; here is what I know."),
    ];
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(epsilon_core::testing::FailTool::new()));
    let agent = epsilon_core::Agent::new(
        LoopLimits {
            no_progress_threshold: 3,
            max_turns: 12,
            ..Default::default()
        },
        Arc::new(epsilon_llm::PanicGuard(FauxProvider::new(steps))),
        Arc::new(registry),
        Arc::new(AutoAllow),
        std::env::temp_dir(),
    );
    let mut rx = agent.subscribe();
    agent.run(user("make the build pass")).await.expect("run");

    let results = tool_results(&agent);
    assert_eq!(results.len(), 4, "four attempts: {results:?}");
    assert!(
        results[2].contains("[harness]"),
        "the third identical result carries the guidance: {:?}",
        results[2]
    );
    assert!(
        results[2].contains("identical to the previous one"),
        "{:?}",
        results[2]
    );
    assert!(
        results[0].contains("intentional failure"),
        "and the result itself is still there, unedited: {:?}",
        results[0]
    );

    // The block rung asked a question the moment the streak was visible.
    let mut asked = false;
    while let Ok(ev) = rx.try_recv() {
        if let AgentEvent::PermissionRequested { request } = &*ev
            && request.capability == "no_progress"
        {
            asked = true;
            assert_eq!(
                request.detail["identical_results"], 3,
                "the request says how stuck it is: {:?}",
                request.detail
            );
        }
    }
    assert!(
        asked,
        "the block rung is a permission request, like the doom-loop brake"
    );
}

#[tokio::test]
async fn a_successful_run_never_sees_the_guard() {
    // Different results each turn: no guidance, no permission request.
    let steps = vec![
        FauxStep::ToolCalls {
            text: None,
            calls: vec![call("tc_1", "echo", serde_json::json!({"text": "first"}))],
        },
        FauxStep::ToolCalls {
            text: None,
            calls: vec![call("tc_2", "echo", serde_json::json!({"text": "second"}))],
        },
        FauxStep::say("done"),
    ];
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(epsilon_core::testing::EchoTool::new()));
    let agent = epsilon_core::Agent::new(
        LoopLimits {
            no_progress_threshold: 2,
            max_turns: 8,
            ..Default::default()
        },
        Arc::new(epsilon_llm::PanicGuard(FauxProvider::new(steps))),
        Arc::new(registry),
        Arc::new(AutoAllow),
        std::env::temp_dir(),
    );
    let mut rx = agent.subscribe();
    let outcome = agent.run(user("say two things")).await.expect("run");
    assert_eq!(outcome.reason, TurnEndReason::Completed);

    for r in tool_results(&agent) {
        assert!(!r.contains("[harness]"), "no guidance on progress: {r}");
    }
    let mut asked = false;
    while let Ok(ev) = rx.try_recv() {
        if let AgentEvent::PermissionRequested { request } = &*ev
            && request.capability == "no_progress"
        {
            asked = true;
        }
    }
    assert!(!asked, "progress must not be interrupted");
}

/// An empty final turn gets one nudge, then the run fails with a reason instead of "Completed".
#[tokio::test]
async fn a_run_that_says_nothing_is_nudged_then_failed() {
    let steps = vec![
        FauxStep::ToolCalls {
            text: None,
            calls: vec![call("tc_1", "echo", serde_json::json!({"text": "x"}))],
        },
        FauxStep::say(""), // the empty turn
        FauxStep::say(""), // and again, after the nudge
    ];
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(epsilon_core::testing::EchoTool::new()));
    let agent = epsilon_core::Agent::new(
        LoopLimits {
            empty_turn_retries: 1,
            max_turns: 8,
            ..Default::default()
        },
        Arc::new(epsilon_llm::PanicGuard(FauxProvider::new(steps))),
        Arc::new(registry),
        Arc::new(AutoAllow),
        std::env::temp_dir(),
    );
    let outcome = agent.run(user("do something")).await.expect("run");

    match &outcome.reason {
        TurnEndReason::Error(message) => assert!(
            message.contains("empty turn"),
            "the failure names the cause: {message}"
        ),
        other => panic!("an empty reply must not read as success: {other:?}"),
    }

    // The nudge is in the transcript — the model saw it, and so does anyone reading the session.
    let nudged = agent.history().iter().any(|m| match m {
        Message::User(u) => u.content.iter().any(|c| match c {
            UserContent::Text { text } => text.contains("[harness] your last reply was empty"),
            _ => false,
        }),
        _ => false,
    });
    assert!(nudged, "the retry explains itself to the model");
}

/// A reply that repeats one line instead of answering is the same class of failure.
#[tokio::test]
async fn a_degenerate_reply_is_caught_too() {
    let repeated = "let me try again\n".repeat(30);
    let steps = vec![FauxStep::say(&repeated), FauxStep::say(&repeated)];
    let agent = agent_with(
        steps,
        LoopLimits {
            empty_turn_retries: 0,
            max_turns: 6,
            ..Default::default()
        },
    );
    let outcome = agent.run(user("fix it")).await.expect("run");
    match &outcome.reason {
        TurnEndReason::Error(message) => assert!(
            message.contains("degenerate repetition"),
            "the reason names the shape of the failure: {message}"
        ),
        other => panic!("expected a repetition failure, got {other:?}"),
    }
}
