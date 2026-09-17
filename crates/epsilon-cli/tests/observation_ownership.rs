//! Real Agent/task composition with native filesystem tools and an offline provider.

use std::sync::Arc;

use epsilon_contracts::content::UserContent;
use epsilon_contracts::message::{Message, UserMessage};
use epsilon_contracts::tool::{TaskOutcome, TaskRequest};
use epsilon_core::{Agent, AutoAllow, LoopLimits, ToolRegistry};
use epsilon_llm::{FauxProvider, FauxStep};
use epsilon_tools::{ReadTool, WriteTool};

fn prompt() -> Message {
    Message::User(UserMessage {
        id: epsilon_contracts::new_id("msg"),
        timestamp_ms: 0,
        content: vec![UserContent::Text { text: "go".into() }],
    })
}

fn agent(root: &std::path::Path, steps: Vec<FauxStep>) -> Arc<Agent> {
    let mut tools = ToolRegistry::new();
    tools.register(Arc::new(ReadTool::new()));
    tools.register(Arc::new(WriteTool::new()));
    Agent::new(
        LoopLimits {
            verification_nudges: 0,
            ..LoopLimits::default()
        },
        Arc::new(FauxProvider::new(steps)),
        Arc::new(tools),
        Arc::new(AutoAllow),
        root.to_path_buf(),
    )
}

fn read(id: &str) -> FauxStep {
    FauxStep::call(id, "read", serde_json::json!({"path": "file.txt"}))
}

fn write(id: &str, text: &str) -> FauxStep {
    FauxStep::call(
        id,
        "write",
        serde_json::json!({"path": "file.txt", "content": text}),
    )
}

fn request() -> TaskRequest {
    TaskRequest {
        prompt: "work on file.txt".into(),
        description: "observation fixture".into(),
        subagent_type: "default".into(),
        background: false,
        resume: None,
        model: None,
    }
}

fn assert_refusal(agent: &Agent, call_id: &str, reason: &str) {
    let result = agent
        .history()
        .into_iter()
        .find_map(|m| match m {
            Message::ToolResult(r) if r.tool_call_id == call_id => Some(r),
            _ => None,
        })
        .expect("tool result must exist");
    assert!(result.is_error);
    assert!(result.content.iter().any(|c| matches!(c,
        epsilon_contracts::content::ContentBlock::Text { text } if text.contains(reason))));
}

#[tokio::test]
async fn parent_and_task_child_do_not_share_file_observations() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("file.txt");
    std::fs::write(&path, "original").unwrap();
    // Parent and task children share this deterministic provider queue, just as
    // production children share their parent's provider. Runs are sequential.
    let parent = agent(
        dir.path(),
        vec![
            read("parent_read"),
            FauxStep::say("parent read complete"),
            write("child_blind", "must not land"),
            FauxStep::say("blind child done"),
            read("child_read"),
            write("child_write", "child replacement"),
            // Child default limits allow two verification nudges after a write.
            FauxStep::say("child done"),
            FauxStep::say("no check available"),
            FauxStep::say("finished"),
            write("parent_stale", "must not land"),
            FauxStep::say("parent done"),
            read("parent_reread"),
            write("parent_fresh", "after reread"),
            FauxStep::say("complete"),
        ],
    );
    parent.run(prompt()).await.unwrap();
    assert!(matches!(
        parent.delegate_task(request()).await.unwrap(),
        TaskOutcome::Completed(_)
    ));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "original");
    assert!(matches!(
        parent.delegate_task(request()).await.unwrap(),
        TaskOutcome::Completed(_)
    ));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "child replacement");
    parent.run(prompt()).await.unwrap();
    assert_refusal(&parent, "parent_stale", "changed on disk");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "child replacement");
    parent.run(prompt()).await.unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "after reread");
}

#[tokio::test]
async fn restored_history_does_not_restore_file_write_authority() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("file.txt");
    std::fs::write(&path, "original").unwrap();
    let agent = agent(
        dir.path(),
        vec![
            read("read"),
            FauxStep::say("read complete"),
            write("after_restore", "must not land"),
            FauxStep::say("complete"),
        ],
    );
    agent.run(prompt()).await.unwrap();
    agent.restore_history(agent.history());
    agent.run(prompt()).await.unwrap();
    assert_refusal(&agent, "after_restore", "has not been read");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "original");
}
