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

use std::path::{Path, PathBuf};
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
    /// `--model` override; None falls back to saved config then the catalog default.
    model_id: Option<String>,
    /// `--resume <path>` / `--continue`: seed history from a prior session file.
    resume: Resume,
}

/// Session-restore selection (M12).
#[derive(Debug, Clone, Default)]
enum Resume {
    /// Fresh transcript (default).
    #[default]
    No,
    /// `--continue`: newest session file for the chosen mode under `.epsilon/sessions`.
    Latest,
    /// `--resume <path>`: explicit session file.
    Path(PathBuf),
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
    Rpc,
    /// Print the bundled catalog (optionally filtered by provider), then exit.
    ListModels(Option<String>),
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
    let mut resume = Resume::default();
    // None = not given on the command line; resolution falls back to the saved
    // config value, then the catalog default (ADR-018 precedence).
    let mut model_id: Option<String> = None;
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
                model_id = Some(raw.get(i).cloned().unwrap_or_else(|| {
                    die("--model requires a model id (e.g. claude-sonnet-4-5)")
                }));
            }
            "-c" | "--continue" => resume = Resume::Latest,
            "--resume" => {
                i += 1;
                let p = raw
                    .get(i)
                    .cloned()
                    .unwrap_or_else(|| die("--resume requires a session file path"));
                resume = Resume::Path(PathBuf::from(p));
            }
            "--rpc" => mode = Mode::Rpc,
            "--list-models" => {
                i += 1;
                let provider = positional(&raw, &mut i);
                mode = Mode::ListModels(provider);
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
        resume,
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

    // Project layer (ADR-012, M12): .epsilon/config.toml + trust hash.
    let (project, project_notices) = epsilon_cli::runtime::load_project_config(&workspace);
    for line in &project_notices {
        eprintln!("{line}");
    }

    // Model resolution (ADR-018 precedence, project layer inserted at M12):
    // --model flag > project config > global saved config > catalog default.
    let model_id = args
        .model_id
        .clone()
        .or_else(|| project.model_id.clone())
        .or_else(epsilon_core::config::load_model_id)
        .unwrap_or_else(|| epsilon_core::config::DEFAULT_MODEL_ID.to_string());
    if args.model_id.is_none()
        && let Some(saved) = epsilon_core::config::load_model_id()
    {
        eprintln!("model: {saved} (from saved config)");
    }
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
            eprintln!("provider: real · model: {model_id} (keys from env)");
            Arc::new(PanicGuard(epsilon_llm::GenaiAdapter::new()))
        }
        #[cfg(not(feature = "real"))]
        Provider::Real => die("this binary was built without --features real; \
             rebuild with `cargo build --features real -p epsilon-cli`"),
    };

    match &args.mode {
        Mode::Demo => demo_mode().await,
        Mode::Print(task) => {
            print_mode(
                workspace,
                stream_fn,
                task.clone(),
                &args.mcp_servers,
                &args.confinement,
                &model_id,
                &args.resume,
                &project,
            )
            .await
        }
        Mode::Tui(initial) => {
            tui_mode(
                workspace,
                stream_fn,
                initial.clone(),
                &args.mcp_servers,
                &args.confinement,
                &model_id,
                &args.resume,
                &project,
            )
            .await
        }
        Mode::Serve(initial) => {
            serve_mode(
                workspace,
                stream_fn,
                initial.clone(),
                args.port,
                &args.mcp_servers,
                &args.confinement,
                &model_id,
                &args.resume,
                &project,
            )
            .await
        }
        Mode::Rpc => {
            rpc_mode(
                workspace,
                stream_fn,
                &args.mcp_servers,
                &args.confinement,
                &model_id,
                &args.resume,
                &project,
            )
            .await
        }
        Mode::ListModels(provider_filter) => {
            list_models(provider_filter.as_deref());
            0
        }
    }
}

/// Live provider-listing capability for the picker (M13): present only when the binary
/// was built with `--features real`; the fetch itself runs genai's resolver.
fn live_fetcher() -> Option<epsilon_tui::LiveFetcher> {
    #[cfg(feature = "real")]
    {
        Some(Arc::new(|hint: String| {
            Box::pin(async move { epsilon_llm::catalog::live_models(&hint).await })
                as futures_util::future::BoxFuture<'static, Result<Vec<String>, String>>
        }))
    }
    #[cfg(not(feature = "real"))]
    {
        None
    }
}

/// Editor-embedding frontend: NDJSON over stdio (ADR-011 rpc mode).
#[allow(clippy::too_many_arguments)]
async fn rpc_mode(
    workspace: PathBuf,
    stream_fn: Arc<dyn epsilon_llm::StreamFn>,
    mcp_servers: &[McpServerConfig],
    confinement: &epsilon_tools::Confinement,
    model_id: &str,
    resume: &Resume,
    project: &epsilon_cli::runtime::ProjectConfig,
) -> i32 {
    let approvals = Arc::new(ChannelApprovalUi::new());
    let agent = match build_runtime(
        &workspace,
        stream_fn,
        Arc::clone(&approvals) as Arc<dyn ApprovalUi>,
        mcp_servers,
        ".epsilon/sessions/rpc.jsonl",
        confinement,
        model_id,
        resume,
        project,
        &workspace,
    )
    .await
    {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    epsilon_cli::rpc_mode::run(agent, approvals).await
}

/// Headless catalog dump (`--list-models [provider]`) — pi's `list-models` analogue.
fn list_models(provider_filter: Option<&str>) {
    let filter = provider_filter.map(|p| p.to_ascii_lowercase());
    let mut any = false;
    for info in epsilon_llm::catalog::curated() {
        if let Some(f) = &filter
            && !info.provider.eq_ignore_ascii_case(f)
        {
            continue;
        }
        any = true;
        let key = if info.env_key.is_empty() {
            "—".to_string()
        } else if std::env::var_os(info.env_key).is_some() {
            format!("🔑 {}", info.env_key)
        } else {
            format!("   (set {})", info.env_key)
        };
        println!(
            "{:<22} {:<12} {:>9}-tok  {:<28} {}",
            info.id,
            info.provider,
            format!("{}k", info.context_window / 1_000),
            info.label,
            key,
        );
    }
    if !any {
        let known: Vec<&str> = epsilon_llm::catalog::curated()
            .iter()
            .map(|m| m.provider)
            .collect();
        eprintln!(
            "unknown provider '{provider_filter:?}'; bundled providers: {}",
            known.join(", ")
        );
        std::process::exit(2);
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
#[allow(clippy::too_many_arguments)]
async fn build_runtime(
    workspace: &Path,
    stream_fn: Arc<dyn epsilon_llm::StreamFn>,
    approvals: Arc<dyn ApprovalUi>,
    mcp_servers: &[McpServerConfig],
    session_file: &str,
    confinement: &epsilon_tools::Confinement,
    model_id: &str,
    resume: &Resume,
    project: &epsilon_cli::runtime::ProjectConfig,
    workspace_for_roots: &Path,
) -> Result<Arc<Agent>, String> {
    let mut registry = epsilon_tools::builtin_registry(confinement);
    let mut all_mcp: Vec<McpServerConfig> = mcp_servers.to_vec();
    all_mcp.extend(project.mcp_servers.iter().cloned());
    for cfg in &all_mcp {
        let attached = epsilon_mcp::attach_stdio_server(&mut registry, cfg).await?;
        eprintln!("mcp server '{}' attached ({attached} tools)", cfg.name);
    }

    // Skills (M8 brick, wired M11): conventional roots feed the CONTEXT-tier catalog.
    let skills =
        epsilon_core::skills::discover(&epsilon_cli::runtime::skill_roots(workspace_for_roots));
    if !skills.is_empty() {
        let names: Vec<&str> = skills.iter().map(|s| s.name.as_str()).collect();
        eprintln!("skills: {}", names.join(", "));
    }

    // Extensions (M8/M9a bricks, wired M11): conventional roots PLUS project-declared
    // dirs resolved against the workspace's .epsilon/ (ADR-012 project layer, M12).
    let mut ext_roots = epsilon_cli::runtime::extension_roots(workspace_for_roots);
    if !project.extension_dirs.is_empty() {
        let base = workspace_for_roots.join(".epsilon");
        ext_roots.extend(project.extension_dirs.iter().map(|d| base.join(d)));
    }
    let (ext_hosts, ext_notices) =
        epsilon_cli::runtime::attach_extensions(&mut registry, &ext_roots).await;
    for line in &ext_notices {
        eprintln!("{line}");
    }

    // Ruleset gate: defaults + optional project [[rules]] layer (last-match-wins; any
    // deny still wins immediately — ADR-006 stacking semantics).
    let gate = Arc::new(RulesetGate::layered(
        if project.rules.is_empty() {
            epsilon_safety::PermissionEngine::with_defaults()
        } else {
            epsilon_safety::PermissionEngine::layered(vec![
                epsilon_cli::runtime::project_rules_layer(&project.rules),
            ])
        },
        approvals,
    ));
    let agent = Agent::new(
        LoopLimits::default(),
        stream_fn,
        Arc::new(registry),
        gate,
        workspace.to_path_buf(),
    );
    let store = epsilon_session::SessionStore::open(workspace.join(session_file))
        .map_err(|e| format!("open session store: {e}"))?;
    agent.set_model_id(model_id);
    agent.set_skills(skills);
    agent.attach_session_store(Arc::new(std::sync::Mutex::new(store)));
    epsilon_cli::runtime::spawn_event_forwarder(agent.subscribe(), ext_hosts);

    // Resume (M12): seed runtime history from a prior transcript. The restored view is
    // the store's SHADOWED model view, so resumed sessions continue post-compaction.
    let resume_path = match resume {
        Resume::No => None,
        Resume::Path(p) => Some(p.clone()),
        Resume::Latest => {
            epsilon_cli::runtime::latest_session_file(&workspace.join(".epsilon").join("sessions"))
        }
    };
    if let Some(rp) = resume_path {
        let restored = epsilon_session::SessionStore::open(&rp)
            .map_err(|e| format!("open {}: {e}", rp.display()))?;
        let msgs: Vec<Message> = restored
            .model_view()
            .map_err(|e| format!("read {}: {e}", rp.display()))?
            .into_iter()
            .map(|v| v.into_message())
            .collect();
        let count = msgs.len();
        agent.restore_history(msgs);
        eprintln!("resumed {} entries from {}", count, rp.display());
    }
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
#[allow(clippy::too_many_arguments)]
async fn print_mode(
    workspace: PathBuf,
    stream_fn: Arc<dyn epsilon_llm::StreamFn>,
    task: String,
    mcp_servers: &[McpServerConfig],
    confinement: &epsilon_tools::Confinement,
    model_id: &str,
    resume: &Resume,
    project: &epsilon_cli::runtime::ProjectConfig,
) -> i32 {
    // Print is unattended: asks auto-approve rather than dead-ending on a dialog.
    let agent = match build_runtime(
        &workspace,
        stream_fn,
        Arc::new(epsilon_safety::AutoApprove),
        mcp_servers,
        ".epsilon/sessions/cli.jsonl",
        confinement,
        model_id,
        resume,
        project,
        &workspace,
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
#[allow(clippy::too_many_arguments)]
async fn tui_mode(
    workspace: PathBuf,
    stream_fn: Arc<dyn epsilon_llm::StreamFn>,
    initial_task: Option<String>,
    mcp_servers: &[McpServerConfig],
    confinement: &epsilon_tools::Confinement,
    model_id: &str,
    resume: &Resume,
    project: &epsilon_cli::runtime::ProjectConfig,
) -> i32 {
    let approvals = Arc::new(ChannelApprovalUi::new());
    // Composition happens here; the TUI crate stays a pure frontend.
    let agent = build_runtime(
        &workspace,
        stream_fn,
        Arc::clone(&approvals) as Arc<dyn ApprovalUi>,
        mcp_servers,
        ".epsilon/sessions/tui.jsonl",
        confinement,
        model_id,
        resume,
        project,
        &workspace,
    )
    .await;
    match agent {
        Ok(agent) => {
            epsilon_tui::run(epsilon_tui::TuiConfig {
                agent,
                approvals,
                initial_task,
                model_id: model_id.to_string(),
                live: live_fetcher(),
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
#[allow(clippy::too_many_arguments)]
async fn serve_mode(
    workspace: PathBuf,
    stream_fn: Arc<dyn epsilon_llm::StreamFn>,
    initial_task: Option<String>,
    port: u16,
    mcp_servers: &[McpServerConfig],
    confinement: &epsilon_tools::Confinement,
    model_id: &str,
    resume: &Resume,
    project: &epsilon_cli::runtime::ProjectConfig,
) -> i32 {
    let approvals = Arc::new(ChannelApprovalUi::new());
    let agent = match build_runtime(
        &workspace,
        stream_fn,
        Arc::clone(&approvals) as Arc<dyn ApprovalUi>,
        mcp_servers,
        ".epsilon/sessions/serve.jsonl",
        confinement,
        model_id,
        resume,
        project,
        &workspace,
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
