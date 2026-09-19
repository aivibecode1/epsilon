//! The identity overlays, through the real loop.
//!
//! The unit tests in `prompt.rs` cover what the loader reads and where the text lands in the
//! assembled prompt. What they cannot show is the thing a user notices: that a written
//! `.epsilon/SYSTEM.md` changes the request the provider actually receives. The gap this
//! closes was a trust prompt with nothing behind it — the file was detected, gated, and then
//! never read — so the honest test is on the wire, not on the loader.

use std::sync::Arc;

use epsilon_contracts::content::UserContent;
use epsilon_contracts::message::{Message, UserMessage};
use epsilon_contracts::tool::{TaskHost, TaskRequest};
use epsilon_core::prompt::SystemOverlay;
use epsilon_core::{Agent, AutoAllow, LoopLimits, ToolRegistry};
use epsilon_llm::{FauxProvider, FauxStep, PanicGuard};

fn user(text: &str) -> Message {
    Message::User(UserMessage {
        id: epsilon_contracts::new_id("msg"),
        timestamp_ms: 0,
        content: vec![UserContent::Text {
            text: text.to_string(),
        }],
    })
}

/// The provider is moved into the guard, so its record is read through a handle taken first.
type Requests = Arc<std::sync::Mutex<Vec<epsilon_llm::LlmContext>>>;

fn agent(workspace: &std::path::Path, steps: Vec<FauxStep>) -> (Arc<Agent>, Requests) {
    let provider = FauxProvider::new(steps);
    let requests = provider.recorded_requests_handle();
    let agent = Agent::new(
        LoopLimits::default(),
        Arc::new(PanicGuard(provider)),
        Arc::new(ToolRegistry::new()),
        Arc::new(AutoAllow),
        workspace.to_path_buf(),
    );
    (agent, requests)
}

/// The shape a frontend attaches after `prompt::load_system_overlay` read the files.
fn overlay(custom: Option<&str>, append: Option<&str>) -> SystemOverlay {
    SystemOverlay {
        custom_path: custom.map(|_| ".epsilon/SYSTEM.md".to_string()),
        custom: custom.map(str::to_string),
        append_path: append.map(|_| ".epsilon/APPEND_SYSTEM.md".to_string()),
        append: append.map(str::to_string),
    }
}

fn recorded(requests: &Requests) -> Vec<epsilon_llm::LlmContext> {
    requests.lock().unwrap().clone()
}

#[tokio::test]
async fn a_workspace_contract_reaches_the_request_and_replaces_the_built_in_one() {
    let dir = tempfile::tempdir().unwrap();
    let (agent, requests) = agent(dir.path(), vec![FauxStep::say("ok")]);
    agent.set_system_overlay(overlay(
        Some("epsilon-house-rules"),
        Some("epsilon-house-style"),
    ));

    agent.run(user("hello")).await.expect("run");

    let reqs = recorded(&requests);
    let prompt = &reqs[0].system_prompt;
    assert!(prompt.contains("epsilon-house-rules"), "{prompt}");
    assert!(prompt.contains("epsilon-house-style"), "{prompt}");
    assert!(
        !prompt.contains("Epsilon Agent Operating Contract"),
        "the workspace's own contract replaced the built-in one: {prompt}"
    );
    // Nothing is loaded from the workspace's files here: the overlay is the frontend's
    // decision, and this test is about what the loop does with it once attached.
    assert!(
        !prompt.contains("<project_context>"),
        "a bare temp workspace has no project instructions: {prompt}"
    );
}

#[tokio::test]
async fn the_overlay_is_byte_stable_across_turns() {
    // ADR-008: the identity tier stays byte-identical for a session lifetime, which is the
    // whole reason a user-supplied file is read once at composition instead of per turn.
    let dir = tempfile::tempdir().unwrap();
    let (agent, requests) = agent(dir.path(), vec![FauxStep::say("one"), FauxStep::say("two")]);
    agent.set_system_overlay(overlay(Some("epsilon-house-rules"), None));

    agent.run(user("first")).await.expect("first run");
    agent.run(user("second")).await.expect("second run");

    let reqs = recorded(&requests);
    assert!(reqs.len() >= 2, "{reqs:#?}");
    assert_eq!(
        reqs[0].system_prompt, reqs[1].system_prompt,
        "the second turn must not move the cached prefix"
    );
}

#[tokio::test]
async fn a_delegated_child_runs_under_the_same_contract_as_its_parent() {
    // The workspace's contract is a property of the workspace, not of the top-level session:
    // a child that fell back to the built-in contract would be a different agent than the one
    // the task was delegated from.
    let dir = tempfile::tempdir().unwrap();
    let (agent, requests) = agent(dir.path(), vec![FauxStep::say("child done")]);
    agent.set_system_overlay(overlay(Some("epsilon-house-rules"), None));

    let host = agent.clone() as Arc<dyn TaskHost>;
    host.run_task(TaskRequest {
        prompt: "do the thing".into(),
        description: "overlay probe".into(),
        subagent_type: "default".into(),
        background: false,
        resume: None,
        model: None,
    })
    .await
    .expect("delegation");

    let reqs = recorded(&requests);
    let child = reqs.last().expect("the child's request");
    assert!(
        child.system_prompt.contains("epsilon-house-rules"),
        "the child ran without the parent's contract: {}",
        child.system_prompt
    );
}
