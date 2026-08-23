//! End-to-end loop scenarios against the faux provider — offline, deterministic, free.
//! This is the payoff of the lego methodology: the full pipeline exercised without network.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use epsilon_contracts::agent_event::{AgentEvent, PermissionRequest, TurnEndReason};
use epsilon_contracts::content::UserContent;
use epsilon_contracts::message::{Message, UserMessage};
use epsilon_core::testing::{EchoTool, FailTool, SlowProvider};
use epsilon_core::{AutoAllow, LoopLimits, RunError, ToolRegistry};
use epsilon_llm::FauxProvider;

fn user_msg(text: &str) -> Message {
    Message::User(UserMessage {
        id: epsilon_contracts::new_id("msg"),
        timestamp_ms: 0,
        content: vec![UserContent::Text {
            text: text.to_string(),
        }],
    })
}

fn agent_with(
    steps: Vec<epsilon_llm::FauxStep>,
    gate: Arc<dyn epsilon_core::PermissionGate>,
    limits: LoopLimits,
) -> Arc<epsilon_core::Agent> {
    let provider = FauxProvider::new(steps);
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(EchoTool::new()));
    registry.register(Arc::new(FailTool::new()));
    epsilon_core::Agent::new(
        limits,
        Arc::new(epsilon_llm::PanicGuard(provider)),
        Arc::new(registry),
        gate,
        std::env::temp_dir(),
    )
}

fn echo_call(id: &str, text: &str) -> epsilon_llm::FauxStep {
    epsilon_llm::FauxStep::call(id, "echo", serde_json::json!({ "text": text }))
}

#[tokio::test]
async fn happy_multi_tool_flow_completes() {
    let agent = agent_with(
        vec![
            echo_call("tc_1", "first"),
            echo_call("tc_2", "second"),
            epsilon_llm::FauxStep::say("all done"),
        ],
        Arc::new(AutoAllow),
        LoopLimits::default(),
    );
    let mut rx = agent.subscribe();

    let outcome = agent.run(user_msg("go")).await.unwrap();

    assert_eq!(outcome.reason, TurnEndReason::Completed);
    // transcript: prompt + a(tc1) + tr(tc1) + a(tc2) + tr(tc2) + a(done)
    assert_eq!(agent.history().len(), 6);
    assert_eq!(outcome.usage.input, 300); // 3 provider calls x 100

    // Event stream saw every phase.
    let mut starts = 0usize;
    let mut ends = 0usize;
    while let Ok(ev) = rx.try_recv() {
        match *ev {
            AgentEvent::TurnStart { .. } => starts += 1,
            AgentEvent::AgentEnd { .. } => ends += 1,
            _ => {}
        }
    }
    assert_eq!(starts, 3);
    assert_eq!(ends, 1);
}

#[tokio::test]
async fn truncated_calls_fail_and_reason_is_sticky_max_tokens() {
    let agent = agent_with(
        vec![
            epsilon_llm::FauxStep::Truncated {
                calls: vec![epsilon_llm::FauxToolCall {
                    id: "tc_a".into(),
                    name: "echo".into(),
                    arguments: serde_json::json!({"text": "will be cut"}),
                }],
            },
            // next step completes cleanly — but MaxTokens must stay sticky:
            epsilon_llm::FauxStep::say("recovered"),
        ],
        Arc::new(AutoAllow),
        LoopLimits::default(),
    );

    let outcome = agent.run(user_msg("go")).await.unwrap();

    assert_eq!(outcome.reason, TurnEndReason::MaxTokens);
    let error_results: Vec<_> = outcome
        .produced
        .iter()
        .filter_map(|m| match m {
            Message::ToolResult(t) if t.is_error => Some(t.tool_name.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        error_results,
        ["echo"],
        "truncated call must fail, not execute"
    );
}

#[tokio::test]
async fn doom_loop_brake_denies_repetition() {
    /// Denies doom_loop asks; records how many times it was consulted.
    struct DoomDeny {
        asks: Mutex<usize>,
    }
    #[async_trait]
    impl epsilon_core::PermissionGate for DoomDeny {
        async fn authorize(&self, req: PermissionRequest) -> epsilon_core::Authorization {
            use epsilon_contracts::agent_event::PermissionDecision as PD;
            if req.capability == "doom_loop" {
                *self.asks.lock().unwrap() += 1;
                return epsilon_core::Authorization {
                    decision: PD::Deny,
                    asked: true,
                };
            }
            epsilon_core::Authorization {
                decision: PD::AllowOnce,
                asked: false,
            }
        }
    }

    let gate = Arc::new(DoomDeny {
        asks: Mutex::new(0),
    });
    let limits = LoopLimits {
        doom_loop_threshold: 3,
        ..Default::default()
    };
    let agent = agent_with(
        vec![
            echo_call("tc_1", "same"),
            echo_call("tc_2", "same"),
            echo_call("tc_3", "same"), // streak hits 3 -> denied
            epsilon_llm::FauxStep::say("ok"),
        ],
        Arc::clone(&gate) as Arc<dyn epsilon_core::PermissionGate>,
        limits,
    );

    let outcome = agent.run(user_msg("loop")).await.unwrap();

    assert!(
        *gate.asks.lock().unwrap() >= 1,
        "doom-loop gate must be consulted"
    );
    let blocked = outcome.produced.iter().any(|m| matches!(
        m,
        Message::ToolResult(t) if t.is_error && t.details.get("blocked") == Some(&serde_json::json!(true))
    ));
    assert!(blocked, "third identical call must be blocked");
}

#[tokio::test]
async fn steering_prefill_drains_before_second_call() {
    let provider = FauxProvider::new(vec![
        epsilon_llm::FauxStep::say("working..."),
        epsilon_llm::FauxStep::say("final answer"),
    ]);
    let recorded = provider.recorded_requests_handle();
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(EchoTool::new()));
    let agent = epsilon_core::Agent::new(
        LoopLimits::default(),
        Arc::new(epsilon_llm::PanicGuard(provider)),
        Arc::new(registry),
        Arc::new(AutoAllow),
        std::env::temp_dir(),
    );

    // User interjects before the run starts (classic steering window).
    agent.steer(user_msg("focus on tests"));

    let outcome = agent.run(user_msg("start")).await.unwrap();
    assert_eq!(outcome.reason, TurnEndReason::Completed);

    let requests = recorded.lock().unwrap();
    // The run makes exactly one provider call; the pre-filled steering message must be in
    // ITS context (drained at the first drain point, before the first model call).
    assert_eq!(requests.len(), 1, "one provider call expected");
    let second_context_texts: Vec<String> = requests[0]
        .messages
        .iter()
        .filter_map(|m| match m {
            Message::User(u) => u.content.iter().find_map(|c| match c {
                UserContent::Text { text } => Some(text.clone()),
                _ => None,
            }),
            _ => None,
        })
        .collect();
    assert!(
        second_context_texts
            .iter()
            .any(|t| t.contains("focus on tests")),
        "steering message must reach the model context"
    );
}

#[tokio::test]
async fn unknown_tool_yields_error_result_not_crash() {
    let agent = agent_with(
        vec![
            epsilon_llm::FauxStep::call("tc_x", "does_not_exist", serde_json::json!({})),
            epsilon_llm::FauxStep::say("handled"),
        ],
        Arc::new(AutoAllow),
        LoopLimits::default(),
    );

    let outcome = agent.run(user_msg("go")).await.unwrap();
    assert!(outcome.produced.iter().any(|m| matches!(
        m,
        Message::ToolResult(t) if t.is_error && t.tool_name == "does_not_exist"
    )));
    assert_eq!(outcome.reason, TurnEndReason::Completed);
}

#[tokio::test]
async fn failing_tool_becomes_error_result() {
    let agent = agent_with(
        vec![
            epsilon_llm::FauxStep::call("tc_f", "fail", serde_json::json!({})),
            epsilon_llm::FauxStep::say("noted"),
        ],
        Arc::new(AutoAllow),
        LoopLimits::default(),
    );

    let outcome = agent.run(user_msg("go")).await.unwrap();
    assert!(outcome
        .produced
        .iter()
        .any(|m| matches!(m, Message::ToolResult(t) if t.is_error && t.content[0] == epsilon_contracts::content::ContentBlock::text("intentional failure"))));
}

#[tokio::test]
async fn concurrent_runs_are_rejected() {
    let provider = SlowProvider::new(
        FauxProvider::new(vec![
            epsilon_llm::FauxStep::Assistant {
                text: String::new()
            };
            8
        ]),
        200,
    );
    let agent = epsilon_core::Agent::new(
        LoopLimits::default(),
        Arc::new(epsilon_llm::PanicGuard(provider)),
        Arc::new(ToolRegistry::new()),
        Arc::new(AutoAllow),
        std::env::temp_dir(),
    );

    let a2 = Arc::clone(&agent);
    let first = tokio::spawn(async move { a2.run(user_msg("one")).await });

    // Give the first run a beat to start, then try to double-run.
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    let second = agent.run(user_msg("two")).await;

    first.await.unwrap().unwrap();
    assert!(matches!(second.unwrap_err(), RunError::AlreadyRunning));
}
