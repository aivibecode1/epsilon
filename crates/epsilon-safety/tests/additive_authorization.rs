//! Additive preflight authorization with inert tools only: no file or process operations.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use epsilon_contracts::agent_event::{PermissionDecision, PermissionRequest, TurnEndReason};
use epsilon_contracts::content::UserContent;
use epsilon_contracts::message::{Message, UserMessage};
use epsilon_contracts::tool::{
    ExecutionMode, Tool, ToolContext, ToolDescriptor, ToolFailure, ToolInvocation, ToolOutput,
    ToolUpdateSink,
};
use epsilon_core::{Agent, Authorization, LoopLimits, PermissionGate, ToolRegistry};
use epsilon_llm::{FauxProvider, FauxStep};
use epsilon_safety::{Action, ApprovalUi, PermissionEngine, Rule, Ruleset, RulesetGate};

struct InertTool(ToolDescriptor, Arc<Mutex<usize>>);

#[async_trait]
impl Tool for InertTool {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.0
    }

    async fn execute(
        &self,
        _: ToolInvocation,
        _: ToolContext,
        _: ToolUpdateSink,
    ) -> Result<ToolOutput, ToolFailure> {
        *self.1.lock().unwrap() += 1;
        Ok(ToolOutput::text("inert"))
    }
}

#[derive(Default)]
struct RecordingUi(Mutex<Vec<PermissionRequest>>);

#[async_trait]
impl ApprovalUi for RecordingUi {
    async fn ask(&self, req: PermissionRequest) -> PermissionDecision {
        self.0.lock().unwrap().push(req);
        PermissionDecision::AlwaysSession
    }
}

struct RecordingGate {
    inner: RulesetGate,
    seen: Mutex<Vec<PermissionRequest>>,
}

#[async_trait]
impl PermissionGate for RecordingGate {
    async fn authorize(&self, req: PermissionRequest) -> Authorization {
        self.seen.lock().unwrap().push(req.clone());
        self.inner.authorize(req).await
    }
}

fn gate(layers: Vec<Ruleset>) -> (Arc<RecordingGate>, Arc<RecordingUi>) {
    let ui = Arc::new(RecordingUi::default());
    let gate = Arc::new(RecordingGate {
        inner: RulesetGate::layered(PermissionEngine::layered(layers), ui.clone()),
        seen: Mutex::new(Vec::new()),
    });
    (gate, ui)
}

fn rules(rules: Vec<Rule>) -> Ruleset {
    Ruleset {
        name: "fixture".into(),
        rules,
    }
}

async fn run(
    name: &str,
    args: serde_json::Value,
    gate: Arc<RecordingGate>,
) -> (epsilon_core::RunOutcome, usize) {
    let calls = Arc::new(Mutex::new(0));
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(InertTool(
        ToolDescriptor {
            name: name.into(),
            label: name.into(),
            description: "Inert authorization fixture".into(),
            parameters_schema: serde_json::json!({"type": "object"}),
            execution_mode: ExecutionMode::Sequential,
        },
        calls.clone(),
    )));
    // Lexical paths only: neither this directory nor the arguments are accessed by the tool.
    let workspace = std::env::temp_dir().join("epsilon-additive-policy-fixture");
    let agent = Agent::new(
        LoopLimits {
            verification_nudges: 0,
            ..Default::default()
        },
        Arc::new(FauxProvider::new(vec![
            FauxStep::call("fixture", name, args),
            FauxStep::say("done"),
        ])),
        Arc::new(registry),
        gate,
        workspace,
    );
    let result = agent
        .run(Message::User(UserMessage {
            id: epsilon_contracts::new_id("msg"),
            timestamp_ms: 0,
            content: vec![UserContent::Text {
                text: "fixture".into(),
            }],
        }))
        .await
        .unwrap();
    let count = *calls.lock().unwrap();
    (result, count)
}

fn assert_blocked(result: &epsilon_core::RunOutcome, calls: usize) {
    assert_eq!(calls, 0);
    assert_eq!(result.reason, TurnEndReason::Blocked);
    assert!(
        result.produced.iter().any(
            |m| matches!(m, Message::ToolResult(r) if r.tool_call_id == "fixture" && r.is_error)
        )
    );
}

#[tokio::test]
async fn additive_plan_denies_original_capabilities_before_external_approval() {
    for name in ["bash", "shell", "write", "edit"] {
        for key in ["path", "file_path", "dir"] {
            let (gate, ui) = gate(vec![
                epsilon_safety::plan_ruleset(),
                rules(vec![
                    Rule::new(name, "*", Action::Allow),
                    Rule::new("external_dir", "*", Action::Allow),
                ]),
            ]);
            let (result, calls) = run(
                name,
                serde_json::json!({key: "../inert-fixture"}),
                gate.clone(),
            )
            .await;
            assert_blocked(&result, calls);
            let seen = gate.seen.lock().unwrap();
            assert_eq!(seen.len(), 1);
            assert_eq!(seen[0].capability, name);
            assert!(ui.0.lock().unwrap().is_empty());
        }
    }
}

#[tokio::test]
async fn additive_project_subject_deny_survives_external_allow() {
    let (gate, ui) = gate(vec![rules(vec![
        Rule::new("read", "../inert-fixture", Action::Deny),
        Rule::new("external_dir", "*", Action::Allow),
    ])]);
    let (result, calls) = run(
        "read",
        serde_json::json!({"path": "../inert-fixture"}),
        gate.clone(),
    )
    .await;
    assert_blocked(&result, calls);
    let seen = gate.seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].capability, "read");
    assert_eq!(seen[0].pattern, "../inert-fixture");
    assert!(ui.0.lock().unwrap().is_empty());
}

#[tokio::test]
async fn additive_external_deny_still_blocks_allowed_tool() {
    let (gate, ui) = gate(vec![rules(vec![
        Rule::new("read", "*", Action::Allow),
        Rule::new("external_dir", "*", Action::Deny),
    ])]);
    let (result, calls) = run(
        "read",
        serde_json::json!({"path": "../inert-fixture"}),
        gate.clone(),
    )
    .await;
    assert_blocked(&result, calls);
    let seen = gate.seen.lock().unwrap();
    assert_eq!(
        seen.iter()
            .map(|r| r.capability.as_str())
            .collect::<Vec<_>>(),
        ["read", "external_dir"]
    );
    assert!(ui.0.lock().unwrap().is_empty());
}

#[tokio::test]
async fn additive_approvals_have_separate_session_keys() {
    let (gate, ui) = gate(vec![rules(vec![Rule::new("read", "*", Action::Ask)])]);
    for _ in 0..2 {
        let (result, calls) = run(
            "read",
            serde_json::json!({"path": "../inert-fixture"}),
            gate.clone(),
        )
        .await;
        assert_eq!(calls, 1);
        assert_eq!(result.reason, TurnEndReason::Completed);
    }
    let asks = ui.0.lock().unwrap();
    assert_eq!(
        asks.len(),
        2,
        "one approval per capability, remembered separately"
    );
    assert_eq!(asks[0].capability, "read");
    assert_eq!(asks[1].capability, "external_dir");
    assert_ne!(asks[0].request_id, asks[1].request_id);
    assert_eq!(
        gate.inner.session_allows(),
        vec![
            ("read".into(), "../inert-fixture".into()),
            ("external_dir".into(), "../inert-fixture".into()),
        ]
    );
}

#[tokio::test]
async fn additive_in_workspace_calls_keep_one_normalized_authorization() {
    let (gate, ui) = gate(vec![]);
    let (result, calls) = run(
        "read",
        serde_json::json!({"path": "./inert-fixture"}),
        gate.clone(),
    )
    .await;
    assert_eq!(calls, 1);
    assert_eq!(result.reason, TurnEndReason::Completed);
    let seen = gate.seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].capability, "read");
    assert_eq!(seen[0].pattern, "inert-fixture");
    assert!(ui.0.lock().unwrap().is_empty());
}
