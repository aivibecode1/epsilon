//! M72: the batch planner, through the real loop.
//!
//! The planner's promises are about *time* and *order*, so these scenarios observe exactly that:
//! a barrier tool that only completes if a peer is running at the same moment, and a recorder
//! that logs the order calls actually executed in. The unit tests in `exec.rs` cover the
//! classification itself; these cover the claim that the classification is obeyed.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

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

/// Records the order calls were entered and left, and where each call waited.
#[derive(Default)]
struct Log {
    started: Mutex<Vec<String>>,
    finished: Mutex<Vec<String>>,
}

impl Log {
    fn started(&self) -> Vec<String> {
        self.started.lock().unwrap().clone()
    }
    fn entered(&self, label: String) {
        self.started.lock().unwrap().push(label);
    }
    fn left(&self, label: String) {
        self.finished.lock().unwrap().push(label);
    }
}

/// Counts how many calls are inside the tool at the same moment, and remembers the high-water
/// mark: the one measurement that distinguishes "these ran together" from "these ran one at a
/// time", with nothing to be fooled by.
///
/// (A `tokio::sync::Barrier` looks like the obvious primitive and is not: a waiter that times out
/// is still counted as an arrival, so it releases the *next* call and a serialized batch reports
/// success. That version of this test passed under full serialization.)
#[derive(Default)]
struct Overlap {
    in_flight: std::sync::atomic::AtomicUsize,
    peak: std::sync::atomic::AtomicUsize,
}

impl Overlap {
    fn enter(&self) -> usize {
        use std::sync::atomic::Ordering;
        let n = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak.fetch_max(n, Ordering::SeqCst);
        n
    }
    fn leave(&self) {
        use std::sync::atomic::Ordering;
        self.in_flight.fetch_sub(1, Ordering::SeqCst);
    }
    fn peak(&self) -> usize {
        self.peak.load(std::sync::atomic::Ordering::SeqCst)
    }
}

/// Holds itself open long enough that calls which were allowed to overlap demonstrably do.
struct OverlapTool {
    descriptor: ToolDescriptor,
    overlap: Arc<Overlap>,
    log: Arc<Log>,
}

#[async_trait]
impl Tool for OverlapTool {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    fn resource_key(&self, args: &serde_json::Value, _base: &std::path::Path) -> Option<PathBuf> {
        Some(PathBuf::from(args.get("path")?.as_str()?))
    }

    async fn execute(
        &self,
        call: ToolInvocation,
        _ctx: ToolContext,
        _on_update: ToolUpdateSink,
    ) -> Result<ToolOutput, ToolFailure> {
        let label = call
            .args
            .get("label")
            .and_then(|v| v.as_str())
            .unwrap_or("?")
            .to_string();
        let now = self.overlap.enter();
        self.log.entered(label.clone());
        tokio::time::sleep(Duration::from_millis(150)).await;
        self.overlap.leave();
        self.log.left(label.clone());
        Ok(ToolOutput {
            content: vec![ContentBlock::text(format!(
                "{label} ran with {now} call(s) in flight"
            ))],
            details: serde_json::json!({}),
            usage: None,
            terminate: false,
        })
    }
}

/// Records entry order, so call-order preservation can be asserted directly.
struct OrderTool {
    descriptor: ToolDescriptor,
    log: Arc<Log>,
}

#[async_trait]
impl Tool for OrderTool {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    fn resource_key(&self, args: &serde_json::Value, _base: &std::path::Path) -> Option<PathBuf> {
        Some(PathBuf::from(args.get("path")?.as_str()?))
    }

    async fn execute(
        &self,
        call: ToolInvocation,
        _ctx: ToolContext,
        _on_update: ToolUpdateSink,
    ) -> Result<ToolOutput, ToolFailure> {
        let label = call
            .args
            .get("label")
            .and_then(|v| v.as_str())
            .unwrap_or("?")
            .to_string();
        // A real edit takes time; this makes the order observable rather than a race.
        tokio::time::sleep(Duration::from_millis(60)).await;
        self.log.entered(label.clone());
        Ok(ToolOutput {
            content: vec![ContentBlock::text(label)],
            details: serde_json::json!({}),
            usage: None,
            terminate: false,
        })
    }
}

fn tool_descriptor(name: &str, mode: ExecutionMode) -> ToolDescriptor {
    ToolDescriptor {
        name: name.into(),
        label: name.into(),
        description: "test tool".into(),
        parameters_schema: serde_json::json!({
            "type": "object",
            "properties": {"label": {"type": "string"}, "path": {"type": "string"}},
            "required": ["path"]
        }),
        execution_mode: mode,
    }
}

/// The text of every tool result in the transcript, in order.
fn tool_results(agent: &epsilon_core::Agent) -> Vec<String> {
    agent
        .history()
        .iter()
        .filter_map(|m| match m {
            Message::ToolResult(t) => Some(
                t.content
                    .iter()
                    .filter_map(|c| match c {
                        ContentBlock::Text { text } => Some(text.clone()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
            _ => None,
        })
        .collect()
}

fn agent_with(tools: Vec<Arc<dyn Tool>>, steps: Vec<FauxStep>) -> Arc<epsilon_core::Agent> {
    let mut registry = ToolRegistry::new();
    for t in tools {
        registry.register(t);
    }
    epsilon_core::Agent::new(
        LoopLimits::default(),
        Arc::new(epsilon_llm::PanicGuard(FauxProvider::new(steps))),
        Arc::new(registry),
        Arc::new(AutoAllow),
        std::env::temp_dir(),
    )
}

fn faux_call(id: &str, name: &str, label: &str, path: &str) -> epsilon_llm::FauxToolCall {
    epsilon_llm::FauxToolCall {
        id: id.into(),
        name: name.into(),
        arguments: serde_json::json!({"label": label, "path": path}),
    }
}

/// One assistant message asking for several tools — a batch.
fn batch(calls: Vec<epsilon_llm::FauxToolCall>) -> FauxStep {
    FauxStep::ToolCalls { text: None, calls }
}

#[tokio::test]
async fn edits_to_different_files_run_at_the_same_time() {
    let log = Arc::new(Log::default());
    let overlap = Arc::new(Overlap::default());
    let mk = |name: &str| {
        Arc::new(OverlapTool {
            descriptor: tool_descriptor(name, ExecutionMode::Sequential),
            overlap: Arc::clone(&overlap),
            log: Arc::clone(&log),
        }) as Arc<dyn Tool>
    };
    let agent = agent_with(
        vec![mk("edit_a"), mk("edit_b")],
        vec![
            batch(vec![
                faux_call("c1", "edit_a", "first", "/ws/a.rs"),
                faux_call("c2", "edit_b", "second", "/ws/b.rs"),
            ]),
            FauxStep::say("both done"),
        ],
    );

    agent.run(user("edit both files")).await.expect("run");

    let results = tool_results(&agent);
    assert_eq!(results.len(), 2, "{results:#?}");
    assert_eq!(
        overlap.peak(),
        2,
        "two files, two calls, both in flight at once: {results:#?}"
    );
    // And the results still come back in the order the model asked for them.
    assert!(results[0].starts_with("first"), "{results:#?}");
    assert!(results[1].starts_with("second"), "{results:#?}");
}

#[tokio::test]
async fn edits_to_the_same_file_run_one_after_the_other() {
    let order = Arc::new(Log::default());
    let mk = || {
        Arc::new(OrderTool {
            descriptor: tool_descriptor("edit_same", ExecutionMode::Sequential),
            log: Arc::clone(&order),
        }) as Arc<dyn Tool>
    };
    let agent = agent_with(
        vec![mk()],
        vec![
            batch(vec![
                faux_call("c1", "edit_same", "first", "/ws/a.rs"),
                faux_call("c2", "edit_same", "second", "/ws/a.rs"),
                faux_call("c3", "edit_same", "third", "/ws/a.rs"),
            ]),
            FauxStep::say("done"),
        ],
    );

    agent
        .run(user("three edits to one file"))
        .await
        .expect("run");

    assert_eq!(
        order.started(),
        vec!["first", "second", "third"],
        "same-file calls must run in the order the model asked for them"
    );
    // And the results still come back in model order (the DSH rule the pool already followed).
    let results = tool_results(&agent);
    assert_eq!(results, vec!["first", "second", "third"]);
}

#[tokio::test]
async fn a_shell_call_keeps_the_whole_batch_serialized() {
    // `bash` declares no resource, so the barrier rule stands: a command can touch anything.
    //
    // Two calls to two *different* files are the control: on their own they overlap (asserted
    // above). With a shell call in the same batch they must not, so the peak stays at one. A
    // single-call version of this test could not tell serialization from a missing partner.
    let log = Arc::new(Log::default());
    let overlap = Arc::new(Overlap::default());
    let mk = |name: &str| {
        Arc::new(OverlapTool {
            descriptor: tool_descriptor(name, ExecutionMode::Sequential),
            overlap: Arc::clone(&overlap),
            log: Arc::clone(&log),
        }) as Arc<dyn Tool>
    };
    let shell = Arc::new(ShellaLike {
        descriptor: tool_descriptor("shell_like", ExecutionMode::Sequential),
    }) as Arc<dyn Tool>;
    let agent = agent_with(
        vec![mk("edit_x"), mk("edit_y"), shell],
        vec![
            batch(vec![
                faux_call("c1", "edit_x", "first", "/ws/a.rs"),
                faux_call("c2", "edit_y", "second", "/ws/b.rs"),
                epsilon_llm::FauxToolCall {
                    id: "c3".into(),
                    name: "shell_like".into(),
                    arguments: serde_json::json!({"label": "sh", "path": ""}),
                },
            ]),
            FauxStep::say("done"),
        ],
    );

    agent.run(user("two edits and a shell")).await.expect("run");

    assert_eq!(
        overlap.peak(),
        1,
        "with a shell call in the batch nothing may overlap: {:#?}",
        tool_results(&agent)
    );
    assert_eq!(
        log.started(),
        vec!["first", "second"],
        "and they ran in the order the model asked for"
    );
}

/// A shell-shaped tool: `Sequential`, and it declares no resource at all.
struct ShellaLike {
    descriptor: ToolDescriptor,
}

#[async_trait]
impl Tool for ShellaLike {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    async fn execute(
        &self,
        _call: ToolInvocation,
        _ctx: ToolContext,
        _on_update: ToolUpdateSink,
    ) -> Result<ToolOutput, ToolFailure> {
        Ok(ToolOutput {
            content: vec![ContentBlock::text("shell ran")],
            details: serde_json::json!({}),
            usage: None,
            terminate: false,
        })
    }
}
