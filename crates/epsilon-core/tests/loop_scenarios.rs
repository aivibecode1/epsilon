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
use epsilon_session::ModelViewEntry;

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

/// The [`AgentEvent::Notice`] texts a subscription collected, drained destructively.
///
/// The caller subscribes **before** the run: notice events reach a live broadcast channel, not a
/// log, so a subscriber created afterwards sees nothing. Every F07 test that asserts on a notice
/// therefore subscribes first and drains here.
fn notices_from(rx: &mut tokio::sync::broadcast::Receiver<Arc<AgentEvent>>) -> Vec<String> {
    let mut out = Vec::new();
    loop {
        match rx.try_recv() {
            Ok(ev) => {
                if let AgentEvent::Notice { text } = &*ev {
                    out.push(text.clone());
                }
            }
            Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => continue,
            Err(_) => break,
        }
    }
    out
}

/// Await a spawned run with a ceiling, naming what was being waited for when it expires
/// (`what`), and failing fast rather than hanging the battery.
///
/// This is the floor, not the signal. The tests below still decide *when* to abort from the
/// agent's own state — a provider-entry watch, a `SlowProvider` stall — because a sleep that
/// guesses when the run is live is a race, and a race in a test is a flake the next reader
/// inherits. What a timeout adds is the other half: when the state a test waits for never
/// arrives, the wait ends in a failure that names it. An unbounded wait on the agent under test
/// is one regression away from freezing the whole suite — F07's aborted-turn salvage test hung
/// for 420 seconds at exactly such a wait when a durable append began failing one write earlier
/// than the test assumed, and a hang tells you nothing while a ten-second failure names the bug.
async fn run_bounded(
    run: tokio::task::JoinHandle<Result<epsilon_core::RunOutcome, RunError>>,
    what: &str,
) -> Result<epsilon_core::RunOutcome, RunError> {
    match tokio::time::timeout(std::time::Duration::from_secs(30), run).await {
        Ok(joined) => joined.expect("the run task must not panic"),
        Err(_) => panic!(
            "the run did not finish within 30s while waiting for {what}: a run that neither \
             returns nor ends is a wedge, so this fails the test instead of holding the battery"
        ),
    }
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

    // Bounded like every other await on the agent under test in this file: this one waits on a
    // run that is *expected* to be the live one, so the only thing an unbounded await could add
    // is a hang if the second run's `AlreadyRunning` refusal ever stopped being the outcome.
    run_bounded(first, "the first of two concurrent runs to end")
        .await
        .unwrap();
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
    assert!(
        agent.abort_current_run(),
        "the run must still be live when the abort lands: it is stalled in stream creation, so \
         the only way this fails is the run having already ended"
    );
    let outcome = run_bounded(handle, "the aborted run to end").await.unwrap();
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
    assert!(
        agent.abort_current_run(),
        "the run must still be live when the abort lands: the provider hangs after its first \
         delta, so the only way this fails is the run having already ended"
    );
    let outcome = run_bounded(handle, "the aborted mid-stream run to end")
        .await
        .unwrap();
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
    assert!(
        agent.abort_current_run(),
        "the run must still be live when the abort lands: the provider hangs after its first \
         delta, so the only way this fails is the run having already ended"
    );
    let outcome = run_bounded(
        run,
        "the aborted mid-stream run to commit its terminal message",
    )
    .await
    .unwrap();
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
    assert!(
        agent.abort_current_run(),
        "the first slow call (400ms) must still be running when the abort lands"
    );
    let outcome = run_bounded(run, "the mid-batch abort to end")
        .await
        .unwrap();
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

/// The panic the tool above raises. Named so the assertion below compares against the
/// *payload* the tool actually produced rather than a phrase the author reasoned about —
/// the first version of this test asserted against the tool's description, which is a
/// different string the failure never contains.
const TOOL_PANIC: &str = "a native tool that panics must not unwind through the run";

/// A native tool that panics where the loop has no boundary around it.
///
/// `Sequential` with no resource, so `plan_batch` puts the call on the **Barrier**
/// path (`exec.rs:211-217`) — which is the path that awaits `run_one` directly in the
/// loop rather than inside a `tokio::spawn`. That is the whole point: the Segments
/// path turns a panic into a `JoinError` the batch already handles, and the Barrier
/// path has no equivalent, so an unwinding tool takes the run with it.
struct BarrierPanickingTool {
    descriptor: epsilon_contracts::tool::ToolDescriptor,
}

#[async_trait]
impl epsilon_contracts::tool::Tool for BarrierPanickingTool {
    fn descriptor(&self) -> &epsilon_contracts::tool::ToolDescriptor {
        &self.descriptor
    }

    async fn execute(
        &self,
        _call: epsilon_contracts::tool::ToolInvocation,
        _ctx: epsilon_contracts::tool::ToolContext,
        _sink: epsilon_contracts::tool::ToolUpdateSink,
    ) -> Result<epsilon_contracts::tool::ToolOutput, epsilon_contracts::tool::ToolFailure> {
        panic!("{TOOL_PANIC}")
    }
}

/// **A panicking native tool lands a failed tool result, not an unwind.**
///
/// `contain_panic` (`lib.rs:546-574`) already wraps every other seam a third party can
/// break — the provider call, the detached body, all four turn hooks — and a tool is the
/// one that reaches *user code*: a native tool is arbitrary Rust, and an extension or
/// plugin tool is arbitrary *someone else's* Rust. `Tool::execute` was the one boundary
/// without a containment, on the path that cannot make one for itself.
///
/// What the unwind costs is not only the run. The model already emitted a tool call, the
/// run already spent money on it, and the transcript has a pending row for it: unwinding
/// past `run` drops all three on the floor. So the failure has to become a tool *result*
/// — the model reads "the tool panicked" and can decide what to do — rather than an
/// exception no caller in the front ends is written to catch.
#[tokio::test]
async fn a_panicking_native_tool_lands_a_failed_result_on_the_barrier_path() {
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(BarrierPanickingTool {
        descriptor: epsilon_contracts::tool::ToolDescriptor {
            name: "explodes".into(),
            label: "Explodes".into(),
            description: "panics when executed".into(),
            parameters_schema: serde_json::json!({ "type": "object" }),
            // Sequential and resource-less: `plan_batch` returns Barrier, which is the
            // path under test.
            execution_mode: epsilon_contracts::tool::ExecutionMode::Sequential,
        },
    }));
    let agent = epsilon_core::Agent::new(
        LoopLimits::default(),
        Arc::new(SlowProvider::new(
            FauxProvider::new(vec![
                epsilon_llm::FauxStep::call("tc_1", "explodes", serde_json::json!({})),
                epsilon_llm::FauxStep::say("recovered from the panic"),
            ]),
            0,
        )),
        Arc::new(registry),
        Arc::new(AutoAllow),
        std::env::temp_dir(),
    );

    let outcome = agent
        .run(user_msg("make it explode"))
        .await
        .expect("a panicking tool must not unwind through the run");

    // The tool result the model reads names the panic and keeps its own message.
    let panicked = outcome.produced.iter().find_map(|m| match m {
        Message::ToolResult(r) if r.tool_name == "explodes" => Some(r),
        _ => None,
    });
    let result = panicked.unwrap_or_else(|| {
        panic!(
            "the panicking call must have produced a tool result: {:?}",
            outcome.produced
        )
    });
    assert!(
        result.is_error,
        "a panicking tool reported success would be the worst outcome available: {:?}",
        result.content
    );
    assert!(
        result.content.iter().any(|c| match c {
            epsilon_contracts::content::ContentBlock::Text { text } =>
                text.contains("panicked") && text.contains(TOOL_PANIC),
            _ => false,
        }),
        "the result must name the panic and keep its own message: {:?}",
        result.content
    );
}

/// **And on the Segments path, which used to be the asymmetry itself.**
///
/// A `Parallel` tool with no resource plans to `Segments`, where the call runs inside a
/// `tokio::spawn` and a panic already turned into a `JoinError` the batch handled — so the
/// Barrier path was the one with no boundary at all. The containment was therefore put in
/// `run_one`, which both plans share, and this test is what stops someone narrowing it
/// back to the Barrier arm: one boundary, two plans, one place.
#[tokio::test]
async fn a_panicking_tool_lands_a_failed_result_on_the_segments_path_too() {
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(BarrierPanickingTool {
        descriptor: epsilon_contracts::tool::ToolDescriptor {
            name: "explodes".into(),
            label: "Explodes".into(),
            description: "panics when executed".into(),
            parameters_schema: serde_json::json!({ "type": "object" }),
            // Parallel and resource-less: two calls conflict with nothing, so each
            // becomes its own group and the batch runs them in spawned tasks.
            execution_mode: epsilon_contracts::tool::ExecutionMode::Parallel,
        },
    }));
    registry.register(Arc::new(EchoTool::new()));
    let agent = epsilon_core::Agent::new(
        LoopLimits::default(),
        Arc::new(SlowProvider::new(
            FauxProvider::new(vec![
                epsilon_llm::FauxStep::ToolCalls {
                    text: None,
                    calls: vec![
                        epsilon_llm::FauxToolCall {
                            id: "tc_boom".into(),
                            name: "explodes".into(),
                            arguments: serde_json::json!({}),
                        },
                        epsilon_llm::FauxToolCall {
                            id: "tc_ok".into(),
                            name: "echo".into(),
                            arguments: serde_json::json!({ "text": "still here" }),
                        },
                    ],
                },
                epsilon_llm::FauxStep::say("recovered from both"),
            ]),
            0,
        )),
        Arc::new(registry),
        Arc::new(AutoAllow),
        std::env::temp_dir(),
    );

    let outcome = agent
        .run(user_msg("make it explode in parallel"))
        .await
        .expect("a panicking tool must not unwind through the run");

    // **The sibling's work survives.** A panic in one spawned task taking the whole batch
    // with it is the same failure one layer up: the surviving call had already run and
    // its result is real work the user would otherwise lose.
    let ok = outcome.produced.iter().find_map(|m| match m {
        Message::ToolResult(r) if r.tool_call_id == "tc_ok" => Some(r),
        _ => None,
    });
    assert!(
        ok.is_some_and(|r| !r.is_error),
        "the sibling call's real work must survive a panic in its neighbour: {:?}",
        outcome.produced
    );
}

/// Hooks that panic on the **tool** seams.
///
/// `before_tool_call` and `after_tool_call` are user code exactly as the four turn hooks
/// are — and the turn hooks are contained (`lib.rs:4368`) while the tool hooks are not.
/// A panic in `after_tool_call` unwinds out of `run_one`, which on the Segments plan is
/// inside a `tokio::spawn`, so the task dies and the batch's join arm sees a
/// `JoinError`. That is the trigger this test needs, and it is a real hole in its own
/// right: the tool hook seam has no panic boundary.
struct PanickingToolHooks {
    /// The tool whose `after_tool_call` panics; anything else passes through.
    trips_on: String,
}

#[async_trait]
impl epsilon_contracts::tool::ToolHooks for PanickingToolHooks {
    async fn after_tool_call(
        &self,
        ctx: epsilon_contracts::tool::AfterToolCallContext,
    ) -> Result<Option<epsilon_contracts::tool::AfterToolCallResult>, String> {
        if ctx.tool_name == self.trips_on {
            panic!("an after_tool_call hook that panics must not discard its siblings");
        }
        Ok(None)
    }
}

/// **One group's join failure must not discard the other groups' completed work.**
///
/// The batch's join arm returns `Err` at the *first* failing group, and `Err` out of
/// `execute_batch` throws away `slots` — the results the other groups had already
/// produced, and the results the failing group's own task had already pushed into its
/// local `out` before it died. Every one of those is real work the loop paid for and a
/// user would want back: "never lose a user's work" is the repository's own rule
/// (AGENTS.md §6), and this is it in a one-line arm.
///
/// What it costs the caller is worse than a lost result. `execute_batch` returning `Err`
/// means the whole turn fails — so the transcript carries tool calls with no answers,
/// the model is told nothing about what happened, and a front end shows a failed run
/// whose work had actually finished.
#[tokio::test]
async fn a_join_failure_keeps_the_other_groups_finished_work() {
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(EchoTool::new()));
    registry.register(Arc::new(BarrierPanickingHookTool {
        descriptor: epsilon_contracts::tool::ToolDescriptor {
            name: "hooked".into(),
            label: "Hooked".into(),
            description: "succeeds; its hook then panics".into(),
            parameters_schema: serde_json::json!({ "type": "object" }),
            // Parallel and resource-less: two calls conflict with nothing, so each is its
            // own group and the batch runs them in separate spawned tasks — which is what
            // makes a panic a `JoinError` rather than an unwind.
            execution_mode: epsilon_contracts::tool::ExecutionMode::Parallel,
        },
    }));
    let agent = epsilon_core::Agent::new(
        LoopLimits::default(),
        Arc::new(SlowProvider::new(
            FauxProvider::new(vec![
                epsilon_llm::FauxStep::ToolCalls {
                    text: None,
                    calls: vec![
                        epsilon_llm::FauxToolCall {
                            id: "tc_boom".into(),
                            name: "hooked".into(),
                            arguments: serde_json::json!({}),
                        },
                        epsilon_llm::FauxToolCall {
                            id: "tc_ok".into(),
                            name: "echo".into(),
                            arguments: serde_json::json!({ "text": "still here" }),
                        },
                    ],
                },
                epsilon_llm::FauxStep::say("recovered"),
            ]),
            0,
        )),
        Arc::new(registry),
        Arc::new(AutoAllow),
        std::env::temp_dir(),
    );
    agent.set_tool_hooks(Arc::new(PanickingToolHooks {
        trips_on: "hooked".into(),
    }));

    let outcome = agent
        .run(user_msg("run both in parallel"))
        .await
        .expect("one group failing must not fail the whole batch");

    // **The sibling's real work survives**, which is the assertion the defect existed
    // for. And the panicking call gets an answer too, because `slots.into_iter().flatten()`
    // silently dropping it would leave the transcript holding a call with no result.
    let ok = outcome.produced.iter().find_map(|m| match m {
        Message::ToolResult(r) if r.tool_call_id == "tc_ok" => Some(r),
        _ => None,
    });
    assert!(
        ok.is_some_and(|r| !r.is_error),
        "the surviving call's real work must reach the model: {:?}",
        outcome.produced
    );
    let answered = outcome
        .produced
        .iter()
        .any(|m| matches!(m, Message::ToolResult(r) if r.tool_call_id == "tc_boom"));
    assert!(
        answered,
        "the failed call must still be answered — a call with no result is a hole the \
         model has to guess at: {:?}",
        outcome.produced
    );
}

/// A tool that succeeds, so the panic comes from the hook rather than from it.
struct BarrierPanickingHookTool {
    descriptor: epsilon_contracts::tool::ToolDescriptor,
}

#[async_trait]
impl epsilon_contracts::tool::Tool for BarrierPanickingHookTool {
    fn descriptor(&self) -> &epsilon_contracts::tool::ToolDescriptor {
        &self.descriptor
    }

    async fn execute(
        &self,
        _call: epsilon_contracts::tool::ToolInvocation,
        _ctx: epsilon_contracts::tool::ToolContext,
        _sink: epsilon_contracts::tool::ToolUpdateSink,
    ) -> Result<epsilon_contracts::tool::ToolOutput, epsilon_contracts::tool::ToolFailure> {
        Ok(epsilon_contracts::tool::ToolOutput::text("the tool ran"))
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
            //
            // Raised from 1_500 by M194, preserving the relationship rather than the digit.
            // The comment above states the property — "the bare prompt does not [trip it]"
            // — and that stopped being true when the no-anchor context estimate began
            // counting the system prompt: the fixed prefix is 1_633 tokens of agent
            // contract (printed by `an_empty_session_still_costs_the_system_prompt` in
            // `usage_anchor.rs`), so a bare prompt now measures ~1_633 and cleared a
            // 1_500 budget on its own. The test then compacted on turn one, and
            // `requests[0]` carried 2 messages instead of the bare prompt it asserts on.
            //
            // Measured with `EPSILON_TRACE_COMPACTION` printing what the trigger reads:
            //   after the bare prompt     1_674
            //   after the tool result     2_123
            // The budget goes between them, so the bare prompt does not trip it and the
            // result does — which is the property the comment above has always claimed.
            // 2_400 was my first guess and was above *both*, so nothing compacted at all.
            compaction_threshold_tokens: 1_900,
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

// ---- M91: the CORE-02 test gaps M89's review recorded (drain point, overflow pop, epoch) ----

/// M91 (closes the M89 limits entry "the drain-point mirror … not exercised *with an active
/// replacement*"; `audit/evidence/m89-review.md`): the loop appends every message it adds to
/// the transcript to the run's replacement context, including the messages a drain point
/// injects. The existing tests pin the turn's own assistant message and tool results; this
/// pins the drain point: with a replacement installed at the end of turn one, the steering
/// message queued during that turn must appear in the very next request — the one built
/// from the replacement.
#[tokio::test]
async fn a_drain_point_message_reaches_an_installed_replacement() {
    let provider = FauxProvider::new(vec![
        epsilon_llm::FauxStep::call("tc_1", "steer", serde_json::json!({})),
        epsilon_llm::FauxStep::say("after the drain"),
    ]);
    let recorded = provider.recorded_requests_handle();
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
    assert_eq!(
        *hooks.prepared.lock().unwrap(),
        2,
        "one prepare per completed turn: the install after turn one, and the turn after it"
    );

    let requests = recorded.lock().unwrap();
    assert_eq!(requests.len(), 2, "turn one and the turn after the swap");
    assert_eq!(
        request_texts(&requests[0]),
        vec!["go".to_string()],
        "turn one still runs on the un-replaced context"
    );
    // Request #2 is built from the replacement (the swap landed at the end of turn one)…
    assert_eq!(requests[1].messages[0], replacement);
    // …and the interjection the drain point injected AFTER the swap is in it: the mirror
    // the review could not find a test for. A replacement that froze at install time — or
    // a drain point that only appended to runtime history — would send just
    // `[replacement]` here and silently drop the user's input.
    assert_eq!(
        request_texts(&requests[1]),
        vec![
            "replacement context only".to_string(),
            "typed while the turn ran".to_string()
        ],
        "the drained steering message must reach the replacement too"
    );
    assert_eq!(
        requests[1].messages.len(),
        2,
        "exactly the replacement plus the drained message"
    );

    // The interjection is transcript state, exactly once (ADR-007: the replacement is
    // run-scoped, the transcript is the record).
    let history = agent.history();
    assert_eq!(
        history
            .iter()
            .filter(|m| message_text(m) == "typed while the turn ran")
            .count(),
        1,
        "the drained interjection is in the transcript exactly once"
    );
    assert!(
        !history.contains(&replacement),
        "the replacement itself never enters the transcript"
    );
}

/// M91 (closes the M89 limits entry "the overflow-discard pop … not exercised *with an
/// active replacement*"; `audit/evidence/m89-review.md`): an overflow retry discards the
/// failed attempt from runtime history, from `produced` and from the run's replacement.
/// A committed compaction then invalidates the replacement (the epoch rule), so the
/// request the run actually sends is the rebuilt source — and the joint contract a caller
/// sees is: the post-retry request matches the transcript and carries NOTHING of the
/// discarded attempt. The replacement pop is not separately observable because the same
/// commit that triggers the retry is what drops the replacement.
#[tokio::test]
async fn an_overflow_retry_discards_the_failed_attempt_from_transcript_and_replacement() {
    let provider = FauxProvider::new(vec![
        echo_call("tc_1", "first"),
        epsilon_llm::FauxStep::Error {
            message: "context window exceeded (faux): 200000 > 128000".into(),
        },
        epsilon_llm::FauxStep::say("after recovery"),
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
    // The summarizer is its own provider, so the main script stays exactly three steps:
    // turn one, the overflowed turn, the retry.
    agent.set_summarizer(Arc::new(epsilon_llm::PanicGuard(FauxProvider::new(vec![
        epsilon_llm::FauxStep::say("## Objective\nsummarized state"),
    ]))));

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
    assert_eq!(
        outcome.reason,
        TurnEndReason::Completed,
        "the overflow retry recovered the run"
    );
    assert_eq!(
        *hooks.prepared.lock().unwrap(),
        2,
        "the failed attempt never reached the hook seam: one prepare for turn one, one for the retry"
    );

    let requests = recorded.lock().unwrap();
    assert_eq!(
        requests.len(),
        3,
        "turn one, the overflowed turn, and the retry"
    );
    assert_eq!(request_texts(&requests[0]), vec!["go".to_string()]);
    assert_eq!(
        requests[1].messages,
        vec![replacement.clone()],
        "the failed turn ran on the replacement, so the pop had something to pop"
    );

    // The retry's request is the rebuilt source — the compaction summary proves the
    // replacement was dropped, and the transcript rides with it…
    let retry = &requests[2];
    let texts = request_texts(retry);
    assert!(
        texts.iter().any(|t| t.starts_with("<compaction_summary>")),
        "the post-retry request must be rebuilt from the source: {texts:?}"
    );
    assert!(
        !texts.iter().any(|t| t == "replacement context only"),
        "the stale replacement must not ride along: {texts:?}"
    );
    // …and nothing of the discarded attempt is in it. The retained tail keeps the whole
    // pre-overflow transcript (the messages are small), so a missed discard would leave
    // an error-stamped assistant message in the request — an extra message the provider
    // would read as the model's own failed output.
    assert!(
        !retry
            .messages
            .iter()
            .any(|m| matches!(m, Message::Assistant(a) if a.error_message.is_some())),
        "the discarded failed attempt must not reach the provider: {texts:?}"
    );
    let history = agent.history();
    assert_eq!(
        history.len(),
        5,
        "prompt + turn one's assistant and result + the summary + the recovered answer"
    );
    assert!(
        !history
            .iter()
            .any(|m| matches!(m, Message::Assistant(a) if a.error_message.is_some())),
        "the failed attempt is gone from the transcript"
    );
    assert_eq!(
        retry.messages,
        history[..4].to_vec(),
        "the post-retry request matches the transcript"
    );
}

/// M91: a tool that swaps the agent's provider-view source mid-run (the CORE-02 epoch
/// guard): `Some(store)` attaches that store, `None` detaches the attached one. Both must
/// invalidate the run's replacement, so the next request rebuilds from the new source.
/// The agent handle is late-bound, like [`SteerDuringTurn`].
struct SwapStoreDuringTurn {
    descriptor: epsilon_contracts::tool::ToolDescriptor,
    agent: Arc<Mutex<Option<Arc<epsilon_core::Agent>>>>,
    store: Option<Arc<Mutex<epsilon_session::SessionStore>>>,
}

#[async_trait]
impl epsilon_contracts::tool::Tool for SwapStoreDuringTurn {
    fn descriptor(&self) -> &epsilon_contracts::tool::ToolDescriptor {
        &self.descriptor
    }
    async fn execute(
        &self,
        _call: epsilon_contracts::tool::ToolInvocation,
        _ctx: epsilon_contracts::tool::ToolContext,
        _sink: epsilon_contracts::tool::ToolUpdateSink,
    ) -> Result<epsilon_contracts::tool::ToolOutput, epsilon_contracts::tool::ToolFailure> {
        let output = match (&self.store, self.agent.lock().unwrap().as_ref()) {
            (Some(store), Some(agent)) => {
                agent.attach_session_store(Arc::clone(store));
                "attached"
            }
            (None, Some(agent)) => {
                agent.detach_session_store();
                "detached"
            }
            _ => "no agent",
        };
        Ok(epsilon_contracts::tool::ToolOutput::text(output))
    }
}

/// M91 (closes the M89 limits entry "the epoch is also bumped when the store is attached or
/// detached mid-run … not regression-tested, because no in-tree caller attaches a store
/// mid-run today"; `audit/evidence/m89-review.md`): with a replacement installed, attaching
/// a session store mid-run swaps the source the next request reads from. The replacement
/// must not survive that swap.
#[tokio::test]
async fn attaching_a_store_mid_run_invalidates_the_replacement() {
    let dir = tempfile::tempdir().unwrap();
    let late = Arc::new(Mutex::new(
        epsilon_session::SessionStore::open(dir.path().join("late.jsonl")).unwrap(),
    ));

    let provider = FauxProvider::new(vec![
        echo_call("tc_1", "first"),
        epsilon_llm::FauxStep::call("tc_2", "attach_store", serde_json::json!({})),
        epsilon_llm::FauxStep::say("after the swap"),
    ]);
    let recorded = provider.recorded_requests_handle();
    let slot: Arc<Mutex<Option<Arc<epsilon_core::Agent>>>> = Arc::new(Mutex::new(None));
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(EchoTool::new()));
    registry.register(Arc::new(SwapStoreDuringTurn {
        descriptor: epsilon_contracts::tool::ToolDescriptor {
            name: "attach_store".into(),
            label: "Attach store".into(),
            description: "Inert fixture: attaches a session store while the turn runs".into(),
            parameters_schema: serde_json::json!({"type": "object"}),
            execution_mode: epsilon_contracts::tool::ExecutionMode::Sequential,
        },
        agent: slot.clone(),
        store: Some(Arc::clone(&late)),
    }));
    let agent = epsilon_core::Agent::new(
        LoopLimits::default(),
        Arc::new(epsilon_llm::PanicGuard(provider)),
        Arc::new(registry),
        Arc::new(AutoAllow),
        dir.path().to_path_buf(),
    );
    *slot.lock().unwrap() = Some(agent.clone());

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
    assert_eq!(
        requests.len(),
        3,
        "turn one, the attach turn, and the turn after the swap"
    );
    assert_eq!(
        request_texts(&requests[0]),
        vec!["go".to_string()],
        "turn one ran before anything was attached"
    );
    assert_eq!(
        requests[1].messages,
        vec![replacement.clone()],
        "the replacement was live when the store was attached"
    );
    // The attach happened DURING the turn whose request was the replacement: that
    // replacement is stale now, so the turn after the swap reads the newly attached
    // store — whose view holds only what was pushed since the attach.
    let after_swap = &requests[2];
    assert!(
        !after_swap.messages.iter().any(|m| m == &replacement),
        "the stale replacement must not survive a mid-run attach: {:?}",
        request_texts(after_swap)
    );
    // Each script step is one TURN, so the attach runs in the turn whose request was the
    // replacement; the store is attached during it and receives what is pushed after —
    // starting with that turn's own tool result. The request after the swap is that
    // store view, not the three-message replacement the run held a moment before.
    assert_eq!(
        after_swap.messages.len(),
        1,
        "the request is the store's view, not the replacement: {:?}",
        request_texts(after_swap)
    );
    assert!(
        matches!(&after_swap.messages[0], Message::ToolResult(t) if t.tool_call_id == "tc_2"),
        "the store's first entry is the attach turn's own result"
    );

    // The store really is the attached one, and it received everything pushed after the
    // swap — the attach became the durable mirror.
    let attached = agent.store_handle_for_tests();
    assert!(Arc::ptr_eq(&attached, &late));
    let stored: Vec<Message> = attached
        .lock()
        .unwrap()
        .read_all()
        .unwrap()
        .into_iter()
        .filter_map(|e| match e.kind {
            epsilon_contracts::session_entry::EntryKind::Message { message, .. } => Some(message),
            _ => None,
        })
        .collect();
    assert_eq!(
        stored.len(),
        2,
        "the attach turn's result plus the answer the turn after the swap produced"
    );
    assert_eq!(
        after_swap.messages,
        stored[..1].to_vec(),
        "the post-swap request is the store's own view"
    );
    assert_eq!(message_text(&stored[1]), "after the swap");
}

/// M91 (same entry): the other half of the swap — a store attached BEFORE the run that is
/// DETACHED mid-run must invalidate the replacement the same way, and the next request
/// falls back to the runtime transcript.
#[tokio::test]
async fn detaching_the_store_mid_run_invalidates_the_replacement() {
    let dir = tempfile::tempdir().unwrap();
    let held = Arc::new(Mutex::new(
        epsilon_session::SessionStore::open(dir.path().join("held.jsonl")).unwrap(),
    ));

    let provider = FauxProvider::new(vec![
        echo_call("tc_1", "first"),
        epsilon_llm::FauxStep::call("tc_2", "detach_store", serde_json::json!({})),
        epsilon_llm::FauxStep::say("after the detach"),
    ]);
    let recorded = provider.recorded_requests_handle();
    let slot: Arc<Mutex<Option<Arc<epsilon_core::Agent>>>> = Arc::new(Mutex::new(None));
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(EchoTool::new()));
    registry.register(Arc::new(SwapStoreDuringTurn {
        descriptor: epsilon_contracts::tool::ToolDescriptor {
            name: "detach_store".into(),
            label: "Detach store".into(),
            description: "Inert fixture: detaches the session store while the turn runs".into(),
            parameters_schema: serde_json::json!({"type": "object"}),
            execution_mode: epsilon_contracts::tool::ExecutionMode::Sequential,
        },
        agent: slot.clone(),
        store: None,
    }));
    let agent = epsilon_core::Agent::new(
        LoopLimits::default(),
        Arc::new(epsilon_llm::PanicGuard(provider)),
        Arc::new(registry),
        Arc::new(AutoAllow),
        dir.path().to_path_buf(),
    );
    *slot.lock().unwrap() = Some(agent.clone());
    // Attached before the run: this test's whole point is the DETACH, so the source swap
    // has to be the detach alone.
    agent.attach_session_store(Arc::clone(&held));

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
    assert_eq!(
        requests.len(),
        3,
        "turn one, the detach turn, and the turn after the swap"
    );
    assert_eq!(
        request_texts(&requests[0]),
        vec!["go".to_string()],
        "turn one read the attached store's view"
    );
    assert_eq!(
        requests[1].messages,
        vec![replacement.clone()],
        "the replacement was live when the store was detached"
    );
    // The detach bumped the epoch, so the turn after it rebuilds from runtime history —
    // the whole transcript, which is neither the replacement nor the store's view.
    let after_swap = &requests[2];
    assert!(
        !after_swap.messages.iter().any(|m| m == &replacement),
        "the stale replacement must not survive a mid-run detach"
    );
    assert!(
        request_texts(after_swap).iter().any(|t| t == "go"),
        "the rebuilt request is the transcript, so it carries the original prompt: {:?}",
        request_texts(after_swap)
    );
    assert_eq!(
        after_swap.messages,
        agent.history()[..5].to_vec(),
        "the post-detach request matches the transcript (the run's own answer is not in it yet)"
    );
    assert_eq!(agent.history().len(), 6);
    assert!(
        agent.session_store().is_none(),
        "the detach really dropped the store"
    );
}

// ---------------------------------------------------------------------------
// F07 — a durable write that fails is reported, and the views never diverge in
// silence. The failure is injected at the store's write path
// (`SessionStore::set_write_fault_for_tests`): a real one cannot be staged here,
// because the store's handle is already open and a read-only file, a revoked
// directory permission and a non-writable attribute all still accept writes
// through it.
// ---------------------------------------------------------------------------

/// A prompt whose durable append fails must not start a run: the store's projection is what
/// the provider request is built from, so a turn whose prompt never reached the file would be
/// answered from a transcript that does not contain the question.
///
/// The `Notice` assertion is the one that matters most: this is the only report a TUI user
/// gets. The TUI discards `run()`'s `Err` (`crates/epsilon-tui/src/app.rs`), so before the
/// notice existed a failed durable write was invisible to the person who would have to retype
/// the turn.
#[tokio::test]
async fn a_failed_durable_append_is_reported_and_no_request_is_built_from_the_old_view() {
    let dir = tempfile::tempdir().unwrap();
    let mut store =
        epsilon_session::SessionStore::open_in(dir.path().join("s.jsonl"), dir.path()).unwrap();
    // Occurrence 1: the first append — the prompt — is the write this test is about.
    store.set_write_fault_for_tests(epsilon_session::FaultOp::Append, "disk full", 1, 1);
    let store = Arc::new(Mutex::new(store));
    let handle = Arc::clone(&store);

    // The script would answer if the run ever reached the provider.
    let provider = FauxProvider::new(vec![epsilon_llm::FauxStep::say("answered anyway")]);
    let recorded = provider.recorded_requests_handle();
    let agent = epsilon_core::Agent::new(
        LoopLimits::default(),
        Arc::new(epsilon_llm::PanicGuard(provider)),
        Arc::new(ToolRegistry::new()),
        Arc::new(AutoAllow),
        std::env::temp_dir(),
    );
    agent.attach_session_store(store);
    let mut rx = agent.subscribe();

    let err = agent
        .run(user_msg("hello"))
        .await
        .expect_err("a run whose prompt cannot be recorded must not start");
    let text = err.to_string();
    assert!(
        text.contains("could not record this turn") && text.contains("disk full"),
        "the error must name the failure and its cause: {text}"
    );

    // (a) The user learns. The notice is the only report a TUI user gets.
    let notices = notices_from(&mut rx);
    assert_eq!(notices.len(), 1, "exactly one report: {notices:?}");
    assert!(
        notices[0].contains("not on disk") && notices[0].contains("may be missing it"),
        "the notice says the session may be missing the turn, not just that a write failed: {}",
        notices[0]
    );

    // (b) No request was built from the store's older view — the run stopped before the
    // provider was called at all.
    assert!(
        recorded.lock().unwrap().is_empty(),
        "the provider must never be asked to answer a question the store does not hold"
    );
    // The prompt is in memory (what the user typed is not lost) and NOT in the store.
    assert_eq!(agent.history().len(), 1, "the prompt survives in memory");
    assert!(
        handle.lock().unwrap().model_view().unwrap().is_empty(),
        "the store's view is the older transcript, which is why no request may use it"
    );

    // (c) The run did not wedge the agent: the guard reset the run-scoped state even though
    // `run()` returned early.
    assert!(
        !agent.is_running(),
        "a failed durable write must not leave the agent looking busy forever"
    );
}

/// The other half of F07: the next request is refused rather than built from a store whose
/// projection is missing a turn the loop believes it recorded.
///
/// This is the *abort-salvage* path on purpose. Four of the durable writes sit in `select!` arms
/// that return the assistant message rather than a `Result`, so they cannot stop the run the way
/// the prompt and tool-result appends do; without the latch those are exactly the writes that
/// would leave the two views disagreeing with nobody told. Cancel mid-stream (the abort message
/// is a real assistant turn, so losing it loses a turn), let its append fail, and then ask for
/// another run: the store is missing the answer the transcript holds, and no request may be
/// built from it.
#[tokio::test]
async fn an_aborted_turn_lost_to_the_store_refuses_the_next_request() {
    use epsilon_contracts::message::{AssistantMessage, StopReason, Usage};
    use epsilon_contracts::stream::StreamEvent;

    /// The provider's stream: two events, then a hang, signalling the delta on its way out.
    ///
    /// The signal has to be the *delta*, not the stream's construction: the run applies `Start`
    /// first and only then can the inner select — the arm that records the partial text — be the
    /// one an abort lands on. Abort after the stream exists but before the delta is folded and the
    /// run ends Aborted all the same, but the salvaged message is empty, and this test is about a
    /// *turn* being lost, so it needs the text the store failed to record.
    ///
    /// `next()` is the run's own await, which is what makes the signal exact: the sender runs
    /// inside the run's poll, so the run cannot be scheduled again between the flip and the
    /// hand-over of the delta. By the time the test's watch wakes and the abort lands, the run is
    /// parked in the select that polls the delta and the cancellation token — the token first
    /// (both selects are `biased`), so the arm that keeps partial text is the one that runs.
    struct SignalDeltaStream {
        events: std::collections::VecDeque<epsilon_contracts::stream::StreamEvent>,
        foldable: tokio::sync::watch::Sender<bool>,
    }

    #[async_trait]
    impl epsilon_llm::EventStream for SignalDeltaStream {
        async fn next(&mut self) -> Option<epsilon_contracts::stream::StreamEvent> {
            use epsilon_contracts::stream::StreamEvent;
            if matches!(self.events.front(), Some(StreamEvent::TextDelta { .. })) {
                let _ = self.foldable.send(true);
            }
            if let Some(ev) = self.events.pop_front() {
                Some(ev)
            } else {
                std::future::pending().await
            }
        }
    }

    /// Signals when the provider has actually been entered, so the test never depends on a
    /// sleep to know the run reached its first request; the delta arm signals separately.
    struct HangAfterStartProvider {
        entered: tokio::sync::watch::Sender<bool>,
        foldable: tokio::sync::watch::Sender<bool>,
    }

    #[async_trait]
    impl epsilon_llm::StreamFn for HangAfterStartProvider {
        async fn stream(
            &self,
            _model: &epsilon_llm::ModelRef,
            _ctx: epsilon_llm::LlmContext,
        ) -> epsilon_llm::BoxEventStream {
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
            let stream = Box::new(SignalDeltaStream {
                events: [
                    StreamEvent::Start { partial },
                    StreamEvent::TextDelta {
                        index: 0,
                        delta: "half an answer".into(),
                    },
                ]
                .into_iter()
                .collect(),
                foldable: self.foldable.clone(),
            });
            let _ = self.entered.send(true);
            stream
        }
    }

    let dir = tempfile::tempdir().unwrap();
    let mut store =
        epsilon_session::SessionStore::open_in(dir.path().join("s.jsonl"), dir.path()).unwrap();
    // The write this test is about is the SECOND append of the run. `set_write_fault_for_tests`
    // takes an occurrence and a count — `times` alone cannot aim at a later write, because a
    // count starting at the first append can only fail the prompt first, and since F07 a failed
    // prompt append ends the run before the provider is ever called. So: let the prompt's append
    // through (`occurrence: 2`), fail the very next one once (`times: 1`) — the salvage write.
    // `appends()` around the run proves the sequence instead of assuming it, so a write inserted
    // between the prompt and the stream fails this test at its own assertion rather than silently
    // moving the fault onto a write whose caller does not care.
    store.set_write_fault_for_tests(epsilon_session::FaultOp::Append, "disk full", 2, 1);
    assert_eq!(
        store.appends(),
        0,
        "arming a fault must not itself write: an append here would take the position the \
         occurrence argument is counting"
    );
    let store = Arc::new(Mutex::new(store));
    let handle = Arc::clone(&store);
    let (entered_tx, mut entered_rx) = tokio::sync::watch::channel(false);
    let (foldable_tx, mut foldable_rx) = tokio::sync::watch::channel(false);

    let agent = epsilon_core::Agent::new(
        LoopLimits::default(),
        Arc::new(epsilon_llm::PanicGuard(HangAfterStartProvider {
            entered: entered_tx,
            foldable: foldable_tx,
        })),
        Arc::new(ToolRegistry::new()),
        Arc::new(AutoAllow),
        std::env::temp_dir(),
    );
    agent.attach_session_store(store);
    let mut rx = agent.subscribe();

    // The prompt's append succeeds (the fault's occurrence is the append after it), so the run
    // reaches the provider — the salvage append is the one that fails, and the store ends up
    // holding the prompt but not the answer.
    let run = tokio::spawn({
        let agent = Arc::clone(&agent);
        async move { agent.run(user_msg("go")).await }
    });
    // Bounded: satisfied when the provider is entered. A run that dies before its first request —
    // which is what F07's prompt-append `?` does, and what an append fault aimed one write too
    // early does — never enters the provider, so the watch never flips, and an unbounded
    // `wait_for` waits forever instead of failing. It cost a 420-second battery round: a hang
    // tells the next reader nothing, a failure in ten seconds names the write that went wrong.
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        entered_rx
            .wait_for(|entered| *entered)
            .await
            .expect("the provider entry watch is never dropped while the run lives")
    })
    .await
    .expect(
        "the provider was never entered: the run ended before its first request. The fault is \
         armed for the append after the prompt, so a run that stops first means a durable append \
         now ends the run earlier than this test assumes (F07) and the fault is aimed at the \
         wrong write",
    );
    // Then wait for the delta to be on its way out (see `SignalDeltaStream`): at this point the
    // run is parked in the select that keeps partial text, so the abort cannot land on the
    // connect-time arm and salvage an empty message instead.
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        foldable_rx
            .wait_for(|folded| *folded)
            .await
            .expect("the delta watch is never dropped while the run lives")
    })
    .await
    .expect("the provider was entered but never handed over its first delta");
    assert!(agent.abort_current_run(), "the run is live");
    let err = run_bounded(
        run,
        "the aborted turn's failed salvage append to end the run",
    )
    .await
    .expect_err("the aborted turn could not be recorded, so the run reports it");
    assert!(
        err.to_string().contains("could not record this turn"),
        "the run's own error names the durable failure: {err}"
    );
    // The sequence the fault plan claims, asserted rather than assumed: the prompt append that
    // succeeded, the salvage append that failed, and no third writer — and no armed failure left
    // unspent, which would mean the abort never reached the salvage path at all.
    {
        let st = handle.lock().unwrap();
        assert_eq!(
            st.appends(),
            2,
            "one append for the prompt and one for the aborted turn's answer, in that order"
        );
        assert!(
            !st.write_fault_pending_for_tests(),
            "the armed failure was spent, so the write that failed was the salvage append and not \
             some earlier one"
        );
    }
    let notices = notices_from(&mut rx);
    assert!(
        notices.iter().any(|n| n.contains("not on disk")),
        "and the user is told the session may be missing the turn: {notices:?}"
    );

    // The divergence is real and visible from the store: the prompt, no answer.
    assert_eq!(
        handle.lock().unwrap().len(),
        1,
        "the prompt was recorded; the aborted turn was not"
    );
    assert!(
        agent
            .history()
            .iter()
            .any(|m| matches!(m, Message::Assistant(a) if a.text() == "half an answer")),
        "the transcript in memory still has the answer the store lost"
    );

    // The next request is refused: building it from the store would hand the model a transcript
    // that disagrees with the loop's own record — the exact F07 divergence.
    let err = agent
        .run(user_msg("next question"))
        .await
        .expect_err("no request may be built from a store missing a recorded turn");
    assert!(
        err.to_string()
            .contains("missing a turn this session recorded"),
        "the refusal names the divergence rather than a generic store error: {err}"
    );
}

/// `pop_last` is the overflow-retry discard, and its failure used to be discarded too
/// (`let _ = s.pop_last()`). The pop is what removes the oversized attempt from the store; when
/// the store refuses it, the retry would re-send exactly the request the compaction was
/// supposed to fix. So the run stops instead — and says why.
///
/// This is the one durable-write failure whose two views diverge in a way the test can see from
/// the store alone: the loop discarded the failed attempt from its own transcript, and the store
/// still holds it. That divergence is why the report cannot be a log line nobody reads.
#[tokio::test]
async fn a_failed_store_pop_stops_the_overflow_retry_and_is_reported() {
    // An oversized-request refusal is the only path that pops. The summarizer is its own
    // provider, so the main script stays exactly two steps: the overflowed turn, and the retry
    // that must never happen.
    let provider = FauxProvider::new(vec![
        epsilon_llm::FauxStep::Error {
            message: "context window exceeded (faux): 200000 > 128000".into(),
        },
        epsilon_llm::FauxStep::say("after recovery"),
    ]);
    let recorded = provider.recorded_requests_handle();
    let agent = epsilon_core::Agent::new(
        LoopLimits::default(),
        Arc::new(epsilon_llm::PanicGuard(provider)),
        Arc::new(ToolRegistry::new()),
        Arc::new(AutoAllow),
        std::env::temp_dir(),
    );
    agent.set_summarizer(Arc::new(epsilon_llm::PanicGuard(FauxProvider::new(vec![
        epsilon_llm::FauxStep::say("## Objective\nsummarized state"),
    ]))));

    let dir = tempfile::tempdir().unwrap();
    let mut store =
        epsilon_session::SessionStore::open_in(dir.path().join("s.jsonl"), dir.path()).unwrap();
    // Occurrence 1: the pop this test is about is the only pop of the run; the appends around it
    // are a different operation and are never affected by a `FaultOp::PopLast` plan.
    store.set_write_fault_for_tests(epsilon_session::FaultOp::PopLast, "disk full on pop", 1, 1);
    let store = Arc::new(Mutex::new(store));
    let handle = Arc::clone(&store);
    agent.attach_session_store(store);
    let mut rx = agent.subscribe();

    let err = agent
        .run(user_msg("hello"))
        .await
        .expect_err("a pop the store refused stops the overflow retry");
    let text = err.to_string();
    assert!(
        text.contains("disk full on pop"),
        "the error carries the store's own reason: {text}"
    );
    let notices = notices_from(&mut rx);
    assert!(
        notices
            .iter()
            .any(|n| n.contains("not on disk") && n.contains("disk full on pop")),
        "the failed pop is announced like every other durable-write failure: {notices:?}"
    );
    assert_eq!(
        recorded.lock().unwrap().len(),
        1,
        "the retry never ran: only the overflowed attempt reached the provider"
    );
    // What the store kept is the divergence: the loop dropped the failed attempt from its own
    // transcript, and the store still offers it to the next request.
    let kept = handle.lock().unwrap();
    assert!(
        kept.model_view()
            .unwrap()
            .iter()
            .any(|v| matches!(v, ModelViewEntry::Message(Message::Assistant(a)) if a.stop_reason == epsilon_contracts::message::StopReason::Error)),
        "the store still holds the discarded attempt, which is why the run had to stop"
    );
    assert!(
        !agent
            .history()
            .iter()
            .any(|m| matches!(m, Message::Assistant(a) if a.stop_reason == epsilon_contracts::message::StopReason::Error)),
        "and the loop's own transcript did discard it — the two views disagree, which is the \
         report's whole point"
    );
}

/// A `/tree` jump must change the conversation, not just the file.
///
/// This is the test that pins the fix behind `reproject_current_branch`. Before
/// it, `/tree` moved the store's leaf and then printed the resulting branch — so
/// the session file recorded a new position while the user kept reading a
/// transcript that no longer described it. The audit called `/tree` a viewer for
/// exactly this reason, and the claim was correct.
///
/// The shape is a real fork, not a mock: one turn, a branch off an ancestor, and
/// a second turn on the sibling. Both branches then exist on disk, which is what
/// makes "the transcript changed" a meaningful assertion — before the jump the
/// runtime holds the sibling's messages, and after it must hold the ancestor's.
#[tokio::test]
async fn jumping_the_tree_leaf_re_projects_the_visible_conversation() {
    let dir = tempfile::tempdir().expect("tmp");
    let path = dir.path().join("tree.jsonl");

    let agent = agent_with(
        vec![
            epsilon_llm::FauxStep::Assistant {
                text: "first answer".into(),
            },
            epsilon_llm::FauxStep::Assistant {
                text: "second answer".into(),
            },
        ],
        Arc::new(AutoAllow),
        LoopLimits::default(),
    );
    // Attach BEFORE the run, or the durable mirror has nothing to write into and
    // the tree would be empty — which is the M61 lesson about ordering.
    let store = epsilon_session::SessionStore::open_in(&path, dir.path()).expect("store");
    agent.attach_session_store(Arc::new(std::sync::Mutex::new(store)));

    agent
        .run(user_msg("first question"))
        .await
        .expect("turn one");
    let after_first = agent.history().len();
    assert!(
        after_first >= 2,
        "a turn leaves a user and an assistant message"
    );

    // The entry to jump back to: the first user message. Take it from the store
    // rather than guessing, because the ids are the store's business.
    let target = {
        let handle = agent.session_store().expect("attached");
        let st = handle.lock().expect("lock");
        st.branch_entries()
            .expect("branch")
            .into_iter()
            .find(|e| {
                matches!(
                    e.kind,
                    epsilon_contracts::session_entry::EntryKind::Message { .. }
                )
            })
            .map(|e| e.id)
            .expect("at least one message entry")
    };

    // Jump. This is what the TUI's Enter arm does after `navigate_to`.
    {
        let handle = agent.session_store().expect("attached");
        let mut st = handle.lock().expect("lock");
        st.navigate_to(&target).expect("navigate to a real entry");
    }
    let projected = agent.reproject_current_branch().expect("re-project");

    assert_eq!(
        projected, 1,
        "jumping to the first user message leaves exactly that one message visible"
    );
    let visible = agent.history();
    assert_eq!(
        visible.len(),
        1,
        "the runtime transcript must be the branch that was jumped to, not the one \
         it was on before: {visible:?}"
    );
    assert!(
        matches!(&visible[0], epsilon_contracts::message::Message::User(_)),
        "and it must be the message we jumped to, not an assistant reply: {:?}",
        visible[0]
    );
}

// ---------------------------------------------------------------------------
// M311 — the same F07 rule, applied to two sites that had been left out.
// ---------------------------------------------------------------------------

/// **A model switch whose session record fails says so.**
///
/// `record_model_change` used to be `let _ = guard.append(ModelChange)`, and M303's own doc
/// claimed *"the front end decides whether to say so"* — which no front end could do, because
/// nothing told one. The consequence is the owner's 2026-10-04 report stated backwards: a switch
/// that applies and is visible in the status bar, whose record did not land, comes back on its
/// previous model at the next resume with no explanation.
///
/// **Why the `Notice` and not an `Err`.** `set_model_id` must not fail: the switch has already
/// happened and refusing would undo a choice the reader can see take effect. The `Err` would also
/// reach only callers that look at it, and the TUI discards `run()`'s result — the identical
/// argument `persist` records for emitting a notice instead of returning alone.
///
/// The fault is injected at the store's write path for the same reason the F07 tests do it: the
/// handle is already open, so a read-only file and a revoked directory permission both still
/// accept writes through it.
#[tokio::test]
async fn a_model_switch_whose_session_record_fails_is_reported() {
    let dir = tempfile::tempdir().unwrap();
    let mut store =
        epsilon_session::SessionStore::open_in(dir.path().join("s.jsonl"), dir.path()).unwrap();
    store.set_write_fault_for_tests(epsilon_session::FaultOp::Append, "disk full", 1, 1);
    let agent = epsilon_core::Agent::new(
        LoopLimits::default(),
        Arc::new(epsilon_llm::PanicGuard(FauxProvider::new(Vec::new()))),
        Arc::new(ToolRegistry::new()),
        Arc::new(AutoAllow),
        std::env::temp_dir(),
    );
    agent.attach_session_store(Arc::new(Mutex::new(store)));
    let mut rx = agent.subscribe();

    agent.set_model_id("deepseek-v4-flash");

    // The switch itself must have applied — otherwise this test would pass against a build that
    // simply refused the switch, which is the wrong fix for the wrong reason.
    assert_eq!(
        agent.model_id(),
        "deepseek-v4-flash",
        "the switch must still apply: the notice reports what did not happen, it does not undo it"
    );

    let notices: Vec<String> = std::iter::from_fn(|| rx.try_recv().ok())
        .filter_map(|ev| match &*ev {
            epsilon_contracts::agent_event::AgentEvent::Notice { text } => Some(text.clone()),
            _ => None,
        })
        .collect();
    assert!(
        notices
            .iter()
            .any(|n| n.contains("could not record") && n.contains("previous model")),
        "a failed session record must be announced, and must say what the consequence is. \
         Notices seen: {notices:?}"
    );
}

// ---------------------------------------------------------------------------
// M312 — the refusal has to be *reported*, or it is a lost turn.
// ---------------------------------------------------------------------------

/// **A turn whose session lease was taken is refused, and the reader is told.**
///
/// The store test proves the write stops. This proves the stop is **visible**, which is the half
/// that had to come first: a refusal nobody prints is not a refusal, it is a dropped turn — which
/// is why M311 gave `append`'s callers a way to report before the store was allowed to refuse at
/// all. The takeover here is real, not a fault injection: another process's token in the lock
/// file, which is the whole condition `LeaseOwnership::Taken` describes.
///
/// Two assertions that only make sense together: the run **fails**, so the loop does not keep
/// building a transcript the file is not keeping, and a **notice** carries the reason, because the
/// TUI discards `run()`'s result and a window has no console.
#[tokio::test]
async fn a_turn_whose_session_lease_was_taken_is_refused_and_reported() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.jsonl");
    let store = epsilon_session::SessionStore::open_in(&path, dir.path()).unwrap();
    // Another process takes this session while we hold it open. Same shape as the store test;
    // here it is the *loop* that has to notice.
    std::fs::write(
        path.with_extension("lock"),
        "pid=999999 owner=another-epsilon\n",
    )
    .expect("another holder took the lock");

    let agent = epsilon_core::Agent::new(
        LoopLimits::default(),
        Arc::new(epsilon_llm::PanicGuard(FauxProvider::new(vec![
            epsilon_llm::FauxStep::say("an answer nothing recorded"),
        ]))),
        Arc::new(ToolRegistry::new()),
        Arc::new(AutoAllow),
        dir.path().to_path_buf(),
    );
    agent.attach_session_store(Arc::new(Mutex::new(store)));
    let mut rx = agent.subscribe();

    let outcome = agent.run(user_msg("go")).await;
    assert!(
        outcome.is_err(),
        "the turn must fail rather than carry on against a session file this process no longer \
         owns — continuing is what interleaves two writers. Outcome: {outcome:?}"
    );

    let notices: Vec<String> = std::iter::from_fn(|| rx.try_recv().ok())
        .filter_map(|ev| match &*ev {
            epsilon_contracts::agent_event::AgentEvent::Notice { text } => Some(text.clone()),
            _ => None,
        })
        .collect();
    assert!(
        notices
            .iter()
            .any(|n| n.contains("lease is no longer ours")),
        "the refusal must reach the reader in words, naming the cause. Notices seen: {notices:?}"
    );

    let written = std::fs::read_to_string(&path).expect("read the session file");
    assert!(
        !written.contains("an answer nothing recorded"),
        "and the turn must not be half-recorded: a refused write is not a lost one"
    );
}
