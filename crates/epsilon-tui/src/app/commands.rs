//! Slash-command execution, split by what the command is *about*.
//!
//! ## Why this file exists
//!
//! `run_command` had grown to **1,147 lines**: a single `match` with 34 arms, no one of them
//! larger than 97 lines. Nothing about it was hard to *do* — that is exactly the problem. A
//! function that long cannot be reviewed, cannot be tested one concern at a time, and gives a
//! new command nowhere obvious to go.
//!
//! So it is now a dispatcher over three themed executors. The 34 arms were moved by line
//! range, not retyped, and each executor's arm bodies are byte-identical to what `app.rs`
//! held (verified by the checker in `docs/milestones/M162-slash-commands-by-subject.md`).
//! Because the arms sat at the same indent depth inside the same `match`-inside-`fn` shape,
//! the move required no re-indentation at all — which is also why it is hard to get wrong.
//!
//! ## Why the catch-all is deliberate
//!
//! `run_command` routes two groups by name and sends **everything else** to
//! `run_workspace_command`. A new `Command` variant therefore cannot be silently unrouted:
//! it lands in the third executor, where the compiler will insist its arm exist, and the
//! `Command::Unknown` fallback stays honest. An exhaustive-by-name router would instead make
//! a forgotten command a runtime no-op, which is the kind of bug that ships.
//!
//! ## The grouping
//!
//! - **provider** — who answers, and how hard they think: `/models`, `/model`, `/think`,
//!   `/scoped-models`, `/login`, `/logout`.
//! - **session** — what the session *is*: fork, new, resume, browse, inspect the tree,
//!   clone, reload, rename.
//! - **workspace** — everything the frame and the workspace do, from `/theme` to `/export`.
//!
//! It is a judgement about cohesion, not a claim that these three are the only right groups.
//! What matters is that the boundaries are legible to the next reader.

use super::*;

impl App {
    /// Route a parsed slash command to the executor that owns it.
    ///
    /// Replies are pushed synchronously into the transcript so they render on the very next
    /// frame; the notices channel stays reserved for async events. Returns `true` to quit.
    pub(super) fn run_command(
        &mut self,
        agent: &Arc<Agent>,
        command: crate::picker::Command,
    ) -> bool {
        match &command {
            // Who answers, and how hard they think.
            crate::picker::Command::Models
            | crate::picker::Command::Model(_)
            | crate::picker::Command::Think(_)
            | crate::picker::Command::ScopedModels
            | crate::picker::Command::Login { .. }
            | crate::picker::Command::Logout(_) => self.run_provider_command(agent, command),
            // What the session is.
            crate::picker::Command::Fork
            | crate::picker::Command::New
            | crate::picker::Command::Resume(_)
            | crate::picker::Command::Sessions
            | crate::picker::Command::Session
            | crate::picker::Command::Tree
            | crate::picker::Command::Clone
            | crate::picker::Command::Reload
            | crate::picker::Command::Name(_) => self.run_session_command(agent, command),
            // Show the user something.
            crate::picker::Command::Help
            | crate::picker::Command::Skills
            | crate::picker::Command::Tasks
            | crate::picker::Command::Timeline
            | crate::picker::Command::Hotkeys
            | crate::picker::Command::Settings
            | crate::picker::Command::Changelog => self.run_view_command(agent, command),
            // Move bytes or permissions across the trust boundary.
            crate::picker::Command::Trust(_)
            | crate::picker::Command::Export(_)
            | crate::picker::Command::Import(_)
            | crate::picker::Command::Share
            | crate::picker::Command::Copy => self.run_transfer_command(agent, command),
            // The catch-all: the conversation and the frame.
            _ => self.run_workspace_command(agent, command),
        }
    }

    /// Provider and model commands.
    fn run_provider_command(
        &mut self,
        agent: &Arc<Agent>,
        command: crate::picker::Command,
    ) -> bool {
        match command {
            crate::picker::Command::Models => {
                self.picker_open = true;
                self.picker_index = 0;
                // Reuse the editor line as the picker's filter box.
                self.input.clear();
                self.cursor_chars = 0;
                // M20 auto-fetch (opencode/Hermes convention): the highlighted
                // provider's live list starts refreshing immediately, so by the
                // time the user descends into it the rows are current.
                self.request_live(false);
            }
            crate::picker::Command::Model(id) => self.apply_model(agent, id),
            crate::picker::Command::Think(arg) => {
                match arg.as_deref().map(str::trim) {
                    None => {
                        // Direct push: slash replies render this frame.
                        let cur = agent.thinking_level();
                        self.ui
                            .push_notice(format!("thinking level: {}", cur.as_str()));
                    }
                    Some(word) => match epsilon_contracts::ThinkingLevel::parse(word) {
                        Some(level) => self.set_thinking(agent, level),
                        None => {
                            // Direct push: slash replies render this frame.
                            self.ui.push_notice(format!(
                                "unknown level '{word}' - use: {}",
                                levels_help()
                            ));
                        }
                    },
                }
            }
            crate::picker::Command::ScopedModels => {
                if self.favorites.is_empty() {
                    self.ui
                        .push_notice("scoped-models: no favorites yet — open Ctrl+P and press Ctrl+F on a model to star it, then Ctrl+P cycles favorites");
                } else {
                    // Ponytail: route to the model picker seeded with favorites (pi scopedModels → model picker).
                    self.picker_open = true;
                    self.picker_index = 0;
                    self.invalidate_picker();
                    self.ui.push_notice(format!(
                        "scoped-models: {} favorite(s) — cycling favorites · open Ctrl+P to pick, Ctrl+F to star/unstar",
                        self.favorites.len()
                    ));
                }
            }
            crate::picker::Command::Login { provider, key } => {
                let provider = provider.trim().to_string();
                // M52: `/login <provider> oauth` (or `--oauth`) starts the
                // subscription ceremony instead of storing a pasted key.
                if !provider.is_empty() && epsilon_llm::oauth_login::is_oauth_ceremony_arg(&key) {
                    self.start_oauth_login(provider);
                    return false;
                }
                if provider.is_empty() {
                    // Bare `/login` — open the provider picker so the user
                    // picks the target instead of having to type the id.
                    // Mirrors opencode's DialogProvider select step.
                    self.auth_pick_open = true;
                    self.auth_pick_index = 0;
                    self.input.clear();
                    self.cursor_chars = 0;
                    self.ui.push_notice(
                        "connect a provider — pick one (↑/↓ Enter), or type to filter",
                    );
                    return false;
                }
                if key.trim().is_empty() {
                    // Masked prompt overlay like opencode's DialogPrompt placeholder="API key".
                    self.auth_prompt_open = true;
                    self.auth_provider = provider.clone();
                    self.auth_key.clear();
                    self.auth_key_cursor = 0;
                    let oauth_hint = if epsilon_llm::oauth_login::login_flow(&provider).is_some() {
                        " · Ctrl+O = subscription login (OAuth)"
                    } else {
                        ""
                    };
                    self.ui.push_notice(format!(
                        "enter API key for '{provider}' — paste the key (masked), Enter saves to ~/.config/epsilon/auth.json, Esc cancels{oauth_hint}"
                    ));
                    return false;
                }
                match epsilon_llm::auth::save_credential(&provider, key.trim()) {
                    Ok(path) => {
                        self.ui.push_notice(format!(
                            "saved key for '{provider}' → {}  (next request uses it)",
                            path.display()
                        ));
                        self.invalidate_picker();
                    }
                    Err(e) => self
                        .ui
                        .push_notice(format!("not saved for '{provider}': {e}")),
                }
            }
            crate::picker::Command::Logout(provider) => {
                let provider = provider.trim().to_string();
                if provider.is_empty() {
                    self.ui
                        .push_notice("usage: /logout <provider> — e.g. /logout anthropic");
                    return false;
                }
                match epsilon_llm::auth::remove_credential(&provider) {
                    Ok(true) => {
                        self.ui
                            .push_notice(format!("removed stored key for '{provider}'"));
                        self.invalidate_picker();
                    }
                    Ok(false) => self
                        .ui
                        .push_notice(format!("no stored key for '{provider}' (nothing removed)")),
                    Err(e) => self
                        .ui
                        .push_notice(format!("not removed for '{provider}': {e}")),
                }
            }
            // `run_command` owns the routing, so reaching a variant here means the
            // dispatcher sent it to the wrong executor. `debug_assert` catches that in
            // every test build and stays silent in release - a user's release binary
            // must not panic over an internal routing mistake, and a test must not be
            // able to pass by quietly doing nothing.
            #[allow(unreachable_patterns)]
            other => {
                debug_assert!(false, "{other:?} is not a run_provider_command case");
                return false;
            }
        }
        false
    }

    /// Session-identity commands.
    fn run_session_command(&mut self, agent: &Arc<Agent>, command: crate::picker::Command) -> bool {
        match command {
            crate::picker::Command::Sessions
            | crate::picker::Command::Session
            | crate::picker::Command::Tree => self.run_navigate_command(agent, command),
            crate::picker::Command::Fork
            | crate::picker::Command::Clone
            | crate::picker::Command::Name(_) => self.run_branch_command(agent, command),
            crate::picker::Command::New
            | crate::picker::Command::Resume(_)
            | crate::picker::Command::Reload => self.run_restore_command(agent, command),
            // `run_command` owns the routing, so reaching a variant here means the
            // dispatcher sent it to the wrong executor. `debug_assert` catches that in
            // every test build and stays silent in release - a user's release binary
            // must not panic over an internal routing mistake, and a test must not be
            // able to pass by quietly doing nothing.
            #[allow(unreachable_patterns)]
            other => {
                debug_assert!(false, "{other:?} is not a run_session case");
                false
            }
        }
    }

    fn run_navigate_command(
        &mut self,
        agent: &Arc<Agent>,
        command: crate::picker::Command,
    ) -> bool {
        match command {
            crate::picker::Command::Sessions => {
                self.open_session_browser(agent);
            }
            crate::picker::Command::Session => {
                // No durable mirror is a normal state (a plain `--tui` run, a child agent), not a
                // crash: `store_handle_for_tests` panics on it, so the command asks for the
                // Option instead.
                let (path, leaf, count) = match agent.session_store() {
                    Some(store) => match store.lock() {
                        Ok(st) => (
                            st.path().to_path_buf(),
                            st.leaf_id().unwrap_or_else(|| "(none)".into()),
                            st.branch_entries().map(|v| v.len()).unwrap_or(0),
                        ),
                        Err(_) => {
                            self.ui.push_notice("session: store unavailable");
                            return false;
                        }
                    },
                    None => {
                        let (tin, tout) = self.ui.usage();
                        self.ui.push_notice(format!(
                            "session: no durable session file attached \u{b7} tokens {tin}/{tout} \u{b7} model {}",
                            self.model_id
                        ));
                        // Cost, broken down (M143). The header shows one number; this says where
                        // it went. A total alone cannot tell you the cache is working, and the
                        // cache is the main lever a user has on cost.
                        self.ui.push_notice(self.cost_summary());
                        for line in agent.cache_report() {
                            self.ui.push_notice(line);
                        }
                        for line in repair_lines(agent) {
                            self.ui.push_notice(line);
                        }
                        return false;
                    }
                };
                let (tin, tout) = self.ui.usage();
                if let Some(n) = self.session_name.as_deref() {
                    self.ui.push_notice(format!(
                        "session: '{n}' · {} · leaf {} · {} entries · tokens {tin}/{tout} · model {}",
                        path.display(),
                        leaf,
                        count,
                        self.model_id
                    ));
                } else {
                    self.ui.push_notice(format!(
                        "session: {} · leaf {} · {} entries · tokens {tin}/{tout} · model {}",
                        path.display(),
                        leaf,
                        count,
                        self.model_id
                    ));
                }
                // Cache attribution (M69): the footer's CH% says whether the prompt cache is
                // paying off; these lines say what it cost when it did not.
                for line in agent.cache_report() {
                    self.ui.push_notice(line);
                }
                // CONTROL (M143): the cost breakdown line removed — the state before this
                // slice, where the four per-component accessors had no caller at all.
                // Session repairs (M75): a resumed session may have been missing tool results or
                // an end marker. The kinds are worth naming — they are what the user lost.
                for line in repair_lines(agent) {
                    self.ui.push_notice(line);
                }
            }
            crate::picker::Command::Tree => {
                // Alias for /timeline — open the same picker (pi parity: /branch → timeline).
                let entries = {
                    let guard = agent.store_handle_for_tests();
                    let Ok(st) = guard.lock() else {
                        self.ui.push_notice("tree: store unavailable");
                        return false;
                    };
                    match st.branch_entries() {
                        Ok(v) => v,
                        Err(e) => {
                            self.ui.push_notice(format!("tree failed: {e}"));
                            return false;
                        }
                    }
                };
                if entries.is_empty() {
                    self.ui.push_notice("tree: session is empty");
                    return false;
                }
                fn snip2(e: &epsilon_contracts::SessionEntry) -> String {
                    match &e.kind {
                        epsilon_contracts::session_entry::EntryKind::Message {
                            message, ..
                        } => {
                            let txt: String = match message {
                                epsilon_contracts::message::Message::User(u) => u
                                    .content
                                    .iter()
                                    .filter_map(|c| match c {
                                        epsilon_contracts::content::UserContent::Text { text } => {
                                            Some(text.as_str())
                                        }
                                        _ => None,
                                    })
                                    .collect::<Vec<_>>()
                                    .join(" "),
                                epsilon_contracts::message::Message::Assistant(a) => a.text(),
                                epsilon_contracts::message::Message::ToolResult(t) => t
                                    .content
                                    .iter()
                                    .filter_map(|c| match c {
                                        epsilon_contracts::content::ContentBlock::Text { text } => {
                                            Some(text.as_str())
                                        }
                                        _ => None,
                                    })
                                    .collect::<String>(),
                                _ => String::new(),
                            };
                            let mut s: String = txt.chars().take(100).collect();
                            if txt.chars().count() > 100 {
                                s.push('…');
                            }
                            if s.trim().is_empty() {
                                "(empty)".into()
                            } else {
                                s
                            }
                        }
                        epsilon_contracts::session_entry::EntryKind::Compaction {
                            summary, ..
                        } => {
                            let mut s: String = summary.chars().take(100).collect();
                            if summary.chars().count() > 100 {
                                s.push('…');
                            }
                            format!("[compaction] {s}")
                        }
                        epsilon_contracts::session_entry::EntryKind::TurnEnd { reason, .. } => {
                            format!("[turn_end {reason:?}]")
                        }
                        _ => "[other]".into(),
                    }
                }
                fn kind_tag2(e: &epsilon_contracts::SessionEntry) -> &'static str {
                    match &e.kind {
                        epsilon_contracts::session_entry::EntryKind::Message {
                            message, ..
                        } => match message {
                            epsilon_contracts::message::Message::User(_) => "user",
                            epsilon_contracts::message::Message::Assistant(_) => "assistant",
                            epsilon_contracts::message::Message::ToolResult(_) => "tool",
                            _ => "msg",
                        },
                        epsilon_contracts::session_entry::EntryKind::Compaction { .. } => "compact",
                        epsilon_contracts::session_entry::EntryKind::TurnEnd { .. } => "turn",
                        _ => "other",
                    }
                }
                self.timeline_entries = entries
                    .iter()
                    .map(|e| (e.seq, kind_tag2(e).to_string(), snip2(e)))
                    .collect();
                self.timeline_index = self.timeline_entries.len().saturating_sub(1);
                self.timeline_open = true;
            }
            // `run_command` owns the routing, so reaching a variant here means the
            // dispatcher sent it to the wrong executor. `debug_assert` catches that in
            // every test build and stays silent in release - a user's release binary
            // must not panic over an internal routing mistake, and a test must not be
            // able to pass by quietly doing nothing.
            #[allow(unreachable_patterns)]
            other => {
                debug_assert!(false, "{other:?} is not a run_navigate case");
                return false;
            }
        }
        false
    }

    fn run_branch_command(&mut self, agent: &Arc<Agent>, command: crate::picker::Command) -> bool {
        match command {
            crate::picker::Command::Fork => {
                let store = agent.store_handle_for_tests();
                let (path, leaf) = match store.lock() {
                    Ok(st) => (st.path().to_path_buf(), st.leaf_id()),
                    Err(_) => {
                        self.ui.push_notice("fork: session store unavailable");
                        return false;
                    }
                };
                if leaf.is_none() {
                    self.ui
                        .push_notice("fork: nothing to fork — session is empty");
                    return false;
                }
                let fork_path = {
                    let stem = path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("session");
                    let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("jsonl");
                    let fork_name =
                        format!("{stem}-fork-{}.{}", epsilon_contracts::new_id("fork"), ext);
                    path.with_file_name(fork_name)
                };
                match std::fs::copy(&path, &fork_path) {
                    Ok(_) => self.ui.push_notice(format!(
                        "forked session → {} (open with --session {})",
                        fork_path.display(),
                        fork_path.display()
                    )),
                    Err(e) => self.ui.push_notice(format!("fork failed: {e}")),
                }
            }
            crate::picker::Command::Clone => {
                let store = agent.store_handle_for_tests();
                let (path, leaf) = match store.lock() {
                    Ok(st) => (st.path().to_path_buf(), st.leaf_id()),
                    Err(_) => {
                        self.ui.push_notice("clone: session store unavailable");
                        return false;
                    }
                };
                if leaf.is_none() {
                    self.ui
                        .push_notice("clone: nothing to clone — session is empty");
                    return false;
                }
                let clone_path = {
                    let stem = path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("session");
                    let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("jsonl");
                    let name = format!(
                        "{stem}-clone-{}.{}",
                        epsilon_contracts::new_id("clone"),
                        ext
                    );
                    path.with_file_name(name)
                };
                match std::fs::copy(&path, &clone_path) {
                    Ok(_) => self.ui.push_notice(format!(
                        "cloned session → {} (open with --session {})",
                        clone_path.display(),
                        clone_path.display()
                    )),
                    Err(e) => self.ui.push_notice(format!("clone failed: {e}")),
                }
            }
            crate::picker::Command::Name(arg) => {
                let name = arg.as_deref().unwrap_or("").trim();
                if name.is_empty() {
                    if let Some(n) = self.session_name.clone() {
                        self.ui.push_notice(format!("session name: '{n}'"));
                    } else {
                        // Also check persisted Custom entry (survives resume before in-memory is set).
                        let persisted: Option<String> = (|| {
                            let store = agent.store_handle_for_tests();
                            let st = store.lock().ok()?;
                            let entries = st.read_all().ok()?;
                            for e in entries.iter().rev() {
                                if let epsilon_contracts::session_entry::EntryKind::Custom {
                                    custom_type,
                                    data,
                                } = &e.kind
                                    && custom_type == "session_name"
                                    && let Some(v) = data
                                        .as_ref()
                                        .and_then(|d| d.get("name"))
                                        .and_then(|v| v.as_str())
                                {
                                    return Some(v.to_string());
                                }
                            }
                            None
                        })();
                        if let Some(n) = persisted {
                            self.session_name = Some(n.clone());
                            self.ui.push_notice(format!("session name: '{n}'"));
                        } else {
                            self.ui.push_notice(
                                "usage: /name <display name> — sets session display name",
                            );
                        }
                    }
                } else {
                    self.session_name = Some(name.to_string());
                    // Persist as non-LLM Custom entry so it survives restarts via
                    // the same session file pi/gap-analysis parity intended (extend
                    // to a dedicated EntryKind when a UI needs to render it).
                    let _ = (|| -> Result<(), String> {
                        let store = agent.store_handle_for_tests();
                        let mut st = store.lock().map_err(|_| "store lock".to_string())?;
                        st.append(epsilon_contracts::session_entry::EntryKind::Custom {
                            custom_type: "session_name".into(),
                            data: Some(serde_json::json!({"name": name})),
                        })
                        .map_err(|e| format!("{e}"))?;
                        Ok(())
                    })();
                    self.ui.push_notice(format!("session name set to '{name}'"));
                }
            }
            // `run_command` owns the routing, so reaching a variant here means the
            // dispatcher sent it to the wrong executor. `debug_assert` catches that in
            // every test build and stays silent in release - a user's release binary
            // must not panic over an internal routing mistake, and a test must not be
            // able to pass by quietly doing nothing.
            #[allow(unreachable_patterns)]
            other => {
                debug_assert!(false, "{other:?} is not a run_branch case");
                return false;
            }
        }
        false
    }

    fn run_restore_command(&mut self, agent: &Arc<Agent>, command: crate::picker::Command) -> bool {
        match command {
            crate::picker::Command::New => {
                let (tin, tout) = self.ui.usage();
                self.ui.clear_lines();
                self.ui.push_notice(format!(
                    "new session — transcript cleared (session file continues; use /fork for a branch file) (tokens {tin}/{tout})"
                ));
            }
            crate::picker::Command::Resume(path) => {
                let p = path.trim();
                if p.is_empty() {
                    // M57: bare `/resume` opens the browser — the path form is
                    // still there for scripts and muscle memory.
                    self.open_session_browser(agent);
                } else if self.ui.running() || agent.is_running() {
                    self.ui.push_notice(
                        "resume: busy — wait for the current run to finish, then /resume again",
                    );
                } else {
                    let raw = std::path::PathBuf::from(p);
                    let abs = if raw.is_absolute() {
                        raw
                    } else {
                        self.workspace_root.join(raw)
                    };
                    if !abs.exists() {
                        self.ui.push_notice(format!(
                            "resume: no file at '{}' — check the path",
                            abs.display()
                        ));
                    } else {
                        match self.switch_session_file(agent, &abs, Some(p)) {
                            Ok(count) => self.ui.push_notice(format!(
                                "resumed {} messages from {} (history restored, store swapped)",
                                count,
                                abs.display()
                            )),
                            Err(e) => self.ui.push_notice(format!("resume failed: {e}")),
                        }
                    }
                }
            }
            crate::picker::Command::Reload => {
                let mut roots = Vec::new();
                if let Some(dir) = epsilon_core::config::global_dir() {
                    roots.push(dir.join("skills"));
                }
                roots.push(self.workspace_root.join(".agents").join("skills"));
                roots.push(self.workspace_root.join(".epsilon").join("skills"));
                let skills = epsilon_core::skills::discover(&roots);
                let n = skills.len();
                agent.set_skills(skills);
                // M50: agent defs reload alongside skills (same roots pattern —
                // global always, project pair gated by the trust bit pi's
                // agentScope maps onto: trust_review_pending means untrusted).
                let agents = epsilon_core::agents::discover_agents(
                    &{
                        let mut r = Vec::new();
                        if let Some(dir) = epsilon_core::config::global_dir() {
                            r.push(dir.join("agents"));
                        }
                        r
                    },
                    &[
                        self.workspace_root.join(".agents").join("agents"),
                        self.workspace_root.join(".epsilon").join("agents"),
                    ],
                    !self.trust_review_pending,
                );
                let an = agents.len();
                agent.set_agents(agents);
                // G6: re-read keybindings.json so a remap takes effect without restart (pi reload()).
                let kb_before = self.keybindings.effective();
                self.keybindings.reload();
                let kb_note = if self.keybindings.effective() != kb_before {
                    " · keybindings reloaded"
                } else {
                    ""
                };
                self.ui.push_notice(format!(
                    "reload: {n} skill(s) + {an} agent(s) loaded (prompt tier re-read on next turn){kb_note}"
                ));
                self.invalidate_picker();
            }
            // `run_command` owns the routing, so reaching a variant here means the
            // dispatcher sent it to the wrong executor. `debug_assert` catches that in
            // every test build and stays silent in release - a user's release binary
            // must not panic over an internal routing mistake, and a test must not be
            // able to pass by quietly doing nothing.
            #[allow(unreachable_patterns)]
            other => {
                debug_assert!(false, "{other:?} is not a run_restore case");
                return false;
            }
        }
        false
    }

    /// Everything the frame and the workspace do.
    /// Commands that act on the running conversation or the frame itself.
    ///
    /// This is the dispatcher's **catch-all**: a new `Command` variant lands here, where
    /// the compiler demands its arm, rather than going unrouted. That is why it is allowed
    /// to be the vague one - and why it is also kept small. The two executors it used to
    /// hold are `run_view_command` and `run_transfer_command`.
    fn run_workspace_command(
        &mut self,
        agent: &Arc<Agent>,
        command: crate::picker::Command,
    ) -> bool {
        match command {
            crate::picker::Command::Clear => {
                let (tin, tout) = self.ui.usage();
                self.ui.clear_lines();
                self.ui.push_notice(format!(
                    "transcript cleared — session totals kept (tokens {tin}/{tout})"
                ));
            }
            crate::picker::Command::Search(query) => {
                // M157: this scanned the session *log on disk* and printed its hits as
                // transcript notices — a list of places you could go, printed into the one
                // place you already were. Nothing moved, nothing was highlighted, and with
                // no session log attached it answered "search unavailable this run", which
                // is exactly when a user most wants it.
                //
                // It searches the rendered transcript now, because that is what the reader
                // is looking at (pi searches the screen too, not the log). The trade is
                // honest and recorded in `search.rs`: the log reached further back than the
                // viewport, but it could not show you what it found.
                self.search_own_transcript(&query);
            }
            crate::picker::Command::Quit => return true,
            crate::picker::Command::Theme(arg) => {
                // The palette is a **view** choice, not session state, so it is applied to the
                // live theme and not written to config: a cosmetic toggle should not quietly
                // rewrite a file the user may have under version control. `EPSILON_THEME` and
                // `[tui] theme` remain the way to make a default permanent.
                let next = match arg.as_deref().map(str::trim) {
                    None | Some("") => self.theme.palette().flipped(),
                    Some(name) => match crate::ui::theme::ThemeMode::from_name(name) {
                        Some(mode) => mode,
                        None => {
                            self.ui.push_notice(format!(
                                "unknown palette '{name}' \u{2014} use `light` or `dark`, or bare `/theme` to toggle"
                            ));
                            return false;
                        }
                    },
                };
                let was = self.theme.palette();
                self.theme.set_palette(next);
                // M160: the transcript is drawn through the render cache, and a cache entry
                // carries the escape codes it was drawn with. The palette is part of `CacheKey`
                // now, so the next reconcile rebuilds; this drops the rows already drawn for
                // this frame so none of them survives it. Without both, the chrome would change
                // immediately while cached rows kept the old palette \u{2014} half a theme change,
                // which is worse than none because it looks deliberate.
                self.render_cache.clear();
                self.ui
                    .push_notice(format!("palette: {} \u{2192} {}", was.name(), next.name()));
            }
            crate::picker::Command::Compact => {
                if self.compact_running {
                    self.ui
                        .push_notice("compact already running — wait for it to finish");
                    return false;
                }
                if self.ui.running() {
                    self.ui.push_notice(
                        "compact queued — will run after the current turn (loop also auto-compacts on pressure)",
                    );
                } else {
                    self.compact_running = true;
                    self.ui.push_notice("compacting context…");
                    let a = Arc::clone(agent);
                    // best-effort: clear spinner when the task finishes (even if it did nothing).
                    // We don't have a direct callback from compact_now, so we spawn a fire-and-forget
                    // that resets the flag after a short grace; the real compaction stream would
                    // clear it if it existed. For now the spinner stays until the next successful /compact.
                    tokio::spawn(async move {
                        let mut produced = Vec::new();
                        let _ = a.compact_now(&mut produced).await;
                    });
                }
            }
            crate::picker::Command::Unknown(word) => {
                let hint = crate::picker::suggest_slash(&word)
                    .map(|item| format!(" — did you mean '{}'?", item.value.trim()))
                    .unwrap_or_default();
                self.ui.push_notice(format!(
                    "unknown command '/{word}'{hint} — /help lists commands"
                ));
            }
            crate::picker::Command::Editor => {
                self.request_external_editor();
            }
            // `run_command` owns the routing, so reaching a variant here means the
            // dispatcher sent it to the wrong executor. `debug_assert` catches that in
            // every test build and stays silent in release - a user's release binary
            // must not panic over an internal routing mistake, and a test must not be
            // able to pass by quietly doing nothing.
            #[allow(unreachable_patterns)]
            other => {
                debug_assert!(false, "{other:?} is not a run_workspace_command case");
                return false;
            }
        }
        false
    }

    /// Commands that show the user something and change nothing: `/help`, `/skills`,
    /// `/tasks`, `/timeline`, `/hotkeys`, `/settings`, `/changelog`.
    fn run_view_command(&mut self, agent: &Arc<Agent>, command: crate::picker::Command) -> bool {
        match command {
            crate::picker::Command::Help => {
                for item in crate::picker::SLASH_ITEMS {
                    self.ui.push_notice(format!(
                        "{} — {}",
                        item.value.trim_end(),
                        item.description
                    ));
                }
                for line in [
                    "auth: /login anthropic sk-...  (paste inside TUI, masked) · /logout anthropic · keys saved to ~/.config/epsilon/auth.json",
                    "subscription login: /login anthropic oauth · /login xai oauth (device code) · /login openrouter oauth — browser round-trip, same store",
                    "also: /connect /auth are aliases for /login — same masked prompt, paste works inside it",
                    "thinking: Ctrl+T cycles · current level in the status bar",
                    // The header is the harness's own report on itself, and a report with
                    // no legend is only transparent to the person who wrote it. These two
                    // lines decode it, including the two placeholders that mean "unknown"
                    // rather than "zero".
                    "header: 42%/200k = context used/window (warns at 70%, red at 90%) · ↑12k ↓3.4k = tokens in/out this session",
                    "header: cache 87% = share of the LAST prompt served from cache (— = none yet) · $0.42 = estimated cost at list prices",
                    "header: a trailing + on the cost means the session moved tokens the model registry does not price, so the figure is a lower bound",
                    "keys: Enter submit · Esc aborts a run (quits when idle) · Ctrl+X expands tool output",
                    "      PgUp/PgDn scroll transcript · Ctrl+M model picker · Ctrl+P command palette",
                ] {
                    self.ui.push_notice(line);
                }
            }
            crate::picker::Command::Skills => {
                let names = agent.skill_names();
                if names.is_empty() {
                    self.ui.push_notice(
                        "no skills discovered — drop SKILL.md dirs into .epsilon/skills/",
                    );
                } else {
                    self.ui.push_notice(format!("skills: {}", names.join(", ")));
                }
            }
            crate::picker::Command::Tasks => {
                // M54: the registry is the only place a delegated child's state
                // is visible; Esc (abort) now reaches running children too, so
                // this doubles as the "what am I still waiting on" view.
                let snapshots = agent.task_snapshots();
                if snapshots.is_empty() {
                    self.ui.push_notice(
                        "no subagent tasks — the model starts them with the `task` tool (background=true runs detached and reports back here)",
                    );
                } else {
                    let mut running = 0usize;
                    for s in &snapshots {
                        let state = match s.state {
                            epsilon_contracts::tool::TaskState::Running => "running",
                            epsilon_contracts::tool::TaskState::Completed => "completed",
                            epsilon_contracts::tool::TaskState::Failed => "failed",
                        };
                        if s.state == epsilon_contracts::tool::TaskState::Running {
                            running += 1;
                        }
                        let head = s.output.lines().next().unwrap_or("").trim();
                        let preview: String = head.chars().take(120).collect();
                        // M56: name the model the child runs on, so a scout on
                        // a cheap model is distinguishable at a glance.
                        let on_model = s
                            .model
                            .as_deref()
                            .map(|m| format!(" · @{m}"))
                            .unwrap_or_default();
                        self.ui.push_notice(format!(
                            "task {} · {state}{on_model} · {} · {} turn(s) · {}",
                            s.id,
                            s.description,
                            s.turns,
                            if preview.is_empty() {
                                "(no output yet)"
                            } else {
                                preview.as_str()
                            }
                        ));
                    }
                    self.ui.push_notice(format!(
                        "{} task(s) · {running} running — resume one with the task tool (task_id) or Esc a run to stop them",
                        snapshots.len()
                    ));
                }
            }
            crate::picker::Command::Timeline => {
                let entries = {
                    let guard = agent.store_handle_for_tests();
                    let Ok(st) = guard.lock() else {
                        self.ui.push_notice("timeline: store unavailable");
                        return false;
                    };
                    match st.branch_entries() {
                        Ok(v) => v,
                        Err(e) => {
                            self.ui.push_notice(format!("timeline failed: {e}"));
                            return false;
                        }
                    }
                };
                if entries.is_empty() {
                    self.ui.push_notice("timeline: session is empty");
                    return false;
                }
                fn snip(e: &epsilon_contracts::SessionEntry) -> String {
                    match &e.kind {
                        epsilon_contracts::session_entry::EntryKind::Message {
                            message, ..
                        } => {
                            let txt: String = match message {
                                epsilon_contracts::message::Message::User(u) => u
                                    .content
                                    .iter()
                                    .filter_map(|c| match c {
                                        epsilon_contracts::content::UserContent::Text { text } => {
                                            Some(text.as_str())
                                        }
                                        _ => None,
                                    })
                                    .collect::<Vec<_>>()
                                    .join(" "),
                                epsilon_contracts::message::Message::Assistant(a) => a.text(),
                                epsilon_contracts::message::Message::ToolResult(t) => t
                                    .content
                                    .iter()
                                    .filter_map(|c| match c {
                                        epsilon_contracts::content::ContentBlock::Text { text } => {
                                            Some(text.as_str())
                                        }
                                        _ => None,
                                    })
                                    .collect::<String>(),
                                _ => String::new(),
                            };
                            let mut s: String = txt.chars().take(100).collect();
                            if txt.chars().count() > 100 {
                                s.push('…');
                            }
                            if s.trim().is_empty() {
                                "(empty)".into()
                            } else {
                                s
                            }
                        }
                        epsilon_contracts::session_entry::EntryKind::Compaction {
                            summary, ..
                        } => {
                            let mut s: String = summary.chars().take(100).collect();
                            if summary.chars().count() > 100 {
                                s.push('…');
                            }
                            format!("[compaction] {s}")
                        }
                        epsilon_contracts::session_entry::EntryKind::TurnEnd { reason, .. } => {
                            format!("[turn_end {reason:?}]")
                        }
                        _ => "[other]".into(),
                    }
                }
                fn kind_tag(e: &epsilon_contracts::SessionEntry) -> &'static str {
                    match &e.kind {
                        epsilon_contracts::session_entry::EntryKind::Message {
                            message, ..
                        } => match message {
                            epsilon_contracts::message::Message::User(_) => "user",
                            epsilon_contracts::message::Message::Assistant(_) => "assistant",
                            epsilon_contracts::message::Message::ToolResult(_) => "tool",
                            _ => "msg",
                        },
                        epsilon_contracts::session_entry::EntryKind::Compaction { .. } => "compact",
                        epsilon_contracts::session_entry::EntryKind::TurnEnd { .. } => "turn",
                        _ => "other",
                    }
                }
                self.timeline_entries = entries
                    .iter()
                    .map(|e| (e.seq, kind_tag(e).to_string(), snip(e)))
                    .collect();
                self.timeline_index = self.timeline_entries.len().saturating_sub(1);
                self.timeline_open = true;
            }
            crate::picker::Command::Hotkeys => {
                for line in [
                    "hotkeys: Enter submit · Esc aborts run (quits when idle) · Ctrl+X expands tool output",
                    "         PgUp/PgDn scroll · Ctrl+M model picker · Ctrl+P command palette · Ctrl+T think",
                    "         Ctrl+U kill to start · Ctrl+W kill word · Ctrl+Y yank · Ctrl+F favorite",
                    "         /help lists slash commands · /models switches model",
                ] {
                    self.ui.push_notice(line);
                }
                // G6 polish: prove the remap in the transcript — effective
                // bindings from ~/.config/epsilon/keybindings.json (sorted).
                let mut eff: Vec<(String, Vec<String>)> =
                    self.keybindings.effective().into_iter().collect();
                eff.sort_by(|a, b| a.0.cmp(&b.0));
                if !eff.is_empty() {
                    self.ui
                        .push_notice("bindings (effective — ~/.config/epsilon/keybindings.json):");
                    for (id, keys) in eff {
                        self.ui.push_notice(format!("  {id}: {}", keys.join(" | ")));
                    }
                }
            }
            crate::picker::Command::Settings => {
                // Ponytail: no overlay form at v1 (pi SettingsSelectorComponent is ~30 toggles).
                // Instead show actionable guidance: what exists + where it lives + how to change it.
                let global = epsilon_core::config::global_config_path()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "~/.config/epsilon/config.toml".to_string());
                let project = self.workspace_root.join(".epsilon").join("config.toml");
                self.ui.push_notice(format!(
                    "settings: global {global} · project {}",
                    project.display()
                ));
                self.ui.push_notice(format!(
                    "  current model: {} · think: {}",
                    self.model_id,
                    self.thinking.as_str()
                ));
                let rec = if self.recents.is_empty() {
                    "(none yet)".to_string()
                } else {
                    self.recents[..self.recents.len().min(5)].join(", ")
                };
                let fav = if self.favorites.is_empty() {
                    "(none — Ctrl+F to star)".to_string()
                } else {
                    self.favorites[..self.favorites.len().min(5)].join(", ")
                };
                self.ui.push_notice(format!("  recents: {rec}"));
                self.ui.push_notice(format!("  favorites: {fav}"));
                self.ui
                    .push_notice("edit the toml directly (model: [model] id, favorites: [picker] favorite), then /reload to apply without restart");
            }
            crate::picker::Command::Changelog => {
                // Ponytail: read workspace CHANGELOG.md (pi parseChangelog parity, first section)
                // and emit it into the transcript — no new dep, reuses read_to_string.
                let candidates = [
                    self.workspace_root.join("CHANGELOG.md"),
                    self.workspace_root.join("docs/CHANGELOG.md"),
                ];
                let mut found: Option<(std::path::PathBuf, String)> = None;
                for c in candidates {
                    if let Ok(s) = std::fs::read_to_string(&c)
                        && !s.trim().is_empty()
                    {
                        found = Some((c, s));
                        break;
                    }
                }
                if let Some((path, raw)) = found {
                    // Emit header + first ~120 lines (header + Unreleased + last release) to keep
                    // the transcript bounded; the full file stays on disk at `path`.
                    let lines: Vec<&str> = raw.lines().collect();
                    let take = lines.len().min(120);
                    self.ui.push_notice(format!(
                        "changelog: {} ({} lines, showing {} — see file for full history)",
                        path.display(),
                        lines.len(),
                        take
                    ));
                    for line in &lines[..take] {
                        self.ui.push_notice((*line).to_string());
                    }
                    if lines.len() > take {
                        self.ui.push_notice(format!(
                            "… {} more lines in {}",
                            lines.len() - take,
                            path.display()
                        ));
                    }
                } else {
                    self.ui
                        .push_notice("changelog: no CHANGELOG.md in workspace (see docs/milestones/ for milestone logs)");
                }
            }
            // `run_command` owns the routing, so reaching a variant here means the
            // dispatcher sent it to the wrong executor. `debug_assert` catches that in
            // every test build and stays silent in release - a user's release binary
            // must not panic over an internal routing mistake, and a test must not be
            // able to pass by quietly doing nothing.
            #[allow(unreachable_patterns)]
            other => {
                debug_assert!(false, "{other:?} is not a run_view_command case");
                return false;
            }
        }
        false
    }

    /// Commands that move bytes or permissions across the trust boundary: `/trust`,
    /// `/export`, `/import`, `/share`, `/copy`.
    fn run_transfer_command(
        &mut self,
        agent: &Arc<Agent>,
        command: crate::picker::Command,
    ) -> bool {
        match command {
            crate::picker::Command::Trust(arg) => {
                let a = arg.as_deref().unwrap_or("").trim().to_ascii_lowercase();
                if a.is_empty() {
                    match epsilon_core::config::current_project_config_hash_in(
                        &self.workspace_root,
                    ) {
                        None => self.ui.push_notice(
                            "trust: no project config (.epsilon/config.toml not found) — nothing to trust",
                        ),
                        Some(hash) => {
                            let trust_path =
                                self.workspace_root.join(".epsilon").join("trust.json");
                            match std::fs::read_to_string(&trust_path) {
                                Ok(raw) => {
                                    let prev = serde_json::from_str::<serde_json::Value>(&raw)
                                        .ok()
                                        .and_then(|v| {
                                            v.get("config_sha256")
                                                .and_then(|h| h.as_str())
                                                .map(|s| s.to_string())
                                        });
                                    match prev {
                                        Some(prev) if prev == hash => self.ui.push_notice(
                                            format!("trust: config is trusted (hash {}…)", &hash[..8.min(hash.len())]),
                                        ),
                                        Some(prev) => self.ui.push_notice(format!(
                                            "trust: config changed (prev {}… current {}…) — /trust yes to accept, /trust no to keep untrusted",
                                            &prev[..8.min(prev.len())],
                                            &hash[..8.min(hash.len())]
                                        )),
                                        None => self.ui.push_notice(
                                            "trust: trust file unreadable — /trust yes to accept",
                                        ),
                                    }
                                }
                                Err(_) => self.ui.push_notice(format!(
                                    "trust: config present (hash {}…) but not yet trusted — /trust yes to accept",
                                    &hash[..8.min(hash.len())]
                                )),
                            }
                        }
                    }
                    return false;
                }
                if a == "yes" || a == "y" {
                    if self.trust_review_pending {
                        self.trust_review_pending = false;
                    }
                    let ws = self.workspace_root.clone();
                    // M45: same dual-memory accept as the review gate above.
                    if let Some(store) = crate::trust_store::ProjectTrustStore::open() {
                        match store.set(&ws, Some(true)) {
                            Ok(()) => self.ui.push_notice(
                                "project path trusted (remembered for future runs)".to_string(),
                            ),
                            Err(e) => self.ui.push_notice(format!("trust store note: {e}")),
                        }
                    }
                    match epsilon_core::config::current_project_config_hash_in(&ws) {
                        Some(hash) => {
                            match epsilon_core::config::accept_project_config_hash_in(&ws, &hash) {
                                Ok(path) => self.ui.push_notice(format!(
                                    "trust accepted — hash {}… saved to {}",
                                    &hash[..8.min(hash.len())],
                                    path.display()
                                )),
                                Err(e) => self.ui.push_notice(format!("trust accept failed: {e}")),
                            }
                        }
                        None => self.ui.push_notice("trust: no config file to accept"),
                    }
                } else if a == "no" || a == "n" {
                    self.ui
                        .push_notice("trust: not accepted — config stays untrusted (edits still require /trust yes)");
                } else {
                    self.ui.push_notice("usage: /trust [yes|no]");
                }
            }
            crate::picker::Command::Export(arg) => {
                let tail = arg.as_deref().unwrap_or("").trim();
                // Bare `/export` defaults to html into .epsilon/exports/ (pi parity:
                // `/export` without args writes the HTML export automatically).
                let bare = tail.is_empty();
                let (fmt, raw_path) = if bare {
                    ("html".to_string(), String::new())
                } else {
                    let mut parts = tail.split_whitespace();
                    let first = parts.next().unwrap_or("");
                    let rest = parts.next();
                    if first.eq_ignore_ascii_case("html")
                        || first.eq_ignore_ascii_case("json")
                        || first.eq_ignore_ascii_case("jsonl")
                    {
                        (first.to_ascii_lowercase(), rest.unwrap_or("").to_string())
                    } else {
                        let inferred = if tail.ends_with(".json") {
                            "json"
                        } else if tail.ends_with(".jsonl") {
                            "jsonl"
                        } else {
                            "html"
                        };
                        (inferred.to_string(), tail.to_string())
                    }
                };
                let dest: std::path::PathBuf = if raw_path.is_empty() {
                    // default: <workspace>/.epsilon/exports/session-<id>.{html,json}
                    let id = epsilon_contracts::new_id("export");
                    let ext = if fmt == "html" { "html" } else { "json" };
                    self.workspace_root
                        .join(".epsilon")
                        .join("exports")
                        .join(format!("session-{id}.{ext}"))
                } else {
                    let p = std::path::Path::new(&raw_path);
                    if p.is_absolute() {
                        p.to_path_buf()
                    } else {
                        self.workspace_root.join(p)
                    }
                };
                if fmt == "html" {
                    match write_export_html(agent, &dest, &self.workspace_root) {
                        Ok(n) => self.ui.push_notice(format!(
                            "exported html → {} ({} entries)",
                            dest.display(),
                            n
                        )),
                        Err(e) => self.ui.push_notice(format!("export failed: {e}")),
                    }
                } else {
                    match write_export_json(agent, &dest) {
                        Ok(n) => self.ui.push_notice(format!(
                            "exported json → {} ({} entries)",
                            dest.display(),
                            n
                        )),
                        Err(e) => self.ui.push_notice(format!("export failed: {e}")),
                    }
                }
            }
            crate::picker::Command::Import(path) => {
                let p = path.trim();
                if p.is_empty() {
                    self.ui.push_notice(
                        "usage: /import <path> — switches session live, no restart needed (same as /resume)",
                    );
                } else if self.ui.running() || agent.is_running() {
                    self.ui.push_notice(
                        "import: busy — wait for the current run to finish, then /import again",
                    );
                } else {
                    let raw = std::path::PathBuf::from(p);
                    let abs = if raw.is_absolute() {
                        raw
                    } else {
                        self.workspace_root.join(raw)
                    };
                    if !abs.exists() {
                        self.ui.push_notice(format!(
                            "import: no file at '{}' — check the path",
                            abs.display()
                        ));
                    } else {
                        match self.switch_session_file(agent, &abs, Some(p)) {
                            Ok(count) => self.ui.push_notice(format!(
                                "imported {} messages from {} (store swapped)",
                                count,
                                abs.display()
                            )),
                            Err(e) => self.ui.push_notice(format!("import failed: {e}")),
                        }
                    }
                }
            }
            crate::picker::Command::Share => {
                // Pi `handleShareCommand` parity (ponytail: reuse write_export_html,
                // try `gh gist create --public=false` when present, honest guidance otherwise).
                let share_dest = self
                    .workspace_root
                    .join(".epsilon")
                    .join("exports")
                    .join(format!("share-{}.html", epsilon_contracts::new_id("share")));
                match write_export_html(agent, &share_dest, &self.workspace_root) {
                    Ok(n) => {
                        let gh_attempt = std::process::Command::new("gh")
                            .args(["gist", "create", "--public=false"])
                            .arg(&share_dest)
                            .output();
                        match gh_attempt {
                            Ok(out) if out.status.success() => {
                                let gist_url =
                                    String::from_utf8_lossy(&out.stdout).trim().to_string();
                                let gist_id = gist_url.rsplit('/').next().unwrap_or("").to_string();
                                let preview = std::env::var("PI_SHARE_VIEWER_URL")
                                    .unwrap_or_else(|_| "https://pi.dev/session/".to_string())
                                    + &gist_id;
                                if gist_id.is_empty() {
                                    self.ui.push_notice(format!(
                                        "share: exported html → {} ({} entries) · gist: {} (preview parse failed — copy the URL above)",
                                        share_dest.display(),
                                        n,
                                        gist_url
                                    ));
                                } else {
                                    self.ui.push_notice(format!(
                                        "share: gist {} · preview {} · file {} ({} entries)",
                                        gist_url,
                                        preview,
                                        share_dest.display(),
                                        n
                                    ));
                                }
                            }
                            Ok(out) => {
                                let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
                                let hint = if err.is_empty() {
                                    "unknown error".to_string()
                                } else {
                                    err
                                };
                                self.ui.push_notice(format!(
                                    "share: exported html → {} ({} entries) · gist not created ({}).\n       Publish the file or run: gh gist create --public=false {}",
                                    share_dest.display(),
                                    n,
                                    hint,
                                    share_dest.display()
                                ));
                            }
                            Err(e) => {
                                let hint = if e.kind() == std::io::ErrorKind::NotFound {
                                    "gh not installed — https://cli.github.com/ (then `gh auth login`)".to_string()
                                } else {
                                    format!("{e}")
                                };
                                self.ui.push_notice(format!(
                                    "share: exported html → {} ({} entries) · gist deferred ({}).\n       When gh is available: gh gist create --public=false {}",
                                    share_dest.display(),
                                    n,
                                    hint,
                                    share_dest.display()
                                ));
                            }
                        }
                    }
                    Err(e) => self.ui.push_notice(format!("share failed: {e}")),
                }
            }
            crate::picker::Command::Copy => {
                // Copy the last assistant message to the clipboard (pi
                // app.message.copy parity). Two paths, and the notice names the
                // one that landed: `arboard` reaches this machine's clipboard,
                // and OSC 52 asks the *terminal* to reach the one the user is
                // sitting at — the only path that works over SSH, where arboard
                // either fails or fills a clipboard nobody will paste from.
                let text = self
                    .ui
                    .lines()
                    .iter()
                    .rev()
                    .find(|l| l.kind == crate::state::LineKind::Assistant)
                    .map(|l| l.full.as_deref().unwrap_or(l.text.as_str()).to_string());
                match text {
                    Some(text) => {
                        let chars = text.chars().count();
                        // The sequence is handed to the *terminal*, so it is
                        // parked for the frame boundary rather than written from
                        // a key handler; `drive` writes it through the backend's
                        // own writer once the frame is out. A local rather than a
                        // `self`-capturing closure: the borrow checker is right
                        // that a closure holding `&mut self` across the call
                        // conflicts with the notice pushed after it.
                        let mut osc = None;
                        let copied = crate::clipboard::copy(
                            &text,
                            crate::clipboard::Multiplexer::detect(),
                            crate::clipboard::is_remote(),
                            |sequence| {
                                osc = Some(sequence.to_string());
                                Ok(())
                            },
                        );
                        self.osc_pending = osc;
                        self.ui
                            .push_notice(crate::clipboard::notice(chars, &copied));
                    }
                    None => self.ui.push_notice("copy: no assistant message to copy"),
                }
            }
            // `run_command` owns the routing, so reaching a variant here means the
            // dispatcher sent it to the wrong executor. `debug_assert` catches that in
            // every test build and stays silent in release - a user's release binary
            // must not panic over an internal routing mistake, and a test must not be
            // able to pass by quietly doing nothing.
            #[allow(unreachable_patterns)]
            other => {
                debug_assert!(false, "{other:?} is not a run_transfer_command case");
                return false;
            }
        }
        false
    }
}
