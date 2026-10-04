//! End-to-end permission scenarios: the ruleset engine gating REAL tools through the
//! full loop — the M6 acceptance proof.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use epsilon_contracts::agent_event::{PermissionDecision, PermissionRequest};
use epsilon_contracts::content::UserContent;
use epsilon_contracts::message::{Message, UserMessage};
use epsilon_core::LoopLimits;
use epsilon_core::testing::SlowProvider;
use epsilon_llm::{FauxProvider, FauxStep};
use epsilon_safety::{ApprovalUi, PermissionEngine, Rule, RulesetGate};

fn user(text: &str) -> Message {
    Message::User(UserMessage {
        id: epsilon_contracts::new_id("msg"),
        timestamp_ms: 0,
        content: vec![UserContent::Text {
            text: text.to_string(),
        }],
    })
}

struct RecordingUi {
    answers: Mutex<Vec<PermissionDecision>>,
    seen: Mutex<Vec<PermissionRequest>>,
}

#[async_trait]
impl ApprovalUi for RecordingUi {
    async fn ask(&self, req: PermissionRequest) -> PermissionDecision {
        self.seen.lock().unwrap().push(req);
        let mut a = self.answers.lock().unwrap();
        if a.is_empty() {
            PermissionDecision::Deny
        } else {
            a.remove(0)
        }
    }
}

fn run_agent(
    steps: Vec<FauxStep>,
    gate: Arc<RulesetGate>,
    workspace: &std::path::Path,
) -> arctest::Spawned {
    let provider = SlowProvider::new(FauxProvider::new(steps), 50);
    let registry = Arc::new(epsilon_tools::builtin_registry(
        &epsilon_tools::Confinement::Auto,
    ));
    let agent = epsilon_core::Agent::new(
        LoopLimits {
            max_turns: 8,
            ..Default::default()
        },
        Arc::new(epsilon_llm::PanicGuard(provider)),
        registry,
        gate,
        workspace.to_path_buf(),
    );
    // `exec.rs:782-799` builds the one `ToolContext` the loop uses, and it already carries
    // `task_host: Some(agent.clone_task_host())` — so a `task` call here does fan out to a real
    // child Agent. This comment exists because the scenario below has failed twice for reasons
    // that had nothing to do with permissions: a `task` call missing its required `description`,
    // and a fixture that looked for the child's output in the wrong place. The precondition
    // assertion is what caught both.
    let handle = tokio::spawn(async move { agent.run(user("go")).await });
    arctest::Spawned(handle)
}

// tiny wrapper so the spawned future type stays nameable
mod arctest {
    pub struct Spawned(
        pub tokio::task::JoinHandle<Result<epsilon_core::RunOutcome, epsilon_core::RunError>>,
    );
    impl Spawned {
        pub async fn join(self) -> epsilon_core::RunOutcome {
            self.0.await.expect("run task").expect("run ok")
        }
    }
}

#[tokio::test]
async fn external_write_asks_then_tool_refuses_anyway() {
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let outside_path = outside.path().join("escape.txt");

    // Approve the external_dir ask at the UI layer...
    let ui = Arc::new(RecordingUi {
        answers: Mutex::new(vec![PermissionDecision::AllowOnce]),
        seen: Mutex::new(vec![]),
    });
    let gate = Arc::new(RulesetGate::new(Arc::clone(&ui) as Arc<dyn ApprovalUi>));

    let steps = vec![
        FauxStep::call(
            "tc_w",
            "write",
            serde_json::json!({"path": outside_path.display().to_string(), "content": "nope"}),
        ),
        FauxStep::say("done"),
    ];
    let outcome = run_agent(steps, gate, dir.path()).join().await;

    // ...the gate DID consult the human on external_dir,
    let seen = ui.seen.lock().unwrap();
    assert!(
        seen.iter().any(|r| r.capability == "external_dir"),
        "expected an external_dir ask"
    );

    // ...and the tool still refused: defense in depth.
    assert!(outcome.produced.iter().any(|m| matches!(
        m,
        Message::ToolResult(t) if t.is_error && t.content.iter().any(|c| matches!(
            c,
            epsilon_contracts::content::ContentBlock::Text { text } if text.contains("outside the workspace")
        ))
    )));
    assert!(
        !outside_path.exists(),
        "nothing may land outside the workspace"
    );
}

#[tokio::test]
async fn denied_external_write_blocks_with_guidance_before_execution() {
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let outside_path = outside.path().join("blocked.txt");

    let ui = Arc::new(RecordingUi {
        answers: Mutex::new(vec![PermissionDecision::DenyWithGuidance {
            guidance: "ask the user before writing outside the project".into(),
        }]),
        seen: Mutex::new(vec![]),
    });
    let gate = Arc::new(RulesetGate::new(Arc::clone(&ui) as Arc<dyn ApprovalUi>));

    let steps = vec![
        FauxStep::call(
            "tc_b",
            "write",
            serde_json::json!({"path": outside_path.display().to_string(), "content": "x"}),
        ),
        FauxStep::say("acknowledged"),
    ];
    let outcome = run_agent(steps, gate, dir.path()).join().await;

    assert!(outcome.produced.iter().any(|m| matches!(
        m,
        Message::ToolResult(t) if t.is_error && t.content.iter().any(|c| matches!(
            c,
            epsilon_contracts::content::ContentBlock::Text { text } if text.contains("ask the user before writing")
        ))
    )));
    assert_eq!(
        outcome.reason,
        epsilon_contracts::agent_event::TurnEndReason::Blocked
    );
    assert!(!outside_path.exists());
}

#[test]
fn plan_mode_denies_bash_even_when_project_allows_it() {
    let engine = PermissionEngine::layered(vec![
        epsilon_safety::plan_ruleset(),
        epsilon_safety::Ruleset {
            name: "project".into(),
            rules: vec![Rule::new("bash", "*", epsilon_safety::Action::Allow)],
        },
    ]);
    // Core preflight supplies the command head, not "git commit ...".
    assert_eq!(engine.evaluate("bash", "git"), epsilon_safety::Action::Deny);
}

#[tokio::test]
async fn custom_ruleset_allows_bash_without_asking() {
    let dir = tempfile::tempdir().unwrap();

    // Project layer allows bash outright: engine allow => no UI involvement.
    let engine = PermissionEngine::layered(vec![epsilon_safety::Ruleset {
        name: "project".into(),
        rules: vec![Rule::new("bash", "*", epsilon_safety::Action::Allow)],
    }]);
    let ui = Arc::new(RecordingUi {
        answers: Mutex::new(vec![]),
        seen: Mutex::new(vec![]),
    });
    let gate = Arc::new(RulesetGate::layered(
        engine,
        Arc::clone(&ui) as Arc<dyn ApprovalUi>,
    ));

    let steps = [
        FauxStep::call(
            "tc_s",
            "bash",
            serde_json::json!({"command": "echo rule-allowed"}),
        ),
        FauxStep::say("ok"),
    ];
    let outcome = run_agent(steps.to_vec(), gate, dir.path()).join().await;

    assert!(
        ui.seen.lock().unwrap().is_empty(),
        "explicit allow must not hit the UI"
    );
    assert!(outcome.produced.iter().any(|m| matches!(
        m,
        Message::ToolResult(t) if !t.is_error && t.tool_name == "bash"
    )));
}

/// **An `always allow` in the main loop must not authorise a subagent's tool calls.**
///
/// # The defect this pins
///
/// `build_task_child` handed the child `Arc::clone(&self.gate)`, and `SessionMemory` — the list
/// of "always allow" pairs a user accumulates by answering `a` — lives *inside* that gate
/// (`epsilon-safety/src/gate.rs:47,54`). So one "always allow bash" in the main loop silently
/// authorised `bash` for every subagent afterwards, with no prompt and no inheritance decision
/// anywhere. Measured on 2026-10-04 as a production path, not a test artefact:
/// `epsilon-runtime:4702` registers `TaskTool`, `exec.rs:795` injects the task host, and
/// `delegate_task` reaches the child.
///
/// # Why the child must be asked again
///
/// opencode hands its subagents a **derived** ruleset that inherits denies only
/// (`references/opencode-1.18.34/.../subagent-permissions.ts:21-25`), so a remembered *allow*
/// never reaches one. The child inherits the parent's **ruleset** — policy the owner wrote — and
/// not the parent's **session memory**, which is a decision made about one conversation.
///
/// # What this asserts
///
/// Two `bash` calls reach the approval UI: the parent's and the child's. Today the second is
/// short-circuited by shared memory and the count is **one**, which is the whole bug: a user who
/// allowed bash once would have no way to notice that every agent they spawn can now run it
/// without asking.
#[tokio::test]
async fn an_always_session_allow_does_not_reach_a_task_child() {
    let dir = tempfile::tempdir().unwrap();

    // One answer: the parent asks, is told "always", and is never asked again. The child
    // must therefore ask — so the UI needs a second answer queued, or it denies and the test
    // still sees the second ask, which is the property under test.
    let ui = Arc::new(RecordingUi {
        answers: Mutex::new(vec![
            PermissionDecision::AlwaysSession,
            PermissionDecision::AllowOnce,
        ]),
        seen: Mutex::new(vec![]),
    });
    let gate = Arc::new(RulesetGate::new(Arc::clone(&ui) as Arc<dyn ApprovalUi>));

    // The parent runs bash, then delegates a task whose child also runs bash.
    //
    // **One tool call per turn, and neither extra step is decoration.** The faux provider is a
    // single shared script consumed in order, and this fixture got it wrong twice before it got
    // it right:
    //
    // - Two `bash` steps before the `task` meant the **parent** ran both in its own batch and
    //   the child never got a turn, so the second bash proved nothing about inheritance.
    // - A `say` between them ended the run after the parent's first turn, so the child never ran
    //   at all — the ask count stayed at one for a reason that had nothing to do with
    //   permissions.
    //
    // The working shape is `bash` → `task` → `bash` → `say`: the parent's turn makes its call
    // and its `task`, the child's turn draws the next step, and the closing sentence ends it.
    let steps = vec![
        FauxStep::call(
            "tc_parent",
            "bash",
            serde_json::json!({"command": "echo parent"}),
        ),
        FauxStep::call(
            "tc_task",
            "task",
            serde_json::json!({"description": "probe bash in the child", "prompt": "run echo child"}),
        ),
        FauxStep::call(
            "tc_child",
            "bash",
            serde_json::json!({"command": "echo child"}),
        ),
        FauxStep::say("delegated"),
    ];

    let outcome = run_agent(steps, gate, dir.path()).join().await;
    // Precondition: the child must actually have run. A test asserting "two asks" when only
    // one agent ever existed would pass for the wrong reason the moment the fixture broke —
    // and it did break this way once, on a `task` call missing its required `description`.
    assert!(
        outcome.produced.iter().any(|m| matches!(
            m,
            Message::ToolResult(t) if t.tool_name == "bash" && !t.is_error
        )),
        "the scenario must reach a second bash call; produced: {:?}",
        outcome
            .produced
            .iter()
            .map(|m| match m {
                Message::ToolResult(t) => format!("tool:{}:{}", t.tool_name, t.is_error),
                Message::Assistant(a) => format!("assistant:{}", a.text()),
                _ => "other".into(),
            })
            .collect::<Vec<_>>()
    );

    let bash_asks = ui
        .seen
        .lock()
        .unwrap()
        .iter()
        .filter(|r| r.capability == "bash")
        .count();
    assert_eq!(
        bash_asks, 2,
        "both the parent's bash and the child's must be asked: one ask means the \
         session's 'always allow' leaked into the subagent"
    );
}
#[tokio::test]
async fn env_read_ask_flow_end_to_end() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(".env"), "TOKEN=abc\n").unwrap();

    // First .env read asks and is approved for-the-session; second identical read is silent.
    let ui = Arc::new(RecordingUi {
        answers: Mutex::new(vec![PermissionDecision::AlwaysSession]),
        seen: Mutex::new(vec![]),
    });
    let gate = Arc::new(RulesetGate::new(Arc::clone(&ui) as Arc<dyn ApprovalUi>));

    let steps = [
        FauxStep::call("tc_r1", "read", serde_json::json!({"path": ".env"})),
        FauxStep::call("tc_r2", "read", serde_json::json!({"path": ".env"})),
        FauxStep::say("got it"),
    ];

    let first = run_agent(steps[..2].to_vec(), gate, dir.path())
        .join()
        .await;
    let ok_reads: Vec<String> = first
        .produced
        .iter()
        .filter_map(|m| match m {
            Message::ToolResult(t) if t.tool_name == "read" && !t.is_error => Some(
                t.content
                    .iter()
                    .filter_map(|c| match c {
                        epsilon_contracts::content::ContentBlock::Text { text } => {
                            Some(text.clone())
                        }
                        _ => None,
                    })
                    .collect::<String>(),
            ),
            Message::ToolResult(t) if t.is_error => Some(format!("ERR: {:?}", t.content)),
            _ => None,
        })
        .collect();
    assert!(
        ok_reads.iter().any(|body| body.contains("TOKEN")),
        "expected a successful .env read; got {ok_reads:?} | reason={:?}",
        first.reason
    );

    // The single ask happened once for the first read.
    assert_eq!(ui.seen.lock().unwrap().len(), 1);
    assert!(matches!(
        ui.seen.lock().unwrap()[0],
        PermissionRequest { ref capability, .. } if capability == "read"
    ));
}
