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

const USAGE: &str = "usage: epsilon            interactive TUI (default)
  epsilon -p \"task\"       headless run · epsilon --serve [--port N] · epsilon --rpc
  epsilon --demo          scripted offline demo · --list-models [provider]
  epsilon auth list       stored provider keys (~/.config/epsilon/auth.json)
  epsilon auth login <provider> [key]   save an API key (prompted if key omitted)
  epsilon auth logout <provider>        remove the stored key
  flags: --model id · --provider faux|real · --script f · --mcp name=cmd[:a,b]
         --sandbox auto|off · --plan (read-only) · -c/--continue | --resume f · --version";

struct Args {
    mode: Mode,
    script_path: Option<String>,
    workspace: Option<PathBuf>,
    mcp_servers: Vec<McpServerConfig>,
    port: u16,
    confinement: epsilon_tools::Confinement,
    /// Explicit --provider; None = auto-detect from env keys (M14).
    provider: Option<Provider>,
    /// `--model` override; None falls back to saved config then the catalog default.
    model_id: Option<String>,
    /// `--resume <path>` / `--continue`: seed history from a prior session file.
    resume: Resume,
    /// `--plan`: read-only research run (opencode plan-agent parity) — write/edit
    /// denied outright, `git commit`/`git push` blocked; everything else normal.
    plan_mode: bool,
    /// `--yes`: headless runs auto-APPROVE gated asks instead of the safe default
    /// (auto-deny). Explicit opt-in for trusted automation.
    auto_approve: bool,
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

/// Model source: the offline faux provider (default) or a real direct-HTTP provider
/// (M17 pi port).
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
    /// M22: opencode-style credential management (list/login/logout), then exit.
    Auth(AuthAction),
    /// Hidden M16a benchmark: build n synthetic transcript messages, time real draws,
    /// print a report, exit. Not advertised in --help usage beyond this line.
    TuiBench(usize),
}

/// One `epsilon auth …` subcommand (opencode auth parity).
#[derive(Debug, Clone)]
enum AuthAction {
    /// Show stored providers with masked keys.
    List,
    /// Store a key for a provider; prompts on the terminal when omitted.
    Login {
        provider: String,
        key: Option<String>,
    },
    /// Remove a provider's stored key.
    Logout { provider: String },
}

fn parse_args() -> Args {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    if raw.iter().any(|a| a == "--version" || a == "-V") {
        println!("epsilon {}", env!("CARGO_PKG_VERSION"));
        std::process::exit(0);
    }
    // M22: `epsilon auth <sub>` is a pre-mode subcommand — it never touches the
    // workspace or runtime, so parse and dispatch it immediately.
    if raw.first().map(|a| a == "auth").unwrap_or(false) {
        let mode = match raw.get(1).map(|s| s.as_str()) {
            Some("list") => Mode::Auth(AuthAction::List),
            Some("login") => {
                let provider = raw
                    .get(2)
                    .cloned()
                    .unwrap_or_else(|| die("auth login requires a provider id (e.g. anthropic)"));
                let key = raw.get(3).cloned();
                Mode::Auth(AuthAction::Login { provider, key })
            }
            Some("logout") => {
                let provider = raw
                    .get(2)
                    .cloned()
                    .unwrap_or_else(|| die("auth logout requires a provider id"));
                Mode::Auth(AuthAction::Logout { provider })
            }
            _ => die("usage: epsilon auth list | login <provider> [key] | logout <provider>"),
        };
        return Args {
            mode,
            script_path: None,
            workspace: None,
            mcp_servers: Vec::new(),
            port: 8787,
            confinement: epsilon_tools::Confinement::Auto,
            provider: None,
            model_id: None,
            resume: Resume::default(),
            plan_mode: false,
            auto_approve: false,
        };
    }
    let mut mode = Mode::Tui(None); // M14: plain `epsilon` opens the interactive TUI
    let mut script_path = None;
    let mut workspace = None;
    let mut mcp_servers = Vec::new();
    let mut port: u16 = 8787;
    let mut confinement = epsilon_tools::Confinement::Auto;
    // --plan: read-only research run (opencode plan-agent parity).
    let mut plan_mode = false;
    // --yes: unattended runs approve gated asks (explicit opt-in; default is deny).
    let mut auto_approve = false;
    let mut provider: Option<Provider> = None;
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
                    Some("faux") => provider = Some(Provider::Faux),
                    Some("real") => provider = Some(Provider::Real),
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
            "--demo" => mode = Mode::Demo,
            "--plan" => plan_mode = true,
            "--yes" => auto_approve = true,
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
            "--tui-bench" => {
                i += 1;
                let n = raw
                    .get(i)
                    .and_then(|v| v.parse::<usize>().ok())
                    .unwrap_or_else(|| {
                        die("--tui-bench requires a message count (e.g. --tui-bench 200)")
                    });
                mode = Mode::TuiBench(n);
            }
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
        plan_mode,
        auto_approve,
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
    // M22: auth subcommands are plain file I/O — handle them before building
    // the tokio runtime (and exit immediately after).
    if let Mode::Auth(action) = &args.mode {
        std::process::exit(auth_command(action));
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let code = runtime.block_on(dispatch(args));
    std::process::exit(code);
}

/// `epsilon auth list|login|logout` — opencode's credential UX (M22).
///
/// Keys are stored in `~/.config/epsilon/auth.json`; `list` masks everything
/// but the last 4 characters so screenshots never leak full secrets. Login
/// reads the key from the argument when given (scripting) or prompts on the
/// terminal when omitted (interactive; the typed key is not echoed where the
/// terminal supports it — best-effort, same as opencode's prompt fallback).
fn auth_command(action: &AuthAction) -> i32 {
    match action {
        AuthAction::List => {
            let creds = epsilon_llm::auth::load_credentials_in(
                &epsilon_llm::auth::global_dir().unwrap_or_default(),
            );
            if creds.is_empty() {
                println!("no stored credentials (~/.config/epsilon/auth.json)");
                println!("add one:  epsilon auth login <provider>");
                return 0;
            }
            println!("stored credentials (~/.config/epsilon/auth.json):");
            for (provider, cred) in &creds {
                let k = cred.key.trim();
                let masked = if k.len() <= 4 {
                    "****".to_string()
                } else {
                    format!("…{}", &k[k.len() - 4..])
                };
                println!("  {provider:<14} {masked}");
            }
            0
        }
        AuthAction::Login { provider, key } => {
            let key = match key {
                Some(k) => k.clone(),
                None => {
                    // Interactive: read a line without echoing when possible.
                    eprint!("API key for {provider} (input hidden): ");
                    match read_secret_line() {
                        Some(k) if !k.trim().is_empty() => k,
                        _ => {
                            eprintln!("no key entered; nothing stored");
                            return 2;
                        }
                    }
                }
            };
            match epsilon_llm::auth::save_credential(provider.trim(), key.trim()) {
                Ok(path) => {
                    println!("stored {provider} key in {}", path.display());
                    0
                }
                Err(e) => {
                    eprintln!("{e}");
                    1
                }
            }
        }
        AuthAction::Logout { provider } => {
            match epsilon_llm::auth::remove_credential(provider.trim()) {
                Ok(true) => {
                    println!("removed stored key for {provider}");
                    0
                }
                Ok(false) => {
                    println!("no stored key for {provider}");
                    0
                }
                Err(e) => {
                    eprintln!("{e}");
                    1
                }
            }
        }
    }
}

/// Read one line of input with echo suppressed where the platform allows it.
/// Falls back to plain stdin when the console API declines (piped input).
#[cfg(windows)]
fn read_secret_line() -> Option<String> {
    // Plain stdin line read: the secret never lands in shell history, and
    // echo suppression is deferred until a console-API pass needs it (the
    // workspace forbids `unsafe`, which ReadConsole tricks would require).
    let mut line = String::new();
    std::io::stdin()
        .read_line(&mut line)
        .ok()
        .map(|_| line.trim_end_matches(['\r', '\n']).to_string())
}

/// Unix variant mirrors the Windows one (line read; no termios dance yet).
#[cfg(not(windows))]
fn read_secret_line() -> Option<String> {
    use std::io::BufRead;
    let mut line = String::new();
    std::io::stdin()
        .read_line(&mut line)
        .ok()
        .map(|_| line.trim_end_matches(['\r', '\n']).to_string())
}

async fn dispatch(args: Args) -> i32 {
    let workspace = args
        .workspace
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    // One diagnostics collector for the whole startup path (plan 008 / DEBT-04):
    // replaces the three ad-hoc Vec<String> patterns (project notices, banners,
    // extension notices) with a single ordered sink flushed once.
    struct Diagnostics(Vec<String>);
    impl Diagnostics {
        fn push(&mut self, line: impl Into<String>) {
            self.0.push(line.into());
        }
        fn flush_stderr(&self) {
            for line in &self.0 {
                eprintln!("{line}");
            }
        }
    }
    let mut diag = Diagnostics(Vec::new());

    // Project layer (ADR-012, M12): .epsilon/config.toml + trust hash.
    // M22: the trust decision is explicit. Headless modes keep the loud-banner
    // policy (still load); the TUI re-prompts interactively via startup_notices.
    let (project, trust, project_notices) = epsilon_cli::runtime::load_project_config(&workspace);
    let mut startup_notices = project_notices;
    if matches!(trust, epsilon_cli::runtime::TrustDecision::Changed { .. }) {
        startup_notices.extend([
            "WARNING: .epsilon/config.toml CHANGED since it was trusted; \
             review before running untrusted projects"
                .to_string(),
            "(the interactive TUI asks before anything executes; --yes skips the ask)".to_string(),
        ]);
    }

    // ---- provider + model resolution (M14: real harness out of the box) ------------
    // Precedence for the model id: --model flag > project config > global saved.
    // Provider: explicit --provider wins; otherwise AUTO — if any catalog provider's
    // API key is present in the environment (and the chosen model's credentials are
    // ready), run REAL; only when nothing is configured do we fall back to the offline
    // faux provider, with a banner saying exactly how to go real. pi/opencode parity:
    // a fresh user with one env var gets a working agent with zero flags.
    let candidate_model = args
        .model_id
        .clone()
        .or_else(|| project.model_id.clone())
        .or_else(epsilon_core::config::load_model_id);

    let auto_real = |candidate: &Option<String>| -> Option<(String, String)> {
        if let Some(m) = candidate
            && epsilon_llm::catalog::credentials_ready(m)
        {
            let key = epsilon_llm::catalog::env_key_for(m)
                .unwrap_or("")
                .to_string();
            return Some((m.clone(), key));
        }
        // No usable candidate: first tooled model whose key is present.
        epsilon_llm::catalog::curated()
            .iter()
            .find(|i| {
                i.supports_tools && !i.env_key.is_empty() && std::env::var_os(i.env_key).is_some()
            })
            .map(|i| (i.id.to_string(), i.env_key.to_string()))
    };

    if args.plan_mode {
        diag.push(
            "PLAN MODE: read-only research run — write/edit are denied outright; \
             re-run without --plan to apply changes",
        );
    }
    let use_real: bool = match args.provider {
        Some(Provider::Faux) => false,
        Some(Provider::Real) => true,
        None => {
            #[cfg(feature = "real")]
            {
                auto_real(&candidate_model).is_some()
            }
            #[cfg(not(feature = "real"))]
            {
                if auto_real(&candidate_model).is_some() {
                    diag.push(
                        "API key detected but this binary lacks --features real — \
                         running offline faux. Rebuild: cargo install --path crates/epsilon-cli --features real",
                    );
                }
                false
            }
        }
    };

    let model_id = if use_real {
        match candidate_model.filter(|m| epsilon_llm::catalog::credentials_ready(m)) {
            Some(m) => m,
            None => match auto_real(&None) {
                Some((m, _)) => m,
                None => die("--provider real requested but no provider API key found \
                     (set e.g. DEEPSEEK_API_KEY / ANTHROPIC_API_KEY)"),
            },
        }
    } else {
        candidate_model.unwrap_or_else(|| epsilon_core::config::DEFAULT_MODEL_ID.to_string())
    };

    let steps: Vec<FauxStep> = if use_real {
        Vec::new()
    } else {
        match &args.script_path {
            Some(p) => {
                let raw =
                    std::fs::read_to_string(p).unwrap_or_else(|e| die(&format!("read {p}: {e}")));
                serde_json::from_str(&raw)
                    .unwrap_or_else(|e| die(&format!("invalid script json: {e}")))
            }
            None => default_steps(),
        }
    };

    let stream_fn: Arc<dyn epsilon_llm::StreamFn> = if use_real {
        #[cfg(feature = "real")]
        {
            // M22: say WHERE the key came from so users can tell auth.json from env.
            let provider = epsilon_llm::catalog::lookup_model(model_id.as_str())
                .map(|m| m.provider)
                .unwrap_or("");
            let key_source = if epsilon_llm::auth::load_credentials().contains_key(provider) {
                "stored (epsilon auth login)"
            } else {
                "env var"
            };
            diag.push(format!(
                "provider: real (direct HTTP) · model: {model_id} · key {key_source}"
            ));
            Arc::new(PanicGuard(epsilon_llm::DirectProvider::new()))
        }
        #[cfg(not(feature = "real"))]
        unreachable!("use_real cannot be true without the real feature")
    } else {
        diag.push("provider: faux (offline scripted)");
        diag.push(
            "go real:  epsilon auth login <provider>   (or set the provider's API key env var)",
        );
        Arc::new(PanicGuard(FauxProvider::new(steps)))
    };
    // Startup notices flush here, in order, once (plan 008).
    diag.flush_stderr();

    let shared = SharedRun {
        workspace,
        stream_fn,
        mcp_servers: args.mcp_servers.clone(),
        confinement: args.confinement.clone(),
        model_id,
        resume: args.resume.clone(),
        project,
        plan_mode: args.plan_mode,
        startup_notices,
    };
    match &args.mode {
        Mode::Auth(_) => unreachable!("auth subcommands exit inside main() before dispatch"),
        Mode::Demo => demo_mode().await,
        Mode::Print(task) => print_mode(&shared, args.auto_approve, task.clone()).await,
        Mode::Tui(initial) => tui_mode(&shared, initial.clone()).await,
        Mode::Serve(initial) => serve_mode(&shared, initial.clone(), args.port).await,
        Mode::Rpc => rpc_mode(&shared).await,
        Mode::ListModels(provider_filter) => {
            list_models(provider_filter.as_deref());
            0
        }
        Mode::TuiBench(n) => {
            for line in epsilon_tui::bench::run(*n) {
                println!("{line}");
            }
            0
        }
    }
}

/// Live provider-listing capability for the picker (M13, M17 rewire): present only
/// when the binary was built with `--features real`; the fetch is a plain
/// authenticated GET against the provider's models endpoint.
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
async fn rpc_mode(shared: &SharedRun) -> i32 {
    let approvals = Arc::new(ChannelApprovalUi::new());
    let agent = match build_runtime(RunParams {
        workspace: &shared.workspace,
        stream_fn: Arc::clone(&shared.stream_fn),
        approvals: Arc::clone(&approvals) as Arc<dyn ApprovalUi>,
        mcp_servers: &shared.mcp_servers,
        session_file: ".epsilon/sessions/rpc.jsonl",
        confinement: &shared.confinement,
        model_id: &shared.model_id,
        resume: &shared.resume,
        project: &shared.project,
        plan_mode: shared.plan_mode,
    })
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
/// Inputs every frontend mode shares (plan 008): one struct instead of nine
/// positionally-similar parameters per mode function.
struct SharedRun {
    workspace: PathBuf,
    stream_fn: Arc<dyn epsilon_llm::StreamFn>,
    mcp_servers: Vec<McpServerConfig>,
    confinement: epsilon_tools::Confinement,
    model_id: String,
    resume: Resume,
    project: epsilon_cli::runtime::ProjectConfig,
    plan_mode: bool,
    /// Project-config notices + trust banner; the TUI re-prompts when the
    /// trust line is present (M22 / DIRECTION-01).
    startup_notices: Vec<String>,
}

/// Everything `build_runtime` needs to compose one agent (plan 008 / DEBT-01).
/// One `workspace` field replaces the old twin (`workspace` + `workspace`
/// were always the same path at every call site — the second name was pure noise).
struct RunParams<'a> {
    workspace: &'a Path,
    stream_fn: Arc<dyn epsilon_llm::StreamFn>,
    approvals: Arc<dyn ApprovalUi>,
    mcp_servers: &'a [McpServerConfig],
    /// Session JSONL path, relative to the workspace (e.g. ".epsilon/sessions/tui.jsonl").
    session_file: &'a str,
    confinement: &'a epsilon_tools::Confinement,
    model_id: &'a str,
    resume: &'a Resume,
    project: &'a epsilon_cli::runtime::ProjectConfig,
    plan_mode: bool,
}

async fn build_runtime(
    RunParams {
        workspace,
        stream_fn,
        approvals,
        mcp_servers,
        session_file,
        confinement,
        model_id,
        resume,
        project,
        plan_mode,
    }: RunParams<'_>,
) -> Result<Arc<Agent>, String> {
    let mut registry = epsilon_tools::builtin_registry(confinement);
    let mut all_mcp: Vec<McpServerConfig> = mcp_servers.to_vec();
    all_mcp.extend(project.mcp_servers.iter().cloned());
    for cfg in &all_mcp {
        let attached = epsilon_mcp::attach_stdio_server(&mut registry, cfg).await?;
        eprintln!("mcp server '{}' attached ({attached} tools)", cfg.name);
    }

    // Skills (M8 brick, wired M11): conventional roots feed the CONTEXT-tier catalog.
    let skills = epsilon_core::skills::discover(&epsilon_cli::runtime::skill_roots(workspace));
    if !skills.is_empty() {
        let names: Vec<&str> = skills.iter().map(|s| s.name.as_str()).collect();
        eprintln!("skills: {}", names.join(", "));
    }

    // Extensions (M8/M9a bricks, wired M11): conventional roots PLUS project-declared
    // dirs resolved against the workspace's .epsilon/ (ADR-012 project layer, M12).
    let mut ext_roots = epsilon_cli::runtime::extension_roots(workspace);
    if !project.extension_dirs.is_empty() {
        let base = workspace.join(".epsilon");
        ext_roots.extend(project.extension_dirs.iter().map(|d| base.join(d)));
    }
    let (ext_hosts, ext_notices) =
        epsilon_cli::runtime::attach_extensions(&mut registry, &ext_roots).await;
    for line in &ext_notices {
        eprintln!("{line}");
    }

    // Ruleset gate: defaults + optional project [[rules]] layer (last-match-wins; any
    // deny still wins immediately — ADR-006 stacking semantics). `--plan` prepends the
    // plan-mode layer: mutating capabilities are DENIED outright and monotonic-deny
    // makes that boundary hold regardless of project rules or approvals.
    let base_engine = if plan_mode {
        epsilon_safety::PermissionEngine::layered(vec![epsilon_safety::plan_ruleset()])
    } else if project.rules.is_empty() {
        epsilon_safety::PermissionEngine::with_defaults()
    } else {
        epsilon_safety::PermissionEngine::layered(vec![epsilon_cli::runtime::project_rules_layer(
            &project.rules,
        )])
    };
    let gate = Arc::new(RulesetGate::layered(base_engine, approvals));
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
        let mut restored = epsilon_session::SessionStore::open(&rp)
            .map_err(|e| format!("open {}: {e}", rp.display()))?;
        // ponytail-audit follow-up: repair is now WIRED, not test-only — a crashed
        // prior run leaves dangling tool calls on the active branch; synthesize
        // tombstones before the model view so the resumed context stays provider-valid.
        match epsilon_session::repair::repair_on_open(&mut restored) {
            Ok(report) if report.repaired_tool_calls > 0 || report.appended_turn_end => eprintln!(
                "repaired {}: {} tool results synthesized, turn_end: {}",
                rp.display(),
                report.repaired_tool_calls,
                report.appended_turn_end
            ),
            Ok(_) => {}
            Err(e) => eprintln!("repair warning (continuing): {e}"),
        }
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
async fn print_mode(shared: &SharedRun, yes: bool, task: String) -> i32 {
    // Print is unattended: asks auto-approve rather than dead-ending on a dialog.
    let approvals_ui: Arc<dyn ApprovalUi> = if yes {
        eprintln!("WARNING: --yes set — permission asks are AUTO-APPROVED in this run");
        Arc::new(epsilon_safety::AutoApprove)
    } else {
        // Plan 003: unattended runs fail closed on asks (deny-with-guidance).
        Arc::new(epsilon_safety::AutoDeny)
    };
    let agent = match build_runtime(RunParams {
        workspace: &shared.workspace,
        stream_fn: Arc::clone(&shared.stream_fn),
        approvals: approvals_ui,
        mcp_servers: &shared.mcp_servers,
        session_file: ".epsilon/sessions/cli.jsonl",
        confinement: &shared.confinement,
        model_id: &shared.model_id,
        resume: &shared.resume,
        project: &shared.project,
        plan_mode: shared.plan_mode,
    })
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
async fn tui_mode(shared: &SharedRun, initial_task: Option<String>) -> i32 {
    let approvals = Arc::new(ChannelApprovalUi::new());
    // Composition happens here; the TUI crate stays a pure frontend.
    let agent = build_runtime(RunParams {
        workspace: &shared.workspace,
        stream_fn: Arc::clone(&shared.stream_fn),
        approvals: Arc::clone(&approvals) as Arc<dyn ApprovalUi>,
        mcp_servers: &shared.mcp_servers,
        session_file: ".epsilon/sessions/tui.jsonl",
        confinement: &shared.confinement,
        model_id: &shared.model_id,
        resume: &shared.resume,
        project: &shared.project,
        plan_mode: shared.plan_mode,
    })
    .await;
    match agent {
        Ok(agent) => {
            epsilon_tui::run(epsilon_tui::TuiConfig {
                agent,
                approvals,
                initial_task,
                model_id: shared.model_id.clone(),
                live: live_fetcher(),
                startup_notices: shared.startup_notices.clone(),
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
async fn serve_mode(shared: &SharedRun, initial_task: Option<String>, port: u16) -> i32 {
    let approvals = Arc::new(ChannelApprovalUi::new());
    let agent = match build_runtime(RunParams {
        workspace: &shared.workspace,
        stream_fn: Arc::clone(&shared.stream_fn),
        approvals: Arc::clone(&approvals) as Arc<dyn ApprovalUi>,
        mcp_servers: &shared.mcp_servers,
        session_file: ".epsilon/sessions/serve.jsonl",
        confinement: &shared.confinement,
        model_id: &shared.model_id,
        resume: &shared.resume,
        project: &shared.project,
        plan_mode: shared.plan_mode,
    })
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
    // Plan 003: per-boot bearer token — printed ONCE here; every request except
    // /healthz must carry `authorization: Bearer <token>`. Serve mode is a trust
    // boundary; without the token a local process or webpage could drive runs.
    let state = {
        let st = epsilon_server::ServeState::with_boot_token(agent, approvals);
        if let Some(t) = st.auth_token.as_ref() {
            eprintln!("  auth: authorization: Bearer {t}");
        }
        st
    };
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
