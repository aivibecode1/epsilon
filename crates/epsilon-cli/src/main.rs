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

use epsilon_contracts::agent_event::{AgentEvent, TurnEndReason};
use epsilon_contracts::message::{Message, StopReason};
use epsilon_core::{Agent, LoopLimits, ToolRegistry};
use epsilon_llm::{FauxProvider, FauxStep, FauxToolCall, PanicGuard};
use epsilon_mcp::McpServerConfig;
use epsilon_safety::{ApprovalUi, ChannelApprovalUi, RulesetGate};

/// M52 guard: if this crate is built with `real` while `epsilon-core` is not,
/// `Agent::try_hot_swap_to_real` silently degrades to "restart epsilon".
/// Binding core's marker makes that combination a build error.
#[cfg(feature = "real")]
const _REAL_PATH_FORWARDED: () = epsilon_core::REAL_PATH_ENABLED;

const USAGE: &str = "usage: epsilon            interactive TUI (default)
  epsilon -p \"task\"       headless run · epsilon --serve [--port N] · epsilon --rpc
  epsilon --demo          scripted offline demo · --list-models [provider]
  epsilon doctor          why is this not working: model, credentials, sandbox, session, trust
  epsilon auth list       stored provider keys (~/.config/epsilon/auth.json)
  epsilon auth login <provider> [key]   save an API key (prompted if key omitted)
  epsilon auth login <provider> --oauth [--paste]   subscription login (anthropic|openrouter|xai)
  epsilon auth logout <provider>        remove the stored key
  flags: --model id · --provider faux|real · --script f · --mcp name=cmd[:a,b]
         --sandbox auto|read-only|off · --plan · -c/--continue | --resume f · --version
  epsilon --help          every flag, env var and file, in full";

/// Full reference, printed by `epsilon --help` / `-h` (and by `epsilon help`).
///
/// Kept in one place so the shipped binary is its own documentation: a user who
/// installed with `cargo install` has no README next to the exe, and the previous
/// behaviour (`unknown argument: --help`) made the binary look broken.
const HELP: &str = "\
epsilon — a coding-agent harness in Rust.

USAGE
  epsilon [MODE] [FLAGS]

MODES (pick one; the default is the interactive TUI)
  (none)            interactive TUI over the current directory
  -p, --print TASK  headless single prompt: reply text to stdout, diagnostics to
                    stderr, exit 0 only when the model finished cleanly
  --tui [TASK]      interactive TUI, optionally seeded with a first prompt
  --serve [--port N] [TASK]
                    HTTP + SSE server for editors and other frontends (OpenAPI at
                    /openapi.json); default port 8787
  --rpc             NDJSON request/response over stdio (one JSON object per line)
  --demo            scripted offline run against the faux provider — proves the
                    loop works with no key and no network
  --list-models [PROVIDER]
                    print the bundled model catalog (optionally one provider)
  auth list | login <provider> [key] [--oauth [--paste]] | logout <provider>
                    manage stored credentials
  --tui-bench N     render benchmark for the transcript (N synthetic messages)
  doctor            report configuration and available capabilities without
                    starting a model run: build, workspace, instructions loaded,
                    provider keys present (never their values), confinement and
                    integration availability. The first thing to run when
                    something is not working and you do not yet know why.

FLAGS
  -m, --model ID        model to run (catalog id, alias, or provider/model)
  --provider faux|real  override auto-detection (default: real when a key is
                        configured, otherwise the offline faux provider)
  --fallback ID         retryable failures fall through to this model; repeatable
  --workspace DIR       run as if started in DIR
  --lsp auto|off        Language-server diagnostics on edited files (default auto:
                        ask when a server for the language is installed)
  --sandbox MODE        OS confinement for shell commands:
                          auto       best backend, fail closed when none exists (default)
                          read-only  commands may look, not touch (the workspace mounts
                                     read-only where the backend can do it)
                          off        no confinement at all — trusted automation only
  --plan                read-only run: write/edit denied, git commit/push blocked
  --yes                 unattended runs auto-APPROVE permission asks. Without it,
                        unattended runs auto-deny.
  -c, --continue        resume the newest session in this workspace
  --resume FILE         resume a specific session file
  --script FILE         faux-provider script for offline runs
  --mcp NAME=CMD[:A,B]  attach an MCP server over stdio
  -V, --version         print the version
  -h, --help            this text

ENVIRONMENT
  <PROVIDER>_API_KEY    provider credential (ANTHROPIC_API_KEY, OPENAI_API_KEY,
                        DEEPSEEK_API_KEY, GEMINI_API_KEY, XAI_API_KEY,
                        OPENROUTER_API_KEY, MOONSHOT_API_KEY, ZAI_API_KEY,
                        GROQ_API_KEY). Stored credentials win over env vars.
  RUST_LOG              tracing filter for diagnostics
  EPSILON_STREAM_IDLE_SECS
                        seconds of stream silence before a turn fails as a network
                        error (default 120, 0 disables)

FILES
  ~/.config/epsilon/config.toml   model, picker favorites, custom providers,
                                  [oauth.<provider>] client ids, [editor] command
  ~/.config/epsilon/auth.json     stored credentials (chmod 600 where supported)
  ~/.config/epsilon/MEMORY.md     global memory injected into the prompt
  ~/.config/epsilon/history       prompt history
  .epsilon/sessions/*.jsonl       session transcripts for this workspace
  .epsilon/config.toml            project config (trust-gated)
  AGENTS.override.md, AGENTS.md, CLAUDE.md, MEMORY.md
                                  project instructions and memory, injected into the
                                  prompt's CONTEXT tier; instruction files are read from
                                  the workspace and every directory above it
  .epsilon/SYSTEM.md              replaces the built-in agent contract (trust-gated)
  .epsilon/APPEND_SYSTEM.md       appended to the system prompt (trust-gated)
  .epsilon/skills, .agents/skills, ~/.config/epsilon/skills
                                  SKILL.md skill folders (progressive disclosure)

EXAMPLES
  epsilon                                  # start working in this repo
  epsilon -p \"summarize the failing test\"   # one-shot, pipe-friendly
  epsilon --model deepseek-v4-flash -p hi   # pick a model explicitly
  epsilon auth login anthropic --oauth      # subscription login
  epsilon -c -p \"keep going\"                # continue the newest session

Exit codes: 0 success · 1 run or provider failure · 2 bad usage.";

/// The full help text, without the trailing newline ceremony (kept as a helper so
/// tests can assert on it).
fn help_text() -> &'static str {
    HELP
}

struct Args {
    mode: Mode,
    script_path: Option<String>,
    workspace: Option<PathBuf>,
    mcp_servers: Vec<McpServerConfig>,
    port: u16,
    confinement: epsilon_tools::Confinement,
    /// M79: ask a language server about edited files (`--lsp auto|off`).
    lsp: epsilon_tools::Lsp,
    /// Explicit --provider; None = auto-detect from env keys (M14).
    provider: Option<Provider>,
    /// `--model` override; None falls back to saved config then the catalog default.
    model_id: Option<String>,
    /// `--fallback <model>`: extra candidates tried in order when the primary
    /// fails with a retryable error (rate limit, 5xx, network). Repeatable.
    fallbacks: Vec<String>,
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
    /// M76: diagnose the on-ramp — model, credentials, sandbox, session, trust — and exit.
    Doctor,
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
        /// M52: run the provider's OAuth ceremony (`--oauth`) instead of
        /// storing a pasted API key.
        oauth: bool,
        /// M52: skip the loopback callback listener and ask for a pasted
        /// redirect URL (browser on another machine / ssh).
        paste: bool,
    },
    /// Remove a provider's stored key.
    Logout { provider: String },
}

fn parse_args() -> Args {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    if raw
        .iter()
        .any(|a| a == "--help" || a == "-h" || a == "help")
    {
        println!("{}", help_text());
        std::process::exit(0);
    }
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
                // M52: `--oauth` starts the subscription ceremony, `--paste`
                // forces the manual-redirect path, a bare 4th token is the
                // API key (M22 behavior, unchanged).
                let mut oauth = false;
                let mut paste = false;
                let mut key: Option<String> = None;
                for arg in raw.iter().skip(3) {
                    match arg.as_str() {
                        "--oauth" | "oauth" => oauth = true,
                        "--paste" => paste = true,
                        other if other.starts_with("--") => {
                            die(&format!("unknown auth login flag '{other}'"))
                        }
                        other => {
                            if key.is_none() {
                                key = Some(other.to_string());
                            } else {
                                die(&format!("unexpected extra argument '{other}'"));
                            }
                        }
                    }
                }
                Mode::Auth(AuthAction::Login {
                    provider,
                    key,
                    oauth,
                    paste,
                })
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
            lsp: epsilon_tools::Lsp::Auto,
            provider: None,
            model_id: None,
            fallbacks: Vec::new(),
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
    let mut lsp = epsilon_tools::Lsp::Auto;
    // --plan: read-only research run (opencode plan-agent parity).
    let mut plan_mode = false;
    // --yes: unattended runs approve gated asks (explicit opt-in; default is deny).
    let mut auto_approve = false;
    let mut provider: Option<Provider> = None;
    let mut resume = Resume::default();
    // None = not given on the command line; resolution falls back to the saved
    // config value, then the catalog default (ADR-018 precedence).
    let mut model_id: Option<String> = None;
    let mut fallbacks: Vec<String> = Vec::new();
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
            "--lsp" => {
                i += 1;
                let mode = raw
                    .get(i)
                    .cloned()
                    .unwrap_or_else(|| die("--lsp requires auto|off"));
                lsp = match mode.as_str() {
                    "auto" => epsilon_tools::Lsp::Auto,
                    "off" => epsilon_tools::Lsp::Off,
                    other => die(&format!("unknown --lsp mode: {other} (want auto|off)")),
                };
            }
            "--sandbox" => {
                i += 1;
                let mode = raw
                    .get(i)
                    .cloned()
                    .unwrap_or_else(|| die("--sandbox requires auto|read-only|off"));
                confinement = match mode.as_str() {
                    "auto" => epsilon_tools::Confinement::Auto,
                    // The read-only rung of the mode ladder: commands may look, not touch.
                    "read-only" | "readonly" => epsilon_tools::Confinement::ReadOnly,
                    "off" => {
                        eprintln!(
                            "WARNING: OS confinement disabled (--sandbox off); commands run unconfined"
                        );
                        epsilon_tools::Confinement::Off
                    }
                    other => die(&format!(
                        "unknown --sandbox mode: {other} (want auto|read-only|off)"
                    )),
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
            "--fallback" => {
                i += 1;
                fallbacks.push(
                    raw.get(i).cloned().unwrap_or_else(|| {
                        die("--fallback requires a model id (e.g. deepseek-chat)")
                    }),
                );
            }
            "--doctor" => mode = Mode::Doctor,
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
            // M76: the bare subcommand form (`epsilon doctor`), which is how the docs and the
            // usage line spell it. It is a positional, so it must be recognised here — before the
            // unknown-argument arm turns a documented command into a usage dump.
            "doctor" if i == 0 => mode = Mode::Doctor,
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
        lsp,
        provider,
        model_id,
        fallbacks,
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
    // M76: the diagnosis runs before the runtime exists. It needs no agent, no lease and no
    // network — a diagnostic that first builds the machinery cannot diagnose the machinery
    // (and building one prints the startup banner over the report).
    if matches!(args.mode, Mode::Doctor) {
        std::process::exit(doctor_mode(&args));
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
///
/// Mask a stored key for `auth list`: short keys hide entirely; longer ones
/// show only the last 4 characters. A bug here leaks secrets to screenshots.
/// Char-boundary safe: a hand-edited auth.json with a non-ASCII tail must not
/// panic (the old byte-index slice could).
fn mask_key(key: &str) -> String {
    let total = key.chars().count();
    if total <= 4 {
        return "****".to_string();
    }
    let last4: String = key
        .chars()
        .rev()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("…{last4}")
}

fn auth_command(action: &AuthAction) -> i32 {
    match action {
        AuthAction::List => {
            let creds = epsilon_llm::auth::load_credentials();
            if creds.is_empty() {
                println!("no stored credentials (~/.config/epsilon/auth.json)");
                println!("add one:  epsilon auth login <provider>");
                return 0;
            }
            println!("stored credentials (~/.config/epsilon/auth.json):");
            for (provider, cred) in &creds {
                match cred {
                    epsilon_llm::auth::StoredCredential::Api { key, .. } => {
                        println!("  {provider:<14} {}", mask_key(key.trim()));
                    }
                    epsilon_llm::auth::StoredCredential::Oauth { expires, .. } => {
                        println!("  {provider:<14} oauth (expires {expires})");
                    }
                }
            }
            0
        }
        AuthAction::Login {
            provider,
            key,
            oauth,
            paste,
        } => {
            let provider = provider.trim().to_string();
            if *oauth {
                // M52: subscription login (pi `auth/oauth/*.ts` login half).
                return login_oauth(&provider, *paste);
            }
            if *paste {
                eprintln!("--paste only applies to --oauth logins");
                return 2;
            }
            // M48: `google-vertex` needs project + location alongside the key
            // (pi's login prompts for all three). Stored in the credential's
            // `env` map — the same place pi's `credential.env` keeps them —
            // so `wire_for` picks them up without extra plumbing.
            if provider == "google-vertex" {
                return login_google_vertex(key);
            }
            // M49: `amazon-bedrock` offers bearer-token XOR the ambient AWS
            // chain (pi's login select: bearer token / AWS profile /
            // credential chain). A stored key = bearer token; profile/chain
            // choices store a marker + env (the chain material itself stays
            // in env/files, never copied into our store — pi's resolve
            // detects it ambiently too).
            if provider == "amazon-bedrock" {
                return login_amazon_bedrock(key);
            }
            let key = match key {
                Some(k) => k.clone(),
                None => {
                    // Interactive: read a line without echoing when possible.
                    // No echo suppression is wired yet: the key IS visible as
                    // you type (documented limitation, M22 log). The prompt
                    // must not claim otherwise.
                    eprint!("API key for {provider} (visible while typing): ");
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

/// `epsilon auth login <provider> --oauth [--paste]` (M52).
///
/// Runs the provider's OAuth ceremony: PKCE providers open the browser and
/// capture the redirect on a loopback listener (or a paste with `--paste`),
/// device providers print a user code and poll. The resulting credential
/// lands in `~/.config/epsilon/auth.json` through the same store the API-key
/// path uses, so the next request picks it up with no restart.
#[cfg(feature = "real")]
fn login_oauth(provider: &str, paste_only: bool) -> i32 {
    use epsilon_llm::oauth_login::{LoginSaved, login, oauth_providers};
    if !oauth_providers().contains(&provider) {
        eprintln!(
            "provider '{provider}' has no OAuth login here — available: {}",
            oauth_providers().join(", ")
        );
        eprintln!("API-key login still works: epsilon auth login {provider} <key>");
        return 2;
    }
    println!(
        "starting OAuth login for {provider}{} — client id from the environment or ~/.config/epsilon/config.toml",
        if paste_only { " (paste mode)" } else { "" }
    );
    let mut ui = CliLoginUi {
        stdin_rx: None,
        announce_paste: true,
        paste_only,
    };
    match login(provider, &mut ui, !paste_only) {
        Ok(LoginSaved::Oauth {
            provider,
            path,
            expires,
        }) => {
            println!(
                "stored {provider} OAuth tokens (expires {expires}) in {}",
                path.display()
            );
            println!("next request uses them — no restart needed");
            0
        }
        Ok(LoginSaved::Key { provider, path }) => {
            println!("stored {provider} key in {}", path.display());
            0
        }
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

/// Offline builds (no `real` feature) cannot reach a token endpoint; say so
/// instead of pretending the flag exists.
#[cfg(not(feature = "real"))]
fn login_oauth(provider: &str, _paste_only: bool) -> i32 {
    eprintln!(
        "OAuth login for '{provider}' needs the real-provider build:\n  \
         cargo install --path crates/epsilon-cli --features real --force"
    );
    2
}

/// Terminal implementation of the login surface: prints progress, shows the
/// device code, and races the loopback callback against a pasted redirect.
#[cfg(feature = "real")]
struct CliLoginUi {
    stdin_rx: Option<std::sync::mpsc::Receiver<String>>,
    announce_paste: bool,
    paste_only: bool,
}

#[cfg(feature = "real")]
impl CliLoginUi {
    /// Start (once) a reader thread so a paste can arrive while the callback
    /// listener is armed — pi's dual-input path.
    fn stdin_lines(&mut self) -> &std::sync::mpsc::Receiver<String> {
        if self.stdin_rx.is_none() {
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                use std::io::BufRead;
                let stdin = std::io::stdin();
                for line in stdin.lock().lines() {
                    match line {
                        Ok(line) => {
                            if tx.send(line.trim().to_string()).is_err() {
                                return;
                            }
                        }
                        Err(_) => return,
                    }
                }
            });
            self.stdin_rx = Some(rx);
        }
        self.stdin_rx.as_ref().expect("just created")
    }
}

#[cfg(feature = "real")]
impl epsilon_llm::oauth_login::LoginUi for CliLoginUi {
    fn notify(&mut self, message: &str) {
        println!("{message}");
    }

    fn device_code(&mut self, prompt: &epsilon_llm::oauth_login::DevicePrompt) {
        println!();
        println!("  1. open {}", prompt.verification_uri);
        println!("  2. enter the code: {}", prompt.user_code);
        println!();
    }

    fn wait_for_code(
        &mut self,
        callback: Option<&epsilon_llm::oauth_login::CallbackListener>,
    ) -> Option<String> {
        if self.paste_only || callback.is_none() {
            println!("paste the authorization code (or the full redirect URL) and press Enter:");
            return self.stdin_lines().recv().ok();
        }
        let callback = callback.expect("checked");
        if self.announce_paste {
            self.announce_paste = false;
            println!(
                "waiting for the browser callback — if your browser is on another machine,\n\
                 paste the redirect URL from its address bar here and press Enter instead"
            );
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(600);
        loop {
            if let Some(line) = self.stdin_lines().try_recv().ok().filter(|l| !l.is_empty()) {
                return Some(line);
            }
            match callback.try_recv() {
                Some(Ok(code)) => {
                    println!("received the redirect from the browser");
                    return Some(code.code);
                }
                Some(Err(e)) => {
                    eprintln!("{e}");
                    return None;
                }
                None => {}
            }
            if std::time::Instant::now() >= deadline {
                eprintln!("login timed out — nothing stored");
                return None;
            }
            std::thread::sleep(std::time::Duration::from_millis(60));
        }
    }
}

/// `epsilon auth login google-vertex [key]`: pi's Vertex login asks for the
/// auth choice first (API key vs ADC/service-account), then project +
/// location. Non-interactive use passes the key argv and the two env vars;
/// interactive use is prompted for each. The key may be empty (ADC path) —
/// but then `GOOGLE_APPLICATION_CREDENTIALS` (or the ADC default path) plus
/// project + location must resolve, or login refuses to store a dead entry.
fn login_google_vertex(key_arg: &Option<String>) -> i32 {
    use std::collections::BTreeMap;
    fn prompt_line(label: &str) -> String {
        eprint!("{label}: ");
        let mut line = String::new();
        std::io::BufRead::read_line(&mut std::io::stdin().lock(), &mut line).unwrap_or(0);
        line.trim().to_string()
    }
    let key = key_arg.clone().unwrap_or_default();
    let key = if key.trim().is_empty() {
        eprint!("Google Cloud API key (empty = use Application Default Credentials): ");
        read_secret_line().unwrap_or_default()
    } else {
        key
    };
    let project = std::env::var("GOOGLE_CLOUD_PROJECT")
        .or_else(|_| std::env::var("GCLOUD_PROJECT"))
        .map(|v| v.trim().to_string())
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| prompt_line("Google Cloud project ID"));
    if project.is_empty() {
        eprintln!("no project entered; nothing stored");
        return 2;
    }
    let location = std::env::var("GOOGLE_CLOUD_LOCATION")
        .map(|v| v.trim().to_string())
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| prompt_line("Google Cloud location (e.g. us-central1)"));
    if location.is_empty() {
        eprintln!("no location entered; nothing stored");
        return 2;
    }
    let credentials_file = std::env::var("GOOGLE_APPLICATION_CREDENTIALS")
        .map(|v| v.trim().to_string())
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| {
            if is_vertex_placeholder_key(&key) {
                prompt_line(
                    "Service-account file [empty = default ADC path ~/.config/gcloud/application_default_credentials.json]",
                )
            } else {
                String::new()
            }
        });
    if is_vertex_placeholder_key(&key) && credentials_file.is_empty() && !default_adc_file_exists()
    {
        eprintln!(
            "no API key and no ADC file found — run `gcloud auth application-default login`, pass a key, or set GOOGLE_APPLICATION_CREDENTIALS"
        );
        return 2;
    }
    let mut env = BTreeMap::new();
    env.insert("GOOGLE_CLOUD_PROJECT".to_string(), project);
    env.insert("GOOGLE_CLOUD_LOCATION".to_string(), location);
    if !credentials_file.is_empty() {
        env.insert(
            "GOOGLE_APPLICATION_CREDENTIALS".to_string(),
            credentials_file,
        );
    }
    match epsilon_llm::auth::save_credential_full("google-vertex", key.trim(), Some(env)) {
        Ok(path) => {
            println!("stored google-vertex credentials in {}", path.display());
            0
        }
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

/// Placeholder keys pi treats as "no real key, use ADC instead" (pi
/// `resolveApiKey` + `isPlaceholderApiKey`, mirrored from the Vertex wire so
/// the login path compiles without the `real`-gated provider module).
fn is_vertex_placeholder_key(key: &str) -> bool {
    let k = key.trim();
    k.is_empty()
        || k == "gcp-vertex-credentials"
        || (k.starts_with('<') && k.ends_with('>') && k.len() >= 2)
}

/// Whether the default ADC file exists (pi `VERTEX_ADC_PATH`).
fn default_adc_file_exists() -> bool {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map(|h| {
            std::path::Path::new(&h)
                .join(".config/gcloud/application_default_credentials.json")
                .exists()
        })
        .unwrap_or(false)
}

/// `epsilon auth login amazon-bedrock [key]`: pi's Bedrock login is a
/// three-way select (bearer token / AWS profile / existing credential
/// chain). Non-interactive use passes the bearer token argv (or nothing for
/// chain); interactive use picks 1/2/3. Profile names store into the
/// credential `env` map (`AWS_PROFILE` — pi's stored-env shape); chain
/// choice stores an empty-key marker so `credentials_ready` consults the
/// ambient chain instead of failing on "no key".
fn login_amazon_bedrock(key_arg: &Option<String>) -> i32 {
    use std::collections::BTreeMap;
    if let Some(k) = key_arg.as_ref().filter(|k| !k.trim().is_empty()) {
        return match epsilon_llm::auth::save_credential("amazon-bedrock", k.trim()) {
            Ok(path) => {
                println!("stored amazon-bedrock bearer token in {}", path.display());
                0
            }
            Err(e) => {
                eprintln!("{e}");
                1
            }
        };
    }
    eprintln!("Select Amazon Bedrock authentication method:");
    eprintln!("  1) Bearer token (bedrock:CallWithBearerToken)");
    eprintln!("  2) AWS profile (reads ~/.aws/credentials)");
    eprintln!("  3) Existing AWS credential chain (env / ECS / instance role)");
    eprint!("choice [1/2/3]: ");
    let mut line = String::new();
    std::io::BufRead::read_line(&mut std::io::stdin().lock(), &mut line).unwrap_or(0);
    match line.trim() {
        "1" => {
            eprint!("Bedrock bearer token (visible while typing): ");
            match read_secret_line() {
                Some(k) if !k.trim().is_empty() => {
                    match epsilon_llm::auth::save_credential("amazon-bedrock", k.trim()) {
                        Ok(path) => {
                            println!("stored amazon-bedrock bearer token in {}", path.display());
                            0
                        }
                        Err(e) => {
                            eprintln!("{e}");
                            1
                        }
                    }
                }
                _ => {
                    eprintln!("no token entered; nothing stored");
                    2
                }
            }
        }
        "2" => {
            eprint!("AWS profile name: ");
            let mut profile = String::new();
            std::io::BufRead::read_line(&mut std::io::stdin().lock(), &mut profile).unwrap_or(0);
            let profile = profile.trim().to_string();
            if profile.is_empty() {
                eprintln!("no profile entered; nothing stored");
                return 2;
            }
            let mut env = BTreeMap::new();
            env.insert("AWS_PROFILE".to_string(), profile.clone());
            // Empty key + profile env: `resolve_api_key` yields None (no
            // bearer), the transport reads AWS_PROFILE itself (pi parity).
            // `save_credential_full` refuses empty keys, so write the marker
            // entry directly through the `_in` API with a placeholder the
            // resolver treats as absent.
            match epsilon_llm::auth::save_credential_full(
                "amazon-bedrock",
                "USE_AMBIENT_AWS_CHAIN",
                Some(env),
            ) {
                Ok(path) => {
                    println!(
                        "stored amazon-bedrock profile ({profile}) in {}",
                        path.display()
                    );
                    0
                }
                Err(e) => {
                    eprintln!("{e}");
                    1
                }
            }
        }
        "3" => {
            eprintln!(
                "Configure AWS credentials in your environment, then press Enter to continue."
            );
            let mut line = String::new();
            std::io::BufRead::read_line(&mut std::io::stdin().lock(), &mut line).unwrap_or(0);
            match epsilon_llm::auth::save_credential_full(
                "amazon-bedrock",
                "USE_AMBIENT_AWS_CHAIN",
                None,
            ) {
                Ok(path) => {
                    println!("stored amazon-bedrock chain marker in {}", path.display());
                    0
                }
                Err(e) => {
                    eprintln!("{e}");
                    1
                }
            }
        }
        _ => {
            eprintln!("unknown method; nothing stored");
            2
        }
    }
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
    // M45: remembered per-path trust (pi `trust.json`) composes with the hash:
    // an explicit `false` denies even an unchanged config; `true` trusts even
    // a changed one (the user already answered); `None`/unreadable falls back
    // to the hash decision. "No memory" is never a decision.
    let remembered: Option<bool> =
        epsilon_cli::runtime::ProjectTrustStore::open().and_then(|s| s.get(&workspace));
    let trusted_now = match remembered {
        Some(false) => false,
        Some(true) => true,
        None => !matches!(trust, epsilon_cli::runtime::TrustDecision::Changed { .. }),
    };
    // M39: untrusted projects curtail skills/extensions/prompts at the roots —
    // the config hash gates config.toml, but third-party CODE never loads
    // without trust (pi's project-trust scoping).
    if !trusted_now {
        startup_notices.push(
            "project skills/extensions curtailed until trusted \
             (global skills still loaded)"
                .to_string(),
        );
    }
    let trust_review_pending = matches!(trust, epsilon_cli::runtime::TrustDecision::Changed { .. });
    if trust_review_pending {
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
    //
    // M40: the raw flag/config/env candidate is a *reference*, not an id — it
    // goes through pi's match order (exact → provider/id → case-insensitive →
    // partial) so `--model opus`, `anthropic/claude-opus-4-6`, or a bare alias
    // all land on a catalog row. Custom `<name>/...` ids bypass the catalog
    // (M28) but are still accepted. Unknown references die with candidates.
    let candidate_raw = args
        .model_id
        .clone()
        .or_else(|| project.model_id.clone())
        .or_else(epsilon_core::config::load_model_id);
    let candidate_model: Option<String> = match candidate_raw.as_deref() {
        None => None,
        Some(raw) => {
            use epsilon_llm::catalog::resolve_model_for_run;
            if let Some(resolved) = resolve_model_for_run(raw) {
                if !matches!(resolved.kind, epsilon_llm::catalog::ModelMatchKind::Exact) {
                    diag.push(format!(
                        "model {:?} matched catalog entry {:?}",
                        raw, resolved.info.id
                    ));
                }
                // M63: never let a name collide with a capability. When the row
                // the reference landed on has no credentials but a sibling row
                // for the same reference does, the harness runs the one the user
                // can actually call — and says so, because silently running a
                // different model would be worse than saying it.
                if let Some(replaced) = resolved.replaced {
                    diag.push(format!(
                        "{} has no credentials — using {:?} ({}) instead",
                        replaced.id, resolved.info.id, resolved.info.provider
                    ));
                } else if !epsilon_llm::catalog::row_credentials_ready(resolved.info)
                    && !resolved.info.env_key.is_empty()
                {
                    // The requested model cannot run: name the provider, the env
                    // var, the login command, and which providers DO have keys.
                    diag.push(format!(
                        "no credentials for provider {:?} — set {} or run: \
                         epsilon auth login {}",
                        resolved.info.provider, resolved.info.env_key, resolved.info.provider
                    ));
                    let ready = epsilon_llm::catalog::credentialed_providers();
                    if !ready.is_empty() {
                        diag.push(format!(
                            "providers with stored credentials: {} (pick one with --model)",
                            ready.join(", ")
                        ));
                    }
                    // Naming the missing provider is only half an answer; give the
                    // user a model they can run right now.
                    if let Some(suggestion) = epsilon_llm::catalog::suggested_ready_model() {
                        diag.push(format!(
                            "try: --model {} ({})",
                            suggestion.id, suggestion.provider
                        ));
                    }
                }
                Some(resolved.info.id.to_string())
            } else if raw.split_once('/').is_some_and(|(prefix, _)| {
                epsilon_llm::custom::load_custom_providers()
                    .iter()
                    .any(|c| {
                        epsilon_llm::custom::lookup_custom(std::slice::from_ref(c), raw).is_some()
                            || c.name.eq_ignore_ascii_case(prefix)
                    })
            }) {
                Some(raw.to_string())
            } else {
                let candidates = epsilon_llm::catalog::resolve_model_candidates(raw);
                if candidates.is_empty() {
                    die(&format!(
                        "unknown model {raw:?} (not in catalog, no custom provider prefix matches)"
                    ));
                }
                die(&format!(
                    "ambiguous model {raw:?} — matches: {}",
                    candidates
                        .iter()
                        .map(|m| m.id)
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
        }
    };

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
            // M28: custom ids (`lmstudio/foo`) resolve under the custom provider's
            // name, so the diagnostic names it correctly for those too.
            let customs = epsilon_llm::custom::load_custom_providers();
            let provider = epsilon_llm::custom::lookup_custom(&customs, &model_id)
                .map(|c| c.name.clone())
                .or_else(|| {
                    epsilon_llm::catalog::lookup_model(model_id.as_str())
                        .map(|m| m.provider.to_string())
                })
                .unwrap_or_default();
            let key_source = if epsilon_llm::auth::load_credentials().contains_key(&provider) {
                "stored (epsilon auth login)"
            } else {
                "env var"
            };
            diag.push(format!(
                "provider: real (direct HTTP) · model: {model_id} · key {key_source}"
            ));
            // DIRECTION-03: with --fallback, wrap in the chain provider
            // (primary first, candidates after; per-turn primary restoration).
            // M60: the retry policy is engaged here (it was implemented but never
            // wrapped around a production stream, so transient failures were fatal).
            // Waits go to stderr: stdout stays exactly the transcript for `-p`.
            let retrying = |inner: Arc<dyn epsilon_llm::StreamFn>| {
                Arc::new(
                    epsilon_llm::Retrying::new(inner)
                        .with_max_attempts(epsilon_llm::RETRY_MAX_ATTEMPTS)
                        .with_on_retry(|n| {
                            eprintln!("[retry] {}", epsilon_core::retry_notice_text(n))
                        }),
                )
            };
            if args.fallbacks.is_empty() {
                retrying(Arc::new(PanicGuard(epsilon_llm::DirectProvider::new())))
            } else {
                diag.push(format!("fallbacks: {}", args.fallbacks.join(" -> ")));
                retrying(Arc::new(PanicGuard(
                    epsilon_llm::fallback::FallbackProvider::new(
                        std::iter::once(model_id.as_str())
                            .chain(args.fallbacks.iter().map(String::as_str)),
                    ),
                )))
            }
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
    // Silence the never-read warning in faux builds: fallbacks only matter on
    // the real path (see the use_real branch above).
    #[cfg(not(feature = "real"))]
    let _ = &args.fallbacks;
    // Startup notices flush here, in order, once (plan 008).
    diag.flush_stderr();

    let shared = SharedRun {
        workspace,
        stream_fn,
        mcp_servers: args.mcp_servers.clone(),
        confinement: args.confinement.clone(),
        lsp: args.lsp,
        model_id,
        resume: args.resume.clone(),
        project,
        project_trusted: trusted_now,
        plan_mode: args.plan_mode,
        startup_notices,
        trust_review_pending,
    };
    match &args.mode {
        Mode::Auth(_) => unreachable!("auth subcommands exit inside main() before dispatch"),
        Mode::Doctor => unreachable!("--doctor exits inside main() before the runtime is built"),
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
        lsp: shared.lsp,
        model_id: &shared.model_id,
        resume: &shared.resume,
        project: &shared.project,
        project_trusted: shared.project_trusted,
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
    lsp: epsilon_tools::Lsp,
    model_id: String,
    resume: Resume,
    project: epsilon_cli::runtime::ProjectConfig,
    /// M39: config-hash trust at startup (`Changed` ⇒ false). Modes pass it
    /// into the trusted root variants so project code never loads untrusted.
    project_trusted: bool,
    plan_mode: bool,
    /// Project-config notices + trust banner (M22 / DIRECTION-01).
    startup_notices: Vec<String>,
    /// Typed trust-review trigger for the TUI gate (audit: no string sniffing).
    trust_review_pending: bool,
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
    lsp: epsilon_tools::Lsp,
    model_id: &'a str,
    resume: &'a Resume,
    project: &'a epsilon_cli::runtime::ProjectConfig,
    /// M39: config-hash trust from dispatch (`Changed` ⇒ false). Gates the
    /// trusted root variants so project code never loads untrusted.
    project_trusted: bool,
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
        lsp,
        model_id,
        resume,
        project,
        project_trusted,
        plan_mode,
    }: RunParams<'_>,
) -> Result<Arc<Agent>, String> {
    // M79: the product path opts into language-server diagnostics; library callers do not.
    let mut registry = epsilon_tools::builtin_registry_with(confinement, lsp);
    let mut all_mcp: Vec<McpServerConfig> = mcp_servers.to_vec();
    all_mcp.extend(project.mcp_servers.iter().cloned());
    for cfg in &all_mcp {
        let attached = epsilon_mcp::attach_stdio_server(&mut registry, cfg).await?;
        eprintln!("mcp server '{}' attached ({attached} tools)", cfg.name);
    }

    // Skills (M8 brick, wired M11): conventional roots feed the CONTEXT-tier catalog.
    // M39: untrusted projects see global roots only.
    let skills = epsilon_core::skills::discover(&epsilon_cli::runtime::skill_roots_trusted(
        workspace,
        project_trusted,
    ));
    // The catalog tells the model a skill exists; the `skill` tool is how it opens
    // one. Registering the tool from the same list keeps the two from disagreeing —
    // and a skill the user marked as not-model-invocable is filtered out here, so
    // the model cannot reach it even by guessing its name.
    let loadable: Vec<epsilon_tools::skill::LoadableSkill> = skills
        .iter()
        .filter(|s| !s.disable_model_invocation)
        .map(|s| epsilon_tools::skill::LoadableSkill {
            name: s.name.clone(),
            description: s.description.clone(),
            dir: s.dir.clone(),
        })
        .collect();
    if !loadable.is_empty() {
        registry.register(std::sync::Arc::new(epsilon_tools::skill::SkillTool::new(
            loadable,
        )));
    }
    if !skills.is_empty() {
        let names: Vec<&str> = skills.iter().map(|s| s.name.as_str()).collect();
        eprintln!("skills: {}", names.join(", "));
    }

    // Extensions (M8/M9a bricks, wired M11): conventional roots PLUS project-declared
    // dirs resolved against the workspace's .epsilon/ (ADR-012 project layer, M12).
    // M39: project-declared dirs join ONLY when trusted — an untrusted config
    // must not smuggle extension code in through its own declarations.
    let mut ext_roots = epsilon_cli::runtime::extension_roots_trusted(workspace, project_trusted);
    if project_trusted && !project.extension_dirs.is_empty() {
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
        let mut layers = vec![epsilon_safety::plan_ruleset()];
        if !project.rules.is_empty() {
            layers.push(epsilon_cli::runtime::project_rules_layer(&project.rules));
        }
        epsilon_safety::PermissionEngine::layered(layers)
    } else if project.rules.is_empty() {
        epsilon_safety::PermissionEngine::with_defaults()
    } else {
        epsilon_safety::PermissionEngine::layered(vec![epsilon_cli::runtime::project_rules_layer(
            &project.rules,
        )])
    };
    let gate = Arc::new(RulesetGate::layered(base_engine, approvals));
    let agent = Agent::new(
        LoopLimits::for_model(model_id),
        stream_fn,
        Arc::new(registry),
        gate,
        workspace.to_path_buf(),
    );
    let mut store = epsilon_session::SessionStore::open_in(workspace.join(session_file), workspace)
        .map_err(|e| format!("open session store: {e}"))?;
    if store.migrated() {
        match store.persist_migration() {
            Ok(()) => eprintln!("migrated session store to current version"),
            Err(e) => eprintln!("migration warning (continuing): {e}"),
        }
    }
    agent.set_model_id(model_id);
    agent.set_skills(skills);
    // Identity tier: `.epsilon/SYSTEM.md` replaces the built-in contract and
    // `.epsilon/APPEND_SYSTEM.md` extends it (pi `discoverSystemPromptFile`). The same trust
    // decision that gates skills decides whether the project's copy loads, so the prompt the
    // user is asked about at startup is the prompt that takes effect; the global pair under
    // ~/.config/epsilon is the fallback. Announced, because "I wrote SYSTEM.md, was asked to
    // trust it, and nothing changed" is the report this wiring exists to answer.
    let overlay = epsilon_core::prompt::load_system_overlay(workspace, project_trusted);
    if let Some(path) = &overlay.custom_path {
        eprintln!("system prompt: {path} replaces the built-in contract");
    }
    if let Some(path) = &overlay.append_path {
        eprintln!("system prompt: {path} appended");
    }
    agent.set_system_overlay(overlay);
    // Agent defs (M50: pi `agents.ts` parity) — user roots always, project
    // roots only when trusted (same gate as skills). The `task` tool routes
    // `subagent_type` through these; unknown named types fail naming the list.
    let agents = epsilon_core::agents::discover_agents(
        &epsilon_cli::runtime::agent_user_roots(),
        &epsilon_cli::runtime::agent_project_roots(workspace, project_trusted),
        project_trusted,
    );
    if !agents.is_empty() {
        let names: Vec<&str> = agents.iter().map(|a| a.name.as_str()).collect();
        eprintln!("agents: {}", names.join(", "));
    }
    agent.set_agents(agents);
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
        // M41: a resumed file may predate headers — migrate before repair reads
        // it, so the repair pass sees healed entries, never legacy shapes.
        if restored.migrated() {
            match restored.persist_migration() {
                Ok(()) => eprintln!("migrated {} to current version", rp.display()),
                Err(e) => eprintln!("migration warning (continuing): {e}"),
            }
        }
        // ponytail-audit follow-up: repair is now WIRED, not test-only — a crashed
        // prior run leaves dangling tool calls on the active branch; synthesize
        // tombstones before the model view so the resumed context stays provider-valid.
        match epsilon_session::repair::repair_on_open(&mut restored) {
            Ok(report) => {
                for line in report.summary() {
                    eprintln!("repaired {}: {line}", rp.display());
                }
            }
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

/// Build the user message for `text`, resolving any `@` references into attachments (M74).
///
/// Every frontend goes through `epsilon_core::references::user_message`, so `@file` behaves the
/// same in `-p`, the RPC loop and the TUI — a feature that works in one of them only is a feature
/// the others are missing.
fn user_message(workspace: &std::path::Path, text: String) -> Message {
    epsilon_core::references::user_message(workspace, &text)
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
        lsp: shared.lsp,
        model_id: &shared.model_id,
        resume: &shared.resume,
        project: &shared.project,
        project_trusted: shared.project_trusted,
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
    match agent.run(user_message(&shared.workspace, task)).await {
        Ok(outcome) => {
            // Salvage rule (M62) applied to the CLI: a run that failed after the
            // model had already said something still prints that something — what
            // the user watched arrive is never thrown away — and the failure is
            // reported on stderr with its real reason. Before this, `-p` printed
            // "no final assistant text produced" for *any* non-EndTurn ending, so
            // a 401/429/context-overflow looked like an empty response with no
            // explanation anywhere (M63).
            let (text, diagnostic) = print_outcome(&outcome);
            if let Some(text) = text {
                println!("{text}");
            }
            match diagnostic {
                None => 0,
                Some(d) => {
                    eprintln!("{d}");
                    1
                }
            }
        }
        Err(e) => {
            eprintln!("run failed: {e}");
            1
        }
    }
}

/// What a finished run should show a headless caller: the reply text to print on
/// stdout (if any) and a diagnostic for stderr (if the run did not finish cleanly).
///
/// Pure — no printing — so the salvage/report rule is unit-testable without a
/// terminal. Two cases matter:
///
/// - clean `EndTurn` with text: text out, no diagnostic, exit 0;
/// - anything else: whatever text the model managed to deliver is still returned
///   (M62's "never discard visible output" rule), plus one line naming the real
///   stop reason and the provider's own message when there is one.
fn print_outcome(outcome: &epsilon_core::RunOutcome) -> (Option<String>, Option<String>) {
    let last_assistant = outcome.produced.iter().rev().find_map(|m| match m {
        Message::Assistant(a) => Some(a),
        _ => None,
    });
    let text = last_assistant
        .map(|a| a.text())
        .filter(|t| !t.trim().is_empty());
    let stop = last_assistant.map(|a| a.stop_reason.clone());
    let clean = matches!(stop, Some(StopReason::EndTurn));
    if clean && text.is_some() {
        return (text, None);
    }
    let detail = last_assistant
        .and_then(|a| a.error_message.clone())
        .filter(|s| !s.trim().is_empty());
    let reason = match (&outcome.reason, &stop) {
        (_, Some(StopReason::Aborted)) => "aborted by user".to_string(),
        (TurnEndReason::Error(e), _) if !e.is_empty() => e.clone(),
        (TurnEndReason::MaxTokens, _) => "the model hit its output-token limit".to_string(),
        (TurnEndReason::BudgetExhausted, _) => {
            "the turn budget ran out before the task finished".to_string()
        }
        (TurnEndReason::Blocked, _) => "a permission rule blocked this run".to_string(),
        (other, _) => format!("{other:?}"),
    };
    let diagnostic = match detail {
        Some(d) => format!("run failed: {d}"),
        None if text.is_none() => {
            format!("run produced no reply and did not finish cleanly: {reason}")
        }
        None => format!("run did not finish cleanly: {reason} — the reply above may be partial"),
    };
    (text, Some(diagnostic))
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
        lsp: shared.lsp,
        model_id: &shared.model_id,
        resume: &shared.resume,
        project: &shared.project,
        project_trusted: shared.project_trusted,
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
                trust_review_pending: shared.trust_review_pending,
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
        lsp: shared.lsp,
        model_id: &shared.model_id,
        resume: &shared.resume,
        project: &shared.project,
        project_trusted: shared.project_trusted,
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
        // Resolved here, not inside the task: `@` references read the workspace, and the
        // reference to `shared` cannot outlive the function.
        let message = user_message(&shared.workspace, task);
        tokio::spawn({
            let agent = Arc::clone(&agent);
            async move {
                let _ = agent.run(message).await;
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
/// `epsilon doctor`: print the diagnosis and exit with its verdict (M76).
///
/// Deliberately builds no runtime and takes no lease: a diagnosis that needs the machinery to work
/// cannot diagnose the machinery. `--doctor` is handled before `SharedRun` construction for the
/// same reason.
fn doctor_mode(args: &Args) -> i32 {
    let workspace = args
        .workspace
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    let inputs = epsilon_cli::doctor::Inputs {
        workspace: &workspace,
        model_arg: args.model_id.as_deref(),
        confinement: &args.confinement,
        mcp_server_count: args.mcp_servers.len(),
        real_feature: cfg!(feature = "real"),
    };
    epsilon_cli::doctor::run(&inputs)
}

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

    let prompt = user_message(&std::env::temp_dir(), "fix the flaky auth tests".into());
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
        AgentEvent::Notice { text } => eprintln!("[retry] {text}"),
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

#[cfg(test)]
mod runtime_tests {
    use super::*;

    #[tokio::test]
    async fn plan_mode_preserves_project_read_deny() {
        let workspace = tempfile::tempdir().unwrap();
        std::fs::write(workspace.path().join("private.txt"), "private fixture").unwrap();
        std::fs::write(workspace.path().join("public.txt"), "public fixture").unwrap();
        std::fs::create_dir(workspace.path().join(".epsilon")).unwrap();
        std::fs::write(
            workspace.path().join(".epsilon/config.toml"),
            r#"
[[rules]]
capability = "read"
pattern = "*private.txt"
action = "deny"

[[rules]]
capability = "write"
pattern = "*"
action = "allow"
"#,
        )
        .unwrap();
        let (project, trust, _) = epsilon_cli::runtime::load_project_config(workspace.path());
        assert_eq!(trust, epsilon_cli::runtime::TrustDecision::Trusted);
        assert_eq!(project.rules.len(), 2);
        let agent = build_runtime(RunParams {
            workspace: workspace.path(),
            stream_fn: Arc::new(FauxProvider::new(vec![
                FauxStep::call(
                    "private",
                    "read",
                    serde_json::json!({"path": "private.txt"}),
                ),
                FauxStep::call("public", "read", serde_json::json!({"path": "public.txt"})),
                FauxStep::call(
                    "mutation",
                    "write",
                    serde_json::json!({"path": "new.txt", "content": "not written"}),
                ),
                FauxStep::say("done"),
            ])),
            approvals: Arc::new(epsilon_safety::AutoApprove),
            mcp_servers: &[],
            session_file: ".epsilon/session.jsonl",
            confinement: &epsilon_tools::Confinement::Off,
            lsp: epsilon_tools::Lsp::Off,
            model_id: "test-1",
            resume: &Resume::No,
            project: &project,
            project_trusted: true,
            plan_mode: true,
        })
        .await
        .unwrap();
        let outcome = agent
            .run(user_message(workspace.path(), "inspect the files".into()))
            .await
            .unwrap();
        let result = |id: &str| {
            outcome
                .produced
                .iter()
                .find_map(|message| match message {
                    Message::ToolResult(result) if result.tool_call_id == id => Some(result),
                    _ => None,
                })
                .expect("scripted tool call must produce a result")
        };
        let private = result("private");
        assert!(
            private.is_error,
            "plan mode must preserve the project's read deny: {private:?}"
        );
        assert!(format!("{private:?}").contains("Permission denied"));
        assert!(
            !result("public").is_error,
            "ordinary plan reads remain allowed"
        );
        assert!(
            result("mutation").is_error,
            "project allow cannot override plan deny"
        );
        assert!(!workspace.path().join("new.txt").exists());
    }
}

#[cfg(test)]
mod mask_tests {
    use super::*;

    #[test]
    fn mask_hides_short_keys_entirely() {
        assert_eq!(mask_key(""), "****");
        assert_eq!(mask_key("abc"), "****");
        assert_eq!(mask_key("abcd"), "****");
    }

    #[test]
    fn mask_shows_only_last_four_of_long_keys() {
        assert_eq!(mask_key("sk-ant-0123456789"), "…6789");
        assert_eq!(mask_key("12345"), "…2345");
        // The full key must never appear in the masked output.
        let key = "gsk_super_secret_value";
        let masked = mask_key(key);
        assert!(!masked.contains("super"));
        assert!(masked.ends_with("alue"));
    }

    #[test]
    fn mask_is_char_boundary_safe_on_non_ascii_tails() {
        // Multi-byte chars in the tail: must not panic (the old byte-index
        // slice did) and must expose only the final four CHARACTERS.
        let masked = mask_key("key-أبجد");
        assert!(masked.starts_with('…'));
        assert!(masked.ends_with("بجد"));

        let masked = mask_key("日本語テスト");
        assert_eq!(masked, "…語テスト"); // exactly the last 4 chars

        let masked = mask_key("éééééé");
        assert_eq!(masked.chars().count(), 5); // … + last 4 chars

        assert_eq!(mask_key("ééé"), "****"); // 4 or fewer chars: hidden fully
    }
}

/// M63: the headless reporting rule. These tests exist because the old code path
/// silently printed "no final assistant text produced" for every non-EndTurn
/// ending — a provider 401, a 429 that exhausted retries, or a context overflow
/// all looked like an empty answer with no explanation.
#[cfg(test)]
mod print_outcome_tests {
    use super::*;
    use epsilon_contracts::content::ContentBlock;
    use epsilon_contracts::message::{AssistantMessage, Usage};
    use epsilon_contracts::new_id;
    use epsilon_core::RunOutcome;

    fn assistant(stop: StopReason, text: &str, err: Option<&str>) -> Message {
        let content = if text.is_empty() {
            Vec::new()
        } else {
            vec![ContentBlock::text(text)]
        };
        Message::Assistant(AssistantMessage {
            id: new_id("msg"),
            timestamp_ms: 0,
            provider: "test".into(),
            model_id: "test-1".into(),
            content,
            stop_reason: stop,
            error_message: err.map(str::to_string),
            usage: Usage::default(),
        })
    }

    fn outcome(produced: Vec<Message>, reason: TurnEndReason) -> RunOutcome {
        RunOutcome {
            produced,
            reason,
            usage: Usage::default(),
        }
    }

    #[test]
    fn clean_end_turn_prints_the_reply_and_reports_nothing() {
        let out = outcome(
            vec![assistant(StopReason::EndTurn, "done", None)],
            TurnEndReason::Completed,
        );
        let (text, diag) = print_outcome(&out);
        assert_eq!(text.as_deref(), Some("done"));
        assert!(diag.is_none(), "a clean run must not print a diagnostic");
    }

    #[test]
    fn provider_error_surfaces_the_provider_message_not_a_generic_line() {
        // The exact shape a real 401 produces: Error stop reason, no text, the
        // provider's own words in error_message.
        let out = outcome(
            vec![assistant(
                StopReason::Error,
                "",
                Some("HTTP 401 Unauthorized: invalid api key"),
            )],
            TurnEndReason::Error("HTTP 401 Unauthorized: invalid api key".into()),
        );
        let (text, diag) = print_outcome(&out);
        assert!(text.is_none());
        let diag = diag.expect("a failed run must explain itself");
        assert!(
            diag.contains("401") && diag.contains("invalid api key"),
            "the provider's own message must reach the user: {diag}"
        );
    }

    #[test]
    fn partial_text_is_kept_and_flagged_not_thrown_away() {
        // M62 salvage: text arrived, then the stream died. Print what streamed,
        // say so on stderr, exit non-zero.
        let out = outcome(
            vec![assistant(
                StopReason::Error,
                "here is what I found so far",
                Some("network reset mid-stream"),
            )],
            TurnEndReason::Error("network reset mid-stream".into()),
        );
        let (text, diag) = print_outcome(&out);
        assert_eq!(text.as_deref(), Some("here is what I found so far"));
        assert!(diag.unwrap().contains("network reset"));
    }

    #[test]
    fn abort_reports_aborted_and_empty_turn_explains_itself() {
        let out = outcome(
            vec![assistant(StopReason::Aborted, "", None)],
            TurnEndReason::Aborted,
        );
        let (text, diag) = print_outcome(&out);
        assert!(text.is_none());
        assert!(diag.unwrap().contains("aborted by user"));
    }

    #[test]
    fn budget_and_token_limits_name_their_cause() {
        let out = outcome(vec![], TurnEndReason::BudgetExhausted);
        assert!(
            print_outcome(&out)
                .1
                .unwrap()
                .contains("turn budget ran out")
        );
        let out = outcome(vec![], TurnEndReason::MaxTokens);
        assert!(
            print_outcome(&out)
                .1
                .unwrap()
                .contains("output-token limit")
        );
    }

    #[test]
    fn help_text_documents_every_mode_the_parser_accepts() {
        // The parser accepts these; the help must not drift behind it.
        let h = help_text();
        for flag in [
            "--print",
            "--serve",
            "--rpc",
            "--demo",
            "--list-models",
            "--model",
            "--provider",
            "--fallback",
            "--workspace",
            "--sandbox",
            "--plan",
            "--yes",
            "--continue",
            "--resume",
            "--script",
            "--mcp",
            "--version",
            "--help",
        ] {
            assert!(h.contains(flag), "help is missing {flag}");
        }
    }
}
