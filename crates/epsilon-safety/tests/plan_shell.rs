//! Plan-mode shell refusal through core preflight, without starting a process.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use async_trait::async_trait;
use epsilon_contracts::content::UserContent;
use epsilon_contracts::message::{Message, UserMessage};
use epsilon_contracts::tool::{
    ExecutionMode, Tool, ToolContext, ToolDescriptor, ToolFailure, ToolInvocation, ToolOutput,
    ToolUpdateSink,
};
use epsilon_core::{Agent, LoopLimits, ToolRegistry};
use epsilon_llm::{FauxProvider, FauxStep};
use epsilon_safety::{Action, AutoApprove, PermissionEngine, Rule, Ruleset, RulesetGate};

struct RecordingShell {
    descriptor: ToolDescriptor,
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl Tool for RecordingShell {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    async fn execute(
        &self,
        _: ToolInvocation,
        _: ToolContext,
        _: ToolUpdateSink,
    ) -> Result<ToolOutput, ToolFailure> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(ToolOutput::text("inert fixture"))
    }
}

#[tokio::test]
async fn plan_shell_calls_never_reach_execution_despite_project_allow_and_auto_approval() {
    for name in ["bash", "shell"] {
        for command in [
            "git commit -m test",
            "git push",
            "cargo test",
            "echo read-only",
            "",
        ] {
            let calls = Arc::new(AtomicUsize::new(0));
            let mut registry = ToolRegistry::new();
            registry.register(Arc::new(RecordingShell {
                descriptor: ToolDescriptor {
                    name: name.into(), label: name.into(), description: "Inert shell stand-in".into(),
                    parameters_schema: serde_json::json!({"type":"object","properties":{"command":{"type":"string"}},"required":["command"]}),
                    execution_mode: ExecutionMode::Sequential,
                },
                calls: calls.clone(),
            }));
            let engine = PermissionEngine::layered(vec![
                epsilon_safety::plan_ruleset(),
                Ruleset {
                    name: "project".into(),
                    rules: vec![Rule::new(name, "*", Action::Allow)],
                },
            ]);
            let workspace = tempfile::tempdir().unwrap();
            let agent = Agent::new(
                LoopLimits {
                    verification_nudges: 0,
                    ..LoopLimits::default()
                },
                Arc::new(FauxProvider::new(vec![
                    FauxStep::call("shell_call", name, serde_json::json!({"command":command})),
                    FauxStep::say("done"),
                ])),
                Arc::new(registry),
                Arc::new(RulesetGate::layered(engine, Arc::new(AutoApprove))),
                workspace.path().to_path_buf(),
            );
            let result = agent
                .run(Message::User(UserMessage {
                    id: epsilon_contracts::new_id("msg"),
                    timestamp_ms: 0,
                    content: vec![UserContent::Text { text: "go".into() }],
                }))
                .await
                .unwrap();
            assert_eq!(calls.load(Ordering::SeqCst), 0, "{name}: {command}");
            assert!(result.produced.iter().any(|m| matches!(m, Message::ToolResult(r) if r.tool_call_id == "shell_call" && r.is_error)), "expected refusal for {name}: {command}");
        }
    }
}
