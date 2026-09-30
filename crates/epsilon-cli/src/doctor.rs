//! `epsilon doctor`: one command that answers "why is this not working?" (M76).
//!
//! The audit's headline finding was that the engine was complete and the *on-ramp* was missing: a
//! user could install the binary, run it, watch it reach no model, and have no way to find out why
//! (M63 fixed the worst of that; this closes the rest). Half of the findings in `audit/01-usability-review.md`
//! would have been self-diagnosing with this command, which is the test for whether it earns its
//! place: every line it prints must be a fact gathered from the same APIs the harness itself uses,
//! never a guess about what the user probably did.
//!
//! Design rules, so the output stays trustworthy:
//!
//! - **Facts, not advice.** Each line is gathered from the code that acts on it. Where the harness
//!   genuinely cannot know something (which sandbox backend a platform will pick before it runs,
//!   whether a lock file's pid is still alive) it says `unknown` rather than a plausible guess.
//! - **No secrets, ever.** Credential lines report *presence* and *source* (`auth.json`,
//!   environment, not needed), never a value — `doctor` output is the kind of thing a user pastes
//!   into a bug report.
//! - **Sections that fail still print.** A section that cannot be read (no session dir, unreadable
//!   config) reports that and the report continues; a diagnostic tool that gives up halfway is
//!   worse than none.
//! - **The exit code means something**: `0` unless a *blocker* was found — a model the catalog does
//!   not know, or a session file another process is writing.

use std::path::{Path, PathBuf};

/// What the CLI knows when it decides to run a diagnosis.
pub struct Inputs<'a> {
    /// The workspace the run would use.
    pub workspace: &'a Path,
    /// `--model` as the user typed it, when they did.
    pub model_arg: Option<&'a str>,
    /// Effective confinement setting (`--sandbox` / config).
    pub confinement: &'a epsilon_tools::Confinement,
    /// MCP servers configured for this run.
    pub mcp_server_count: usize,
    /// Whether this binary was built with the real-provider path.
    pub real_feature: bool,
}

/// Everything the report prints, gathered once so it can be tested without a live world.
#[derive(Debug, Default)]
pub struct Facts {
    /// Crate version.
    pub version: String,
    /// Target OS.
    pub os: String,
    /// Build has the real-provider path.
    pub real_feature: bool,
    /// The model reference the run would use (`--model`, else project config, else saved), and
    /// where it came from.
    pub model_requested: Option<String>,
    /// One of `--model`, `project config`, `saved config` — the precedence the launch uses.
    pub model_source: Option<String>,
    /// Resolved catalog row: id, provider, wire label, env var.
    pub model_row: Option<(String, String, String, String)>,
    /// How the input matched (`exact`, `substring`, …).
    pub model_match: Option<String>,
    /// The row that was replaced to find one with credentials, if that happened.
    pub model_replaced: Option<String>,
    /// A credential is usable for the resolved row.
    pub credentials_ready: bool,
    /// Where the credential comes from: `auth.json`, `environment`, `not needed`.
    pub credential_source: String,
    /// Providers with a usable credential right now (names only).
    pub credentialed_providers: Vec<String>,
    /// Effective confinement setting.
    pub confinement: String,
    /// Platform sandbox description (what the OS can enforce).
    pub sandbox_backend: String,
    /// Language servers found on this machine (M79), by human name.
    pub lsp_servers: Vec<String>,
    /// Workspace root.
    pub workspace: String,
    /// Session directory.
    pub session_dir: String,
    /// Newest session file, if any: name, size in bytes, modified age in seconds.
    pub session_newest: Option<(String, u64, Option<u64>)>,
    /// Entry lines in the newest session file.
    pub session_entries: Option<usize>,
    /// Another process is writing this session right now (from `SessionSummary::in_use`).
    pub session_in_use: bool,
    /// A lock file exists for the newest session, with its recorded holder line.
    pub session_lease: Option<String>,
    /// The workspace is a git repository.
    pub is_git_repo: bool,
    /// Root instruction files present (name, bytes).
    pub instructions: Vec<(String, u64)>,
    /// Project config path and whether it exists.
    pub project_config: Option<(String, bool)>,
    /// Project config trust state, when a config exists: `trusted` / `untrusted`.
    pub project_trusted: Option<bool>,
    /// MCP servers configured for this run.
    pub mcp_server_count: usize,
    /// Built-in tool names registered with the effective confinement.
    pub tools: Vec<String>,
    /// Recent failures from the newest session: one line each, newest last.
    pub recent_failures: Vec<String>,
    /// Things that will stop a run from doing what the user expects, with what to do about it.
    pub blockers: Vec<String>,
}

impl Facts {
    /// The rendered report. Sections whose facts are absent are omitted, never invented.
    pub fn render(&self) -> String {
        let mut out = String::new();
        out.push_str("epsilon doctor\n==============\n\n");

        out.push_str("build\n");
        out.push_str(&format!("  version            {}\n", self.version));
        out.push_str(&format!("  target             {}\n", self.os));
        out.push_str(&format!(
            "  real providers     {}\n",
            if self.real_feature {
                "yes (network path compiled in)"
            } else {
                "NO — built offline-only, this binary cannot reach a model"
            }
        ));

        out.push_str("\nmodel\n");
        match (&self.model_requested, &self.model_source) {
            (Some(m), Some(source)) => {
                out.push_str(&format!("  requested          {m}  (from {source})\n"))
            }
            _ => out
                .push_str("  requested          none — no --model, no project or saved model id\n"),
        }
        match &self.model_row {
            Some((id, provider, wire, env)) => {
                out.push_str(&format!("  resolved           {id}  ({provider})\n"));
                out.push_str(&format!("  wire               {wire}\n"));
                if let Some(kind) = &self.model_match {
                    out.push_str(&format!("  matched by         {kind}\n"));
                }
                if let Some(replaced) = &self.model_replaced {
                    out.push_str(&format!(
                        "  replaced           {replaced} (no credential for it)\n"
                    ));
                }
                if !env.is_empty() {
                    out.push_str(&format!("  credential env     {env}\n"));
                }
            }
            None => out.push_str("  resolved           UNKNOWN — no catalog row matches this\n"),
        }
        out.push_str(&format!(
            "  credentials        {}\n",
            if self.credentials_ready {
                format!("ok ({})", self.credential_source)
            } else {
                format!("MISSING ({})", self.credential_source)
            }
        ));
        if !self.credentialed_providers.is_empty() {
            out.push_str(&format!(
                "  providers with keys {}\n",
                self.credentialed_providers.join(", ")
            ));
        }

        out.push_str("\nsandbox\n");
        out.push_str(&format!("  confinement        {}\n", self.confinement));
        out.push_str(&format!("  platform backend   {}\n", self.sandbox_backend));
        // Discovery only — the doctor must not spawn a language server to describe one.
        if self.lsp_servers.is_empty() {
            out.push_str(
                "  lsp servers        none found — edits carry no diagnostics (install one to get them)\n",
            );
        } else {
            out.push_str(&format!(
                "  lsp servers        {}\n",
                self.lsp_servers.join(", ")
            ));
        }

        out.push_str("\nworkspace\n");
        out.push_str(&format!("  root               {}\n", self.workspace));
        out.push_str(&format!(
            "  git repository     {}\n",
            if self.is_git_repo { "yes" } else { "no" }
        ));
        for (name, bytes) in &self.instructions {
            out.push_str(&format!("  instructions       {name} ({bytes} bytes)\n"));
        }
        if self.instructions.is_empty() {
            out.push_str(
                "  instructions       none (no AGENTS.md / CLAUDE.md / MEMORY.md at the root)\n",
            );
        }
        match &self.project_config {
            Some((path, true)) => out.push_str(&format!("  project config     {path}\n")),
            Some((path, false)) => out.push_str(&format!("  project config     {path} (absent)\n")),
            None => {}
        }
        if let Some(trusted) = self.project_trusted {
            out.push_str(&format!(
                "  project trust      {}\n",
                if trusted {
                    "trusted (project code may load)"
                } else {
                    "NOT trusted — config, skills and hooks from this project are inert"
                }
            ));
        }
        out.push_str(&format!("  mcp servers        {}\n", self.mcp_server_count));

        out.push_str("\nsession\n");
        out.push_str(&format!("  dir                {}\n", self.session_dir));
        match &self.session_newest {
            Some((name, bytes, age)) => {
                out.push_str(&format!("  newest             {name} ({bytes} bytes"));
                match age {
                    Some(secs) => out.push_str(&format!(", {} ago)\n", human_age(*secs))),
                    None => out.push_str(")\n"),
                }
                if let Some(entries) = self.session_entries {
                    out.push_str(&format!("  entries            {entries}\n"));
                }
            }
            None => out.push_str("  newest             none\n"),
        }
        match &self.session_lease {
            Some(holder) if holder.is_empty() => {
                out.push_str("  write lease        held (holder not recorded)\n")
            }
            Some(holder) => out.push_str(&format!("  write lease        held by {holder}\n")),
            None => out.push_str("  write lease        free\n"),
        }

        out.push_str("\ntools\n");
        out.push_str(&format!(
            "  built-in           {} ({})\n",
            self.tools.len(),
            self.tools.join(", ")
        ));

        if !self.recent_failures.is_empty() {
            out.push_str("\nrecent failures (newest last)\n");
            for f in &self.recent_failures {
                out.push_str(&format!("  {f}\n"));
            }
        }

        if self.blockers.is_empty() {
            out.push_str("\nno blockers found\n");
        } else {
            out.push_str("\nblockers\n");
            for b in &self.blockers {
                out.push_str(&format!("  - {b}\n"));
            }
        }
        out
    }

    /// Exit code: non-zero when a blocker was found, so `doctor` can be used in a script.
    pub fn exit_code(&self) -> i32 {
        if self.blockers.is_empty() { 0 } else { 1 }
    }
}

fn human_age(secs: u64) -> String {
    match secs {
        0..=59 => format!("{secs}s"),
        60..=3599 => format!("{}m", secs / 60),
        3600..=86_399 => format!("{}h", secs / 3600),
        _ => format!("{}d", secs / 86_400),
    }
}

/// Gather the facts. Every branch is a real read; a failure to read is reported as its absence.
pub fn gather(inputs: &Inputs<'_>) -> Facts {
    let mut f = Facts {
        version: env!("CARGO_PKG_VERSION").to_string(),
        os: format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
        real_feature: inputs.real_feature,
        model_requested: inputs.model_arg.map(str::to_string),
        confinement: format!("{:?}", inputs.confinement),
        sandbox_backend: platform_backend(),
        lsp_servers: epsilon_lsp::DiagnosticsService::default()
            .discover()
            .into_iter()
            .map(|(def, _path)| def.name.to_string())
            .collect(),
        workspace: inputs.workspace.display().to_string(),
        session_dir: inputs
            .workspace
            .join(".epsilon")
            .join("sessions")
            .display()
            .to_string(),
        mcp_server_count: inputs.mcp_server_count,
        ..Default::default()
    };

    // ---- model + credentials -----------------------------------------------------------------
    // The launch's precedence, verbatim: `--model` > project config > global saved (M40).
    // Asking the same three places is what keeps this report from disagreeing with the run.
    let (project, _, _) = crate::runtime::load_project_config(inputs.workspace);
    let (requested_raw, source) = if let Some(m) = inputs.model_arg {
        (Some(m.to_string()), "--model")
    } else if let Some(m) = project.model_id.clone() {
        (Some(m), "project config")
    } else if let Some(m) = epsilon_core::config::load_model_id() {
        (Some(m), "saved config")
    } else {
        (None, "(nothing set)")
    };
    f.model_requested = requested_raw.clone();
    f.model_source = Some(source.to_string());
    let resolved = match requested_raw.as_deref() {
        Some(raw) => epsilon_llm::catalog::resolve_model_for_run(raw),
        // Nothing requested: an offline run uses the faux default, and a real run needs a
        // model id. Neither is a catalog lookup, so nothing is claimed about a row.
        None => None,
    };
    match resolved {
        Some(r) => {
            f.model_row = Some((
                r.info.id.to_string(),
                r.info.provider.to_string(),
                wire_label(&r.info.wire),
                r.info.env_key.to_string(),
            ));
            f.model_match = Some(match_kind(r.kind));
            f.model_replaced = r.replaced.map(|m| m.id.to_string());
            f.credentials_ready = epsilon_llm::catalog::row_credentials_ready(r.info);
            f.credential_source = credential_source(r.info.provider, r.info.env_key);
            if !f.credentials_ready {
                // The one thing a user most needs to be told, phrased as the command that fixes it.
                f.blockers.push(format!(
                    "no credential for '{}' — run `epsilon auth login {}` (or set {} in the environment)",
                    r.info.provider, r.info.provider, r.info.env_key
                ));
            }
        }
        None => match requested_raw {
            Some(raw) => {
                f.credentials_ready = false;
                f.credential_source = "unknown model".to_string();
                f.blockers.push(format!(
                    "`{raw}` (from {source}) matches no catalog row — `epsilon --list-models` shows the ids"
                ));
            }
            None => {
                // No model requested at all. Say what that means for each path instead of
                // inventing a row, and point at the keys that are already there.
                f.credentials_ready = false;
                f.credential_source = "no model requested".to_string();
                if !f.credentialed_providers.is_empty() {
                    f.blockers.push(format!(
                        "no model set, but credentials exist for {} — `epsilon --model <id>` (or save one with `/model`) picks a real model; without one this runs the offline faux provider",
                        f.credentialed_providers.join(", ")
                    ));
                }
            }
        },
    }
    f.credentialed_providers = epsilon_llm::catalog::credentialed_providers();

    // ---- workspace ---------------------------------------------------------------------------
    f.is_git_repo = inputs.workspace.join(".git").exists();
    // The candidate names the prompt loader consults at the root, in its own precedence
    // order (`epsilon_core::prompt`): the first one present is the one that loads, and a
    // report that omitted `AGENTS.override.md` would tell a user who has one that no
    // instructions are loaded.
    for name in [
        "AGENTS.override.md",
        "AGENTS.md",
        "AGENTS.MD",
        "CLAUDE.md",
        "CLAUDE.MD",
        "MEMORY.md",
    ] {
        let p = inputs.workspace.join(name);
        if let Ok(meta) = std::fs::metadata(&p) {
            f.instructions.push((name.to_string(), meta.len()));
        }
    }
    let project_config = inputs.workspace.join(".epsilon").join("config.toml");
    f.project_config = Some((
        project_config.display().to_string(),
        project_config.exists(),
    ));
    if project_config.exists() {
        // The same call the launch path makes (M39/M45), so the report cannot disagree with it:
        // hash trust composes with a remembered per-path decision.
        let (_, decision, _) = crate::runtime::load_project_config(inputs.workspace);
        let remembered =
            crate::runtime::ProjectTrustStore::open().and_then(|store| store.get(inputs.workspace));
        f.project_trusted = Some(match remembered {
            Some(explicit) => explicit,
            None => !matches!(decision, crate::runtime::TrustDecision::Changed { .. }),
        });
    }

    // ---- tools -------------------------------------------------------------------------------
    f.tools = epsilon_tools::builtin_registry(inputs.confinement)
        .descriptors()
        .into_iter()
        .map(|d| d.name)
        .collect();

    // ---- session -----------------------------------------------------------------------------
    // M217: the same union `--continue` resolves through, so this report cannot describe a
    // different session than the one a launch would continue.
    let dir = crate::runtime::sessions_root(inputs.workspace);
    if let Some(path) = epsilon_session::discover::latest_session_for_cwd(&dir, inputs.workspace) {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        let age = std::fs::metadata(&path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .map(|d| d.as_secs());
        f.session_newest = Some((name, bytes, age));
        let summary = epsilon_session::discover::summarize(&path);
        f.session_in_use = summary.as_ref().is_some_and(|s| s.in_use);
        let (failures, entries) = scan_session(&path, 5);
        f.session_entries = Some(entries);
        f.session_lease = lease_holder(&path);
        if let Some(holder) = &f.session_lease {
            f.blockers.push(format!(
                "another process holds the write lease on {} ({}) — close it, or remove the .lock \
                 file if that process is gone",
                path.display(),
                if holder.is_empty() {
                    "no holder recorded"
                } else {
                    holder
                }
            ));
        }
        f.recent_failures = failures;
    }

    f
}

/// What the platform's confinement can do — asked of the backend, not asserted here (M77).
///
/// This used to be a `cfg` string in this file, which meant the doctor's claim about confinement
/// and the sandbox's actual behaviour were two independent statements that could drift. Now the
/// report prints what the backend declares about itself.
fn platform_backend() -> String {
    let sandbox = epsilon_safety::sandbox::best_available();
    let caps = sandbox.capabilities();
    if caps.is_full() {
        format!("{} (full confinement)", sandbox.name())
    } else {
        format!(
            "{} ({}) — cannot confine: {}",
            sandbox.name(),
            if sandbox.available() {
                "available"
            } else {
                "NOT AVAILABLE — commands that demand confinement are refused"
            },
            caps.missing().join(", ")
        )
    }
}

fn wire_label(wire: &epsilon_llm::catalog::WireApi) -> String {
    // One short label per wire family, from the type the run path actually dispatches on.
    match wire {
        epsilon_llm::catalog::WireApi::AnthropicMessages => "anthropic messages".to_string(),
        epsilon_llm::catalog::WireApi::OpenAiCompletions { .. } => {
            "openai-compatible chat completions".to_string()
        }
        epsilon_llm::catalog::WireApi::OpenAiResponses { .. } => "openai responses".to_string(),
        epsilon_llm::catalog::WireApi::GoogleGenerativeAi => "google generateContent".to_string(),
        epsilon_llm::catalog::WireApi::GoogleVertex => "google vertex".to_string(),
        epsilon_llm::catalog::WireApi::MistralConversations { .. } => {
            "mistral conversations".to_string()
        }
        epsilon_llm::catalog::WireApi::GitHubCopilot { .. } => {
            "github copilot (family-routed)".to_string()
        }
        epsilon_llm::catalog::WireApi::BedrockConverse => "bedrock converse".to_string(),
    }
}

fn match_kind(kind: epsilon_llm::catalog::ModelMatchKind) -> String {
    format!("{kind:?}")
}

/// Where a usable credential comes from — presence only, never a value.
fn credential_source(provider: &str, env_key: &str) -> String {
    if env_key.is_empty() {
        return "not needed (local model)".to_string();
    }
    let stored = epsilon_llm::auth::global_dir()
        .map(|d| epsilon_llm::auth::load_credentials_in(&d))
        .is_some_and(|map| map.get(provider).is_some_and(|c| c.key().is_some()));
    if stored {
        return "stored in auth.json".to_string();
    }
    if std::env::var(env_key).is_ok() {
        return format!("environment ({env_key})");
    }
    "none found (auth.json or environment)".to_string()
}

/// The lease holder recorded in `<session>.lock`, if a lock file exists (M61).
fn lease_holder(session: &Path) -> Option<String> {
    let lock = session.with_extension("jsonl.lock");
    let holder_path = if lock.exists() {
        lock
    } else {
        let alt = PathBuf::from(format!("{}.lock", session.display()));
        if !alt.exists() {
            return None;
        }
        alt
    };
    let text = std::fs::read_to_string(&holder_path).unwrap_or_default();
    // The file holds one line of identity (`pid=1234 …`); an empty file is still a held lease.
    Some(text.lines().next().unwrap_or("").trim().to_string())
}

/// One pass over a session file: the last `n` failures, and how many entry lines it holds.
fn scan_session(path: &Path, n: usize) -> (Vec<String>, usize) {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return (Vec::new(), 0);
    };
    let mut entries = 0usize;
    let mut out: Vec<String> = Vec::new();
    for line in raw.lines() {
        let Ok(entry) =
            serde_json::from_str::<epsilon_contracts::session_entry::SessionEntry>(line)
        else {
            continue;
        };
        entries += 1;
        use epsilon_contracts::session_entry::EntryKind;
        match &entry.kind {
            EntryKind::Message { message, .. } => {
                use epsilon_contracts::message::Message;
                if let Message::ToolResult(t) = message
                    && t.is_error
                {
                    let text: String = t
                        .content
                        .iter()
                        .filter_map(|c| match c {
                            epsilon_contracts::content::ContentBlock::Text { text } => {
                                Some(text.as_str())
                            }
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join(" ");
                    out.push(format!("{}: {}", t.tool_name, first_line(&text)));
                }
            }
            EntryKind::TurnEnd { reason } => {
                use epsilon_contracts::agent_event::TurnEndReason;
                if !matches!(reason, TurnEndReason::Completed) {
                    out.push(format!("turn ended: {reason:?}"));
                }
            }
            _ => {}
        }
    }
    if out.len() > n {
        out.drain(..out.len() - n);
    }
    (out, entries)
}

fn first_line(text: &str) -> String {
    let line = text.lines().next().unwrap_or("").trim();
    if line.chars().count() > 120 {
        let cut: String = line.chars().take(120).collect();
        format!("{cut}…")
    } else {
        line.to_string()
    }
}

/// Run the diagnosis, print it, and return the exit code.
pub fn run(inputs: &Inputs<'_>) -> i32 {
    let facts = gather(inputs);
    print!("{}", facts.render());
    facts.exit_code()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Facts {
        Facts {
            version: "0.1.0".into(),
            os: "windows x86_64".into(),
            real_feature: true,
            confinement: "Auto".into(),
            sandbox_backend: "job object".into(),
            workspace: "/ws".into(),
            session_dir: "/ws/.epsilon/sessions".into(),
            ..Default::default()
        }
    }

    #[test]
    fn an_intact_setup_reports_no_blockers_and_exits_zero() {
        let mut f = base();
        f.model_row = Some((
            "deepseek-flash".into(),
            "deepseek".into(),
            "openai-compatible chat completions".into(),
            "DEEPSEEK_API_KEY".into(),
        ));
        f.credentials_ready = true;
        f.credential_source = "stored in auth.json".into();
        let out = f.render();
        assert!(
            out.contains("credentials        ok (stored in auth.json)"),
            "{out}"
        );
        assert!(out.contains("no blockers found"), "{out}");
        assert_eq!(f.exit_code(), 0, "a clean bill of health is exit 0");
    }

    #[test]
    fn missing_credentials_are_a_blocker_with_the_command_that_fixes_them() {
        let mut f = base();
        f.model_row = Some((
            "deepseek/deepseek-chat-v3.1".into(),
            "openrouter".into(),
            "openai-compatible chat completions".into(),
            "OPENROUTER_API_KEY".into(),
        ));
        f.credentials_ready = false;
        f.credential_source = "none found (auth.json or environment)".into();
        f.blockers.push(
            "no credential for 'openrouter' — run `epsilon auth login openrouter`              (or set OPENROUTER_API_KEY in the environment)"
                .into(),
        );
        let out = f.render();
        assert!(out.contains("credentials        MISSING"), "{out}");
        assert!(out.contains("epsilon auth login openrouter"), "{out}");
        assert_eq!(
            f.exit_code(),
            1,
            "a blocker is a non-zero exit, for scripts"
        );
    }

    #[test]
    fn an_offline_build_says_so_loudly() {
        let f = Facts {
            real_feature: false,
            ..base()
        };
        let out = f.render();
        assert!(
            out.contains("cannot reach a model"),
            "an offline-only binary must not look like a credential problem: {out}"
        );
    }

    #[test]
    fn a_held_lease_is_reported_with_its_holder() {
        let mut f = base();
        f.session_newest = Some(("serve.jsonl".into(), 919, Some(50_400)));
        f.session_lease = Some("pid=1234".into());
        let out = f.render();
        assert!(out.contains("held by pid=1234"), "{out}");
        // Age is humanized, and the size is a fact, not a guess.
        assert!(out.contains("919 bytes"), "{out}");
        assert!(out.contains("14h ago"), "{out}");
    }

    #[test]
    fn a_lease_file_with_no_holder_still_counts_as_held() {
        let mut f = base();
        f.session_lease = Some(String::new());
        let out = f.render();
        assert!(out.contains("held (holder not recorded)"), "{out}");
    }

    #[test]
    fn a_row_without_a_credential_always_comes_with_the_command_that_fixes_it() {
        // `gather` reads this machine's real credentials, so the test cannot assume a particular
        // key is absent — it picks a provider this machine has *no* credential for (there is
        // always at least one, since nobody holds keys for every provider in the catalog) and
        // asserts the invariant a user depends on: no credential ⇒ a blocker naming the remedy.
        // "credentials MISSING" with no guidance is the failure mode this command exists to remove.
        let credentialed = epsilon_llm::catalog::credentialed_providers();
        let uncredentialed = epsilon_llm::catalog::curated()
            .iter()
            .find(|m| !m.env_key.is_empty() && !credentialed.contains(&m.provider.to_string()))
            .expect("some provider in the catalog has no key on any machine");
        let dir = tempfile::tempdir().unwrap();
        let f = gather(&Inputs {
            workspace: dir.path(),
            model_arg: Some(uncredentialed.id),
            confinement: &epsilon_tools::Confinement::Auto,
            mcp_server_count: 0,
            real_feature: true,
        });
        assert!(f.model_row.is_some(), "the id is a real catalog row");
        assert!(
            !f.credentials_ready,
            "{} has no key here",
            uncredentialed.provider
        );
        assert!(
            f.blockers
                .iter()
                .any(|b| b.contains("no credential") && b.contains(uncredentialed.provider)),
            "the blocker names the provider and its remedy: {:?}",
            f.blockers
        );
        assert_eq!(f.exit_code(), 1);
    }

    #[test]
    fn a_row_with_a_credential_reports_ok_and_no_blocker() {
        // The other direction, on whatever provider this machine does hold a key for.
        let Some(provider) = epsilon_llm::catalog::credentialed_providers()
            .into_iter()
            .next()
        else {
            return; // nothing credentialed on this machine: the test has nothing to assert
        };
        let row = epsilon_llm::catalog::curated()
            .iter()
            .find(|m| m.provider == provider)
            .expect("a credentialed provider has at least one row");
        let dir = tempfile::tempdir().unwrap();
        let f = gather(&Inputs {
            workspace: dir.path(),
            model_arg: Some(row.id),
            confinement: &epsilon_tools::Confinement::Auto,
            mcp_server_count: 0,
            real_feature: true,
        });
        assert!(f.credentials_ready, "{} is credentialed here", row.id);
        assert!(
            !f.blockers.iter().any(|b| b.contains("no credential")),
            "a usable credential must not produce a blocker: {:?}",
            f.blockers
        );
    }

    #[test]
    fn an_unknown_model_argument_is_a_blocker_that_names_it() {
        let dir = tempfile::tempdir().unwrap();
        let f = gather(&Inputs {
            workspace: dir.path(),
            model_arg: Some("nonsense-xyz"),
            confinement: &epsilon_tools::Confinement::Auto,
            mcp_server_count: 0,
            real_feature: true,
        });
        assert!(f.model_row.is_none(), "no row claims this id");
        assert!(
            f.blockers
                .iter()
                .any(|b| b.contains("nonsense-xyz") && b.contains("matches no catalog row")),
            "the blocker names the value the user typed and says what is wrong with it: {:?}",
            f.blockers
        );
        assert_eq!(f.exit_code(), 1);
    }

    #[test]
    fn absent_sections_are_reported_as_absent_never_invented() {
        let out = base().render();
        assert!(
            out.contains("UNKNOWN — no catalog row matches this"),
            "{out}"
        );
        assert!(
            out.contains("none (no AGENTS.md / CLAUDE.md / MEMORY.md at the root)"),
            "{out}"
        );
        assert!(out.contains("newest             none"), "{out}");
        assert!(!out.contains("recent failures"), "no failures, no section");
    }

    #[test]
    fn the_report_never_carries_a_secret() {
        // Whatever the credential source string says, it is a *name*, never a value: the whole
        // point of a report a user pastes into a bug report.
        let mut f = base();
        f.credentials_ready = true;
        f.credential_source = "environment (DEEPSEEK_API_KEY)".into();
        f.model_row = Some((
            "deepseek-flash".into(),
            "deepseek".into(),
            "openai-compatible chat completions".into(),
            "DEEPSEEK_API_KEY".into(),
        ));
        let out = f.render();
        assert!(
            out.contains("DEEPSEEK_API_KEY"),
            "the variable NAME is useful"
        );
        for needle in ["sk-", "Bearer ", "api_key=", "token="] {
            assert!(
                !out.contains(needle),
                "no secret shape in the report: {needle}"
            );
        }
    }

    #[test]
    fn recent_failures_are_listed_newest_last() {
        let mut f = base();
        f.recent_failures = vec![
            "bash: cargo test failed".into(),
            "turn ended: Aborted".into(),
        ];
        let out = f.render();
        let bash = out.find("cargo test failed").unwrap();
        let turn = out.find("turn ended: Aborted").unwrap();
        assert!(
            bash < turn,
            "oldest first, so the newest is where the eye lands: {out}"
        );
    }

    #[test]
    fn the_sandbox_line_comes_from_the_backend_not_from_this_file() {
        // The failure this guards against: a `cfg` string in the report drifting away from what
        // the sandbox actually does. The line must name the backend and, when it cannot confine
        // everything, must name what it cannot confine.
        let line = platform_backend();
        let sandbox = epsilon_safety::sandbox::best_available();
        assert!(
            line.contains(sandbox.name()),
            "{line} should name {}",
            sandbox.name()
        );
        let caps = sandbox.capabilities();
        if caps.is_full() {
            assert!(line.contains("full confinement"), "{line}");
        } else {
            for missing in caps.missing() {
                assert!(line.contains(missing), "{line} should name {missing}");
            }
        }
    }

    #[test]
    fn ages_are_humanized_without_lying_about_precision() {
        assert_eq!(human_age(30), "30s");
        assert_eq!(human_age(90), "1m");
        assert_eq!(human_age(3_600), "1h");
        assert_eq!(human_age(90_000), "1d");
    }

    #[test]
    fn long_failure_lines_are_clipped_to_one_line() {
        let long = "x".repeat(400);
        let clipped = first_line(&long);
        assert!(
            clipped.chars().count() <= 121,
            "{}",
            clipped.chars().count()
        );
        assert!(clipped.ends_with('…'));
    }
}
