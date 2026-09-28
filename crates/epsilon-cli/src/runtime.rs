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
}
