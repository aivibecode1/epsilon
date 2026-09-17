//! Runtime composition helpers: conventional-path discovery for skills and extensions.
//!
//! M11 makes the M8 bricks reachable from the binary: skills discovered from
//! conventional directories populate the CONTEXT-tier catalog, and `extension.json`
//! manifests spawn isolated hosts whose tools join the normal registry. Paths are
//! hardcoded conventions here; the full ADR-012 config layer (M12) will let projects
//! declare them explicitly.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use epsilon_contracts::agent_event::AgentEvent;
use epsilon_core::ToolRegistry;
use epsilon_ext::ExtHost;

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

/// Current SHA-256 of the project config file, if present (used by the TUI
/// re-prompt to persist an acceptance).
pub fn current_project_config_hash(workspace: &Path) -> Option<String> {
    let raw = std::fs::read_to_string(workspace.join(PROJECT_CONFIG_REL)).ok()?;
    Some(sha256_hex(raw.as_bytes()))
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
    /// `.epsilon/prompts/` (pi `prompts`: system-prompt overlays).
    Prompts,
    /// `.epsilon/themes/` (pi `themes`: TUI theme code).
    Themes,
    /// `.epsilon/SYSTEM.md` (pi `SYSTEM.md`: appended system instructions).
    SystemMd,
    /// `.epsilon/APPEND_SYSTEM.md` (pi `APPEND_SYSTEM.md`).
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
        AgentEvent::CompactionFinished => "compaction_finished",
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
