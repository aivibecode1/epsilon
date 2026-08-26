//! Terminal runtime: the event loop and all rendering.
//!
//! This crate is a pure frontend: it owns NO runtime composition. The CLI builds the
//! agent (provider, tools, ruleset gate) and hands over [`TuiConfig`]; permission asks
//! flow through the shared [`ChannelApprovalUi`], answered directly on the parked
//! [`PendingAsk`].

use std::collections::VecDeque;
use std::sync::Arc;

use crossterm::event::{
    Event as TermEvent, EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers,
};
use epsilon_contracts::agent_event::{AgentEvent, PermissionDecision};
use epsilon_contracts::content::UserContent;
use epsilon_contracts::message::{Message, UserMessage};
use epsilon_contracts::new_id;
use epsilon_core::Agent;
use epsilon_safety::{ChannelApprovalUi, PendingAsk};
use futures_util::StreamExt;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::state::UiState;
use crate::theme::Theme;

/// Async provider-listing hook injected by the CLI when built with `--features real`
/// (M13; provider-name arg since M20): takes a PROVIDER name (`"anthropic"`,
/// `"openrouter"`, ...), returns that provider's live model ids. Errors in-band.
pub type LiveFetcher = Arc<
    dyn Fn(String) -> futures_util::future::BoxFuture<'static, Result<Vec<String>, String>>
        + Send
        + Sync,
>;

/// Everything the frontend needs: an agent to drive and its approval queue.
pub struct TuiConfig {
    /// The composed session runtime.
    pub agent: Arc<Agent>,
    /// Where the gate parks asks; must be the same instance inside the agent's gate.
    pub approvals: Arc<ChannelApprovalUi>,
    /// Task submitted automatically once the UI is live.
    pub initial_task: Option<String>,
    /// Active model id (already resolved by the CLI through flag > config > default).
    pub model_id: String,
    /// Live per-provider listing capability; absent without the `real` feature.
    pub live: Option<LiveFetcher>,
    /// Startup diagnostics flushed as notices once the UI is live (project
    /// config notices, trust banners). M22: also carries the trust re-prompt
    /// trigger — a `TrustDecision::Changed` line makes the TUI ask before the
    /// first run proceeds (pi trust-manager semantics, ADR-012).
    pub startup_notices: Vec<String>,
    /// M22 (audit): typed trust-review trigger. The TUI must not sniff banner
    /// strings for security behavior — rewording copy would silently disable
    /// the gate.
    pub trust_review_pending: bool,
}

/// Redraw cadence (spinner/clock resolution).
const TICK_MS: u64 = 120;

/// Editor history depth.
const HISTORY_MAX: usize = 100;

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn summarize_detail(detail: &serde_json::Value) -> String {
    let s = detail.to_string();
    if s.chars().count() <= 96 {
        s
    } else {
        let head: String = s.chars().take(96).collect();
        format!("{head}…")
    }
}

/// Terminal runtime state (crate-private; the bench module drives it through
/// [`bench_app`] + [`draw_frame`]).
pub(crate) struct App {
    ui: UiState,
    input: String,
    cursor_chars: usize,
    history: VecDeque<String>,
    history_idx: Option<usize>,
    pending_ask: Option<PendingAsk>,
    /// M22 / DIRECTION-01: set when the project config changed since trust;
    /// blocks submits until the user answers (`trust` / `trust no`).
    trust_review_pending: bool,
    guidance_mode: bool,
    scroll_from_bottom: usize,
    model_id: String,
    picker_open: bool,
    picker_index: usize,
    recents: Vec<String>,
    /// Favorite (starred) model ids, persisted in config `[picker] favorite`
    /// (M23; opencode favorites / pi scoped-models hybrid).
    favorites: Vec<String>,
    /// M26: per-favorite thinking levels `[picker.favorite_thinking]`
    /// (pi scoped-models' optional per-entry level), loaded at startup and
    /// refreshed by toggle_favorite. Kept in memory so cycling never hits
    /// disk mid-keypress (and tests can inject without touching global state).
    favorite_thinking: Vec<(String, String)>,
    /// M13/M20: per-provider live listing results (provider → ids), merged into
    /// both picker stages on top of the bundled catalog.
    live_cache: Vec<(String, Vec<String>)>,
    /// M28: user-declared custom providers (`[provider_custom.<name>]`),
    /// loaded once at startup; threaded into every picker row builder so
    /// configured endpoints are visible and descendable in the UI.
    customs: Vec<epsilon_llm::custom::CustomProvider>,
    live: Option<LiveFetcher>,
    prefs: crate::state::RenderPrefs,
    fetching_hint: Option<String>,
    /// M20: providers whose live listing already landed this session (auto-fetch
    /// runs once per provider; Ctrl+F force-refreshes regardless).
    live_fetched: std::collections::BTreeSet<String>,
    fetched_tx: tokio::sync::mpsc::UnboundedSender<(String, Result<Vec<String>, String>)>,
    notices_tx: tokio::sync::mpsc::UnboundedSender<String>,
    /// M15 spinner/elapsed: set on AgentStart, cleared on AgentEnd.
    run_started_at: Option<std::time::Instant>,
    spinner_phase: usize,
    /// M16a: markdown+wrap happens once per line here, never in the draw loop.
    render_cache: crate::render_cache::RenderCache,
    /// M16a: picker rows cached per (filter, sources) tuple; rebuilt only when one
    /// of those changes, never on Up/Down.
    picker_rows: Option<Vec<crate::picker::PickerRow>>,
    /// M16b: Emacs kill-ring for Ctrl+U/Ctrl+W kills + Ctrl+Y yank.
    kill_ring: crate::kill_ring::KillRing,
    /// M16b: true while kills are consecutive (accumulate into one ring entry).
    last_kill_was_contiguous: bool,
    /// M16b: a Ctrl+Y sequence is active (repeat cycles entries).
    yank_in_progress: bool,
    /// M16b: char-length of the text the current yank inserted.
    yanked_len: Option<usize>,
    /// M16b: pasted payloads keyed by marker id, expanded on submit.
    paste_payloads: std::collections::BTreeMap<usize, String>,
    /// M16b: next marker id.
    next_paste_id: usize,
    /// M16c: picker stage — None = single-stage legacy list, Some(provider) = stage 2.
    picker_provider: Option<String>,
    /// M16c: command palette open (Ctrl+P).
    palette_open: bool,
    /// M17b: slash-autocomplete popup active (drives Up/Down/Tab/Enter interception).
    slash_open: bool,
    /// M17b: selected index within the open slash popup.
    slash_index: usize,
    /// M16c: draft stashed when a dialog opened; restored when it closes.
    stash: Option<(String, usize)>,
    /// Parity sweep: editor undo stack (Ctrl+Z), pi undo-stack.ts semantics.
    undo: crate::undo::UndoStack,
    /// Workspace root for the footer (pi footer parity).
    workspace_root: std::path::PathBuf,
    /// Cached git branch + when it was read (2s TTL — no git dependency).
    branch_cache: (Option<String>, std::time::Instant),
    /// M25: mirror of the agent's thinking level for the footer. Written ONLY
    /// by [`Self::set_thinking`] (the same choke point that updates the agent),
    /// so display and execution state cannot drift apart.
    thinking: epsilon_contracts::ThinkingLevel,
}

impl App {
    /// Invalidate the picker row cache (call whenever filter/recents/live change).
    fn invalidate_picker(&mut self) {
        self.picker_rows = None;
    }

    /// Current picker rows, filtered+ordered (cached; see [`Self::invalidate_picker`]).
    fn picker_rows(&mut self) -> Vec<crate::picker::PickerRow> {
        if let Some(rows) = &self.picker_rows {
            return rows.clone();
        }
        let rows = crate::picker::build_rows_with(
            &self.input,
            &self.model_id,
            &self.recents,
            &self.favorites,
            &self.live_cache,
            &self.customs,
        );
        self.picker_rows = Some(rows.clone());
        rows
    }

    /// Toggle the favorite star on a model id (M23, opencode toggleFavorite):
    /// persists immediately and reports the NEW state for the notice.
    ///
    /// M26 (pi scoped-models' per-entry thinking level): starring remembers the
    /// CURRENT session level for that model; unstarring drops the memory. The
    /// level is captured through `agent`, which is also why this takes the
    /// handle instead of reading it in the caller.
    fn toggle_favorite(&mut self, agent: &Arc<Agent>, id: &str) {
        match epsilon_core::config::toggle_favorite_model(id) {
            Ok(now_favorite) => {
                let level = epsilon_core::config::set_favorite_thinking(
                    id,
                    if now_favorite {
                        Some(agent.thinking_level().as_str())
                    } else {
                        None
                    },
                );
                if let Err(e) = level {
                    self.ui
                        .push_notice(format!("thinking memory not saved: {e}"));
                }
                self.favorites = epsilon_core::config::load_favorite_models();
                self.favorite_thinking = epsilon_core::config::load_favorite_thinking();
                self.invalidate_picker();
                self.ui.push_notice(if now_favorite {
                    format!(
                        "★ {id} favorited at think:{} (Ctrl+P cycles favorites)",
                        agent.thinking_level().as_str()
                    )
                } else {
                    format!("☆ {id} unfavorited")
                });
            }
            Err(e) => {
                self.ui.push_notice(format!("favorite not saved: {e}"));
            }
        }
    }

    /// Cycle to the next/previous favorite model (M23; pi's cycleModel over
    /// scoped models — opencode's `cycleFavorite`). No-op with guidance when
    /// no favorites exist.
    fn cycle_favorite(&mut self, agent: &Arc<Agent>, direction: isize) {
        if self.favorites.is_empty() {
            self.ui
                .push_notice("no favorites yet — star one in the picker (Ctrl+F)");
            return;
        }
        let len = self.favorites.len() as isize;
        let idx = self
            .favorites
            .iter()
            .position(|f| f.eq_ignore_ascii_case(&self.model_id));
        // pi semantics: current-not-in-list enters at the forward end.
        let next = match idx {
            Some(i) => {
                let n = (i as isize + direction).rem_euclid(len);
                self.favorites[n as usize].clone()
            }
            None if direction > 0 => self.favorites[0].clone(),
            None => self.favorites[(len - 1) as usize].clone(),
        };
        if next.eq_ignore_ascii_case(&self.model_id) {
            return; // single-entry list: already there
        }
        // M26 (pi `_cycleScopedModel`, agent-session.ts:1625-1647): an explicit
        // per-favorite level overrides the session level; no memory inherits it.
        // Read from the in-memory mirror (loaded at startup, refreshed on star
        // toggle) so cycling stays a pure keypress operation.
        if let Some(level) = self
            .favorite_thinking
            .iter()
            .find(|(id, _)| id == &next)
            .and_then(|(_, lvl)| epsilon_contracts::ThinkingLevel::parse(lvl))
        {
            self.set_thinking(agent, level);
        }
        self.apply_model(agent, next);
    }
}

/// Braille spinner frames (pi loader convention).
const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// M16b acceptance: editor grows to fit the draft up to this many rows, then
/// scrolls internally (tail visible — that's where the caret lives).
const INPUT_MAX_ROWS: usize = 8;

impl App {
    #[allow(clippy::too_many_arguments)]
    fn new(
        model_id: String,
        fetched_tx: tokio::sync::mpsc::UnboundedSender<(String, Result<Vec<String>, String>)>,
        notices_tx: tokio::sync::mpsc::UnboundedSender<String>,
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
            trust_review_pending: false,
            guidance_mode: false,
            scroll_from_bottom: 0,
            model_id,
            picker_open: false,
            picker_index: 0,
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
            run_started_at: None,
            spinner_phase: 0,
            render_cache: crate::render_cache::RenderCache::new(),
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
            stash: None,
            undo: crate::undo::UndoStack::new(),
            branch_cache: (
                None,
                std::time::Instant::now() - std::time::Duration::from_secs(10),
            ),
            workspace_root,
            thinking: epsilon_contracts::ThinkingLevel::default(),
        }
    }

    /// Git branch with a short TTL so we read `.git/HEAD` at most every 2s (pi's
    /// footer-data-provider caching, minus the watcher).
    fn cached_branch(&mut self) -> Option<String> {
        const TTL: std::time::Duration = std::time::Duration::from_secs(2);
        if self.branch_cache.1.elapsed() > TTL {
            let b = crate::footer::git_branch(&self.workspace_root);
            self.branch_cache = (b, std::time::Instant::now());
        }
        self.branch_cache.0.clone()
    }

    fn insert_char(&mut self, c: char) {
        let byte_at = self.byte_cursor();
        self.input.insert(byte_at, c);
        self.cursor_chars += 1;
    }

    fn backspace(&mut self) {
        if let Some(prev) = self
            .input
            .char_indices()
            .nth(self.cursor_chars.wrapping_sub(1))
            .map(|(i, _)| i)
        {
            self.input.remove(prev);
            self.cursor_chars -= 1;
        }
    }

    /// Kill from cursor to start (Ctrl+U); pushes onto the kill ring.
    fn kill_to_start(&mut self) {
        self.push_undo();
        let killed: String = self.input.chars().take(self.cursor_chars).collect();
        let (s, c) = crate::editor_ops::kill_to_start(&self.input, self.cursor_chars);
        // A fresh kill starts a new entry; consecutive kills extend the last one.
        let contiguous = self.last_kill_was_contiguous;
        self.kill_ring.push(killed, contiguous, false);
        self.last_kill_was_contiguous = true;
        self.input = s;
        self.cursor_chars = c;
        self.end_yank();
    }

    /// Kill the word before the cursor (Ctrl+W); pushes/accumulates on the ring.
    /// Paste markers count as ONE unit.
    fn kill_word_back(&mut self) {
        self.push_undo();
        let start = crate::editor_ops::word_left_atomic(&self.input, self.cursor_chars);
        let chars: Vec<char> = self.input.chars().collect();
        let killed: String = chars[start..self.cursor_chars].iter().collect();
        let kept: String = chars[..start]
            .iter()
            .chain(chars[self.cursor_chars..].iter())
            .collect();
        let contiguous = self.last_kill_was_contiguous;
        // Backward kills prepend when accumulating (Emacs rule).
        self.kill_ring.push(killed, contiguous, true);
        self.last_kill_was_contiguous = true;
        self.input = kept;
        self.cursor_chars = start;
        self.end_yank();
    }

    /// Yank (Ctrl+Y): insert the top of the kill ring; repeated presses cycle
    /// through older entries, replacing the previously yanked text (yank-pop).
    fn yank(&mut self) {
        if self.kill_ring.is_empty() {
            return;
        }
        if self.yank_in_progress
            && let Some(prev_len) = self.yanked_len.take()
        {
            // Delete the previously yanked span behind the cursor.
            let del_start_char = self.cursor_chars - prev_len;
            let byte_start = self.byte_cursor_at(del_start_char);
            let byte_end = self.byte_cursor_at(self.cursor_chars);
            self.input.replace_range(byte_start..byte_end, "");
            self.cursor_chars = del_start_char;
            self.kill_ring.rotate();
        }
        let text = self.kill_ring.peek().cloned().unwrap_or_default();
        let n = text.chars().count();
        self.insert_str(&text);
        self.yanked_len = Some(n);
        self.yank_in_progress = n > 0;
    }

    /// Any non-yank edit ends a yank sequence.
    fn end_yank(&mut self) {
        self.yank_in_progress = false;
        self.yanked_len = None;
    }

    /// Stash the current draft (opencode prompt.stash): called when a dialog opens.
    /// The editor becomes the dialog's filter box.
    fn stash_draft(&mut self) {
        if !self.input.is_empty() {
            self.stash = Some((std::mem::take(&mut self.input), self.cursor_chars));
            self.cursor_chars = 0;
        }
    }

    /// Restore the stashed draft (opencode onCleanup/onMount pair).
    fn unstash_draft(&mut self) {
        if let Some((text, cursor)) = self.stash.take() {
            self.input = text;
            self.cursor_chars = cursor;
        }
    }

    /// Snapshot the current state BEFORE a mutation (parity sweep: Ctrl+Z).
    fn push_undo(&mut self) {
        self.undo.push(crate::undo::EditorSnapshot {
            text: self.input.clone(),
            cursor_chars: self.cursor_chars,
        });
    }

    /// Ctrl+Z: restore the most recent pre-mutation snapshot.
    fn undo(&mut self) -> bool {
        let current = crate::undo::EditorSnapshot {
            text: self.input.clone(),
            cursor_chars: self.cursor_chars,
        };
        match self.undo.undo(current) {
            Some(snap) => {
                self.input = snap.text;
                self.cursor_chars = snap.cursor_chars.min(self.input.chars().count());
                self.end_yank();
                self.sync_slash_state();
                true
            }
            None => false,
        }
    }

    /// M17b: Ctrl+. re-applies the most recently undone edit (opencode input_redo).
    fn redo(&mut self) -> bool {
        let current = crate::undo::EditorSnapshot {
            text: self.input.clone(),
            cursor_chars: self.cursor_chars,
        };
        match self.undo.redo(current) {
            Some(snap) => {
                self.input = snap.text;
                self.cursor_chars = snap.cursor_chars.min(self.input.chars().count());
                self.end_yank();
                self.sync_slash_state();
                true
            }
            None => false,
        }
    }

    fn insert_str(&mut self, s: &str) {
        for c in s.chars() {
            self.insert_char(c);
        }
    }

    fn byte_cursor_at(&self, char_idx: usize) -> usize {
        self.input
            .char_indices()
            .nth(char_idx)
            .map(|(i, _)| i)
            .unwrap_or(self.input.len())
    }

    fn byte_cursor(&self) -> usize {
        self.input
            .char_indices()
            .nth(self.cursor_chars)
            .map(|(i, _)| i)
            .unwrap_or(self.input.len())
    }

    fn remember_history(&mut self, text: String) {
        if text.trim().is_empty() || self.history.back().is_some_and(|h| *h == text) {
            return;
        }
        self.history.push_back(text);
        if self.history.len() > HISTORY_MAX {
            self.history.pop_front();
        }
        self.history_idx = None;
    }

    fn recall(&mut self, older: bool) {
        if self.history.is_empty() {
            return;
        }
        let len = self.history.len();
        let idx = match self.history_idx {
            None => {
                if !older {
                    return;
                }
                len - 1
            }
            Some(i) => {
                let next = if older { i.saturating_sub(1) } else { i + 1 };
                if next >= len {
                    // Past newest entry: restore empty editor.
                    self.history_idx = None;
                    self.input.clear();
                    self.cursor_chars = 0;
                    return;
                }
                next
            }
        };
        self.history_idx = Some(idx);
        self.input = self.history[idx].clone();
        self.cursor_chars = self.input.chars().count();
    }

    fn submit_text(&mut self, text: String) -> Message {
        self.remember_history(text.clone());
        // M16b: persist to the global history file (best-effort; never blocks).
        if let Err(e) = epsilon_core::config::append_prompt_history(&text) {
            self.notice(format!("history not saved: {e}"));
        }
        self.ui.push_local_input(&text);
        Message::User(UserMessage {
            id: new_id("msg"),
            timestamp_ms: now_ms(),
            content: vec![UserContent::Text { text }],
        })
    }

    /// Insert text and refresh the slash-popup state (open/close + index reset).
    fn insert_char_tracked(&mut self, c: char) {
        self.insert_char(c);
        self.sync_slash_state();
    }

    /// Backspace and refresh the slash-popup state.
    fn backspace_tracked(&mut self) {
        self.backspace();
        self.sync_slash_state();
    }

    /// Re-derive slash popup open/index from the current editor buffer
    /// (pi: autocomplete state follows the text, never lingers).
    fn sync_slash_state(&mut self) {
        let candidates = crate::picker::slash_items(&self.input);
        if candidates.is_empty() {
            if self.slash_open {
                self.slash_open = false;
            }
            self.slash_index = 0;
        } else {
            if !self.slash_open {
                self.slash_open = true;
            }
            if self.slash_index >= candidates.len() {
                self.slash_index = 0;
            }
        }
    }

    /// Apply the currently highlighted slash candidate (pi `applyCompletion`):
    /// replace the whole editor with the item's value, cursor at end.
    fn apply_slash_selection(&mut self) {
        let candidates = crate::picker::slash_items(&self.input);
        if let Some(item) = candidates.get(self.slash_index) {
            self.input = item.value.to_string();
            self.cursor_chars = self.input.chars().count();
            self.end_yank();
        }
        // After applying, "/model " keeps the popup closed (argument typing next);
        // bare commands close it too — pi closes the menu on selection.
        self.slash_open = false;
        self.slash_index = 0;
    }

    /// Submit the editor line; returns `true` when a command requested quit.
    ///
    /// M16b: paste markers expand back into their payloads here, so the model sees
    /// the real pasted text while the editor only ever held the compact marker.
    fn submit_editor(&mut self, agent: &Arc<Agent>) -> bool {
        let raw = std::mem::take(&mut self.input);
        self.cursor_chars = 0;
        self.end_yank();
        let text = crate::kill_ring::expand_markers(&raw, &self.paste_payloads);
        self.paste_payloads.clear();
        if text.trim().is_empty() {
            return false;
        }
        // M22 / DIRECTION-01: a changed project config holds ALL submits until
        // reviewed (pi trust-manager: nothing runs against unreviewed rules).
        // Only the two review answers are accepted; everything else is bounced.
        if self.trust_review_pending {
            match text.trim().to_ascii_lowercase().as_str() {
                "trust" | "trust yes" | "y" => {
                    self.trust_review_pending = false;
                    // Persist the acceptance (M22 fix): without this the changed
                    // config re-prompted on every run forever. Best-effort — a
                    // failed write keeps the gate active for the next run. The
                    // workspace root (captured at startup) is the anchor, never
                    // a bare "." which would follow any cwd change.
                    let ws = self.workspace_root.clone();
                    match epsilon_core::config::current_project_config_hash_in(&ws) {
                        Some(hash) => {
                            match epsilon_core::config::accept_project_config_hash_in(&ws, &hash) {
                                Ok(path) => self.ui.push_notice(format!(
                                    "project config accepted for this workspace (saved to {})",
                                    path.display()
                                )),
                                Err(e) => {
                                    self.ui.push_notice(format!("accepted, but not saved: {e}"))
                                }
                            }
                        }
                        None => {
                            self.ui
                                .push_notice("project config accepted for this session");
                        }
                    }
                }
                "trust no" | "n" => {
                    return true; // quit — user declined the changed config
                }
                _ => {
                    self.ui.push_notice(
                        "config review pending - type `trust` to accept or `trust no` to quit",
                    );
                }
            }
            return false;
        }
        if let Some(command) = crate::picker::parse_command(&text) {
            return self.run_command(agent, command);
        }
        let msg = self.submit_text(text);
        if self.ui.running() {
            agent.steer(msg);
            self.notice("steering queued (delivered after current turn)");
        } else {
            self.spawn_run(Arc::clone(agent), msg);
        }
        false
    }

    /// Apply a thinking level (shared by /think and Ctrl+T; pi's
    /// cycleThinkingLevel + status line).
    fn set_thinking(&mut self, agent: &Arc<Agent>, level: epsilon_contracts::ThinkingLevel) {
        agent.set_thinking_level(level);
        // M25: mirror for the footer; this is the ONLY writer of `self.thinking`,
        // so what the footer shows is exactly what the agent will use.
        self.thinking = level;
        // Direct push: slash replies render this frame (see run_command docs).
        self.ui
            .push_notice(format!("thinking level: {}", level.as_str()));
    }

    /// Execute a parsed slash command; returns `true` to quit the session.
    /// Execute a parsed slash command; returns `true` to quit the session.
    ///
    /// Replies are pushed synchronously into the transcript so they render on the
    /// very next frame; the notices channel stays reserved for async events.
    fn run_command(&mut self, agent: &Arc<Agent>, command: crate::picker::Command) -> bool {
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
            crate::picker::Command::Help => {
                for line in [
                    "commands: /models · /model <id> · /think [level] · /skills · /search <query> · /clear · /quit · /help",
                    "thinking: Ctrl+T cycles · current level in the status bar",
                    "keys: Enter submit · Esc aborts a run (quits when idle) · Ctrl+X expands tool output",
                    "      PgUp/PgDn scroll transcript · Ctrl+M model picker",
                ] {
                    self.ui.push_notice(line);
                }
            }
            crate::picker::Command::Clear => {
                let (tin, tout) = self.ui.usage();
                self.ui.clear_lines();
                self.ui.push_notice(format!(
                    "transcript cleared — session totals kept (tokens {tin}/{tout})"
                ));
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
            crate::picker::Command::Search(query) => {
                if query.trim().is_empty() {
                    self.ui
                        .push_notice("usage: /search <query> — scans this session's log");
                } else {
                    match agent.search_history(
                        &query,
                        &epsilon_core::SearchOptions {
                            limit: 20,
                            ..Default::default()
                        },
                    ) {
                        Some(Ok(hits)) if hits.is_empty() => {
                            self.ui.push_notice(format!("no matches for '{query}'"));
                        }
                        Some(Ok(hits)) => {
                            self.ui.push_notice(format!(
                                "{} hit{} for '{}':",
                                hits.len(),
                                if hits.len() == 1 { "" } else { "s" },
                                query
                            ));
                            for h in &hits {
                                self.ui.push_notice(format!(
                                    "#{seq} {role}: {snippet}",
                                    seq = h.seq,
                                    role = h.role,
                                    snippet = h.snippet
                                ));
                            }
                        }
                        Some(Err(e)) => {
                            self.ui.push_notice(format!("session search failed: {e}"));
                        }
                        None => {
                            self.ui.push_notice(
                                "no session log attached (search unavailable this run)",
                            );
                        }
                    }
                }
            }
            crate::picker::Command::Quit => return true,
            crate::picker::Command::Unknown(word) => {
                self.ui
                    .push_notice(format!("unknown command '/{word}' — /help lists commands"));
            }
        }
        false
    }

    /// Switch the session model, persist choice + recents, and tell the user.
    fn apply_model(&mut self, agent: &Arc<Agent>, id: String) {
        if id == self.model_id {
            self.ui.push_notice(format!("model already {id}"));
            return;
        }
        agent.set_model_id(&id);
        match epsilon_core::config::save_model_id(&id) {
            Ok(path) => {
                self.model_id = id.clone();
                self.ui.push_notice(format!(
                    "model switched to {id} — saved to {}",
                    path.display()
                ));
            }
            Err(e) => {
                self.model_id = id.clone();
                self.ui
                    .push_notice(format!("model switched to {id} (not saved: {e})"));
            }
        }
        if epsilon_core::config::push_recent_model(&id).is_ok() {
            // M29: refresh from disk, then force this id to the front of the
            // in-memory list regardless. Under parallel test load (or any
            // concurrent writer) the re-read can lose our own push; the id we
            // just applied is the truth about "most recent".
            self.recents = epsilon_core::config::load_recent_models();
            self.recents.retain(|r| !r.eq_ignore_ascii_case(&id));
            self.recents.insert(0, id.clone());
            self.invalidate_picker();
        }
    }

    /// Request the provider's live listing (M13 Ctrl+F; M20 auto-fetch on open).
    ///
    /// Stage-aware: in stage 2 it lists THAT provider; from stage 1 it lists the
    /// highlighted provider directly. Passes the PROVIDER name to the fetcher —
    /// M17 passed a model id here, which `catalog::live_models` always rejected,
    /// so live fetch never actually worked until now.
    ///
    /// `force` = explicit user action (Ctrl+F): retries even if this provider
    /// already answered, and surfaces failures as notices. Auto-fetch stays
    /// silent on failure and never refetches a provider that already replied —
    /// a stale bundled row beats nagging on every keystroke path.
    fn request_live(&mut self, force: bool) {
        let Some(fetcher) = self.live.clone() else {
            if force {
                self.notice("live listing unavailable: built without --features real");
            }
            return;
        };
        // Target provider: stage 2's provider, else the highlighted stage-1 row.
        let provider = if let Some(p) = self.picker_provider.clone() {
            Some(p)
        } else {
            crate::picker::build_provider_rows(&self.model_id, &self.customs)
                .get(self.picker_index)
                .map(|r| r.id.clone())
        };
        let Some(provider) = provider else {
            return;
        };
        if !epsilon_llm::catalog::live_listing_available(&provider) {
            if force {
                self.notice(format!(
                    "provider '{provider}' has no live listing endpoint"
                ));
            }
            return;
        }
        if self.live_fetched.contains(&provider) && !force {
            return;
        }
        if self.fetching_hint.is_some() {
            return; // one fetch in flight; Ctrl+F again after it lands
        }
        self.fetching_hint = Some(provider.clone());
        if force {
            self.notice(format!("fetching live models for '{provider}'…"));
        } else {
            self.notice(format!("refreshing '{provider}' models in background…"));
        }
        let tx = self.fetched_tx.clone();
        let fut = fetcher(provider.clone());
        tokio::spawn(async move {
            let result = fut.await;
            tx.send((provider, result)).ok();
        });
    }

    /// Record a completed live fetch (event-loop side of [`Self::request_live`]).
    /// Returns a user-facing notice line when one is warranted.
    fn record_live_result(
        &mut self,
        provider: String,
        result: Result<Vec<String>, String>,
    ) -> Option<String> {
        self.fetching_hint = None;
        match result {
            Ok(names) => {
                let n = names.len();
                match self.live_cache.iter_mut().find(|(p, _)| *p == provider) {
                    Some((_, ids)) => *ids = names,
                    None => self.live_cache.push((provider.clone(), names)),
                }
                self.invalidate_picker();
                self.live_fetched.insert(provider);
                Some(format!("live listing: {n} models merged into /models"))
            }
            Err(e) => Some(format!("live listing failed: {e}")),
        }
    }

    fn spawn_run(&self, agent: Arc<Agent>, msg: Message) {
        let notices = self.notices_tx.clone();
        tokio::spawn(async move {
            if let Err(e) = agent.run(msg).await {
                notices.send(format!("run failed: {e}")).ok();
            }
        });
    }

    fn notice(&self, text: impl Into<String>) {
        self.notices_tx.send(text.into()).ok();
    }

    /// Bench/test seam (M16a): build an [`App`] over a caller-supplied [`UiState`] with
    /// inert channels and default picker state. Not part of normal composition.
    pub(crate) fn over_ui(ui: UiState, model_id: String) -> Self {
        let mut app = Self::new(model_id.clone(), inert_channel(), inert_channel());
        app.ui = ui;
        app
    }
}

/// Fresh sender for [`App::over_ui`]; the matching receiver is dropped immediately.
fn inert_channel<T>() -> tokio::sync::mpsc::UnboundedSender<T> {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    drop(rx);
    tx
}

/// Run the interactive frontend until the user quits. Returns the process exit code.
pub async fn run(cfg: TuiConfig) -> i32 {
    use crossterm::execute;
    let mut terminal = match ratatui::try_init() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("terminal init failed: {e}");
            return 1;
        }
    };
    // M15: bracketed paste + mouse wheel (best-effort; some terminals decline).
    let _ = execute!(
        std::io::stdout(),
        crossterm::event::EnableBracketedPaste,
        crossterm::event::EnableMouseCapture
    );
    let code = drive(cfg, &mut terminal).await;
    let _ = execute!(
        std::io::stdout(),
        crossterm::event::DisableMouseCapture,
        crossterm::event::DisableBracketedPaste
    );
    if let Err(e) = ratatui::try_restore() {
        eprintln!("terminal restore failed: {e}");
    }
    code
}

async fn drive(cfg: TuiConfig, terminal: &mut ratatui::DefaultTerminal) -> i32 {
    let TuiConfig {
        agent,
        approvals,
        initial_task,
        model_id,
        live,
        startup_notices,
        trust_review_pending,
    } = cfg;

    let mut events_rx = agent.subscribe();
    let (notices_tx, mut notices_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let (fetched_tx, mut fetched_rx) =
        tokio::sync::mpsc::unbounded_channel::<(String, Result<Vec<String>, String>)>();
    let mut app = App::new(model_id, fetched_tx, notices_tx);
    app.live = live;
    // M16b: seed the recall ring from the persisted global history so Up works
    // across restarts.
    for entry in epsilon_core::config::load_prompt_history() {
        app.history.push_back(entry);
    }
    while app.history.len() > HISTORY_MAX {
        app.history.pop_front();
    }

    // M22 / DIRECTION-01: flush startup diagnostics as notices, and if the
    // project config changed since it was trusted, hold the first submit until
    // the user answers the review prompt (pi trust-manager semantics).
    // Audit fix: the trigger is the typed flag from TuiConfig, not a substring
    // match on banner copy (rewording text must never disable a gate).
    app.trust_review_pending = trust_review_pending;
    for line in startup_notices {
        app.ui.push_notice(line);
    }
    if app.trust_review_pending {
        app.ui
            .push_notice("type `trust` to accept the new config, or `trust no` to quit");
    }

    // M22 fix: an initial task (positional arg / --tui "task") must respect the
    // trust gate like any other submit. When review is pending, stash the text
    // into the editor instead of running it; after `trust` accepts, the user
    // presses Enter to send it (or edits first — their call, not ours).
    if let Some(task) = initial_task {
        if app.trust_review_pending {
            for c in task.chars() {
                app.insert_char(c);
            }
            app.ui
                .push_notice("task staged in the input; answer the config review first");
        } else {
            let msg = app.submit_text(task);
            app.spawn_run(Arc::clone(&agent), msg);
        }
    }

    let mut term_events = EventStream::new();
    let mut tick = tokio::time::interval(std::time::Duration::from_millis(TICK_MS));

    loop {
        let _ = terminal.draw(|f| draw(f, &mut app));

        tokio::select! {
            ev = events_rx.recv() => match ev {
                Ok(ev) => apply_with_scroll(&mut app, &ev),
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    app.ui.push_notice(format!("… {n} events dropped (subscriber lag)"));
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            },

            ask = approvals.next_ask() => {
                if let Some(pending) = ask {
                    app.scroll_from_bottom = 0;
                    app.pending_ask = Some(pending);
                }
            },

            maybe_notice = notices_rx.recv() => {
                if let Some(text) = maybe_notice {
                    app.ui.push_notice(text);
                }
            },

            maybe_fetched = fetched_rx.recv() => {
                if let Some((provider, result)) = maybe_fetched
                    && let Some(text) = app.record_live_result(provider, result)
                {
                    app.ui.push_notice(text);
                }
            },

            maybe_ev = term_events.next() => {
                let Some(Ok(ev)) = maybe_ev else { break };
                match ev {
                    TermEvent::Key(key) => {
                        if key.kind == KeyEventKind::Press
                            && handle_key(&mut app, &agent, key)
                        {
                            break;
                        }
                    }
                    // M15: wheel scrolls the transcript (or navigates the picker).
                    TermEvent::Mouse(m) => match m.kind {
                        crossterm::event::MouseEventKind::ScrollUp => {
                            if app.picker_open {
                                app.picker_index = app.picker_index.saturating_sub(1);
                            } else {
                                app.scroll_from_bottom =
                                    app.scroll_from_bottom.saturating_add(3);
                            }
                        }
                        crossterm::event::MouseEventKind::ScrollDown => {
                            if app.picker_open {
                                let total = app.picker_rows().len();
                                if total > 0 {
                                    app.picker_index = (app.picker_index + 1).min(total - 1);
                                }
                            } else {
                                app.scroll_from_bottom =
                                    app.scroll_from_bottom.saturating_sub(3);
                            }
                        }
                        _ => {}
                    },
                    // M16b: bracketed pastes become atomic `[paste #N]` markers.
                    TermEvent::Paste(text) => {
                        let marker =
                            crate::kill_ring::make_paste_marker(app.next_paste_id, &text);
                        if marker.starts_with(crate::kill_ring::MARKER_PREFIX) {
                            app.paste_payloads.insert(app.next_paste_id, text.clone());
                            app.next_paste_id += 1;
                        }
                        app.insert_str(&marker);
                    }
                    _ => {}
                }
            },

            _ = tick.tick() => {
                if app.ui.running() {
                    app.spinner_phase = app.spinner_phase.wrapping_add(1);
                }
            },
        }
    }
    0
}

/// Apply one event and maintain scroll discipline (pi scroll-view `followEnd`
/// semantics): a pinned view follows new tail rows; once the user has scrolled
/// up, appended lines shift the offset so the reading window stays FROZEN on
/// the same content instead of yanking back to the bottom mid-stream.
fn apply_with_scroll(app: &mut App, ev: &AgentEvent) {
    let before = app.ui.lines().len();
    app.ui.apply(ev);
    match ev {
        AgentEvent::AgentStart => app.run_started_at = Some(std::time::Instant::now()),
        AgentEvent::AgentEnd { .. } => app.run_started_at = None,
        _ => {}
    }
    let appended = app.ui.lines().len().saturating_sub(before);
    if appended > 0 && app.scroll_from_bottom > 0 {
        // Frozen read: keep the same absolute window as the transcript grows.
        app.scroll_from_bottom += appended;
    }
}

/// Handle one key press; returns true when the app should quit.
fn handle_key(app: &mut App, agent: &Arc<Agent>, key: KeyEvent) -> bool {
    if key.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(key.code, KeyCode::Char('c') | KeyCode::Char('q'))
    {
        return true;
    }

    // Command palette (M16c) captures keys before everything else.
    if app.palette_open {
        match key.code {
            KeyCode::Esc => {
                app.palette_open = false;
                app.picker_index = 0;
                app.input.clear();
                app.cursor_chars = 0;
            }
            KeyCode::Enter => {
                let entries = crate::picker::filter_palette(&app.input);
                if let Some(entry) = entries.get(app.picker_index) {
                    let command = entry.command.clone();
                    app.palette_open = false;
                    app.picker_index = 0;
                    app.input.clear();
                    app.cursor_chars = 0;
                    if let Some(parsed) = crate::picker::parse_command(&command) {
                        return app.run_command(agent, parsed);
                    }
                }
            }
            KeyCode::Up => {
                let total = crate::picker::filter_palette(&app.input).len();
                app.picker_index = app
                    .picker_index
                    .saturating_sub(1)
                    .min(total.saturating_sub(1));
            }
            KeyCode::Down => {
                let total = crate::picker::filter_palette(&app.input).len();
                if total > 0 {
                    app.picker_index = (app.picker_index + 1).min(total - 1);
                }
            }
            KeyCode::Backspace => {
                app.backspace();
                app.invalidate_picker();
            }
            KeyCode::Char(c) => {
                app.insert_char(c);
                app.picker_index = 0;
            }
            _ => {}
        }
        return false;
    }

    // Model picker captures keys before the permission dialog (it can open while a
    // run is active, and must always be operable).
    if app.picker_open {
        // Stage-aware navigation (M16c): stage 1 = providers, stage 2 = models.
        let stage2 = app.picker_provider.clone();
        match key.code {
            KeyCode::Esc => {
                if stage2.is_some() {
                    // Stage 2 → back to provider list.
                    app.picker_provider = None;
                    app.picker_index = 0;
                    app.input.clear();
                } else {
                    app.picker_open = false;
                    app.picker_index = 0;
                    app.input.clear();
                    app.cursor_chars = 0;
                    app.unstash_draft();
                }
            }
            KeyCode::Enter => {
                if let Some(provider) = &stage2 {
                    let rows = crate::picker::build_models_for_provider(
                        provider,
                        &app.input,
                        &app.model_id,
                        &app.live_cache,
                        &app.customs,
                    );
                    if let Some(row) = rows.get(app.picker_index) {
                        let id = row.id.clone();
                        app.picker_open = false;
                        app.picker_provider = None;
                        app.picker_index = 0;
                        app.input.clear();
                        app.cursor_chars = 0;
                        app.apply_model(agent, id);
                    }
                } else {
                    // Stage 1: descend into the selected provider's model list.
                    let rows = crate::picker::build_provider_rows(&app.model_id, &app.customs);
                    if let Some(row) = rows.get(app.picker_index) {
                        app.picker_provider = Some(row.id.clone());
                        app.picker_index = 0;
                        app.input.clear();
                        app.cursor_chars = 0;
                        // M20: entering a provider refreshes its live list once
                        // per session (no-op when it already answered).
                        app.request_live(false);
                    }
                }
            }
            KeyCode::Up | KeyCode::Down | KeyCode::Char('f' | 'F')
                if key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                unreachable!("handled by guarded arms below")
            }
            _ => {}
        }
        if matches!(key.code, KeyCode::Enter | KeyCode::Esc) {
            return false;
        }
        // Remaining keys: Up/Down move, Backspace filters (stage 2 only), chars filter.
        match key.code {
            KeyCode::Up => {
                let total = if let Some(provider) = &stage2 {
                    crate::picker::build_models_for_provider(
                        provider,
                        &app.input,
                        &app.model_id,
                        &app.live_cache,
                        &app.customs,
                    )
                    .len()
                } else {
                    crate::picker::build_provider_rows(&app.model_id, &app.customs).len()
                };
                app.picker_index = app
                    .picker_index
                    .saturating_sub(1)
                    .min(total.saturating_sub(1));
            }
            KeyCode::Down => {
                let total = if let Some(provider) = &stage2 {
                    crate::picker::build_models_for_provider(
                        provider,
                        &app.input,
                        &app.model_id,
                        &app.live_cache,
                        &app.customs,
                    )
                    .len()
                } else {
                    crate::picker::build_provider_rows(&app.model_id, &app.customs).len()
                };
                if total > 0 {
                    app.picker_index = (app.picker_index + 1).min(total - 1);
                }
            }
            KeyCode::Backspace => {
                app.backspace();
                app.invalidate_picker();
                if stage2.is_none() {
                    return false; // stage-1 filter is display-only; no index fixups
                }
                let total = crate::picker::build_models_for_provider(
                    stage2.as_deref().unwrap_or(""),
                    &app.input,
                    &app.model_id,
                    &app.live_cache,
                    &app.customs,
                )
                .len();
                if app.picker_index >= total {
                    app.picker_index = total.saturating_sub(1);
                }
            }
            KeyCode::Char('f' | 'F') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                // M23: Ctrl+F toggles the favorite star on the highlighted row
                // (opencode model_favorite_toggle). Live force-refresh moved to
                // Ctrl+G; auto-fetch on open still covers the common case.
                // Audit fix: stage 1 lists PROVIDERS while picker_rows() holds
                // MODELS — with a filter typed, rows[picker_index] is an
                // invisible model and the star lands on a hidden target.
                // Stage 1 therefore stars the highlighted provider's active or
                // first known model instead of indexing the wrong list.
                let stage1_target = if app.picker_provider.is_none() {
                    crate::picker::build_provider_rows(&app.model_id, &app.customs)
                        .get(app.picker_index)
                        .map(|p| p.id.clone())
                        .and_then(|provider| {
                            epsilon_llm::catalog::lookup_model(&app.model_id)
                                .filter(|m| m.provider == provider)
                                .map(|_| app.model_id.clone())
                                .or_else(|| {
                                    crate::picker::build_models_for_provider(
                                        &provider,
                                        &app.input,
                                        &app.model_id,
                                        &app.live_cache,
                                        &app.customs,
                                    )
                                    .first()
                                    .map(|r| r.id.clone())
                                })
                        })
                } else {
                    None
                };
                match stage1_target {
                    Some(id) => app.toggle_favorite(agent, &id),
                    None => {
                        let rows = crate::picker::build_models_for_provider(
                            app.picker_provider.as_deref().unwrap_or(""),
                            &app.input,
                            &app.model_id,
                            &app.live_cache,
                            &app.customs,
                        );
                        if let Some(row) = rows.get(app.picker_index) {
                            let id = row.id.clone();
                            app.toggle_favorite(agent, &id);
                        }
                    }
                }
            }
            KeyCode::Char('g' | 'G') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                // Ctrl+G: force a live listing (M13's Ctrl+F job; M20 also
                // auto-fetches without any key).
                app.request_live(true);
            }
            KeyCode::Char(c) => {
                app.insert_char(c);
                app.invalidate_picker();
                app.picker_index = 0;
            }
            _ => {}
        }
        return false;
    }

    // Permission dialog captures keys next.
    if let Some(_pending) = app.pending_ask.as_ref() {
        if app.guidance_mode {
            match key.code {
                KeyCode::Enter => {
                    let guidance = std::mem::take(&mut app.input);
                    app.cursor_chars = 0;
                    app.guidance_mode = false;
                    if let Some(ask) = app.pending_ask.take() {
                        ask.reply(PermissionDecision::DenyWithGuidance { guidance });
                    }
                }
                KeyCode::Esc => {
                    app.guidance_mode = false;
                    app.input.clear();
                    app.cursor_chars = 0;
                }
                KeyCode::Backspace => app.backspace(),
                KeyCode::Char(c) => app.insert_char(c),
                _ => {}
            }
            return false;
        }
        match key.code {
            KeyCode::Char('y' | 'Y') => {
                if let Some(ask) = app.pending_ask.take() {
                    ask.reply(PermissionDecision::AllowOnce);
                }
            }
            KeyCode::Char('a' | 'A') => {
                if let Some(ask) = app.pending_ask.take() {
                    ask.reply(PermissionDecision::AlwaysSession);
                }
            }
            KeyCode::Char('n' | 'N') | KeyCode::Esc => {
                if let Some(ask) = app.pending_ask.take() {
                    ask.reply(PermissionDecision::Deny);
                }
            }
            KeyCode::Char('e' | 'E') => {
                app.guidance_mode = true;
                app.input.clear();
                app.cursor_chars = 0;
            }
            _ => {}
        }
        return false;
    }

    match key.code {
        KeyCode::Esc => {
            // M17b: Esc first dismisses the slash popup (pi prompt.autocomplete.hide);
            // then aborts a run; only when idle+quiet does it quit.
            if app.slash_open {
                app.slash_open = false;
                app.slash_index = 0;
                return false;
            }
            if app.ui.running() {
                if agent.abort_current_run() {
                    app.notice("abort requested — cancelling current run…");
                }
                false
            } else {
                true
            }
        }
        KeyCode::Enter => {
            // M17b: with the popup open, Enter first applies the highlighted
            // suggestion (pi prompt.autocomplete.select) — then FALLS THROUGH to
            // submit, because a completed command like "/clear" is meant to RUN.
            // Bare-text suggestions (no popup) submit unchanged.
            if app.slash_open {
                app.apply_slash_selection();
            }
            app.submit_editor(agent)
        }
        KeyCode::Char('m' | 'M') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            // Layout-independent picker access (slash may need AltGr on some layouts).
            if !app.picker_open {
                app.picker_open = true;
                app.picker_index = 0;
                app.stash_draft();
                // M20: same auto-fetch as /models (one code path per open).
                app.request_live(false);
            }
            false
        }
        KeyCode::Char('t' | 'T') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            // Cycle thinking level (pi app.thinking.cycle, Shift+Tab there —
            // Shift+Tab is unreliable across terminals, Ctrl+T is not taken).
            let next = agent.thinking_level().next();
            app.set_thinking(agent, next);
            false
        }
        KeyCode::Char('p' | 'P') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            // M16c: command palette (opencode prompt.palette).
            // M22: not while the trust review holds — palette commands bypass
            // submit_editor, so they must not open against unreviewed rules.
            if !app.palette_open && !app.picker_open {
                if app.trust_review_pending {
                    app.ui.push_notice(
                        "config review pending - type `trust` to accept or `trust no` to quit",
                    );
                } else {
                    app.palette_open = true;
                    app.picker_index = 0;
                    app.stash_draft();
                }
            }
            false
        }
        KeyCode::F(2) => {
            // M23: F2 cycles favorites forward (pi app.model.cycleForward over
            // scoped models; opencode model_cycle_recent family). Shift+F2 goes
            // backward. M27 audit: gated like Ctrl+P — cycling persists config
            // (model id, recents) and since M26 also mutates the thinking
            // level, so it must not act against unreviewed rules.
            if app.trust_review_pending {
                app.ui.push_notice(
                    "config review pending - type `trust` to accept or `trust no` to quit",
                );
                return false;
            }
            let dir = if key.modifiers.contains(KeyModifiers::SHIFT) {
                -1
            } else {
                1
            };
            app.cycle_favorite(agent, dir);
            false
        }
        KeyCode::Backspace if key.modifiers.contains(KeyModifiers::ALT) => {
            // Alt+Backspace: kill word backward (opencode input_delete_word_backward).
            app.kill_word_back();
            app.sync_slash_state();
            false
        }
        KeyCode::Backspace => {
            app.push_undo();
            app.backspace_tracked();
            app.end_yank();
            false
        }
        KeyCode::Char('z' | 'Z') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            // Parity sweep: editor undo (pi undo-stack.ts).
            app.undo();
            false
        }
        // M17b: redo (opencode input_redo, ctrl+.) and line-wise ops (pi/opencode).
        KeyCode::Char('.' | '>') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.redo();
            false
        }
        KeyCode::Char('a' | 'A') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.cursor_chars = crate::editor_ops::line_home(&app.input, app.cursor_chars);
            false
        }
        KeyCode::Char('e' | 'E') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.cursor_chars = crate::editor_ops::line_end(&app.input, app.cursor_chars);
            false
        }
        KeyCode::Char('k' | 'K') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            let (_, killed, _) = crate::editor_ops::kill_to_line_end(&app.input, app.cursor_chars);
            let (s, c) = (app.input.clone(), app.cursor_chars);
            let kept: String = {
                let chars: Vec<char> = s.chars().collect();
                chars[..c]
                    .iter()
                    .chain(chars[c + killed.chars().count()..].iter())
                    .collect()
            };
            app.push_undo();
            let contiguous = app.last_kill_was_contiguous;
            app.kill_ring.push(killed, contiguous, false);
            app.last_kill_was_contiguous = true;
            app.input = kept;
            app.end_yank();
            false
        }
        KeyCode::Left if key.modifiers.contains(KeyModifiers::ALT) => {
            app.cursor_chars = crate::editor_ops::word_left_atomic(&app.input, app.cursor_chars);
            false
        }
        KeyCode::Char('b' | 'B') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.cursor_chars = app.cursor_chars.saturating_sub(1);
            false
        }
        KeyCode::Char('f' | 'F') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.cursor_chars = (app.cursor_chars + 1).min(app.input.chars().count());
            false
        }
        KeyCode::Char('b' | 'B') if key.modifiers.contains(KeyModifiers::ALT) => {
            app.cursor_chars = crate::editor_ops::word_left_atomic(&app.input, app.cursor_chars);
            false
        }
        KeyCode::Char('f' | 'F') if key.modifiers.contains(KeyModifiers::ALT) => {
            app.cursor_chars = crate::editor_ops::word_right_atomic(&app.input, app.cursor_chars);
            false
        }
        KeyCode::Char('d' | 'D') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            // Forward-delete character under cursor (pi input.delete / opencode ctrl+d).
            if app.cursor_chars < app.input.chars().count() {
                let (s, _) = crate::editor_ops::delete_forward(&app.input, app.cursor_chars);
                app.push_undo();
                app.input = s;
                app.sync_slash_state();
            }
            false
        }
        KeyCode::Left if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.cursor_chars = crate::editor_ops::word_left(&app.input, app.cursor_chars);
            false
        }
        KeyCode::Right if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.cursor_chars = crate::editor_ops::word_right(&app.input, app.cursor_chars);
            false
        }
        KeyCode::Left => {
            app.cursor_chars = app.cursor_chars.saturating_sub(1);
            false
        }
        KeyCode::Right => {
            app.cursor_chars = (app.cursor_chars + 1).min(app.input.chars().count());
            false
        }
        KeyCode::Up => {
            // M17b: when the slash popup is open, arrows navigate IT (pi
            // prompt.autocomplete.prev) instead of history recall.
            if app.slash_open {
                let total = crate::picker::slash_items(&app.input).len();
                app.slash_index = app
                    .slash_index
                    .saturating_sub(1)
                    .min(total.saturating_sub(1));
                return false;
            }
            app.recall(true);
            false
        }
        KeyCode::Down => {
            if app.slash_open {
                let total = crate::picker::slash_items(&app.input).len();
                if total > 0 {
                    app.slash_index = (app.slash_index + 1).min(total - 1);
                }
                return false;
            }
            app.recall(false);
            false
        }
        KeyCode::PageUp => {
            app.scroll_from_bottom = app.scroll_from_bottom.saturating_add(10);
            false
        }
        KeyCode::PageDown => {
            app.scroll_from_bottom = app.scroll_from_bottom.saturating_sub(10);
            false
        }
        KeyCode::Tab => {
            // M15/M17b: Tab applies the HIGHLIGHTED slash candidate (pi tui.input.tab).
            if app.slash_open {
                app.apply_slash_selection();
            }
            false
        }
        KeyCode::Home => {
            app.cursor_chars = 0;
            false
        }
        KeyCode::End => {
            app.cursor_chars = app.input.chars().count();
            false
        }
        KeyCode::Delete => {
            let (s, _) = crate::editor_ops::delete_forward(&app.input, app.cursor_chars);
            app.input = s;
            false
        }
        KeyCode::Char('u' | 'U') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.kill_to_start();
            false
        }
        KeyCode::Char('w' | 'W') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.kill_word_back();
            app.sync_slash_state();
            false
        }
        KeyCode::Char('y' | 'Y') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.yank();
            false
        }
        // M16b: Ctrl+J inserts a newline (the reliable multi-line combo — Ctrl+Enter
        // is indistinguishable from Enter on legacy Windows consoles).
        KeyCode::Char('j' | 'J') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.insert_char('\n');
            app.end_yank();
            false
        }
        // M13 card polish: expand/collapse truncated tool outputs. Originally a
        // bare `x` (M13), which made the letter x untypable in the editor —
        // M29 fix: it needs Ctrl+X like every other non-typing key.
        KeyCode::Char('x' | 'X') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.prefs.expand_outputs = !app.prefs.expand_outputs;
            let state = if app.prefs.expand_outputs {
                "expanded"
            } else {
                "collapsed"
            };
            app.notice(format!("tool outputs {state} (Ctrl+X toggles)"));
            false
        }
        KeyCode::Char(c) => {
            app.insert_char_tracked(c);
            false
        }
        _ => false,
    }
}

/// Comma list of valid thinking levels for error messages.
fn levels_help() -> String {
    epsilon_contracts::ThinkingLevel::ALL
        .iter()
        .map(|l| l.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Contextual key-hint bar (pi's keybinding-hints pattern).
fn hint_line(app: &App) -> Line<'static> {
    use crate::theme::Theme;
    let text = if app.pending_ask.is_some() {
        "[y] allow · [a] always · [n] deny · [e] guidance · [Esc] deny"
    } else if app.picker_open {
        "type filters · ↑/↓ move · Ctrl+F favorite ★ · Ctrl+G refresh · Enter apply · Esc close"
    } else if app.slash_open {
        "↑/↓ pick · Tab/Enter apply · Esc dismiss"
    } else if app.ui.running() {
        "Esc abort run · type to steer after this turn"
    } else {
        "/help commands · /models picker (Ctrl+M) · Ctrl+X expand outputs · Esc quit"
    };
    Line::from(Span::styled(text.to_string(), Theme::system()))
}

fn draw(f: &mut Frame<'_>, app: &mut App) {
    // M16b acceptance: the editor grows to fit the draft up to N lines, then
    // scrolls internally. Height follows the live buffer's line count.
    let draft_lines = app.input.split('\n').count().clamp(1, INPUT_MAX_ROWS);
    let input_height = draft_lines as u16 + 2; // borders
    let [transcript_area, hints_area, input_area, status_area] = Layout::vertical([
        Constraint::Min(3),
        Constraint::Length(1),
        Constraint::Length(input_height),
        Constraint::Length(1),
    ])
    .areas(f.area());

    draw_transcript(f, app, transcript_area);
    f.render_widget(hint_line(app), hints_area);
    draw_input(f, app, input_area);
    draw_status(f, app, status_area);

    if app.picker_open {
        draw_picker(f, app, f.area());
    }
    if app.palette_open {
        draw_palette(f, app, f.area());
    }
    if app.pending_ask.is_some() {
        draw_dialog(f, app, f.area());
    }
    draw_slash_autocomplete(f, app, transcript_area);
}

/// Headless render seam used by `--tui-bench` (M16a): build a persistent [`App`]
/// whose caches live across frames exactly like the real event loop.
pub(crate) fn bench_app(ui: UiState) -> App {
    App::over_ui(ui, "faux-1".to_string())
}

/// Draw one frame with a caller-owned App (production draw path).
pub(crate) fn draw_frame(f: &mut Frame<'_>, app: &mut App) {
    draw(f, app);
}

/// `/models` overlay: filter-driven list with provider grouping hints and an auth
/// column. The active model floats to the top (picker::build_rows).
fn draw_picker(f: &mut Frame<'_>, app: &mut App, outer: Rect) {
    // M16c two-stage: stage 1 lists providers; stage 2 that provider's models.
    if let Some(provider) = &app.picker_provider.clone() {
        draw_picker_stage2(f, app, outer, provider);
        return;
    }
    let rows = crate::picker::build_provider_rows(&app.model_id, &app.customs)
        .into_iter()
        .map(|p| (p.id, p.model_count, p.auth_ready, p.is_active))
        .collect::<Vec<_>>();
    let width = (outer.width * 4 / 5).max(48).min(outer.width);
    let height = 15u16.min(outer.height.saturating_sub(2));
    let x = outer.x + (outer.width.saturating_sub(width)) / 2;
    let y = outer.y + 1;
    let area = Rect {
        x,
        y,
        width,
        height,
    };

    f.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(Span::styled(
            format!(" models — filter: '{}' ", app.input),
            Style::default().fg(Color::Cyan),
        ));

    let mut lines: Vec<Line<'static>> = Vec::new();
    if rows.is_empty() {
        lines.push(Line::from(Span::styled(
            "no providers",
            Style::default().fg(Color::DarkGray),
        )));
    }
    // M15 viewport fix: pi's centered scroll window — render only the visible slice
    // so long lists stay navigable (previously rows past the box were unreachable).
    let capacity = area.height.saturating_sub(4).max(1) as usize; // borders + footer + headroom
    let start = crate::scroll::scroll_start(rows.len(), app.picker_index, capacity);
    let end = (start + capacity).min(rows.len());

    for (i, (id, count, auth_ready, is_active)) in
        rows.iter().enumerate().skip(start).take(end - start)
    {
        let selected = i == app.picker_index;
        let marker = if *is_active { "● " } else { "  " };
        let auth = if *auth_ready { "🔑" } else { "· " };
        let style = if selected {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default()
        };
        lines.push(Line::from(vec![
            Span::styled(marker.to_string(), style),
            Span::styled(
                format!("{auth} "),
                Style::default().fg(if *auth_ready {
                    Color::Green
                } else {
                    Color::DarkGray
                }),
            ),
            Span::styled(
                id.clone(),
                style.add_modifier(if selected {
                    ratatui::style::Modifier::REVERSED
                } else {
                    ratatui::style::Modifier::empty()
                }),
            ),
            Span::styled(
                format!("  {count} models"),
                Style::default().fg(Color::DarkGray),
            ),
        ]));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "↑/↓ · Enter browse models · Esc close".to_string(),
        Style::default().fg(Color::DarkGray),
    )));

    let para = Paragraph::new(lines).block(block);
    f.render_widget(para, area);
}

/// Stage-2 of the two-stage picker: one provider's models with fuzzy filter.
fn draw_picker_stage2(f: &mut Frame<'_>, app: &App, outer: Rect, provider: &str) {
    let rows = crate::picker::build_models_for_provider(
        provider,
        &app.input,
        &app.model_id,
        &app.live_cache,
        &app.customs,
    );
    let width = (outer.width * 4 / 5).max(48).min(outer.width);
    let height = 15u16.min(outer.height.saturating_sub(2));
    let x = outer.x + (outer.width.saturating_sub(width)) / 2;
    let y = outer.y + 1;
    let area = Rect {
        x,
        y,
        width,
        height,
    };

    f.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(Span::styled(
            format!(" models — {provider}: filter '{}' ", app.input),
            Style::default().fg(Color::Cyan),
        ));

    let mut lines: Vec<Line<'static>> = Vec::new();
    if rows.is_empty() {
        lines.push(Line::from(Span::styled(
            "no matches — Backspace to widen, Esc closes",
            Style::default().fg(Color::DarkGray),
        )));
    }
    let capacity = area.height.saturating_sub(4).max(1) as usize;
    let start = crate::scroll::scroll_start(rows.len(), app.picker_index, capacity);
    let end = (start + capacity).min(rows.len());
    for (i, row) in rows.iter().enumerate().skip(start).take(end - start) {
        let selected = i == app.picker_index;
        let marker = if row.id == app.model_id { "● " } else { "  " };
        let auth = if row.auth_ready { "🔑" } else { "· " };
        // M23: ★ marks favorites (opencode's star column); Ctrl+F toggles.
        let star = if row.is_favorite { "★" } else { " " };
        let style = if selected {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default()
        };
        lines.push(Line::from(vec![
            Span::styled(marker.to_string(), style),
            Span::styled(
                format!("{auth} "),
                Style::default().fg(if row.auth_ready {
                    Color::Green
                } else {
                    Color::DarkGray
                }),
            ),
            Span::styled(
                format!("{star} "),
                Style::default().fg(if row.is_favorite {
                    Color::Yellow
                } else {
                    Color::DarkGray
                }),
            ),
            Span::styled(
                row.label.clone(),
                style.add_modifier(if selected {
                    ratatui::style::Modifier::REVERSED
                } else {
                    ratatui::style::Modifier::empty()
                }),
            ),
            Span::styled(
                format!("  {}", row.id),
                Style::default().fg(Color::DarkGray),
            ),
        ]));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "type filters (fuzzy) · Enter apply · Backspace/Esc back",
        Style::default().fg(Color::DarkGray),
    )));

    let para = Paragraph::new(lines).block(block);
    f.render_widget(para, area);
}

/// M16a virtualized draw: reconcile the render cache (renders ONLY unseen lines),
/// then materialize just the visible window. Per-frame cost is O(visible), not
/// O(transcript).
fn draw_transcript(f: &mut Frame<'_>, app: &mut App, area: Rect) {
    let width = area.width.saturating_sub(1) as usize; // 1-col slack, no h-scroll
    app.render_cache
        .reconcile(app.ui.lines(), width, app.prefs.expand_outputs);
    let total = app.render_cache.total_rows();
    let visible = area.height as usize;

    // Follow mode pins the tail; PageUp/PageDown pin an offset above the bottom.
    let skip = crate::scroll::transcript_skip(total, visible, app.scroll_from_bottom);
    let mut rows = app.render_cache.visible_rows(skip, visible);

    // Streaming tail (in-flight assistant text) rides under the cached rows.
    let streaming = app.ui.streaming_text().to_string();
    if !streaming.is_empty() {
        rows.push(Line::from(Span::styled(
            format!("{streaming}▌"),
            Style::default().add_modifier(Modifier::DIM),
        )));
    }

    let para = Paragraph::new(rows).scroll((skip as u16, 0));
    f.render_widget(para, area);
}

fn draw_input(f: &mut Frame<'_>, app: &App, area: Rect) {
    let title = if app.guidance_mode {
        "guidance for denial — Enter sends, Esc cancels"
    } else if app.ui.running() {
        "input — typing during a run queues steering · Ctrl+J newline"
    } else {
        "input — Ctrl+J newline · Ctrl+Y yank · Ctrl+Z undo"
    };
    let prompt_color = if app.ui.running() && !app.guidance_mode {
        Color::Yellow
    } else {
        Color::Cyan
    };
    let frame_box = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(title, Style::default().fg(Color::DarkGray)));
    let inner_width = area.width.saturating_sub(2) as usize;

    // M16b acceptance: show the TAIL of the draft once it exceeds the box (the
    // caret is where the user is typing), scrolling internally.
    let all_lines: Vec<&str> = app.input.split('\n').collect();
    let total_lines = all_lines.len();
    let visible_rows = area.height.saturating_sub(2).max(1) as usize;
    let skip_lines = total_lines.saturating_sub(visible_rows);
    let shown: Vec<Line<'static>> = all_lines
        .iter()
        .skip(skip_lines)
        .enumerate()
        .map(|(vis_idx, line_text)| {
            if vis_idx == 0 && skip_lines == 0 {
                Line::from(vec![
                    Span::styled("> ", Style::default().fg(prompt_color)),
                    Span::raw((*line_text).to_string()),
                ])
            } else {
                Line::from(Span::raw((*line_text).to_string()))
            }
        })
        .collect();
    let para = Paragraph::new(shown).block(frame_box);
    f.render_widget(para, area);

    // Caret: line/col within the whole buffer; clamp to what is on screen.
    let (line_idx, col_chars) = {
        let mut line = 0usize;
        let mut col = 0usize;
        for c in app.input.chars().take(app.cursor_chars) {
            if c == '\n' {
                line += 1;
                col = 0;
            } else {
                col += 1;
            }
        }
        // "> " prefix only occupies the first visual line.
        let prefix = if line == 0 { 2 } else { 0 };
        (line, col + prefix)
    };
    let screen_line = line_idx.saturating_sub(skip_lines);
    let row = area.y + 1 + screen_line.min(visible_rows - 1) as u16;
    let col = area
        .x
        .saturating_add(col_chars.min(inner_width.saturating_sub(1)) as u16);
    f.set_cursor_position((col, row));
}

fn draw_status(f: &mut Frame<'_>, app: &mut App, area: Rect) {
    // ---- left: run state (spinner + status + elapsed, M15 behavior) --------------
    let spinner = SPINNER[app.spinner_phase % SPINNER.len()];
    let left = if let Some(started) = app.run_started_at {
        let secs = started.elapsed().as_secs();
        format!("{spinner} running {secs}s · {}", app.ui.status())
    } else if app.ui.running() {
        format!("{spinner} {}", app.ui.status())
    } else {
        app.ui.status().to_string()
    };

    // ---- right: pi footer stats — where/what/how-much ----------------------------
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(std::path::PathBuf::from);
    let mut pwd = crate::footer::format_cwd(&app.workspace_root, home.as_deref());
    if let Some(branch) = app.cached_branch() {
        pwd.push_str(&format!(" ({branch})"));
    }

    let (tin, tout) = app.ui.usage();
    let (cread, cwrite) = app.ui.cache_usage();
    // Cost from catalog placeholder prices when the model is known (ADR-010 item 5;
    // integer micro-USD, no float drift). Unknown/custom ids omit cost honestly.
    let cost_micro: u64 = match epsilon_llm::catalog::lookup_model(&app.model_id) {
        Some(info) => {
            tin * info.input_cost_micro_per_mtok / 1_000_000
                + tout * info.output_cost_micro_per_mtok / 1_000_000
        }
        None => 0,
    };
    let mut parts = crate::footer::stats_parts(&crate::footer::FooterStats {
        tokens_in: tin,
        tokens_out: tout,
        cache_read: cread,
        cache_write: cwrite,
        latest_prompt_tokens: app.ui.latest_prompt_tokens(),
        compacted: app.ui.is_compacted(),
        context_window: epsilon_llm::catalog::lookup_model(&app.model_id).map(|i| i.context_window),
        thinking_level: Some(app.thinking),
        cost_micro_usd: cost_micro,
    });
    // Model id anchors the far right (pi puts it there too).
    parts.push(crate::footer::FooterPart {
        text: epsilon_llm::catalog::display_label(&app.model_id).to_string(),
        tone: crate::footer::Tone::Normal,
    });

    let tone_color = |t: crate::footer::Tone| match t {
        crate::footer::Tone::Normal => Color::DarkGray,
        crate::footer::Tone::Warn => Color::Yellow,
        crate::footer::Tone::Error => Color::Red,
    };
    let right_spans: Vec<Span<'static>> =
        std::iter::once(Span::styled(pwd, Style::default().fg(Color::DarkGray)))
            .chain(
                parts
                    .iter()
                    .flat_map(|p| {
                        [
                            Span::raw(" · "),
                            Span::styled(p.text.clone(), Style::default().fg(tone_color(p.tone))),
                        ]
                    })
                    .collect::<Vec<_>>(),
            )
            .collect();

    // Fit: truncate the LEFT side first (status is less important than the stats).
    let right_len: usize = right_spans.iter().map(|s| s.content.chars().count()).sum();
    let width = area.width as usize;
    let left_fit = left
        .chars()
        .count()
        .min(width.saturating_sub(right_len + 2));
    let pad = width
        .saturating_sub(right_len)
        .saturating_sub(left_fit)
        .max(1);

    let mut line_spans = vec![Span::styled(left, Style::default().fg(Color::DarkGray))];
    line_spans.push(Span::raw(" ".repeat(pad)));
    line_spans.extend(right_spans);
    f.render_widget(Line::from(line_spans), area);
}

/// Command palette overlay (M16c, opencode command-palette pattern): fuzzy-filtered
/// list of all commands with keybinding hints.
fn draw_palette(f: &mut Frame<'_>, app: &App, outer: Rect) {
    let entries = crate::picker::filter_palette(&app.input);
    let width = (outer.width * 3 / 5).max(48).min(outer.width);
    let height = 12u16.min(outer.height.saturating_sub(2));
    let x = outer.x + (outer.width.saturating_sub(width)) / 2;
    let y = outer.y + 1;
    let area = Rect {
        x,
        y,
        width,
        height,
    };

    f.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(Span::styled(
            format!(" commands — filter: '{}' ", app.input),
            Style::default().fg(Color::Cyan),
        ));

    let mut lines: Vec<Line<'static>> = Vec::new();
    if entries.is_empty() {
        lines.push(Line::from(Span::styled(
            "no matching commands",
            Style::default().fg(Color::DarkGray),
        )));
    }
    let capacity = area.height.saturating_sub(4).max(1) as usize;
    let start = crate::scroll::scroll_start(entries.len(), app.picker_index, capacity);
    let end = (start + capacity).min(entries.len());
    for (i, entry) in entries.iter().enumerate().skip(start).take(end - start) {
        let selected = i == app.picker_index;
        let mut spans = vec![Span::styled(
            entry.title.clone(),
            if selected {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(ratatui::style::Modifier::REVERSED)
            } else {
                Style::default()
            },
        )];
        if let Some(b) = entry.binding {
            spans.push(Span::styled(
                format!("  [{b}]"),
                Style::default().fg(Color::DarkGray),
            ));
        }
        lines.push(Line::from(spans));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "type to filter · ↑/↓ move · Enter run · Esc close",
        Style::default().fg(Color::DarkGray),
    )));

    f.render_widget(Paragraph::new(lines).block(block), area);
}

// (M17b: slash registry moved to `picker::SLASH_ITEMS`/`picker::slash_items` —
// descriptions + selection live there now.)

/// Popup listing matching slash commands directly above the input box
/// (M17b: pi SelectList rendering — highlighted selection + dim descriptions,
/// arrow-key navigable via `slash_index`).
fn draw_slash_autocomplete(f: &mut Frame<'_>, app: &App, transcript_area: Rect) {
    let candidates = crate::picker::slash_items(&app.input);
    if !app.slash_open || candidates.is_empty() {
        return;
    }
    let height = candidates.len() as u16 + 1;
    let y = transcript_area.bottom().saturating_sub(height);
    let area = Rect {
        x: transcript_area.x,
        y,
        width: transcript_area.width.min(56),
        height,
    };
    f.render_widget(Clear, area);
    let mut lines: Vec<Line<'static>> = candidates
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let selected = i == app.slash_index;
            let mut spans = vec![Span::styled(
                format!(" {} ", item.value),
                if selected {
                    Theme::accent().add_modifier(Modifier::REVERSED)
                } else {
                    Theme::accent()
                },
            )];
            if !item.description.is_empty() {
                spans.push(Span::styled(
                    format!(" {}", item.description),
                    if selected {
                        Theme::system().add_modifier(Modifier::REVERSED)
                    } else {
                        Theme::system()
                    },
                ));
            }
            Line::from(spans)
        })
        .collect();
    lines.push(Line::from(Span::styled(
        "↑/↓ pick · Tab/Enter apply · Esc dismiss".to_string(),
        Theme::system(),
    )));
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Theme::accent());
    f.render_widget(Paragraph::new(lines).block(block), area);
}

fn draw_dialog(f: &mut Frame<'_>, app: &App, outer: Rect) {
    let width = (outer.width * 3 / 5).max(40).min(outer.width);
    let height = 9u16.min(outer.height);
    let x = outer.x + (outer.width.saturating_sub(width)) / 2;
    let y = outer.y + (outer.height.saturating_sub(height)) / 2;
    let area = Rect {
        x,
        y,
        width,
        height,
    };

    f.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow))
        .title(Span::styled(
            " permission required ",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ));

    let body: Vec<Line<'static>> = if let Some(ask) = app.pending_ask.as_ref() {
        let req = &ask.request;
        vec![
            Line::from(Span::styled(
                format!("capability: {}", req.capability),
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from(format!("pattern:   {}", req.pattern)),
            Line::from(format!("detail:    {}", summarize_detail(&req.detail))),
            Line::from(""),
            Line::from(Span::styled(
                "[y] allow once   [a] always this session",
                Style::default().fg(Color::Green),
            )),
            Line::from(Span::styled(
                "[n] deny         [e] deny with guidance",
                Style::default().fg(Color::Red),
            )),
        ]
    } else {
        vec![Line::from("resolving…")]
    };

    let para = Paragraph::new(body).block(block);
    f.render_widget(para, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyEventState;
    use epsilon_safety::ChannelApprovalUi;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    fn ctrl(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::CONTROL,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    /// Type a string one char at a time, then Enter — the real key path.
    fn type_and_enter(app: &mut App, agent: &Arc<Agent>, text: &str) -> bool {
        for c in text.chars() {
            handle_key(app, agent, key(KeyCode::Char(c)));
        }
        handle_key(app, agent, key(KeyCode::Enter))
    }

    #[tokio::test]
    async fn search_command_reports_hits_from_the_real_session_log() {
        let (mut app, agent, (_f, _n)) = test_app();
        // Attach a durable mirror and seed it directly through the store API.
        let tmp = tempfile::tempdir().unwrap();
        let store = std::sync::Arc::new(std::sync::Mutex::new(
            epsilon_core::SessionStore::open(tmp.path().join("s.jsonl")).unwrap(),
        ));
        agent.attach_session_store(std::sync::Arc::clone(&store));
        {
            let mut st = store.lock().unwrap();
            use epsilon_contracts::content::UserContent;
            use epsilon_contracts::message::UserMessage;
            use epsilon_contracts::session_entry::EntryKind;
            st.append(EntryKind::Message {
                message: epsilon_contracts::message::Message::User(UserMessage {
                    id: epsilon_contracts::new_id("msg"),
                    timestamp_ms: 0,
                    content: vec![UserContent::Text {
                        text: "the launch codes are under the keyboard".into(),
                    }],
                }),
                api_request_sha256: None,
            })
            .unwrap();
        }

        // Real key path: /search keyboard
        assert!(!type_and_enter(&mut app, &agent, "/search keyboard"));
        let texts: Vec<String> = app.ui.lines().iter().map(|l| l.text.clone()).collect();
        assert!(
            texts
                .iter()
                .any(|t| t.contains("#0 user") && t.contains("keyboard")),
            "expected a hit line mentioning the seeded entry, got {texts:?}"
        );

        // Miss case
        assert!(!type_and_enter(&mut app, &agent, "/search quantum-unicorn"));
        let texts: Vec<String> = app.ui.lines().iter().map(|l| l.text.clone()).collect();
        assert!(
            texts
                .iter()
                .any(|t| t.contains("no matches for 'quantum-unicorn'")),
            "expected the miss notice, got {texts:?}"
        );

        // Usage hint case
        assert!(!type_and_enter(&mut app, &agent, "/search"));
        let texts: Vec<String> = app.ui.lines().iter().map(|l| l.text.clone()).collect();
        assert!(
            texts.iter().any(|t| t.contains("usage: /search")),
            "expected the usage hint, got {texts:?}"
        );
    }

    type TestReceivers = (
        tokio::sync::mpsc::UnboundedReceiver<(String, Result<Vec<String>, String>)>,
        tokio::sync::mpsc::UnboundedReceiver<String>,
    );

    fn test_app() -> (App, Arc<Agent>, TestReceivers) {
        let (ftx, frx) = tokio::sync::mpsc::unbounded_channel();
        let (ntx, nrx) = tokio::sync::mpsc::unbounded_channel();
        let approvals = Arc::new(ChannelApprovalUi::new());
        let gate = Arc::new(epsilon_safety::RulesetGate::new(
            Arc::clone(&approvals) as Arc<dyn epsilon_safety::ApprovalUi>
        ));
        let agent = Agent::new(
            epsilon_core::LoopLimits::default(),
            Arc::new(epsilon_llm::PanicGuard(epsilon_llm::FauxProvider::new(
                vec![],
            ))),
            Arc::new(epsilon_core::ToolRegistry::new()),
            gate,
            std::env::temp_dir(),
        );
        (App::new("faux-1".into(), ftx, ntx), agent, (frx, nrx))
    }

    #[tokio::test]
    async fn slash_models_opens_picker_via_real_key_path() {
        let (mut app, agent, (_f, _n)) = test_app();
        for c in "/models".chars() {
            handle_key(&mut app, &agent, key(KeyCode::Char(c)));
            if app.picker_open {
                panic!("picker opened early at char {c:?} — some arm intercepts typing");
            }
        }
        assert_eq!(app.input, "/models", "typing must land in the editor");
        let quit_now = handle_key(&mut app, &agent, key(KeyCode::Enter));
        assert!(!quit_now);
        assert!(app.picker_open, "/models must open the picker");
        assert!(app.input.is_empty(), "editor becomes the filter box");

        assert!(!handle_key(&mut app, &agent, key(KeyCode::Esc)));
        assert!(!app.picker_open);
    }

    #[tokio::test]
    async fn slash_model_direct_applies_instantly() {
        let (mut app, agent, (_f, _n)) = test_app();
        let before = app.ui.lines().len();
        assert!(!type_and_enter(&mut app, &agent, "/model deepseek-chat"));
        assert_eq!(
            app.model_id, "deepseek-chat",
            "direct /model applies instantly"
        );
        let line = &app.ui.lines()[before];
        assert!(line.text.contains("deepseek-chat"), "{:?}", line.text);
        // Recents refresh in-memory through apply_model (the persisted side is
        // covered by config-layer round-trip tests; asserting the real global
        // file here races every other TUI test's writes — lesson 16b again).
        assert!(
            app.recents.first().map(String::as_str) == Some("deepseek-chat"),
            "in-memory recents updated with the applied model"
        );
    }

    #[tokio::test]
    async fn unknown_slash_word_errors_instead_of_prompting_the_model() {
        let (mut app, agent, (_f, _n)) = test_app();
        let before = app.ui.lines().len();
        assert!(!type_and_enter(&mut app, &agent, "/bogus"));
        assert!(!app.ui.running(), "must NOT start a run");
        assert_eq!(app.ui.lines().len(), before + 1, "one system notice");
        assert!(app.ui.lines()[before].text.contains("/bogus"));
    }

    #[tokio::test]
    async fn ctrl_m_opens_picker_without_typing_slash() {
        let (mut app, agent, (_f, _n)) = test_app();
        assert!(!handle_key(&mut app, &agent, ctrl(KeyCode::Char('m'))));
        assert!(app.picker_open);
    }

    #[tokio::test]
    async fn x_toggles_expand_and_quit_requests_exit() {
        let (mut app, agent, (_f, _n)) = test_app();
        // M29: bare `x` is a LETTER now; only Ctrl+X toggles output expansion.
        assert!(!handle_key(&mut app, &agent, key(KeyCode::Char('x'))));
        assert!(!app.prefs.expand_outputs, "plain x types, not toggles");
        assert!(app.input.ends_with('x'), "x lands in the editor");
        app.backspace();

        assert!(!handle_key(&mut app, &agent, ctrl(KeyCode::Char('x'))));
        assert!(app.prefs.expand_outputs);

        assert!(!type_and_enter(&mut app, &agent, "/clear"));
        let last = app.ui.lines().last().unwrap();
        assert!(last.text.contains("cleared"), "{:?}", last.text);

        // /quit must request exit through the same Enter path users press.
        assert!(type_and_enter(&mut app, &agent, "/quit"), "/quit exits");
    }

    #[tokio::test]
    async fn ctrl_w_then_ctrl_y_round_trips_text_through_the_kill_ring() {
        let (mut app, agent, (_f, _n)) = test_app();
        for c in "hello world".chars() {
            handle_key(&mut app, &agent, key(KeyCode::Char(c)));
        }
        // Ctrl+W kills "world".
        handle_key(&mut app, &agent, ctrl(KeyCode::Char('w')));
        assert_eq!(app.input, "hello ");
        // Ctrl+Y yanks it back.
        handle_key(&mut app, &agent, ctrl(KeyCode::Char('y')));
        assert_eq!(app.input, "hello world", "yank restores the kill");
    }

    #[tokio::test]
    async fn ctrl_j_inserts_newline_and_enter_still_submits() {
        let (mut app, agent, (_f, _n)) = test_app();
        for c in "line one".chars() {
            handle_key(&mut app, &agent, key(KeyCode::Char(c)));
        }
        handle_key(&mut app, &agent, ctrl(KeyCode::Char('j')));
        for c in "line two".chars() {
            handle_key(&mut app, &agent, key(KeyCode::Char(c)));
        }
        assert_eq!(app.input, "line one\nline two", "Ctrl+J is a newline");

        // Enter submits the WHOLE multi-line buffer (and persists history).
        let before = app.ui.lines().len();
        assert!(!handle_key(&mut app, &agent, key(KeyCode::Enter)));
        assert_eq!(app.ui.lines().len(), before + 1);
        assert_eq!(
            app.history.back().map(String::as_str),
            Some("line one\nline two")
        );
    }

    #[tokio::test]
    async fn consecutive_ctrl_w_accumulates_into_one_yank_entry() {
        let (mut app, agent, (_f, _n)) = test_app();
        for c in "alpha beta gamma".chars() {
            handle_key(&mut app, &agent, key(KeyCode::Char(c)));
        }
        handle_key(&mut app, &agent, ctrl(KeyCode::Char('w'))); // kills "gamma"
        handle_key(&mut app, &agent, ctrl(KeyCode::Char('w'))); // kills "beta "
        assert_eq!(app.input, "alpha ");
        // One yank restores BOTH words (accumulated entry).
        handle_key(&mut app, &agent, ctrl(KeyCode::Char('y')));
        assert_eq!(
            app.input,
            "alpha beta gamma",
            "{:?} ring={}",
            app.input,
            app.kill_ring.len()
        );
    }

    #[tokio::test]
    async fn paste_marker_expands_to_payload_on_submit() {
        let (mut app, agent, (_f, _n)) = test_app();
        // Simulate a bracketed paste of 3 lines.
        let pasted = "l1\nl2\nl3";
        app.paste_payloads.insert(7, pasted.to_string());
        app.next_paste_id = 8;
        for c in "summarize [paste #7 +3 lines]".chars() {
            handle_key(&mut app, &agent, key(KeyCode::Char(c)));
        }
        let before = app.ui.lines().len();
        assert!(!handle_key(&mut app, &agent, key(KeyCode::Enter)));
        // The transcript echo must show the EXPANDED payload.
        let echoed = app.ui.lines()[before].text.clone();
        assert!(echoed.contains("l1\nl2\nl3"), "{echoed:?}");
    }

    #[tokio::test]
    async fn ctrl_p_opens_palette_runs_command_and_restores_draft() {
        let (mut app, agent, (_f, _n)) = test_app();
        for c in "draft in progress".chars() {
            handle_key(&mut app, &agent, key(KeyCode::Char(c)));
        }
        // Ctrl+P opens the palette and stashes the draft.
        assert!(!handle_key(&mut app, &agent, ctrl(KeyCode::Char('p'))));
        assert!(app.palette_open);
        assert_eq!(app.input, "", "editor becomes the filter box");
        assert_eq!(
            app.stash.as_ref().map(|(t, _)| t.as_str()),
            Some("draft in progress")
        );

        // Enter runs the first entry (/models) → opens the picker instead.
        assert!(!handle_key(&mut app, &agent, key(KeyCode::Enter)));
        assert!(!app.palette_open);
        assert!(app.picker_open);

        // Esc closes the picker AND restores the stashed draft.
        assert!(!handle_key(&mut app, &agent, key(KeyCode::Esc)));
        assert!(!app.picker_open);
        assert_eq!(app.input, "draft in progress");
    }

    #[tokio::test]
    async fn ctrl_z_undoes_backspace_and_kills() {
        let (mut app, agent, (_f, _n)) = test_app();
        for c in "undo me".chars() {
            handle_key(&mut app, &agent, key(KeyCode::Char(c)));
        }
        // Backspace deletes "e".
        handle_key(&mut app, &agent, key(KeyCode::Backspace));
        assert_eq!(app.input, "undo m");
        // Ctrl+Z restores it.
        handle_key(&mut app, &agent, ctrl(KeyCode::Char('z')));
        assert_eq!(app.input, "undo me");

        // Ctrl+W kill is undoable too.
        handle_key(&mut app, &agent, ctrl(KeyCode::Char('w')));
        assert_eq!(app.input, "undo ");
        handle_key(&mut app, &agent, ctrl(KeyCode::Char('z')));
        assert_eq!(app.input, "undo me", "kill undone");
    }

    #[tokio::test]
    async fn two_stage_picker_descends_and_applies_model() {
        let (mut app, agent, (_f, _n)) = test_app();
        // Open the picker via Ctrl+M (stage 1: providers).
        assert!(!handle_key(&mut app, &agent, ctrl(KeyCode::Char('m'))));
        assert!(app.picker_open);
        assert!(app.picker_provider.is_none(), "starts at stage 1");

        // Enter on the first provider descends to stage 2.
        assert!(!handle_key(&mut app, &agent, key(KeyCode::Enter)));
        assert!(app.picker_provider.is_some(), "descended into provider");

        // Esc goes back to stage 1 (picker still open).
        assert!(!handle_key(&mut app, &agent, key(KeyCode::Esc)));
        assert!(app.picker_open && app.picker_provider.is_none());

        // Esc again closes entirely and unstashes (nothing stashed here).
        assert!(!handle_key(&mut app, &agent, key(KeyCode::Esc)));
        assert!(!app.picker_open);
    }

    // ---- M21: thinking level surface -------------------------------------------------

    #[tokio::test]
    async fn think_command_sets_levels_and_reports_unknown() {
        let (mut app, agent, (_f, _n)) = test_app();
        assert!(!type_and_enter(&mut app, &agent, "/think high"));
        assert_eq!(
            agent.thinking_level(),
            epsilon_contracts::ThinkingLevel::High
        );
        let texts: Vec<String> = app.ui.lines().iter().map(|l| l.text.clone()).collect();
        assert!(texts.iter().any(|t| t.contains("thinking level: high")));

        // Bare /think reports the current level without changing it.
        assert!(!type_and_enter(&mut app, &agent, "/think"));
        assert_eq!(
            agent.thinking_level(),
            epsilon_contracts::ThinkingLevel::High
        );

        // Unknown level is corrected, not sent to the model.
        assert!(!type_and_enter(&mut app, &agent, "/think banana"));
        assert_eq!(
            agent.thinking_level(),
            epsilon_contracts::ThinkingLevel::High
        );
        let texts: Vec<String> = app.ui.lines().iter().map(|l| l.text.clone()).collect();
        assert!(texts.iter().any(|t| t.contains("unknown level 'banana'")));
    }

    #[tokio::test]
    async fn ctrl_t_cycles_the_thinking_ladder() {
        let (mut app, agent, (_f, _n)) = test_app();
        assert_eq!(
            agent.thinking_level(),
            epsilon_contracts::ThinkingLevel::Off
        );
        for _ in 0..3 {
            handle_key(&mut app, &agent, ctrl(KeyCode::Char('t')));
        }
        // off -> minimal -> low -> medium
        assert_eq!(
            agent.thinking_level(),
            epsilon_contracts::ThinkingLevel::Medium
        );
        // M25: the footer mirror must equal what the agent actually carries
        // (displayed state == executed state, never a stale estimate).
        assert_eq!(app.thinking, agent.thinking_level());
        let texts: Vec<String> = app.ui.lines().iter().map(|l| l.text.clone()).collect();
        assert!(texts.iter().any(|t| t.contains("thinking level: medium")));
    }

    // ---- M22: credential store + trust review ----------------------------------------

    #[tokio::test]
    async fn trust_review_holds_submits_until_answered() {
        let (mut app, agent, (_f, _n)) = test_app();
        // Simulate the startup banner having flagged a changed config.
        app.trust_review_pending = true;

        // Any normal text is bounced with guidance, never sent to the model.
        assert!(!type_and_enter(&mut app, &agent, "run my tests"));
        let texts: Vec<String> = app.ui.lines().iter().map(|l| l.text.clone()).collect();
        assert!(texts.iter().any(|t| t.contains("config review pending")));
        assert!(!app.ui.running(), "nothing was submitted");

        // `trust` accepts and unblocks.
        assert!(!type_and_enter(&mut app, &agent, "trust"));
        assert!(!app.trust_review_pending);
        let texts: Vec<String> = app.ui.lines().iter().map(|l| l.text.clone()).collect();
        assert!(texts.iter().any(|t| t.contains("project config accepted")));

        // After acceptance, submits flow normally again (bare /think reports).
        assert!(!type_and_enter(&mut app, &agent, "/think"));
        let texts: Vec<String> = app.ui.lines().iter().map(|l| l.text.clone()).collect();
        assert!(texts.iter().any(|t| t.contains("thinking level:")));
    }

    #[tokio::test]
    async fn trust_no_quits_instead_of_running_against_unreviewed_config() {
        let (mut app, agent, (_f, _n)) = test_app();
        app.trust_review_pending = true;
        assert!(
            type_and_enter(&mut app, &agent, "trust no"),
            "declining exits the session"
        );
    }

    #[tokio::test]
    async fn trust_gate_blocks_palette_and_stages_initial_task() {
        let (mut app, agent, (_f, _n)) = test_app();
        app.trust_review_pending = true;

        // Ctrl+P must NOT open the palette while review is pending.
        handle_key(&mut app, &agent, ctrl(KeyCode::Char('p')));
        assert!(!app.palette_open, "palette stays closed under the gate");

        // After accepting, the palette opens normally again.
        assert!(!type_and_enter(&mut app, &agent, "trust"));
        handle_key(&mut app, &agent, ctrl(KeyCode::Char('p')));
        assert!(app.palette_open, "palette works after review");
    }

    #[tokio::test]
    async fn trust_gate_blocks_f2_favorite_cycling() {
        // M27 audit (lesson 14: every input path gets a gate test): F2 cycles
        // models AND mutates the thinking level (M26), persisting config — it
        // must not act while the project-config review is pending.
        let (mut app, agent, (_f, _n)) = test_app();
        app.favorites = vec!["faux-1".into(), "deepseek-chat".into()];
        app.favorite_thinking = vec![("deepseek-chat".into(), "high".into())];
        app.trust_review_pending = true;

        handle_key(&mut app, &agent, key(KeyCode::F(2)));
        assert_eq!(app.model_id, "faux-1", "model unchanged under the gate");
        assert_eq!(
            agent.thinking_level(),
            epsilon_contracts::ThinkingLevel::Off,
            "thinking level unchanged under the gate"
        );

        // After accepting, cycling applies model + remembered level as usual.
        assert!(!type_and_enter(&mut app, &agent, "trust"));
        handle_key(&mut app, &agent, key(KeyCode::F(2)));
        assert_eq!(app.model_id, "deepseek-chat");
        assert_eq!(
            agent.thinking_level(),
            epsilon_contracts::ThinkingLevel::High
        );
    }

    // ---- M23: favorites ----------------------------------------------------------------

    #[tokio::test]
    async fn favorites_section_orders_above_recents_and_marks_stars() {
        let rows = crate::picker::build_rows_with(
            "",
            "faux-1",
            &["deepseek-chat".to_string()],
            &["kimi-k2.6".to_string()],
            &[],
            &[],
        );
        // Active model, then favorite, then recent (favorite NOT repeated in
        // recents), then catalog.
        assert_eq!(rows[0].id, "faux-1");
        let ids: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
        let fav_pos = ids.iter().position(|i| *i == "kimi-k2.6").unwrap();
        let rec_pos = ids.iter().position(|i| *i == "deepseek-chat").unwrap();
        assert!(fav_pos < rec_pos, "favorite must precede recent: {ids:?}");
        assert_eq!(rows[fav_pos].provider, "favorite");
        assert!(rows[fav_pos].is_favorite);
        // The star also shows on the same id inside the catalog section.
        assert!(
            rows.iter()
                .all(|r| if r.id == "kimi-k2.6" && r.provider != "favorite" {
                    r.is_favorite
                } else {
                    true
                })
        );
    }

    #[tokio::test]
    async fn f2_cycles_favorites_and_guides_when_empty() {
        let (mut app, agent, (_f, _n)) = test_app();
        // Empty list: guidance, no crash, model unchanged.
        handle_key(&mut app, &agent, key(KeyCode::F(2)));
        assert_eq!(app.model_id, "faux-1");

        app.favorites = vec!["faux-1".into(), "deepseek-chat".into()];
        handle_key(&mut app, &agent, key(KeyCode::F(2)));
        assert_eq!(app.model_id, "deepseek-chat", "forward cycle");
        handle_key(&mut app, &agent, key(KeyCode::F(2)));
        assert_eq!(app.model_id, "faux-1", "wraps around");

        handle_key(
            &mut app,
            &agent,
            KeyEvent::new(KeyCode::F(2), KeyModifiers::SHIFT),
        );
        assert_eq!(app.model_id, "deepseek-chat", "shift = backward");

        // Persistence side effects: cycling routes through apply_model, so the
        // saved model id must reflect the last switch. The recents list also
        // persists, but asserting the real global config here races every other
        // TUI test's writes (lesson 16b) — the in-memory mirror is what F2 owns.
        // The persisted model id is covered by config-layer round-trip tests;
        // asserting the real global file here races every other TUI test's
        // writes (lesson 16b). What F2 owns: session state + in-memory mirrors.
        assert_eq!(
            app.model_id, "deepseek-chat",
            "cycle switched the session model"
        );
        assert!(
            app.recents.first().map(String::as_str) == Some("deepseek-chat"),
            "in-memory recents updated with the cycled-to model"
        );
    }

    #[tokio::test]
    async fn f2_applies_the_favorite_remembered_thinking_level() {
        // M26 (pi _cycleScopedModel): a remembered level overrides the session
        // level on cycle; no memory inherits it. Both paths end with the footer
        // mirror equal to the agent's actual level. The memory is injected via
        // the App field (the same thing a real star toggle populates) so this
        // test never touches the user's real config.
        let (mut app, agent, (_f, _n)) = test_app();
        app.favorites = vec!["faux-1".into(), "deepseek-chat".into()];
        app.favorite_thinking = vec![("deepseek-chat".into(), "high".into())];

        handle_key(&mut app, &agent, key(KeyCode::F(2)));
        assert_eq!(app.model_id, "deepseek-chat");
        assert_eq!(
            agent.thinking_level(),
            epsilon_contracts::ThinkingLevel::High,
            "remembered level applied on cycle"
        );
        assert_eq!(app.thinking, agent.thinking_level(), "mirror stays exact");

        // Cycle back to faux-1, which has NO memory: session level inherited
        // (stays High), never reset by surprise.
        handle_key(&mut app, &agent, key(KeyCode::F(2)));
        assert_eq!(app.model_id, "faux-1");
        assert_eq!(
            agent.thinking_level(),
            epsilon_contracts::ThinkingLevel::High,
            "unremembered favorite inherits the session level"
        );
    }

    #[tokio::test]
    async fn slash_popup_opens_and_arrows_move_selection() {
        let (mut app, agent, (_f, _n)) = test_app();
        // Typing "/" opens the popup with all 6 candidates.
        for c in "/".chars() {
            handle_key(&mut app, &agent, key(KeyCode::Char(c)));
        }
        assert!(app.slash_open, "popup opens on slash");
        assert_eq!(app.slash_index, 0);

        // Down moves the highlight; Up at top stays clamped.
        handle_key(&mut app, &agent, key(KeyCode::Down));
        assert_eq!(app.slash_index, 1);
        handle_key(&mut app, &agent, key(KeyCode::Up));
        handle_key(&mut app, &agent, key(KeyCode::Up));
        assert_eq!(app.slash_index, 0, "clamped at top");

        // Arrows must NOT touch history while the popup is open.
        assert_eq!(app.input, "/");
    }

    #[tokio::test]
    async fn enter_applies_highlighted_suggestion_then_runs_it() {
        let (mut app, agent, (_f, _n)) = test_app();
        for c in "/m".chars() {
            handle_key(&mut app, &agent, key(KeyCode::Char(c)));
        }
        assert!(app.slash_open);
        let candidates = crate::picker::slash_items("/m");
        assert!(candidates.len() >= 2, "/models and /model both match");

        // Highlight the second candidate (/model — argument form) then Enter.
        // pi semantics: apply THEN submit, so "/model " opens the picker with
        // the filter prefilled (parse_command maps bare /model to Models).
        handle_key(&mut app, &agent, key(KeyCode::Down));
        assert!(!handle_key(&mut app, &agent, key(KeyCode::Enter)));
        assert!(app.picker_open, "applied /model ran and opened the picker");
        assert!(!app.slash_open, "popup closed after apply");
    }

    #[tokio::test]
    async fn enter_with_popup_on_exact_text_still_edits_before_running() {
        let (mut app, agent, (_f, _n)) = test_app();
        // "/cl" → only /clear matches; Enter applies+runs it.
        for c in "/cl".chars() {
            handle_key(&mut app, &agent, key(KeyCode::Char(c)));
        }
        assert!(app.slash_open);
        assert_eq!(crate::picker::slash_items("/cl").len(), 1);
        assert!(!handle_key(&mut app, &agent, key(KeyCode::Enter)));
        assert!(
            app.ui
                .lines()
                .last()
                .is_some_and(|l| l.text.contains("cleared")),
            "/clear executed via Enter-apply"
        );
        assert!(!app.slash_open);
    }

    #[tokio::test]
    async fn esc_dismisses_popup_before_quitting_or_aborting() {
        let (mut app, agent, (_f, _n)) = test_app();
        for c in "/h".chars() {
            handle_key(&mut app, &agent, key(KeyCode::Char(c)));
        }
        assert!(app.slash_open);
        // Idle Esc would quit — but the popup eats it first.
        assert!(!handle_key(&mut app, &agent, key(KeyCode::Esc)));
        assert!(!app.slash_open);
        assert_eq!(app.input, "/h", "text survives dismissal");
        // Next Esc quits (idle).
        assert!(handle_key(&mut app, &agent, key(KeyCode::Esc)));
    }

    #[tokio::test]
    async fn tab_completes_highlighted_candidate() {
        let (mut app, agent, (_f, _n)) = test_app();
        for c in "/c".chars() {
            handle_key(&mut app, &agent, key(KeyCode::Char(c)));
        }
        assert!(app.slash_open);
        assert!(!handle_key(&mut app, &agent, key(KeyCode::Tab)));
        assert_eq!(app.input, "/clear", "Tab completed the only /c match");
        assert!(!app.slash_open);
    }

    #[tokio::test]
    async fn popup_tracks_edits_and_closes_when_not_a_command() {
        let (mut app, agent, (_f, _n)) = test_app();
        for c in "/help".chars() {
            handle_key(&mut app, &agent, key(KeyCode::Char(c)));
        }
        assert!(app.slash_open);
        // A space makes it an argument line → popup closes.
        handle_key(&mut app, &agent, key(KeyCode::Char(' ')));
        assert!(!app.slash_open, "space closes the command-name popup");
    }

    // ---- M17b: new editor keys -------------------------------------------------------

    #[tokio::test]
    async fn ctrl_a_ctrl_e_home_end_line_semantics() {
        let (mut app, agent, (_f, _n)) = test_app();
        for c in "hello world".chars() {
            handle_key(&mut app, &agent, key(KeyCode::Char(c)));
        }
        handle_key(&mut app, &agent, ctrl(KeyCode::Char('a')));
        assert_eq!(app.cursor_chars, 0, "Ctrl+A = line home");
        handle_key(&mut app, &agent, ctrl(KeyCode::Char('e')));
        assert_eq!(
            app.cursor_chars,
            app.input.chars().count(),
            "Ctrl+E = line end"
        );
    }

    #[tokio::test]
    async fn ctrl_k_kills_to_line_end_into_kill_ring() {
        let (mut app, agent, (_f, _n)) = test_app();
        for c in "hello world".chars() {
            handle_key(&mut app, &agent, key(KeyCode::Char(c)));
        }
        handle_key(&mut app, &agent, ctrl(KeyCode::Char('a'))); // home
        handle_key(&mut app, &agent, ctrl(KeyCode::Char('e'))); // end
        handle_key(&mut app, &agent, ctrl(KeyCode::Char('b'))); // back one
        handle_key(&mut app, &agent, ctrl(KeyCode::Char('k'))); // kill "d"
        assert_eq!(app.input, "hello worl");
        // Ctrl+Y yanks the killed text back.
        handle_key(&mut app, &agent, ctrl(KeyCode::Char('y')));
        assert_eq!(app.input, "hello world");
    }

    #[tokio::test]
    async fn ctrl_d_forward_deletes_and_alt_backspace_kills_word() {
        let (mut app, agent, (_f, _n)) = test_app();
        for c in "ab cd".chars() {
            handle_key(&mut app, &agent, key(KeyCode::Char(c)));
        }
        handle_key(&mut app, &agent, ctrl(KeyCode::Char('a')));
        handle_key(&mut app, &agent, ctrl(KeyCode::Char('d')));
        assert_eq!(app.input, "b cd", "Ctrl+D deleted 'a'");

        handle_key(&mut app, &agent, key(KeyCode::End));
        let alt_bs = KeyEvent {
            code: KeyCode::Backspace,
            modifiers: KeyModifiers::ALT,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        };
        handle_key(&mut app, &agent, alt_bs);
        assert_eq!(app.input, "b ", "Alt+Backspace killed 'cd'");
    }

    #[tokio::test]
    async fn ctrl_dot_redoes_what_ctrl_z_undid() {
        let (mut app, agent, (_f, _n)) = test_app();
        for c in "keep me".chars() {
            handle_key(&mut app, &agent, key(KeyCode::Char(c)));
        }
        handle_key(&mut app, &agent, key(KeyCode::Backspace));
        assert_eq!(app.input, "keep m");
        handle_key(&mut app, &agent, ctrl(KeyCode::Char('z')));
        assert_eq!(app.input, "keep me", "undo restored");
        handle_key(&mut app, &agent, ctrl(KeyCode::Char('.')));
        assert_eq!(app.input, "keep m", "redo re-applied the deletion");
    }

    #[tokio::test]
    async fn ctrl_b_ctrl_f_move_cursor_by_char() {
        let (mut app, agent, (_f, _n)) = test_app();
        for c in "abc".chars() {
            handle_key(&mut app, &agent, key(KeyCode::Char(c)));
        }
        assert_eq!(app.cursor_chars, 3);
        handle_key(&mut app, &agent, ctrl(KeyCode::Char('b')));
        assert_eq!(app.cursor_chars, 2);
        handle_key(&mut app, &agent, ctrl(KeyCode::Char('f')));
        assert_eq!(app.cursor_chars, 3);
    }

    // ---- M30: sticky scroll (pi followEnd semantics) ----------------------------------

    #[tokio::test]
    async fn scrolling_up_freezes_the_reading_window_while_lines_append() {
        let mut app = App::over_ui(UiState::new(), "faux-1".into());
        app.ui.push_notice("one");
        app.ui.push_notice("two");
        // User pages up 10 rows from the bottom.
        app.scroll_from_bottom = 10;
        // Three rows append mid-read (each AgentStart pushes one system row).
        for _ in 0..3 {
            apply_with_scroll(&mut app, &AgentEvent::AgentStart);
            apply_with_scroll(&mut app, &AgentEvent::AgentEnd { messages: vec![] });
        }
        assert_eq!(
            app.scroll_from_bottom, 16,
            "offset grows with appends (2 rows x 3 pairs) so the window stays put"
        );

        // A pinned view (offset 0) keeps following the tail.
        app.scroll_from_bottom = 0;
        apply_with_scroll(&mut app, &AgentEvent::AgentStart);
        assert_eq!(app.scroll_from_bottom, 0, "pinned view follows new rows");
    }
}
