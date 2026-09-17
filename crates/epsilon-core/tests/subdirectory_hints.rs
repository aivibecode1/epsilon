//! M73: subdirectory instruction hints, through the real loop.
//!
//! The unit tests in `prompt.rs` cover the loader's rules. What they cannot show is the thing
//! that matters to a user: that working in `crates/x/` puts `crates/x/AGENTS.md` in front of the
//! model on the *next* request — and that it was not there before, which is what makes the
//! feature observable rather than assumed.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use epsilon_contracts::content::{ContentBlock, UserContent};
use epsilon_contracts::message::{Message, UserMessage};
use epsilon_contracts::tool::{
    ExecutionMode, Tool, ToolContext, ToolDescriptor, ToolFailure, ToolInvocation, ToolOutput,
    ToolUpdateSink,
};
use epsilon_core::{AutoAllow, LoopLimits, ToolRegistry};
use epsilon_llm::{FauxProvider, FauxStep};

fn user(text: &str) -> Message {
    Message::User(UserMessage {
        id: epsilon_contracts::new_id("msg"),
        timestamp_ms: 0,
        content: vec![UserContent::Text {
            text: text.to_string(),
        }],
    })
}

/// A stand-in for `write`: it acts on the `path` argument and declares it, which is the only
/// thing the hint mechanism consumes.
struct PathTool {
    descriptor: ToolDescriptor,
}

#[async_trait]
impl Tool for PathTool {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    fn resource_key(&self, args: &serde_json::Value, base: &std::path::Path) -> Option<PathBuf> {
        let raw = args.get("path")?.as_str()?;
        let joined = if std::path::Path::new(raw).is_absolute() {
            PathBuf::from(raw)
        } else {
            base.join(raw)
        };
        Some(joined)
    }

    async fn execute(
        &self,
        _call: ToolInvocation,
        _ctx: ToolContext,
        _on_update: ToolUpdateSink,
    ) -> Result<ToolOutput, ToolFailure> {
        Ok(ToolOutput {
            content: vec![ContentBlock::text("ok")],
            details: serde_json::json!({}),
            usage: None,
            terminate: false,
        })
    }
}

struct Workspace {
    _dir: tempfile::TempDir,
    root: PathBuf,
}

fn workspace_with(files: &[(&str, &str)]) -> Workspace {
    let dir = tempfile::tempdir().unwrap();
    for (rel, content) in files {
        let p = dir.path().join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, content).unwrap();
    }
    Workspace {
        root: dir.path().to_path_buf(),
        _dir: dir,
    }
}

/// The provider is moved into the guard, so its record is read through a handle taken first.
type Requests = Arc<std::sync::Mutex<Vec<epsilon_llm::LlmContext>>>;

fn agent(ws: &Workspace, steps: Vec<FauxStep>) -> (Arc<epsilon_core::Agent>, Requests) {
    let provider = FauxProvider::new(steps);
    let requests = provider.recorded_requests_handle();
    let mut registry = ToolRegistry::new();
    for name in ["write_like", "read_like"] {
        registry.register(Arc::new(PathTool {
            descriptor: ToolDescriptor {
                name: name.into(),
                label: name.into(),
                description: "test tool with a declared resource".into(),
                parameters_schema: serde_json::json!({
                    "type": "object",
                    "properties": {"path": {"type": "string"}},
                    "required": ["path"]
                }),
                execution_mode: ExecutionMode::Sequential,
            },
        }));
    }
    let agent = epsilon_core::Agent::new(
        LoopLimits::default(),
        Arc::new(epsilon_llm::PanicGuard(provider)),
        Arc::new(registry),
        Arc::new(AutoAllow),
        ws.root.clone(),
    );
    (agent, requests)
}

/// A snapshot of what the provider was asked, oldest first.
fn recorded(requests: &Requests) -> Vec<epsilon_llm::LlmContext> {
    requests.lock().unwrap().clone()
}

#[tokio::test]
async fn working_in_a_subdirectory_puts_its_instructions_in_the_next_request() {
    let ws = workspace_with(&[
        ("AGENTS.md", "root conventions: use tabs"),
        (
            "crates/x/AGENTS.md",
            "crate x conventions: run the crate's tests",
        ),
        (
            "crates/y/AGENTS.md",
            "crate y conventions: never touch this",
        ),
    ]);
    let (agent, requests) = agent(
        &ws,
        vec![
            FauxStep::call(
                "c1",
                "write_like",
                serde_json::json!({"path": "crates/x/src/lib.rs"}),
            ),
            FauxStep::say("done"),
        ],
    );

    agent
        .run(user("add a function to crate x"))
        .await
        .expect("run");

    let reqs = recorded(&requests);
    assert_eq!(
        reqs.len(),
        2,
        "two provider calls: the tool turn and the reply after it"
    );

    // The first request cannot know where the run will end up.
    assert!(
        !reqs[0].system_prompt.contains("crate x conventions"),
        "the hint must follow the work, not precede it"
    );
    assert!(
        reqs[0].system_prompt.contains("root conventions"),
        "the root file is loaded from the start, as before"
    );

    // The second request carries the crate's own conventions, labelled by path.
    let second = &reqs[1].system_prompt;
    assert!(
        second.contains("crates/x/AGENTS.md"),
        "the hint names the file it came from"
    );
    assert!(
        second.contains("crate x conventions"),
        "and its content is there to be followed"
    );
    // Untouched crates stay out of the prompt entirely.
    assert!(
        !second.contains("crate y conventions"),
        "a directory nobody worked in contributes nothing"
    );
    // And the root file is not repeated by the hint loader (that would double every prompt).
    assert_eq!(
        second.matches("root conventions").count(),
        1,
        "the root file appears exactly once"
    );
}

#[tokio::test]
async fn a_second_turn_in_the_same_directory_does_not_duplicate_the_hint() {
    let ws = workspace_with(&[
        ("AGENTS.md", "root"),
        ("crates/x/AGENTS.md", "crate x conventions"),
    ]);
    let (agent, requests) = agent(
        &ws,
        vec![
            FauxStep::call(
                "c1",
                "write_like",
                serde_json::json!({"path": "crates/x/src/lib.rs"}),
            ),
            FauxStep::call(
                "c2",
                "write_like",
                serde_json::json!({"path": "crates/x/src/other.rs"}),
            ),
            FauxStep::say("done"),
        ],
    );

    agent.run(user("work in crate x")).await.expect("run");

    let reqs = recorded(&requests);
    let last = &reqs.last().expect("at least one request").system_prompt;
    assert_eq!(
        last.matches("crate x conventions").count(),
        1,
        "the same directory's hint is included once, however many calls touched it"
    );
    // The prompt does not grow between the second and third request: same hints, same bytes.
    assert_eq!(
        reqs[1].system_prompt, reqs[2].system_prompt,
        "a byte-stable prompt is the cache discipline (ADR-008)"
    );
}

#[tokio::test]
async fn a_run_that_touches_nothing_never_mentions_a_hint() {
    let ws = workspace_with(&[
        ("AGENTS.md", "root"),
        ("crates/x/AGENTS.md", "crate x conventions"),
    ]);
    let (agent, requests) = agent(&ws, vec![FauxStep::say("no tools needed")]);

    agent.run(user("just answer")).await.expect("run");

    let reqs = recorded(&requests);
    assert_eq!(reqs.len(), 1);
    assert!(
        !reqs[0].system_prompt.contains("crate x conventions"),
        "nothing was touched"
    );
}

#[tokio::test]
async fn a_directory_outside_the_workspace_contributes_nothing() {
    let ws = workspace_with(&[("AGENTS.md", "root")]);
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("AGENTS.md"), "outside conventions").unwrap();

    let (agent, requests) = agent(
        &ws,
        vec![
            FauxStep::call(
                "c1",
                "write_like",
                serde_json::json!({"path": outside.path().join("file.rs").to_string_lossy()}),
            ),
            FauxStep::say("done"),
        ],
    );

    agent.run(user("write outside")).await.expect("run");

    let reqs = recorded(&requests);
    let last = &reqs.last().expect("a request").system_prompt;
    assert!(
        !last.contains("outside conventions"),
        "instructions from outside the workspace are not project context"
    );
}
