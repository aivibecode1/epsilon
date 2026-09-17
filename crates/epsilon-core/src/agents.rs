//! Agent-type definitions for `task` delegation (pi subagent-example parity).
//!
//! Pi ships no built-in `task` tool in its core surface — delegation arrives via
//! the `subagent` example extension (`agents.ts` + `agents/*.md`): markdown agent
//! defs with YAML-lite frontmatter (`name`, `description`, `tools`, `model`) +
//! body-as-system-prompt, discovered from a user dir (always) and a project dir
//! (only with `agentScope` project/both). Opencode bakes the same idea in as
//! native agents (`build`/`plan` primary + `general`/`explore` subagent) with a
//! sorted `- name: description` list appended to the task description.
//!
//! This module ports the discovery half: `AgentDef` mirrors pi's `AgentConfig`
//! (name/description/tools/model/system-prompt/source), `discover_agents`
//! mirrors `discoverAgents` (user roots always, project roots only when
//! trusted — pi's agentScope gate maps onto the M39 trust lattice), and
//! `describe_task` mirrors opencode's `ToolRegistry.describeTask` (sorted list
//! for the task tool description). The execution half (routing `run_task` by
//! type) consumes these defs; unknown types stay an error naming what's
//! available (opencode parity).

use std::path::PathBuf;

/// One discovered agent definition (pi `AgentConfig`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentDef {
    /// Validated agent name (frontmatter `name`, required — no filename fallback:
    /// pi skips defs without a string name AND description).
    pub name: String,
    /// One-line purpose shown in the task description list.
    pub description: String,
    /// Tool subset for children of this type (`None` = full registry, pi `worker`
    /// parity — no `tools` key means every tool).
    pub tools: Option<Vec<String>>,
    /// Model override for children of this type (`None` = inherit parent).
    pub model: Option<String>,
    /// Markdown body = child system prompt (pi: body past the frontmatter).
    pub system_prompt: String,
    /// Where this def came from (user always loads; project needs trust).
    pub source: AgentSource,
}

/// Where an agent def was discovered (pi's `source: "user" | "project"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentSource {
    /// Global dir — always loaded.
    User,
    /// Workspace-local dir — only when the project is trusted.
    Project,
}

/// Discover agent defs across root directories (each scanned one level deep for
/// `*.md` files with frontmatter).
///
/// `user_roots` always load; `project_roots` load only when `project_trusted`
/// (pi's `agentScope: "user"` default vs `"project"`/`"both"` — the trust
/// lattice from M39 is the scope gate here: untrusted projects never inject
/// child prompts). Later roots shadow earlier ones by name (project overrides
/// global), mirroring pi's `discoverAgents` + the skill shadowing rule.
pub fn discover_agents(
    user_roots: &[PathBuf],
    project_roots: &[PathBuf],
    project_trusted: bool,
) -> Vec<AgentDef> {
    let mut out: Vec<AgentDef> = Vec::new();
    for root in user_roots {
        load_agents_from_dir(root, AgentSource::User, &mut out);
    }
    if project_trusted {
        for root in project_roots {
            load_agents_from_dir(root, AgentSource::Project, &mut out);
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

fn load_agents_from_dir(root: &PathBuf, source: AgentSource, out: &mut Vec<AgentDef>) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for e in entries.filter_map(Result::ok) {
        let path = e.path();
        // pi: `.md` files only, files + symlinks, one bad file never takes down
        // the rest of the directory.
        if path.extension().map(|x| x == "md") != Some(true) {
            continue;
        }
        let Ok(raw) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Some(def) = parse_agent_def(&raw, source) else {
            continue;
        };
        // Shadowing: replace the earlier-root def with the same name.
        if let Some(slot) = out.iter_mut().find(|a| a.name == def.name) {
            *slot = def;
        } else {
            out.push(def);
        }
    }
}

/// Parse one agent file: `---` frontmatter (`name` + `description` required
/// strings, `tools` string-or-list, `model` string) + body as system prompt.
/// Returns `None` for anything pi would skip (missing frontmatter, non-string
/// name/description).
fn parse_agent_def(raw: &str, source: AgentSource) -> Option<AgentDef> {
    let mut lines = raw.lines();
    if lines.next().map(str::trim) != Some("---") {
        return None;
    }
    let mut name: Option<String> = None;
    let mut description: Option<String> = None;
    let mut tools: Option<Vec<String>> = None;
    let mut model: Option<String> = None;
    let mut consumed = 1usize;
    let mut closed = false;
    for line in lines.by_ref() {
        consumed += 1;
        if line.trim() == "---" {
            closed = true;
            break;
        }
        let Some((k, v)) = line.split_once(':') else {
            continue;
        };
        match k.trim() {
            "name" => name = Some(v.trim().to_string()),
            "description" => description = Some(v.trim().to_string()),
            // Both spellings are valid YAML and both in use:
            // `tools: read, bash` (string) and `tools: [read, bash]` (list).
            // Anything else yields no tools (full registry) rather than
            // throwing — discovery must not fail on one odd file.
            "tools" => {
                let v = v.trim();
                let list: Vec<String> = if v.starts_with('[') && v.ends_with(']') {
                    v[1..v.len() - 1]
                        .split(',')
                        .map(|t| t.trim().trim_matches('"').trim_matches('\'').to_string())
                        .filter(|t| !t.is_empty())
                        .collect()
                } else {
                    v.split(',')
                        .map(|t| t.trim().to_string())
                        .filter(|t| !t.is_empty())
                        .collect()
                };
                if !list.is_empty() {
                    tools = Some(list);
                }
            }
            "model" => {
                let v = v.trim();
                if !v.is_empty() {
                    model = Some(v.to_string());
                }
            }
            _ => {}
        }
    }
    if !closed {
        return None;
    }
    let (name, description) = (name?, description?);
    if name.is_empty() || description.is_empty() {
        return None;
    }
    let body: Vec<&str> = raw.lines().skip(consumed).collect();
    Some(AgentDef {
        name,
        description,
        tools,
        model,
        system_prompt: body.join("\n"),
        source,
    })
}

/// Task-tool description suffix listing routable agent types (opencode
/// `describeTask` parity): `- name: description` lines sorted by name, joined
/// under one header. Empty when no agents are discovered (description stays
/// exactly today's text — no dangling header).
pub fn describe_task(agents: &[AgentDef]) -> String {
    if agents.is_empty() {
        return String::new();
    }
    let mut s = String::from("Available agent types and the tools they have access to:");
    for a in agents {
        s.push_str(&format!("\n- {}: {}", a.name, a.description));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_agent(root: &std::path::Path, file: &str, content: &str) {
        std::fs::write(root.join(file), content).unwrap();
    }

    fn scout_md() -> &'static str {
        "---\nname: scout\ndescription: Fast recon\ntools: read, grep, find, ls, bash\nmodel: haiku-x\n---\n\nYou are a scout.\n"
    }

    #[test]
    fn discovers_parses_and_sorts() {
        let dir = tempfile::tempdir().unwrap();
        write_agent(dir.path(), "scout.md", scout_md());
        write_agent(
            dir.path(),
            "worker.md",
            "---\nname: worker\ndescription: Full tools\n---\n\nYou do everything.\n",
        );
        // Junk never takes down the directory: wrong extension, no
        // frontmatter, missing description.
        write_agent(
            dir.path(),
            "notes.txt",
            "---\nname: x\ndescription: y\n---\n",
        );
        write_agent(dir.path(), "nofm.md", "just a body");
        write_agent(dir.path(), "nodesc.md", "---\nname: nodesc\n---\nbody");

        let agents = discover_agents(&[dir.path().to_path_buf()], &[], true);
        assert_eq!(agents.len(), 2);
        // Sorted by name: scout before worker.
        assert_eq!(agents[0].name, "scout");
        assert_eq!(
            agents[0].tools.as_deref(),
            Some(
                &[
                    "read".to_string(),
                    "grep".to_string(),
                    "find".to_string(),
                    "ls".to_string(),
                    "bash".to_string()
                ][..]
            )
        );
        assert_eq!(agents[0].model.as_deref(), Some("haiku-x"));
        assert_eq!(agents[0].system_prompt.trim(), "You are a scout.");
        assert_eq!(agents[0].source, AgentSource::User);
        // No `tools` key = full registry (pi worker parity).
        assert_eq!(agents[1].tools, None);
        assert_eq!(agents[1].model, None);
    }

    #[test]
    fn tools_list_spelling_and_project_shadow() {
        let global = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        write_agent(global.path(), "scout.md", scout_md());
        write_agent(
            project.path(),
            "scout.md",
            "---\nname: scout\ndescription: Project override\ntools: [read, grep]\n---\n\nProject scout.\n",
        );
        // Untrusted: project defs never load (pi agentScope gate).
        let agents = discover_agents(
            &[global.path().to_path_buf()],
            &[project.path().to_path_buf()],
            false,
        );
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].description, "Fast recon");
        // Trusted: project shadows global by name, list spelling parses.
        let agents = discover_agents(
            &[global.path().to_path_buf()],
            &[project.path().to_path_buf()],
            true,
        );
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].description, "Project override");
        assert_eq!(agents[0].source, AgentSource::Project);
        assert_eq!(
            agents[0].tools.as_deref(),
            Some(&["read".to_string(), "grep".to_string()][..])
        );
    }

    #[test]
    fn describe_task_lists_sorted_or_empty() {
        assert_eq!(describe_task(&[]), "");
        let dir = tempfile::tempdir().unwrap();
        write_agent(
            dir.path(),
            "worker.md",
            "---\nname: worker\ndescription: Full tools\n---\nbody",
        );
        write_agent(
            dir.path(),
            "alpha.md",
            "---\nname: alpha\ndescription: First\n---\nbody",
        );
        let agents = discover_agents(&[dir.path().to_path_buf()], &[], true);
        let desc = describe_task(&agents);
        let alpha = desc.find("- alpha: First").expect("alpha listed");
        let worker = desc.find("- worker: Full tools").expect("worker listed");
        assert!(alpha < worker, "sorted by name: {desc}");
    }
}
