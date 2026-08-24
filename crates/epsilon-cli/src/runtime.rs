//! Runtime composition helpers: conventional-path discovery for skills and extensions.
//!
//! M11 makes the M8 bricks reachable from the binary: skills discovered from
//! conventional directories populate the CONTEXT-tier catalog, and `extension.json`
//! manifests spawn isolated hosts whose tools join the normal registry. Paths are
//! hardcoded conventions here; the full ADR-012 config layer (M12) will let projects
//! declare them explicitly.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use epsilon_contracts::agent_event::AgentEvent;
use epsilon_core::ToolRegistry;
use epsilon_ext::ExtHost;

/// Parsed `.epsilon/config.toml` — the project layer of ADR-012.
///
/// Declarative only; unknown keys are preserved on disk but ignored by the runtime.
/// First use records a SHA-256 trust hash (`.epsilon/trust.json`); later runs warn
/// loudly when the file changes underneath them.
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

/// Load + trust-check the project config. Returns `(config, diagnostics)`.
///
/// Trust policy (v1, headless): first sight auto-trusts and records the hash; any later
/// content change prints a loud banner but still loads. Interactive re-prompting is a
/// TUI concern and stays open for M13 polish.
pub fn load_project_config(workspace: &Path) -> (ProjectConfig, Vec<String>) {
    let mut notices = Vec::new();
    let path = workspace.join(PROJECT_CONFIG_REL);
    if !path.is_file() {
        return (ProjectConfig::default(), notices);
    }
    let raw = match std::fs::read_to_string(&path) {
        Ok(r) => r,
        Err(e) => {
            notices.push(format!("project config unreadable: {e}"));
            return (ProjectConfig::default(), notices);
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
    match previous {
        None => {
            notices.push(format!(
                "trusting project config {} (hash recorded)",
                path.display()
            ));
        }
        Some(prev) if prev != hash => {
            notices.push(
                "WARNING: .epsilon/config.toml CHANGED since it was trusted; \
                 review before running untrusted projects"
                    .to_string(),
            );
        }
        _ => {}
    }
    let _ = std::fs::write(
        &trust_path,
        serde_json::json!({ "config_sha256": hash }).to_string(),
    );

    // ---- parse ----------------------------------------------------------------------
    let mut cfg = ProjectConfig::default();
    let parsed: toml::Value = match toml::from_str(&raw) {
        Ok(v) => v,
        Err(e) => {
            notices.push(format!("invalid project config: {e}"));
            return (cfg, notices);
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

    (cfg, notices)
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(bytes);
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
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
pub fn skill_roots(workspace: &Path) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(dir) = epsilon_core::config::global_dir() {
        roots.push(dir.join("skills"));
    }
    roots.push(workspace.join(".epsilon").join("skills"));
    roots
}

/// Extension roots: global (`~/.config/epsilon/extensions`) then project.
pub fn extension_roots(workspace: &Path) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(dir) = epsilon_core::config::global_dir() {
        roots.push(dir.join("extensions"));
    }
    roots.push(workspace.join(".epsilon").join("extensions"));
    roots
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

/// Background pump: broadcast agent events into every attached host. The host applies
/// its manifest's filter internally (`send_event(filtered=true)`, M9a contract), so the
/// pump stays dumb: forwarding failures never affect the run.
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
                            host.send_event(true, &payload).await;
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
    }
}

/// Newest `*.jsonl` session under `dir` (for `--continue`).
pub fn latest_session_file(dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().is_some_and(|x| x == "jsonl"))
        .max_by_key(|e| e.metadata().and_then(|m| m.modified()).ok())
        .map(|e| e.path())
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
        assert_eq!(found[0].1.version, "0.1.0");
        assert_eq!(problems.len(), 1);
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

        let old = dir.join("old.jsonl");
        let new = dir.join("new.jsonl");
        std::fs::write(&old, "{}").unwrap();
        std::fs::write(dir.join("notes.txt"), "x").unwrap();

        // Ensure distinct mtimes even on coarse filesystem clocks.
        std::thread::sleep(std::time::Duration::from_millis(50));
        std::fs::write(&new, "{}").unwrap();

        assert_eq!(latest_session_file(&dir), Some(new));
        let _ = old;
    }
}
