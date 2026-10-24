//! Delegated tasks: child agents, the registry that keeps them, and the
//! tool surface that reports them.

use std::sync::Arc;

use epsilon_contracts::agent_event::{AgentEvent, TurnEndReason};
use epsilon_contracts::message::Message;

use crate::{Agent, LoopLimits, RunOutcome, SessionStore, contain_panic};

impl Agent {
    pub(crate) fn clone_task_host(self: &Arc<Self>) -> Arc<dyn epsilon_contracts::tool::TaskHost> {
        Arc::clone(self) as Arc<dyn epsilon_contracts::tool::TaskHost>
    }
}

/// One tracked delegated task (M53): the child stays alive between calls so a
/// later `task` request can resume it, and the tool layer can report status.
pub(crate) struct TaskEntry {
    id: String,
    description: String,
    subagent_type: String,
    /// Effective model for this child (M56) — see [`Agent::build_task_child`].
    model: Option<String>,
    pub(crate) state: epsilon_contracts::tool::TaskState,
    output: String,
    turns: u32,
    usage: Option<epsilon_contracts::message::Usage>,
    /// Whether the current `usage` has already been folded into the parent.
    ///
    /// A background task accumulates usage in this registry; the tool result is
    /// how the parent's accounting sees it. Without this bit every status poll
    /// re-billed the same leg.
    usage_reported: bool,
    pub(crate) child: Arc<Agent>,
}

/// How many tasks the registry keeps before evicting the oldest finished one
/// (a long session must not accumulate live children forever).
const TASK_REGISTRY_CAP: usize = 16;

impl Agent {
    /// Build the child agent for a `task` delegation: agent-type routing
    /// (def lookup, tool subset, model/thinking inheritance, skills, role
    /// brief) plus the depth stamp that stops nested fan-out.
    ///
    /// M56: returns the effective model alongside the child. Precedence is
    /// `def.model` (the operator's configuration) → `request_model` (the call's
    /// own choice) → inherited parent model, matching opencode's
    /// `next.model ?? parent` with the operator's config kept on top.
    fn build_task_child(
        &self,
        subagent_type: &str,
        request_model: Option<&str>,
    ) -> Result<(Arc<Agent>, Option<String>), String> {
        // Child agent shares the same provider/gate/workspace but has an
        // isolated transcript (no steering/follow-up carry-over). Caps are
        // modest so one delegated task cannot exhaust the parent's budget.
        let limits = LoopLimits {
            max_turns: 32,
            ..Default::default()
        };
        // M50: agent-type routing (pi `agents.ts` + opencode `Agent.get`
        // parity). `default` (or empty) means "no def matched": full registry,
        // inherited model, no extra prompt — exactly the old behavior. A named
        // type MUST resolve, or the failure names what's available instead of
        // silently running the wrong configuration.
        let agents = self.agent_list();
        let def = if subagent_type.trim().is_empty() || subagent_type == "default" {
            None
        } else {
            match agents.iter().find(|a| a.name == subagent_type) {
                Some(d) => Some(d.clone()),
                None => {
                    let mut names: Vec<&str> = agents.iter().map(|a| a.name.as_str()).collect();
                    names.sort_unstable();
                    let available = if names.is_empty() {
                        "none — no agent defs discovered".to_string()
                    } else {
                        names.join(", ")
                    };
                    return Err(format!(
                        "unknown agent type: \"{subagent_type}\" is not a valid agent type. Available: {available}"
                    ));
                }
            }
        };
        // M50: depth cap (opencode `subagent_depth` parity, default 1). A child
        // at the cap runs with a registry whose `task` tool keeps working for
        // display but whose host refuses — nested fan-out fails honestly
        // instead of recursing the loop.
        const MAX_TASK_DEPTH: usize = 1;
        let depth = self.task_depth.lock().map(|s| *s).unwrap_or(0);
        if depth >= MAX_TASK_DEPTH {
            return Err(format!(
                "subagent depth limit reached ({MAX_TASK_DEPTH}). Increase the limit to allow nested subagents."
            ));
        }
        let stream_fn = self
            .stream_fn
            .read()
            .map(|g| std::sync::Arc::clone(&*g))
            .unwrap_or_else(|e| std::sync::Arc::clone(&*e.into_inner()));
        let gate: std::sync::Arc<dyn crate::gate::PermissionGate> = self
            .gate
            .child_gate()
            .unwrap_or_else(|| std::sync::Arc::clone(&self.gate));
        // Tool subset: a def WITH a tools list sees only those tools (pi
        // scout/planner/reviewer parity); no list (or no def) = full registry.
        let tools = match def.as_ref().and_then(|d| d.tools.as_ref()) {
            Some(names) => std::sync::Arc::new(self.tools.subset(names)),
            None => std::sync::Arc::clone(&self.tools),
        };
        let child = Agent::new(limits, stream_fn, tools, gate, self.workspace_root.clone());
        // Model: def override wins, then the call's request, else inherit (pi:
        // omitted model inherits the dispatching session's model + thinking
        // level). The resolved id is reported back so the tool result, the
        // registry and `/tasks` all name the model that actually ran.
        let model = if let Some(m) = def.as_ref().and_then(|d| d.model.as_ref()) {
            child.set_model_id(m.clone());
            Some(m.clone())
        } else if let Some(requested) = request_model {
            let resolved = Self::resolve_task_model(requested)?;
            child.set_model_id(resolved.clone());
            Some(resolved)
        } else if let Ok(m) = self.model_id.lock() {
            child.set_model_id(m.clone());
            Some(m.clone()).filter(|s| !s.trim().is_empty())
        } else {
            None
        };
        child.set_thinking_level(self.thinking_level());
        // **The parent's hooks, for the same reason the gate is derived and not shared.**
        //
        // `Agent::new` starts every agent on `NoopToolHooks`/`NoopTurnHooks`, and nothing here
        // ever replaced them — so an ADR-019 `before_tool_call` that exists to **vet or block** a
        // tool call was installed on the parent (`epsilon-runtime/src/lib.rs:1318`) and reached
        // no child at all. A hook that blocks `bash` blocked it for the top-level agent and let
        // every subagent run it, silently. That is the `AGENTS.md` §2.7 shape — a control the
        // operator installed that does not act on a real path — and it is the same class as the
        // M300 permission leak through a second door.
        //
        // **Inherited rather than derived, and that is the difference from the gate.** The gate
        // needed a fresh memory because a *session's remembered allow* is a decision about one
        // conversation. A hook is not a decision about a conversation: it is operator policy —
        // the extension's own veto — and policy that applied to the agent and not to the agents
        // it spawns is the defect, not the design. There is nothing in a hook to un-remember, so
        // the parent's `Arc` is handed over as-is. Both accessors clone an `Arc`, so this costs
        // two refcounts per delegated task.
        child.set_tool_hooks(self.tool_hooks());
        child.set_turn_hooks(self.turn_hooks());
        // Child discovers the same skill catalog (read-only share).
        child.set_skills(self.skill_list());
        child.set_disabled_skills(self.disabled_skills());
        // ...and the same identity overlays, for the same reason: the workspace's
        // `SYSTEM.md` is the contract the user wrote for this repository, not a property of
        // the top-level session.
        child.set_system_overlay(self.system_overlay());
        // Child inherits agent defs (so ITS error messages name the same
        // types) at one deeper depth.
        child.set_agents(self.agent_list());
        if let Ok(mut slot) = child.task_depth_slot() {
            *slot = depth + 1;
        }
        // System prompt: the def body (pi: agent file body past frontmatter)
        // becomes a CONTEXT-tier project instruction so the child knows its
        // role while the task stays the user message. No def (or an empty
        // body) = bare prompt, exactly the old behavior.
        if let Some(body) = def.as_ref().and_then(|d| {
            if d.system_prompt.trim().is_empty() {
                None
            } else {
                Some(d.system_prompt.clone())
            }
        }) {
            child.set_agent_brief(body);
        }
        Ok((child, model))
    }

    /// M58: give a child its own durable session file beside the parent's, so a
    /// `task_id` survives a restart (the child's file is a session: header with
    /// `parent_session`, a `task_meta` entry the rehydrator reads back).
    ///
    /// Returns the file path, or `None` when the parent has no store attached —
    /// children then live in memory only, exactly the pre-M58 behavior (tests
    /// and embedding callers keep working unchanged).
    fn attach_child_session(
        &self,
        child: &Arc<Agent>,
        task_id: &str,
        description: &str,
        subagent_type: &str,
        model: Option<&str>,
    ) -> Option<std::path::PathBuf> {
        use epsilon_contracts::session_entry::EntryKind;
        let parent = self.store()?;
        let (dir, stem, parent_id) = {
            let st = parent.lock().ok()?;
            let path = st.path().to_path_buf();
            (
                path.parent()?.to_path_buf(),
                path.file_stem()?.to_string_lossy().to_string(),
                st.header().map(|h| h.id.clone()),
            )
        };
        let file = dir.join(format!("{stem}-{task_id}.jsonl"));
        // **Both failures on this path are reported, not printed (M311).** These were an
        // `eprintln!` and a `let _ =`, which is the same defect twice: a window has no console,
        // so `eprintln!` reaches nobody, and a discarded `append` reaches nobody by definition.
        // The `RunParams::notices` sink that fixed sixteen `eprintln!` lines one layer up
        // (`epsilon-runtime`) does not exist here — but an `AgentEvent::Notice` does, and every
        // front end already renders those, so this needs no new plumbing.
        // `parent_id` is `None` for a parent with no header; the original `parent_id?` returned
        // early there, and clippy is right that it still reads better than an explicit `let else`
        // — the two failure paths below report *after* this point, so nothing is lost by it.
        let parent_id = parent_id?;
        let mut store = match SessionStore::open_child_in(&file, &self.workspace_root, &parent_id) {
            Ok(s) => s,
            Err(e) => {
                self.emit(AgentEvent::Notice {
                    text: format!(
                        "task '{task_id}': could not open its own session file ({}): {e} — this \
                         subagent runs in memory only and will not be resumable",
                        file.display()
                    ),
                });
                return None;
            }
        };
        if let Err(e) = store.append(EntryKind::Custom {
            custom_type: "task_meta".to_string(),
            data: Some(serde_json::json!({
                "task_id": task_id,
                "description": description,
                "subagent_type": subagent_type,
                "model": model,
            })),
        }) {
            // The file exists and is attached, but it carries no `task_meta` — so a later
            // `/tasks` resume would find a session it cannot rehydrate, silently.
            self.emit(AgentEvent::Notice {
                text: format!(
                    "task '{task_id}': its session file was created but the resume record could \
                     not be written ({e}) — this task will not be resumable after a restart"
                ),
            });
        }
        child.attach_session_store(Arc::new(std::sync::Mutex::new(store)));
        Some(file)
    }

    /// M58: rebuild a child from its persisted session file, for a `task_id`
    /// that outlived the process that created it.
    ///
    /// `Ok(None)` means "no such child on disk" — the caller keeps its honest
    /// unknown-id error. A file that exists but cannot be read is an error, not
    /// a silent miss: the caller asked for a specific task.
    fn rehydrate_task(
        &self,
        id: &str,
    ) -> Result<Option<epsilon_contracts::tool::TaskSnapshot>, String> {
        use epsilon_contracts::session_entry::EntryKind;
        let Some(dir) = self
            .session_path()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        else {
            return Ok(None);
        };
        let file = std::fs::read_dir(&dir)
            .ok()
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .map(|e| e.path())
            .find(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.ends_with(&format!("-{id}.jsonl")))
            });
        let Some(file) = file else {
            return Ok(None);
        };

        // Open + repair exactly like a live resume, so a crash mid-turn does
        // not make a child unresumable.
        let mut store =
            SessionStore::open(&file).map_err(|e| format!("open {}: {e}", file.display()))?;
        // **The repair is reported, not discarded.**
        //
        // This was a `let _ =`, and it was the one path guaranteed to encounter a repair:
        // a resumed child is the session that was interrupted, which is exactly the file
        // `repair_on_open` exists for. Every other open path says something — the runtime's
        // resume puts each repaired kind into its notices, and `attach_session_store` keeps
        // the report on `last_repair` — so this was the third place and the silent one.
        //
        // What the silence costs is the whole point of repairing. A dangling tool call
        // means work the model asked for never happened, and the child comes back with a
        // silently edited transcript: the operator who resumed a task after a crash had no
        // way to know the resume changed anything, or that the interrupted turn was closed
        // out rather than answered. The kinds are named rather than counted, because "a
        // call had no result" and "the session ended mid-run" are different things to have
        // lost (M75) and an operator who cannot tell them apart cannot decide anything.
        match epsilon_session::repair::repair_on_open(&mut store) {
            Ok(report) => {
                for line in report.summary() {
                    self.emit(AgentEvent::Notice {
                        text: format!("task '{id}': resumed its session file and repaired {line}"),
                    });
                }
            }
            Err(e) => {
                self.emit(AgentEvent::Notice {
                    text: format!(
                        "task '{id}': its session file could not be repaired ({e}) — the \
                         resumed child's transcript may be missing work"
                    ),
                });
            }
        }
        let entries = store
            .read_all()
            .map_err(|e| format!("read {}: {e}", file.display()))?;

        // Meta comes from the LAST `task_meta` entry (a resumed child rewrites
        // nothing, so the first is usually it too).
        let mut description = String::new();
        let mut subagent_type = "default".to_string();
        let mut model: Option<String> = None;
        for e in &entries {
            if let EntryKind::Custom {
                custom_type,
                data: Some(data),
            } = &e.kind
                && custom_type == "task_meta"
            {
                if let Some(d) = data.get("description").and_then(|v| v.as_str()) {
                    description = d.to_string();
                }
                if let Some(t) = data.get("subagent_type").and_then(|v| v.as_str()) {
                    subagent_type = t.to_string();
                }
                if let Some(m) = data.get("model").and_then(|v| v.as_str()) {
                    model = Some(m.to_string());
                }
            }
        }
        if description.is_empty() {
            description = id.to_string();
        }

        // Settled state from the last turn's ending, when the file has one.
        let ended = entries.iter().rev().find_map(|e| match &e.kind {
            EntryKind::TurnEnd { reason } => Some(reason.clone()),
            _ => None,
        });
        let failed = matches!(
            ended,
            Some(TurnEndReason::Error(_))
                | Some(TurnEndReason::Aborted)
                | Some(TurnEndReason::BudgetExhausted)
                | Some(TurnEndReason::Blocked)
        );

        let messages: Vec<Message> = store
            .model_view()
            .map_err(|e| format!("read {}: {e}", file.display()))?
            .into_iter()
            .map(|v| v.into_message())
            .collect();
        let turns = messages
            .iter()
            .filter(|m| matches!(m, Message::Assistant(_)))
            .count() as u32;
        let last_assistant = messages.iter().rev().find_map(|m| match m {
            Message::Assistant(a) => Some(a),
            _ => None,
        });
        let mut output = match ended {
            Some(TurnEndReason::Error(ref e)) => {
                format!("[restored from disk] run ended in error: {e}")
            }
            Some(TurnEndReason::Aborted) => "[restored from disk] run was aborted".to_string(),
            _ => last_assistant
                .map(|a| a.text())
                .filter(|t| !t.trim().is_empty())
                .unwrap_or_else(|| format!("[restored from disk: {} message(s)]", messages.len())),
        };
        let usage = last_assistant.map(|a| a.usage);
        let state = if failed {
            epsilon_contracts::tool::TaskState::Failed
        } else {
            epsilon_contracts::tool::TaskState::Completed
        };

        // A stored model can have left the catalog since the child ran (models
        // retire): restoring the transcript still matters more than honoring the
        // old model, so fall back to inheritance and say so in the report.
        let (child, effective_model) = match self.build_task_child(&subagent_type, model.as_deref())
        {
            Ok(built) => built,
            Err(_retired) => {
                let (child, inherited) = self.build_task_child(&subagent_type, None)?;
                output.push_str(&format!(
                    "\n[restored on the inherited model: {}]",
                    inherited.as_deref().unwrap_or("(session default)")
                ));
                (child, inherited)
            }
        };
        child.restore_history(messages);
        child.attach_session_store(Arc::new(std::sync::Mutex::new(store)));
        self.register_task_entry(TaskEntry {
            id: id.to_string(),
            description,
            subagent_type,
            model: effective_model,
            state,
            output,
            turns,
            usage,
            // A rehydrated child's spend belongs to this parent's session, and
            // this parent has not billed it yet.
            usage_reported: false,
            child,
        });
        Ok(self.task_snapshot(id))
    }

    /// Resolve a per-delegation model id against the catalog.
    ///
    /// Deliberately stricter than `resolve_model`'s partial rule: a delegated
    /// run must never silently swap models, so only exact, `provider/id`, and
    /// case-insensitive matches are accepted — a partial match is *offered* as
    /// a suggestion instead of used.
    fn resolve_task_model(raw: &str) -> Result<String, String> {
        let raw = raw.trim();
        if raw.is_empty() {
            return Err("task: model must not be empty (omit it to inherit)".to_string());
        }
        match epsilon_llm::catalog::resolve_model(raw) {
            Some(r) if r.kind != epsilon_llm::catalog::ModelMatchKind::Partial => {
                Ok(r.info.id.to_string())
            }
            Some(r) => Err(format!(
                "task: model \"{raw}\" only partially matches \"{}\", and a delegated run never swaps models on a guess — pass the full id if that is the one you meant",
                r.info.id
            )),
            None => Err(format!(
                "task: unknown model \"{raw}\" — use a catalog id (`epsilon --list-models`, or the /model picker)"
            )),
        }
    }

    /// Collapse a finished child run into the envelope text + turn count.
    fn summarize_task_output(
        subagent_type: &str,
        description: &str,
        model: Option<&str>,
        produced: &[Message],
    ) -> (String, u32) {
        let turns = produced
            .iter()
            .filter(|m| matches!(m, epsilon_contracts::message::Message::Assistant(_)))
            .count() as u32;
        let mut parts: Vec<String> = Vec::new();
        for m in produced {
            match m {
                epsilon_contracts::message::Message::Assistant(a) => {
                    let t = a.text();
                    if !t.trim().is_empty() {
                        parts.push(t);
                    }
                }
                epsilon_contracts::message::Message::ToolResult(r) => {
                    let preview = r
                        .content
                        .iter()
                        .filter_map(|c| match c {
                            epsilon_contracts::content::ContentBlock::Text { text } => {
                                Some(text.as_str())
                            }
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    if !preview.trim().is_empty() {
                        parts.push(format!(
                            "[{}] {}",
                            r.tool_name,
                            preview.chars().take(4000).collect::<String>()
                        ));
                    }
                }
                _ => {}
            }
        }
        if parts.is_empty() {
            let on_model = model.map(|m| format!(" on {m}")).unwrap_or_default();
            parts.push(format!(
                "[subagent {subagent_type}{on_model}: {description} — no textual output]"
            ));
        }
        let text = parts.join("\n\n---\n\n");
        (text, turns)
    }

    /// The user message a delegated prompt travels in.
    fn task_user_message(prompt: &str) -> Message {
        use epsilon_contracts::content::UserContent;
        use epsilon_contracts::message::UserMessage;
        epsilon_contracts::message::Message::User(UserMessage {
            id: epsilon_contracts::new_id("msg"),
            timestamp_ms: crate::now_ms(),
            content: vec![UserContent::Text {
                text: prompt.to_string(),
            }],
        })
    }

    /// The user-shaped notice the parent model sees when a background child
    /// finishes (delivered through the steering drain point).
    fn task_notice(
        id: &str,
        description: &str,
        state: &str,
        model: Option<&str>,
        body: &str,
    ) -> Message {
        // M56: the notice names the model the child ran on — the parent (and
        // the transcript) must be able to tell a scout on a cheap model from
        // the session's own model without asking.
        let on_model = model.map(|m| format!(" model=\"{m}\"")).unwrap_or_default();
        let text = format!(
            "<task id=\"{id}\" state=\"{state}\" description=\"{description}\"{on_model}>\n<task_result>\n{body}\n</task_result>\n</task>\n[background task {state} — resume it with task_id \"{id}\" if you need more]"
        );
        Self::task_user_message(&text)
    }

    /// Insert a new task, evicting the oldest finished entry at the cap.
    fn register_task_entry(&self, entry: TaskEntry) {
        // **A subagent's work is visible while it runs, not only when it lands.**
        //
        // Every `Agent` builds its own event channel (`Agent::new`, `lib.rs:856`) and
        // nothing bridged a child's to the parent's, so a delegated subagent was a black
        // box: for the whole of a five-minute delegation the parent's subscribers saw
        // nothing — no notice, no tool starting, no progress — and learned only the
        // settled summary. The settle notice (`task_notice`) is what makes this easy to
        // miss, because something *does* arrive at the end: the run is invisible in the
        // middle rather than absent, which is the shape that reads as a hang.
        //
        // The forwarded set is deliberately not the whole enum, and the boundary is the
        // design. `ToolExecution*` and `Notice` are forwarded because the terminal's
        // reducer keys on `tool_call_id` and does nothing when the id is unknown
        // (`epsilon-tui/src/state.rs:1533-1540`) — so a subagent's calls set the status
        // line and add no transcript rows, and a notice is already "a transient status
        // line, deliberately NOT part of the transcript". What is **not** forwarded is
        // `MessageEnd`/`AgentEnd`: they would put a subagent's messages in the operator's
        // transcript *and* re-record its usage through `record_usage`, which the
        // tool-result path already folds once (`record_tool_usage`) — so forwarding them
        // would bill every delegated task twice.
        //
        // The forwarder holds the child's `Receiver` and the parent's `Sender`, never the
        // child's `Arc`. A tokio broadcast `Receiver` does not keep its channel open, so
        // when the registry drops this entry the child's `Sender` drops with it, the
        // channel closes, and this task ends instead of looping for ever. Dropping the
        // `JoinHandle` detaches a tokio task without aborting it, which is exactly what is
        // wanted here.
        let mut forwarding = entry.child.subscribe();
        let parent_tx = self.events_tx.clone();
        tokio::spawn(async move {
            loop {
                match forwarding.recv().await {
                    Ok(ev) => {
                        let forwarded = match &*ev {
                            AgentEvent::Notice { .. }
                            | AgentEvent::ToolExecutionStart { .. }
                            | AgentEvent::ToolExecutionUpdate { .. }
                            | AgentEvent::ToolExecutionEnd { .. } => Some((*ev).clone()),
                            _ => None,
                        };
                        if let Some(ev) = forwarded {
                            let _ = parent_tx.send(Arc::new(ev));
                        }
                    }
                    // Lagging is a dropped event, not a closed channel: the child is still
                    // running and the next one is worth having.
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => break,
                }
            }
        });
        if let Ok(mut v) = self.tasks.lock() {
            if v.len() >= TASK_REGISTRY_CAP
                && let Some(pos) = v
                    .iter()
                    .position(|e| e.state != epsilon_contracts::tool::TaskState::Running)
            {
                v.remove(pos);
            }
            v.push(entry);
        }
    }

    /// Update one tracked task in place; unknown ids are ignored.
    fn update_task_entry(&self, id: &str, f: impl FnOnce(&mut TaskEntry)) {
        if let Ok(mut v) = self.tasks.lock()
            && let Some(entry) = v.iter_mut().find(|e| e.id == id)
        {
            f(entry);
        }
    }

    /// Snapshot one tracked task by id.
    pub fn task_snapshot(&self, id: &str) -> Option<epsilon_contracts::tool::TaskSnapshot> {
        let v = self.tasks.lock().ok()?;
        v.iter().find(|e| e.id == id).map(Self::snapshot_of)
    }

    /// Take the usage a child has accumulated and not yet had billed.
    ///
    /// The first caller gets it and marks it reported; every later status poll
    /// gets `None`. The snapshot keeps its copy for display, so this does not
    /// hide the child's spend from the UI.
    pub fn take_task_usage(&self, id: &str) -> Option<epsilon_contracts::message::Usage> {
        let mut tasks = self.tasks.lock().ok()?;
        let entry = tasks.iter_mut().find(|entry| entry.id == id)?;
        if entry.usage_reported {
            return None;
        }
        let usage = entry.usage?;
        entry.usage_reported = true;
        Some(usage)
    }

    /// Every tracked task, in dispatch order (UI badges, tests).
    pub fn task_snapshots(&self) -> Vec<epsilon_contracts::tool::TaskSnapshot> {
        self.tasks
            .lock()
            .map(|v| v.iter().map(Self::snapshot_of).collect())
            .unwrap_or_default()
    }

    /// Ids still tracked, in dispatch order (honest unknown-id errors).
    pub fn task_ids(&self) -> Vec<String> {
        self.tasks
            .lock()
            .map(|v| v.iter().map(|e| e.id.clone()).collect())
            .unwrap_or_default()
    }

    fn snapshot_of(entry: &TaskEntry) -> epsilon_contracts::tool::TaskSnapshot {
        epsilon_contracts::tool::TaskSnapshot {
            id: entry.id.clone(),
            description: entry.description.clone(),
            subagent_type: entry.subagent_type.clone(),
            model: entry.model.clone(),
            state: entry.state.clone(),
            output: entry.output.clone(),
            turns: entry.turns,
            usage: entry.usage,
        }
    }

    /// M93/F18: claim a tracked task for a new run, in ONE lock hold.
    ///
    /// The check-then-set this replaced was two acquisitions: two racing resumes both read
    /// the settled snapshot, both saw a non-running state — before either had written
    /// anything — and both started a leg on the same child. Doing the find, the refusal and
    /// the state change under a single hold makes the claim atomic, so exactly one caller
    /// can win; the losers get the same honesty a status probe would.
    ///
    /// Errors describe the two states a caller can meet, and leave the entry untouched: an
    /// id that is no longer tracked (evicted, or never registered) and a child already
    /// running. Callers invoke this after their last fallible lookup, so a failure can never
    /// leave a phantom `Running` behind.
    fn claim_task_running(&self, id: &str) -> Result<(), String> {
        let mut v = self
            .tasks
            .lock()
            .map_err(|_| "task registry lock poisoned".to_string())?;
        let Some(entry) = v.iter_mut().find(|e| e.id == id) else {
            return Err(format!("task '{id}' is no longer tracked"));
        };
        if entry.state == epsilon_contracts::tool::TaskState::Running {
            return Err(format!(
                "task '{id}' is still running — poll it with task_id and no prompt, or wait for its completion notice"
            ));
        }
        entry.state = epsilon_contracts::tool::TaskState::Running;
        Ok(())
    }

    /// Run, resume, or report a delegated task (M53 wires `task`'s
    /// `background` and `task_id` parameters here).
    ///
    /// - fresh + foreground: build a child, await it, return its envelope;
    /// - fresh + background: register, spawn the run, return `Running` at once;
    /// - `task_id` + prompt: continue the same child (its history is intact);
    /// - `task_id` + empty prompt: status report, never a new run.
    pub async fn delegate_task(
        &self,
        request: epsilon_contracts::tool::TaskRequest,
    ) -> Result<epsilon_contracts::tool::TaskOutcome, String> {
        let prompt = request.prompt.trim().to_string();
        let request_model = request
            .model
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let resume = request
            .resume
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        if let Some(id) = resume {
            // M58: a task from a previous run is not tracked in memory, but its
            // session file is on disk — rehydrate instead of failing.
            let snapshot = match self.task_snapshot(&id) {
                Some(snapshot) => snapshot,
                None => match self.rehydrate_task(&id)? {
                    Some(snapshot) => snapshot,
                    None => {
                        let known = self.task_ids();
                        return Err(if known.is_empty() {
                            format!(
                                "unknown task id '{id}' — no such task is tracked or on disk (ids come from a `task` call's result)"
                            )
                        } else {
                            format!("unknown task id '{id}' — tracked: {}", known.join(", "))
                        });
                    }
                },
            };
            if prompt.is_empty() {
                return Ok(epsilon_contracts::tool::TaskOutcome::Running(snapshot));
            }
            let child = self
                .tasks
                .lock()
                .ok()
                .and_then(|v| v.iter().find(|e| e.id == id).map(|e| Arc::clone(&e.child)))
                .ok_or_else(|| format!("task '{id}' is no longer tracked"))?;
            // M93/F18: the claim IS the refusal — one lock hold, after the last fallible
            // lookup, so a second resume is turned away while the first leg is in flight and
            // a failed lookup never leaves a phantom `Running` behind.
            self.claim_task_running(&id)?;
            if request.background {
                self.spawn_task_run(
                    id.clone(),
                    snapshot.description.clone(),
                    snapshot.subagent_type.clone(),
                    snapshot.model.clone(),
                    Arc::clone(&child),
                    prompt,
                );
                let now = self
                    .task_snapshot(&id)
                    .ok_or_else(|| "task entry vanished while resuming".to_string())?;
                return Ok(epsilon_contracts::tool::TaskOutcome::Running(now));
            }
            return self
                .await_task_run(
                    id,
                    snapshot.subagent_type.clone(),
                    snapshot.model.clone(),
                    prompt,
                    Arc::clone(&child),
                )
                .await;
        }
        let id = epsilon_contracts::new_id("task");
        let description = request.description.clone();
        let subagent_type = request.subagent_type.clone();
        let (child, model) =
            self.build_task_child(&request.subagent_type, request_model.as_deref())?;
        // M58: persist the child as its own session file (before it runs, so a
        // crash mid-turn still leaves a resumable task).
        self.attach_child_session(&child, &id, &description, &subagent_type, model.as_deref());
        self.register_task_entry(TaskEntry {
            id: id.clone(),
            description: description.clone(),
            subagent_type: subagent_type.clone(),
            model: model.clone(),
            state: epsilon_contracts::tool::TaskState::Running,
            output: String::new(),
            turns: 0,
            usage: None,
            usage_reported: false,
            child: Arc::clone(&child),
        });
        if request.background {
            self.spawn_task_run(
                id.clone(),
                description,
                subagent_type,
                model,
                Arc::clone(&child),
                prompt,
            );
            let now = self
                .task_snapshot(&id)
                .ok_or_else(|| "task entry vanished while starting".to_string())?;
            return Ok(epsilon_contracts::tool::TaskOutcome::Running(now));
        }
        self.await_task_run(id, subagent_type, model, prompt, child)
            .await
    }

    /// A child run that ended in an in-band provider failure (the never-fail
    /// contract delivers those as an assistant message with a failure stop
    /// reason, so `run()` still returns Ok).
    fn task_run_failure(outcome: &RunOutcome) -> Option<String> {
        // An armed abort returns before any message exists; the reason is the
        // only evidence. Check it first, then the in-band failure message.
        if outcome.reason == TurnEndReason::Aborted {
            return Some("subagent run aborted".to_string());
        }
        let produced = &outcome.produced;
        produced.iter().rev().find_map(|m| match m {
            Message::Assistant(a) if a.stop_reason.is_failure() => Some(
                if a.stop_reason == epsilon_contracts::message::StopReason::Aborted {
                    "subagent run aborted".to_string()
                } else {
                    a.error_message
                        .clone()
                        .unwrap_or_else(|| format!("subagent stopped: {:?}", a.stop_reason))
                },
            ),
            _ => None,
        })
    }

    /// Foreground path: await the child, record it, envelope the result.
    ///
    /// M93/F18: the claim — or the fresh register — writes `Running` before this body runs,
    /// so exactly like the detached body it must always end in a terminal record. The
    /// fallible work is contained the way `contain_panic` contains the detached body, so a
    /// panic here lands the entry `Failed` naming the panic instead of stranding the
    /// just-claimed entry `Running` forever.
    async fn await_task_run(
        &self,
        id: String,
        subagent_type: String,
        model: Option<String>,
        prompt: String,
        child: Arc<Agent>,
    ) -> Result<epsilon_contracts::tool::TaskOutcome, String> {
        let description = self
            .task_snapshot(&id)
            .map(|s| s.description)
            .unwrap_or_default();
        let panic_id = id.clone();
        contain_panic(
            async {
                match child.run(Self::task_user_message(&prompt)).await {
                    Ok(outcome) => {
                        // In-band failures are still failures: a subagent that died on
                        // a provider error must not be reported as a completed task.
                        if let Some(reason) = Self::task_run_failure(&outcome) {
                            let message = format!("subagent run failed: {reason}");
                            self.update_task_entry(&id, |entry| {
                                entry.state = epsilon_contracts::tool::TaskState::Failed;
                                entry.output = message.clone();
                            });
                            return Err(message);
                        }
                        let (text, turns) = Self::summarize_task_output(
                            &subagent_type,
                            &description,
                            model.as_deref(),
                            &outcome.produced,
                        );
                        self.update_task_entry(&id, |e| {
                            e.state = epsilon_contracts::tool::TaskState::Completed;
                            e.output = text.clone();
                            // Cumulative: a resumed child keeps counting its turns.
                            e.turns += turns;
                            e.usage = Some(outcome.usage);
                            // The foreground result below carries this usage
                            // to the parent in the same call.
                            e.usage_reported = true;
                        });
                        Ok(epsilon_contracts::tool::TaskOutcome::Completed(
                            epsilon_contracts::tool::TaskHostResult {
                                text,
                                usage: Some(outcome.usage),
                                details: serde_json::json!({
                                    "task_id": id,
                                    "subagent_type": subagent_type,
                                    "model": model,
                                    "description": description,
                                    "state": "completed",
                                    "turns": turns,
                                    "produced": outcome.produced.len(),
                                    "reason": format!("{:?}", outcome.reason),
                                }),
                            },
                        ))
                    }
                    Err(e) => {
                        let message = format!("task host run failed: {e}");
                        self.update_task_entry(&id, |entry| {
                            entry.state = epsilon_contracts::tool::TaskState::Failed;
                            entry.output = message.clone();
                        });
                        Err(message)
                    }
                }
            },
            move |panic| {
                // The claim is now paired: the foreground leg lands the same truthful record
                // the detached body's panic arm writes.
                let message = format!("subagent run failed: the run panicked ({panic})");
                self.update_task_entry(&panic_id, |entry| {
                    entry.state = epsilon_contracts::tool::TaskState::Failed;
                    entry.output = message.clone();
                });
                Err(message)
            },
        )
        .await
    }

    /// Background path: detach the run, record its result, queue the notice.
    fn spawn_task_run(
        &self,
        id: String,
        description: String,
        subagent_type: String,
        model: Option<String>,
        child: Arc<Agent>,
        prompt: String,
    ) {
        let tasks = Arc::clone(&self.tasks);
        let notices = self.task_notices_tx.clone();
        let notice_id = id.clone();
        // The parent may outlive the dispatch call; keep a weak upgrade rather
        // than a strong Arc cycle. If the parent is gone, the entry keeps the
        // usage for a later poll/rehydrate.
        let parent = self.self_ref.get().and_then(|weak| weak.upgrade());
        tokio::spawn(async move {
            // M93/F18: from the resume claim (and the fresh register) onward the entry says
            // `Running`, so this body must always end with a terminal record — including when
            // the run itself unwinds. The fallible work is contained exactly like a
            // panicking turn hook (`contain_panic`, the std-only `poll_fn` + `catch_unwind`
            // boundary), so a panic lands as a failed record naming the panic instead of
            // leaving the task `Running` forever.
            let (state, text, turns, usage) = contain_panic(
                async {
                    match child.run(Agent::task_user_message(&prompt)).await {
                        Ok(outcome) => match Agent::task_run_failure(&outcome) {
                            Some(reason) => (
                                "failed",
                                format!("subagent run failed: {reason}"),
                                0,
                                Some(outcome.usage),
                            ),
                            None => {
                                let (text, turns) = Agent::summarize_task_output(
                                    &subagent_type,
                                    &description,
                                    model.as_deref(),
                                    &outcome.produced,
                                );
                                ("completed", text, turns, Some(outcome.usage))
                            }
                        },
                        Err(e) => ("failed", format!("task host run failed: {e}"), 0, None),
                    }
                },
                |panic| {
                    (
                        "failed",
                        format!("subagent run failed: the run panicked ({panic})"),
                        0,
                        None,
                    )
                },
            )
            .await;
            // The registry must learn the outcome even when some unrelated panic poisoned the
            // lock: a `Running` entry whose run is over is exactly the lie this milestone
            // removes, so this write recovers the guard instead of silently skipping.
            let mut v = tasks
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(entry) = v.iter_mut().find(|e| e.id == id) {
                entry.state = if state == "completed" {
                    epsilon_contracts::tool::TaskState::Completed
                } else {
                    epsilon_contracts::tool::TaskState::Failed
                };
                entry.output = text.clone();
                entry.turns += turns;
                entry.usage = usage;
                // Fold the spend now when the parent still exists: completion
                // is a real event, and waiting for a poll loses it entirely if
                // nobody polls. The task entry is the display copy; mark it
                // reported so the next `take_task_usage` cannot bill it twice.
                if let (Some(parent), Some(spend)) = (parent.as_ref(), usage) {
                    let model = model.clone().unwrap_or_else(|| parent.model_id());
                    parent.record_external_usage(&model, spend);
                    entry.usage_reported = true;
                } else {
                    entry.usage_reported = false;
                }
            }
            let _ = notices.send(Agent::task_notice(
                &notice_id,
                &description,
                state,
                model.as_deref(),
                &text,
            ));
        });
    }
}

#[async_trait::async_trait]
impl epsilon_contracts::tool::TaskHost for Agent {
    async fn run_task(
        &self,
        request: epsilon_contracts::tool::TaskRequest,
    ) -> Result<epsilon_contracts::tool::TaskOutcome, String> {
        self.delegate_task(request).await
    }

    fn task_snapshot(&self, id: &str) -> Option<epsilon_contracts::tool::TaskSnapshot> {
        Agent::task_snapshot(self, id)
    }

    fn take_task_usage(&self, id: &str) -> Option<epsilon_contracts::message::Usage> {
        Agent::take_task_usage(self, id)
    }

    fn task_ids(&self) -> Vec<String> {
        Agent::task_ids(self)
    }
}
