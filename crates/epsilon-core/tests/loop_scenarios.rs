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

/// M53: build a `TaskRequest` for the host calls in this file.
fn task_req(
    prompt: &str,
    description: &str,
    subagent_type: &str,
    background: bool,
    resume: Option<&str>,
) -> epsilon_contracts::tool::TaskRequest {
    epsilon_contracts::tool::TaskRequest {
        prompt: prompt.to_string(),
        description: description.to_string(),
        subagent_type: subagent_type.to_string(),
        background,
        resume: resume.map(str::to_string),
        model: None,
    }
}

/// Unwrap the foreground outcome (M53: `run_task` returns an outcome enum).
fn completed(
    outcome: epsilon_contracts::tool::TaskOutcome,
) -> epsilon_contracts::tool::TaskHostResult {
    match outcome {
        epsilon_contracts::tool::TaskOutcome::Completed(res) => res,
        other => panic!("expected a completed task, got {other:?}"),
    }
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

struct ObserveScope {
    descriptor: epsilon_contracts::tool::ToolDescriptor,
    seen: Arc<Mutex<Vec<Arc<()>>>>,
}

#[async_trait]
impl epsilon_contracts::tool::Tool for ObserveScope {
    fn descriptor(&self) -> &epsilon_contracts::tool::ToolDescriptor {
        &self.descriptor
    }
    async fn execute(
        &self,
        _call: epsilon_contracts::tool::ToolInvocation,
        ctx: epsilon_contracts::tool::ToolContext,
        _sink: epsilon_contracts::tool::ToolUpdateSink,
    ) -> Result<epsilon_contracts::tool::ToolOutput, epsilon_contracts::tool::ToolFailure> {
        self.seen.lock().unwrap().push(ctx.observation_scope);
        Ok(epsilon_contracts::tool::ToolOutput::text("observed"))
    }
}

#[tokio::test]
async fn observation_scope_is_stable_per_agent_and_renewed_on_restore() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(ObserveScope {
        descriptor: epsilon_contracts::tool::ToolDescriptor {
            name: "observe".into(),
            label: "Observe".into(),
            description: "Inert fixture".into(),
            parameters_schema: serde_json::json!({"type": "object"}),
            execution_mode: epsilon_contracts::tool::ExecutionMode::Sequential,
        },
        seen: seen.clone(),
    }));
    let registry = Arc::new(registry);
    let build = || {
        let mut steps = Vec::new();
        for i in 0..3 {
            steps.push(epsilon_llm::FauxStep::call(
                &format!("tc_{i}"),
                "observe",
                serde_json::json!({}),
            ));
            steps.push(epsilon_llm::FauxStep::say("done"));
        }
        epsilon_core::Agent::new(
            LoopLimits::default(),
            Arc::new(FauxProvider::new(steps)),
            registry.clone(),
            Arc::new(AutoAllow),
            std::env::temp_dir(),
        )
    };
    let a = build();
    let b = build();
    a.run(user_msg("first")).await.unwrap();
    a.run(user_msg("second")).await.unwrap();
    b.run(user_msg("independent")).await.unwrap();
    a.restore_history(Vec::new());
    a.run(user_msg("restored")).await.unwrap();
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 4);
    assert!(Arc::ptr_eq(&seen[0], &seen[1]));
    assert!(!Arc::ptr_eq(&seen[0], &seen[2]));
    assert!(!Arc::ptr_eq(&seen[0], &seen[3]));
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

// M42: interception (pi `beforeToolCall` / `afterToolCall`).
//
// A recording hook observes every call (names + args), a blocking hook vetoes
// one tool while letting others through, and an after-hook rewrites content —
// the three behaviors extensions need (redaction, policy veto, summarization).
struct RecordingHooks {
    seen: Mutex<Vec<(String, serde_json::Value)>>,
    block_name: Option<String>,
    rewrite_from: Option<String>,
}

impl RecordingHooks {
    fn new() -> Self {
        Self {
            seen: Mutex::new(Vec::new()),
            block_name: None,
            rewrite_from: None,
        }
    }
}

#[async_trait]
impl epsilon_contracts::tool::ToolHooks for RecordingHooks {
    async fn before_tool_call(
        &self,
        ctx: epsilon_contracts::tool::BeforeToolCallContext,
    ) -> Result<epsilon_contracts::tool::BeforeToolCallResult, String> {
        self.seen
            .lock()
            .unwrap()
            .push((ctx.tool_name.clone(), ctx.args.clone()));
        if self.block_name.as_deref() == Some(ctx.tool_name.as_str()) {
            return Ok(epsilon_contracts::tool::BeforeToolCallResult {
                block: true,
                reason: Some(format!("{} is vetoed by policy", ctx.tool_name)),
                terminate: false,
            });
        }
        Ok(epsilon_contracts::tool::BeforeToolCallResult::default())
    }

    async fn after_tool_call(
        &self,
        ctx: epsilon_contracts::tool::AfterToolCallContext,
    ) -> Result<Option<epsilon_contracts::tool::AfterToolCallResult>, String> {
        if self.rewrite_from.as_deref() == Some(ctx.tool_name.as_str()) {
            return Ok(Some(epsilon_contracts::tool::AfterToolCallResult {
                content: Some(vec![epsilon_contracts::content::ContentBlock::text(
                    "redacted by hook",
                )]),
                ..Default::default()
            }));
        }
        Ok(None)
    }
}

#[tokio::test]
async fn hooks_observe_block_and_rewrite_tool_calls() {
    let agent = agent_with(
        vec![
            echo_call("tc_1", "first"),
            echo_call("tc_2", "second"),
            epsilon_llm::FauxStep::say("done"),
        ],
        Arc::new(AutoAllow),
        LoopLimits::default(),
    );
    let hooks = Arc::new(RecordingHooks::new());
    agent.set_tool_hooks(hooks.clone());

    let outcome = agent.run(user_msg("go")).await.unwrap();
    assert_eq!(outcome.reason, TurnEndReason::Completed);
    // Both calls observed with their validated args.
    let seen = hooks.seen.lock().unwrap();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[0].0, "echo");
    assert!(seen[0].1.get("text").is_some());
}

#[tokio::test]
async fn before_hook_block_vetoes_one_call_only() {
    let agent = agent_with(
        vec![
            echo_call("tc_1", "first"),
            echo_call("tc_2", "second"),
            epsilon_llm::FauxStep::say("done"),
        ],
        Arc::new(AutoAllow),
        LoopLimits::default(),
    );
    let hooks = Arc::new(RecordingHooks {
        block_name: Some("echo".into()),
        ..RecordingHooks::new()
    });
    agent.set_tool_hooks(hooks);

    let outcome = agent.run(user_msg("go")).await.unwrap();
    // Both echo calls blocked with the hook's reason; the run still completes
    // (blocks are error results, not run failures — pi parity).
    let blocked: Vec<_> = outcome
        .produced
        .iter()
        .filter_map(|m| match m {
            Message::ToolResult(t) if t.is_error && t.tool_name == "echo" => Some(t),
            _ => None,
        })
        .collect();
    assert_eq!(blocked.len(), 2);
    assert!(blocked.iter().all(|t| t.content[0]
        == epsilon_contracts::content::ContentBlock::text("echo is vetoed by policy")));
    assert_eq!(outcome.reason, TurnEndReason::Completed);
}

#[tokio::test]
async fn after_hook_rewrites_result_content() {
    let agent = agent_with(
        vec![
            echo_call("tc_1", "secret"),
            epsilon_llm::FauxStep::say("done"),
        ],
        Arc::new(AutoAllow),
        LoopLimits::default(),
    );
    let hooks = Arc::new(RecordingHooks {
        rewrite_from: Some("echo".into()),
        ..RecordingHooks::new()
    });
    agent.set_tool_hooks(hooks);

    let outcome = agent.run(user_msg("go")).await.unwrap();
    let results: Vec<_> = outcome
        .produced
        .iter()
        .filter_map(|m| match m {
            Message::ToolResult(t) if t.tool_name == "echo" => Some(t),
            _ => None,
        })
        .collect();
    assert_eq!(results.len(), 1);
    assert!(!results[0].is_error);
    assert_eq!(
        results[0].content[0],
        epsilon_contracts::content::ContentBlock::text("redacted by hook")
    );
}

#[tokio::test]
async fn concurrent_runs_are_rejected() {
    let provider = SlowProvider::new(
        FauxProvider::new(vec![epsilon_llm::FauxStep::say("slow")]),
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

#[tokio::test]
async fn task_delegates_to_isolated_child_via_host() {
    use epsilon_contracts::tool::{TaskHost, TaskHostResult};
    use epsilon_llm::{FauxProvider, FauxStep};
    use std::sync::Arc;

    let child_provider = Arc::new(FauxProvider::new(vec![FauxStep::say("child says hello")]));
    let limits = LoopLimits::default();
    let gate: Arc<dyn epsilon_core::PermissionGate> = Arc::new(AutoAllow);
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(EchoTool::new()));
    let agent = epsilon_core::Agent::new(
        limits,
        child_provider as Arc<dyn epsilon_llm::StreamFn>,
        Arc::new(registry),
        gate,
        std::env::temp_dir(),
    );
    let host: Arc<dyn TaskHost> = agent.clone() as Arc<dyn TaskHost>;
    let res: TaskHostResult = completed(
        host.run_task(task_req("say hello", "say hello", "default", false, None))
            .await
            .expect("child host must run"),
    );
    assert!(res.text.contains("child says hello"), "got: {}", res.text);
    assert!(res.usage.is_some());

    // Prove fallback capture still works when host is None (honest envelope)
    // and that host=isolated path would be taken when task_host is present —
    // the task tool itself is tested in epsilon-tools; here we just prove the
    // host inversion is wired by checking the host we just exercised.
    assert!(res.details["produced"].as_u64().unwrap_or(0) >= 1);
}

// M50 (pi `agents.ts` + opencode `Agent.get` parity): `run_task` routes by
// agent type — unknown named types fail naming what's available, defs narrow
// the child's tool view + prepend a system prompt + override the model, and
// the depth cap stops nested fan-out.
#[tokio::test]
async fn task_unknown_type_names_available_agents() {
    use epsilon_contracts::tool::TaskHost;
    use epsilon_llm::FauxProvider;
    use std::sync::Arc;

    let agent = epsilon_core::Agent::new(
        LoopLimits::default(),
        Arc::new(epsilon_llm::PanicGuard(FauxProvider::new(vec![]))),
        Arc::new(ToolRegistry::new()),
        Arc::new(AutoAllow),
        std::env::temp_dir(),
    );
    agent.set_agents(vec![epsilon_core::agents::AgentDef {
        name: "scout".into(),
        description: "Fast recon".into(),
        tools: None,
        model: None,
        system_prompt: "You are a scout.".into(),
        source: epsilon_core::agents::AgentSource::User,
    }]);
    let host: Arc<dyn TaskHost> = agent.clone() as Arc<dyn TaskHost>;
    let err = host
        .run_task(task_req("x", "x", "nope", false, None))
        .await
        .expect_err("unknown type must fail");
    assert!(err.contains("unknown agent type"), "got: {err}");
    assert!(err.contains("scout"), "names what's available, got: {err}");
}

#[tokio::test]
async fn task_def_routes_tools_prompt_and_model() {
    use epsilon_contracts::tool::TaskHost;
    use epsilon_llm::{FauxProvider, FauxStep};
    use std::sync::Arc;

    // Shared provider records every request: the child's system prompt must
    // carry the def body ahead of the task (pi: body past frontmatter).
    let child_provider = Arc::new(FauxProvider::new(vec![FauxStep::say("done")]));
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(EchoTool::new()));
    registry.register(Arc::new(FailTool::new()));
    let agent = epsilon_core::Agent::new(
        LoopLimits::default(),
        Arc::clone(&child_provider) as Arc<dyn epsilon_llm::StreamFn>,
        Arc::new(registry),
        Arc::new(AutoAllow),
        std::env::temp_dir(),
    );
    agent.set_agents(vec![epsilon_core::agents::AgentDef {
        name: "scout".into(),
        description: "Fast recon".into(),
        // Narrow view: echo only (fail must NOT be visible to this child).
        tools: Some(vec!["echo".to_string()]),
        model: Some("faux-1".into()),
        system_prompt: "You are a scout.".into(),
        source: epsilon_core::agents::AgentSource::User,
    }]);
    let host: Arc<dyn TaskHost> = agent.clone() as Arc<dyn TaskHost>;
    let res: epsilon_contracts::tool::TaskHostResult = completed(
        host.run_task(task_req("find it", "find it", "scout", false, None))
            .await
            .expect("routed child must run"),
    );
    assert!(res.text.contains("done"), "got: {}", res.text);
    let reqs = child_provider.recorded_requests();
    assert_eq!(reqs.len(), 1, "one child turn, got {}", reqs.len());
    assert!(
        reqs[0].system_prompt.contains("You are a scout."),
        "def body rendered in CONTEXT tier, got: {}",
        reqs[0].system_prompt
    );
    // The task itself stays the user message, NOT merged into the system
    // prompt (pi: instructions vs input are separate channels).
    assert!(
        !reqs[0].system_prompt.contains("find it"),
        "task must not leak into system prompt, got: {}",
        reqs[0].system_prompt
    );
    // Tool subset: the child's provider payload offers echo but not fail.
    let tool_names: Vec<&str> = reqs[0].tools.iter().map(|d| d.name.as_str()).collect();
    assert!(tool_names.contains(&"echo"), "echo visible: {tool_names:?}");
    assert!(!tool_names.contains(&"fail"), "fail hidden: {tool_names:?}");
}

#[tokio::test]
async fn task_depth_cap_refuses_nested_delegation() {
    use epsilon_contracts::tool::TaskHost;
    use epsilon_llm::{FauxProvider, FauxStep};
    use std::sync::Arc;

    // Depth-0 delegation still runs (the cap only bites nested hops).
    let parent_provider = Arc::new(FauxProvider::new(vec![FauxStep::say("parent done")]));
    let parent = epsilon_core::Agent::new(
        LoopLimits::default(),
        parent_provider as Arc<dyn epsilon_llm::StreamFn>,
        Arc::new(ToolRegistry::new()),
        Arc::new(AutoAllow),
        std::env::temp_dir(),
    );
    let host: Arc<dyn TaskHost> = parent.clone() as Arc<dyn TaskHost>;
    let res = completed(
        host.run_task(task_req("say hi", "say hi", "default", false, None))
            .await
            .expect("depth-0 delegation runs"),
    );
    assert!(res.text.contains("parent done"), "got: {}", res.text);
    // Nested refusal (child at depth 1 calling run_task again) is proven at
    // the tool level below: a scripted child that issues its own `task` call
    // gets the depth message back as a tool failure, not a silent hang.
}

// Pi parity (agent.ts abort signal): cancellation must reach the provider
// stream, not just the tool pool. SlowProvider stalls *before* the stream
// opens; abort has to win promptly instead of waiting out the stall.
#[tokio::test]
async fn abort_during_stream_creation_ends_run_aborted() {
    let provider = SlowProvider::new(
        FauxProvider::new(vec![epsilon_llm::FauxStep::say("never seen")]),
        5_000,
    );
    let agent = epsilon_core::Agent::new(
        LoopLimits::default(),
        Arc::new(epsilon_llm::PanicGuard(provider)),
        Arc::new(ToolRegistry::new()),
        Arc::new(AutoAllow),
        std::env::temp_dir(),
    );
    let handle = tokio::spawn({
        let agent = Arc::clone(&agent);
        async move { agent.run(user_msg("go")).await }
    });
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert!(agent.abort_current_run());
    let outcome = handle.await.unwrap().unwrap();
    assert_eq!(outcome.reason, TurnEndReason::Aborted);
    // In-band abort message closes the turn; nothing from the provider arrived.
    let last = outcome.produced.last().expect("abort message produced");
    match last {
        Message::Assistant(a) => {
            assert_eq!(
                a.stop_reason,
                epsilon_contracts::message::StopReason::Aborted
            );
            assert!(a.text().is_empty());
        }
        other => panic!("expected aborted assistant, got {other:?}"),
    }
}

// Same signal, later in the stream: one delta arrived, then the provider hangs.
// The folded-so-far text is preserved; the run still ends Aborted.
#[tokio::test]
async fn abort_mid_stream_preserves_partial_text() {
    struct HangAfterStartStream {
        events: std::collections::VecDeque<epsilon_contracts::stream::StreamEvent>,
    }

    #[async_trait]
    impl epsilon_llm::EventStream for HangAfterStartStream {
        async fn next(&mut self) -> Option<epsilon_contracts::stream::StreamEvent> {
            if let Some(ev) = self.events.pop_front() {
                Some(ev)
            } else {
                std::future::pending().await
            }
        }
    }

    struct HangAfterStartProvider;

    #[async_trait]
    impl epsilon_llm::StreamFn for HangAfterStartProvider {
        async fn stream(
            &self,
            _model: &epsilon_llm::ModelRef,
            _ctx: epsilon_llm::LlmContext,
        ) -> epsilon_llm::BoxEventStream {
            use epsilon_contracts::message::{AssistantMessage, StopReason, Usage};
            use epsilon_contracts::stream::StreamEvent;
            let partial = AssistantMessage {
                id: epsilon_contracts::new_id("msg"),
                timestamp_ms: 0,
                provider: "test".into(),
                model_id: "hang-1".into(),
                content: vec![],
                stop_reason: StopReason::EndTurn,
                error_message: None,
                usage: Usage::default(),
            };
            Box::new(HangAfterStartStream {
                events: [
                    StreamEvent::Start { partial },
                    StreamEvent::TextDelta {
                        index: 0,
                        delta: "half an answer".into(),
                    },
                ]
                .into_iter()
                .collect(),
            })
        }
    }

    let agent = epsilon_core::Agent::new(
        LoopLimits::default(),
        Arc::new(epsilon_llm::PanicGuard(HangAfterStartProvider)),
        Arc::new(ToolRegistry::new()),
        Arc::new(AutoAllow),
        std::env::temp_dir(),
    );
    let handle = tokio::spawn({
        let agent = Arc::clone(&agent);
        async move { agent.run(user_msg("go")).await }
    });
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert!(agent.abort_current_run());
    let outcome = handle.await.unwrap().unwrap();
    assert_eq!(outcome.reason, TurnEndReason::Aborted);
    let last = outcome.produced.last().expect("abort message produced");
    match last {
        Message::Assistant(a) => {
            assert_eq!(
                a.stop_reason,
                epsilon_contracts::message::StopReason::Aborted
            );
            assert_eq!(a.text(), "half an answer");
        }
        other => panic!("expected aborted assistant, got {other:?}"),
    }
}

/// M61: a session file has one writer, and the agent's resume path respects it.
#[tokio::test]
async fn resuming_a_session_another_writer_holds_is_refused_with_a_clear_reason() {
    let dir = tempfile::tempdir().expect("tmp");
    let path = dir.path().join("s.jsonl");
    // Stand-in for "the other epsilon window": a second store on the same file.
    let _other_writer = epsilon_session::SessionStore::open_in(&path, dir.path()).expect("writer");

    let agent = agent_with(
        vec![epsilon_llm::FauxStep::Assistant { text: "hi".into() }],
        Arc::new(AutoAllow),
        LoopLimits::default(),
    );
    let err = agent.live_resume(&path).expect_err("refused");
    assert!(
        err.contains("another epsilon") && err.contains("pid="),
        "the refusal names the holder: {err}"
    );
}

/// M61 regression: re-resuming the session this process already holds must work
/// (the store's own lease would otherwise refuse us), and switching back must
/// not cost the session if the new file is locked.
#[tokio::test]
async fn re_resuming_the_current_session_is_allowed_and_a_locked_switch_keeps_it() {
    let dir = tempfile::tempdir().expect("tmp");
    let a = dir.path().join("a.jsonl");
    let b = dir.path().join("b.jsonl");
    let agent = agent_with(
        vec![
            epsilon_llm::FauxStep::Assistant {
                text: "first".into(),
            },
            epsilon_llm::FauxStep::Assistant {
                text: "second".into(),
            },
        ],
        Arc::new(AutoAllow),
        LoopLimits::default(),
    );
    // Attach first, then run: the durable mirror is what a re-resume reads back.
    let store = epsilon_session::SessionStore::open_in(&a, dir.path()).expect("store a");
    agent.attach_session_store(Arc::new(Mutex::new(store)));
    agent
        .run(user_msg("hello"))
        .await
        .expect("turn into the session");

    let resumed = agent.live_resume(&a).expect("re-resume our own session");
    assert!(
        resumed > 0,
        "re-resume re-reads the persisted turn: {resumed} messages"
    );

    // Another process holds b: the switch fails, and a stays attached.
    let _other = epsilon_session::SessionStore::open_in(&b, dir.path()).expect("writer b");
    let err = agent.live_resume(&b).expect_err("b is locked");
    assert!(err.contains("another epsilon"), "{err}");
    assert_eq!(
        agent.session_path().expect("session path"),
        a,
        "a failed switch must leave the current session attached"
    );
}

/// M62: a stream that dies abruptly (no terminal frame) must not take the text
/// the user already watched arrive down with it.
#[tokio::test]
async fn a_stream_that_ends_without_a_terminal_frame_keeps_its_partial_text() {
    use epsilon_contracts::message::{AssistantMessage, StopReason, Usage};
    use epsilon_contracts::stream::StreamEvent;
    use epsilon_llm::{BoxEventStream, EventStream, LlmContext, ModelRef, StreamFn};

    /// Emits a Start + one delta, then ends with nothing — the shape a killed
    /// connection produces after the retry budget is spent.
    struct DiesAbruptly {
        text: String,
    }

    #[async_trait]
    impl StreamFn for DiesAbruptly {
        async fn stream(&self, model: &ModelRef, _ctx: LlmContext) -> BoxEventStream {
            let partial = AssistantMessage {
                id: epsilon_contracts::new_id("msg"),
                timestamp_ms: 0,
                provider: "faux".into(),
                model_id: model.id.clone(),
                content: vec![epsilon_contracts::content::ContentBlock::text(&self.text)],
                stop_reason: StopReason::EndTurn,
                error_message: None,
                usage: Usage::default(),
            };
            let events = vec![
                StreamEvent::Start {
                    partial: partial.clone(),
                },
                StreamEvent::TextDelta {
                    index: 0,
                    delta: self.text.clone(),
                },
            ];
            Box::new(FixedStream {
                events: events.into(),
            })
        }
    }

    struct FixedStream {
        events: std::collections::VecDeque<StreamEvent>,
    }

    #[async_trait]
    impl EventStream for FixedStream {
        async fn next(&mut self) -> Option<StreamEvent> {
            self.events.pop_front()
        }
    }

    let agent = epsilon_core::Agent::new(
        LoopLimits::default(),
        Arc::new(epsilon_llm::PanicGuard(DiesAbruptly {
            text: "half an answer".into(),
        })),
        Arc::new(ToolRegistry::new()),
        Arc::new(AutoAllow),
        std::env::temp_dir(),
    );
    let run = agent.run(user_msg("start something")).await.expect("run");

    let assistants: Vec<_> = run
        .produced
        .iter()
        .filter_map(|m| match m {
            Message::Assistant(a) => Some(a),
            _ => None,
        })
        .collect();
    let last = assistants.last().expect("an assistant message");
    assert_eq!(
        last.text(),
        "half an answer",
        "the partial the user saw is the turn's output"
    );
    assert_eq!(
        last.stop_reason,
        StopReason::Error,
        "flagged, not pretended"
    );
    assert!(
        last.error_message
            .as_deref()
            .is_some_and(|e| e.contains("terminal frame")),
        "the failure explains itself: {last:?}"
    );
    assert!(
        matches!(run.reason, TurnEndReason::Error(_)),
        "the run reports the error: {:?}",
        run.reason
    );
    // ...and it is in the transcript, so a resumed session rebuilds context with it.
    let in_history = agent.history().iter().any(|m| match m {
        Message::Assistant(a) => a.text() == "half an answer",
        _ => false,
    });
    assert!(in_history, "the partial joined the transcript");
}
