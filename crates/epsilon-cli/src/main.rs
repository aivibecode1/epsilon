//! Epsilon CLI — currently the M5 milestone demo: runs a fully scripted session against the
//! faux provider and prints the live event stream.
//!
//! ```text
//! cargo run -p epsilon-cli
//! ```

use std::sync::Arc;

use epsilon_contracts::agent_event::AgentEvent;
use epsilon_core::testing::{EchoTool, FailTool};
use epsilon_core::{LoopLimits, ToolRegistry};
use epsilon_llm::{FauxProvider, FauxStep, FauxToolCall, PanicGuard};

fn script() -> Vec<FauxStep> {
    vec![
        FauxStep::ToolCalls {
            text: Some("I'll inspect the two files, then write the fix.".into()),
            calls: vec![
                FauxToolCall {
                    id: "tc_1".into(),
                    name: "echo".into(),
                    arguments: serde_json::json!({"text": "reading src/auth.rs"}),
                },
                FauxToolCall {
                    id: "tc_2".into(),
                    name: "echo".into(),
                    arguments: serde_json::json!({"text": "reading tests/auth.rs"}),
                },
            ],
        },
        FauxStep::call("tc_3", "fail", serde_json::json!({})),
        FauxStep::say("One check failed; here is the summary of what I found."),
    ]
}

#[tokio::main]
async fn main() {
    let provider = FauxProvider::new(script());
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(EchoTool::new()));
    registry.register(Arc::new(FailTool::new()));

    let agent = epsilon_core::Agent::new(
        LoopLimits::default(),
        Arc::new(PanicGuard(provider)),
        Arc::new(registry),
        Arc::new(epsilon_core::AutoAllow),
    );
    let mut rx = agent.subscribe();

    let prompt =
        epsilon_contracts::message::Message::User(epsilon_contracts::message::UserMessage {
            id: epsilon_contracts::new_id("msg"),
            timestamp_ms: 0,
            content: vec![epsilon_contracts::content::UserContent::Text {
                text: "fix the flaky auth tests".into(),
            }],
        });

    let agent_task = tokio::spawn(async move { agent.run(prompt).await });

    // Print the live event stream (this is what the TUI will consume in M9).
    while let Ok(ev) = rx.recv().await {
        match &*ev {
            AgentEvent::AgentStart => println!("\n=== run start ==="),
            AgentEvent::TurnStart { turn_index } => println!("-- turn {turn_index} --"),
            AgentEvent::MessageEnd { message } => match &**message {
                epsilon_contracts::message::Message::Assistant(a) => {
                    println!("assistant> {}", a.text());
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

    match agent_task.await.expect("run task") {
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
