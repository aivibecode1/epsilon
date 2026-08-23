//! Epsilon CLI.
//!
//! Modes (ADR-011: one session runtime beneath all frontends):
//! - default             : scripted M5-style demo against the faux provider
//! - `-p "<task>"`       : print mode — headless single run, real tools, final text to stdout
//! - `--tui ["task"]`    : interactive ratatui frontend; typing mid-run steers
//! - `--serve [--port N] ["task"]` : HTTP/SSE server for remote frontends
//! - `--script <file>`   : faux-script JSON driving ANY mode offline/reproducibly
//! - `--mcp <name>=<cmd>[:arg,arg…]` : attach an MCP stdio server (repeatable)
//! - `--workspace <dir>` : override workspace root (default: cwd)
//!
//! Offline-first: without `--script`, print/tui/serve use a small built-in faux script so
//! every mode is runnable without keys. Real providers plug in behind `--features real`.

use std::path::PathBuf;
use std::sync::Arc;

use epsilon_contracts::agent_event::AgentEvent;
use epsilon_contracts::content::UserContent;
use epsilon_contracts::message::{Message, StopReason, UserMessage};
use epsilon_contracts::new_id;
use epsilon_core::{Agent, LoopLimits, ToolRegistry};
use epsilon_llm::{FauxProvider, FauxStep, FauxToolCall, PanicGuard};
use epsilon_mcp::McpServerConfig;
use epsilon_safety::{ApprovalUi, ChannelApprovalUi, RulesetGate};

const USAGE: &str = "usage: epsilon [-p \"task\"] [--tui [task]] [--serve [--port N] [task]] \
[--script f] [--mcp name=cmd[:arg,arg]] [--workspace dir] [--sandbox auto|off] \
[--provider faux|real] [--model id]";

struct Args {
    mode: Mode,
    script_path: Option<String>,
    workspace: Option<PathBuf>,
    mcp_servers: Vec<McpServerConfig>,
    port: u16,
    confinement: epsilon_tools::Confinement,
    provider: Provider,
    model_id: String,
}

/// Model source: the offline faux provider (default) or a real genai-backed provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Provider {
    Faux,
    Real,
}

enum Mode {
    Demo,
    Print(String),
    Tui(Option<String>),
    Serve(Option<String>),
}

fn parse_args() -> Args {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let mut mode = Mode::Demo;
    let mut script_path = None;
    let mut workspace = None;
    let mut mcp_servers = Vec::new();
    let mut port: u16 = 8787;
    let mut confinement = epsilon_tools::Confinement::Auto;
    let mut provider = Provider::Faux;
    let mut model_id = "faux-1".to_string();
    let mut i = 0;
    while i < raw.len() {
        match raw[i].as_str() {
            "-p" | "--print" => {
                i += 1;
                let task = raw
                    .get(i)
                    .cloned()
                    .unwrap_or_else(|| die("-p requires a task string"));
                mode = Mode::Print(task);
            }
            "--tui" => {
                i += 1;
                let task = positional(&raw, &mut i);
                mode = Mode::Tui(task);
            }
            "--serve" => {
                i += 1;
                if raw.get(i).map(|a| a == "--port").unwrap_or(false) {
                    i += 1;
                    port = raw
                        .get(i)
                        .and_then(|v| v.parse().ok())
                        .unwrap_or_else(|| die("--port requires a number"));
                    i += 1;
                }
                let task = positional(&raw, &mut i);
                mode = Mode::Serve(task);
            }
            "--script" => {
                i += 1;
                script_path = Some(
                    raw.get(i)
                        .cloned()
                        .unwrap_or_else(|| die("--script requires a path")),
                );
            }
            "--workspace" => {
                i += 1;
                let dir = raw
                    .get(i)
                    .cloned()
                    .unwrap_or_else(|| die("--workspace requires a directory"));
                workspace = Some(PathBuf::from(dir));
            }
            "--sandbox" => {
                i += 1;
                let mode = raw
                    .get(i)
                    .cloned()
                    .unwrap_or_else(|| die("--sandbox requires auto|off"));
                confinement = match mode.as_str() {
                    "auto" => epsilon_tools::Confinement::Auto,
                    "off" => {
                        eprintln!(
                            "WARNING: OS confinement disabled (--sandbox off); commands run unconfined"
                        );
                        epsilon_tools::Confinement::Off
                    }
                    other => die(&format!("unknown --sandbox mode: {other} (want auto|off)")),
                };
            }
            "--provider" => {
                i += 1;
                match raw.get(i).map(String::as_str) {
                    Some("faux") => provider = Provider::Faux,
                    Some("real") => provider = Provider::Real,
                    other => die(&format!(
                        "unknown --provider: {:?} (want faux|real; real needs --features real + API keys in env)",
                        other
                    )),
                }
            }
            "--model" => {
                i += 1;
                model_id = raw
                    .get(i)
                    .cloned()
                    .unwrap_or_else(|| die("--model requires a model id (e.g. claude-sonnet-4-5)"));
            }
            "--port" => die("--port belongs to --serve"),
            a if a.starts_with("--mcp") => {
                let spec = a
                    .strip_prefix("--mcp")
                    .and_then(|r| r.strip_prefix('='))
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| {
                        i += 1;
                        raw.get(i)
                            .cloned()
                            .unwrap_or_else(|| die("--mcp needs name=cmd[:args]"))
                    });
                mcp_servers.push(parse_mcp_spec(&spec));
            }
            other => die(&format!("unknown argument: {other}\n{USAGE}")),
        }
        i += 1;
    }
    Args {
        mode,
        script_path,
        workspace,
        mcp_servers,
        port,
        confinement,
        provider,
        model_id,
    }
}

/// Optional positional value for a flag: consumed unless absent or another flag.
fn positional(raw: &[String], i: &mut usize) -> Option<String> {
    match raw.get(*i) {
        Some(v) if !v.starts_with('-') => {
            let v = v.clone();
            *i += 1;
            Some(v)
        }
        _ => None,
    }
}

/// `<name>=<command>[:arg,arg…]` — args are comma-separated inside one token (no spaces).
fn parse_mcp_spec(spec: &str) -> McpServerConfig {
    let (name, rest) = spec
        .split_once('=')
        .unwrap_or_else(|| die("--mcp expects name=cmd[:arg,arg]"));
    let (command, argstr) = match rest.split_once(':') {
        // Windows paths carry a drive colon ("C:\…"): only treat it as an arg separator
        // when what follows is not a single drive letter.
        Some((head, tail))
            if !(tail.len() >= 2 && tail.as_bytes()[1] == b'\\' && head.len() == 1) =>
        {
            (head, Some(tail))
        }
        _ => (rest, None),
    };
    McpServerConfig {
        name: name.to_string(),
        command: command.to_string(),
        args: argstr
            .map(|s| {
                s.split(',')
                    .filter(|a| !a.is_empty())
                    .map(String::from)
                    .collect()
            })
            .unwrap_or_default(),
    }
}

fn die(msg: &str) -> ! {
    eprintln!("{msg}\n{USAGE}");
    std::process::exit(2);
}

fn main() {
    let args = parse_args();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let code = runtime.block_on(dispatch(args));
    std::process::exit(code);
}

async fn dispatch(args: Args) -> i32 {
    let workspace = args
        .workspace
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let steps: Vec<FauxStep> = match &args.script_path {
        Some(p) => {
            let raw = std::fs::read_to_string(p).unwrap_or_else(|e| die(&format!("read {p}: {e}")));
            serde_json::from_str(&raw).unwrap_or_else(|e| die(&format!("invalid script json: {e}")))
        }
        None => default_steps(),
    };

    // Provider selection (M10): faux by default; real routes through genai, which
    // resolves auth per model family from standard env vars (never stored).
    let stream_fn: Arc<dyn epsilon_llm::StreamFn> = match args.provider {
        Provider::Faux => Arc::new(PanicGuard(FauxProvider::new(steps))),
        #[cfg(feature = "real")]
        Provider::Real => {
            eprintln!("provider: real · model: {} (keys from env)", args.model_id);
            Arc::new(PanicGuard(epsilon_llm::GenaiAdapter::new()))
        }
        #[cfg(not(feature = "real"))]
        Provider::Real => die("this binary was built without --features real; \
             rebuild with `cargo build --features real -p epsilon-cli`"),
    };

    match args.mode {
        Mode::Demo => demo_mode().await,
        Mode::Print(task) => {
            print_mode(
                workspace,
                stream_fn,
                task,
                &args.mcp_servers,
                &args.confinement,
                &args.model_id,
            )
            .await
        }
        Mode::Tui(initial) => {
            tui_mode(
                workspace,
                stream_fn,
                initial,
                &args.mcp_servers,
                &args.confinement,
                &args.model_id,
            )
            .await
        }
        Mode::Serve(initial) => {
            serve_mode(
                workspace,
                stream_fn,
                initial,
                args.port,
                &args.mcp_servers,
                &args.confinement,
                &args.model_id,
            )
            .await
        }
    }
}

fn default_steps() -> Vec<FauxStep> {
    vec![
        FauxStep::call(
            "tc_1",
            "bash",
            serde_json::json!({"command": "echo epsilon-ready"}),
        ),
        FauxStep::say("Run complete."),
    ]
}

/// Shared composition: real tools + ruleset gate over the given approval UI.
async fn build_runtime(
    workspace: PathBuf,
    stream_fn: Arc<dyn epsilon_llm::StreamFn>,
    approvals: Arc<dyn ApprovalUi>,
    mcp_servers: &[McpServerConfig],
    session_file: &str,
    confinement: &epsilon_tools::Confinement,
    model_id: &str,
) -> Result<Arc<Agent>, String> {
    let mut registry = epsilon_tools::builtin_registry(confinement);
    for cfg in mcp_servers {
        let attached = epsilon_mcp::attach_stdio_server(&mut registry, cfg).await?;
        eprintln!("mcp server '{}' attached ({attached} tools)", cfg.name);
    }

    let gate = Arc::new(RulesetGate::new(approvals));
    let agent = Agent::new(
        LoopLimits::default(),
        stream_fn,
        Arc::new(registry),
        gate,
        workspace.clone(),
    );
    let store = epsilon_session::SessionStore::open(workspace.join(session_file))
        .map_err(|e| format!("open session store: {e}"))?;
    agent.set_model_id(model_id);
    agent.attach_session_store(Arc::new(std::sync::Mutex::new(store)));
    Ok(agent)
}

fn user_message(text: String) -> Message {
    Message::User(UserMessage {
        id: new_id("msg"),
        timestamp_ms: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0),
        content: vec![UserContent::Text { text }],
    })
}

/// Headless single-prompt run; prints only the FINAL assistant text (pi behavior).
async fn print_mode(
    workspace: PathBuf,
    stream_fn: Arc<dyn epsilon_llm::StreamFn>,
    task: String,
    mcp_servers: &[McpServerConfig],
    confinement: &epsilon_tools::Confinement,
    model_id: &str,
) -> i32 {
    // Print is unattended: asks auto-approve rather than dead-ending on a dialog.
    let agent = match build_runtime(
        workspace,
        stream_fn,
        Arc::new(epsilon_safety::AutoApprove),
        mcp_servers,
        ".epsilon/sessions/cli.jsonl",
        confinement,
        model_id,
    )
    .await
    {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    match agent.run(user_message(task)).await {
        Ok(outcome) => {
            for m in outcome.produced.iter().rev() {
                if let Message::Assistant(a) = m
                    && a.stop_reason == StopReason::EndTurn
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

/// Interactive ratatui frontend (M9b).
async fn tui_mode(
    workspace: PathBuf,
    stream_fn: Arc<dyn epsilon_llm::StreamFn>,
    initial_task: Option<String>,
    mcp_servers: &[McpServerConfig],
    confinement: &epsilon_tools::Confinement,
    model_id: &str,
) -> i32 {
    let approvals = Arc::new(ChannelApprovalUi::new());
    // Composition happens here; the TUI crate stays a pure frontend.
    let agent = build_runtime(
        workspace,
        stream_fn,
        Arc::clone(&approvals) as Arc<dyn ApprovalUi>,
        mcp_servers,
        ".epsilon/sessions/tui.jsonl",
        confinement,
        model_id,
    )
    .await;
    match agent {
        Ok(agent) => {
            epsilon_tui::run(epsilon_tui::TuiConfig {
                agent,
                approvals,
                initial_task,
            })
            .await
        }
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

/// HTTP/SSE server exposing the runtime to remote frontends (M9b).
async fn serve_mode(
    workspace: PathBuf,
    stream_fn: Arc<dyn epsilon_llm::StreamFn>,
    initial_task: Option<String>,
    port: u16,
    mcp_servers: &[McpServerConfig],
    confinement: &epsilon_tools::Confinement,
    model_id: &str,
) -> i32 {
    let approvals = Arc::new(ChannelApprovalUi::new());
    let agent = match build_runtime(
        workspace,
        stream_fn,
        Arc::clone(&approvals) as Arc<dyn ApprovalUi>,
        mcp_servers,
        ".epsilon/sessions/serve.jsonl",
        confinement,
        model_id,
    )
    .await
    {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    if let Some(task) = initial_task {
        tokio::spawn({
            let agent = Arc::clone(&agent);
            async move {
                let _ = agent.run(user_message(task)).await;
            }
        });
    }

    let addr = format!("127.0.0.1:{port}");
    let listener = match tokio::net::TcpListener::bind(&addr).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("bind {addr}: {e}");
            return 1;
        }
    };
    eprintln!(
        "epsilon serving on http://{addr}\n  events: GET  /events (SSE)\n  prompt: POST /prompt {{\"task\":\"…\"}}\n  approvals: GET /approvals · POST /approvals/<id>"
    );
    let state = epsilon_server::ServeState::new(agent, approvals);
    match epsilon_server::serve(state, listener).await {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("server error: {e}");
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

    let prompt = user_message("fix the flaky auth tests".into());
    let run_task = tokio::spawn(async move { agent.run(prompt).await });

    while let Ok(ev) = rx.recv().await {
        render_demo_event(&ev);
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

fn render_demo_event(ev: &Arc<AgentEvent>) {
    match &**ev {
        AgentEvent::AgentStart => println!("\n=== run start ==="),
        AgentEvent::TurnStart { turn_index } => println!("-- turn {turn_index} --"),
        AgentEvent::MessageEnd { message } => match &**message {
            Message::Assistant(a) => {
                if !a.text().is_empty() {
                    println!("assistant> {}", a.text());
                }
                for (_, name, _) in a.tool_calls() {
                    println!("tool-call> {name}");
                }
            }
            Message::ToolResult(t) => {
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
