//! Epsilon CLI — M6 demo: runs a scripted session with REAL built-in tools
//! (write → read → grep) inside a temporary workspace, gated by the ruleset engine.
//!
//! ```text
//! cargo run -p epsilon-cli
//! ```

use std::sync::Arc;

use epsilon_contracts::agent_event::AgentEvent;
use epsilon_core::LoopLimits;
use epsilon_llm::{FauxProvider, FauxStep, FauxToolCall, PanicGuard};
use epsilon_safety::{AutoApprove, RulesetGate};

fn script() -> Vec<FauxStep> {
    vec![
        FauxStep::call(
            "tc_1",
            "write",
            serde_json::json!({
                "path": "notes/demo.txt",
                "content": "hello from epsilon\nline two\n"
            }),
        ),
        FauxStep::call(
            "tc_2",
            "read",
            serde_json::json!({"path": "notes/demo.txt"}),
        ),
        FauxStep::ToolCalls {
            text: Some("Now let me confirm the content is on disk.".into()),
            calls: vec![FauxToolCall {
                id: "tc_3".into(),
                name: "grep".into(),
                arguments: serde_json::json!({"pattern": "epsilon"}),
            }],
        },
        FauxStep::say(
            "Wrote notes/demo.txt, read it back, and grepped for 'epsilon' — \
             the file pipeline works end to end.",
        ),
    ]
}

#[tokio::main]
async fn main() {
    // Fresh scratch workspace per run; everything the agent touches lives here.
    let workspace = tempfile::tempdir().expect("tempdir");

    let provider = FauxProvider::new(script());
    let registry = epsilon_tools::builtin_registry();
    // Default ruleset + auto-approving UI so the demo is non-interactive;
    // swap AutoApprove for a TUI dialog implementation in M9.
    let gate = RulesetGate::new(Arc::new(AutoApprove));

    let agent = epsilon_core::Agent::new(
        LoopLimits::default(),
        Arc::new(PanicGuard(provider)),
        Arc::new(registry),
        Arc::new(gate),
        workspace.path().to_path_buf(),
    );
    let mut rx = agent.subscribe();

    let prompt =
        epsilon_contracts::message::Message::User(epsilon_contracts::message::UserMessage {
            id: epsilon_contracts::new_id("msg"),
            timestamp_ms: 0,
            content: vec![epsilon_contracts::content::UserContent::Text {
                text: "create and verify notes/demo.txt".into(),
            }],
        });

    let run_task = tokio::spawn(async move { agent.run(prompt).await });

    while let Ok(ev) = rx.recv().await {
        match &*ev {
            AgentEvent::AgentStart => println!("\n=== run start ==="),
            AgentEvent::TurnStart { turn_index } => println!("-- turn {turn_index} --"),
            AgentEvent::PermissionRequested { request } => {
                println!("permission ask> {} {}", request.capability, request.pattern);
            }
            AgentEvent::MessageEnd { message } => match &**message {
                epsilon_contracts::message::Message::Assistant(a) => {
                    if !a.text().is_empty() {
                        println!("assistant> {}", a.text());
                    }
                    for (_, name, _) in a.tool_calls() {
                        println!("tool-call> {name}");
                    }
                }
                epsilon_contracts::message::Message::ToolResult(t) => {
                    println!(
                        "tool-result[{}]> {}{}",
                        t.tool_name,
                        if t.is_error { "ERROR: " } else { "" },
                        t.content
                            .iter()
                            .filter_map(|c| match c {
                                epsilon_contracts::content::ContentBlock::Text { text } => {
                                    Some(text.clone())
                                }
                                _ => None,
                            })
                            .collect::<String>()
                    );
                }
                _ => {}
            },
            AgentEvent::AgentEnd { .. } => println!("=== run end ==="),
            _ => {}
        }
    }

    match run_task.await.expect("run task") {
        Ok(outcome) => println!(
            "\nfinal reason: {:?}\nmessages produced: {}\ntokens in/out: {}/{}",
            outcome.reason,
            outcome.produced.len(),
            outcome.usage.input,
            outcome.usage.output,
        ),
        Err(e) => eprintln!("run failed: {e}"),
    }
}
