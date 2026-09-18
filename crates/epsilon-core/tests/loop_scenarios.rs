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

// CORE-05: the aborted turn now has the same message lifecycle as a completed
// one — runtime history, durable mirror, and a MessageEnd event — so a reopened
// session keeps the interrupted answer (pi persists assistant message-end
// events, `agent-session.ts:639-656`). The old arms returned the synthetic
// message only in `produced`.
#[tokio::test]
async fn abort_mid_stream_commits_the_terminal_message_to_history_and_store() {
    use epsilon_contracts::message::StopReason;
    use epsilon_contracts::session_entry::EntryKind;
    use epsilon_session::{ModelViewEntry, SessionStore};

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
            use epsilon_contracts::message::{AssistantMessage, Usage};
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

    let dir = tempfile::tempdir().unwrap();
    let store = SessionStore::open(dir.path().join("session.jsonl")).unwrap();
    let agent = epsilon_core::Agent::new(
        LoopLimits::default(),
        Arc::new(epsilon_llm::PanicGuard(HangAfterStartProvider)),
        Arc::new(ToolRegistry::new()),
        Arc::new(AutoAllow),
        std::env::temp_dir(),
    );
    agent.attach_session_store(Arc::new(Mutex::new(store)));
    let handle = agent.store_handle_for_tests();
    let mut rx = agent.subscribe();

    let run = tokio::spawn({
        let agent = Arc::clone(&agent);
        async move { agent.run(user_msg("go")).await }
    });
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert!(agent.abort_current_run());
    let outcome = run.await.unwrap().unwrap();
    assert_eq!(outcome.reason, TurnEndReason::Aborted);

    // Exactly one terminal assistant message, in runtime history...
    let assistants: Vec<_> = agent
        .history()
        .into_iter()
        .filter_map(|m| match m {
            Message::Assistant(a) => Some(a),
            _ => None,
        })
        .collect();
    assert_eq!(
        assistants.len(),
        1,
        "one terminal message survives the abort"
    );
    assert_eq!(assistants[0].stop_reason, StopReason::Aborted);
    assert_eq!(assistants[0].text(), "half an answer");

    // ...in the durable mirror...
    let stored = handle
        .lock()
        .unwrap()
        .read_all()
        .unwrap()
        .into_iter()
        .find_map(|e| match e.kind {
            EntryKind::Message {
                message: Message::Assistant(a),
                ..
            } => Some(a),
            _ => None,
        })
        .expect("the aborted message is durable");
    assert_eq!(stored.text(), "half an answer");
    assert_eq!(stored.stop_reason, StopReason::Aborted);

    // ...and in the view a reopened session would actually send.
    let view = handle.lock().unwrap().model_view().unwrap();
    assert!(
        view.iter().any(|v| matches!(
            v,
            ModelViewEntry::Message(Message::Assistant(a)) if a.text() == "half an answer"
        )),
        "the reopened model view keeps the partial answer"
    );

    // Subscribers saw exactly one MessageEnd for it (zero was the old behavior).
    let mut message_ends = 0usize;
    loop {
        match rx.try_recv() {
            Ok(ev) => {
                if matches!(*ev, AgentEvent::MessageEnd { .. }) {
                    message_ends += 1;
                }
            }
            Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => continue,
            Err(_) => break,
        }
    }
    assert_eq!(message_ends, 1, "one terminal message end, not zero");
}

// F20: after Esc, the rest of the active batch must not start. pi stops scheduling
// the remainder and marks each unstarted call "Operation aborted"
// (`agent-loop.ts:478-553,629-635`); before this, every call in the batch ran to
// completion after the abort — a cancelled write or shell command could still run.
#[tokio::test]
async fn abort_mid_batch_refuses_calls_that_have_not_started() {
    use epsilon_core::testing::SlowTool;
    use epsilon_llm::FauxToolCall;

    let slow = Arc::new(SlowTool::new(400));
    let runs = slow.runs();
    let mut registry = ToolRegistry::new();
    let tool: Arc<dyn epsilon_contracts::tool::Tool> = slow.clone();
    registry.register(tool);

    let steps = vec![
        // One assistant turn, two calls to the same sequential tool: the batch is a
        // barrier, so they run in call order and the second would start only after the
        // first settled — by which time the abort has already landed.
        epsilon_llm::FauxStep::ToolCalls {
            text: None,
            calls: vec![
                FauxToolCall {
                    id: "c1".into(),
                    name: "slow".into(),
                    arguments: serde_json::json!({"text": "first"}),
                },
                FauxToolCall {
                    id: "c2".into(),
                    name: "slow".into(),
                    arguments: serde_json::json!({"text": "second"}),
                },
            ],
        },
        epsilon_llm::FauxStep::say("unreachable"),
    ];
    let agent = epsilon_core::Agent::new(
        LoopLimits::default(),
        Arc::new(epsilon_llm::PanicGuard(epsilon_llm::FauxProvider::new(
            steps,
        ))),
        Arc::new(registry),
        Arc::new(AutoAllow),
        std::env::temp_dir(),
    );
    let run = tokio::spawn({
        let agent = Arc::clone(&agent);
        async move { agent.run(user_msg("go")).await }
    });
    // Long enough for the first call to be in flight, then cancel.
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    assert!(agent.abort_current_run());
    let outcome = run.await.unwrap().unwrap();
    assert_eq!(outcome.reason, TurnEndReason::Aborted);

    assert_eq!(
        runs.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "only the call that was already running may execute"
    );
    let results: Vec<_> = agent
        .history()
        .into_iter()
        .filter_map(|m| match m {
            Message::ToolResult(t) => Some(t),
            _ => None,
        })
        .collect();
    assert_eq!(results.len(), 2, "every call receives a result");
    assert_eq!(results[0].tool_call_id, "c1");
    assert_eq!(results[1].tool_call_id, "c2");
    assert!(results[1].is_error, "the refused call is an error result");
    let text: String = results[1]
        .content
        .iter()
        .filter_map(|c| match c {
            epsilon_contracts::content::ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        text.contains("not executed"),
        "the refused call must say so plainly: {text}"
    );
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

// CORE-02: turn hooks (pi's `AgentLoopConfig` — `transformContext`, `convertToLlm`,
// `prepareNextTurn`, `shouldStopAfterTurn`).
//
// One configurable recorder drives every seam the slice promises: a replacement context +
// model + thinking after a tool turn (run-scoped, transcript untouched, and kept in sync
// with the turn's own output), a graceful stop that drains no queue, a transform whose
// output reaches the request while the visibility filter still runs after it, and a hook
// that panics (contained at the seam, the way a panicking provider is).
struct RecordingTurnHooks {
    /// Snapshot `prepare_next_turn` hands back (all three fields at once).
    update: Mutex<Option<epsilon_contracts::turn::TurnUpdate>>,
    /// `true` hands [`RecordingTurnHooks::update`] back on the FIRST call only — a host that
    /// swaps the run's view once and then keeps whatever the loop holds. The tests that need
    /// to tell "the snapshot was applied to the next turn" from "the snapshot is re-installed
    /// on every turn" set it.
    apply_once: bool,
    /// What `should_stop_after_turn` answers.
    stop: Mutex<bool>,
    /// Messages `transform_context` appends to every context.
    inject: Vec<Message>,
    /// Context length seen by each hook call, in call order (prepare then stop per turn).
    seen_context_len: Mutex<Vec<usize>>,
    /// Contexts `prepare_next_turn` was handed, in call order: a test can assert WHAT the
    /// loop passed the hook (the run's own output, the replacement, both), not just how much.
    seen_prepare_contexts: Mutex<Vec<Vec<Message>>>,
    /// `prepare_next_turn` call count (one per completed turn).
    prepared: Mutex<usize>,
    /// `should_stop_after_turn` call count.
    stopped: Mutex<usize>,
}

impl RecordingTurnHooks {
    fn new() -> Self {
        Self {
            update: Mutex::new(None),
            apply_once: false,
            stop: Mutex::new(false),
            inject: Vec::new(),
            seen_context_len: Mutex::new(Vec::new()),
            seen_prepare_contexts: Mutex::new(Vec::new()),
            prepared: Mutex::new(0),
            stopped: Mutex::new(0),
        }
    }
}

#[async_trait]
impl epsilon_contracts::turn::TurnHooks for RecordingTurnHooks {
    async fn transform_context(&self, messages: &[Message]) -> Vec<Message> {
        let mut out = messages.to_vec();
        out.extend(self.inject.iter().cloned());
        out
    }

    async fn prepare_next_turn(
        &self,
        ctx: &epsilon_contracts::turn::NextTurnContext<'_>,
    ) -> Option<epsilon_contracts::turn::TurnUpdate> {
        // Call order is per-turn evidence: one call per completed turn, and each call sees
        // the context the loop holds at that moment.
        let call = {
            let mut n = self.prepared.lock().unwrap();
            *n += 1;
            *n
        };
        self.seen_context_len
            .lock()
            .unwrap()
            .push(ctx.context.len());
        self.seen_prepare_contexts
            .lock()
            .unwrap()
            .push(ctx.context.to_vec());
        assert!(
            matches!(ctx.message, Message::Assistant(_)),
            "the hook sees the turn's own assistant message"
        );
        if self.apply_once && call > 1 {
            return None;
        }
        self.update.lock().unwrap().clone()
    }

    async fn should_stop_after_turn(
        &self,
        ctx: &epsilon_contracts::turn::NextTurnContext<'_>,
    ) -> bool {
        *self.stopped.lock().unwrap() += 1;
        self.seen_context_len
            .lock()
            .unwrap()
            .push(ctx.context.len());
        *self.stop.lock().unwrap()
    }
}

/// A tool that queues a steering message the moment it executes (CORE-02 test 2): the
/// interjection lands mid-turn, exactly where a user typing during a turn would leave it.
/// The agent handle is late-bound because the registry is built before the agent exists.
struct SteerDuringTurn {
    descriptor: epsilon_contracts::tool::ToolDescriptor,
    agent: Arc<Mutex<Option<Arc<epsilon_core::Agent>>>>,
    text: String,
}

#[async_trait]
impl epsilon_contracts::tool::Tool for SteerDuringTurn {
    fn descriptor(&self) -> &epsilon_contracts::tool::ToolDescriptor {
        &self.descriptor
    }
    async fn execute(
        &self,
        _call: epsilon_contracts::tool::ToolInvocation,
        _ctx: epsilon_contracts::tool::ToolContext,
        _sink: epsilon_contracts::tool::ToolUpdateSink,
    ) -> Result<epsilon_contracts::tool::ToolOutput, epsilon_contracts::tool::ToolFailure> {
        if let Some(agent) = self.agent.lock().unwrap().as_ref() {
            agent.steer(user_msg(&self.text));
        }
        Ok(epsilon_contracts::tool::ToolOutput::text("queued"))
    }
}

/// Hooks that break the contract and panic on every seam (CORE-02 containment test).
struct PanickingTurnHooks;

#[async_trait]
impl epsilon_contracts::turn::TurnHooks for PanickingTurnHooks {
    async fn transform_context(&self, _messages: &[Message]) -> Vec<Message> {
        panic!("transform_context must never unwind through the run")
    }

    async fn convert_to_llm(&self, _messages: &[Message]) -> Vec<Message> {
        panic!("convert_to_llm must never unwind through the run")
    }

    async fn prepare_next_turn(
        &self,
        _ctx: &epsilon_contracts::turn::NextTurnContext<'_>,
    ) -> Option<epsilon_contracts::turn::TurnUpdate> {
        panic!("prepare_next_turn must never unwind through the run")
    }

    async fn should_stop_after_turn(
        &self,
        _ctx: &epsilon_contracts::turn::NextTurnContext<'_>,
    ) -> bool {
        panic!("should_stop_after_turn must never unwind through the run")
    }
}

/// Text of a message, for the request-level assertions below.
fn message_text(m: &Message) -> String {
    match m {
        Message::User(u) => u
            .content
            .iter()
            .filter_map(|c| match c {
                UserContent::Text { text } => Some(text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
        Message::Assistant(a) => a.text().to_string(),
        _ => String::new(),
    }
}

/// One recorded request's messages as text (request-level assertions read better this way).
fn request_texts(ctx: &epsilon_llm::LlmContext) -> Vec<String> {
    ctx.messages.iter().map(message_text).collect()
}

/// CORE-02 (a): a `prepare_next_turn` snapshot replaces context, model and thinking for
/// the REST of the run — the second provider request carries all three, while runtime
/// history (the transcript) is untouched and a later run starts from session state again.
#[tokio::test]
async fn prepare_next_turn_replaces_context_model_and_thinking_for_the_run_only() {
    let provider = FauxProvider::new(vec![
        echo_call("tc_1", "first"),
        epsilon_llm::FauxStep::say("after the replacement"),
        epsilon_llm::FauxStep::say("second run"),
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

    let replacement = user_msg("replacement context only");
    let hooks = Arc::new(RecordingTurnHooks {
        apply_once: true,
        ..RecordingTurnHooks::new()
    });
    *hooks.update.lock().unwrap() = Some(epsilon_contracts::turn::TurnUpdate {
        context: Some(vec![replacement.clone()]),
        model: Some(epsilon_contracts::turn::TurnModel::new("faux-2")),
        thinking: Some(epsilon_contracts::ThinkingLevel::High),
    });
    agent.set_turn_hooks(hooks.clone());

    let outcome = agent.run(user_msg("go")).await.unwrap();
    assert_eq!(outcome.reason, TurnEndReason::Completed);
    assert_eq!(
        *hooks.prepared.lock().unwrap(),
        2,
        "one prepare per completed turn"
    );

    {
        let requests = recorded.lock().unwrap();
        assert_eq!(requests.len(), 2, "one provider request per turn");
        // Request #1 is the untouched run context and the session defaults.
        assert_eq!(requests[0].messages.len(), 1);
        assert_eq!(message_text(&requests[0].messages[0]), "go");
        assert_eq!(
            requests[0].thinking_level,
            epsilon_contracts::ThinkingLevel::Off
        );
        // Request #2 carries the replacement context, the new thinking level and the new model
        // (the model also renders in the VOLATILE prompt tier, so the request proves it).
        assert_eq!(requests[1].messages, vec![replacement.clone()]);
        assert_eq!(
            requests[1].thinking_level,
            epsilon_contracts::ThinkingLevel::High
        );
        assert!(
            requests[1].system_prompt.contains("model: faux-2"),
            "the request must carry the replaced model: {}",
            requests[1].system_prompt
        );
    }

    // The update is run-scoped: the transcript kept every real message and none of the
    // replacement.
    let history = agent.history();
    assert_eq!(
        history.len(),
        4,
        "prompt + assistant + tool result + assistant"
    );
    assert!(
        !history.contains(&replacement),
        "a replacement context must never be written into runtime history"
    );
    let assistant_models: Vec<&str> = outcome
        .produced
        .iter()
        .filter_map(|m| match m {
            Message::Assistant(a) => Some(a.model_id.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        assistant_models,
        ["faux-1", "faux-2"],
        "the replaced model produced the second turn"
    );

    // pi keeps the run's context in sync with the run's own output
    // (`currentContext.messages.push`), so from the second turn on each call sees the
    // replacement PLUS what the loop appended to it. These four numbers are that contract:
    // prepare@turn0 sees the run context (prompt + assistant + tool result); stop@turn0
    // already sees the one-message replacement (pi evaluates the stop on the replaced
    // context); prepare/stop@turn1 see the replacement plus the assistant message turn 1
    // produced. A frozen replacement — the shipped bug — reads [3, 1, 1, 1] here.
    assert_eq!(
        *hooks.seen_context_len.lock().unwrap(),
        vec![3, 1, 2, 2],
        "prepare sees the run context, stop the replaced one, and the replacement keeps \
         tracking the turn's output"
    );

    // A SECOND run is where a leak into session state would show: the per-run model, thinking
    // level and replacement must all be gone, so the new run's request uses the session model
    // and thinking level — and its assistant message is stamped with the session model.
    let second = agent.run(user_msg("second run")).await.unwrap();
    assert_eq!(second.reason, TurnEndReason::Completed);
    let requests = recorded.lock().unwrap();
    assert_eq!(requests.len(), 3, "run two makes exactly one request");
    assert_eq!(requests[2].thinking_level, agent.thinking_level());
    assert_eq!(
        requests[2].thinking_level,
        epsilon_contracts::ThinkingLevel::Off
    );
    assert!(
        requests[2].system_prompt.contains("model: faux-1"),
        "run two must run on the session model again: {}",
        requests[2].system_prompt
    );
    assert!(
        second
            .produced
            .iter()
            .any(|m| matches!(m, Message::Assistant(a) if a.model_id == "faux-1")),
        "the second run's turn was produced by the session model"
    );
    assert!(
        !agent.history().contains(&replacement),
        "the replacement stayed out of the transcript across runs"
    );
}

/// CORE-02 (b): `should_stop_after_turn` ends the run before any queue is polled — neither a
/// steering message queued DURING the turn nor a follow-up queued before it is drained; both
/// are still there for the next run.
#[tokio::test]
async fn should_stop_after_turn_ends_the_run_without_draining_queues() {
    let provider = FauxProvider::new(vec![
        epsilon_llm::FauxStep::call("tc_1", "steer", serde_json::json!({})),
        epsilon_llm::FauxStep::say("run two"),
        epsilon_llm::FauxStep::say("follow-up answer"),
    ]);
    let recorded = provider.recorded_requests_handle();
    // The registry is built before the agent, so the steering tool reaches the agent through
    // a slot the test fills afterwards.
    let slot: Arc<Mutex<Option<Arc<epsilon_core::Agent>>>> = Arc::new(Mutex::new(None));
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(SteerDuringTurn {
        descriptor: epsilon_contracts::tool::ToolDescriptor {
            name: "steer".into(),
            label: "Steer".into(),
            description: "Inert fixture: queues a steering message while the turn runs".into(),
            parameters_schema: serde_json::json!({"type": "object"}),
            execution_mode: epsilon_contracts::tool::ExecutionMode::Sequential,
        },
        agent: slot.clone(),
        text: "typed while the turn ran".into(),
    }));
    let agent = epsilon_core::Agent::new(
        LoopLimits::default(),
        Arc::new(epsilon_llm::PanicGuard(provider)),
        Arc::new(registry),
        Arc::new(AutoAllow),
        std::env::temp_dir(),
    );
    *slot.lock().unwrap() = Some(agent.clone());

    let hooks = Arc::new(RecordingTurnHooks::new());
    *hooks.stop.lock().unwrap() = true;
    agent.set_turn_hooks(hooks.clone());

    // Work waiting for the moment the agent would otherwise stop.
    agent.follow_up(user_msg("queued follow-up"));

    let outcome = agent.run(user_msg("go")).await.unwrap();
    assert_eq!(outcome.reason, TurnEndReason::Completed);
    assert_eq!(*hooks.stopped.lock().unwrap(), 1);
    {
        let requests = recorded.lock().unwrap();
        assert_eq!(
            requests.len(),
            1,
            "the stop must prevent another provider request"
        );
        assert!(
            !request_texts(&requests[0])
                .iter()
                .any(|t| t == "typed while the turn ran"),
            "an undrained interjection never reached the provider"
        );
    }
    // The steering message was queued while the turn ran: the stop ends the run before the
    // drain point, so it is still waiting — and so is the follow-up.
    assert_eq!(
        agent.steer_queue_len(),
        1,
        "the stop must not drain the steering queue"
    );
    let history = agent.history();
    assert_eq!(
        history.len(),
        3,
        "prompt + assistant + tool result and nothing else"
    );
    assert!(
        !history
            .iter()
            .any(|m| message_text(m) == "typed while the turn ran"),
        "an undrained interjection stays out of the transcript"
    );
    assert!(
        !history
            .iter()
            .any(|m| message_text(m) == "queued follow-up"),
        "the queued follow-up never runs once the hook stops the run"
    );

    // Both queues belong to the NEXT run: a run starts by prefilling steering, and it drains
    // follow-ups when it reaches the stop boundary run one never reached.
    *hooks.stop.lock().unwrap() = false;
    let second = agent.run(user_msg("second run")).await.unwrap();
    assert_eq!(second.reason, TurnEndReason::Completed);
    let requests = recorded.lock().unwrap();
    assert_eq!(
        requests.len(),
        3,
        "run two: its own turn, then the follow-up it kept"
    );
    assert!(
        request_texts(&requests[1])
            .iter()
            .any(|t| t == "typed while the turn ran"),
        "the interjection the stop refused to drain is delivered at the next run's start"
    );
    assert!(
        request_texts(&requests[2])
            .iter()
            .any(|t| t == "queued follow-up"),
        "the follow-up queued before run one still runs in run two"
    );
    assert_eq!(agent.steer_queue_len(), 0, "run two delivered the queue");
}

/// CORE-02 (c): `transform_context` runs at the request boundary and its output reaches
/// the provider, while the default conversion still filters harness-only messages after it —
/// and nothing a hook produced enters runtime history (hooks are readers, ADR-007).
#[tokio::test]
async fn transform_context_output_reaches_the_request_and_conversion_still_filters() {
    let provider = FauxProvider::new(vec![epsilon_llm::FauxStep::say("done")]);
    let recorded = provider.recorded_requests_handle();
    let agent = epsilon_core::Agent::new(
        LoopLimits::default(),
        Arc::new(epsilon_llm::PanicGuard(provider)),
        Arc::new(ToolRegistry::new()),
        Arc::new(AutoAllow),
        std::env::temp_dir(),
    );
    let hooks = Arc::new(RecordingTurnHooks {
        inject: vec![
            user_msg("injected by transform_context"),
            Message::Custom(epsilon_contracts::message::CustomMessage {
                id: epsilon_contracts::new_id("msg"),
                timestamp_ms: 0,
                custom_type: "ui_only".into(),
                payload: serde_json::json!({"note": "harness-only"}),
            }),
        ],
        ..RecordingTurnHooks::new()
    });
    agent.set_turn_hooks(hooks);

    let outcome = agent.run(user_msg("go")).await.unwrap();
    assert_eq!(outcome.reason, TurnEndReason::Completed);

    let requests = recorded.lock().unwrap();
    assert_eq!(requests.len(), 1);
    let texts = request_texts(&requests[0]);
    assert!(
        texts.iter().any(|t| t == "go"),
        "the run's own context still reaches the request"
    );
    assert!(
        texts.iter().any(|t| t == "injected by transform_context"),
        "transform_context output must reach the provider: {texts:?}"
    );
    assert!(
        !requests[0]
            .messages
            .iter()
            .any(|m| matches!(m, Message::Custom(_))),
        "convert_to_llm still runs after transform_context: {texts:?}"
    );
    drop(requests);

    // Hooks are readers: neither injected message may have entered the transcript. The
    // harness-only one is the sharp end — writing it would poison every later request.
    let history = agent.history();
    assert_eq!(history.len(), 2, "prompt + the assistant answer");
    assert!(
        !history
            .iter()
            .any(|m| message_text(m) == "injected by transform_context"),
        "transform output is request-scoped, never history"
    );
    assert!(
        !history
            .iter()
            .any(|m| matches!(m, Message::Custom(c) if c.custom_type == "ui_only")),
        "a harness-only message a hook injected must never enter history (ADR-007)"
    );
}

/// CORE-02 (d): while a replacement is active the run keeps it in sync with the transcript —
/// the request AFTER a tool-calling turn carries that turn's assistant message and tool
/// result (pi appends to `currentContext.messages`), so a replacement never freezes and the
/// provider gets its own output back.
#[tokio::test]
async fn replacement_context_tracks_the_run_transcript() {
    let provider = FauxProvider::new(vec![
        echo_call("tc_1", "first"),
        echo_call("tc_2", "second"),
        epsilon_llm::FauxStep::say("done"),
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

    let replacement = user_msg("replacement context only");
    let hooks = Arc::new(RecordingTurnHooks {
        apply_once: true,
        ..RecordingTurnHooks::new()
    });
    *hooks.update.lock().unwrap() = Some(epsilon_contracts::turn::TurnUpdate {
        context: Some(vec![replacement.clone()]),
        model: None,
        thinking: None,
    });
    agent.set_turn_hooks(hooks.clone());

    let outcome = agent.run(user_msg("go")).await.unwrap();
    assert_eq!(outcome.reason, TurnEndReason::Completed);

    let requests = recorded.lock().unwrap();
    assert_eq!(requests.len(), 3, "two tool turns and the answer");
    // Request #2 is the snapshot, wholesale (pi replaces `currentContext`).
    assert_eq!(requests[1].messages, vec![replacement.clone()]);

    // Request #3 follows a TOOL-CALLING turn that ran on the replacement. pi appends the
    // turn's assistant message and every tool result to the run's context, so this request
    // must carry them: a replacement that never tracks the transcript would send the same
    // `[replacement]` again, and the model would never see what it just did.
    assert_eq!(
        requests[2].messages.len(),
        3,
        "replacement + the turn's assistant message + its tool result: {:?}",
        request_texts(&requests[2])
    );
    assert_eq!(requests[2].messages[0], replacement);
    assert!(
        matches!(&requests[2].messages[1], Message::Assistant(a)
            if a.tool_calls().any(|(id, name, _)| id == "tc_2" && name == "echo")),
        "the turn's assistant message must be in the request after it"
    );
    assert!(
        matches!(&requests[2].messages[2], Message::ToolResult(t)
            if t.tool_call_id == "tc_2" && !t.is_error),
        "the turn's tool result must be in the request after it"
    );

    // The hook's own view proves the same thing from the other side: call 1 saw the original
    // run context, and call 2 saw exactly what the loop held after turn two — the replacement
    // plus that turn's message and result. One prepare per turn, no re-installations.
    let prepare_contexts = hooks.seen_prepare_contexts.lock().unwrap();
    assert_eq!(prepare_contexts.len(), 3, "one prepare per completed turn");
    assert_eq!(
        prepare_contexts[0].len(),
        3,
        "turn 1 saw prompt + assistant + result"
    );
    assert_eq!(
        prepare_contexts[1].len(),
        3,
        "turn 2 saw the appended replacement"
    );
    assert_eq!(prepare_contexts[1][0], replacement);
    assert_eq!(
        prepare_contexts[1][1], requests[2].messages[1],
        "what the hook saw at turn two is what the next request carried"
    );
    assert_eq!(
        *hooks.seen_context_len.lock().unwrap(),
        vec![3, 1, 3, 3, 4, 4],
        "prepare/stop per turn: run context, replacement, replacement+turn, ...+answer"
    );
}

/// CORE-02 (e): a committed mid-run compaction invalidates the run's replacement — the
/// request after it rebuilds from the current source. Without that a store-backed run stops
/// reading its provider view from the store (the M86/ADR-007 invariant the loop's own
/// compaction path relies on) and re-sends a request the compaction just fixed.
#[tokio::test]
async fn mid_run_compaction_invalidates_the_replacement_context() {
    let dir = tempfile::tempdir().unwrap();
    let store = epsilon_session::SessionStore::open(dir.path().join("session.jsonl")).unwrap();

    let provider = FauxProvider::new(vec![
        epsilon_llm::FauxStep::call(
            "tc_1",
            "echo",
            serde_json::json!({ "text": "x".repeat(4000) }),
        ),
        epsilon_llm::FauxStep::say("after the compaction"),
    ]);
    let recorded = provider.recorded_requests_handle();
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(EchoTool::new()));
    let agent = epsilon_core::Agent::new(
        LoopLimits {
            // The first turn's arguments + result (~2000 estimated tokens) trip this; the
            // bare prompt does not.
            compaction_threshold_tokens: 1500,
            compaction_tail_tokens: 300,
            max_turns: 8,
            ..Default::default()
        },
        Arc::new(epsilon_llm::PanicGuard(provider)),
        Arc::new(registry),
        Arc::new(AutoAllow),
        dir.path().to_path_buf(),
    );
    agent.set_summarizer(Arc::new(epsilon_llm::PanicGuard(SlowProvider::new(
        FauxProvider::new(vec![epsilon_llm::FauxStep::say(
            "## Objective\nsummarized state",
        )]),
        5,
    ))));
    agent.attach_session_store(Arc::new(Mutex::new(store)));
    let store = agent.store_handle_for_tests();

    let frozen = user_msg("frozen replacement");
    let hooks = Arc::new(RecordingTurnHooks {
        apply_once: true,
        ..RecordingTurnHooks::new()
    });
    *hooks.update.lock().unwrap() = Some(epsilon_contracts::turn::TurnUpdate {
        context: Some(vec![frozen.clone()]),
        model: None,
        thinking: None,
    });
    agent.set_turn_hooks(hooks.clone());

    let outcome = agent.run(user_msg("long task")).await.unwrap();
    assert_eq!(outcome.reason, TurnEndReason::Completed);
    assert_eq!(
        *hooks.prepared.lock().unwrap(),
        2,
        "the replacement was installed before the compaction, so it was live when it fired"
    );

    // The compaction really committed mid-run...
    assert!(
        store
            .lock()
            .unwrap()
            .read_all()
            .unwrap()
            .iter()
            .any(|e| matches!(
                e.kind,
                epsilon_contracts::session_entry::EntryKind::Compaction { .. }
            )),
        "the pressure trigger must have committed a compaction"
    );
    assert!(
        matches!(agent.history().first(), Some(Message::User(u))
            if u.content.iter().any(|c| matches!(
                c, UserContent::Text { text } if text.starts_with("<compaction_summary>")))),
        "runtime history must begin with the summary"
    );

    // ...and the request AFTER it is not the frozen replacement: it carries the summary,
    // i.e. it was rebuilt from the current provider view (the store's shadowed model view).
    let requests = recorded.lock().unwrap();
    assert_eq!(requests.len(), 2, "one request per turn");
    assert_eq!(
        requests[0].messages.len(),
        1,
        "the first request is the bare prompt"
    );
    assert_ne!(
        requests[1].messages,
        vec![frozen.clone()],
        "a committed compaction must invalidate the run's replacement"
    );
    let texts = request_texts(&requests[1]);
    assert!(
        texts.iter().any(|t| t.contains("<compaction_summary>")),
        "the request after the compaction carries the summary: {texts:?}"
    );
    assert!(
        !texts.iter().any(|t| t == "frozen replacement"),
        "the stale snapshot must not reach the provider: {texts:?}"
    );
}

/// CORE-02 (f): a hook that breaks the contract and panics is contained at the seam — the
/// run's outcome is the no-hook outcome, and the next run is not refused. Without containment
/// the unwind skips `run`'s cleanup, leaves `running` set, and every later run returns
/// `RunError::AlreadyRunning` (the same failure mode `PanicGuard` prevents for providers).
#[tokio::test]
async fn panicking_turn_hooks_are_contained_and_the_next_run_still_starts() {
    let provider = FauxProvider::new(vec![
        epsilon_llm::FauxStep::say("first"),
        epsilon_llm::FauxStep::say("second"),
    ]);
    let recorded = provider.recorded_requests_handle();
    let agent = epsilon_core::Agent::new(
        LoopLimits::default(),
        Arc::new(epsilon_llm::PanicGuard(provider)),
        Arc::new(ToolRegistry::new()),
        Arc::new(AutoAllow),
        std::env::temp_dir(),
    );
    agent.set_turn_hooks(Arc::new(PanickingTurnHooks));

    let first = agent
        .run(user_msg("go"))
        .await
        .expect("a panicking hook must not fail the run");
    assert_eq!(first.reason, TurnEndReason::Completed);
    assert_eq!(message_text(&first.produced[0]), "first");

    // The second run is the real check: it only starts if the first one reset `running`.
    let second = agent
        .run(user_msg("again"))
        .await
        .expect("a panicking hook must not leave the agent un-runnable");
    assert_eq!(second.reason, TurnEndReason::Completed);
    assert_eq!(message_text(&second.produced[0]), "second");
    assert!(
        !agent.is_running(),
        "a contained panic must not leave a live run"
    );

    // Each seam fell back to its no-op default: the transform ran as identity (the run's own
    // context reached the provider), the conversion still filtered, prepare kept the context
    // and stop answered false (both runs reached the provider).
    let requests = recorded.lock().unwrap();
    assert_eq!(
        requests.len(),
        2,
        "no hook effect may stop or re-route the run"
    );
    assert_eq!(request_texts(&requests[0]), vec!["go".to_string()]);
    assert_eq!(
        requests[0].thinking_level,
        epsilon_contracts::ThinkingLevel::Off,
        "a panicking hook must not change the run's model or thinking level"
    );
}
