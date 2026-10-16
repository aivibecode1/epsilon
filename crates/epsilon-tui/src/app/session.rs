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
    pub(super) fn new(
        model_id: String,
        fetched_tx: tokio::sync::mpsc::UnboundedSender<(String, Result<Vec<String>, String>)>,
        notices_tx: tokio::sync::mpsc::UnboundedSender<String>,
        oauth_tx: tokio::sync::mpsc::UnboundedSender<OauthUiEvent>,
    ) -> Self {
        let workspace_root =
            std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        // The footer's location label, one per repertoire: the shortening reads the
        // environment (the home prefix) and the ellipsis mark, and the mark is the one
        // thing the draw's own theme decides. Both are derived here, once — see the
        // field's doc for the round-105 defect the second variant fixes.
        let location_labels = {
            let home = std::env::var_os("HOME")
                .or_else(|| std::env::var_os("USERPROFILE"))
                .map(std::path::PathBuf::from);
            [
                crate::ui::format::format_path(
                    &workspace_root,
                    home.as_deref(),
                    28,
                    crate::ui::theme::UNICODE.ellipsis,
                ),
                crate::ui::format::format_path(
                    &workspace_root,
                    home.as_deref(),
                    28,
                    crate::ui::theme::ASCII.ellipsis,
                ),
            ]
        };
        // Thinking visibility is UI state, not app state: the push sites that
        // must skip are `UiState` methods, and a flag anywhere else would be a
        // second source of truth to keep in sync. Loaded once here like the
        // other display preferences; flipped by `app.thinking.toggle`.
        let mut ui = UiState::new();
        ui.hide_thinking = epsilon_core::config::load_hide_thinking();
        // Criterion 6: persisted live listings load at startup and read as
        // cached until a fetch replaces them. Corrupt files delete-and-continue
        // inside the loader, never an error; faux (non-`real`) builds ignore
        // the file entirely and browse the bundle only. Reads the global dir
        // directly (not `config_dir_override`): the override is test-only and
        // always `None` in production, so load and persist resolve the same
        // dir wherever it matters. A zero stamp means "never" (pre-epoch
        // clocks, hand-edited files), not "just now" — those rows are dropped
        // here so no age line can claim a fresh fetch that never happened.
        #[cfg(feature = "real")]
        let live_cache: Vec<LiveCacheEntry> = epsilon_core::config::global_dir()
            .map(|dir| epsilon_core::config::load_live_cache_in(&dir))
            .unwrap_or_default()
            .into_iter()
            .filter(|e| e.fetched_at_ms > 0)
            .map(|e| LiveCacheEntry {
                provider: e.provider,
                ids: e.ids,
                fetched_at_ms: e.fetched_at_ms,
                from_cache: true,
            })
            .collect();
        #[cfg(not(feature = "real"))]
        let live_cache: Vec<LiveCacheEntry> = Vec::new();
        let mut app = Self {
            ui,
            input: String::new(),
            cursor_chars: 0,
            history: VecDeque::new(),
            history_idx: None,
            pending_ask: None,
            approval_painted_id: None,
            ask_slot: Default::default(),
            trust_review_pending: false,
            pending_import: None,
            pending_preset: None,
            guidance_mode: false,
            scroll_from_bottom: 0,
            filter: crate::state::TranscriptFilter::default(),
            search: None,
            pending_jump: None,
            viewport_rows: 0,
            geometry: TranscriptGeometry::default(),
            model_id,
            picker_open: false,
            // M233: set from the CLI's decision after `App::new`, like `live` and `theme`. The
            // default here is "not known", which the header must not read as "online".
            offline: false,
            picker_index: 0,
            // One until the first paint teaches it the real window; see the field's doc.
            picker_capacity: 1,
            session_browser: None,
            external_editor_requested: false,
            theme: crate::ui::theme::Theme::detect(),
            osc_pending: None,
            recents: epsilon_core::config::load_recent_models(),
            favorites: epsilon_core::config::load_favorite_models(),
            default_model: epsilon_core::config::load_model_id(),
            config_dir_override: None,
            favorite_thinking: epsilon_core::config::load_favorite_thinking(),
            live_cache,
            customs: epsilon_llm::custom::custom_providers(),
            live: None,
            prefs: crate::state::RenderPrefs::default(),
            live_fetching: std::collections::BTreeSet::new(),
            live_fetched: std::collections::BTreeSet::new(),
            fetched_tx,
            notices_tx,
            oauth_tx,
            // Replaced by `set_goal_round_channel` before the first frame. A channel built
            // here would have no reader: the receiver belongs to `drive`, which is the only
            // place that drains it.
            goal_round_tx: tokio::sync::mpsc::unbounded_channel().0,
            goal_round_rx: tokio::sync::mpsc::unbounded_channel().1,
            gate_runner: None,
            goal_round_pending: false,
            oauth_code_tx: None,
            run_started_at: None,
            spinner_phase: 0,
            render_cache: crate::render_cache::RenderCache::new(),
            card_open: Vec::new(),
            card_restale: std::collections::HashSet::new(),
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
            timeline_fork_refill: None,
            timeline_capacity: 1,
            timeline_entries: Vec::new(),
            tree_open: false,
            tree_cursor: 0,
            tree_rows: Vec::new(),
            tree_collapsed: std::collections::BTreeSet::new(),
            tree_capped: false,
            viewer_tabs: Vec::new(),
            viewer_tab: 0,
            viewer_capacity: 0,
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
            compact_running: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            stash: None,
            undo: crate::undo::UndoStack::new(),
            branch_cache: (
                None,
                std::time::Instant::now() - std::time::Duration::from_secs(10),
            ),
            // `None` means "never loaded": the first `@` pays for the search once, and
            // every keystroke after it reads the cached rules.
            at_ignore: None,
            // A quarter of the row is the most the location may take before the
            // hints stop being readable; past that the tail of the path is what
            // matters. Derived once — see the field's doc.
            location_labels,
            workspace_root,
            // Nothing has read the session's goal yet, and a fresh process has not armed one:
            // the badge appears the first time something looks, which is a `/goal` command or
            // the end of the first turn.
            goal_badge: None,
            // Populated immediately below; see `discover_prompt_templates`.
            prompt_templates: Vec::new(),
            // Populated at startup and on `/reload`; see `refresh_skills`.
            skills_cache: Vec::new(),
            keybindings: crate::keybindings::KeybindingsManager::create(),
            // The persisted `[thinking] level` is the default a launch starts
            // from — `Off` before this key existed, because nothing was ever
            // written and nothing was ever read. `set_thinking` is the writer
            // that keeps this mirror and the agent equal (M25), so what the
            // footer shows is what the run uses. A project's own value, when a
            // workspace has one, reaches the agent through composition
            // (`resolve_default_thinking_level`) and outranks this global read.
            thinking: epsilon_core::config::load_thinking_level().unwrap_or_default(),
            session_name: None,
            steer_queue_len: 0,
            auth_prompt_open: false,
            auth_provider: String::new(),
            auth_key: String::new(),
            auth_key_cursor: 0,
            auth_pick_open: false,
            auth_pick_index: 0,
            think_pick_open: false,
            think_pick_index: 0,
            changes_open: false,
            changes_index: 0,
            changes_rows: Vec::new(),
            changes_scope: String::new(),
            settings_open: false,
            settings_index: 0,
            settings_preset: String::new(),
            // Criterion 4 mirror for the settings panel's plan row (see
            // `settings_preset` above); synced when `/settings` opens.
            settings_plan: false,
            has_run_command: false,
            scope_open: false,
            scope_index: 0,
            scope_snapshot: Vec::new(),
            anthropic_billing_warned: false,
        };
        // A keybindings file that could not be used names itself at startup
        // (S-126, terminal half): every rebind in it is silently not applied
        // otherwise. Pushed here — not inside the literal above, which has
        // already moved `ui` — as a transcript line on the first frame, not a
        // print lost above the alternate screen.
        if let Some(warning) = app.keybindings.load_warning() {
            app.ui.push_notice(warning.to_string());
        }
        app
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
        let browser = SessionBrowser::build(dir, self.workspace_root.clone(), current, "");
        // An empty directory OPENS (P-243): pi shows "No named sessions…"
        // with the key that starts one, and a refusal would hide exactly
        // the message that teaches it. The draw shows the empty state.
        self.input.clear();
        self.cursor_chars = 0;
        self.session_browser = Some(browser);
    }

    /// Phase 3 (TUI as IDE): workspace tree sidebar (F6). Opens focused and
    /// rebuilds from the workspace; closing returns the keyboard to the composer.
    pub(super) fn toggle_tree(&mut self) {
        self.tree_open = !self.tree_open;
        if self.tree_open {
            self.tree_cursor = 0;
            self.refresh_tree();
        }
    }

    /// Rebuild the sidebar rows from the workspace through the shared core
    /// walk — same traversal, same ignore rules as `@` completion, with the
    /// tree's own row budget (`tree.rs`). A workspace that cannot be read is
    /// an empty tree with an honest empty state, not an error: the walk
    /// already returns no rows there, and refusing to open would hide exactly
    /// the message that teaches it (the P-243 rule, applied here).
    pub(super) fn refresh_tree(&mut self) {
        let ignore = epsilon_core::ignore::IgnoreSet::load(
            &self.workspace_root,
            crate::at_files::AT_IGNORE_FILES,
        );
        let mut budget = crate::tree::TREE_ROW_BUDGET;
        let paths = epsilon_core::walk::walk_files(&self.workspace_root, &ignore, &mut budget);
        self.tree_capped = paths.len() >= crate::tree::TREE_ROW_BUDGET;
        self.tree_rows = crate::tree::build_visible_rows(&paths, &self.tree_collapsed);
        if self.tree_cursor >= self.tree_rows.len() {
            self.tree_cursor = self.tree_rows.len().saturating_sub(1);
        }
    }

    /// Whether the read-only file viewer is open.
    pub(super) fn viewer_open(&self) -> bool {
        !self.viewer_tabs.is_empty()
    }

    /// The visible tab, if any.
    pub(super) fn active_viewer(&self) -> Option<&crate::tree::FileViewer> {
        self.viewer_tabs.get(self.viewer_tab)
    }

    /// The visible tab, mutably.
    ///
    /// Every key and draw path reaches the viewer through this, so a second
    /// copy of "the current file" cannot drift beside the tabs.
    pub(super) fn active_viewer_mut(&mut self) -> Option<&mut crate::tree::FileViewer> {
        self.viewer_tabs.get_mut(self.viewer_tab)
    }

    /// Whether the viewer is in edit mode.
    pub(super) fn viewer_editing(&self) -> bool {
        self.active_viewer().is_some_and(|v| v.editing)
    }

    /// Open `rel` in a tab, focusing it when already open (Phase 3, tabs).
    pub(super) fn open_viewer_tab(&mut self, viewer: crate::tree::FileViewer) {
        if let Some(i) = self.viewer_tabs.iter().position(|t| t.rel == viewer.rel) {
            self.viewer_tab = i;
        } else {
            self.viewer_tabs.push(viewer);
            self.viewer_tab = self.viewer_tabs.len() - 1;
        }
        self.viewer_capacity = 0;
    }

    /// Close the visible tab; keeping `viewer_tab` a valid index, or empty.
    pub(super) fn close_viewer_tab(&mut self) {
        if self.viewer_tabs.is_empty() {
            return;
        }
        self.viewer_tab = self.viewer_tab.min(self.viewer_tabs.len() - 1);
        self.viewer_tabs.remove(self.viewer_tab);
        if !self.viewer_tabs.is_empty() {
            self.viewer_tab = self.viewer_tab.min(self.viewer_tabs.len() - 1);
        }
    }

    /// Step the visible tab, wrapping past both ends.
    pub(super) fn switch_viewer_tab(&mut self, forward: bool) {
        if self.viewer_tabs.is_empty() {
            return;
        }
        let n = self.viewer_tabs.len();
        if forward {
            self.viewer_tab = (self.viewer_tab + 1) % n;
        } else {
            self.viewer_tab = (self.viewer_tab + n - 1) % n;
        }
    }

    /// Save the viewer's edit buffer through the checkpoint store, so `/restore`
    /// undoes a human save exactly like an agent edit — one undo, via
    /// `epsilon_runtime::checkpoint_save` (the shipped TUI cannot name the tool
    /// crate, so the runtime owns the store edge, as it already does for restore).
    /// The changed-underneath guard and the keep-the-buffer-on-failure rule stay:
    /// a refused or failed save loses nothing and says why.
    pub(super) fn save_viewer(&mut self) {
        let Some(viewer) = self.active_viewer() else {
            return;
        };
        if !viewer.editing {
            return;
        }
        let rel = viewer.rel.clone();
        let text = viewer.buffer_text();
        if rel.contains("..") {
            self.ui
                .push_notice(format!("{rel}: refusing to write outside the workspace"));
            return;
        }
        let path = self.workspace_root.join(&rel);
        let opened = viewer.opened_bytes.clone();
        match std::fs::read(&path) {
            Ok(current) if current != opened => {
                self.ui.push_notice(format!(
                    "{rel} changed on disk since it was opened — re-open it (Enter) instead of overwriting"
                ));
                return;
            }
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                self.ui.push_notice(format!(
                    "save failed ({e}) — your edits are still in the buffer"
                ));
                return;
            }
            _ => {}
        }
        // No `.tui-bak` sidecar: the checkpoint entry IS the way back, and a
        // second undo format is what this goal removes (ADR-040).
        if let Err(e) = epsilon_runtime::checkpoint_save(
            &self.workspace_root,
            &path,
            text.as_bytes(),
            "tui-edit",
        ) {
            self.ui.push_notice(format!(
                "save failed ({e}) — your edits are still in the buffer"
            ));
            return;
        }
        if let Some(viewer) = self.active_viewer_mut() {
            viewer.lines = viewer.edit_lines.clone();
            viewer.opened_bytes = text.into_bytes();
            viewer.editing = false;
            viewer.discard_armed = false;
            viewer.scroll = viewer.scroll.min(viewer.lines.len().saturating_sub(1));
        }
        self.refresh_tree();
        self.ui
            .push_notice(format!("saved {rel} — `/restore {rel}` puts it back"));
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
        // The session's own records — model and thinking level — applied
        // through the ONE applier the launch path runs (`apply_resumed_context`,
        // ADR-021), so a live switch and a `--resume` launch cannot drift apart.
        // Read here, applied before hydration clears the rows so the notices
        // below survive it. `model_explicit` has no meaning on a live switch —
        // the launch already happened — so it is `false`.
        let resumed = agent.session_store().map(|store| {
            epsilon_runtime::apply_resumed_context(agent, &store, &self.model_id, false)
        });
        // Read now, push after hydration below clears the rows: a recorded
        // directory the run does not share silently reinterprets relative
        // paths, and a recorded directory that no longer exists makes it
        // certain (round 36).
        let cwd_note = epsilon_core::Agent::resume_cwd_note(path, &self.workspace_root);
        {
            let history = agent.history();
            self.ui.hydrate_from_history(&history);
            if let Some(note) = cwd_note {
                self.ui.push_notice(note);
            }
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
            // P-054/P-183 replay half: compactions are entries, not messages, so the
            // markers hydration left behind regain their digests here, beside the
            // bills — one helper for every hydrating path, so they cannot drift.
            self.ui.hydrate_compaction_digests(agent);
            // P-055: opening a session whose history contains compactions says
            // so — a resumed conversation reads differently when parts of it
            // are summaries, and nothing else on screen names that. Counted
            // from the store like the bills above (compactions are entries,
            // not messages, so hydration cannot see them). A read failure is
            // silent: the transcript already hydrated, and a count no action
            // can follow is not worth noise.
            if let Some(store) = agent.session_store()
                && let Ok(st) = store.lock()
                && let Ok(entries) = st.read_all()
            {
                let n = entries
                    .iter()
                    .filter(|e| {
                        matches!(
                            e.kind,
                            epsilon_contracts::session_entry::EntryKind::Compaction { .. }
                        )
                    })
                    .count();
                if n > 0 {
                    self.ui.push_notice(format!(
                        "Session compacted {} time{}",
                        n,
                        if n == 1 { "" } else { "s" }
                    ));
                }
            }
            if let Some(shown) = typed.filter(|s| !s.is_empty()) {
                self.history.push_back(shown.to_string());
                while self.history.len() > HISTORY_MAX {
                    self.history.pop_front();
                }
            }
            // Re-hydrate session_name from the newly attached store (survives swap).
            self.hydrate_session_name(agent);
            // The title tracks what just attached (P-017).
            self.refresh_title();
        }
        // The frontend follows the composed agent (audit §1a/§5): the model the
        // session was last used on moves the mirror the header, the picker and
        // `/session` read, and the recorded level moves the footer's. Without
        // this the two halves of the resume disagree — the switch restored the
        // run's model and left the screen naming the old one.
        if let Some(applied) = resumed {
            if let Some((provider, model)) = &applied.model {
                self.model_id = model.clone();
                self.invalidate_picker();
                self.ui.push_notice(format!(
                    "session was last used on {provider}/{model}; continuing there"
                ));
            }
            if let Some(level) = applied.thinking {
                self.thinking = level;
                self.ui
                    .push_notice(format!("thinking level: {}", level.as_str()));
            }
        }
        Ok(count)
    }

    /// Re-read the display name from the attached store, if it holds one.
    ///
    /// Extracted from `switch_session_file` unchanged: startup needs the same
    /// read (a resumed named session would otherwise title itself nameless),
    /// and two copies of a store scan are how they disagree.
    pub(super) fn hydrate_session_name(&mut self, agent: &Arc<Agent>) {
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

    /// Emit the terminal title for the current session (P-017).
    ///
    /// Called after every change of what it names: switch, rename, startup.
    /// Best-effort by design — a title that cannot be written (stdout gone) is
    /// not worth failing a switch over.
    pub(super) fn refresh_title(&self) {
        let base = self
            .workspace_root
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.workspace_root.to_string_lossy().into_owned());
        let title = terminal_title(self.session_name.as_deref(), &base);
        let _ = crossterm::execute!(std::io::stdout(), crossterm::terminal::SetTitle(title));
    }
}

/// Terminal title tracking session name + cwd basename (P-017):
/// `epsilon - {name} - {base}`, or `epsilon - {base}`.
///
/// Control characters are stripped: the name is user-typed, and a BEL or ESC
/// in it would break out of the OSC sequence. Pure, so the wording (including
/// what a hostile name becomes) is pinned by unit tests rather than by review.
pub(super) fn terminal_title(session_name: Option<&str>, cwd_basename: &str) -> String {
    use epsilon_runtime::text_safety::display_safe;
    let clean = |s: &str| display_safe(s).0.into_owned();
    match session_name.map(clean).filter(|n| !n.trim().is_empty()) {
        Some(name) => format!("epsilon - {name} - {}", clean(cwd_basename)),
        None => format!("epsilon - {}", clean(cwd_basename)),
    }
}

#[cfg(test)]
mod title_tests {
    use super::terminal_title;

    #[test]
    fn title_tracks_name_and_base() {
        assert_eq!(
            terminal_title(Some("work"), "proj"),
            "epsilon - work - proj"
        );
        assert_eq!(terminal_title(None, "proj"), "epsilon - proj");
        assert_eq!(terminal_title(Some("   "), "proj"), "epsilon - proj");
    }

    #[test]
    fn title_strips_terminal_breakouts() {
        // A session name is user-typed: BEL/ESC/C0/bidi must not survive into
        // the OSC sequence, while readable text does.
        let hostile = "a\x1bb\x07c\x00d\u{202e}e";
        let title = terminal_title(Some(hostile), "base\x1b]");
        for c in title.chars() {
            assert!(
                !('\u{0}'..='\u{1f}').contains(&c) && c != '\u{7f}' && c != '\u{202e}',
                "control character survived into the title: {title:?}"
            );
        }
        assert!(
            title.contains('a') && title.contains("base"),
            "readable text must survive the stripping: {title:?}"
        );
    }
}
