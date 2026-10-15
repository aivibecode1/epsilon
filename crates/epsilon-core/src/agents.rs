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
//! (name/description/tools/model/system-prompt/source) and `discover_agents`
//! mirrors `discoverAgents` (user roots always, project roots only when
//! trusted — pi's agentScope gate maps onto the M39 trust lattice). The
//! execution half (routing `run_task` by type) consumes these defs; unknown
//! types stay an error naming what's available (opencode parity).

use std::path::{Path, PathBuf};

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

/// What discovery found, and what it could not look at.
///
/// **Why this type exists, and why it is a sibling of `skills::Discovered` rather than a
/// second idea.** `load_agents_from_dir` used to `let Ok(entries) = read_dir(root) else
/// { return }` and `let Ok(raw) = read_to_string(&path) else { continue }`, so an agents root
/// nobody could read and an agent `.md` nobody could parse were both silent — and a user whose
/// agents had not loaded was told nothing, which is the shape `AGENTS.md` §2.7 calls worse than
/// a missing feature. `Vec<AgentDef>` cannot express either fact.
///
/// One idiom per crate, so this is deliberately the same shape as
/// [`crate::skills::Discovered`]: a value plus the lines that say what discovery could not do.
/// A second vocabulary for the same idea in the same crate is how people learn to ignore both.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Discovered {
    /// The defs that passed every gate, sorted by name as [`discover_agents`] sorts them.
    pub agents: Vec<AgentDef>,
    /// One line per thing discovery could not do, in the order it happened. **Empty means
    /// discovery genuinely completed** — which is not the same claim as "there was nothing to
    /// report", and is the distinction the old signature could not express.
    pub notices: Vec<String>,
}

/// Discover agent defs across root directories (each scanned one level deep for
/// `*.md` files with frontmatter).
///
/// `user_roots` always load; `project_roots` load only when `project_trusted`
/// (pi's `agentScope: "user"` default vs `"project"`/`"both"` — the trust
/// lattice from M39 is the scope gate here: untrusted projects never inject
/// child prompts). Later roots shadow earlier ones by name (project overrides
/// global), mirroring pi's `discoverAgents` + the skill shadowing rule.
///
/// **The wrapper.** [`discover_agents_with_notices`] carries everything this dropped;
/// this keeps the `Vec<AgentDef>` signature so a caller with no surface to print
/// on is not forced to invent one — the same split `skills::discover` makes.
pub fn discover_agents(
    user_roots: &[PathBuf],
    project_roots: &[PathBuf],
    project_trusted: bool,
) -> Vec<AgentDef> {
    discover_agents_with_notices(user_roots, project_roots, project_trusted).agents
}

/// Discover agent defs and report what could not be discovered — the channel
/// [`discover_agents`] lacked.
///
/// # What earns a notice
///
/// The rule is the one `skills::discover_with_notices` uses, unchanged: **a notice is for
/// something a person wrote, or has on disk, that this run did not use.** Each of these was a
/// silent loss:
///
/// * **a root that could not be read** — named, with the reason. A root that is simply
///   *absent* is not a notice: most workspaces have no `agents` directory at all, and a channel
///   that speaks on every launch is one people learn to stop reading.
/// * **an entry the directory could not hand over**, and **a `.md` that is there and cannot be
///   read** — an agent def somebody wrote that will never load. A file with no `name` and
///   `description` in its frontmatter is announced too: in an agents root a `.md` *is* an agent
///   def by intent, so silence there is what makes "my agent does nothing" unfixable.
/// * **a name that shadows an earlier root's** — the file that won, and the rule that decided it.
///
/// What deliberately does **not** produce one: an **untrusted project's roots**, which are never
/// read at all. That is the `agentScope` gate doing its job, and the trust prompt already says
/// so; a notice per untrusted launch would train people to read past it.
///
/// [`crate::skills::Discovered`] is the shape this mirrors, and `epsilon-tui`'s `/reload` is
/// where both are forwarded — through `push_notice`, so a person looking for *why* an agent did
/// not load finds it above the count that came up short.
pub fn discover_agents_with_notices(
    user_roots: &[PathBuf],
    project_roots: &[PathBuf],
    project_trusted: bool,
) -> Discovered {
    let mut agents: Vec<AgentDef> = Vec::new();
    let mut notices: Vec<String> = Vec::new();
    for root in user_roots {
        load_agents_from_dir(root, AgentSource::User, &mut agents, &mut notices);
    }
    if project_trusted {
        for root in project_roots {
            load_agents_from_dir(root, AgentSource::Project, &mut agents, &mut notices);
        }
    }
    agents.sort_by(|a, b| a.name.cmp(&b.name));
    Discovered { agents, notices }
}

fn load_agents_from_dir(
    root: &PathBuf,
    source: AgentSource,
    out: &mut Vec<AgentDef>,
    notices: &mut Vec<String>,
) {
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(e) => {
            if !root_is_absent(root, &e) {
                notices.push(format!("agents: cannot read root {}: {e}", root.display()));
            }
            return;
        }
    };
    for e in entries {
        let e = match e {
            Ok(e) => e,
            Err(e) => {
                // An entry the directory could not hand over is something discovery could not
                // do, which is the whole subject of this function.
                notices.push(format!(
                    "agents: cannot read an entry under {}: {e}",
                    root.display()
                ));
                continue;
            }
        };
        let path = e.path();
        // pi: `.md` files only, files + symlinks, one bad file never takes down
        // the rest of the directory.
        if path.extension().map(|x| x == "md") != Some(true) {
            continue;
        }
        let raw = match std::fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(e) => {
                // `NotFound` is the entry vanishing between the listing and the read, which is a
                // race and not a state a person can act on. Everything else is a def somebody
                // wrote that will silently not load.
                if e.kind() != std::io::ErrorKind::NotFound {
                    notices.push(format!("agents: cannot read {}: {e}", path.display()));
                }
                continue;
            }
        };
        let Some(def) = parse_agent_def(&raw, source) else {
            notices.push(format!(
                "agents: ignoring {}: no name and description in the frontmatter, and both are \
                 required before a def is discovered",
                path.display()
            ));
            continue;
        };
        // Shadowing: replace the earlier-root def with the same name.
        if let Some(slot) = out.iter_mut().find(|a| a.name == def.name) {
            *slot = def;
            // **Only the winner is named, and that is a limit of the data, not of the channel.**
            // `AgentDef` carries no path (pi parity: a def is a prompt, not a handle to a file),
            // so the line cannot name the file it displaced. It names what won and which file it
            // came from, which is what a person searching their agents roots needs; inventing a
            // path field to complete the sentence would be a schema change beyond this fix.
            notices.push(format!(
                "agents: '{}' at {} shadows a definition of the same name from an earlier root; \
                 the later root wins",
                slot.name,
                path.display()
            ));
        } else {
            out.push(def);
        }
    }
}

/// Whether a root that failed to open is the ordinary "there are none here" case.
///
/// **The same predicate as `skills::root_is_absent`** (`skills.rs:242`), deliberately duplicated
/// rather than moved: that file belongs to another writer, and one shared helper in `paths` is
/// the right follow-up if a third copy ever appears — not a reason to edit a file mid-flight.
///
/// A link that reaches nothing is the opposite of absent: the agents are on disk and cannot be
/// read, and `read_dir` cannot tell it from a root that was never there. `crate::paths::is_link`
/// is the shared check because on Windows the answer that matters is the junction, which
/// `FileType::is_symlink()` does not report.
fn root_is_absent(root: &Path, err: &std::io::Error) -> bool {
    err.kind() == std::io::ErrorKind::NotFound && !crate::paths::is_link(root)
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

    /// Discovery returns the agents **sorted by name**, which is the property the
    /// task tool's description suffix used to print. `describe_task` — the sorted
    /// `- name: description` block appended to a tool description — went when
    /// nothing read it; the sort itself is `discover_agents`' own, and this is what
    /// keeps a reader from having to trust that.
    #[test]
    fn discovery_sorts_agents_by_name() {
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
        let names: Vec<&str> = agents.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, vec!["alpha", "worker"], "sorted by name");
    }

    /// **The defect this channel exists for: an unreadable root read as an empty one.**
    ///
    /// `load_agents_from_dir` used to `let Ok(entries) = read_dir(root) else { return }`, so a
    /// user whose agents never loaded was told nothing, and `reload: 0 agent(s)` is a count with
    /// no reason attached. A root that is a **file** is the portable way to make `read_dir` fail
    /// for a reason other than absence: a permission error needs an ACL and a privilege, and a
    /// test that needs either is a test that quietly asserts nothing.
    #[test]
    fn an_agents_root_that_cannot_be_read_names_the_root_and_the_reason() {
        let dir = tempfile::tempdir().unwrap();
        let not_a_root = dir.path().join("agents");
        std::fs::write(&not_a_root, "a file where an agents root should be\n").unwrap();

        let found = discover_agents_with_notices(std::slice::from_ref(&not_a_root), &[], true);
        assert!(
            found.agents.is_empty(),
            "there are genuinely no agents here; what is wrong is that nothing said so: {found:?}"
        );
        assert_eq!(found.notices.len(), 1, "one thing went wrong: {found:?}");

        // The whole lead, so a notice that named the root and no reason does not satisfy this.
        let notice = &found.notices[0];
        let prefix = format!("agents: cannot read root {}: ", not_a_root.display());
        let reason = notice.strip_prefix(prefix.as_str()).unwrap_or_else(|| {
            panic!("the notice must name the root AND say what went wrong; got {notice:?}")
        });
        assert!(
            !reason.trim().is_empty(),
            "a notice that names a root and no reason is the defect again: {notice:?}"
        );
    }

    /// **The negative control for the test above.**
    ///
    /// A root that does not exist is the ordinary case — most workspaces have no agents
    /// directory at all — so it must stay silent. Were it announced, the test above would pass
    /// for the wrong reason, and a channel that speaks on every launch is a channel people learn
    /// to stop reading.
    #[test]
    fn an_agents_root_that_is_simply_absent_is_not_a_notice() {
        let dir = tempfile::tempdir().unwrap();
        let found = discover_agents_with_notices(&[dir.path().join("no-such-agents")], &[], true);
        assert!(found.agents.is_empty());
        assert!(
            found.notices.is_empty(),
            "discovery genuinely completed and found nothing; that is not a problem to report: \
             {found:?}"
        );
    }

    /// **A well-formed tree pays nothing, and the wrapper still answers exactly as before.**
    ///
    /// Two halves: the notices are only worth reading if a healthy launch produces none, and the
    /// new return type must not have changed what loads or how it is ordered.
    #[test]
    fn a_clean_tree_is_silent_and_the_agents_that_load_are_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        write_agent(dir.path(), "scout.md", scout_md());
        write_agent(
            dir.path(),
            "alpha.md",
            "---\nname: alpha\ndescription: First\n---\nbody",
        );
        write_agent(dir.path(), "notes.txt", "not an agent def");
        let roots = [dir.path().to_path_buf()];

        let found = discover_agents_with_notices(&roots, &[], true);
        assert!(
            found.notices.is_empty(),
            "two well-formed defs and nothing to report: {found:?}"
        );
        assert_eq!(
            found.agents,
            discover_agents(&roots, &[], true),
            "the wrapper must not change the answer"
        );
        assert_eq!(
            found
                .agents
                .iter()
                .map(|a| a.name.as_str())
                .collect::<Vec<_>>(),
            vec!["alpha", "scout"],
            "still sorted by name"
        );
    }

    /// **The other half of the defect: a `.md` that is there and will not load.**
    ///
    /// Invalid UTF-8 is the portable way to produce it — the file exists, `read_to_string`
    /// refuses it, and before the notices channel the def the author wrote simply did not
    /// appear in the `task` tool's list. The good file beside it is the point: one bad file never
    /// takes down the directory (pi parity), and adding diagnostics must not turn that into one.
    #[test]
    fn an_agent_md_that_is_there_but_unreadable_is_announced_and_the_rest_still_load() {
        let dir = tempfile::tempdir().unwrap();
        write_agent(dir.path(), "scout.md", scout_md());
        std::fs::write(dir.path().join("broken.md"), [0xff, 0xfe, 0x00]).unwrap();
        let roots = [dir.path().to_path_buf()];

        let found = discover_agents_with_notices(&roots, &[], true);
        assert_eq!(
            found
                .agents
                .iter()
                .map(|a| a.name.as_str())
                .collect::<Vec<_>>(),
            vec!["scout"],
            "one bad file never takes down the directory: {found:?}"
        );
        assert_eq!(found.notices.len(), 1, "{found:?}");
        assert!(
            found.notices[0].contains(&dir.path().join("broken.md").display().to_string()),
            "the notice must name the file it could not read: {found:?}"
        );
        assert_eq!(
            found.agents,
            discover_agents(&roots, &[], true),
            "the failure changed nothing about what loaded"
        );
    }

    /// A `.md` the parser rejects is announced; a file that is not `.md` is not.
    ///
    /// Both halves matter. In an agents root a `.md` is a def by intent, so one with no `name`
    /// and `description` is a thing a person wrote that will never load and silence is what
    /// makes that unfixable. A notice for every unrelated file in the root is what makes people
    /// ignore the channel.
    #[test]
    fn a_rejected_agent_md_is_announced_and_another_extension_is_not() {
        let dir = tempfile::tempdir().unwrap();
        write_agent(dir.path(), "nofm.md", "just a body");
        write_agent(dir.path(), "nodesc.md", "---\nname: nodesc\n---\nbody");
        write_agent(
            dir.path(),
            "notes.txt",
            "---\nname: x\ndescription: y\n---\n",
        );

        let found = discover_agents_with_notices(&[dir.path().to_path_buf()], &[], true);
        assert!(found.agents.is_empty(), "both defs are rejected: {found:?}");
        assert_eq!(
            found.notices.len(),
            2,
            "the two .md files, and nothing for notes.txt: {found:?}"
        );
        for file in ["nofm.md", "nodesc.md"] {
            assert!(
                found.notices.iter().any(|n| n.contains(file)),
                "a def the parser rejects must be named: {found:?}"
            );
        }
    }

    /// A def that shadows an earlier root's is announced with the file that won.
    ///
    /// The winner is unchanged (later root, pi `discoverAgents` parity) — what changes is that
    /// a person reading `reload: 1 agent(s)` with two files on disk can see which one is live.
    #[test]
    fn a_shadowing_def_is_announced_with_the_file_that_won() {
        let global = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        write_agent(global.path(), "scout.md", scout_md());
        write_agent(
            project.path(),
            "scout.md",
            "---\nname: scout\ndescription: Project override\n---\n\nProject scout.\n",
        );

        let found = discover_agents_with_notices(
            &[global.path().to_path_buf()],
            &[project.path().to_path_buf()],
            true,
        );
        assert_eq!(found.agents.len(), 1, "one name, one winner: {found:?}");
        assert_eq!(
            found.agents[0].source,
            AgentSource::Project,
            "the recorded rule stands: the later root wins"
        );
        assert_eq!(found.notices.len(), 1, "one shadow, one line: {found:?}");
        assert!(
            found.notices[0].contains(&project.path().join("scout.md").display().to_string()),
            "the notice must name the file that won: {found:?}"
        );
        assert!(
            found.notices[0].contains("later root wins"),
            "and say which side won: {found:?}"
        );
    }

    /// **A root that is a link to nothing is not "absent".**
    ///
    /// `read_dir` answers `NotFound` for a dangling link and for a root that was never there, so
    /// without the link check the most confusing case — agents on disk, unreachable — is the one
    /// that stays silent. The link is made while its target exists, because a junction to a
    /// missing directory cannot be created at all on Windows.
    #[test]
    fn an_agents_root_that_is_a_link_to_nothing_is_announced_rather_than_read_as_absent() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("real-agents");
        std::fs::create_dir_all(&target).unwrap();
        write_agent(&target, "scout.md", scout_md());
        let link = dir.path().join("linked-agents");
        assert!(
            crate::test_support::dir_link(&target, &link),
            "the platform refused to create a directory link, so this test asserted nothing"
        );

        // The control: a live link finds the def and reports nothing.
        let live = discover_agents_with_notices(std::slice::from_ref(&link), &[], true);
        assert_eq!(
            live.agents.len(),
            1,
            "a working link is not a problem: {live:?}"
        );
        assert!(live.notices.is_empty(), "{live:?}");

        std::fs::remove_dir_all(&target).unwrap();
        // Pin the platform fact the branch depends on. If a future platform answers
        // something else here, the test says so instead of passing on another path.
        let err = std::fs::read_dir(&link).expect_err("the link now reaches nothing");
        assert_eq!(
            err.kind(),
            std::io::ErrorKind::NotFound,
            "this is the error the link check exists to tell apart from an absent root: {err}"
        );

        let dangling = discover_agents_with_notices(std::slice::from_ref(&link), &[], true);
        assert!(
            dangling.agents.is_empty(),
            "nothing is reachable: {dangling:?}"
        );
        assert_eq!(
            dangling.notices.len(),
            1,
            "one thing went wrong: {dangling:?}"
        );
        assert!(
            dangling.notices[0].contains(&link.display().to_string()),
            "the notice must name the root: {dangling:?}"
        );
    }
}
