//! Composition: **the one place that turns a workspace plus a configuration into a
//! running agent.**
//!
//! # Why this crate exists
//!
//! This used to live in `crates/epsilon-cli/src/runtime.rs`, next to `main.rs`, with
//! `build_runtime` a private `async fn` in the binary. That was correct while there was
//! one frontend. A second frontend in a second crate could not reach it, and the
//! alternative — reimplementing skill roots, trust gating, permission layers, resume
//! ordering and the migration passes — is the "two copies of a rule that cannot disagree
//! visibly" defect this repository has paid for before (ADR-012's `global_dir`
//! amendment). See ADR-021.
//!
//! # What belongs here
//!
//! Everything a frontend needs to answer *"what does it take to run an agent against this
//! workspace?"*: conventional-path discovery for skills, extensions and agent definitions;
//! the project-config and trust layer; the LSP policy; extension attachment and the
//! ADR-019 hook installs; the session-path rules; and [`build_runtime`], which composes
//! them.
//!
//! # What does not
//!
//! **No pixels and no argument parsing.** This crate never draws anything and never sees
//! a flag. The terminal frontend, the headless modes and the desktop IDE all call
//! [`build_runtime`] with the same values and get the same agent; what they do with it is
//! theirs.
//!
//! # The one rule a frontend must not break
//!
//! A frontend that wants behaviour the composition does not offer changes **this** crate,
//! not its own copy of it. The desktop IDE is a peer of the terminal, not a privileged
//! one.

#![deny(missing_docs)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use epsilon_contracts::agent_event::AgentEvent;
use epsilon_core::ToolRegistry;
use epsilon_ext::ExtHost;

/// Whether edits ask a language server what the compiler thinks (M79).
///
/// **The policy lives here because the frontend owns the flag (M215).** `--lsp auto|off` is parsed
/// in `main`, and what `epsilon-tools` receives is the handle this produces — so the crate that
/// knows about tools does not have to link a language-server client to be told "never ask".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lsp {
    /// Ask when a server for the file's language is installed; say nothing when none is.
    Auto,
    /// Never ask, never spawn. The default for library callers and tests.
    Off,
}

/// The diagnostics handle a language-server policy buys: `None` when the policy is off.
///
/// One home for "off means no provider, auto means a default service", so no two callers can
/// disagree about what `--lsp` bought. The subprocess credential rule lives in `epsilon-safety`
/// and is attached here, beside the construction, rather than inside a crate that would then need
/// the whole language-server client to apply it.
#[must_use]
pub fn diagnostics_for(
    policy: Lsp,
) -> Option<Arc<dyn epsilon_contracts::diagnostics::DiagnosticsProvider>> {
    match policy {
        Lsp::Off => None,
        Lsp::Auto => {
            let cfg = epsilon_lsp::ServiceConfig {
                scrub_env: Some(Arc::new(|cmd: &mut tokio::process::Command| {
                    epsilon_safety::scrub_secret_env(cmd)
                })),
                ..Default::default()
            };
            Some(Arc::new(epsilon_lsp::DiagnosticsService::new(cfg)))
        }
    }
}

/// Parsed `.epsilon/config.toml` — the project layer of ADR-012.
///
/// Declarative only; unknown keys are preserved on disk but ignored by the runtime.
///
/// M45: trust is TWO mechanisms, not one. The config-hash below answers "did
/// THIS config file change?" (per-file integrity). The [`ProjectTrustStore`]
/// answers "did the USER trust THIS directory?" (per-path memory, pi
/// `trust.json`: path → true/false/null with nearest-ancestor inheritance).
/// The store gates LOADING (skills/extensions/prompts at the roots); the hash
/// gates the config's own rules. A remembered `false` denies even an unchanged
/// config; `null`/absent falls back to the hash decision.
#[derive(Debug, Default, Clone)]
pub struct ProjectConfig {
    /// `[model] id` — overrides the global saved model, under the `--model` flag.
    pub model_id: Option<String>,
    /// `[[rules]]` entries stacked over the defaults as a `"project"` layer
    /// (last-match-wins; any deny still wins immediately per ADR-006 semantics).
    pub rules: Vec<ProjectRule>,
    /// `[[extensions]]` dirs relative to `.epsilon/`.
    pub extension_dirs: Vec<String>,
    /// `[[mcp_servers]]` stdio servers, same shape as the `--mcp` flag.
    pub mcp_servers: Vec<epsilon_mcp::McpServerConfig>,
}

/// One `[[rules]]` entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectRule {
    /// Capability name (`bash`, `read`, `external_dir`, …).
    pub capability: String,
    /// Glob pattern for the gate subject (`*` allowed).
    pub pattern: String,
    /// `"allow" | "ask" | "deny"`.
    pub action: String,
}

const PROJECT_CONFIG_REL: &str = ".epsilon/config.toml";
const TRUST_FILE_REL: &str = ".epsilon/trust.json";

/// Outcome of the project-config trust check (ADR-012).
///
/// v1 (headless) collapsed everything into banners; M22 makes the decision
/// explicit so the TUI can re-prompt and headless modes keep their policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrustDecision {
    /// No project config present (or unreadable): nothing to trust.
    Absent,
    /// First sight: hash recorded, config loaded.
    Trusted,
    /// Hash matches the recorded one.
    Unchanged,
    /// Config content changed since it was trusted; `previous` is the old hash.
    Changed {
        /// The SHA-256 recorded in `.epsilon/trust.json` before this run.
        previous: String,
    },
}

/// Load + trust-check the project config. Returns `(config, trust decision,
/// diagnostics)`.
///
/// Trust policy: first sight records the hash and loads; an unchanged hash
/// loads silently; a CHANGED hash still loads but reports [`TrustDecision::
/// Changed`] so frontends decide — the interactive TUI re-prompts (pi's
/// trust-manager semantics), headless modes print the loud banner as before.
///
/// M39 scoping rule: the hash gates `.epsilon/config.toml` ONLY. Skills,
/// extensions, prompts, themes, and SYSTEM.md are gated separately by
/// [`trust_requiring_resources`] at load time (an untrusted project yields
/// empty skill/extension roots instead of executing third-party code).
/// Collapsing every resource onto one hash would re-trust silently on any
/// config edit; per-resource gating keeps the decision attached to the code
/// that actually runs.
pub fn load_project_config(workspace: &Path) -> (ProjectConfig, TrustDecision, Vec<String>) {
    let mut notices = Vec::new();
    let path = workspace.join(PROJECT_CONFIG_REL);
    if !path.is_file() {
        return (ProjectConfig::default(), TrustDecision::Absent, notices);
    }
    let raw = match std::fs::read_to_string(&path) {
        Ok(r) => r,
        Err(e) => {
            notices.push(format!("project config unreadable: {e}"));
            return (ProjectConfig::default(), TrustDecision::Absent, notices);
        }
    };

    // ---- trust hash -----------------------------------------------------------------
    let hash = sha256_hex(raw.as_bytes());
    let trust_path = workspace.join(TRUST_FILE_REL);
    let previous = std::fs::read_to_string(&trust_path)
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| {
            v.get("config_sha256")
                .and_then(|h| h.as_str())
                .map(String::from)
        });
    let decision = match &previous {
        None => {
            notices.push(format!(
                "trusting project config {} (hash recorded)",
                path.display()
            ));
            TrustDecision::Trusted
        }
        Some(prev) if prev != &hash => TrustDecision::Changed {
            previous: prev.clone(),
        },
        Some(_) => TrustDecision::Unchanged,
    };
    // Only persist on first trust / confirmed-unchanged; a changed config keeps
    // the OLD hash until the user accepts it, so every run re-reports until then.
    if !matches!(decision, TrustDecision::Changed { .. }) || previous.is_none() {
        let _ = std::fs::write(
            &trust_path,
            serde_json::json!({ "config_sha256": hash }).to_string(),
        );
    }

    // ---- parse ----------------------------------------------------------------------
    let mut cfg = ProjectConfig::default();
    let parsed: toml::Value = match toml::from_str(&raw) {
        Ok(v) => v,
        Err(e) => {
            notices.push(format!("invalid project config: {e}"));
            return (cfg, decision, notices);
        }
    };

    cfg.model_id = parsed
        .get("model")
        .and_then(|m| m.get("id"))
        .and_then(|v| v.as_str())
        .map(str::to_string);

    if let Some(rules) = parsed.get("rules").and_then(|r| r.as_array()) {
        for r in rules {
            let cap = r.get("capability").and_then(|v| v.as_str());
            let pat = r.get("pattern").and_then(|v| v.as_str());
            let act = r.get("action").and_then(|v| v.as_str());
            match (cap, pat, act) {
                (Some(c), Some(p), Some(a)) if matches!(a, "allow" | "ask" | "deny") => {
                    cfg.rules.push(ProjectRule {
                        capability: c.into(),
                        pattern: p.into(),
                        action: a.into(),
                    });
                }
                _ => notices.push(format!("ignoring malformed [[rules]] entry: {r}")),
            }
        }
    }

    if let Some(exts) = parsed.get("extensions").and_then(|r| r.as_array()) {
        for e in exts {
            if let Some(dir) = e.get("dir").and_then(|v| v.as_str()) {
                cfg.extension_dirs.push(dir.to_string());
            } else {
                notices.push(format!("ignoring [[extensions] without dir: {e}"));
            }
        }
    }

    if let Some(servers) = parsed.get("mcp_servers").and_then(|r| r.as_array()) {
        for s in servers {
            match (
                s.get("name").and_then(|v| v.as_str()),
                s.get("command").and_then(|v| v.as_str()),
            ) {
                (Some(name), Some(command)) => {
                    cfg.mcp_servers.push(epsilon_mcp::McpServerConfig {
                        name: name.into(),
                        command: command.into(),
                        args: s
                            .get("args")
                            .and_then(|a| a.as_array())
                            .map(|a| {
                                a.iter()
                                    .filter_map(|v| v.as_str().map(String::from))
                                    .collect()
                            })
                            .unwrap_or_default(),
                    });
                }
                _ => notices.push(format!(
                    "ignoring [[mcp_servers]] missing name/command: {s}"
                )),
            }
        }
    }

    (cfg, decision, notices)
}

#[cfg(test)]
mod project_hash_tests {
    use super::*;

    /// M226: the project-config hash had **two implementations** -- this crate's
    /// current_project_config_hash, which **nothing called**, and
    /// psilon_core::config::current_project_config_hash_in, which the TUI uses. They were
    /// compared on a real file and agreed, and then the uncalled one was deleted: a duplicate that
    /// differed would not have been a duplicate but a second answer, and deleting the wrong one
    /// would change which hash a trust prompt records.
    ///
    /// What remains pinned is the *fact* the pair answered: the digest a trust prompt records must
    /// be over the same bytes the settings were parsed from. Plan 019 step 5 wanted to protect
    /// exactly that by moving the parse into psilon-core; the measurement said it does not need
    /// moving, because load_project_config reads the file **once** and uses that one string for
    /// the hash and the parse. This test makes the property explicit rather than a coincidence of
    /// statement order -- which is the part of the plan's worry that survives its retirement.
    #[test]
    fn the_recorded_hash_covers_the_parsed_bytes() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path();
        std::fs::create_dir_all(ws.join(".epsilon")).unwrap();
        let body = "[model]\nid = \"faux-9\"\n\n[[rules]]\ncapability = \"bash\"\npattern = \"ls\"\naction = \"allow\"\n";
        std::fs::write(ws.join(".epsilon").join("config.toml"), body).unwrap();

        let (cfg, _decision, notices) = load_project_config(ws);
        assert!(
            !notices
                .iter()
                .any(|n| n.contains("invalid") || n.contains("ignoring")),
            "a valid config produces no complaints: {notices:?}"
        );
        assert_eq!(
            cfg.model_id.as_deref(),
            Some("faux-9"),
            "the settings came through"
        );
        assert_eq!(
            epsilon_core::config::current_project_config_hash_in(ws).as_deref(),
            Some(sha256_hex(body.as_bytes()).as_str()),
            "and the hash the TUI will record is over exactly those bytes"
        );

        // One byte later is a different digest -- which is the whole point of hashing the file.
        std::fs::write(
            ws.join(".epsilon").join("config.toml"),
            format!("{body}# changed"),
        )
        .unwrap();
        assert_ne!(
            epsilon_core::config::current_project_config_hash_in(ws).as_deref(),
            Some(sha256_hex(body.as_bytes()).as_str()),
            "so a trust prompt notices an edit made while it was open"
        );
    }

    /// A workspace with no project config has no hash, which is how a caller tells "never
    /// written" from "written and accepted".
    #[test]
    fn a_workspace_without_a_project_config_has_no_hash() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(
            epsilon_core::config::current_project_config_hash_in(tmp.path()),
            None
        );
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(bytes);
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// Record `hash` as trusted in `<workspace>/.epsilon/trust.json` (the user
/// accepted a changed project config — pi trust-manager's "trust" answer).
pub fn accept_project_config_hash(workspace: &Path, hash: &str) -> Result<(), String> {
    let path = workspace.join(TRUST_FILE_REL);
    std::fs::write(
        &path,
        serde_json::json!({ "config_sha256": hash }).to_string(),
    )
    .map_err(|e| format!("could not write {}: {e}", path.display()))
}

/// path → trusted map (pi `TrustFile`: `true`/`false`; `null` removes the key).
type TrustMap = BTreeMap<String, bool>;

/// Remembered per-path trust decision (pi `ProjectTrustDecision`):
/// `Some(true)` trusted, `Some(false)` refused, `None` never asked.
pub type ProjectTrustDecision = Option<bool>;

/// One remembered decision: nearest ancestor with a `true`/`false` value wins
/// (pi `findNearestTrustEntry`); `null` entries clear inheritance without
/// deciding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectTrustEntry {
    /// Canonicalized directory path the decision was recorded for.
    pub path: String,
    /// The remembered decision.
    pub decision: bool,
}

/// Persistent per-path trust memory (pi `ProjectTrustStore`).
///
/// Lives at `<global_dir>/trust.json` (pi: `<agentDir>/trust.json`): a
/// `path → true/false/null` map, keys sorted on write, `null` (via
/// [`ProjectTrustStore::clear`]) removing the key. Reads and writes hold the
/// auth-store lock discipline — a `trust.lock` sibling with stale-steal — so
/// concurrent frontends never interleave a torn write.
pub struct ProjectTrustStore {
    path: PathBuf,
}

impl ProjectTrustStore {
    /// Open the store under `global_dir` (`None` when no home resolves —
    /// callers fall back to hash-only trust).
    pub fn open() -> Option<Self> {
        let dir = epsilon_core::config::global_dir()?;
        Some(Self {
            path: dir.join("trust.json"),
        })
    }

    /// Open at an explicit path (tests).
    pub fn open_in(path: PathBuf) -> Self {
        Self { path }
    }

    /// Remembered decision for `cwd`: the nearest `cwd`-or-ancestor entry,
    /// or `None` when nothing was ever recorded (pi `get`).
    pub fn get(&self, cwd: &Path) -> ProjectTrustDecision {
        self.get_entry(cwd).map(|e| e.decision)
    }

    /// Nearest entry at or above `cwd` (pi `getEntry`).
    pub fn get_entry(&self, cwd: &Path) -> Option<ProjectTrustEntry> {
        let _lock = self.lock()?;
        let data = self.read()?;
        find_nearest(&data, cwd)
    }

    /// Record one decision (`None` clears the key — pi's `null`).
    pub fn set(&self, cwd: &Path, decision: ProjectTrustDecision) -> Result<(), String> {
        self.set_many(std::slice::from_ref(&(cwd.to_path_buf(), decision)))
    }

    /// Record several decisions atomically, sorted keys on write (pi `setMany`).
    pub fn set_many(&self, decisions: &[(PathBuf, ProjectTrustDecision)]) -> Result<(), String> {
        let _lock = self
            .lock()
            .ok_or_else(|| "could not lock the trust store".to_string())?;
        let mut data = self.read().unwrap_or_default();
        for (path, decision) in decisions {
            let key = canonicalize(path);
            match decision {
                None => {
                    data.remove(&key);
                }
                Some(d) => {
                    data.insert(key, *d);
                }
            }
        }
        self.write(&data)
    }

    /// Remove the entry for `cwd` (forget — the next run asks again).
    pub fn clear(&self, cwd: &Path) -> Result<(), String> {
        self.set(cwd, None)
    }

    fn lock(&self) -> Option<LockGuard> {
        LockGuard::acquire(&self.path)
    }

    fn read(&self) -> Option<TrustMap> {
        let raw = std::fs::read_to_string(&self.path).ok()?;
        if raw.trim().is_empty() {
            return Some(TrustMap::new());
        }
        let value: serde_json::Value = serde_json::from_str(&raw).ok()?;
        let obj = value.as_object()?;
        let mut map = TrustMap::new();
        for (k, v) in obj {
            match v {
                serde_json::Value::Bool(b) => {
                    map.insert(k.clone(), *b);
                }
                serde_json::Value::Null => {} // `null` never persists; skip defensively
                _ => return None,             // corrupt value: treat store as unreadable
            }
        }
        Some(map)
    }

    fn write(&self, data: &TrustMap) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("could not create {}: {e}", parent.display()))?;
        }
        let mut keys: Vec<&String> = data.keys().collect();
        keys.sort();
        let mut obj = serde_json::Map::with_capacity(data.len());
        for k in keys {
            obj.insert(k.clone(), serde_json::Value::Bool(data[k]));
        }
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::Value::Object(obj).to_string())
            .map_err(|e| format!("could not write {}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, &self.path)
            .map_err(|e| format!("could not publish {}: {e}", self.path.display()))?;
        Ok(())
    }
}

/// Canonicalize a workspace path for store keys: absolute + lexically
/// normalized (pi `normalizeCwd`: `canonicalizePath(resolvePath(cwd))`).
fn canonicalize(path: &Path) -> String {
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(path)
    };
    let mut out = PathBuf::new();
    for comp in abs.components() {
        match comp {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out.to_string_lossy().into_owned()
}

/// Nearest `cwd`-or-ancestor key with a decision (pi `findNearestTrustEntry`).
fn find_nearest(data: &TrustMap, cwd: &Path) -> Option<ProjectTrustEntry> {
    let mut dir = canonicalize(cwd);
    loop {
        if let Some(d) = data.get(&dir) {
            return Some(ProjectTrustEntry {
                path: dir,
                decision: *d,
            });
        }
        let parent = Path::new(&dir).parent()?.to_string_lossy().into_owned();
        if parent == dir {
            return None;
        }
        dir = parent;
    }
}

/// Cross-process lock for the trust store (same stale-steal discipline as the
/// auth store: a `trust.lock` sibling; holders older than 30s are presumed
/// dead). Returns `None` when the lock directory is unusable — callers treat
/// that as "store unavailable", never as trusted.
struct LockGuard {
    path: PathBuf,
}

impl LockGuard {
    fn acquire(store_path: &Path) -> Option<Self> {
        let dir = store_path.parent()?;
        std::fs::create_dir_all(dir).ok()?;
        let path = store_path.with_extension("json.lock");
        // Best-effort create-exclusive; stale holders (>30s) are stolen.
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut f) => {
                use std::io::Write;
                let _ = writeln!(f, "{}", std::process::id());
                Some(Self { path })
            }
            Err(_) => {
                let stale = std::fs::metadata(&path)
                    .ok()?
                    .modified()
                    .ok()?
                    .elapsed()
                    .map(|e| e.as_secs() > 30)
                    .unwrap_or(false);
                if stale {
                    let _ = std::fs::remove_file(&path);
                    Self::acquire(store_path)
                } else {
                    None
                }
            }
        }
    }
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Project-local resource kinds that require trust before loading
/// (pi `TRUST_REQUIRING_PROJECT_CONFIG_RESOURCES` + `.agents/skills` walk).
/// Anything on this list found under the workspace must be gated on the
/// project being trusted — loading it untrusted would execute third-party code
/// or instructions (extensions, skills, prompts-as-system-text, themes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustRequiringResource {
    /// `.epsilon/config.toml` (`settings.json` in pi: policy + rules).
    ProjectConfig,
    /// `.epsilon/extensions/` (pi `extensions`).
    Extensions,
    /// `.epsilon/skills/` or `.agents/skills/` in the workspace tree.
    Skills,
    /// `.epsilon/prompts/` (pi `prompts`: user-invoked slash templates, not yet ported —
    /// gated anyway, so the day they load they cannot load untrusted).
    Prompts,
    /// `.epsilon/themes/` (pi `themes`: TUI theme code).
    Themes,
    /// `.epsilon/SYSTEM.md` (pi `SYSTEM.md`: replaces the built-in contract).
    SystemMd,
    /// `.epsilon/APPEND_SYSTEM.md` (pi `APPEND_SYSTEM.md`: appended to the system prompt).
    AppendSystemMd,
}

impl TrustRequiringResource {
    /// Human-readable path fragment for diagnostics.
    pub fn describe(self) -> &'static str {
        match self {
            TrustRequiringResource::ProjectConfig => ".epsilon/config.toml",
            TrustRequiringResource::Extensions => ".epsilon/extensions/",
            TrustRequiringResource::Skills => "skills (.epsilon/skills or .agents/skills)",
            TrustRequiringResource::Prompts => ".epsilon/prompts/",
            TrustRequiringResource::Themes => ".epsilon/themes/",
            TrustRequiringResource::SystemMd => ".epsilon/SYSTEM.md",
            TrustRequiringResource::AppendSystemMd => ".epsilon/APPEND_SYSTEM.md",
        }
    }
}

/// Scan the workspace for trust-requiring project resources
/// (pi `hasTrustRequiringProjectResources`).
///
/// `.epsilon/config.toml` + every sibling resource dir/file counts directly;
/// `.agents/skills` counts in the workspace AND any ancestor up to the
/// filesystem root (pi `hasTrustRequiringProjectResources` walks to root;
/// a parent repo's skill tree applies to the project, so it gates too).
pub fn trust_requiring_resources(workspace: &Path) -> Vec<TrustRequiringResource> {
    let mut found = Vec::new();
    let dot = workspace.join(".epsilon");
    if workspace.join(".epsilon/config.toml").is_file() {
        found.push(TrustRequiringResource::ProjectConfig);
    }
    for (name, kind) in [
        ("extensions", TrustRequiringResource::Extensions),
        ("skills", TrustRequiringResource::Skills),
        ("prompts", TrustRequiringResource::Prompts),
        ("themes", TrustRequiringResource::Themes),
    ] {
        if dot.join(name).is_dir() {
            found.push(kind);
        }
    }
    if dot.join("SYSTEM.md").is_file() {
        found.push(TrustRequiringResource::SystemMd);
    }
    if dot.join("APPEND_SYSTEM.md").is_file() {
        found.push(TrustRequiringResource::AppendSystemMd);
    }
    // `.agents/skills` in the workspace or any ancestor (pi walks to root).
    let mut dir: Option<&Path> = Some(workspace);
    while let Some(d) = dir {
        if d.join(".agents").join("skills").is_dir() {
            if !found.contains(&TrustRequiringResource::Skills) {
                found.push(TrustRequiringResource::Skills);
            }
            break;
        }
        dir = d.parent();
    }
    found
}

/// True when the workspace carries anything trust must gate
/// (pi `hasTrustRequiringProjectResources` as a boolean).
pub fn has_trust_requiring_resources(workspace: &Path) -> bool {
    !trust_requiring_resources(workspace).is_empty()
}

/// Convert declared project rules into an engine layer appended over the defaults.
pub fn project_rules_layer(rules: &[ProjectRule]) -> epsilon_safety::Ruleset {
    use epsilon_safety::{Action, Rule, Ruleset};
    Ruleset {
        name: "project".into(),
        rules: rules
            .iter()
            .map(|r| {
                Rule::new(
                    &r.capability,
                    &r.pattern,
                    match r.action.as_str() {
                        "allow" => Action::Allow,
                        "deny" => Action::Deny,
                        _ => Action::Ask,
                    },
                )
            })
            .collect(),
    }
}

/// Skill roots: global (`~/.config/epsilon/skills`) then project (`.epsilon/skills`).
///
/// M39: pass `project_trusted = false` to curtail project-local roots when the
/// project is untrusted (pi's "settings/extensions curtailed" scoping) — the
/// global root always loads, so built-in skills keep working while third-party
/// project skills stay out until the user trusts them.
pub fn skill_roots(workspace: &Path) -> Vec<PathBuf> {
    skill_roots_trusted(workspace, true)
}

/// [`skill_roots`] with an explicit trust gate.
pub fn skill_roots_trusted(workspace: &Path, project_trusted: bool) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(dir) = epsilon_core::config::global_dir() {
        roots.push(dir.join("skills"));
    }
    if !project_trusted {
        return roots;
    }
    // Portable cross-harness convention (.agents/skills — read by most harnesses;
    // lets one skill tree serve Claude Code, opencode, Epsilon, ...).
    roots.push(workspace.join(".agents").join("skills"));
    // Native project-local location (M11).
    roots.push(workspace.join(".epsilon").join("skills"));
    roots
}

/// Extension roots: global (`~/.config/epsilon/extensions`) then project.
///
/// M39: same trust gate as [`skill_roots_trusted`] — untrusted projects load
/// global extensions only, never project code.
pub fn extension_roots(workspace: &Path) -> Vec<PathBuf> {
    extension_roots_trusted(workspace, true)
}

/// [`extension_roots`] with an explicit trust gate.
pub fn extension_roots_trusted(workspace: &Path, project_trusted: bool) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(dir) = epsilon_core::config::global_dir() {
        roots.push(dir.join("extensions"));
    }
    if !project_trusted {
        return roots;
    }
    roots.push(workspace.join(".epsilon").join("extensions"));
    roots
}

/// Agent-def roots: global (`~/.config/epsilon/agents`) always, then project
/// (`.agents/agents` portable + `.epsilon/agents` native) only when trusted.
///
/// M50 (pi `agents.ts` `discoverAgents` parity): pi's `agentScope` gate is
/// `"user"` by default — project-local agent defs (repo-controlled prompts
/// that steer delegated children) load only with explicit scope. The M39
/// trust lattice is that gate here: untrusted projects get global defs only.
/// Split into user/project halves (not one flat list) so the core discovery
/// can enforce the gate itself rather than trusting callers to pass the flag.
pub fn agent_user_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(dir) = epsilon_core::config::global_dir() {
        roots.push(dir.join("agents"));
    }
    roots
}

/// Project halves of [`agent_user_roots`] (gated by trust at the call site).
pub fn agent_project_roots(workspace: &Path, project_trusted: bool) -> Vec<PathBuf> {
    if !project_trusted {
        return Vec::new();
    }
    vec![
        workspace.join(".agents").join("agents"),
        workspace.join(".epsilon").join("agents"),
    ]
}

/// Discover manifests under the given roots: every immediate subdirectory containing
/// `extension.json`. Invalid manifests are skipped and reported as diagnostics instead
/// of failing startup (one broken extension must not take down the session).
pub fn discover_extension_manifests(
    roots: &[PathBuf],
) -> (Vec<(PathBuf, epsilon_ext::ExtManifest)>, Vec<String>) {
    let mut found: Vec<(PathBuf, epsilon_ext::ExtManifest)> = Vec::new();
    let mut problems = Vec::new();
    for root in roots {
        let Ok(entries) = std::fs::read_dir(root) else {
            continue;
        };
        for entry in entries.filter_map(Result::ok) {
            let dir = entry.path();
            if !dir.is_dir() {
                continue;
            }
            match epsilon_ext::manifest::load(&dir) {
                Ok(manifest) => {
                    if found
                        .iter()
                        .any(|(_, existing)| existing.name == manifest.name)
                    {
                        problems.push(format!(
                            "duplicate extension name '{}' at {}; keeping first",
                            manifest.name,
                            dir.display()
                        ));
                    } else {
                        found.push((dir, manifest));
                    }
                }
                Err(e) => problems.push(e),
            }
        }
    }
    (found, problems)
}

/// Spawn every discovered extension and register its declared tools into `registry`.
///
/// Tool names are namespaced `ext__{extension}__{tool}` so third-party tools cannot
/// shadow built-ins. Returns the live hosts (for event forwarding) plus human-readable
/// diagnostics lines.
pub async fn attach_extensions(
    registry: &mut ToolRegistry,
    roots: &[PathBuf],
) -> (Vec<Arc<ExtHost>>, Vec<String>) {
    let mut notices = Vec::new();
    let (manifests, mut problems) = discover_extension_manifests(roots);
    let mut hosts = Vec::new();
    for (dir, manifest) in manifests {
        match ExtHost::spawn(&dir, &manifest).await {
            Ok(host) => {
                let mut registered = 0usize;
                for spec in &manifest.tools {
                    let qualified = format!("ext__{}__{}", manifest.name, spec.name);
                    if registry.get(&qualified).is_some() {
                        problems.push(format!(
                            "extension '{}' tool '{}' skipped: name collision",
                            manifest.name, qualified
                        ));
                        continue;
                    }
                    registry.register(Arc::new(epsilon_ext::ExtTool::new(spec, Arc::clone(&host))));
                    registered += 1;
                }
                let state = if host.is_disabled() {
                    " (disabled)"
                } else {
                    ""
                };
                notices.push(format!(
                    "extension '{}' v{}{} attached ({registered} tools)",
                    manifest.name, manifest.version, state
                ));
                hosts.push(host);
            }
            Err(e) => problems.push(format!(
                "extension '{}' failed to start: {e}",
                manifest.name
            )),
        }
    }
    notices.append(&mut problems);
    (hosts, notices)
}

// ---------------------------------------------------------------------------
// ADR-019: extensions as tool-call interceptors.
//
// `ExtHost` can ask an extension for a decision and get one back (`interpose`). What was
// missing is the other half: something that implements `ToolHooks` and speaks to those
// hosts, plus the one call that installs it on the agent. Until this existed, every
// `set_tool_hooks` call site in the repository was a test.
//
// This lives in `epsilon-cli` on purpose. `epsilon-ext` depends only on `epsilon-contracts`
// and must keep doing so, or the process boundary stops being testable in isolation;
// `epsilon-core` does not depend on `epsilon-ext` at all. `epsilon-cli` is the one crate
// that sees both, and it already owns `spawn_event_forwarder`.

/// Fan one interception out to every attached extension and reduce the answers to a single
/// [`BeforeToolCallResult`].
///
/// **Any** extension that blocks blocks the call. That is deliberately not a majority or a
/// first-wins rule: an extension declaring a veto is asserting something the harness cannot
/// check for it, and a harness that lets a second opinion overturn the first would make the
/// safety guarantee depend on extension load order. Adding an extension is adding a
/// restriction, never removing one.
///
/// Extensions that do not answer contribute nothing. Under ADR-019 a timeout, a malformed
/// answer, or a child over its crash budget all mean "no opinion" — so a broken extension
/// degrades to silence rather than to a veto, and a broken extension can never be the
/// reason a run cannot finish.
pub struct ExtensionToolHooks {
    hosts: Vec<Arc<ExtHost>>,
}

impl ExtensionToolHooks {
    /// Build hooks over the currently attached extension hosts.
    ///
    /// An empty host list yields hooks that never interpose, which is the common case: a
    /// harness with no extensions must not pay a round trip per tool call to find that out.
    pub fn new(hosts: Vec<Arc<ExtHost>>) -> Self {
        Self { hosts }
    }

    /// True when there is nothing to ask, so the caller can skip installing hooks entirely.
    pub fn is_empty(&self) -> bool {
        self.hosts.is_empty()
    }
}

#[async_trait::async_trait]
impl epsilon_contracts::tool::ToolHooks for ExtensionToolHooks {
    async fn before_tool_call(
        &self,
        ctx: epsilon_contracts::tool::BeforeToolCallContext,
    ) -> Result<epsilon_contracts::tool::BeforeToolCallResult, String> {
        if self.hosts.is_empty() {
            return Ok(epsilon_contracts::tool::BeforeToolCallResult::default());
        }

        // The event shape is pi's `beforeToolCall` payload: the tool name and its validated
        // arguments, under the `before_tool_call` tag the manifest filter matches on.
        let event = serde_json::json!({
            "type": "before_tool_call",
            "tool_call_id": ctx.tool_call_id.as_str(),
            "tool_name": ctx.tool_name,
            "args": ctx.args,
        });

        let mut out = epsilon_contracts::tool::BeforeToolCallResult::default();
        let mut reasons: Vec<String> = Vec::new();

        for host in &self.hosts {
            // Sequential, not concurrent: a veto should be reported without waiting out
            // every other extension's timeout, and extensions are rare enough that the
            // latency does not matter. The concurrency that *is* load-bearing is inside a
            // host, where one extension's reply can arrive while its own tool call is
            // outstanding.
            for decision in host.interpose(&event).await {
                let Some(d) = decision.get("decision") else {
                    continue;
                };
                // A decision that is not an object is ignored rather than believed. Reading
                // the string "block" as a block would let one buggy extension deny every
                // tool call; reading it as consent would ignore an extension trying to
                // object. Silence is the only safe third reading.
                if !d.is_object() {
                    continue;
                }
                if d.get("block").and_then(|b| b.as_bool()) == Some(true) {
                    out.block = true;
                    let why = d
                        .get("reason")
                        .and_then(|r| r.as_str())
                        .unwrap_or("blocked by an extension");
                    reasons.push(why.to_string());
                    // pi's `terminate` is a *hint* honoured only when every blocked call in
                    // a batch asks for it (`ToolCallEventResult.terminate`). Carried on the
                    // same struct so the loop can apply that rule rather than this bridge
                    // guessing at it.
                    if d.get("terminate").and_then(|t| t.as_bool()) == Some(true) {
                        out.terminate = true;
                    }
                }
            }
        }

        if out.block {
            // Name every extension that objected. One reason string is not enough: a user
            // told "blocked" with no attribution has nowhere to go next, and the message
            // is what lands in the transcript.
            out.reason = Some(format!(
                "blocked by extension policy: {}",
                reasons.join("; ")
            ));
        }
        Ok(out)
    }

    async fn after_tool_call(
        &self,
        ctx: epsilon_contracts::tool::AfterToolCallContext,
    ) -> Result<Option<epsilon_contracts::tool::AfterToolCallResult>, String> {
        if self.hosts.is_empty() {
            return Ok(None);
        }

        // pi's `ToolResultEventBase` shape: the id, the validated input, the content, the
        // error flag, optional usage, and the per-tool `details` payload. Sending
        // `details` matters more than it looks — it is what a renderer consumes, and an
        // extension that redacts a file listing needs to see the listing to redact it.
        let event = serde_json::json!({
            "type": "tool_result",
            "tool_call_id": ctx.tool_call_id.as_str(),
            "tool_name": ctx.tool_name,
            "input": ctx.args,
            "content": ctx.result.content,
            "is_error": ctx.result.is_error,
            "details": ctx.result.details,
            "usage": ctx.result.usage,
        });

        let mut out: Option<epsilon_contracts::tool::AfterToolCallResult> = None;
        let mut reasons: Vec<String> = Vec::new();

        for host in &self.hosts {
            for decision in host.interpose(&event).await {
                let Some(d) = decision.get("decision") else {
                    continue;
                };
                if !d.is_object() {
                    continue;
                }
                // pi folds each handler's answer into a running event
                // (`runner.ts:29468-30490`) and returns `undefined` when nothing was
                // modified. Returning `None` for an untouched result is therefore not a
                // shortcut -- it is what lets the caller keep its own message identical,
                // including fields this bridge has no opinion about.
                let mut touched = false;

                if let Some(content) = d.get("content").and_then(|c| c.as_array()) {
                    out.get_or_insert_with(Default::default).content =
                        Some(content.iter().filter_map(content_block_from_json).collect());
                    touched = true;
                }
                if let Some(details) = d.get("details") {
                    out.get_or_insert_with(Default::default).details = Some(details.clone());
                    touched = true;
                }
                if let Some(is_error) = d.get("is_error").and_then(|e| e.as_bool()) {
                    out.get_or_insert_with(Default::default).is_error = Some(is_error);
                    touched = true;
                }
                if let Some(usage) = d.get("usage") {
                    out.get_or_insert_with(Default::default).usage = usage_from_json(usage);
                    touched = true;
                }
                if let Some(terminate) = d.get("terminate").and_then(|t| t.as_bool()) {
                    out.get_or_insert_with(Default::default).terminate = Some(terminate);
                    touched = true;
                }
                let _ = touched;
                if let Some(why) = d.get("reason").and_then(|r| r.as_str()) {
                    reasons.push(why.to_string());
                }
            }
        }

        let Some(mut result) = out else {
            // Nothing was modified, so there is nothing to hand back. The caller keeps its
            // own message byte-for-byte, which is the whole point of the `Option`.
            return Ok(None);
        };

        // A rewritten result needs a reason in the transcript. An extension that silently
        // changes what the model sees is the hardest kind of change to notice later, and
        // pi logs that it modified; here the notice goes where the user will actually look.
        if !reasons.is_empty() && result.details.is_none() {
            result.details = Some(serde_json::json!({
                "rewritten_by_extension": reasons.join("; ")
            }));
        }
        Ok(Some(result))
    }
}

/// Decode one content block from an extension's answer.
///
/// A block that does not fit is **dropped, not guessed at**. A rewrite is an assertion about
/// what the model will see, and an extension returning a malformed block has produced an
/// answer this harness cannot honour; silently coercing it would put text nobody wrote in
/// front of the model. Dropping it leaves the model's view slightly short rather than
/// wrong, and the remaining blocks still apply.
fn content_block_from_json(v: &serde_json::Value) -> Option<epsilon_contracts::ContentBlock> {
    use epsilon_contracts::ContentBlock;
    match v.get("type").and_then(|t| t.as_str()) {
        Some("text") => Some(ContentBlock::text(
            v.get("text").and_then(|t| t.as_str())?.to_string(),
        )),
        // Only the two block kinds the harness can represent without inventing semantics.
        // Image and tool-use blocks are dropped rather than approximated.
        _ => None,
    }
}

/// Decode a `Usage` patch from an extension's answer.
///
/// Returns `None` for anything malformed, which leaves the original usage in place. Usage
/// feeds the cost readout, so a wrong value here is worse than no value: a number the user
/// trusts and that is wrong is the exact failure the metrics work was meant to remove.
fn usage_from_json(v: &serde_json::Value) -> Option<epsilon_contracts::Usage> {
    serde_json::from_value(v.clone()).ok()
}

/// Turn-level hooks backed by extension processes (ADR-019, completing the seam).
///
/// `ToolHooks` could stop a call or rewrite its result. `TurnHooks` sits one level out: it
/// can reshape the context the model sees, swap the model or thinking level, and end the run
/// cleanly. Before this, every `set_turn_hooks` call in the repository was a test — the seam
/// had been built since M42 and never reached by anything a user could install.
///
/// ## Why this refuses to touch the context
///
/// `transform_context` and `convert_to_llm` return the messages the provider will receive.
/// Wiring an arbitrary child process into them is a different kind of decision than wiring one
/// into a veto: a wrong context is not a refused tool call, it is a corrupted conversation,
/// and the corruption is silent — the model simply behaves as if it had been told something
/// false. There is no error result to show the user and nothing to roll back.
///
/// So this type implements the two read-shaped methods as **pass-through**, and the
/// context-changing surface is reached only through `prepare_next_turn`, where the loop
/// already treats the result as optional and a broken host yields `None`. That is a real
/// capability gap against pi, recorded in `audit/02-port-fidelity.md`, and it is preferable
/// to a port that lets a third-party binary rewrite the prompt by accident.
///
/// The reasoning is not "extensions are untrusted" — the host is the user's own process and
/// pi deliberately allows this. It is that a *malformed or buggy* answer here is
/// unrecoverable in a way a malformed veto is not, and the harness has no way to tell the
/// difference.
pub struct ExtensionTurnHooks {
    hosts: Vec<Arc<ExtHost>>,
    /// Set when an extension asked to end the run; read by [`Self::should_stop_after_turn`].
    ///
    /// A cell rather than a field on the struct because the hook is shared behind an `Arc`
    /// and must stay `Sync`; `AtomicBool` is the honest primitive for one bit set from an
    /// async context and read by the loop.
    stop: std::sync::atomic::AtomicBool,
}

impl ExtensionTurnHooks {
    /// Build turn hooks over the currently attached extension hosts.
    pub fn new(hosts: Vec<Arc<ExtHost>>) -> Self {
        Self {
            hosts,
            stop: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// True when there is nothing to ask.
    pub fn is_empty(&self) -> bool {
        self.hosts.is_empty()
    }
}

#[async_trait::async_trait]
impl epsilon_contracts::turn::TurnHooks for ExtensionTurnHooks {
    /// **Pass-through by design.** See the type-level note: a child process does not get to
    /// rewrite the messages the provider receives.
    async fn transform_context(
        &self,
        messages: &[epsilon_contracts::message::Message],
    ) -> Vec<epsilon_contracts::message::Message> {
        messages.to_vec()
    }

    /// **Pass-through by design.** The built-in visibility filter lives in the default
    /// implementation, so the loop still gets exactly the messages it got before this hook
    /// existed — an extension must not be able to reveal a harness-only message to the model,
    /// which is a far quieter leak than a tool result.
    async fn convert_to_llm(
        &self,
        messages: &[epsilon_contracts::message::Message],
    ) -> Vec<epsilon_contracts::message::Message> {
        messages
            .iter()
            .filter(|m| m.is_llm_visible())
            .cloned()
            .collect()
    }

    /// Ask extensions whether the run should continue, and for a replacement context.
    ///
    /// A host that answers nothing, times out, or answers with something unreadable
    /// contributes nothing — the same failure policy as the tool hooks, for the same reason:
    /// a broken extension must not be able to end a run.
    async fn prepare_next_turn(
        &self,
        ctx: &epsilon_contracts::turn::NextTurnContext<'_>,
    ) -> Option<epsilon_contracts::turn::TurnUpdate> {
        if self.hosts.is_empty() {
            return None;
        }

        // pi's `turn_end` payload. `tool_results` is sent as the messages the turn produced,
        // which is what an extension needs to decide whether the work is actually finished.
        let event = serde_json::json!({
            "type": "turn_end",
            "message": ctx.message,
            "tool_results": ctx.tool_results,
            "new_messages": ctx.new_messages,
        });

        let mut update: Option<epsilon_contracts::turn::TurnUpdate> = None;

        for host in &self.hosts {
            for decision in host.interpose(&event).await {
                let Some(d) = decision.get("decision") else {
                    continue;
                };
                if !d.is_object() {
                    continue;
                }
                if d.get("stop").and_then(|s| s.as_bool()) == Some(true) {
                    self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
                }
                // A model swap. `TurnUpdate.model` is a catalog id, and the next provider
                // request is made with it directly -- so a typo here does not degrade the run,
                // it fails the request. That is accepted rather than filtered, and the reason
                // is worth stating: a silent "unknown model, keep going" would leave the
                // extension believing it had switched models while the run continued on the
                // old one, which is the harder failure to notice. A loud failure is correct.
                if let Some(model) = d.get("model").and_then(|m| m.as_str()) {
                    update.get_or_insert_with(Default::default).model =
                        Some(epsilon_contracts::turn::TurnModel {
                            id: model.to_string(),
                        });
                }
            }
        }

        update
    }

    /// Whether an extension asked the run to end.
    ///
    /// Read from the flag `prepare_next_turn` set rather than re-asking every host. The loop
    /// calls this once per turn, and a second round trip to every child would be a per-turn
    /// cost for an answer the last call already has.
    async fn should_stop_after_turn(
        &self,
        _ctx: &epsilon_contracts::turn::NextTurnContext<'_>,
    ) -> bool {
        self.stop.load(std::sync::atomic::Ordering::SeqCst)
    }
}

/// Install turn-level extension hooks on `agent`, if any extension is attached.
///
/// Separate from [`install_extension_hooks`] rather than bundled, because the two seams have
/// different risk profiles and a caller may legitimately want one without the other.
pub fn install_extension_turn_hooks(
    agent: &Arc<epsilon_core::Agent>,
    hosts: Vec<Arc<ExtHost>>,
) -> bool {
    let hooks = ExtensionTurnHooks::new(hosts);
    if hooks.is_empty() {
        return false;
    }
    agent.set_turn_hooks(Arc::new(hooks));
    true
}

/// Install extension interception on `agent`, if any extension is attached.
///
/// Returns whether hooks were installed, so the caller can say so in its diagnostics rather
/// than leaving the user to wonder why an extension could not block anything.
pub fn install_extension_hooks(agent: &Arc<epsilon_core::Agent>, hosts: Vec<Arc<ExtHost>>) -> bool {
    let hooks = ExtensionToolHooks::new(hosts);
    if hooks.is_empty() {
        return false;
    }
    agent.set_tool_hooks(Arc::new(hooks));
    true
}

/// Background pump: broadcast agent events into every attached host. Each host
/// evaluates its OWN manifest filter (plan 007 — the old hardcoded `true` made
/// forwarding dead code), so the pump stays dumb: forwarding failures never affect
/// the run.
pub fn spawn_event_forwarder(
    mut rx: tokio::sync::broadcast::Receiver<Arc<AgentEvent>>,
    hosts: Vec<Arc<ExtHost>>,
) {
    if hosts.is_empty() {
        return;
    }
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(ev) => {
                    let payload = serde_json::to_value(&*ev).unwrap_or(serde_json::Value::Null);
                    for host in &hosts {
                        if !host.is_disabled() {
                            host.send_event(&payload).await;
                        }
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

#[cfg(test)]
fn event_type_tag(ev: &AgentEvent) -> &'static str {
    // Serde tags from contracts (snake_case); used by tests to keep mapping honest.
    match ev {
        AgentEvent::AgentStart => "agent_start",
        AgentEvent::AgentEnd { .. } => "agent_end",
        AgentEvent::TurnStart { .. } => "turn_start",
        AgentEvent::TurnEnd { .. } => "turn_end",
        AgentEvent::MessageStart { .. } => "message_start",
        AgentEvent::MessageUpdate { .. } => "message_update",
        AgentEvent::MessageEnd { .. } => "message_end",
        AgentEvent::ToolExecutionStart { .. } => "tool_execution_start",
        AgentEvent::ToolExecutionUpdate { .. } => "tool_execution_update",
        AgentEvent::ToolExecutionEnd { .. } => "tool_execution_end",
        AgentEvent::PermissionRequested { .. } => "permission_requested",
        AgentEvent::PermissionDecided { .. } => "permission_decided",
        AgentEvent::CompactionStarted => "compaction_started",
        AgentEvent::CompactionFinished { .. } => "compaction_finished",
        AgentEvent::Notice { .. } => "notice",
    }
}

/// Newest `*.jsonl` session under `dir` (for `--continue`).
///
/// M41: pi's `findMostRecentSession` filters by session-cwd and SKIPS files
/// whose first line isn't a valid header (foreign `.jsonl` files share the
/// directory). The cwd filter only applies when the newest candidate carries
/// one — headerless v1 files have no cwd to compare, so they stay eligible
/// rather than vanishing from `--continue`.
pub fn latest_session_file(dir: &Path) -> Option<PathBuf> {
    latest_session_file_in(dir, None)
}

/// The session file a **new** launch writes: `.epsilon/sessions/--<cwd>--/<ISO>_<token>.jsonl`.
///
/// pi names one file per session and resolves it on create
/// (`references/pi-0.85.1/packages/agent/src/harness/session/jsonl/repo.ts:47-50`). This harness
/// wrote one *fixed* file per frontend — `tui.jsonl` — so three launches appended three
/// conversations to one lane, and `--continue` handed the model all three as a single context with
/// no boundary between them. The naming rule itself lives in [`epsilon_session::time`] (M209); this
/// is the call site that finally uses it.
///
/// **M217 added the per-cwd directory** (pi `repo.ts:42-45`), which is what makes "the newest
/// session *in this workspace*" answerable across workspaces rather than only inside one. Both
/// halves of the rule now live in [`epsilon_session::time::session_file_for`], which this calls, so
/// the writer cannot compose half of it — a launch writing a path no reader resolves is the quiet
/// failure this repository keeps finding.
///
/// `token` is a uniqueness component, not an identity claim: the store still mints the session's
/// own header id, so the id in the name and the id in the header are different strings today. It is
/// generated by [`epsilon_contracts::new_id`], which is monotonic, so two launches in the same
/// millisecond cannot land on one file — the failure mode a bare timestamp would have.
///
/// The result is **relative to the workspace**, which is how `RunParams::session_file` is
/// interpreted, with `/` separators on both platforms (`session_file_for` joins with `\` on
/// Windows, and a stored relative path is compared and printed as one string).
#[must_use]
pub fn fresh_session_file(now_ms: u64, token: &str, cwd: &Path) -> String {
    let rel = epsilon_session::time::session_file_for(
        &Path::new(".epsilon").join("sessions"),
        cwd,
        now_ms,
        token,
    );
    rel.to_string_lossy().replace('\\', "/")
}

/// The sessions directory for a workspace: `.epsilon/sessions/`, the root both layouts hang off.
///
/// Every reader takes this rather than composing `.epsilon/sessions` itself, so the root is one
/// string in the tree rather than four (M217).
#[must_use]
pub fn sessions_root(workspace: &Path) -> PathBuf {
    workspace.join(".epsilon").join("sessions")
}

/// Now, in milliseconds since the epoch — the clock [`fresh_session_file`] names a file with.
#[must_use]
pub fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Newest session under `dir` started from `cwd` (pi `findMostRecentSession`
/// cwd match). `None` cwd disables the filter (same as [`latest_session_file`]).
///
/// M57: the scan itself lives in [`epsilon_session::latest_session`] — the TUI's
/// `/sessions` browser lists the same directory, and two implementations of
/// "which files here are sessions" would drift.
pub fn latest_session_file_in(dir: &Path, cwd: Option<&Path>) -> Option<PathBuf> {
    epsilon_session::latest_session(dir, cwd)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovers_manifests_and_skips_broken_ones_with_diagnostics() {
        let tmp = tempfile::tempdir().expect("tmp");
        let root = tmp.path().join("extensions");

        // Valid extension directory.
        let good = root.join("good");
        std::fs::create_dir_all(&good).unwrap();
        std::fs::write(
            good.join("extension.json"),
            serde_json::json!({
                "name": "weather",
                "version": "1.0.0",
                "bin": "./run",
                "tools": [{"name":"lookup","description":"d","schema":{"type":"object"}}],
                "events": ["tool_execution_end"]
            })
            .to_string(),
        )
        .unwrap();

        // Broken manifest + a stray file that is not a directory.
        let bad = root.join("bad");
        std::fs::create_dir_all(&bad).unwrap();
        std::fs::write(bad.join("extension.json"), "{not json").unwrap();
        std::fs::write(root.join("stray.txt"), "x").unwrap();

        let (found, problems) = discover_extension_manifests(&[root]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].1.name, "weather");
        assert_eq!(
            problems.len(),
            1,
            "only the broken manifest reports: {problems:?}"
        );
    }

    #[test]
    fn duplicate_names_keep_first_and_report() {
        let tmp = tempfile::tempdir().expect("tmp");
        let mk = |root: &Path, name: &str| {
            let dir = root.join(name);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(
                dir.join("extension.json"),
                serde_json::json!({
                    "name": "dup", "version": "0.1.0", "bin": "./run"
                })
                .to_string(),
            )
            .unwrap();
            dir
        };
        let a = mk(tmp.path(), "a");
        let b_dir = tmp.path().join("b");
        std::fs::create_dir_all(&b_dir).unwrap();
        std::fs::write(
            b_dir.join("extension.json"),
            serde_json::json!({"name": "dup", "version": "0.2.0", "bin": "./run"}).to_string(),
        )
        .unwrap();

        let (found, problems) = discover_extension_manifests(&[a.parent().unwrap().to_path_buf()]);
        assert_eq!(found.len(), 1);
        // Exactly one problem, naming the directory that lost. Which directory
        // wins depends on read_dir order (alphabetical on NTFS, arbitrary on
        // ext4 — this failed on Linux CI where 'b' can precede 'a'), so assert
        // the contract "first seen wins" rather than a specific version.
        assert_eq!(problems.len(), 1);
        let winner_dir = found[0]
            .0
            .file_name()
            .and_then(|n| n.to_str())
            .expect("dir name");
        let loser = if winner_dir == "a" { "b" } else { "a" };
        assert!(
            problems[0].contains(&format!("at .*{loser}")) || problems[0].contains(loser),
            "problem should name the skipped dir {loser}: {:?}",
            problems[0]
        );
    }

    #[test]
    fn missing_roots_are_silent_noops() {
        let (found, problems) =
            discover_extension_manifests(&[std::env::temp_dir().join("definitely-missing-m11")]);
        assert!(found.is_empty());
        assert!(problems.is_empty());
    }

    #[test]
    fn event_tags_cover_every_variant() {
        // Exhaustive by construction: adding a new AgentEvent variant breaks this arm
        // set at compile time, keeping forwarded-tag mapping honest.
        let ev = AgentEvent::AgentStart;
        assert_eq!(event_type_tag(&ev), "agent_start");
    }

    #[test]
    fn latest_session_picks_newest_jsonl_only() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("sessions");
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(latest_session_file(&dir), None);

        // Discovery requires a valid header line (foreign jsonl is skipped).
        let header = |cwd: &str| {
            format!(
                "{{\"type\":\"session\",\"version\":3,\"id\":\"ses_x\",\"timestamp\":\"t\",\"cwd\":\"{cwd}\"}}\n"
            )
        };
        let old = dir.join("old.jsonl");
        let new = dir.join("new.jsonl");
        std::fs::write(&old, header("w")).unwrap();
        std::fs::write(dir.join("notes.txt"), "x").unwrap();
        std::fs::write(dir.join("foreign.jsonl"), "{}\n").unwrap();

        // Ensure distinct mtimes even on coarse filesystem clocks.
        std::thread::sleep(std::time::Duration::from_millis(50));
        std::fs::write(&new, header("w")).unwrap();

        assert_eq!(latest_session_file(&dir), Some(new.clone()));
        // …and the cwd filter keeps same-dir sessions, drops others.
        assert_eq!(
            latest_session_file_in(&dir, Some(Path::new("w"))),
            Some(new)
        );
        assert_eq!(
            latest_session_file_in(&dir, Some(Path::new("elsewhere"))),
            None
        );
        let _ = old;
    }

    #[test]
    fn trust_scan_names_every_resource_kind() {
        // Each pi resource kind has an Epsilon path that triggers it.
        //
        // Hermeticity note: the ancestor walk intentionally reaches outside
        // the tempdir (a parent repo's skill tree gates the project — pi
        // walks to root too), and this machine HAS ambient `.agents/skills`
        // above TMP (the repo itself). So the "quiet" assertions can't use a
        // bare tempdir: they use a workspace that pins down every `.agents`
        // lookup by planting its own `.agents/skills` FIRST… no — simpler:
        // they assert on the `.epsilon/*` resources only (no ancestor
        // dependence), and the `.agents` behavior gets its own isolated test
        // below where the tree is fully controlled.
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path().join("ws");
        std::fs::create_dir_all(&ws).unwrap();

        // Stray files never count.
        std::fs::create_dir_all(ws.join(".epsilon")).unwrap();
        std::fs::write(ws.join(".epsilon").join("x"), "").ok();
        std::fs::create_dir_all(ws.join(".epsilon").join("config-staging")).ok();
        let kinds = trust_requiring_resources(&ws);
        assert!(
            !kinds.contains(&TrustRequiringResource::ProjectConfig),
            "stray files must not count: {kinds:?}"
        );

        std::fs::write(ws.join(".epsilon/config.toml"), "[model]\nid = \"x\"\n").unwrap();
        std::fs::create_dir_all(ws.join(".epsilon/extensions")).unwrap();
        std::fs::create_dir_all(ws.join(".epsilon/skills")).unwrap();
        std::fs::create_dir_all(ws.join(".epsilon/prompts")).unwrap();
        std::fs::create_dir_all(ws.join(".epsilon/themes")).unwrap();
        std::fs::write(ws.join(".epsilon/SYSTEM.md"), "sys").unwrap();
        std::fs::write(ws.join(".epsilon/APPEND_SYSTEM.md"), "app").unwrap();
        let kinds = trust_requiring_resources(&ws);
        for expected in [
            TrustRequiringResource::ProjectConfig,
            TrustRequiringResource::Extensions,
            TrustRequiringResource::Skills,
            TrustRequiringResource::Prompts,
            TrustRequiringResource::Themes,
            TrustRequiringResource::SystemMd,
            TrustRequiringResource::AppendSystemMd,
        ] {
            assert!(
                kinds.contains(&expected),
                "missing {expected:?} in {kinds:?}"
            );
        }
        assert!(has_trust_requiring_resources(&ws));
    }

    #[test]
    fn every_system_md_the_trust_gate_names_is_actually_loaded() {
        // D-7 was a trust prompt with no effect: the two names were detected here and read
        // nowhere (`epsilon_core::prompt` had no slot for either). Detection lives in this
        // crate and loading in the next one, so the invariant has to be a test — otherwise
        // the next name added to one list silently recreates the complaint.
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path().join("ws");
        std::fs::create_dir_all(&ws).unwrap();
        assert!(
            !trust_requiring_resources(&ws).contains(&TrustRequiringResource::SystemMd),
            "an empty workspace asks for nothing"
        );
        assert_eq!(
            epsilon_core::prompt::load_system_overlay_in(&ws, true, None),
            epsilon_core::prompt::SystemOverlay::default()
        );

        std::fs::create_dir_all(ws.join(".epsilon")).unwrap();
        std::fs::write(ws.join(".epsilon/SYSTEM.md"), "epsilon-contract").unwrap();
        assert!(
            trust_requiring_resources(&ws).contains(&TrustRequiringResource::SystemMd),
            "the gate asks about it"
        );
        assert_eq!(
            epsilon_core::prompt::load_system_overlay_in(&ws, true, None)
                .custom
                .as_deref(),
            Some("epsilon-contract"),
            "and trusting the project loads it"
        );

        std::fs::write(ws.join(".epsilon/APPEND_SYSTEM.md"), "epsilon-append").unwrap();
        assert!(
            trust_requiring_resources(&ws).contains(&TrustRequiringResource::AppendSystemMd),
            "the gate asks about the append file too"
        );
        assert_eq!(
            epsilon_core::prompt::load_system_overlay_in(&ws, true, None)
                .append
                .as_deref(),
            Some("epsilon-append")
        );
    }

    #[test]
    fn agents_skills_counts_in_workspace_and_ancestors() {
        // `.agents/skills` in the workspace counts; an ancestor's counts too
        // (pi walks to root). Fully controlled tree: the ONLY `.agents`
        // above `child` is the planted one, so ambient machine state can't
        // leak in — except ancestors ABOVE the tempdir, which the walk still
        // reaches. Guard those: skip when an ambient tree exists (documents
        // the pi-parity walk instead of failing on dirty machines).
        let plain = tempfile::tempdir().unwrap();
        let top = plain.path().join("top");
        std::fs::create_dir_all(top.join(".agents").join("skills")).unwrap();
        assert!(trust_requiring_resources(&top).contains(&TrustRequiringResource::Skills));
        let child = top.join("sub").join("proj");
        std::fs::create_dir_all(&child).unwrap();
        assert!(trust_requiring_resources(&child).contains(&TrustRequiringResource::Skills));

        // A tree with no skills anywhere stays quiet — unless the ambient
        // machine has `.agents/skills` above TMP (like this repo does), in
        // which case pi-parity says the walk FINDS it. Both outcomes are
        // correct behavior; assert accordingly.
        let bare = tempfile::tempdir().unwrap();
        let bare_ws = bare.path().join("ws");
        std::fs::create_dir_all(&bare_ws).unwrap();
        let found = trust_requiring_resources(&bare_ws);
        let mut ambient = Some(bare.path());
        let mut ambient_has_skills = false;
        while let Some(d) = ambient {
            if d.join(".agents").join("skills").is_dir() {
                ambient_has_skills = true;
                break;
            }
            ambient = d.parent();
        }
        assert_eq!(
            found.contains(&TrustRequiringResource::Skills),
            ambient_has_skills,
            "walk must agree with the real ancestor chain: {found:?}"
        );
    }

    #[test]
    fn untrusted_roots_curtail_project_but_keep_global() {
        // Pi's "untrusted project ⇒ settings/extensions curtailed" scoping:
        // project-local roots drop out, the caller's global root survives.
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path();
        std::fs::create_dir_all(ws.join(".epsilon").join("skills")).unwrap();
        std::fs::create_dir_all(ws.join(".epsilon").join("extensions")).unwrap();
        std::fs::create_dir_all(ws.join(".agents").join("skills")).unwrap();

        let trusted_skills = skill_roots_trusted(ws, true);
        assert!(
            trusted_skills
                .iter()
                .any(|p| p.ends_with(".epsilon/skills")),
            "trusted keeps native root: {trusted_skills:?}"
        );
        assert!(
            trusted_skills
                .iter()
                .any(|p| p.ends_with("skills") && p.starts_with(ws)),
            "trusted keeps .agents root: {trusted_skills:?}"
        );
        let untrusted_skills = skill_roots_trusted(ws, false);
        assert!(
            untrusted_skills.iter().all(|p| !p.starts_with(ws)),
            "untrusted drops every workspace root: {untrusted_skills:?}"
        );

        let trusted_ext = extension_roots_trusted(ws, true);
        assert!(
            trusted_ext.iter().any(|p| p.starts_with(ws)),
            "trusted keeps project extensions: {trusted_ext:?}"
        );
        let untrusted_ext = extension_roots_trusted(ws, false);
        assert!(
            untrusted_ext.iter().all(|p| !p.starts_with(ws)),
            "untrusted drops project extensions: {untrusted_ext:?}"
        );

        // Back-compat shims stay fully trusted (all existing callers).
        assert_eq!(skill_roots(ws), skill_roots_trusted(ws, true));
        assert_eq!(extension_roots(ws), extension_roots_trusted(ws, true));
    }

    #[test]
    fn agent_roots_gate_project_on_trust() {
        // M50: pi's agentScope gate — user roots always, project roots only
        // when trusted (repo-controlled child prompts must not steer
        // delegation in untrusted checkouts).
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path();
        let user = agent_user_roots();
        assert!(
            user.iter().all(|p| !p.starts_with(ws)),
            "user roots never touch the workspace: {user:?}"
        );
        let trusted = agent_project_roots(ws, true);
        assert_eq!(trusted.len(), 2, "both project roots: {trusted:?}");
        assert!(
            trusted.iter().all(|p| p.starts_with(ws)),
            "project roots live under the workspace: {trusted:?}"
        );
        assert!(
            agent_project_roots(ws, false).is_empty(),
            "untrusted drops every project agent root"
        );
    }
}

#[cfg(test)]
mod ext_hooks_tests {
    use super::*;
    use epsilon_contracts::tool::{BeforeToolCallContext, BeforeToolCallResult, ToolHooks};

    /// A hook with no hosts must be free.
    ///
    /// The no-extension case is the common one, and the one where a per-call round trip
    /// would be pure cost. A harness that paid for this would be measurably slower for every
    /// user who has no extensions, in exchange for a feature none of them use.
    #[tokio::test]
    async fn no_extensions_means_no_interposition() {
        let hooks = ExtensionToolHooks::new(Vec::new());
        assert!(hooks.is_empty());
        let out = hooks
            .before_tool_call(ctx("write", serde_json::json!({"path": "a.txt"})))
            .await
            .expect("an empty host list is not an error");
        assert!(!out.block, "nothing asked, so nothing blocked");
    }

    /// Reduce a batch of raw extension answers to one verdict.
    ///
    /// Extracted as a free function so the arithmetic can be tested directly. This seam can
    /// do real damage in either direction -- a mistake either runs a tool that should have
    /// been stopped, or stops one that should have run -- and the only inputs are JSON, so
    /// the cases are enumerable without spawning a process.
    fn reduce(decisions: &[serde_json::Value]) -> BeforeToolCallResult {
        let mut out = BeforeToolCallResult::default();
        let mut reasons: Vec<String> = Vec::new();
        for d in decisions {
            let Some(dec) = d.get("decision") else {
                continue;
            };
            // Not an object: not a decision this harness knows how to read. Ignored rather
            // than guessed at -- see `a_non_object_decision_is_ignored...`.
            if !dec.is_object() {
                continue;
            }
            if dec.get("block").and_then(|b| b.as_bool()) == Some(true) {
                out.block = true;
                reasons.push(
                    dec.get("reason")
                        .and_then(|r| r.as_str())
                        .unwrap_or("blocked by an extension")
                        .to_string(),
                );
            }
        }
        if out.block {
            out.reason = Some(format!(
                "blocked by extension policy: {}",
                reasons.join("; ")
            ));
        }
        out
    }

    #[test]
    fn a_single_veto_blocks_and_is_attributed() {
        let out = reduce(&[serde_json::json!({
            "decision": {"block": true, "reason": "no writes outside src/"}
        })]);
        assert!(out.block, "one veto must block");
        let why = out
            .reason
            .expect("a block with no reason gives the user nowhere to go");
        assert!(
            why.contains("no writes outside src/"),
            "the reason must survive into the transcript: {why}"
        );
    }

    /// Any veto wins, whatever the order. Adding an extension may add a restriction; it must
    /// never remove one, so there is no majority and no first-wins.
    #[test]
    fn a_veto_wins_even_alongside_an_abstention() {
        let out = reduce(&[
            serde_json::json!({"decision": {"block": false}}),
            serde_json::json!({"decision": {"block": true, "reason": "denied"}}),
            serde_json::json!({"decision": {"block": false}}),
        ]);
        assert!(
            out.block,
            "a later veto must not be outvoted by earlier abstentions"
        );
        assert!(out.reason.unwrap().contains("denied"));
    }

    /// An abstention is silence: not consent, and not a veto.
    #[test]
    fn abstentions_alone_do_not_block() {
        let out = reduce(&[
            serde_json::json!({"decision": {"block": false}}),
            serde_json::json!({}),
            serde_json::json!({"decision": {}}),
        ]);
        assert!(!out.block, "no extension objected, so the call proceeds");
    }

    /// A malformed decision is not believed in either direction.
    ///
    /// Reading the string `"block"` as a block would let one buggy extension deny every tool
    /// call in the harness -- the most damaging misreading available at this seam, so it
    /// gets its own test rather than riding along with the others.
    #[test]
    fn a_non_object_decision_is_ignored_rather_than_read_as_a_veto() {
        for bad in [
            serde_json::json!({"decision": "block everything"}),
            serde_json::json!({"decision": true}),
            serde_json::json!({"decision": ["block"]}),
            serde_json::json!("block"),
        ] {
            let out = reduce(std::slice::from_ref(&bad));
            assert!(
                !out.block,
                "a non-object decision must not block: {bad} produced {:?}",
                out.reason
            );
        }
    }

    /// A veto carrying no reason still blocks, and still says something a user can act on.
    #[test]
    fn a_reasonless_veto_still_blocks_and_still_says_something() {
        let out = reduce(&[serde_json::json!({"decision": {"block": true}})]);
        assert!(out.block);
        let why = out.reason.expect("a block must always carry a reason");
        assert!(
            why.contains("extension"),
            "the fallback reason should still name the source: {why}"
        );
    }

    /// Every veto is named, not just the first.
    ///
    /// A user told "blocked" with one of two reasons has no way to know which rule fired, and
    /// disabling the wrong one is the obvious next mistake.
    #[test]
    fn multiple_vetoes_are_all_named() {
        let out = reduce(&[
            serde_json::json!({"decision": {"block": true, "reason": "no network"}}),
            serde_json::json!({"decision": {"block": true, "reason": "no writes"}}),
        ]);
        let why = out.reason.expect("blocked");
        assert!(
            why.contains("no network") && why.contains("no writes"),
            "{why}"
        );
    }

    fn ctx(tool: &str, args: serde_json::Value) -> BeforeToolCallContext {
        BeforeToolCallContext {
            // `ToolCallId` is a `String` alias (M42 recorded this as a gotcha).
            tool_call_id: "tc_1".to_string(),
            tool_name: tool.to_string(),
            args,
        }
    }

    /// The hook must be usable through `dyn ToolHooks`, which is how the agent stores it.
    ///
    /// A hook that only compiled when called concretely would pass every test in this module
    /// and fail at the one call site that matters.
    #[tokio::test]
    async fn the_hook_is_usable_as_a_trait_object() {
        let hooks: Arc<dyn ToolHooks> = Arc::new(ExtensionToolHooks::new(Vec::new()));
        let out = hooks
            .before_tool_call(ctx("bash", serde_json::json!({"command": "ls"})))
            .await
            .expect("empty hooks are permissive");
        assert!(!out.block);

        // `after_tool_call` returns `Option<...>`; extensions do not rewrite results yet, so
        // the inherited default must be a pure no-op. The agent calls it on every settled
        // tool call, so "it does not panic" is the property worth pinning.
        let after = hooks
            .after_tool_call(epsilon_contracts::tool::AfterToolCallContext {
                tool_call_id: "tc_1".to_string(),
                tool_name: "bash".to_string(),
                args: serde_json::json!({}),
                result: result_message(),
                is_error: false,
            })
            .await
            .expect("the default after-hook is a no-op");
        assert!(
            after.is_none(),
            "extensions do not rewrite results yet, so the default must be None"
        );
    }

    /// A settled tool result, for the after-hook call above. Built field-by-field to match
    /// the shape used elsewhere (`epsilon-core/tests/sequence_repair.rs:155`) rather than
    /// inventing a constructor the type does not have.
    fn result_message() -> epsilon_contracts::message::ToolResultMessage {
        epsilon_contracts::message::ToolResultMessage {
            id: epsilon_contracts::new_id("msg"),
            timestamp_ms: 0,
            tool_call_id: "tc_1".to_string(),
            tool_name: "bash".to_string(),
            content: vec![],
            details: serde_json::json!({}),
            usage: None,
            is_error: false,
        }
    }
}

#[cfg(test)]
mod ext_hooks_e2e {
    use super::*;
    use epsilon_contracts::tool::{BeforeToolCallContext, ToolHooks};
    use epsilon_ext::manifest::ExtManifest;
    use std::time::Duration;

    /// Locate the extension fixture binary.
    ///
    /// `CARGO_BIN_EXE_ext-fixture` is set only for `epsilon-ext`'s own integration tests,
    /// so it is not available here. The path is derived instead: the test binary lives in
    /// `<target>/debug/deps/`, and the fixture is dropped beside it in `<target>/debug/`.
    /// A missing fixture skips rather than fails, because its absence says nothing about
    /// the bridge -- it is a build-layout fact, and a test that cannot run must not be
    /// reported as a test that passed.
    pub(super) fn fixture_exe() -> Option<std::path::PathBuf> {
        let exe = std::env::current_exe().ok()?;
        let candidate = exe
            .parent()?
            .parent()?
            .join(format!("ext-fixture{}", std::env::consts::EXE_SUFFIX));
        candidate.exists().then_some(candidate)
    }

    /// Spawn the ext fixture, which answers `interpose` frames per its `--mode`.
    pub(super) async fn fixture(
        dir: &std::path::Path,
        mode: &str,
        events: &[&str],
    ) -> Option<Arc<ExtHost>> {
        let manifest = ExtManifest {
            name: "fixture".into(),
            version: "0.1.0".into(),
            bin: fixture_exe()?,
            args: vec![mode.to_string()],
            events: events.iter().map(|e| e.to_string()).collect(),
            tools: vec![],
        };
        ExtHost::spawn(dir, &manifest).await.ok()
    }

    fn ctx(tool: &str) -> BeforeToolCallContext {
        BeforeToolCallContext {
            tool_call_id: "tc_1".to_string(),
            tool_name: tool.to_string(),
            args: serde_json::json!({"command": "rm -rf /"}),
        }
    }

    /// The whole seam, end to end: a real child process, the real bridge, the real trait
    /// method — and a real veto reaching the caller.
    ///
    /// Every other test of this bridge either reduces hand-written JSON or runs against no
    /// host at all. Neither proves that the two halves are wired to each other: the hook
    /// could speak the wrong frame name, or send a payload the child never sees, and the
    /// reducer tests would still pass. This is the only test that would notice.
    #[tokio::test]
    async fn a_live_extension_can_veto_a_tool_call_through_the_bridge() {
        let dir = tempfile::tempdir().unwrap();
        let Some(host) = fixture(dir.path(), "--veto", &["before_tool_call"]).await else {
            eprintln!("skipping: ext-fixture was not built");
            return;
        };
        let hooks = ExtensionToolHooks::new(vec![host]);

        let out = hooks
            .before_tool_call(ctx("bash"))
            .await
            .expect("a live extension answering is not an error");

        assert!(
            out.block,
            "the extension said no and the call went ahead anyway"
        );
        let why = out.reason.expect("a block must say why");
        assert!(
            why.contains("extension says no"),
            "the child's own reason must reach the transcript: {why}"
        );
    }

    /// The negative case, live: a child that abstains must not block.
    ///
    /// Without this, a bridge that ignored the answer and blocked unconditionally would
    /// pass the veto test above.
    #[tokio::test]
    async fn a_live_extension_that_abstains_lets_the_call_proceed() {
        let dir = tempfile::tempdir().unwrap();
        let Some(host) = fixture(dir.path(), "--abstain", &["before_tool_call"]).await else {
            eprintln!("skipping: ext-fixture was not built");
            return;
        };
        let hooks = ExtensionToolHooks::new(vec![host]);

        let out = hooks
            .before_tool_call(ctx("ls"))
            .await
            .expect("an abstention is not an error");
        assert!(
            !out.block,
            "an extension with no opinion must not stop the call: {:?}",
            out.reason
        );
    }

    /// A child that goes silent must not stall the call, and must not merely not-block it.
    ///
    /// **This test found a real defect.** It was written expecting the failure policy alone
    /// to be enough, and it passed the "does not block" half while taking **ten seconds** —
    /// because `interpose` reused `INVOKE_TIMEOUT_SECS`. The policy was satisfied and the
    /// cost was not noticed: an interpose runs before *every* tool call, so a silent
    /// extension on a 30-call turn would add five minutes of pure waiting.
    ///
    /// "Must not block a run" and "must not delay a run" are different requirements, and only
    /// the first was written down. `INTERPOSE_TIMEOUT_SECS` now separates them, and the bound
    /// here is 3 s against a 1 s budget — wide enough not to be a timing race, tight enough
    /// to catch a regression to the invoke budget.
    #[tokio::test]
    async fn a_live_extension_that_never_answers_does_not_stall_the_call() {
        let dir = tempfile::tempdir().unwrap();
        let Some(host) = fixture(dir.path(), "--interpose-silent", &["before_tool_call"]).await
        else {
            eprintln!("skipping: ext-fixture was not built");
            return;
        };
        let hooks = ExtensionToolHooks::new(vec![host]);

        // The fixture ignores interpose frames entirely, so this must return quickly with no
        // decision rather than waiting out the ten-second invoke timeout.
        let out = tokio::time::timeout(Duration::from_secs(3), hooks.before_tool_call(ctx("ls")))
            .await
            .expect("a silent extension must not hold the tool call open");
        assert!(!out.expect("silence is not an error").block);
    }

    /// The manifest filter gates the interpose path too.
    ///
    /// Asking an extension that never declared interest would be a protocol round trip it
    /// did not consent to -- and would cost a tool call's latency for an answer nobody
    /// wanted.
    #[tokio::test]
    async fn an_extension_that_declared_no_interest_is_never_asked() {
        let dir = tempfile::tempdir().unwrap();
        // Declares no events, so `before_tool_call` is not one it wants to be consulted on.
        let Some(host) = fixture(dir.path(), "--veto", &[]).await else {
            eprintln!("skipping: ext-fixture was not built");
            return;
        };
        let hooks = ExtensionToolHooks::new(vec![host]);

        let out = hooks
            .before_tool_call(ctx("bash"))
            .await
            .expect("an uninterested host is not an error");
        assert!(
            !out.block,
            "a host that was never asked must not have blocked anything"
        );
    }
}

#[cfg(test)]
mod ext_hooks_after_tests {
    use super::*;
    use epsilon_contracts::ContentBlock;
    use epsilon_contracts::tool::{AfterToolCallContext, AfterToolCallResult, ToolHooks};

    /// Fold a batch of extension answers into the one optional patch the hook returns.
    ///
    /// Mirrors the body of `after_tool_call` with the host loop removed, so the folding
    /// rules can be tested as arithmetic. The alternative -- a live child per case -- would
    /// need a new fixture mode for every one of these, and would be slower without testing
    /// anything the reducer does not.
    fn fold(decisions: &[serde_json::Value]) -> Option<AfterToolCallResult> {
        let mut out: Option<AfterToolCallResult> = None;
        let mut reasons: Vec<String> = Vec::new();
        for d in decisions {
            let Some(dec) = d.get("decision") else {
                continue;
            };
            if !dec.is_object() {
                continue;
            }
            if let Some(content) = dec.get("content").and_then(|c| c.as_array()) {
                out.get_or_insert_with(Default::default).content =
                    Some(content.iter().filter_map(content_block_from_json).collect());
            }
            if let Some(details) = dec.get("details") {
                out.get_or_insert_with(Default::default).details = Some(details.clone());
            }
            if let Some(is_error) = dec.get("is_error").and_then(|e| e.as_bool()) {
                out.get_or_insert_with(Default::default).is_error = Some(is_error);
            }
            if let Some(usage) = dec.get("usage") {
                out.get_or_insert_with(Default::default).usage = usage_from_json(usage);
            }
            if let Some(terminate) = dec.get("terminate").and_then(|t| t.as_bool()) {
                out.get_or_insert_with(Default::default).terminate = Some(terminate);
            }
            if let Some(why) = dec.get("reason").and_then(|r| r.as_str()) {
                reasons.push(why.to_string());
            }
        }
        // Mirrors the impl: nothing modified means no patch at all.
        let mut result = out?;
        if !reasons.is_empty() && result.details.is_none() {
            result.details =
                Some(serde_json::json!({"rewritten_by_extension": reasons.join("; ")}));
        }
        Some(result)
    }

    /// An untouched result must come back as `None`.
    ///
    /// This is pi's `if (!modified) return undefined` (`runner.ts:30504`), and it is
    /// load-bearing rather than cosmetic: the loop uses `None` to keep its own message
    /// byte-for-byte, and `Some(empty)` would rebuild a message with no field changed --
    /// same contents, different provenance, and `usage` silently dropped by `unwrap_or`.
    #[test]
    fn no_opinion_means_no_patch_at_all() {
        assert!(
            fold(&[]).is_none(),
            "no extension answered, so there is no patch"
        );
        assert!(fold(&[serde_json::json!({})]).is_none());
        assert!(fold(&[serde_json::json!({"decision": {}})]).is_none());
    }

    /// A content replacement is what the whole feature is for: an extension redacting a
    /// file listing before the model sees it.
    #[test]
    fn content_is_replaced_wholesale() {
        let out = fold(&[serde_json::json!({
            "decision": {"content": [{"type": "text", "text": "redacted"}]}
        })])
        .expect("a content replacement is a modification");
        let content = out.content.expect("content was set");
        assert_eq!(content.len(), 1);
        assert_eq!(content[0], ContentBlock::text("redacted"));
    }

    /// Several text blocks survive in order.
    ///
    /// Order is the model's reading order, so a reordering is a semantic change even when
    /// the set is identical.
    #[test]
    fn multiple_blocks_keep_their_order() {
        let out = fold(&[serde_json::json!({
            "decision": {"content": [
                {"type": "text", "text": "first"},
                {"type": "text", "text": "second"},
            ]}
        })])
        .expect("modified");
        let content = out.content.expect("content was set");
        assert_eq!(content.len(), 2);
        assert_eq!(content[0], ContentBlock::text("first"));
        assert_eq!(content[1], ContentBlock::text("second"));
    }

    /// A block the harness cannot represent is dropped, never approximated.
    ///
    /// A malformed block is an answer this harness cannot honour. Coercing it would put
    /// text nobody wrote in front of the model; dropping it leaves the model's view short
    /// rather than wrong.
    #[test]
    fn an_unrepresentable_block_is_dropped_rather_than_invented() {
        let out = fold(&[serde_json::json!({
            "decision": {"content": [
                {"type": "text", "text": "kept"},
                {"type": "image", "data": "..."},
                {"no_type": true},
                {"type": "text"},
            ]}
        })])
        .expect("the text block is still a modification");
        let content = out.content.expect("content was set");
        assert_eq!(
            content,
            vec![ContentBlock::text("kept")],
            "only the well-formed text block should survive"
        );
    }

    /// The error flag can be cleared as well as set.
    ///
    /// A tool that failed for a reason an extension understands -- a retryable network
    /// error, say -- is the case this exists for, so `false` has to survive the round trip
    /// and not be treated as "no opinion".
    #[test]
    fn the_error_flag_can_be_cleared() {
        let out = fold(&[serde_json::json!({"decision": {"is_error": false}})])
            .expect("clearing the flag is a modification");
        assert_eq!(out.is_error, Some(false));
    }

    /// `details` carries the terminate hint, so it is read as a value and never assumed to
    /// be an object the harness can merge into.
    #[test]
    fn details_are_taken_verbatim() {
        let out = fold(&[serde_json::json!({
            "decision": {"details": {"truncated": true, "lines": 3}}
        })])
        .expect("details is a modification");
        assert_eq!(out.details.unwrap()["truncated"], serde_json::json!(true));
    }

    /// Usage feeds the cost readout, so a malformed patch is discarded rather than guessed.
    #[test]
    fn a_malformed_usage_patch_is_discarded() {
        let out = fold(&[serde_json::json!({
            "decision": {"usage": {"input_tokens": "lots"}}
        })])
        .expect("the decision still counts as a modification");
        assert!(
            out.usage.is_none(),
            "a usage value this harness cannot read must not reach the cost line"
        );
    }

    /// A silent rewrite is the hardest kind of change to notice later, so it leaves a note.
    #[test]
    fn a_silent_rewrite_records_that_it_happened() {
        let out = fold(&[serde_json::json!({
            "decision": {
                "content": [{"type": "text", "text": "quietly different"}],
                "reason": "stripped a secret"
            }
        })])
        .expect("modified");
        let details = out.details.expect("the reason must be recorded");
        assert!(
            details["rewritten_by_extension"]
                .as_str()
                .unwrap()
                .contains("stripped a secret"),
            "{details}"
        );
    }

    /// An extension that supplies its own `details` owns them.
    ///
    /// The harness's note is appended to what the extension said, never replacing it: a
    /// renderer consumes `details`, and overwriting one with provenance text would break
    /// the card the user is looking at.
    #[test]
    fn the_harness_note_does_not_overwrite_extension_details() {
        let out = fold(&[serde_json::json!({
            "decision": {
                "details": {"truncated": true},
                "content": [{"type": "text", "text": "x"}],
                "reason": "trimmed"
            }
        })])
        .expect("modified");
        let details = out.details.expect("details");
        assert_eq!(
            details,
            serde_json::json!({"truncated": true}),
            "the extension's details are the renderer's contract and must survive intact"
        );
    }

    fn ctx() -> AfterToolCallContext {
        AfterToolCallContext {
            tool_call_id: "tc_1".to_string(),
            tool_name: "read".to_string(),
            args: serde_json::json!({"path": "a.txt"}),
            result: epsilon_contracts::message::ToolResultMessage {
                id: epsilon_contracts::new_id("msg"),
                timestamp_ms: 0,
                tool_call_id: "tc_1".to_string(),
                tool_name: "read".to_string(),
                content: vec![ContentBlock::text("original")],
                details: serde_json::json!({}),
                usage: None,
                is_error: false,
            },
            is_error: false,
        }
    }

    /// With no hosts the hook is a no-op, and `None` is the correct no-op.
    #[tokio::test]
    async fn no_extensions_means_no_patch() {
        let hooks = ExtensionToolHooks::new(Vec::new());
        let out = hooks
            .after_tool_call(ctx())
            .await
            .expect("an empty host list is not an error");
        assert!(out.is_none(), "nothing asked, so nothing rewritten");
    }
}

#[cfg(test)]
mod ext_hooks_after_e2e {
    use super::ext_hooks_e2e::fixture;
    use super::*;
    use epsilon_contracts::tool::{AfterToolCallContext, ToolHooks};

    fn read_ctx() -> AfterToolCallContext {
        AfterToolCallContext {
            tool_call_id: "tc_1".to_string(),
            tool_name: "read".to_string(),
            args: serde_json::json!({"path": "secrets.env"}),
            result: epsilon_contracts::message::ToolResultMessage {
                id: epsilon_contracts::new_id("msg"),
                timestamp_ms: 0,
                tool_call_id: "tc_1".to_string(),
                tool_name: "read".to_string(),
                content: vec![epsilon_contracts::ContentBlock::text("API_KEY=hunter2")],
                details: serde_json::json!({"path": "secrets.env"}),
                usage: None,
                is_error: false,
            },
            is_error: false,
        }
    }

    /// A real child rewrites a real result, end to end.
    ///
    /// The reducer tests above cover the folding rules; none of them can tell whether the
    /// `tool_result` event ever reaches the child, whether the child keys its answer off
    /// the event type rather than arrival order, or whether the rewritten content survives
    /// the trip back. Those are wiring properties, and wiring is what the last two controls
    /// in this session proved reducers cannot check.
    #[tokio::test]
    async fn a_live_extension_can_rewrite_a_tool_result_through_the_bridge() {
        let dir = tempfile::tempdir().unwrap();
        let Some(host) = fixture(dir.path(), "--rewrite", &["tool_result"]).await else {
            eprintln!("skipping: ext-fixture was not built");
            return;
        };
        let hooks = ExtensionToolHooks::new(vec![host]);

        let out = hooks
            .after_tool_call(read_ctx())
            .await
            .expect("a live extension answering is not an error")
            .expect("the extension rewrote the content, so there must be a patch");

        let content = out.content.expect("content was replaced");
        assert_eq!(content.len(), 1);
        assert_eq!(
            content[0],
            epsilon_contracts::ContentBlock::text("[redacted by extension]"),
            "the extension's replacement must reach the caller verbatim"
        );
        assert!(
            !format!("{content:?}").contains("hunter2"),
            "the original secret must not survive a rewrite; got {content:?}"
        );
    }

    /// The same child, asked about a *before* call, must not rewrite anything.
    ///
    /// Both events travel the same channel in a real run, so a host that answered both
    /// alike would be applying a content patch to a call that has not run. This is the one
    /// test that distinguishes "keyed on the event type" from "answered whatever arrives".
    #[tokio::test]
    async fn a_rewrite_extension_does_not_disturb_the_before_hook() {
        let dir = tempfile::tempdir().unwrap();
        let Some(host) = fixture(dir.path(), "--rewrite", &["before_tool_call"]).await else {
            eprintln!("skipping: ext-fixture was not built");
            return;
        };
        let hooks = ExtensionToolHooks::new(vec![host]);

        let out = hooks
            .before_tool_call(epsilon_contracts::tool::BeforeToolCallContext {
                tool_call_id: "tc_1".to_string(),
                tool_name: "read".to_string(),
                args: serde_json::json!({"path": "secrets.env"}),
            })
            .await
            .expect("an abstention is not an error");
        assert!(
            !out.block,
            "an extension that only rewrites results must not block calls"
        );
    }
}

#[cfg(test)]
mod ext_turn_hooks_tests {
    use super::*;
    use epsilon_contracts::message::{Message, StopReason, UserMessage};
    use epsilon_contracts::turn::{NextTurnContext, TurnHooks, TurnUpdate};
    use epsilon_contracts::{ContentBlock, UserContent};

    fn user(text: &str) -> Message {
        Message::User(UserMessage {
            id: epsilon_contracts::new_id("msg"),
            timestamp_ms: 0,
            content: vec![UserContent::Text {
                text: text.to_string(),
            }],
        })
    }

    pub(super) fn assistant(text: &str) -> Message {
        Message::Assistant(epsilon_contracts::message::AssistantMessage {
            id: epsilon_contracts::new_id("msg"),
            timestamp_ms: 0,
            provider: "faux".to_string(),
            model_id: "faux-model".to_string(),
            content: vec![ContentBlock::text(text)],
            stop_reason: StopReason::EndTurn,
            error_message: None,
            usage: epsilon_contracts::Usage::default(),
        })
    }

    /// A harness-only message: `Message::Custom` is exactly the variant `is_llm_visible`
    /// rejects (`message.rs:273`), so this is the real shape rather than a stand-in.
    fn hidden() -> Message {
        Message::Custom(epsilon_contracts::message::CustomMessage {
            id: epsilon_contracts::new_id("msg"),
            timestamp_ms: 0,
            custom_type: "diagnostic".to_string(),
            payload: serde_json::json!({"note": "never send me to the provider"}),
        })
    }

    pub(super) fn ctx<'a>(
        message: &'a Message,
        results: &'a [Message],
        context: &'a [Message],
    ) -> NextTurnContext<'a> {
        NextTurnContext {
            message,
            tool_results: results,
            context,
            new_messages: context,
        }
    }

    /// The context must reach the provider exactly as it did before any extension existed.
    ///
    /// This is the property the whole type is built around, so it is the first test: a
    /// bridge that let a child process rewrite the prompt would be a silent corruption, with
    /// no error result and nothing to roll back.
    #[tokio::test]
    async fn transform_context_is_pass_through() {
        let hooks = ExtensionTurnHooks::new(Vec::new());
        let messages = vec![user("one"), assistant("two")];
        let out = hooks.transform_context(&messages).await;
        assert_eq!(
            out, messages,
            "an extension must not be able to rewrite what the provider receives"
        );
    }

    /// Harness-only messages stay hidden.
    ///
    /// The built-in visibility filter is the one thing a child process must never be able to
    /// switch off: revealing a harness message is a far quieter leak than a tool result, and
    /// the default implementation is where that filter lives.
    #[tokio::test]
    async fn convert_to_llm_still_hides_harness_messages() {
        let hooks = ExtensionTurnHooks::new(Vec::new());
        let visible = user("visible");
        let h = hidden();
        assert!(!h.is_llm_visible(), "the fixture must actually be hidden");
        let out = hooks.convert_to_llm(&[visible.clone(), h]).await;
        assert_eq!(
            out,
            vec![visible],
            "a harness-only message must not reach the provider"
        );
    }

    /// With no hosts the turn seam is inert.
    #[tokio::test]
    async fn no_extensions_means_no_turn_update_and_no_stop() {
        let hooks = ExtensionTurnHooks::new(Vec::new());
        assert!(hooks.is_empty());
        let m = assistant("done");
        let v = vec![m.clone()];
        let results: Vec<Message> = vec![];
        assert!(
            hooks
                .prepare_next_turn(&ctx(&m, &results, &v))
                .await
                .is_none(),
            "nothing asked, so no update"
        );
        assert!(!hooks.should_stop_after_turn(&ctx(&m, &results, &v)).await);
    }

    /// The decision reducer, tested directly.
    fn fold(decisions: &[serde_json::Value]) -> (Option<TurnUpdate>, bool) {
        let mut update: Option<TurnUpdate> = None;
        let mut stop = false;
        for d in decisions {
            let Some(dec) = d.get("decision") else {
                continue;
            };
            if !dec.is_object() {
                continue;
            }
            if dec.get("stop").and_then(|s| s.as_bool()) == Some(true) {
                stop = true;
            }
            if let Some(model) = dec.get("model").and_then(|m| m.as_str()) {
                update.get_or_insert_with(Default::default).model =
                    Some(epsilon_contracts::turn::TurnModel {
                        id: model.to_string(),
                    });
            }
        }
        (update, stop)
    }

    /// A stop request is honoured and remembered across the two calls the loop makes.
    ///
    /// The loop calls `prepare_next_turn` and then `should_stop_after_turn` as separate
    /// seams, so the flag has to survive between them. Asserting only on the first call would
    /// miss a bridge that answered the question and then forgot it.
    #[test]
    fn a_stop_request_is_carried_from_prepare_to_should_stop() {
        let (update, stop) = fold(&[serde_json::json!({"decision": {"stop": true}})]);
        assert!(stop, "an extension asked the run to end");
        assert!(update.is_none(), "a stop is not a context update");
    }

    /// A model swap is carried through with the id verbatim.
    ///
    /// No validation here on purpose: the next provider request uses the id directly, and a
    /// silent "unknown model, carry on" would leave the extension believing it had switched
    /// models while the run continued on the old one. A loud failure is the better wrong.
    #[test]
    fn a_model_swap_is_carried_verbatim() {
        let (update, _) = fold(&[serde_json::json!({"decision": {"model": "fast-model"}})]);
        let model = update
            .expect("a model swap is an update")
            .model
            .expect("model set");
        assert_eq!(model.id, "fast-model");
    }

    /// Silence, a malformed answer, and a non-object decision all mean "no opinion".
    ///
    /// A broken extension must not be able to end a run. That is the single most important
    /// rule on this seam, and it is why a stop is only ever set by an explicit `true`.
    #[test]
    fn only_an_explicit_true_can_end_a_run() {
        for bad in [
            serde_json::json!({}),
            serde_json::json!({"decision": {}}),
            serde_json::json!({"decision": {"stop": false}}),
            serde_json::json!({"decision": "stop"}),
            serde_json::json!({"decision": true}),
            serde_json::json!({"decision": {"stop": "yes"}}),
            serde_json::json!({"decision": {"stop": 1}}),
        ] {
            let (_, stop) = fold(std::slice::from_ref(&bad));
            assert!(!stop, "a broken answer must not end the run: {bad}");
        }
    }

    /// A stop survives a later extension's abstention.
    ///
    /// Extensions are consulted in order and the run's fate must not depend on which one
    /// loaded last.
    #[test]
    fn a_stop_is_not_overturned_by_a_later_abstention() {
        let (_, stop) = fold(&[
            serde_json::json!({"decision": {"stop": true}}),
            serde_json::json!({"decision": {"stop": false}}),
            serde_json::json!({}),
        ]);
        assert!(
            stop,
            "an abstention must not resurrect a run an extension ended"
        );
    }

    /// The hook must be usable through `dyn TurnHooks`, which is how the agent stores it.
    #[tokio::test]
    async fn the_hook_is_usable_as_a_trait_object() {
        let hooks: Arc<dyn TurnHooks> = Arc::new(ExtensionTurnHooks::new(Vec::new()));
        let m = assistant("done");
        let v = vec![m.clone()];
        let results: Vec<Message> = vec![];
        let messages = vec![user("hi")];

        assert_eq!(hooks.transform_context(&messages).await, messages);
        assert!(
            hooks
                .prepare_next_turn(&ctx(&m, &results, &v))
                .await
                .is_none()
        );
        assert!(!hooks.should_stop_after_turn(&ctx(&m, &results, &v)).await);
    }

    /// Installing with no hosts installs nothing.
    ///
    /// The seam is cheap to leave in place, but a hook object that exists and answers "no
    /// opinion" to every question is harder to reason about in a stack trace than one that
    /// was never installed, so the empty case stays distinguishable.
    #[test]
    fn an_empty_host_list_reports_nothing_installed() {
        // `install_extension_turn_hooks` needs an `Agent`, so the emptiness check that
        // actually gates installation is exercised here directly on the hook.
        assert!(ExtensionTurnHooks::new(Vec::new()).is_empty());
    }
}

#[cfg(test)]
mod ext_turn_hooks_e2e {
    use super::ext_hooks_e2e::fixture;
    use super::ext_turn_hooks_tests::{assistant, ctx};
    use super::*;
    use epsilon_contracts::message::Message;
    use epsilon_contracts::turn::TurnHooks;

    /// A live child ends the run, end to end.
    ///
    /// Every test above either folds hand-written JSON or runs against no host at all. None
    /// can tell whether the `turn_end` event ever reaches a child, whether the child keys its
    /// answer off the event type rather than arrival order, or whether the stop survives the
    /// gap between the two calls the loop makes. This is the only test that would notice, and
    /// the reducer-only pattern from the tool seams says exactly that it should exist.
    #[tokio::test]
    async fn a_live_extension_can_end_the_run_through_the_bridge() {
        let dir = tempfile::tempdir().unwrap();
        let Some(host) = fixture(dir.path(), "--turn", &["turn_end"]).await else {
            eprintln!("skipping: ext-fixture was not built");
            return;
        };
        let hooks = ExtensionTurnHooks::new(vec![host]);

        let m = assistant("turn complete");
        let v = vec![m.clone()];
        let results: Vec<Message> = vec![];

        hooks.prepare_next_turn(&ctx(&m, &results, &v)).await;
        assert!(
            hooks.should_stop_after_turn(&ctx(&m, &results, &v)).await,
            "the extension asked the run to end and the loop would not be told"
        );
    }

    /// The same child asked about a tool call must not end the run.
    ///
    /// A host that stopped the run on the wrong event would pass the test above while being
    /// unusable in practice -- every tool call would terminate the session.
    #[tokio::test]
    async fn a_turn_extension_does_not_disturb_the_tool_hooks() {
        let dir = tempfile::tempdir().unwrap();
        // Declares only the tool event, so the fixture never answers a turn_end.
        let Some(host) = fixture(dir.path(), "--turn", &["before_tool_call"]).await else {
            eprintln!("skipping: ext-fixture was not built");
            return;
        };
        let hooks = ExtensionTurnHooks::new(vec![host]);

        let m = assistant("turn complete");
        let v = vec![m.clone()];
        let results: Vec<Message> = vec![];
        assert!(
            !hooks.should_stop_after_turn(&ctx(&m, &results, &v)).await,
            "an extension that declared no turn interest must not end the run"
        );
    }

    /// A live child answering with rubbish must not end the run.
    ///
    /// The failure policy is the important part of this seam: a broken extension degrades to
    /// silence, never to a stop. Tested against a real process because the property is about
    /// what crosses the boundary, not about how the JSON is folded.
    #[tokio::test]
    async fn a_live_extension_answering_garbage_does_not_end_the_run() {
        let dir = tempfile::tempdir().unwrap();
        let Some(host) = fixture(dir.path(), "--turn-garbage", &["turn_end"]).await else {
            eprintln!("skipping: ext-fixture was not built");
            return;
        };
        let hooks = ExtensionTurnHooks::new(vec![host]);

        let m = assistant("turn complete");
        let v = vec![m.clone()];
        let results: Vec<Message> = vec![];
        hooks.prepare_next_turn(&ctx(&m, &results, &v)).await;
        assert!(
            !hooks.should_stop_after_turn(&ctx(&m, &results, &v)).await,
            "a non-object decision must not be read as a stop"
        );
    }

    /// A live child must not be able to rewrite the provider's messages.
    ///
    /// The counterpart to the two pass-through unit tests, and the one that matters most on
    /// this seam: a corrupted context is not a refused tool call, it is a conversation that
    /// quietly stops being true.
    #[tokio::test]
    async fn a_live_extension_cannot_rewrite_the_context() {
        let dir = tempfile::tempdir().unwrap();
        let Some(host) = fixture(dir.path(), "--turn", &["turn_end"]).await else {
            eprintln!("skipping: ext-fixture was not built");
            return;
        };
        let hooks = ExtensionTurnHooks::new(vec![host]);

        let m = assistant("turn complete");
        let messages = vec![m.clone()];
        assert_eq!(
            hooks.transform_context(&messages).await,
            messages,
            "the provider must see exactly the context it would have seen"
        );
    }
}

#[cfg(test)]
mod protocol_doc_tests {
    /// Read the protocol page.
    fn doc() -> String {
        std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/user/extensions.md"),
        )
        .expect("docs/user/extensions.md should exist; it is linked from docs/README.md")
    }

    /// The user-facing protocol page must not drift from the constants it documents.
    ///
    /// A protocol doc quoting a timeout the code no longer uses is worse than no doc: an
    /// author reads it, writes to it, and hits a failure they cannot explain. This reads the
    /// page and asserts the numbers in it are the ones in force, so a change to either side
    /// fails here rather than in someone else's extension.
    ///
    /// **It is asserted as a table row, not a bare substring.** The first version of this
    /// test searched for the digits alone and passed against a control that had changed the
    /// documented interpose budget from 1s to 3s — because the page mentions "1" and "2"
    /// dozens of times for unrelated reasons. A test that cannot fail is worse than no test,
    /// and this is the fifth time this session that has been true. Anchoring to the row is
    /// what makes the claim mean something.
    #[test]
    fn the_documented_limits_are_the_limits_in_force() {
        let doc = doc();
        // (constant, the exact cell the page must contain for it)
        // Rendered to `String` before the tuple: the constants are deliberately different
        // integer types, and this sidesteps that rather than casting one to the other.
        let expected: [(&str, String); 3] = [
            (
                "interpose",
                format!("**{} second**", epsilon_ext::INTERPOSE_TIMEOUT_SECS),
            ),
            (
                "invoke",
                format!("**{} seconds**", epsilon_ext::INVOKE_TIMEOUT_SECS),
            ),
            ("crash", format!("**{}**", epsilon_ext::CRASH_BUDGET)),
        ];
        for (label, needle) in expected {
            assert!(
                doc.contains(&needle),
                "docs/user/extensions.md does not contain `{needle}` for the {label} limit; \
                 the constant or the doc changed and one of them is now wrong"
            );
        }
    }

    /// Every event tag the bridge sends must appear as a word in the protocol page.
    ///
    /// The tags are the contract's vocabulary: an author keys off them, and a tag living
    /// only in `runtime.rs` is one nobody outside this repository can discover.
    ///
    /// Matched with word boundaries on purpose. `turn_end` is a substring of `turn_ending`,
    /// so a plain `contains` passed a control that had renamed the tag everywhere — the
    /// sixth near-miss of this kind, and the reason the previous two checks are anchored.
    #[test]
    fn every_event_tag_the_bridge_sends_is_documented() {
        let doc = doc();
        for tag in [
            "before_tool_call",
            "tool_result",
            "turn_end",
            "interpose_result",
        ] {
            // A tag must appear as a standalone token, not as a prefix of a longer word.
            let hit = doc
                .split(|c: char| !c.is_alphanumeric() && c != '_')
                .any(|word| word == tag);
            assert!(
                hit,
                "the protocol page never names `{tag}` as a standalone token; a third \
                 party cannot discover a tag that exists only in the source"
            );
        }
    }

    /// `docs/history.md` was corrupt for twenty-plus milestones (D23): 18 consecutive
    /// lines carried a spurious `M161`–`M178` prefix glued onto whatever followed, which
    /// destroyed three rows and a section heading, and the 18 milestone rows themselves were
    /// lost entirely. The repair was manual; this is the check that stops it recurring.
    ///
    /// Every assertion here is a property of the FILE's structure, not of what it says, so it
    /// cannot be satisfied by editing prose. `D23` is closed in `f6636ff`.
    #[test]
    fn the_history_timeline_is_structurally_sound() {
        let doc = include_str!("../../../docs/history.md");

        // A glued line: a bare milestone number at the start of a line, which is how the
        // corruption presented (`M163| M144 | ...`).
        let glued: Vec<&str> = doc
            .lines()
            .filter(|l| {
                let t = l.trim_start();
                t.starts_with('M') && t[1..].chars().next().is_some_and(|c| c.is_ascii_digit())
            })
            .collect();
        assert!(
            glued.is_empty(),
            "docs/history.md has bare milestone numbers at line start (the D23 signature): {glued:?}"
        );

        // Every numbered row is well-formed: it ends with the closing pipe.
        let unclosed: Vec<&str> = doc
            .lines()
            .filter(|l| l.starts_with("| M") && !l.trim_end().ends_with('|'))
            .collect();
        assert!(
            unclosed.is_empty(),
            "docs/history.md rows missing their closing pipe: {:?}",
            unclosed
                .iter()
                .map(|l| &l[..l.len().min(60)])
                .collect::<Vec<_>>()
        );

        // Milestone numbers ascend WITHIN each table, which catches a row inserted in the
        // wrong place -- the symptom of the displaced M135/M144 rows.
        //
        // Scoped per table, not across the file: `## Original acceptance criteria` is a
        // second table that starts again at M6, so a whole-file ascending check reports
        // "M6 after M186" and is wrong. This test did exactly that on its first run, which
        // is the same scoping mistake the missing-milestones test documents -- caught here
        // from the other side, by a check that was too broad instead of too narrow.
        let mut prev: Option<u32> = None;
        for line in doc.lines() {
            if line.starts_with('#') {
                // A heading starts a new table, and the sequence restarts with it.
                prev = None;
                continue;
            }
            let Some(rest) = line.strip_prefix("| M") else {
                continue;
            };
            let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            let Ok(n) = digits.parse::<u32>() else {
                continue;
            };
            if let Some(p) = prev
                && n < p
            {
                panic!("docs/history.md goes backwards within one table: M{n} after M{p}");
            }
            prev = Some(n);
        }
    }

    /// Every milestone up to the newest has a row, in one table or the other.
    ///
    /// Scoped across BOTH tables on purpose. An earlier check of mine sliced only the
    /// timeline section, reported 79 missing milestones, and I started reconstructing them —
    /// until M96 turned up at line 143 under `## Original acceptance criteria`, a second
    /// table by design. The missing list was an artefact of the slice. Checking across both
    /// answers the question that was actually being asked.
    /// The two history tests above are the model, not an exception: a doc
    /// convention enforced only by review drifts, and M196 showed the gap sitting
    /// undetected for two milestones.
    ///
    /// **M196, added after a real incident.** Reworking one test in
    /// `epsilon-tui/src/app/tests.rs` by LINE RANGE -- with `cargo fmt` having
    /// reformatted the region between the read and the write -- silently deleted
    /// **five** neighbouring tests, including the tree-picker and label-editor
    /// ones. The battery returned `GATE PASS` at 1,516 tests: a deleted test is
    /// not a failing test, and a whole category of behaviour lost its only
    /// coverage with nothing reporting it.
    ///
    /// The count moved from 1,520 to 1,516, and that comparison -- a human reading
    /// two logs -- was the entire detection mechanism. This makes it mechanical.
    #[test]
    fn no_source_file_lost_a_test_since_head() {
        // Compared against HEAD, so it fires the moment a working-tree edit drops
        // a test. Once a deletion is COMMITTED there is nothing left to catch it,
        // which is why this belongs in the loop before `git commit` rather than
        // in the suite.
        // **Run git from the REPO ROOT, not the cwd.** Cargo sets a test's cwd
        // to the crate directory, and from there `git diff -- crates/` matches
        // nothing: the pathspec is relative, `crates/` is not under
        // `crates/epsilon-cli/`, and the command exits 0 having printed nothing.
        // The check then passes vacuously -- which is exactly what happened the
        // first two times this test was written, and why both controls "passed".
        // `-C <root>` plus `--relative` is what makes it cwd-independent.
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(std::path::Path::parent)
            .map(std::path::Path::to_path_buf);
        let Ok(out) = std::process::Command::new("git")
            .args(["-C"])
            .arg(root.as_deref().unwrap_or(std::path::Path::new(".")))
            .args(["diff", "--unified=0", "HEAD", "--", "crates/"])
            .output()
        else {
            // No git (a source tarball, a vendored copy): the check does not
            // apply, and failing here would break the build for no reason.
            return;
        };
        if !out.status.success() {
            return;
        }
        let text = String::from_utf8_lossy(&out.stdout);

        // If the working tree really is clean there is nothing to check and
        // returning is correct. But a check that silently stops inspecting is
        // indistinguishable from one that finds nothing wrong, so the pathspec is
        // verified to actually select files when any exist.
        if !text.contains("diff --git") {
            let dirty = std::process::Command::new("git")
                .args(["-C"])
                .arg(root.as_deref().unwrap_or(std::path::Path::new(".")))
                .args(["status", "--porcelain", "--", "crates/"])
                .output()
                .ok()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .unwrap_or_default();
            assert!(
                dirty.is_empty(),
                "the working tree has changes under crates/ but `git diff -- crates/` \
                 selected none of them -- the pathspec is wrong and this check is not \
                 running. Output was: {dirty:?}"
            );
            return;
        }

        // **By NAME, not by count.** The first version compared the number of
        // removed `#[test]` attributes against the number added, and its control
        // PASSED while a real test was being deleted -- because the same diff also
        // added two new tests. A net count cannot see one name vanishing among
        // three arriving. The names are what the coverage is keyed on, so the
        // names are what this compares.
        //
        // **And only test functions, M198.** This collected every `fn` in the
        // diff, so deleting a dead helper read exactly like deleting a test. M198
        // removed four pieces of dead code -- `catalog::CustomProvider` and its
        // three methods, `llm::http::status_error`,
        // `tools::files::_path_witness`, `mcp::invalidate_health` -- and the
        // guard failed on all of them: the "a guard that cries wolf is a guard
        // that gets disabled" outcome its own comment warns about, arriving on
        // schedule.
        //
        // A test function is one carried by `#[test]`, `#[tokio::test]` or
        // `#[bench]`. That attribute sits in the hunk *above* the signature, so
        // the scan looks back over the contiguous attribute block; `-U0` keeps
        // hunks tight enough that the block is not padded with unrelated lines.
        let collect = |sign: &str| -> std::collections::BTreeSet<String> {
            let mut out = std::collections::BTreeSet::new();
            let lines: Vec<&str> = text.lines().collect();
            for (index, line) in lines.iter().enumerate() {
                let Some(rest) = line.strip_prefix(sign) else {
                    continue;
                };
                let mut rest = rest.trim_start();
                if let Some(r) = rest.strip_prefix("async ") {
                    rest = r.trim_start();
                }
                // `pub fn` counts too. Making a function public rewrites its line
                // from `-fn name(` to `+pub fn name(`, which a `fn`-only match
                // reads as a DELETION with no replacement -- a false positive, and
                // a guard that cries wolf is a guard that gets disabled.
                //
                // **One strip, not two.** `"pub fn "` already contains `"fn "`,
                // so stripping the visibility leaves `name(` with no `fn` left
                // to strip -- which is why the first attempt at this fix still
                // reported `fmt_tokens` as lost. Strip the VISIBILITY only, and
                // then remove a leading `fn ` if one is still there:
                //   `fn name(`        -> no visibility, `fn ` removed
                //   `pub fn name(`    -> visibility removed, no `fn ` left
                //   `pub(crate) fn name(` -> same
                let mut tail = None;
                for vis in ["pub(crate) fn ", "pub fn ", "fn "] {
                    if let Some(r) = rest.strip_prefix(vis) {
                        tail = Some(r);
                        break;
                    }
                }
                let Some(tail) = tail else {
                    continue;
                };
                let name = tail.split('(').next().unwrap_or(tail).trim();
                if name.is_empty() {
                    continue;
                }
                // Look back over the attribute block. The diff sign is stripped
                // first, because in a diff an attribute and the item it decorates
                // BOTH carry the `+`/`-` prefix: `trim` alone leaves `-#[test]`,
                // which does not start with `#`, and the classifier then sees a
                // test as a plain function. (The control below is what caught
                // that -- the guard itself would have quietly stopped reporting
                // deleted tests, which is the exact failure it exists to prevent.)
                let is_test = lines[..index]
                    .iter()
                    .rev()
                    .map(|l| l.trim().trim_start_matches(['+', '-', ' ']))
                    .take_while(|t| t.starts_with('#') || t.starts_with("//") || t.is_empty())
                    .any(|t| {
                        t.starts_with("#[test")
                            || t.starts_with("#[tokio::test")
                            || t.starts_with("#[rstest")
                            || t.starts_with("#[bench")
                    });
                if is_test {
                    out.insert(name.to_string());
                }
            }
            out
        };
        let removed = collect("-");
        let added = collect("+");

        let lost: Vec<&String> = removed.difference(&added).collect();
        assert!(
            lost.is_empty(),
            "test(s) deleted from crates/ with nothing replacing them: {lost:?}. A deleted \
             test is not a failing test, so the suite stays GREEN while the coverage \
             disappears -- which is how five tests were lost at M196. Revert the file \
             (`git checkout -- <file>`) and re-apply the change by test NAME, never by \
             line range: `cargo fmt` rewrites the region between your read and your \
             write, so a line range silently takes its neighbours with it."
        );
    }

    /// The M198 narrowing above is a change to a guard, so it gets a control —
    /// the repository's rule is that a guard which cannot be shown to fail has
    /// not been shown to work.
    ///
    /// The narrow risk is specific: a check that only looks at `#[test]`-carried
    /// lines could stop seeing a deleted test whose attribute the diff moved or
    /// dropped. So the control asserts the *classifier*, not the guard — the
    /// classifier is the part that changed, and it can be exercised directly on
    /// the diff text the guard would be handed, with no repository mutation at
    /// all.
    ///
    /// Two shapes, because they differ in exactly the way that matters: the
    /// attribute inside the same hunk, and the attribute separated from the
    /// signature by intervening lines (which is what `cargo fmt` produces when it
    /// reflows a signature across several lines).
    #[test]
    fn the_test_classifier_reads_an_attribute_from_the_hunk_above() {
        /// The same predicate the guard uses, extracted so it can be checked
        /// without touching the working tree.
        fn names_in(diff: &str, sign: &str) -> std::collections::BTreeSet<String> {
            let mut out = std::collections::BTreeSet::new();
            let lines: Vec<&str> = diff.lines().collect();
            for (index, line) in lines.iter().enumerate() {
                let Some(rest) = line.strip_prefix(sign) else {
                    continue;
                };
                let mut rest = rest.trim_start();
                if let Some(r) = rest.strip_prefix("async ") {
                    rest = r.trim_start();
                }
                let mut tail = None;
                for vis in ["pub(crate) fn ", "pub fn ", "fn "] {
                    if let Some(r) = rest.strip_prefix(vis) {
                        tail = Some(r);
                        break;
                    }
                }
                let Some(tail) = tail else { continue };
                let name = tail.split('(').next().unwrap_or(tail).trim();
                if name.is_empty() {
                    continue;
                }
                // Look back over the attribute block. The same diff sign is
                // stripped first, because in a diff an attribute and the item
                // it decorates both carry the `+`/`-` prefix -- `trim` alone
                // leaves `-#[test]`, which does not start with `#`.
                let is_test = lines[..index]
                    .iter()
                    .rev()
                    .map(|l| l.trim().trim_start_matches(['+', '-', ' ']))
                    .take_while(|t| t.starts_with('#') || t.starts_with("//") || t.is_empty())
                    .any(|t| {
                        t.starts_with("#[test")
                            || t.starts_with("#[tokio::test")
                            || t.starts_with("#[rstest")
                            || t.starts_with("#[bench")
                    });
                if is_test {
                    out.insert(name.to_string());
                }
            }
            out
        }

        let diff = "\
@@ -1,3 +1,0 @@
-#[test]
-fn a_real_test() {
@@ -9,2 +6,0 @@
-#[tokio::test]
-async fn an_async_test() {
@@ -20,2 +17,0 @@
-fn a_dead_helper() {
@@ -30,2 +27,0 @@
-/// Doc comment.
-pub fn a_dead_public_helper() {
@@ -40,2 +37,2 @@
-#[test]
-fn renamed_test() {
+#[test]
+fn renamed_test_v2() {
";
        let removed = names_in(diff, "-");
        let added = names_in(diff, "+");

        assert!(removed.contains("a_real_test"), "{removed:?}");
        assert!(removed.contains("an_async_test"), "{removed:?}");
        // The whole point of the narrowing: these are NOT tests and must not be
        // reported as lost coverage.
        assert!(!removed.contains("a_dead_helper"), "{removed:?}");
        assert!(!removed.contains("a_dead_public_helper"), "{removed:?}");

        // A **rename** is the interesting case, and the assertion is the
        // opposite of the one this test started with.
        //
        // Renaming a test is a removal of one name and the addition of another,
        // so `removed.difference(&added)` DOES contain the old name -- and the
        // guard reports it. That is **correct and worth keeping**: a renamed test
        // has a new name, and anything referring to the old one (a milestone doc
        // citing it, a `cargo test <name>` in someone's muscle memory) is now
        // wrong. The guard cannot know the rename was deliberate.
        //
        // So the first version of this assertion -- "a rename's net effect must be
        // nil" -- was asserting that the guard is blind to renames. It failed, and
        // it should have: the property it named is a defect, not a requirement.
        assert!(
            removed.difference(&added).any(|n| n == "renamed_test"),
            "a renamed test must still be reported: its old name is gone"
        );
        assert!(added.contains("renamed_test_v2"), "{added:?}");
    }

    #[test]
    fn every_milestone_has_a_row_somewhere_in_the_history() {
        let doc = include_str!("../../../docs/history.md");
        let present: std::collections::BTreeSet<u32> = doc
            .lines()
            .filter_map(|l| l.strip_prefix("| M"))
            .filter_map(|rest| {
                let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
                digits.parse().ok()
            })
            .collect();

        let newest = *present.iter().max().expect("at least one milestone row");
        let missing: Vec<u32> = (1..=newest).filter(|n| !present.contains(n)).collect();
        assert!(
            missing.is_empty(),
            "docs/history.md has no row for milestones {missing:?} (newest is M{newest})"
        );
    }

    /// No production spawn may leave `stdin` unset.
    ///
    /// **This exists because fixing the bug once was not enough.** The Windows
    /// stdin-inheritance defect (M197) was found, fixed on the unconfined shell
    /// path, given a test, and shipped. An independent review then found the
    /// *same* omission on the two confined paths — which are the default — plus
    /// the MCP stdio server. Four sites, one fix applied, three missed, and a
    /// green battery after each step: nothing in the suite could see a site that
    /// was simply not looked at.
    ///
    /// So the property is now mechanical. Every `Command::new` under
    /// `crates/*/src` is found, and each is required to configure `stdin`
    /// explicitly within a small window after the spawn. `Stdio::null()` for a
    /// command, `Stdio::piped()` for a protocol peer — either is fine, because
    /// **the point is that the choice is written down**. Silence means INHERIT,
    /// and in the TUI the inherited handle is the console the user is typing
    /// into.
    ///
    /// Evidence: `audit/evidence/m197-windows-stdin-inheritance.md`.
    #[test]
    fn no_production_spawn_leaves_stdin_unset() {
        let Some(root) = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(std::path::Path::parent)
            .map(std::path::Path::to_path_buf)
        else {
            return;
        };
        let crates = root.join("crates");

        // Collect `.rs` files by walking, rather than trusting a hand-kept list
        // that would itself rot the way a hand-kept pathspec did.
        fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return;
            };
            for e in entries.flatten() {
                let p = e.path();
                if p.is_dir() {
                    walk(&p, out);
                } else if p.extension().is_some_and(|x| x == "rs") {
                    // `src/bin` holds fixture binaries, not production paths.
                    if !p.components().any(|c| c.as_os_str() == "tests") {
                        out.push(p);
                    }
                }
            }
        }
        let mut files = Vec::new();
        walk(&crates, &mut files);
        assert!(
            !files.is_empty(),
            "found no sources to check under {crates:?}"
        );

        let mut offenders: Vec<String> = Vec::new();
        for f in &files {
            let Ok(text) = std::fs::read_to_string(f) else {
                continue;
            };
            // Test code starts at the first *module-level* test marker. A
            // `#[cfg(test)]` on a single helper `fn` is not one — `jobobject.rs`
            // has two, at lines 53 and 163, long before its `mod tests`, and
            // treating either as the boundary would count the whole test module
            // as production. Only markers that introduce a module count.
            let test_start = text
                .lines()
                .enumerate()
                .filter(|(_, l)| {
                    let t = l.trim();
                    t.starts_with("mod tests")
                        || t.starts_with("#[cfg(test)]") && t.contains("mod ")
                })
                .map(|(n, _)| n)
                .min()
                .unwrap_or(usize::MAX);
            for (i, line) in text.lines().enumerate() {
                // The guard must not match itself. Three ways its own source can
                // contain the token: the comparison, a doc comment naming it, and
                // the pattern string. A comment line cannot be a call.
                if !line.contains("Command::new(") || line.contains("contains(") {
                    continue;
                }
                let trimmed = line.trim_start();
                if trimmed.starts_with("//") || trimmed.starts_with("///") {
                    continue;
                }
                if i >= test_start {
                    continue;
                }
                // Look a few lines ahead: the configuration follows the spawn.
                // The window is generous because the configuration is not
                // necessarily adjacent — `editor.rs` sets all three streams twelve
                // lines below the `Command::new`, and a 6-line window called that
                // a violation.
                let window: String = text.lines().skip(i).take(16).collect::<Vec<_>>().join("\n");
                // An explicit `Stdio::inherit()` is a *decision*, not an omission.
                // The external editor is the one legitimate case: it is the user's
                // terminal, and it must see the real console. What the rule forbids
                // is the silence that means "nobody thought about it".
                //
                // `.output()` is the other exemption: it configures all three
                // streams to pipes internally, so a `gh gist` call written that
                // way never inherits anything, and flagging it would be noise
                // that trains a reader to skip the message.
                if window.contains("stdin(")
                    || window.contains("Stdio::inherit()")
                    || window.contains(".output()")
                    // Delegation is a legitimate way to configure: `shell.rs`
                    // calls `ChildStdio::configure(&mut cmd)`, which is where
                    // the decision now lives. Forbidding that would push the
                    // three lines back inline and undo the refactor.
                    || window.contains("configure(")
                {
                    continue;
                }
                offenders.push(format!(
                    "{}:{}: {}",
                    f.strip_prefix(&root).unwrap_or(f).display(),
                    i + 1,
                    line.trim()
                ));
            }
        }
        assert!(
            offenders.is_empty(),
            "these spawns never set stdin, so the child INHERITS the harness's — \
             which in the TUI is the console the user is typing into. Add \
             `.stdin(Stdio::null())` for a command or `.stdin(Stdio::piped())` \
             for a protocol peer:\n{}",
            offenders.join("\n")
        );
    }

    /// A panel that returns early must not silently swallow a bound chord.
    ///
    /// **The shape of the defect.** `keys.rs` is one long arm list, and a panel
    /// that is open claims it by matching on `KeyCode` and then `return false` —
    /// which is correct for the keys that panel wants, and *silently wrong* for
    /// every key it does not. `app.message.copy` was bound (F4) and had a working
    /// global arm 620 lines below the tree block, and pressing it with `/tree` open
    /// did nothing at all: no notice, no clipboard, no state change. Two keystrokes
    /// from a defect that a green battery and `/hotkeys` both endorsed.
    ///
    /// **What this asserts.** Not "the tree honours copy" — one binding, one
    /// block, and the next panel has the same shape. It asserts the *invariant*
    /// that made the bug possible: for every panel block that returns early, the
    /// set of binding ids it claims is recorded here, and a binding that the
    /// palette advertises must be either claimed by that block or explicitly
    /// listed as deliberately panel-scoped.
    ///
    /// The test is a cross-reference, so it can only be as good as the list. The
    /// list is short and each row says why, which is what makes it reviewable —
    /// and a row that goes stale fails loudly rather than quietly.
    #[test]
    fn a_panel_that_swallows_keys_declares_which_ones() {
        // Ids each early-returning panel handles itself, with the reason it is
        // allowed to keep the rest. Anything a panel claims MUST appear in the
        // handler's own `match`; anything it does not claim must be here.
        //
        // The rows are `(panel, binding ids the panel handles)`. A binding id in
        // this table is one the panel deliberately owns; the test's job is to
        // catch a panel that grows a catch-all without saying so.
        const PANELS: &[(&str, &[&str])] = &[
            // Enter/Esc are the panel's own vocabulary; everything else is
            // handled by the arms below the block.
            ("auth_prompt_open", &[]),
            ("auth_pick_open", &[]),
            ("label_prompt_open", &[]),
            ("timeline_open", &["app.message.copy"]),
            ("palette_open", &[]),
            ("picker_open", &[]),
        ];

        let keys = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../epsilon-tui/src/app/keys.rs"),
        )
        .expect("the TUI key handler is the thing being checked");

        let mut problems = Vec::new();
        for (panel, _claimed) in PANELS {
            let Some(open) = keys.find(&format!("if app.{panel} {{")) else {
                problems.push(format!(
                    "{panel}: no such block in keys.rs — the table is stale"
                ));
                continue;
            };
            // The block ends at the first `return false;` at one indent level, which
            // is how every one of these blocks is written. A block that never
            // returns is reported rather than skipped, because that is a different
            // (and equally surprising) shape.
            let rest = &keys[open..];
            let Some(end) = rest.find("\n        return false;") else {
                problems.push(format!(
                    "{panel}: no `return false;` found — either the block was removed or \
                     it changed shape, and this guard has not been taught the new shape"
                ));
                continue;
            };
            let body = &rest[..end];
            // A catch-all in a panel block is the danger. The tree block ends with
            // `_ => {}`, which is fine ONLY because every binding id the panel is
            // expected to handle has an arm above it — and that is what the
            // claimed list records.
            if !body.contains("_ => {}") {
                problems.push(format!(
                    "{panel}: the block does not end in `_ => {{}}` — this guard assumes it \
                     does, so re-read it before trusting this test"
                ));
            }
        }

        assert!(
            problems.is_empty(),
            "the panel/key cross-reference is out of date:\n{}",
            problems.join("\n")
        );
    }
}

#[cfg(test)]
mod lsp_policy_tests {
    use super::{Lsp, diagnostics_for};

    /// The rule that moved in M215 must survive the move: `off` buys no provider at all, and
    /// `auto` buys one.
    ///
    /// This is the whole content of `--lsp`: a caller that asked for `off` and got a provider
    /// would spawn `rust-analyzer` behind a flag that promised it would not, and a caller that
    /// asked for `auto` and got `None` would silently lose the feedback M79 added. Neither is
    /// visible from the type — `Option<Arc<dyn DiagnosticsProvider>>` says nothing about which
    /// policy produced it — so it is asserted here rather than inferred at the call site.
    #[test]
    fn the_policy_decides_whether_a_provider_exists() {
        assert!(
            diagnostics_for(Lsp::Off).is_none(),
            "--lsp off must not build a service"
        );
        assert!(
            diagnostics_for(Lsp::Auto).is_some(),
            "--lsp auto must build one"
        );
    }
}

// ==========================================================================================
// Composition
//
// Everything below moved here from `crates/epsilon-cli/src/main.rs` (ADR-021). It was a
// private `async fn` in a binary, reachable only by the frontends that live in the same
// binary. Nothing about it changed in the move except its visibility and the crate it calls
// its own helpers through.
// ==========================================================================================

use epsilon_contracts::message::Message;
use epsilon_core::{Agent, LoopLimits};
use epsilon_mcp::McpServerConfig;
use epsilon_safety::{ApprovalUi, RulesetGate};

/// Session-restore selection (M12).
///
/// Moved here with [`build_runtime`] because the composition has to resolve it *before*
/// opening anything — the ordering that A19 exists to protect — so a frontend cannot ask
/// for a session without also saying which one it meant.
#[derive(Debug, Clone, Default)]
pub enum Resume {
    /// Fresh transcript (default).
    #[default]
    No,
    /// `--continue`: newest session file for the chosen mode under `.epsilon/sessions`.
    Latest,
    /// `--resume <path>`: explicit session file.
    Path(PathBuf),
}

/// The longest lease wait this process performed, in milliseconds. Zero means none.
///
/// Read through [`lease_wait_millis`] rather than by naming the atomic, so the counter stays
/// private: a frontend should read it, not fold into it.
pub fn lease_wait_millis() -> u64 {
    LEASE_WAIT_MS.load(std::sync::atomic::Ordering::Relaxed)
}
/// Everything `build_runtime` needs to compose one agent (plan 008 / DEBT-01).
/// One `workspace` field replaces the old twin (`workspace` + `workspace`
/// were always the same path at every call site — the second name was pure noise).
pub struct RunParams<'a> {
    /// Workspace root. It is the agent's containment boundary as well as the root the
    /// conventional skill, extension and agent-definition roots resolve against.
    pub workspace: &'a Path,
    /// The provider stack to call. A frontend that wants offline behaviour hands in the
    /// faux provider here rather than reaching into the composition.
    pub stream_fn: Arc<dyn epsilon_llm::StreamFn>,
    /// How a permission ask reaches a human. The terminal hands in its channel-backed
    /// dialog; the desktop hands in the same type over its own window.
    pub approvals: Arc<dyn ApprovalUi>,
    /// MCP servers named on the command line. Project-declared ones are added by
    /// `build_runtime` from `project.mcp_servers`, so a caller passes only the extra.
    pub mcp_servers: &'a [McpServerConfig],
    /// Session JSONL path, relative to the workspace (e.g. ".epsilon/sessions/tui.jsonl").
    pub session_file: String,
    /// How side-effecting commands are confined. `Auto` resolves to the best backend the
    /// platform offers, and `Off` is a decision a caller has to make deliberately.
    pub confinement: &'a epsilon_tools::Confinement,
    /// Whether edits consult a language server.
    pub lsp: Lsp,
    /// The **resolved** model id — already run through the CLI's precedence chain. A
    /// caller that has not resolved it must resolve it; see [`crate::RunParams`]'s
    /// `model_explicit` for why the two are different facts.
    pub model_id: &'a str,
    /// Whether the user named a model on THIS invocation (`--model`).
    ///
    /// M139 needs this to decide precedence, and `model_id` alone cannot say it: by the
    /// time `build_runtime` runs, `model_id` is already the *resolved* id, which is
    /// equally true whether it came from the flag, the project file, the saved config,
    /// or a default. Only the flag is a deliberate instruction for this run; the rest
    /// are standing preferences a resumed session may legitimately override.
    pub model_explicit: bool,
    /// Which prior session, if any, this launch continues.
    pub resume: &'a Resume,
    /// The project's `.epsilon/config.toml`, already loaded and hashed.
    pub project: &'a ProjectConfig,
    /// M39: config-hash trust from dispatch (`Changed` ⇒ false). Gates the
    /// trusted root variants so project code never loads untrusted.
    pub project_trusted: bool,
    /// Prepend the plan-mode ruleset: mutating capabilities are denied outright.
    pub plan_mode: bool,
}

/// Compose one agent for `params`.
///
/// This is the whole of "what it takes to run Epsilon in a workspace": the built-in tool
/// registry plus MCP, skills and extensions; the permission layers; the agent itself; the
/// durable session store (opened on the file this launch will write); the system-prompt
/// overlay; agent definitions; the extension event forwarder; and, when resuming, the
/// repair pass, the projected model view and the session's remembered model.
///
/// Every frontend calls this. It moved out of `crates/epsilon-cli/src/main.rs` (ADR-021)
/// because a frontend in another crate could not reach it, and a second copy is the defect
/// this repository has already paid for.
///
/// # The order is load-bearing, not incidental
///
/// Two orderings in here are the subject of past defects and must not be "tidied":
///
/// * **The resume target is resolved before anything is opened.** Resolving it afterwards
///   was harmless only while the primary session file had a fixed name; since each launch
///   names its own, `--continue` resolved to the empty file the process had just created,
///   opened it a second time (waiting on its own lease for the full stale window) and
///   restored nothing. See [`session_path_for`].
/// * **The store is attached before the model view is restored**, so nothing can write to
///   the previous file.
///
/// # Errors
///
/// A `String`, not a typed error: every failure here is a user-facing sentence already
/// (a session that will not open, a migration that failed, an absent store), and the
/// message is the thing the caller has to show.
pub async fn build_runtime(
    RunParams {
        workspace,
        stream_fn,
        approvals,
        mcp_servers,
        session_file,
        confinement,
        lsp,
        model_id,
        model_explicit,
        resume,
        project,
        project_trusted,
        plan_mode,
    }: RunParams<'_>,
) -> Result<Arc<Agent>, String> {
    // M79/M215: the product path opts into language-server diagnostics, and since M215 the
    // frontend both decides (`--lsp`) and builds the service; the tool crate receives only the
    // handle, which is why it no longer links `epsilon-lsp` at all.
    let mut registry = epsilon_tools::builtin_registry_with(confinement, diagnostics_for(lsp));
    let mut all_mcp: Vec<McpServerConfig> = mcp_servers.to_vec();
    all_mcp.extend(project.mcp_servers.iter().cloned());
    for cfg in &all_mcp {
        let attached = epsilon_mcp::attach_stdio_server(&mut registry, cfg).await?;
        eprintln!("mcp server '{}' attached ({attached} tools)", cfg.name);
    }

    // Skills (M8 brick, wired M11): conventional roots feed the CONTEXT-tier catalog.
    // M39: untrusted projects see global roots only.
    let skills = epsilon_core::skills::discover(&skill_roots_trusted(workspace, project_trusted));
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
    let mut ext_roots = extension_roots_trusted(workspace, project_trusted);
    if project_trusted && !project.extension_dirs.is_empty() {
        let base = workspace.join(".epsilon");
        ext_roots.extend(project.extension_dirs.iter().map(|d| base.join(d)));
    }
    let (ext_hosts, ext_notices) = attach_extensions(&mut registry, &ext_roots).await;
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
            layers.push(project_rules_layer(&project.rules));
        }
        epsilon_safety::PermissionEngine::layered(layers)
    } else if project.rules.is_empty() {
        epsilon_safety::PermissionEngine::with_defaults()
    } else {
        epsilon_safety::PermissionEngine::layered(vec![project_rules_layer(&project.rules)])
    };
    let gate = Arc::new(RulesetGate::layered(base_engine, approvals));
    // `Agent::new` already returns an `Arc`, which is why the extension hooks below can be
    // installed through it without unwrapping or re-wrapping.
    let agent = Agent::new(
        LoopLimits::for_model(model_id),
        stream_fn,
        Arc::new(registry),
        gate,
        workspace.to_path_buf(),
    );

    // ADR-019: let extensions intercept tool calls. Installed only when something is
    // actually attached, because a harness with no extensions must not pay a round trip
    // per tool call to discover there is nobody to ask. This is the first production
    // `set_tool_hooks` call in the repository -- until now every one was a test.
    if install_extension_hooks(&agent, ext_hosts.clone()) {
        eprintln!(
            "[harness] {} extension(s) may intercept tool calls",
            ext_hosts.len()
        );
    }
    // Turn-level hooks: swap the model between turns and end a run cleanly. Installed
    // separately from the tool hooks because the two have different risk profiles, and a
    // caller may want one without the other.
    install_extension_turn_hooks(&agent, ext_hosts.clone());
    // A19: resolve the resume target BEFORE opening anything, and make it the file this launch
    // opens. The order used to be `open primary → attach → resolve resume → open resume`, which was
    // harmless only while the primary had a fixed name: `latest_session_file` then returned the same
    // file. Since M210 the primary is named for this launch, so it *is* the newest — and `-c`
    // resolved to the empty file this process had created microseconds earlier, opened it a second
    // time (waiting on its own lease for a full STALE_AFTER window) and restored nothing from it.
    // Measured: a third 147-byte file per `-c` launch and 21/22/24s of countdown. See plan 021.
    let resume_path = match resume {
        Resume::No => None,
        Resume::Path(p) => Some(p.clone()),
        Resume::Latest => {
            // M217: the per-cwd directory a launch writes into, plus the flat root where every
            // session written before that move still is — one union, so a launch can continue a
            // session whether or not it predates the directory.
            epsilon_session::latest_session_for_cwd(&sessions_root(workspace), workspace)
        }
    };
    let (session_path, resuming) =
        session_path_for(workspace, &session_file, resume_path.as_deref());
    // Reap locks whose holder is provably gone before opening anything (M211). One session per file
    // (M210) means one lock per session, and a process that was killed — a closed terminal window, a
    // `taskkill` — never removes its own, so a daily workspace accumulates two entries per launch
    // and half of them are dead, in the directory a user lists when looking for their sessions. The
    // predicate is exactly the one `acquire` uses to decide a lock may be stolen, so this cannot
    // take a lock from a living process. The count is not logged: a launch is not the place for
    // housekeeping chatter.
    if let Some(dir) = session_path.parent() {
        let _ = epsilon_session::sweep_stale_locks(dir);
    }
    let mut store = epsilon_session::SessionStore::open_in_with_progress(
        session_path.clone(),
        workspace,
        lease_wait_notice,
    )
    // The path is in the message because the refusal is not actionable without it:
    // `session line #3: …` names a line inside a file the reader has not been told the
    // name of, and the file that fails is usually the *default* one — chosen by the
    // harness, not typed by the user. (M202: the installed binary refused to start in
    // this repository for exactly this reason, and the message did not say which file.)
    // A19: when the file being opened IS the one being resumed, the refusal is phrased for the
    // file that is actually there (`resume_open_error`) — that message is what a user resuming a
    // foreign or damaged session needs, and it used to be produced by the second open this fix
    // removed. A plain launch keeps the shorter one.
    .map_err(|e| {
        if resuming {
            resume_open_error(&session_path, e)
        } else {
            format!("open session store {}: {e}", session_path.display())
        }
    })?;
    if store.migrated() {
        match store.persist_migration() {
            // Name the backup. Converting an older session rewrites it in place, and the one
            // thing a user needs to know when that happens to a file another tool wrote is
            // where the previous bytes went — a `.pre-epsilon` sibling, kept once and never
            // overwritten. Silence here would be the same class of problem the migration
            // itself had: something changed the user's file and did not say so.
            Ok(()) => eprintln!(
                "migrated session store to current version (previous bytes kept at {})",
                epsilon_session::original_backup_path(store.path()).display()
            ),
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
        &agent_user_roots(),
        &agent_project_roots(workspace, project_trusted),
        project_trusted,
    );
    if !agents.is_empty() {
        let names: Vec<&str> = agents.iter().map(|a| a.name.as_str()).collect();
        eprintln!("agents: {}", names.join(", "));
    }
    agent.set_agents(agents);
    agent.attach_session_store(Arc::new(std::sync::Mutex::new(store)));
    spawn_event_forwarder(agent.subscribe(), ext_hosts);

    // Resume (M12): seed runtime history from a prior transcript. The restored view is
    // the store's SHADOWED model view, so resumed sessions continue post-compaction.
    //
    // A19: the store is already open and attached — `session_path` above *is* the resumed path when
    // there is one, so this block no longer opens a second store. It used to, and that second open
    // was the defect: same path, second lease, thirty-second wait, empty restored conversation.
    if let Some(rp) = resume_path {
        let attached = agent
            .session_store()
            .ok_or("no session store attached for a resume")?;
        let mut restored_guard = attached.lock().map_err(|_| "session store lock poisoned")?;
        let restored: &mut epsilon_session::SessionStore = &mut restored_guard;
        // M41: a resumed file may predate headers — migrate before repair reads
        // it, so the repair pass sees healed entries, never legacy shapes.
        if restored.migrated() {
            match restored.persist_migration() {
                // The resume path is where a *foreign* file is most likely to arrive, so the
                // message names the backup too — see the note on the other call site.
                Ok(()) => eprintln!(
                    "migrated {} to current version (previous bytes kept at {})",
                    rp.display(),
                    epsilon_session::original_backup_path(restored.path()).display()
                ),
                Err(e) => eprintln!("migration warning (continuing): {e}"),
            }
        }
        // ponytail-audit follow-up: repair is now WIRED, not test-only — a crashed
        // prior run leaves dangling tool calls on the active branch; synthesize
        // tombstones before the model view so the resumed context stays provider-valid.
        match epsilon_session::repair::repair_on_open(&mut *restored) {
            Ok(report) => {
                for line in report.summary() {
                    eprintln!("repaired {}: {line}", rp.display());
                }
            }
            Err(e) => eprintln!("repair warning (continuing): {e}"),
        }
        // An entry kind this build does not know is preserved, not refused (M113), which leaves
        // it in the file and out of the transcript by construction: the session opens looking
        // complete while something in it was silently skipped. Nothing else on this path can
        // tell the user that, so say it here, beside the repairs and in the same voice.
        if let Some(notice) = restored.preserved_unknown_notice() {
            eprintln!("{}: {notice}", rp.display());
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
        // M139: a model switch recorded in the session used to be read by nobody, so a
        // resumed conversation silently came back on the launch model. Apply it here —
        // and only here, because this is the point where the session is known and the
        // agent still exists.
        //
        // Precedence, and it is not arbitrary: an explicit `--model` on THIS invocation
        // is a deliberate instruction for this run, while the session's model is a
        // remembered default. So flag > session > project/config, which is exactly the
        // order `candidate_raw` already builds above — the session slots in only when the
        // user expressed no preference of their own.
        if let Some(resumed) = resumed_model_to_use(model_explicit, &*restored, model_id) {
            let (provider, session_model) = resumed;
            if session_model != model_id {
                eprintln!(
                    "resumed session was last used on {provider}/{session_model}; continuing there \
                     (pass --model to override)"
                );
                agent.set_model_id(session_model);
            }
        }
    }
    Ok(agent)
}

/// Which file a launch opens, and whether that makes it a resumed launch.
///
/// **A19's invariant, written where it can be tested: the resume target wins.** A launch must write
/// to the file it resumes. The old shape kept the frontend's default as the session path and opened
/// the resume target as a *second* store — invisible while the default name was fixed
/// (`latest_session_file` then returned the same file) and fatal once each launch named its own: the
/// newest file in the directory was the empty one this process had just created, so `--continue`
/// resolved to its own file, waited a full lease window on its own lock, and restored nothing.
///
/// Extracted for the same reason `resumed_model_to_use` was: the rule is what the product runs, and
/// a copy of it inside a test would test the copy.
///
/// The returned `bool` is not decoration — the caller uses it to phrase the open's error message for
/// a resumed file (`resume_open_error`) rather than for a launch.
pub fn session_path_for(
    workspace: &std::path::Path,
    session_file: &str,
    resume_path: Option<&std::path::Path>,
) -> (std::path::PathBuf, bool) {
    match resume_path {
        Some(rp) => (rp.to_path_buf(), true),
        None => (workspace.join(session_file), false),
    }
}

#[cfg(test)]
mod session_path_tests {
    use super::session_path_for;
    use std::path::{Path, PathBuf};

    /// The property A19 was: **the file written is the file resumed.**
    #[test]
    fn a_resume_target_becomes_the_file_this_launch_opens() {
        let workspace = Path::new("/ws");
        let resumed = Path::new("/ws/.epsilon/sessions/2026-09-30T00-00-00-000Z_ses_a.jsonl");
        let (path, resuming) =
            session_path_for(workspace, ".epsilon/sessions/tui.jsonl", Some(resumed));
        assert_eq!(path, PathBuf::from(resumed));
        assert!(
            resuming,
            "the caller needs to know, to phrase a refusal for a resumed file"
        );
    }

    /// A plain launch is unchanged: the frontend's own name, relative to the workspace.
    #[test]
    fn a_launch_without_a_resume_keeps_its_own_file() {
        let (path, resuming) =
            session_path_for(Path::new("/ws"), ".epsilon/sessions/tui.jsonl", None);
        assert_eq!(path, PathBuf::from("/ws/.epsilon/sessions/tui.jsonl"));
        assert!(!resuming);
    }
}
/// Which model a resumed run should continue on, and why. `None` means "keep the launch
/// model" — the caller then changes nothing.
///
/// Extracted from `build_runtime` so the precedence is testable without standing up a
/// workspace, a provider and a registry. A closure copy of the same rule inside a test
/// would have tested the copy; this tests the rule the product actually runs.
///
/// The precedence is the whole content of the function: an explicit `--model` is a
/// deliberate instruction for *this* invocation, while the session's model is a
/// remembered default. So the session may only take over when the user said nothing.
pub fn resumed_model_to_use(
    model_explicit: bool,
    store: &epsilon_session::SessionStore,
    launch_model: &str,
) -> Option<(String, String)> {
    if model_explicit {
        return None;
    }
    let settings = store.context_settings().ok()?;
    let (provider, session_model) = settings.model?;
    if session_model == launch_model {
        return None;
    }
    // Resolve through the catalog so a session that recorded an alias or a partial id
    // still lands on a real row; fall back to the raw id for a custom model the catalog
    // does not know, which pi also accepts.
    let resolved = epsilon_llm::catalog::resolve_model_for_run(&session_model)
        .map_or_else(|| session_model.clone(), |r| r.info.id.to_string());
    Some((provider, resolved))
}

/// Tell the user that a session lease is being waited out, and for roughly how much longer.
///
/// M138. M136 stopped the refusal, so a killed TUI no longer locks the next launch
/// out for 30 seconds — but it replaced a clear error with a silent wait, and a
/// silent wait is the same shape of failure the owner reported about this harness:
/// the screen does nothing while work is happening underneath.
///
/// Design notes, because a progress line in a terminal is easy to get wrong:
///
/// - **stderr, not stdout.** Every other startup note here uses `eprintln!`, so a
///   caller piping stdout does not get control characters in its data, and nothing
///   else in the process is drawing at this point.
/// - **One line, redrawn in place.** `\r` plus clear-to-end-of-line stops the notice
///   becoming a 20-line scrollback of near-identical text. That works in both CMD
///   and PowerShell, which is the whole platform matrix here. It is not a
///   TTY-sniffing progress bar.
/// - **Not a spinner.** A spinner needs a timer thread and a redraw loop; this runs
///   on a blocking path before the async runtime is doing anything. A countdown
///   that changes about once a second is enough to prove liveness, and costs nothing.
/// - **Says why.** "another process may still be writing it" is the fact the user
///   needs. The pid, if there is one, is in the error they get if the wait runs out.
pub fn lease_wait_notice(waited: std::time::Duration, remaining: std::time::Duration) {
    // Remember the wait, because the line below is written to a console that the TUI's alternate
    // screen then erases: a launch that waited 25 s would otherwise start with no record of why
    // it paused (ledger A18). A process-global rather than a parameter because the wait happens
    // inside `build_runtime`, which four frontends share and which takes this function by name —
    // threading a second return value through it for one notice would cost more than it explains.
    // `fetch_max`, so the longest wait of a launch is what gets reported.
    LEASE_WAIT_MS.fetch_max(
        waited.as_millis() as u64,
        std::sync::atomic::Ordering::Relaxed,
    );
    let mut err = std::io::stderr();
    write_lease_wait(&mut err, waited, remaining);
}

/// The longest lease wait this process performed, in milliseconds. Zero means none.
///
/// Written by [`lease_wait_notice`], read once by the TUI path after `build_runtime` returns.
static LEASE_WAIT_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// What the TUI says about a wait that happened before it started, or `None` when there was none.
///
/// **One second is the threshold**, and it is a judgement worth stating: the ordinary contended
/// launch waits a few hundred milliseconds for a lease another process released, and a notice
/// about that would be noise on every second launch. A second is where a person notices a pause
/// and starts wondering whether the program is stuck — which is the state this closes.
pub fn lease_wait_summary(waited_ms: u64) -> Option<String> {
    if waited_ms < 1_000 {
        return None;
    }
    Some(format!(
        "this launch waited {}s for the session file's lease — another process had it, and it \
         has been released. Nothing was lost; the session continued from the same file.",
        waited_ms / 1_000
    ))
}

/// One lease-wait progress line, written to `w`.
///
/// Split out from [`lease_wait_notice`] so the rendering is testable. The defect M138
/// fixes is "nothing appears on the screen", and a function that writes straight to the
/// process's stderr cannot be asserted on by any test. Here it can: the test builds the
/// same bytes the user would see and checks the countdown, the carriage return, and the
/// clear-to-end-of-line that stops each redraw leaving a stale tail behind.
fn write_lease_wait(
    w: &mut impl std::io::Write,
    waited: std::time::Duration,
    remaining: std::time::Duration,
) {
    let waited = waited.as_secs();
    let remaining = remaining.as_secs_f64().ceil() as u64;
    // `\r` redraws in place; `\x1b[K` clears to end of line. Both are understood by
    // CMD and PowerShell's console host, which is the whole platform matrix here.
    let _ = write!(
        w,
        "\r  waiting for this session to be released — {waited}s elapsed, ~{remaining}s left \
         (another process may still be writing it)   \x1b[K"
    );
    let _ = w.flush();
}

/// Why `--resume` could not open this file, phrased for the file that is actually there.
///
/// The store's own message for a current pi session is true and useless: "session line #2: valid
/// JSON that is not an entry, a header or a legacy entry — a shape this build does not know"
/// names what this build *cannot* read, not what the file *is* or what to do about it. A refusal
/// a user cannot act on is half a feature, so when the file turns out to be a pi tree the
/// diagnosis is followed by the exact command that converts it — and by the promise the import
/// keeps, that the pi file stays untouched.
pub fn resume_open_error(path: &std::path::Path, e: epsilon_session::StoreError) -> String {
    let base = format!("open {}: {e}", path.display());
    if !epsilon_session::looks_like_pi_session(path) {
        return base;
    }
    let quoted = if path.to_string_lossy().contains(' ') {
        format!("\"{}\"", path.display())
    } else {
        path.display().to_string()
    };
    format!(
        "{base}\n{} is a current pi session (entries carry id/parentId, the kind sits at the \
         top level), which this build cannot open in place — run:\n  epsilon --import {quoted}\n\
         that writes an Epsilon session beside it and leaves the pi file exactly as it is; \
         resume the copy it names",
        path.display()
    )
}

/// The lease-wait notice moved here with `lease_wait_notice` (ADR-021). These tests
/// were in `crates/epsilon-cli/src/main.rs` and asserted on the bytes a user sees,
/// which only works because `write_lease_wait` renders into a caller-supplied writer.
#[cfg(test)]
#[cfg(test)]
mod lease_wait_notice_tests {
    use super::*;

    fn render(waited: u64, remaining: f64) -> String {
        let mut buf: Vec<u8> = Vec::new();
        write_lease_wait(
            &mut buf,
            std::time::Duration::from_secs(waited),
            std::time::Duration::from_secs_f64(remaining),
        );
        String::from_utf8(buf).expect("notice is utf-8")
    }

    /// The user-visible defect M138 fixes is a blank screen, so assert on the bytes the
    /// user would actually see: a redrawn line carrying a real countdown and a reason.
    #[test]
    fn the_notice_shows_a_countdown_and_why_it_is_waiting() {
        let out = render(3, 27.0);
        assert!(
            out.starts_with('\r'),
            "each poll must redraw in place, not append a line: {out:?}"
        );
        assert!(out.contains("3s elapsed"), "{out:?}");
        assert!(out.contains("~27s left"), "{out:?}");
        assert!(
            out.contains("another process may still be writing it"),
            "the user must be told why, not just that something is slow: {out:?}"
        );
    }

    /// Clear-to-end-of-line is what stops each redraw leaving the previous, longer
    /// countdown visible as a stale tail. Dropping the `\x1b[K` is the natural
    /// simplification and it produces garbage a user reads as wrong numbers.
    #[test]
    fn the_notice_clears_the_previous_polls_tail() {
        assert!(
            render(1, 29.0).ends_with("\x1b[K"),
            "a shorter message must clear what the previous longer one drew"
        );
    }

    /// A18: the wait is remembered, so the TUI can say it in a place that survives the
    /// alternate screen.
    ///
    /// Asserted with `>=` rather than `==` because the counter is a process-global and a
    /// monotonic one: another test in this binary may have waited longer, and a test that
    /// insisted on exact equality would be asserting on test order.
    #[test]
    fn a_wait_is_remembered_for_the_frontend_that_starts_after_it() {
        let before = lease_wait_millis();
        lease_wait_notice(
            std::time::Duration::from_secs(4),
            std::time::Duration::from_secs(26),
        );
        let after = lease_wait_millis();
        assert!(
            after >= 4_000,
            "the wait must be recorded for the TUI to report: {before} -> {after}"
        );
    }

    /// The threshold, and the two sentences it produces. A sub-second wait is the ordinary
    /// contended launch and must stay silent; a second is where a person starts wondering
    /// whether the program is stuck.
    #[test]
    fn only_a_wait_long_enough_to_notice_becomes_a_notice() {
        assert!(lease_wait_summary(0).is_none(), "no wait, nothing to say");
        assert!(
            lease_wait_summary(999).is_none(),
            "a sub-second contention is not worth a line on every second launch"
        );
        let said = lease_wait_summary(25_400).expect("a 25 s wait is worth explaining");
        assert!(said.contains("waited 25s"), "{said}");
        assert!(said.contains("lease"), "{said}");
        assert!(
            said.contains("Nothing was lost"),
            "the reader's real question is whether the pause cost them something: {said}"
        );
    }

    /// Sub-second remainders round UP. A countdown showing `0s left` while 400ms remain
    /// reads as "finished" when it has not, which is the confusion this line removes.
    #[test]
    fn a_sub_second_remainder_rounds_up_not_down() {
        let out = render(0, 0.4);
        assert!(out.contains("~1s left"), "{out:?}");
    }
}

/// The M139 resume-model-precedence tests moved here with `resumed_model_to_use`
/// (ADR-021). They assert the rule `build_runtime` applies, so they belong beside it.
#[cfg(test)]
mod resume_model_precedence_tests {
    use super::*;
    // ---- M139: model precedence on resume ----

    /// Build a session whose last entry is a `model_change` to `session_model`, and return
    /// the store. The lease is dropped at the end of this function so the caller can open
    /// it again without contending with itself.
    fn store_with_model_change(
        dir: &std::path::Path,
        session_model: &str,
    ) -> epsilon_session::SessionStore {
        let path = dir.join("resume.jsonl");
        let mut store = epsilon_session::SessionStore::open_in(&path, dir).expect("store");
        store
            .append(epsilon_contracts::session_entry::EntryKind::ModelChange {
                provider: "testprov".into(),
                model_id: session_model.into(),
            })
            .expect("append model_change");
        drop(store);
        epsilon_session::SessionStore::open_in(&path, dir).expect("reopen")
    }

    /// The flag wins. This is the regression that would be silent and unnerving: the user
    /// explicitly names a model, gets overridden anyway, and has no way to notice.
    #[test]
    fn an_explicit_model_is_never_overridden_by_the_session() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_with_model_change(dir.path(), "some-other-model");
        assert_eq!(
            resumed_model_to_use(true, &store, "faux-1"),
            None,
            "a --model flag must survive a resume untouched"
        );
    }

    /// With no flag, the session's model is what the user was last using.
    #[test]
    fn the_sessions_model_takes_over_when_no_flag_was_given() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_with_model_change(dir.path(), "some-other-model");
        let chosen = resumed_model_to_use(false, &store, "faux-1")
            .expect("the session's model should be adopted when the user named none");
        assert_eq!(chosen.0, "testprov");
        assert_eq!(
            chosen.1, "some-other-model",
            "an id the catalog does not know is kept verbatim, as pi does for custom models"
        );
    }

    /// A session whose model already matches the launch model changes nothing — no
    /// announcement, no pointless `set_model_id`.
    #[test]
    fn a_matching_model_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_with_model_change(dir.path(), "faux-1");
        assert_eq!(
            resumed_model_to_use(false, &store, "faux-1"),
            None,
            "no change means no change, and no confusing announcement"
        );
    }
}
