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
use epsilon_core::{LoopLimits, ToolRegistry};
use epsilon_llm::{FauxProvider, FauxStep, FauxToolCall, PanicGuard};
use epsilon_mcp::McpServerConfig;
use epsilon_safety::{ApprovalUi, ChannelApprovalUi};
// ADR-021: composition is a library crate now, so the desktop IDE can build the same agent
// this binary does. `Resume` came with it, because a frontend cannot ask for a session
// without also saying which one it meant.
use epsilon_runtime::{Resume, RunParams, build_runtime, lease_wait_millis, lease_wait_summary};

/// M52 guard: if this crate is built with `real` while `epsilon-core` is not,
/// `Agent::try_hot_swap_to_real` silently degrades to "restart epsilon".
/// Binding core's marker makes that combination a build error.
#[cfg(feature = "real")]
const _REAL_PATH_FORWARDED: () = epsilon_core::REAL_PATH_ENABLED;

const USAGE: &str = "usage: epsilon            interactive TUI (default)
  epsilon -p \"task\"       headless run · epsilon --serve [--port N] · epsilon --rpc
  epsilon --demo          scripted offline demo · --list-models [provider]
  epsilon doctor          why is this not working: model, credentials, sandbox, session, trust
  epsilon --import PI.jsonl   convert a pi session into an Epsilon session beside it
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
  --import FILE     convert a pi session file into an Epsilon session file **beside
                    it** (<FILE>.epsilon.jsonl) and exit without starting a turn.
                    For a current pi file — entries with id/parentId, the kind at
                    the top level — which --resume cannot open in place. The source
                    is read and never written: no backup, no rewrite, no rename.
                    Prints what it imported and anything it could only preserve.

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
  --plan                read-only run: write/edit denied, and bash/shell denied
                        outright (so git commit/push cannot run either)
  --light               draw the TUI for a light terminal background. There is no
                        reliable way to detect one (COLORTERM describes colour
                        depth, OSC 11 needs a round-trip the terminal may not
                        answer), so the palette is selected: this flag, else
                        EPSILON_THEME=light, else [tui] theme in
                        ~/.config/epsilon/config.toml, else dark.
  --yes                 unattended runs auto-APPROVE permission asks. Without it,
                        unattended runs auto-deny.
  -c, --continue        resume the newest session in this workspace
  --resume FILE         resume a specific session file
  --import FILE         convert a pi session file into <FILE>.epsilon.jsonl beside
                        it (leaving the pi file untouched), print the summary, exit
  --script FILE         faux-provider script for offline runs
  --mcp NAME=CMD[:A,B]  attach an MCP server over stdio
  -V, --version         print the version
  -h, --help            this text

ENVIRONMENT
  <PROVIDER>_API_KEY    provider credential (ANTHROPIC_API_KEY, OPENAI_API_KEY,
                        DEEPSEEK_API_KEY, GEMINI_API_KEY, XAI_API_KEY,
                        OPENROUTER_API_KEY, MOONSHOT_API_KEY, ZAI_API_KEY,
                        GROQ_API_KEY). Stored credentials win over env vars.
  EPSILON_THEME         TUI palette: `dark` (default) or `light`, for a terminal
                        with a light background. `--light` overrides it; an
                        unrecognised value falls back to dark.
  EPSILON_ASCII         TUI characters: `1` forces the ASCII set (`+-|` instead
                        of `╭─│╰╯`, `x` instead of `✗`), `0` forces the
                        Unicode set. Unset, Windows Terminal / a VS Code terminal
                        / a VT-capable $TERM get Unicode and a legacy console
                        window gets ASCII — set `0` if yours has a Unicode font.
  EPSILON_STREAM_IDLE_SECS
                        seconds of stream silence before a turn fails as a network
                        error (default 120, 0 disables)

FILES
  ~/.config/epsilon/config.toml   model, picker favorites, custom providers,
                                  [oauth.<provider>] client ids, [editor] command
  ~/.config/epsilon/auth.json     stored credentials (chmod 600 where supported)
  ~/.config/epsilon/MEMORY.md     global memory injected into the prompt
  ~/.config/epsilon/history       prompt history
  .epsilon/sessions/<workspace>/  session transcripts, one file per launch
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
  epsilon --import ~/.pi/agent/sessions/…/s.jsonl   # bring a pi session over

Exit codes: 0 success · 1 run or provider failure · 2 bad usage.";

/// The full help text, without the trailing newline ceremony (kept as a helper so
/// tests can assert on it).
fn help_text() -> &'static str {
    HELP
}

struct Args {
    mode: Mode,
    script_path: Option<String>,
    /// `--stream-pace-ms N`: delay every faux stream event by N ms, so a turn lasts
    /// seconds instead of microseconds. See the flag's parse site for why this exists.
    stream_pace_ms: u64,
    workspace: Option<PathBuf>,
    mcp_servers: Vec<McpServerConfig>,
    port: u16,
    confinement: epsilon_tools::Confinement,
    /// M79: ask a language server about edited files (`--lsp auto|off`).
    lsp: epsilon_cli::runtime::Lsp,
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
    /// and `bash`/`shell` denied outright (M82: command heads cannot establish a
    /// read-only shell boundary, so shell is refused rather than classified);
    /// everything else normal.
    plan_mode: bool,
    /// `--yes`: headless runs auto-APPROVE gated asks instead of the safe default
    /// (auto-deny). Explicit opt-in for trusted automation.
    auto_approve: bool,
    /// `--light` (D8): draw for a light terminal background. `false` means the
    /// flag was absent, not that dark was asked for — `EPSILON_THEME` and the
    /// config are still consulted, which is what lets a user set it once.
    light: bool,
}

// Session-restore selection (M12) moved to psilon-runtime with uild_runtime (ADR-021);
// it is imported above.

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
    /// Import a pi session file into a new Epsilon session file beside it, print what it did,
    /// and exit — no turn, no model, no review loop. Handled before the runtime exists, like
    /// `doctor` and `auth`: an import is file conversion, and building an agent to do it would
    /// print a startup banner over the report.
    Import(PathBuf),
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
            stream_pace_ms: 0,
            workspace: None,
            mcp_servers: Vec::new(),
            port: 8787,
            confinement: epsilon_tools::Confinement::Auto,
            lsp: epsilon_cli::runtime::Lsp::Auto,
            provider: None,
            model_id: None,
            fallbacks: Vec::new(),
            resume: Resume::default(),
            plan_mode: false,
            auto_approve: false,
            light: false,
        };
    }
    let mut mode = Mode::Tui(None); // M14: plain `epsilon` opens the interactive TUI
    let mut script_path = None;
    let mut stream_pace_ms: u64 = 0;
    let mut workspace = None;
    let mut mcp_servers = Vec::new();
    let mut port: u16 = 8787;
    let mut confinement = epsilon_tools::Confinement::Auto;
    let mut lsp = epsilon_cli::runtime::Lsp::Auto;
    // --plan: read-only research run (opencode plan-agent parity).
    let mut plan_mode = false;
    // --yes: unattended runs approve gated asks (explicit opt-in; default is deny).
    let mut auto_approve = false;
    // --light: draw for a light terminal background (D8).
    let mut light = false;
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
                    "auto" => epsilon_cli::runtime::Lsp::Auto,
                    "off" => epsilon_cli::runtime::Lsp::Off,
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
            "--light" => light = true,
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
            "--import" => {
                i += 1;
                let p = raw
                    .get(i)
                    .cloned()
                    .unwrap_or_else(|| die("--import requires a pi session file path"));
                mode = Mode::Import(PathBuf::from(p));
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
            // Hidden: slow the faux stream down so a turn lasts seconds, not
            // microseconds. The reason this flag exists at all is in
            // `epsilon-llm/src/paced.rs`: every TUI test and probe this
            // repository has ever run drove a turn that was over before the
            // first keystroke, so the reported "freezes the moment you start
            // typing" state has never been executed by anything. A long turn
            // measured in seconds is the precondition for reproducing it.
            "--stream-pace-ms" => {
                i += 1;
                stream_pace_ms = raw
                    .get(i)
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or_else(|| die("--stream-pace-ms requires a delay (e.g. 40)"));
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
        stream_pace_ms,
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
        light,
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
    // Same reason, for the same class of command: an import converts a file. It needs no model,
    // no provider, no workspace and no turn, so it runs before the runtime is built.
    if let Mode::Import(source) = &args.mode {
        std::process::exit(import_mode(source));
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
    match login(
        provider,
        &epsilon_core::config::oauth_config(provider),
        &mut ui,
        !paste_only,
    ) {
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

    // ADR-021: which provider, which model, and what to tell the user about both, are
    // decided in `epsilon-runtime` so every frontend decides them the same way. The CLI
    // used to do this inline; a desktop window could not have reached it, and three of
    // the branches are ways a window ends up naming a provider it is not calling.
    let launch = match epsilon_runtime::resolve_launch(&epsilon_runtime::LaunchRequest {
        workspace: workspace.clone(),
        model_reference: args.model_id.clone(),
        provider: match args.provider {
            Some(Provider::Faux) => epsilon_runtime::ProviderChoice::Faux,
            Some(Provider::Real) => epsilon_runtime::ProviderChoice::Real,
            None => epsilon_runtime::ProviderChoice::Auto,
        },
        fallbacks: args.fallbacks.clone(),
        script_path: args.script_path.as_ref().map(std::path::PathBuf::from),
        stream_pace_ms: args.stream_pace_ms,
        lsp: args.lsp,
        plan_mode: args.plan_mode,
        confinement: args.confinement.clone(),
        resume: args.resume.clone(),
    }) {
        Ok(launch) => launch,
        Err(message) => die(&message),
    };
    // Startup notices flush here, in order, once (plan 008).
    for line in &launch.notices {
        eprintln!("{line}");
    }

    let shared = SharedRun {
        workspace: launch.workspace.clone(),
        stream_fn: launch.stream_fn.clone(),
        mcp_servers: args.mcp_servers.clone(),
        confinement: launch.confinement.clone(),
        lsp: launch.lsp,
        model_id: launch.model_id.clone(),
        model_explicit: launch.model_explicit,
        resume: launch.resume.clone(),
        project: launch.project.clone(),
        project_trusted: launch.project_trusted,
        plan_mode: launch.plan_mode,
        offline: launch.offline,
        startup_notices: launch.notices.clone(),
        trust_review_pending: launch.trust_review_pending,
    };
    match &args.mode {
        Mode::Auth(_) => unreachable!("auth subcommands exit inside main() before dispatch"),
        Mode::Doctor => unreachable!("--doctor exits inside main() before the runtime is built"),
        Mode::Import(_) => unreachable!("--import exits inside main() before the runtime is built"),
        Mode::Demo => demo_mode().await,

        Mode::Print(task) => print_mode(&shared, args.auto_approve, task.clone()).await,
        Mode::Tui(initial) => {
            tui_mode(&shared, initial.clone(), resolve_theme_mode(args.light)).await
        }
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
        session_file: ".epsilon/sessions/rpc.jsonl".to_string(),
        confinement: &shared.confinement,
        lsp: shared.lsp,
        model_id: &shared.model_id,
        model_explicit: shared.model_explicit,
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

/// Which palette the TUI should draw with (D8).
///
/// Precedence, most explicit first: `--light` on the command line, then
/// `EPSILON_THEME`, then `[tui] theme` in `~/.config/epsilon/config.toml`. `None`
/// means none of them said anything, and the TUI's own detector supplies the
/// default.
///
/// `--light` is a flag rather than `--theme <dark|light>` because the failure it
/// fixes is one-directional: the default is already dark, and the user who needs
/// this is the one staring at unreadable text on a white terminal. A value flag
/// would make them spell out the thing they are trying to leave.
///
/// An unrecognised value falls back to the default rather than failing, the same
/// rule `epsilon_core::config` follows for every other key: a config typo must
/// not stop the TUI from starting. The fallback is [`ThemeMode::Dark`] — *not*
/// "try the next source" — and that distinction is the whole point of the
/// environment being checked here rather than by the TUI's own detector. A user
/// who typed `EPSILON_THEME=solarized` meant to say something about the palette,
/// and the surprising outcome is not "dark, and here is why" but "the *other*
/// palette, because of a config key they had forgotten about". `--light` is still
/// the escape hatch, and it is checked first.
fn resolve_theme_mode(light_flag: bool) -> Option<epsilon_tui::ThemeMode> {
    let from_env = std::env::var(epsilon_tui::ui::theme::THEME_ENV).unwrap_or_default();
    resolve_theme_mode_from(light_flag, explicit_theme_mode(&from_env), || {
        epsilon_core::config::load_config_string("tui", "theme")
            .and_then(|value| epsilon_tui::ThemeMode::from_name(&value))
    })
}

/// Precedence over already-read inputs, so the rule can be tested without
/// mutating the test process's environment.
///
/// `env` is the result of reading `EPSILON_THEME`, not the raw string, because
/// "unset" and "set to something unrecognised" have to be different answers:
/// `None` means the user said nothing and the config is next, while
/// `Some(ThemeMode::Dark)` is the documented fallback for a value that names
/// neither variant. Treating the two the same is the defect this function exists
/// to make unrepeatable.
fn resolve_theme_mode_from(
    light_flag: bool,
    env: Option<epsilon_tui::ThemeMode>,
    config: impl FnOnce() -> Option<epsilon_tui::ThemeMode>,
) -> Option<epsilon_tui::ThemeMode> {
    if light_flag {
        return Some(epsilon_tui::ThemeMode::Light);
    }
    if let Some(mode) = env {
        return Some(mode);
    }
    config()
}

/// The mode `EPSILON_THEME` asks for, with a typo resolving to the documented
/// default rather than to "unset".
///
/// Empty (or absent) is the only input that returns `None`: an empty variable is
/// indistinguishable from one that was never set, which is how shells and
/// `.env` files behave anyway.
fn explicit_theme_mode(value: &str) -> Option<epsilon_tui::ThemeMode> {
    if value.trim().is_empty() {
        return None;
    }
    Some(epsilon_tui::ThemeMode::from_name(value).unwrap_or_default())
}

/// Shared composition: real tools + ruleset gate over the given approval UI.
/// Inputs every frontend mode shares (plan 008): one struct instead of nine
/// positionally-similar parameters per mode function.
struct SharedRun {
    workspace: PathBuf,
    stream_fn: Arc<dyn epsilon_llm::StreamFn>,
    mcp_servers: Vec<McpServerConfig>,
    confinement: epsilon_tools::Confinement,
    lsp: epsilon_cli::runtime::Lsp,
    model_id: String,
    /// Whether the user named a model on this invocation (M139). Distinguishes a
    /// deliberate `--model` from a project/config default, which is what decides
    /// whether a resumed session's own model may take over.
    model_explicit: bool,
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
    /// Whether this process runs **offline** (M233): --provider faux, or no usable
    /// credential. Decided once here and carried, because the TUI header must name the
    /// provider it is actually calling, and the model id alone cannot say that.
    offline: bool,
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
        session_file: ".epsilon/sessions/cli.jsonl".to_string(),
        confinement: &shared.confinement,
        lsp: shared.lsp,
        model_id: &shared.model_id,
        model_explicit: shared.model_explicit,
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

/// What an admitted `serve` run that failed must print, or `None` when it succeeded.
///
/// F07's last surface. The TUI reports a failed durable write as a notice plus `run failed`
/// (`app.rs`'s `spawn_run`) and `-p` reports it on stderr with exit 1 (`print_mode`); the
/// server's boot task called `let _ = agent.run(message).await`, so on that path the notice and
/// the `Err` were both discarded and the person on the other end of the server learned nothing.
///
/// **Why this is stderr and not a frame on `/events`.** The `Notice` core emits is a broadcast
/// event, and a broadcast reaches only receivers alive at the moment it is sent — it is a live
/// stream, not a log, so it is not replayed to whoever subscribes later. The boot task starts
/// before the listener is bound (line below), so at that instant the server has no subscriber at
/// all and the notice is dropped on the floor; a client cannot have connected yet, because there
/// is not yet a socket to connect to. The protocol therefore has no channel that can carry this
/// failure to a client, and inventing one (a replay buffer, a sticky error in `/state`) is a
/// server redesign, not a report. What the server does have is its own log, which is where the
/// sibling failure on this path already goes — the `/prompt` route's `eprintln!("serve: admitted
/// run failed: …")` in `epsilon-server` — so this uses it, in the same shape, rather than leaving
/// the failure silent. A failure *during* a client's own prompt still reaches that client, as the
/// `409` its next `/prompt` gets when the F07 latch refuses a request built from a store that is
/// missing a turn.
///
/// Pure — it decides the line and prints nothing — so what an operator would see is assertable
/// without a terminal or a child process. The `serve:` prefix matches the server's own reports,
/// and the error's `Display` already carries the notice's whole sentence ("the session file could
/// not record this turn … may be missing it"), so the log line names the consequence, not just
/// the syscall.
fn admitted_run_failure(
    run: &Result<epsilon_core::RunOutcome, epsilon_core::RunError>,
) -> Option<String> {
    match run {
        Ok(_) => None,
        Err(e) => Some(format!("serve: admitted run failed: {e}")),
    }
}

/// Interactive ratatui frontend (M9b).
async fn tui_mode(
    shared: &SharedRun,
    initial_task: Option<String>,
    theme_mode: Option<epsilon_tui::ThemeMode>,
) -> i32 {
    let approvals = Arc::new(ChannelApprovalUi::new());
    // Composition happens here; the TUI crate stays a pure frontend.
    let agent = build_runtime(RunParams {
        workspace: &shared.workspace,
        stream_fn: Arc::clone(&shared.stream_fn),
        approvals: Arc::clone(&approvals) as Arc<dyn ApprovalUi>,
        mcp_servers: &shared.mcp_servers,
        session_file: epsilon_cli::runtime::fresh_session_file(
            epsilon_cli::runtime::now_millis(),
            &epsilon_contracts::new_id("ses"),
            &shared.workspace,
        ),
        confinement: &shared.confinement,
        lsp: shared.lsp,
        model_id: &shared.model_id,
        model_explicit: shared.model_explicit,
        resume: &shared.resume,
        project: &shared.project,
        project_trusted: shared.project_trusted,
        plan_mode: shared.plan_mode,
    })
    .await;
    match agent {
        Ok(agent) => {
            // A18: the lease wait is drawn on the console before the alternate screen exists, and
            // the alternate screen erases it — so a launch that waited says so in the transcript,
            // where it survives. Built here rather than in `shared` because `shared` is assembled
            // before the store is opened, which is when the wait happens.
            let mut startup_notices = shared.startup_notices.clone();
            if let Some(waited) = lease_wait_summary(lease_wait_millis()) {
                startup_notices.push(waited);
            }
            epsilon_tui::run(epsilon_tui::TuiConfig {
                agent,
                approvals,
                initial_task,
                model_id: shared.model_id.clone(),
                live: live_fetcher(),
                startup_notices,
                trust_review_pending: shared.trust_review_pending,
                theme_mode,
                // M233: the header has to name the provider this process is actually calling.
                // `--provider faux` with a saved `deepseek/...` model used to render
                // `DeepSeek V3.1` over a faux turn, because the offline marker was keyed off the
                // model id instead of this decision.
                offline: shared.offline,
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
        session_file: ".epsilon/sessions/serve.jsonl".to_string(),
        confinement: &shared.confinement,
        lsp: shared.lsp,
        model_id: &shared.model_id,
        model_explicit: shared.model_explicit,
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
                let run = agent.run(message).await;
                // `let _ =` here was the defect (F07): a run that failed because the session
                // file refused a write left this server reporting nothing at all. The notice the
                // failure emits reaches no client — see `admitted_run_failure`, which is why the
                // server's own log carries it instead.
                if let Some(line) = admitted_run_failure(&run) {
                    eprintln!("{line}");
                }
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

/// `epsilon --import <pi-file>`: convert a pi session file into an Epsilon session file beside
/// it, print what happened, and exit.
///
/// No runtime, no workspace, no turn — an import reads one file and writes another. The source
/// never matters to the workspace, which is why this mode ignores `--workspace` entirely.
///
/// Two failures get their own sentences rather than a raw I/O error: a source that is not there
/// (naming the path they typed), and an import that fails on a file which is not pi-shaped at
/// all — for which the useful next step is the command that *does* open it.
fn import_mode(source: &Path) -> i32 {
    if !source.exists() {
        eprintln!(
            "no such file: {} — --import takes a pi session file (usually under \
             ~/.pi/agent/sessions/)",
            source.display()
        );
        return 2;
    }
    match epsilon_session::import_pi_session(source) {
        Ok(report) => {
            for line in report.summary() {
                eprintln!("{line}");
            }
            0
        }
        Err(e) => {
            eprintln!("import {}: {e}", source.display());
            // The failure is already phrased for the file that is there — a pi reader says what
            // it could not read and where. The one case that leaves a user without a next step
            // is a file that is not a pi session at all, which is usually an Epsilon session and
            // has a command of its own.
            if !epsilon_session::looks_like_pi_session(source) {
                eprintln!(
                    "{} does not look like a pi session — if it is an Epsilon session, open it \
                     with: epsilon --resume {}",
                    source.display(),
                    source.display()
                );
            }
            1
        }
    }
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
            session_file: ".epsilon/session.jsonl".to_string(),
            confinement: &epsilon_tools::Confinement::Off,
            lsp: epsilon_cli::runtime::Lsp::Off,
            model_id: "test-1",
            model_explicit: false,
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

    fn user_msg(text: &str) -> Message {
        Message::User(epsilon_contracts::message::UserMessage {
            id: epsilon_contracts::new_id("msg"),
            timestamp_ms: 0,
            content: vec![epsilon_contracts::content::UserContent::Text { text: text.into() }],
        })
    }

    /// The `Notice` texts a subscription collected, drained destructively.
    ///
    /// Subscribe **before** the run: a notice is a live broadcast event, not a log, so a
    /// subscriber created afterwards sees nothing at all.
    fn notices_from(rx: &mut tokio::sync::broadcast::Receiver<Arc<AgentEvent>>) -> Vec<String> {
        let mut out = Vec::new();
        loop {
            match rx.try_recv() {
                Ok(ev) => {
                    if let AgentEvent::Notice { text } = &*ev {
                        out.push(text.clone());
                    }
                }
                Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => continue,
                Err(_) => break,
            }
        }
        out
    }

    /// F07 on the server path: a run refused because the session file would not record it is
    /// **reported**, not discarded with the notice.
    ///
    /// The defect this pins was one call site — `serve_mode`'s boot task did
    /// `let _ = agent.run(message).await` — and it was invisible because the failure has two
    /// halves and each half went nowhere: the `Err` was thrown away, and the `Notice` core emits
    /// had no live subscriber (the task starts before the listener is bound, so no client can be
    /// connected). The run below is a real one, refused by a real
    /// [`epsilon_session::SessionStore`] write fault, so the error text under assertion is the
    /// sentence core actually builds rather than a string this test invented.
    ///
    /// **What this proves and what it does not.** It proves the reporting *layer*: given the
    /// failure the boot task receives, the line the server prints names it — and that a later
    /// run's latched refusal is reported the same way, so no `/prompt` from then on is answered
    /// with silence. It cannot prove the call site calls it from here: driving a store failure
    /// through `serve_mode` is not possible in this process (that function builds its own agent
    /// and its own store, and — as `epsilon-session`'s own note says — a store that already holds
    /// an open handle still accepts writes through a read-only file, a revoked directory
    /// permission and a non-writable attribute, so the fault has to be injected). The call site
    /// is held instead by `the_serve_boot_task_reports_a_failed_run_instead_of_discarding_it`
    /// below, which reads it. Neither test alone is enough, which is why there are two.
    #[tokio::test]
    async fn a_durable_write_failure_on_the_serve_path_reaches_the_server_log() {
        let dir = tempfile::tempdir().unwrap();
        let mut store =
            epsilon_session::SessionStore::open_in(dir.path().join("serve.jsonl"), dir.path())
                .unwrap();
        // Occurrence 1 is the run's first append — the prompt. A prompt the store refuses ends
        // the run before the provider is entered (F07), which is the failure the boot task sees.
        store.set_write_fault_for_tests(epsilon_session::FaultOp::Append, "disk full", 1, 1);
        let store = Arc::new(std::sync::Mutex::new(store));
        let handle = Arc::clone(&store);

        let provider = FauxProvider::new(vec![FauxStep::say("answered anyway")]);
        let recorded = provider.recorded_requests_handle();
        let agent = epsilon_core::Agent::new(
            LoopLimits::default(),
            Arc::new(PanicGuard(provider)),
            Arc::new(ToolRegistry::new()),
            Arc::new(epsilon_core::AutoAllow),
            dir.path().to_path_buf(),
        );
        agent.attach_session_store(store);
        let mut rx = agent.subscribe();

        // Exactly what `serve_mode`'s boot task now does with the result.
        let run = agent.run(user_msg("hello")).await;
        let reported = admitted_run_failure(&run).unwrap_or_else(|| {
            panic!(
                "a run whose durable write failed must not be discarded: the server has to \
                 report it, and this one produced no line ({run:?})"
            )
        });
        assert!(
            reported.starts_with("serve: admitted run failed: "),
            "the line must carry the server's own prefix, like the /prompt route's report: {reported}"
        );
        assert!(
            reported.contains("could not record this turn") && reported.contains("disk full"),
            "the report must name the failure and the disk's own reason: {reported}"
        );
        assert!(
            reported.contains("not on disk") && reported.contains("may be missing it"),
            "the report must name the consequence the user acts on, not just the syscall: {reported}"
        );

        // The notice half, and why the log is load-bearing here: the report above holds no
        // receiver, so if the server did not log it this sentence reached nobody at all.
        let notices = notices_from(&mut rx);
        assert_eq!(notices.len(), 1, "exactly one report: {notices:?}");
        assert!(
            notices[0].contains("could not record this turn"),
            "the notice the log relays is core's own: {notices:?}"
        );

        // The session is now latched: the store is missing a turn, so no later request may be
        // built from it (F07's second half). That refusal is a failure of the *next* run, which
        // is why the first assertion here is that it, too, reaches the server's log — a client
        // that prompts from now on gets its `409`/error from the same report, never silence.
        let again = agent.run(user_msg("second")).await;
        let refusal = admitted_run_failure(&again).unwrap_or_else(|| {
            panic!("the latched refusal must be reported too, not swallowed: {again:?}")
        });
        assert!(
            refusal.contains("missing a turn") && refusal.contains("checking the disk"),
            "the refusal has to say what to do next: {refusal}"
        );

        // The store and the loop really did diverge here: the prompt is in memory and not on
        // disk, which is what "the session may be missing a turn" means. Asserted so a future
        // edit cannot make this test green by making the fault a no-op.
        assert_eq!(agent.history().len(), 2, "both prompts survive in memory");
        assert_eq!(
            handle.lock().unwrap().model_view().unwrap().len(),
            1,
            "the store holds only the turn that reached the disk"
        );
        assert!(
            recorded.lock().unwrap().is_empty(),
            "no request may be built from a store that is missing a turn"
        );
    }

    /// What keeps the test above honest: it reads the boot task and requires it to report.
    ///
    /// The lesson is F2's, learned on `card_is_open`: a behavioural test of a *reporting* layer
    /// passes just as happily when the call site goes back to discarding the result, because the
    /// layer it exercises has not changed. What was wrong was that `serve_mode` dropped the
    /// failure, so this asserts the shape that was wrong — the run's result is handed to the
    /// reporter — and fails on the day `let _ = agent.run(` is written again.
    #[test]
    fn the_serve_boot_task_reports_a_failed_run_instead_of_discarding_it() {
        let source = include_str!("main.rs");
        const GONE: &str = "`serve_mode` is gone, so this guard is reading nothing: find where the \
                            server's initial task runs the agent and hold that instead";
        let after = source
            .split_once("async fn serve_mode(")
            .unwrap_or_else(|| panic!("{GONE}"))
            .1;
        // Up to the end of the function: the first line that closes it at column zero.
        let body: String = after
            .lines()
            .take_while(|line| *line != "}")
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            body.contains("admitted_run_failure(&run)"),
            "the server's boot task must report the run's failure through the same path the rest \
             of the server logs: {body}"
        );
        assert!(
            !body.contains("let _ = agent.run("),
            "the run's result is being discarded again — that is the F07 defect on this path: \
             {body}"
        );
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
            "--light",
            "--yes",
            "--continue",
            "--resume",
            "--import",
            "--script",
            "--mcp",
            "--version",
            "--help",
        ] {
            assert!(h.contains(flag), "help is missing {flag}");
        }
    }

    #[test]
    fn help_does_not_advertise_an_environment_variable_nothing_reads() {
        // `RUST_LOG` was listed here for a long time and **nothing reads it**: no crate in
        // the workspace depends on `tracing`/`tracing-subscriber` (only the uncommitted
        // reference harness under `references/` does), no crate installs a subscriber, and
        // no source file reads the variable. The advertised list is a promise, so the dead
        // one is gone and this test fails if it comes back without a consumer — a check
        // against the *help*, not against `std::env`, because the defect was the promise.
        let h = help_text();
        assert!(
            !h.contains("RUST_LOG"),
            "help advertises RUST_LOG, which no crate initializes tracing for"
        );
    }

    #[test]
    fn the_light_flag_selects_the_light_palette_regardless_of_the_environment() {
        // The one branch that cannot come from the environment, and the one the
        // user reaches for when the default is unreadable: `--light` wins, and it
        // resolves to a palette rather than to a string the TUI parses again.
        assert_eq!(
            resolve_theme_mode(true),
            Some(epsilon_tui::ThemeMode::Light)
        );
        // Without the flag the answer is **decided**, not merely typed: this test
        // used to assert `None | Dark | Light`, which is every possible answer and
        // therefore no answer at all. `EPSILON_THEME` set here means the
        // environment's value is the answer (unset means the config key is, and the
        // test below reads that key rather than assuming CI has none).
        let raw = std::env::var(epsilon_tui::ui::theme::THEME_ENV).unwrap_or_default();
        let answer = resolve_theme_mode(false);
        if raw.trim().is_empty() {
            let from_config = epsilon_core::config::load_config_string("tui", "theme")
                .and_then(|value| epsilon_tui::ThemeMode::from_name(&value));
            assert_eq!(answer, from_config, "unset, so the config key decides");
        } else {
            assert_eq!(
                answer,
                explicit_theme_mode(&raw),
                "`{raw}` is set, so it decides — and a malformed value is dark"
            );
        }
    }

    #[test]
    fn an_unrecognised_theme_value_falls_back_to_dark_not_to_the_config() {
        // B3. `--help` and two doc comments promise that `EPSILON_THEME=solarized`
        // ends up *dark*. "Dark" has to mean the palette, not "ask the next source
        // in the list": falling through to `[tui] theme` means a typo silently
        // selects **light** for a user whose config says light, which is the
        // opposite of the documented answer and the opposite of what they typed.
        //
        // `Dark` and `None` are different readings of that promise and this test
        // only accepts one of them — the existing `the_light_flag_…` test asserted
        // `None | Dark | Light` and so passed under either.
        assert_eq!(
            explicit_theme_mode("solarized"),
            Some(epsilon_tui::ThemeMode::Dark),
            "a typo must be resolved here, not deferred to the config"
        );
        // So a config that *does* name light cannot win against a populated
        // environment variable, whichever way it is populated. This is the exact
        // configuration from the review, and the closure is never called.
        let light_config = || Some(epsilon_tui::ThemeMode::Light);
        assert_eq!(
            resolve_theme_mode_from(false, explicit_theme_mode("solarized"), light_config),
            Some(epsilon_tui::ThemeMode::Dark),
            "EPSILON_THEME=solarized + `[tui] theme = \"light\"` must be dark"
        );
        assert_eq!(
            resolve_theme_mode_from(false, explicit_theme_mode("light"), light_config),
            Some(epsilon_tui::ThemeMode::Light)
        );
        assert_eq!(
            resolve_theme_mode_from(false, explicit_theme_mode("dark"), light_config),
            Some(epsilon_tui::ThemeMode::Dark)
        );
        // `--light` still wins over a populated environment.
        assert_eq!(
            resolve_theme_mode_from(true, explicit_theme_mode("dark"), light_config),
            Some(epsilon_tui::ThemeMode::Light)
        );
        // And an *unset* (or empty) variable is the one case that does defer — the
        // user said nothing, so the config is the most explicit thing left. An
        // empty variable is indistinguishable from an unset one, which is how
        // shells and `.env` files behave anyway.
        assert_eq!(explicit_theme_mode("   "), None);
        assert_eq!(
            resolve_theme_mode_from(false, explicit_theme_mode(""), light_config),
            Some(epsilon_tui::ThemeMode::Light),
            "an unset EPSILON_THEME must still honour the config key"
        );
    }

    /// B3's second half: the promise `resolve_theme_mode` makes about the
    /// *environment* is the one the review found broken, so it is the one that has
    /// to be tested through the environment.
    ///
    /// A test process may not write `EPSILON_THEME` — `std::env::set_var` is
    /// `unsafe` from Rust 1.85 and the workspace forbids `unsafe_code` on the
    /// command line, where no local `allow` can overrule it — so the variable is
    /// read from the environment the test binary was *started* with, and this test
    /// asserts the exact decision for whatever that is:
    ///
    /// ```text
    /// EPSILON_THEME= cargo      test -p epsilon-cli --bin epsilon theme
    /// EPSILON_THEME=solarized   cargo test -p epsilon-cli --bin epsilon theme
    /// EPSILON_THEME=light       cargo test -p epsilon-cli --bin epsilon theme
    /// ```
    ///
    /// All three pass on the fixed code and the first two fail on the old one,
    /// which is the negative control the review's version of this test could not
    /// produce: with `solarized` set, the old `from_name`-then-config wiring returns
    /// `None` (or the config's answer) where the documentation promises `Dark`.
    /// Writing the assertion against the variable rather than a constant is what
    /// makes the same test correct in an environment that sets nothing.
    #[test]
    fn the_environment_value_decides_including_a_malformed_one() {
        let variable = epsilon_tui::ui::theme::THEME_ENV;
        let raw = std::env::var(variable).unwrap_or_default();
        if raw.trim().is_empty() {
            // Nothing set: the config is the next source, and asserting *that*
            // keeps this test meaningful in a CI shell with no EPSILON_THEME.
            let from_config = epsilon_core::config::load_config_string("tui", "theme")
                .and_then(|value| epsilon_tui::ThemeMode::from_name(&value));
            assert_eq!(resolve_theme_mode(false), from_config);
            return;
        }
        let expected =
            epsilon_tui::ThemeMode::from_name(&raw).unwrap_or(epsilon_tui::ThemeMode::Dark);
        assert_eq!(
            resolve_theme_mode(false),
            Some(expected),
            "{variable}={raw:?} must select {expected:?} — a typo falls back to the \
             default, not through to the config key"
        );
    }
}
