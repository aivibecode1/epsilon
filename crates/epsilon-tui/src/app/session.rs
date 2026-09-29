//! Construction, and switching which session file is in front of the user.
//!
//! ## Why this file exists
//!
//! `App::new` and the five methods that decide *which* session is loaded: where its file
//! lives, what the session browser lists, and what happens when the user resumes a
//! different one. They belong together because every one of them is a question about
//! identity - which session is this - as opposed to a question about its contents.
//!
//! ## The rule for this move
//!
//! Sliced by line range, with each boundary walked back from the next method's name and its
//! doc comment rather than from a regex over method heads. An earlier milestone derived
//! boundaries that way and silently missed four rustfmt-wrapped signatures; deriving them
//! from the three anchor methods (`insert_char`, `sync_slash_state`, `submit_editor`) makes
//! a miss impossible, because each boundary is one of them.
//!
//! Bodies are byte-identical to `app.rs` at the parent commit apart from the `pub(super)`
//! markers, which were collected from the compiler rather than guessed.

use super::*;

impl App {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
        model_id: String,
        fetched_tx: tokio::sync::mpsc::UnboundedSender<(String, Result<Vec<String>, String>)>,
        notices_tx: tokio::sync::mpsc::UnboundedSender<String>,
        oauth_tx: tokio::sync::mpsc::UnboundedSender<OauthUiEvent>,
    ) -> Self {
        let workspace_root =
            std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        Self {
            ui: UiState::new(),
            input: String::new(),
            cursor_chars: 0,
            history: VecDeque::new(),
            history_idx: None,
            pending_ask: None,
            approval_painted_id: None,
            trust_review_pending: false,
            guidance_mode: false,
            scroll_from_bottom: 0,
            search: None,
            pending_jump: None,
            viewport_rows: 0,
            geometry: TranscriptGeometry::default(),
            model_id,
            picker_open: false,
            picker_index: 0,
            session_browser: None,
            external_editor_requested: false,
            theme: crate::ui::theme::Theme::detect(),
            osc_pending: None,
            recents: epsilon_core::config::load_recent_models(),
            favorites: epsilon_core::config::load_favorite_models(),
            favorite_thinking: epsilon_core::config::load_favorite_thinking(),
            live_cache: Vec::new(),
            customs: epsilon_llm::custom::load_custom_providers(),
            live: None,
            prefs: crate::state::RenderPrefs::default(),
            fetching_hint: None,
            live_fetched: std::collections::BTreeSet::new(),
            fetched_tx,
            notices_tx,
            oauth_tx,
            oauth_code_tx: None,
            run_started_at: None,
            spinner_phase: 0,
            render_cache: crate::render_cache::RenderCache::new(),
            card_open: Vec::new(),
            card_restale: Vec::new(),
            picker_rows: None,
            kill_ring: crate::kill_ring::KillRing::new(),
            last_kill_was_contiguous: false,
            yank_in_progress: false,
            yanked_len: None,
            paste_payloads: std::collections::BTreeMap::new(),
            next_paste_id: 1,
            picker_provider: None,
            palette_open: false,
            slash_open: false,
            slash_index: 0,
            at_open: false,
            at_index: 0,
            at_rows: None,
            at_fragment: String::new(),
            timeline_open: false,
            timeline_index: 0,
            timeline_entries: Vec::new(),
            jump_mode: None,
            tree_filter: crate::app::TreeFilter::default(),
            tree_query: String::new(),
            tree_searching: false,
            folded: std::collections::HashSet::new(),
            timeline_all: Vec::new(),
            label_prompt_open: false,
            label_prompt_target: None,
            label_prompt_text: String::new(),
            label_prompt_cursor: 0,
            compact_running: false,
            stash: None,
            undo: crate::undo::UndoStack::new(),
            branch_cache: (
                None,
                std::time::Instant::now() - std::time::Duration::from_secs(10),
            ),
            // `None` means "never loaded": the first `@` pays for the search once, and
            // every keystroke after it reads the cached rules.
            at_ignore: None,
            workspace_root,
            // Populated immediately below; see `discover_prompt_templates`.
            prompt_templates: Vec::new(),
            keybindings: crate::keybindings::KeybindingsManager::create(),
            thinking: epsilon_contracts::ThinkingLevel::default(),
            session_name: None,
            steer_queue_len: 0,
            auth_prompt_open: false,
            auth_provider: String::new(),
            auth_key: String::new(),
            auth_key_cursor: 0,
            auth_pick_open: false,
            auth_pick_index: 0,
        }
    }

    /// Git branch with a short TTL so we read `.git/HEAD` at most every 2s (pi's
    /// footer-data-provider caching, minus the watcher).
    pub(super) fn cached_branch(&mut self) -> Option<String> {
        const TTL: std::time::Duration = std::time::Duration::from_secs(2);
        if self.branch_cache.1.elapsed() > TTL {
            let b = crate::git::branch(&self.workspace_root);
            self.branch_cache = (b, std::time::Instant::now());
        }
        self.branch_cache.0.clone()
    }

    /// M57: the workspace's session directory (`--continue` scans the same one).
    pub(super) fn session_dir(&self) -> std::path::PathBuf {
        self.workspace_root.join(".epsilon").join("sessions")
    }

    /// M57: open the session browser (newest first, current one marked).
    pub(super) fn open_session_browser(&mut self, agent: &Arc<Agent>) {
        if self.ui.running() || agent.is_running() {
            self.ui
                .push_notice("sessions: busy — wait for the current run to finish");
            return;
        }
        let current = agent.session_path();
        let dir = self.session_dir();
        if !dir.is_dir() {
            self.ui.push_notice(format!(
                "sessions: no session directory at '{}' yet — one appears after the first run",
                dir.display()
            ));
            return;
        }
        let browser = SessionBrowser::build(dir, current, "");
        if browser.rows.is_empty() {
            self.ui.push_notice(
                "sessions: nothing to resume here — the directory holds no session files",
            );
            return;
        }
        self.input.clear();
        self.cursor_chars = 0;
        self.session_browser = Some(browser);
    }

    /// M59: ask the event loop to open `$EDITOR` on the current draft.
    ///
    /// Resolution happens HERE so the user gets an immediate, actionable notice
    /// instead of a terminal hand-off that fails invisibly.
    pub(super) fn request_external_editor(&mut self) {
        if self.ui.running() || self.session_browser.is_some() || self.picker_open {
            self.ui
                .push_notice("editor: finish the current dialog or run first");
            return;
        }
        let command = editor_command(epsilon_core::config::load_config_string(
            "editor", "command",
        ));
        match command {
            Ok(command) => {
                self.ui.push_notice(format!(
                    "opening {command} — the UI returns when the editor exits"
                ));
                self.external_editor_requested = true;
            }
            Err(e) => self.ui.push_notice(format!("editor: {e}")),
        }
    }

    /// M57: switch this app onto another session file — the ONE switch path,
    /// shared by `/resume <path>`, `/import`, and the browser's Enter, so the
    /// three surfaces cannot drift apart. Returns the restored message count;
    /// callers keep their own notice wording (each command names itself).
    pub(super) fn switch_session_file(
        &mut self,
        agent: &Arc<Agent>,
        path: &std::path::Path,
        typed: Option<&str>,
    ) -> Result<usize, String> {
        let count = agent.live_resume(path)?;
        {
            let history = agent.history();
            self.ui.hydrate_from_history(&history);
            // M144/F22: summarization bills are not messages, so `live_resume`'s history
            // does not contain them and `hydrate_from_history` cannot see them. They are
            // read from the store and folded in *after* hydration, which clears the rows —
            // adding them first would silently discard them. Compaction is a real provider
            // call; a resumed session that hid it would under-report the one number a user
            // opens a long session to check.
            if let Some(store) = agent.session_store() {
                match store.lock() {
                    Ok(st) => match st.usage_accounts() {
                        Ok(rows) => self.ui.add_usage_accounts(&rows),
                        // A read failure must not fail the resume: the transcript is
                        // already hydrated, and a user can act on a session whose cost
                        // under-reports far better than on one that refuses to open.
                        Err(e) => self
                            .ui
                            .push_notice(format!("session cost totals may under-report: {e}")),
                    },
                    Err(_) => self.ui.push_notice(
                        "session cost totals may under-report: the session file is locked"
                            .to_string(),
                    ),
                }
            }
            if let Some(shown) = typed.filter(|s| !s.is_empty()) {
                self.history.push_back(shown.to_string());
                while self.history.len() > HISTORY_MAX {
                    self.history.pop_front();
                }
            }
            // Re-hydrate session_name from the newly attached store (survives swap).
            self.session_name = (|| {
                let store = agent.session_store()?;
                let st = store.lock().ok()?;
                let entries = st.read_all().ok()?;
                for e in entries.iter().rev() {
                    if let epsilon_contracts::session_entry::EntryKind::Custom { custom_type, data } =
                        &e.kind
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
        }
        Ok(count)
    }
}
