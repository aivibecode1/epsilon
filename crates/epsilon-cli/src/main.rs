//! Epsilon CLI.
//!
//! Modes:
//! - default            : scripted M5-style demo against the faux provider
//! - `-p "<task>"`      : print mode — one headless run over the CURRENT directory using
//!   the REAL built-in tools and the default ruleset (asks auto-approved until M9 TUI)
//! - `--script <file>`  : faux script JSON for `-p` (offline/testing)
//! - `--workspace <dir>`: override workspace root (default: cwd)

use std::sync::Arc;

use epsilon_contracts::agent_event::AgentEvent;
use epsilon_core::{LoopLimits, ToolRegistry};
use epsilon_llm::{FauxProvider, FauxStep, FauxToolCall, PanicGuard};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut task: Option<String> = None;
    let mut script_path: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-p" | "--print" => {
                i += 1;
                task = Some(args.get(i).cloned().expect("-p requires a task string"));
            }
            "--script" => {
                i += 1;
                script_path = Some(args.get(i).cloned().expect("--script requires a path"));
            }
            other => {
                eprintln!("unknown argument: {other}\nusage: epsilon [-p \"task\"] [--script f] ");
                std::process::exit(2);
            }
        }
        i += 1;
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let code = match task {
        Some(task) => runtime.block_on(print_mode(task, script_path)),
        None => runtime.block_on(demo_mode()),
    };
    std::process::exit(code);
}

/// Headless single-prompt run over the current directory.
async fn print_mode(task: String, script_path: Option<String>) -> i32 {
    let workspace = std::env::current_dir().unwrap_or_default();

    // Offline by default: a faux script drives the model so `-p` is reproducible without
    // keys. Real providers arrive with serve/TUI modes behind --features real.
    let steps: Vec<FauxStep> = match &script_path {
        Some(p) => {
            let raw = std::fs::read_to_string(p).expect("read script");
            serde_json::from_str(&raw).expect("invalid script json")
        }
        None => vec![
            FauxStep::call(
                "tc_1",
                "bash",
                serde_json::json!({"command": "echo print-mode"}),
            ),
            FauxStep::say("Print-mode run complete."),
        ],
    };

    let provider = FauxProvider::new(steps);
    let registry = epsilon_tools::builtin_registry();
    let gate = epsilon_safety::RulesetGate::new(Arc::new(epsilon_safety::AutoApprove));

    let agent = epsilon_core::Agent::new(
        LoopLimits::default(),
        Arc::new(PanicGuard(provider)),
        Arc::new(registry),
        Arc::new(gate),
        workspace.clone(),
    );
    agent.attach_session_store(Arc::new(std::sync::Mutex::new(
        epsilon_session::SessionStore::open(workspace.join(".epsilon/sessions/cli.jsonl"))
            .expect("open session"),
    )));

    let prompt =
        epsilon_contracts::message::Message::User(epsilon_contracts::message::UserMessage {
            id: epsilon_contracts::new_id("msg"),
            timestamp_ms: 0,
            content: vec![epsilon_contracts::content::UserContent::Text { text: task }],
        });

    match agent.run(prompt).await {
        Ok(outcome) => {
            // Print only the FINAL assistant text (pi print-mode behavior).
            for m in outcome.produced.iter().rev() {
                if let epsilon_contracts::message::Message::Assistant(a) = m
                    && a.stop_reason == epsilon_contracts::message::StopReason::EndTurn
                    && !a.text().is_empty()
                {
                    println!("{}", a.text());
                    return 0;
                }
            }
            eprintln!("no final assistant text produced");
            1
        }
        Err(e) => {
            eprintln!("run failed: {e}");
            1
        }
    }
}

/// The classic scripted demo (M5), kept as the zero-argument experience.
async fn demo_mode() -> i32 {
    let script = vec![
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
    ];

    let provider = FauxProvider::new(script);
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(epsilon_core::testing::EchoTool::new()));
    registry.register(Arc::new(epsilon_core::testing::FailTool::new()));

    let agent = epsilon_core::Agent::new(
        LoopLimits::default(),
        Arc::new(PanicGuard(provider)),
        Arc::new(registry),
        Arc::new(epsilon_core::AutoAllow),
        std::env::temp_dir(),
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

    let run_task = tokio::spawn(async move { agent.run(prompt).await });

    while let Ok(ev) = rx.recv().await {
        match &*ev {
            AgentEvent::AgentStart => println!("\n=== run start ==="),
            AgentEvent::TurnStart { turn_index } => println!("-- turn {turn_index} --"),
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
        Ok(outcome) => {
            println!(
                "\nfinal reason: {:?}\nmessages produced: {}\ntokens in/out: {}/{}",
                outcome.reason,
                outcome.produced.len(),
                outcome.usage.input,
                outcome.usage.output,
            );
            0
        }
        Err(e) => {
            eprintln!("run failed: {e}");
            1
        }
    }
}
