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
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::state::UiState;
use crate::theme::Theme;

/// Async provider-listing hook injected by the CLI when built with `--features real`
/// (M13): takes a model-id hint, returns that provider's live catalog. Errors in-band.
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
    guidance_mode: bool,
    scroll_from_bottom: usize,
    model_id: String,
    picker_open: bool,
    picker_index: usize,
    recents: Vec<String>,
    live_cache: Vec<String>,
    live: Option<LiveFetcher>,
    prefs: crate::state::RenderPrefs,
    fetching_hint: Option<String>,
    fetched_tx: tokio::sync::mpsc::UnboundedSender<Result<Vec<String>, String>>,
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
    /// Workspace root for the footer (pi footer parity).
    workspace_root: std::path::PathBuf,
    /// Cached git branch + when it was read (2s TTL — no git dependency).
    branch_cache: (Option<String>, std::time::Instant),
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
        let rows =
            crate::picker::build_rows(&self.input, &self.model_id, &self.recents, &self.live_cache);
        self.picker_rows = Some(rows.clone());
        rows
    }
}

/// Braille spinner frames (pi loader convention).
const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

impl App {
    #[allow(clippy::too_many_arguments)]
    fn new(
        model_id: String,
        fetched_tx: tokio::sync::mpsc::UnboundedSender<Result<Vec<String>, String>>,
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
            guidance_mode: false,
            scroll_from_bottom: 0,
            model_id,
            picker_open: false,
            picker_index: 0,
            recents: epsilon_core::config::load_recent_models(),
            live_cache: Vec::new(),
            live: None,
            prefs: crate::state::RenderPrefs::default(),
            fetching_hint: None,
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
            branch_cache: (
                None,
                std::time::Instant::now() - std::time::Duration::from_secs(10),
            ),
            workspace_root,
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
            }
            crate::picker::Command::Model(id) => self.apply_model(agent, id),
            crate::picker::Command::Help => {
                for line in [
                    "commands: /models · /model <id> · /skills · /clear · /quit · /help",
                    "keys: Enter submit · Esc aborts a run (quits when idle) · x expands tool output",
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
            self.recents = epsilon_core::config::load_recent_models();
            self.invalidate_picker();
        }
    }

    /// Apply the currently highlighted picker row.
    fn pick_selected(&mut self, agent: &Arc<Agent>) {
        let rows = self.picker_rows();
        let Some(row) = rows.get(self.picker_index) else {
            return;
        };
        // UX guard (M14): Enter on the already-active model must not silently close
        // the picker with zero feedback — that read as "slash commands don't work".
        if row.id == self.model_id {
            self.ui.push_notice(format!(
                "{id} is already active — ↓ to browse, Esc to close",
                id = row.id
            ));
            return;
        }
        let id = row.id.clone();
        self.picker_open = false;
        self.picker_index = 0;
        self.input.clear();
        self.cursor_chars = 0;
        self.apply_model(agent, id);
    }

    /// Kick off a live listing for the highlighted row's provider (M13, `real` builds).
    fn fetch_live_for_selected(&mut self) {
        let Some(fetcher) = self.live.clone() else {
            self.notice("live listing unavailable: built without --features real");
            return;
        };
        let rows = self.picker_rows();
        let Some(row) = rows.get(self.picker_index) else {
            return;
        };
        if row.is_custom {
            self.notice("custom ids have no provider to list; select one first");
            return;
        }
        if self.fetching_hint.is_some() {
            return; // one in flight
        }
        let hint = row.id.clone();
        self.fetching_hint = Some(hint.clone());
        self.notice(format!("fetching live models for '{}'…", row.provider));
        let tx = self.fetched_tx.clone();
        let fut = fetcher(hint);
        tokio::spawn(async move {
            let result = fut.await;
            tx.send(result).ok();
        });
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
    } = cfg;

    let mut events_rx = agent.subscribe();
    let (notices_tx, mut notices_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let (fetched_tx, mut fetched_rx) =
        tokio::sync::mpsc::unbounded_channel::<Result<Vec<String>, String>>();
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

    if let Some(task) = initial_task {
        let msg = app.submit_text(task);
        app.spawn_run(Arc::clone(&agent), msg);
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
                if let Some(result) = maybe_fetched {
                    match result {
                        Ok(mut names) => {
                            let n = names.len();
                            app.live_cache.append(&mut names);
                            app.invalidate_picker();
                            app.ui.push_notice(format!("live listing: {n} models merged into /models"));
                        }
                        Err(e) => app.ui.push_notice(format!("live listing failed: {e}")),
                    }
                    app.fetching_hint = None;
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

/// Apply one event; any newly appended transcript row resets follow-scroll pinning.
fn apply_with_scroll(app: &mut App, ev: &AgentEvent) {
    let before = app.ui.lines().len();
    app.ui.apply(ev);
    match ev {
        AgentEvent::AgentStart => app.run_started_at = Some(std::time::Instant::now()),
        AgentEvent::AgentEnd { .. } => app.run_started_at = None,
        _ => {}
    }
    if app.ui.lines().len() > before {
        app.scroll_from_bottom = 0;
    }
}

/// Handle one key press; returns true when the app should quit.
fn handle_key(app: &mut App, agent: &Arc<Agent>, key: KeyEvent) -> bool {
    if key.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(key.code, KeyCode::Char('c') | KeyCode::Char('q'))
    {
        return true;
    }

    // Model picker captures keys before the permission dialog (it can open while a
    // run is active, and must always be operable).
    if app.picker_open {
        match key.code {
            KeyCode::Esc => {
                app.picker_open = false;
                app.picker_index = 0;
                app.input.clear();
                app.cursor_chars = 0;
            }
            KeyCode::Enter => app.pick_selected(agent),
            KeyCode::Up => {
                let total = app.picker_rows().len();
                app.picker_index = app
                    .picker_index
                    .saturating_sub(1)
                    .min(total.saturating_sub(1));
            }
            KeyCode::Down => {
                let total = app.picker_rows().len();
                if total > 0 {
                    app.picker_index = (app.picker_index + 1).min(total - 1);
                }
            }
            KeyCode::Backspace => {
                app.backspace();
                app.invalidate_picker();
                let total = app.picker_rows().len();
                if app.picker_index >= total {
                    app.picker_index = total.saturating_sub(1);
                }
            }
            KeyCode::Char('f' | 'F') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                // Ctrl+F: fetch live listing for the highlighted row's provider.
                app.fetch_live_for_selected();
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
            // M12: Esc during a run aborts it cooperatively; idle Esc quits.
            if app.ui.running() {
                if agent.abort_current_run() {
                    app.notice("abort requested — cancelling current run…");
                }
                false
            } else {
                true
            }
        }
        KeyCode::Enter => app.submit_editor(agent),
        KeyCode::Char('m' | 'M') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            // Layout-independent picker access (slash may need AltGr on some layouts).
            if !app.picker_open {
                app.picker_open = true;
                app.picker_index = 0;
                app.input.clear();
                app.cursor_chars = 0;
            }
            false
        }
        KeyCode::Backspace => {
            app.backspace();
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
            app.recall(true);
            false
        }
        KeyCode::Down => {
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
            // M15 slash autocomplete: Tab completes the first candidate.
            let candidates = slash_candidates(&app.input);
            if let Some(first) = candidates.first() {
                app.input = (*first).to_string();
                app.cursor_chars = app.input.chars().count();
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
        KeyCode::Char('x' | 'X') => {
            // M13 card polish: expand/collapse truncated tool outputs globally.
            app.prefs.expand_outputs = !app.prefs.expand_outputs;
            let state = if app.prefs.expand_outputs {
                "expanded"
            } else {
                "collapsed"
            };
            app.notice(format!("tool outputs {state} (x toggles)"));
            false
        }
        KeyCode::Char(c) => {
            app.insert_char(c);
            false
        }
        _ => false,
    }
}

/// Contextual key-hint bar (pi's keybinding-hints pattern).
fn hint_line(app: &App) -> Line<'static> {
    use crate::theme::Theme;
    let text = if app.pending_ask.is_some() {
        "[y] allow · [a] always · [n] deny · [e] guidance · [Esc] deny"
    } else if app.picker_open {
        "type to filter · ↑/↓ move · Ctrl+F fetch live · Enter apply · Esc close"
    } else if app.ui.running() {
        "Esc abort run · type to steer after this turn"
    } else {
        "/help commands · /models picker (Ctrl+M) · x expand outputs · Esc quit"
    };
    Line::from(Span::styled(text.to_string(), Theme::system()))
}

fn draw(f: &mut Frame<'_>, app: &mut App) {
    let [transcript_area, hints_area, input_area, status_area] = Layout::vertical([
        Constraint::Min(3),
        Constraint::Length(1),
        Constraint::Length(3),
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
    let rows = app.picker_rows();
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
            "no matches — keep typing or press Esc",
            Style::default().fg(Color::DarkGray),
        )));
    }
    // M15 viewport fix: pi's centered scroll window — render only the visible slice
    // so long lists stay navigable (previously rows past the box were unreachable).
    let capacity = area.height.saturating_sub(4).max(1) as usize; // borders + footer + headroom
    let start = crate::scroll::scroll_start(rows.len(), app.picker_index, capacity);
    let end = (start + capacity).min(rows.len());

    let mut lines: Vec<Line<'static>> = Vec::new();
    if rows.is_empty() {
        lines.push(Line::from(Span::styled(
            "no matches — keep typing or press Esc",
            Style::default().fg(Color::DarkGray),
        )));
    }
    for (i, row) in rows.iter().enumerate().skip(start).take(end - start) {
        let selected = i == app.picker_index;
        let marker = if row.id == app.model_id { "● " } else { "  " };
        let auth = if row.auth_ready { "🔑" } else { "· " };
        let source_tag = if row.is_live {
            "⟳ "
        } else if row.is_recent {
            "↻ "
        } else {
            ""
        };
        let style = if selected {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default()
        };
        let label = if row.is_custom {
            format!("use custom id '{0}'", row.id)
        } else {
            row.label.clone()
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
            Span::styled(source_tag.to_string(), Style::default().fg(Color::Cyan)),
            Span::styled(
                label,
                style.add_modifier(if selected {
                    ratatui::style::Modifier::REVERSED
                } else {
                    ratatui::style::Modifier::empty()
                }),
            ),
            Span::styled(
                format!("  {} · {}", row.provider, row.id),
                Style::default().fg(Color::DarkGray),
            ),
        ]));
    }

    lines.push(Line::from(""));
    let position = if rows.len() > capacity {
        format!("{}–{}/{} · ", start + 1, end, rows.len())
    } else {
        String::new()
    };
    lines.push(Line::from(Span::styled(
        format!("{position}↑/↓ · type filter · Ctrl+F fetch · Enter apply · Esc close"),
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
        "input — Ctrl+J newline · Ctrl+Y yank"
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
    let para = Paragraph::new(Text::from(Line::from(vec![
        Span::styled("> ", Style::default().fg(prompt_color)),
        Span::raw(app.input.clone()),
    ])))
    .block(frame_box);
    f.render_widget(para, area);

    // M16b: cursor position accounts for embedded newlines — the caret sits on the
    // LAST visible line of the buffer (multi-line drafts clamp to the box).
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
    let visible_rows = area.height.saturating_sub(2).max(1) as usize; // borders
    let row = area.y + 1 + (line_idx.min(visible_rows - 1)) as u16;
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

/// Known slash commands for Tab completion (M15, opencode convention).
const SLASH_COMMANDS: [&str; 6] = ["/models", "/model ", "/skills", "/clear", "/help", "/quit"];

/// Candidates whose head matches the typed `/prefix` (input has no space yet).
fn slash_candidates(input: &str) -> Vec<&'static str> {
    if !input.starts_with('/') || input.contains(' ') {
        return Vec::new();
    }
    SLASH_COMMANDS
        .iter()
        .filter(|c| c.starts_with(input))
        .copied()
        .collect()
}

/// Popup listing matching slash commands directly above the input box.
fn draw_slash_autocomplete(f: &mut Frame<'_>, app: &App, transcript_area: Rect) {
    let candidates = slash_candidates(&app.input);
    if candidates.is_empty() || candidates.len() == SLASH_COMMANDS.len() && app.input == "/" {
        // "/" alone shows all — fine to render; skip only the degenerate no-match case:
    }
    if candidates.is_empty() {
        return;
    }
    let height = candidates.len() as u16 + 1;
    let y = transcript_area.bottom().saturating_sub(height);
    let area = Rect {
        x: transcript_area.x,
        y,
        width: transcript_area.width.min(48),
        height,
    };
    f.render_widget(Clear, area);
    let mut lines: Vec<Line<'static>> = candidates
        .iter()
        .map(|c| Line::from(Span::styled((*c).to_string(), Theme::accent())))
        .collect();
    lines.push(Line::from(Span::styled(
        "Tab completes · Esc dismisses".to_string(),
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

    type TestReceivers = (
        tokio::sync::mpsc::UnboundedReceiver<Result<Vec<String>, String>>,
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
        assert!(
            app.recents.contains(&"deepseek-chat".to_string()),
            "recents updated"
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
        assert!(!handle_key(&mut app, &agent, key(KeyCode::Char('x'))));
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
}
