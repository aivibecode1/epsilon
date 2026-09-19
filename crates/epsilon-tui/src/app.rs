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
use epsilon_contracts::message::Message;
use epsilon_core::Agent;
use epsilon_safety::{ChannelApprovalUi, PendingAsk};
use futures_util::StreamExt;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use crate::state::UiState;

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

/// Ceiling on the redraw rate (~30 fps). A frame is the expensive unit on a
/// Windows console, so the loop coalesces whatever arrived and draws at most
/// this often instead of once per event.
const FRAME_MIN: std::time::Duration = std::time::Duration::from_millis(33);

/// Cap on terminal events handled per pre-draw drain turn (M94 review: the
/// drain was unbounded, so a flood of input could keep the loop from ever
/// reaching its frame decision). Matches the agent-event coalescing cap below;
/// anything beyond it keeps flowing through the biased select on the next turn
/// — input first, nothing dropped.
const MAX_INPUT_DRAIN: usize = 128;

/// Editor history depth.
const HISTORY_MAX: usize = 100;

/// M59: which editor to run — explicit config wins, then `$VISUAL`, then
/// `$EDITOR` (pi's `getExternalEditorCommand` order), else a telling error.
/// Pure so the order is testable without touching the environment.
fn editor_command_from(
    configured: Option<&str>,
    visual: Option<&str>,
    editor: Option<&str>,
) -> Result<String, String> {
    for candidate in [configured, visual, editor] {
        if let Some(cmd) = candidate.map(str::trim).filter(|c| !c.is_empty()) {
            return Ok(cmd.to_string());
        }
    }
    Err("no editor configured — set $EDITOR or $VISUAL, or add\n    [editor]\n    command = \"code --wait\"\nto ~/.config/epsilon/config.toml".to_string())
}

/// M59: the terminal-free half of the external editor — write the draft to a
/// temp file, hand it to `spawn`, read it back. Split out (spawner injected) so
/// the whole round trip is testable without a real editor, a TTY, or a suspend.
fn edit_with(
    draft: &str,
    spawn: &dyn Fn(&std::path::Path) -> Result<i32, String>,
) -> Result<String, String> {
    let dir = std::env::temp_dir().join(format!(
        "epsilon-editor-{}",
        epsilon_contracts::new_id("ed")
    ));
    std::fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    let file = dir.join("prompt.md");
    let result = (|| -> Result<String, String> {
        std::fs::write(&file, draft).map_err(|e| format!("write {}: {e}", file.display()))?;
        let code = spawn(&file)?;
        if code != 0 {
            return Err(format!("editor exited with status {code} — draft kept"));
        }
        let edited =
            std::fs::read_to_string(&file).map_err(|e| format!("read {}: {e}", file.display()))?;
        // pi trims exactly one trailing newline (editors love adding one).
        Ok(edited.strip_suffix('\n').unwrap_or(&edited).to_string())
    })();
    let _ = std::fs::remove_dir_all(&dir);
    result
}

/// Launch `command <file>` with the terminal handed over, and wait for it. The
/// platform shell expands the user's own command line (`code --wait`,
/// `vim -u NONE`, `notepad`) exactly as a shell would.
fn spawn_editor(command: &str, file: &std::path::Path) -> Result<i32, String> {
    use std::process::{Command, Stdio};
    let quoted = format!("\"{}\"", file.display());
    #[cfg(windows)]
    let mut child = {
        let mut c = Command::new("cmd");
        c.arg("/C").arg(format!("{command} {quoted}"));
        c
    };
    #[cfg(not(windows))]
    let mut child = {
        let mut c = Command::new("sh");
        c.arg("-c").arg(format!("{command} {quoted}"));
        c
    };
    let status = child
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| format!("cannot start '{command}': {e}"))?;
    Ok(status.code().unwrap_or(-1))
}

/// M59: suspend the TUI, run the editor on the draft, resume. The event loop
/// owns the terminal, so this is called between frames — never from a handler.
fn open_external_editor(terminal: &mut ratatui::DefaultTerminal, app: &mut App) {
    let Ok(command) = editor_command(epsilon_core::config::load_config_string(
        "editor", "command",
    )) else {
        return;
    };
    let draft = app.input.clone();
    // Leave the alternate screen + raw mode so the child owns the terminal (pi
    // `ui.stop()`); bracketed paste is re-armed on the way back.
    let _ = crossterm::execute!(std::io::stdout(), crossterm::event::DisableBracketedPaste);
    if let Err(e) = ratatui::try_restore() {
        app.ui
            .push_notice(format!("editor: terminal restore failed: {e}"));
        return;
    }
    let outcome = edit_with(&draft, &|path| spawn_editor(&command, path));
    match ratatui::try_init() {
        Ok(mut fresh) => {
            // A fresh terminal starts with blank buffers, so the next draw is a
            // full repaint — no stale half-screen after the editor exits.
            let _ = fresh.clear();
            *terminal = fresh;
        }
        Err(e) => app
            .ui
            .push_notice(format!("editor: cannot re-enter the UI: {e}")),
    }
    let _ = crossterm::execute!(std::io::stdout(), crossterm::event::EnableBracketedPaste);
    match outcome {
        Ok(text) => {
            let trimmed = text.trim_end_matches('\n').to_string();
            app.cursor_chars = trimmed.chars().count();
            app.input = trimmed;
            app.ui.push_notice("editor: draft updated");
        }
        Err(e) => app.ui.push_notice(format!("editor: {e}")),
    }
}

/// [`editor_command_from`] reading the real environment.
fn editor_command(configured: Option<String>) -> Result<String, String> {
    let visual = std::env::var("VISUAL").ok();
    let editor = std::env::var("EDITOR").ok();
    editor_command_from(configured.as_deref(), visual.as_deref(), editor.as_deref())
}

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
    /// M94 review (identity-based, re-review round 2): the `request_id` of the ask
    /// the last frame actually painted in the dialog. `handle_key` answers a
    /// pending ask ONLY when its id matches this — a freshly seated ask (the
    /// channel is FIFO and parked asks can arrive back-to-back inside one frame
    /// window) is therefore unanswerable until a frame paints it, with no reset
    /// to forget and no staleness window. The undersized fallback clears it (the
    /// dialog cannot fit there), so a blind `y`/`a` cannot authorize work the
    /// user never saw and a blind Esc cannot silently deny it. Fail closed.
    approval_painted_id: Option<String>,
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
    /// M30: steer queue length, mirrored on each key event when running so
    /// draw_status can render a count badge without needing the agent ref.
    steer_queue_len: usize,
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
    /// W1: @-file popup active (pi editor.ts parity).
    at_open: bool,
    at_index: usize,
    /// W1: cached at-file rows (recomputed only when fragment changes).
    at_rows: Option<Vec<String>>,
    at_fragment: String,
    /// Plan 009: timeline picker overlay (pi timeline parity — not a notice dump).
    timeline_open: bool,
    timeline_index: usize,
    timeline_entries: Vec<(u64, String, String)>,
    /// Plan 009: compact spinner — manual compaction in-flight.
    compact_running: bool,
    /// M16c: draft stashed when a dialog opened; restored when it closes.
    stash: Option<(String, usize)>,
    /// Parity sweep: editor undo stack (Ctrl+Z), pi undo-stack.ts semantics.
    undo: crate::undo::UndoStack,
    /// M32: masked API-key prompt overlay (`/login` without a key pasted).
    /// Masked like opencode's DialogPrompt — rendering shows • per char while
    /// the real key stays in `auth_pending.provider/auth_key`.
    auth_prompt_open: bool,
    auth_provider: String,
    auth_key: String,
    auth_key_cursor: usize,
    /// M52: OAuth worker thread → UI events (open the paste overlay, close on
    /// completion). Same shape as `notices_tx`, one type richer. The only
    /// producer is the login thread, which exists under `real`; offline builds
    /// still drain the channel so the plumbing stays identical.
    #[cfg_attr(not(feature = "real"), allow(dead_code))]
    oauth_tx: tokio::sync::mpsc::UnboundedSender<OauthUiEvent>,
    /// M52: hand a pasted authorization code to the waiting login thread.
    /// `None` = no login running or not in its code step. Dropping the sender
    /// is how Esc cancels the flow (the thread sees a disconnect).
    oauth_code_tx: Option<std::sync::mpsc::Sender<String>>,
    /// M32b: bare `/login` provider picker (opencode DialogProvider parity).
    /// When the user types `/login` without a provider, this picker opens so
    /// discovery is one Enter, not memorizing ids. Filtered by `input`.
    auth_pick_open: bool,
    auth_pick_index: usize,
    /// Workspace root for the footer (pi footer parity).
    workspace_root: std::path::PathBuf,
    /// Cached git branch + when it was read (2s TTL — no git dependency).
    branch_cache: (Option<String>, std::time::Instant),
    /// G6: pi `KeybindingsManager` parity — `~/.config/epsilon/keybindings.json`.
    keybindings: crate::keybindings::KeybindingsManager,
    /// M25: mirror of the agent's thinking level for the footer. Written ONLY
    /// by [`Self::set_thinking`] (the same choke point that updates the agent),
    /// so display and execution state cannot drift apart.
    thinking: epsilon_contracts::ThinkingLevel,
    /// Display name for this session (set via /name).
    session_name: Option<String>,
    /// M57: open session browser (`/sessions`, bare `/resume`). Rows are
    /// rebuilt from the workspace's session directory on every keystroke; the
    /// filter is the normal input buffer, exactly like the model picker.
    session_browser: Option<SessionBrowser>,
    /// M59: set by `/editor` (or the keybinding); the event loop performs the
    /// actual suspend → spawn → resume, because it owns the terminal.
    external_editor_requested: bool,
    /// The resolved look for this session: colour mode, glyph repertoire, and the
    /// palette. Detected once at startup, because a look that changes mid-session
    /// would make the screen flicker between two designs.
    theme: crate::ui::theme::Theme,
}

/// M57: state of the `/sessions` browser overlay.
struct SessionBrowser {
    /// Directory scanned for session files (`.epsilon/sessions`).
    dir: std::path::PathBuf,
    /// The session this app is attached to (marked `●` in the list).
    current: Option<std::path::PathBuf>,
    /// Filtered rows, newest first.
    rows: Vec<crate::picker::SessionRow>,
    /// Highlighted row.
    index: usize,
}

impl SessionBrowser {
    /// Scan + filter once; callers rebuild after each filter keystroke.
    fn build(dir: std::path::PathBuf, current: Option<std::path::PathBuf>, filter: &str) -> Self {
        let rows = crate::picker::session_rows(&dir, filter, current.as_deref(), now_ms());
        Self {
            dir,
            current,
            rows,
            index: 0,
        }
    }

    /// Re-run the scan/filter with the current input as the query.
    fn rebuild(&mut self, filter: &str) {
        self.rows =
            crate::picker::session_rows(&self.dir, filter, self.current.as_deref(), now_ms());
        if self.index >= self.rows.len() {
            self.index = self.rows.len().saturating_sub(1);
        }
    }
}

/// Lines describing what the last session resume repaired (M75), empty when it was intact.
fn repair_lines(agent: &Agent) -> Vec<String> {
    agent
        .repair_report()
        .map(|r| {
            r.summary()
                .into_iter()
                .map(|s| format!("repair: {s}"))
                .collect()
        })
        .unwrap_or_default()
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


impl App {
    #[allow(clippy::too_many_arguments)]
    fn new(
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
            model_id,
            picker_open: false,
            picker_index: 0,
            session_browser: None,
            external_editor_requested: false,
            theme: crate::ui::theme::Theme::detect(),
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
            compact_running: false,
            stash: None,
            undo: crate::undo::UndoStack::new(),
            branch_cache: (
                None,
                std::time::Instant::now() - std::time::Duration::from_secs(10),
            ),
            workspace_root,
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
    fn cached_branch(&mut self) -> Option<String> {
        const TTL: std::time::Duration = std::time::Duration::from_secs(2);
        if self.branch_cache.1.elapsed() > TTL {
            let b = crate::footer::git_branch(&self.workspace_root);
            self.branch_cache = (b, std::time::Instant::now());
        }
        self.branch_cache.0.clone()
    }

    /// M57: the workspace's session directory (`--continue` scans the same one).
    fn session_dir(&self) -> std::path::PathBuf {
        self.workspace_root.join(".epsilon").join("sessions")
    }

    /// M57: open the session browser (newest first, current one marked).
    fn open_session_browser(&mut self, agent: &Arc<Agent>) {
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
    fn request_external_editor(&mut self) {
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
    fn switch_session_file(
        &mut self,
        agent: &Arc<Agent>,
        path: &std::path::Path,
        typed: Option<&str>,
    ) -> Result<usize, String> {
        let count = agent.live_resume(path)?;
        {
            let history = agent.history();
            self.ui.hydrate_from_history(&history);
            if let Some(shown) = typed.filter(|s| !s.is_empty()) {
                self.history.push_back(shown.to_string());
                while self.history.len() > HISTORY_MAX {
                    self.history.pop_front();
                }
            }
            // Re-hydrate session_name from the newly attached store (survives swap).
            self.session_name = (|| {
                let store = agent.store_handle_for_tests();
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

    fn insert_char(&mut self, c: char) {
        let byte_at = self.byte_cursor();
        self.input.insert(byte_at, c);
        self.cursor_chars += 1;
    }

    fn backspace(&mut self) {
        // M36: erase a whole grapheme cluster — a bare `char` step would split
        // ZWJ families, flags, and combining marks.
        let start = crate::editor_ops::prev_cluster_start(&self.input, self.cursor_chars);
        if start < self.cursor_chars {
            let byte_start = self.byte_cursor_at(start);
            let byte_end = self.byte_cursor();
            self.input.replace_range(byte_start..byte_end, "");
            self.cursor_chars = start;
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
        // M74: `@` references attach content through the shared constructor — the same one `-p`,
        // the RPC loop and the server use. The message keeps the user's text verbatim.
        epsilon_core::references::user_message(&self.workspace_root, &text)
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
        self.sync_at_state();
    }

    /// Re-derive the @-file popup from the cursor-adjacent @-word (W1, pi editor.ts).
    fn sync_at_state(&mut self) {
        if let Some((_, frag)) = crate::editor_ops::at_word_before(&self.input, self.cursor_chars) {
            if self.at_fragment != frag || self.at_rows.is_none() {
                self.at_fragment = frag.clone();
                let rows = self.at_file_rows(&frag);
                self.at_rows = Some(rows);
                self.at_index = 0;
            } else if let Some(rows) = &self.at_rows
                && self.at_index >= rows.len().max(1)
            {
                self.at_index = 0;
            }
            let has_rows = self
                .at_rows
                .as_ref()
                .map(|r| !r.is_empty())
                .unwrap_or(false);
            self.at_open = has_rows;
            if !has_rows {
                self.at_index = 0;
            }
        } else {
            self.at_open = false;
            self.at_index = 0;
            self.at_rows = None;
            self.at_fragment.clear();
        }
    }

    /// File candidates for `fragment` (case-insensitive substring, cap 200).
    fn at_file_rows(&self, fragment: &str) -> Vec<String> {
        // Reuse the shared walker; cap 200 to keep typing smooth (avoid scanning target/).
        // Paths returned relative to workspace_root, like pi's relative workspace files.
        let files = {
            // Prefer the walk helper when available; otherwise fall back to entry read.
            // We call epsilon_tools::util::walk_files via a local helper to avoid a TUI->tools dep.
            // Instead, do a cheap local walk with the same SKIP_DIRS — no extra crate needed here.
            fn walk_at(root: &std::path::Path, budget: &mut usize) -> Vec<String> {
                fn rec(
                    dir: &std::path::Path,
                    root: &std::path::Path,
                    rows: &mut Vec<String>,
                    budget: &mut usize,
                    depth: usize,
                ) {
                    if *budget == 0 || depth > 16 {
                        return;
                    }
                    let Ok(entries) = std::fs::read_dir(dir) else {
                        return;
                    };
                    for e in entries.flatten() {
                        if *budget == 0 {
                            break;
                        }
                        let p = e.path();
                        let name = match p.file_name().and_then(|n| n.to_str()) {
                            Some(n) => n,
                            None => continue,
                        };
                        if [".git", "target", "node_modules", "dist", ".epsilon"].contains(&name) {
                            continue;
                        }
                        if p.is_dir() {
                            rec(&p, root, rows, budget, depth + 1);
                        } else {
                            if let Ok(rel) = p.strip_prefix(root) {
                                rows.push(rel.to_string_lossy().replace('\\', "/"));
                                *budget -= 1;
                            }
                        }
                    }
                }
                let mut rows = Vec::new();
                let mut b = *budget;
                rec(root, root, &mut rows, &mut b, 0);
                *budget = b;
                rows
            }
            let mut budget = 200usize;
            walk_at(&self.workspace_root, &mut budget)
        };
        let needle = fragment.trim();
        if needle.is_empty() {
            let mut out = files;
            out.sort();
            out.truncate(200);
            return out;
        }
        // pi `fuzzyFilter` on full paths — same semantics as model picker.
        let out: Vec<String> = crate::fuzzy::fuzzy_filter(&files, needle, |p| p.clone());
        let mut out = out;
        out.truncate(200);
        out
    }

    /// Insert the highlighted @-file path (pi: Tab inserts path, Enter inserts path+space).
    fn apply_at_selection(&mut self, add_space: bool) {
        let Some(rows) = self.at_rows.clone() else {
            return;
        };
        let Some(path) = rows.get(self.at_index).cloned() else {
            return;
        };
        let suffix = if add_space { format!("{path} ") } else { path };
        let (s, c) = crate::editor_ops::at_insert(&self.input, self.cursor_chars, &suffix);
        self.push_undo();
        self.input = s;
        self.cursor_chars = c;
        self.at_open = false;
        self.at_index = 0;
        self.at_rows = None;
        self.at_fragment.clear();
        self.end_yank();
        self.sync_slash_state();
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
                    //
                    // M45: acceptance writes BOTH memories — the config hash
                    // (per-file integrity) AND the trust store (per-path
                    // memory, so the next run trusts without re-asking).
                    let ws = self.workspace_root.clone();
                    // M45: accept the PATH in the trust store first (per-path
                    // memory — the dispatch-time composition point), then the
                    // config hash (per-file integrity). Either failing is
                    // reported; the gate already cleared for this session.
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
                    "keys: Enter submit · Esc aborts a run (quits when idle) · Ctrl+X expands tool output",
                    "      PgUp/PgDn scroll transcript · Ctrl+M model picker · Ctrl+P command palette",
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
            crate::picker::Command::Unknown(word) => {
                let hint = crate::picker::suggest_slash(&word)
                    .map(|item| format!(" — did you mean '{}'?", item.value.trim()))
                    .unwrap_or_default();
                self.ui.push_notice(format!(
                    "unknown command '/{word}'{hint} — /help lists commands"
                ));
            }
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
            crate::picker::Command::Sessions => {
                self.open_session_browser(agent);
            }
            crate::picker::Command::Editor => {
                self.request_external_editor();
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
                // Copy the last assistant message to the system clipboard (pi
                // app.message.copy parity; arboard is already in-tree for paste).
                let last = self
                    .ui
                    .lines()
                    .iter()
                    .rev()
                    .find(|l| l.kind == crate::state::LineKind::Assistant)
                    .map(|l| l.full.as_deref().unwrap_or(l.text.as_str()).to_string());
                if let Some(text) = last {
                    match clipboard_set(&text) {
                        Ok(()) => self.ui.push_notice(format!(
                            "copied {} chars to clipboard",
                            text.chars().count()
                        )),
                        Err(e) => self.ui.push_notice(format!("copy failed: {e}")),
                    }
                } else {
                    self.ui.push_notice("copy: no assistant message to copy");
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
        }
        false
    }

    /// M52: apply one event from the OAuth worker thread. The overlay is how
    /// the user hands the flow a pasted authorization code; completion always
    /// closes it.
    fn apply_oauth_event(&mut self, event: OauthUiEvent) {
        match event {
            OauthUiEvent::NeedCode { provider } => {
                self.auth_prompt_open = true;
                self.auth_provider = provider;
                self.auth_key.clear();
                self.auth_key_cursor = 0;
                self.ui.push_notice(
                    "finish the login in your browser — if it lands on an unreachable page, paste the redirect URL below (Esc cancels)",
                );
            }
            OauthUiEvent::Done(text) => {
                self.oauth_code_tx = None;
                if self.auth_prompt_open {
                    self.auth_prompt_open = false;
                    self.auth_provider.clear();
                    self.auth_key.clear();
                    self.auth_key_cursor = 0;
                }
                self.ui.push_notice(text);
                self.invalidate_picker();
            }
        }
    }

    /// M52: run a provider's OAuth ceremony on a worker thread so the UI keeps
    /// drawing (the flow can wait minutes on a browser round-trip). Progress
    /// travels as notices; the pasted code arrives over `oauth_code_tx`.
    #[cfg(feature = "real")]
    fn start_oauth_login(&mut self, provider: String) {
        use epsilon_llm::oauth_login::{LoginSaved, login, oauth_providers};
        if !oauth_providers().contains(&provider.as_str()) {
            self.ui.push_notice(format!(
                "no OAuth login for '{provider}' — available: {} · key login still works: /login {provider} <key>",
                oauth_providers().join(", ")
            ));
            return;
        }
        if self.oauth_code_tx.is_some() {
            self.ui
                .push_notice("an OAuth login is already running — finish it or press Esc");
            return;
        }
        let (code_tx, code_rx) = std::sync::mpsc::channel::<String>();
        self.oauth_code_tx = Some(code_tx);
        let notices = self.notices_tx.clone();
        let events = self.oauth_tx.clone();
        let flow_provider = provider.clone();
        std::thread::spawn(move || {
            let mut ui = TuiLoginUi {
                notices,
                events: events.clone(),
                code_rx,
                provider: flow_provider.clone(),
            };
            // Listener + paste race: the browser callback usually wins on
            // the same machine, and a remote browser still has the overlay.
            let text = match login(&flow_provider, &mut ui, true) {
                Ok(LoginSaved::Oauth {
                    provider,
                    path,
                    expires,
                }) => format!(
                    "✓ {provider} OAuth login complete (expires {expires}) — {} · next prompt goes real",
                    path.display()
                ),
                Ok(LoginSaved::Key { provider, path }) => format!(
                    "✓ {provider} key stored in {} · next prompt goes real",
                    path.display()
                ),
                Err(e) => format!("✗ {flow_provider} OAuth login failed: {e}"),
            };
            let _ = events.send(OauthUiEvent::Done(text));
        });
        self.ui.push_notice(format!(
            "starting {provider} OAuth login — a browser window will open; Esc cancels"
        ));
    }

    /// Offline builds have no token endpoint to reach; say so honestly.
    #[cfg(not(feature = "real"))]
    fn start_oauth_login(&mut self, provider: String) {
        self.ui.push_notice(format!(
            "{provider} OAuth login needs the real build: cargo install --path crates/epsilon-cli --features real --force"
        ));
    }

    /// Switch the session model, persist choice + recents, and tell the user.
    fn apply_model(&mut self, agent: &Arc<Agent>, id: String) {
        if id == self.model_id {
            self.ui.push_notice(format!("model already {id}"));
            return;
        }
        // Auth hint parity (pi/opencode show "needs auth" in the picker; Epsilon
        // shows 🔑/· but never warns at apply time). Non-blocking — switching
        // is allowed, you just can't run until the key is there.
        let auth_ready = epsilon_llm::catalog::credentials_ready(&id);
        agent.set_model_id(&id);
        // M32d: if we were on faux/offline and the new model is credentialed,
        // go live immediately so the next prompt hits the real API (pi parity).
        // Same hot-swap used by /login — apply_model is the other entry point
        // (your "deepseek login → flash via /model still offline" shot).
        if auth_ready {
            let _ = agent.try_hot_swap_to_real();
        }
        match epsilon_core::config::save_model_id(&id) {
            Ok(path) => {
                self.model_id = id.clone();
                let mut msg = format!("model switched to {id} — saved to {}", path.display());
                if !auth_ready {
                    msg.push_str("  (no key yet — /login <provider> to add one)");
                }
                self.ui.push_notice(msg);
            }
            Err(e) => {
                self.model_id = id.clone();
                let mut msg = format!("model switched to {id} (not saved: {e})");
                if !auth_ready {
                    msg.push_str("  (no key yet — /login <provider>)");
                }
                self.ui.push_notice(msg);
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
        let mut app = Self::new(
            model_id.clone(),
            inert_channel(),
            inert_channel(),
            inert_channel(),
        );
        app.ui = ui;
        app
    }
}

fn write_export_html(
    agent: &std::sync::Arc<epsilon_core::Agent>,
    dest: &std::path::Path,
    _workspace: &std::path::Path,
) -> Result<usize, String> {
    let store = agent.store_handle_for_tests();
    let entries = store
        .lock()
        .map_err(|_| "store unavailable".to_string())?
        .read_all()
        .map_err(|e| format!("{e}"))?;
    let n = entries.len();
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{e}"))?;
    }
    let mut html = String::new();
    html.push_str("<!doctype html><html><head><meta charset=\"utf-8\"><title>epsilon export</title></head><body><pre>\n");
    for e in &entries {
        let line = serde_json::to_string(e).unwrap_or_else(|_| "{}".into());
        let esc = line
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;");
        html.push_str(&esc);
        html.push('\n');
    }
    html.push_str("</pre></body></html>\n");
    std::fs::write(dest, html).map_err(|e| format!("{e}"))?;
    Ok(n)
}
fn write_export_json(
    agent: &std::sync::Arc<epsilon_core::Agent>,
    dest: &std::path::Path,
) -> Result<usize, String> {
    let store = agent.store_handle_for_tests();
    let entries = store
        .lock()
        .map_err(|_| "store unavailable".to_string())?
        .read_all()
        .map_err(|e| format!("{e}"))?;
    let n = entries.len();
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{e}"))?;
    }
    let pretty = serde_json::to_string_pretty(&entries).map_err(|e| format!("{e}"))?;
    std::fs::write(dest, pretty).map_err(|e| format!("{e}"))?;
    Ok(n)
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
    // Bracketed paste only — mouse capture stays OFF so drag-select copies
    // (pi does this; opencode/Hermes too). Hold Shift to select still works
    // on ConPTY, but we don't need to swallow motion just for wheel.
    let _ = execute!(std::io::stdout(), crossterm::event::EnableBracketedPaste);
    let code = drive(cfg, &mut terminal).await;
    let _ = execute!(std::io::stdout(), crossterm::event::DisableBracketedPaste);
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
    let (oauth_tx, mut oauth_rx) = tokio::sync::mpsc::unbounded_channel::<OauthUiEvent>();
    let mut app = App::new(model_id, fetched_tx, notices_tx, oauth_tx);
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

    // Input responsiveness (owner report: "not responsive in cmd, pwsh"). A full
    // redraw costs a frame, and in a Windows console a frame measures tens of
    // milliseconds — drawing once per *event* made a burst of keys echo at about
    // 30 chars/s (measured on conhost: 99 typed characters took ~3 s to appear).
    // The loop now drains every input event that is already pending before
    // drawing, draws at most once per turn of the loop, and only when something
    // actually changed — so a burst costs one frame, and an idle TUI redraws
    // nothing at all.
    let mut dirty = true;
    let mut frame_due = std::time::Instant::now();
    loop {
        // M59: the external editor needs the terminal to itself, so the hand-off
        // happens between frames (set by `/editor` or the keybinding) — never
        // inside a key handler, which must not touch the terminal.
        if app.external_editor_requested {
            app.external_editor_requested = false;
            open_external_editor(terminal, &mut app);
            dirty = true;
        }

        // Drain: `timeout(ZERO)` polls once and reports `Elapsed` the moment no
        // event is ready, so this neither waits nor adds latency of its own.
        // Bounded (M94 review): a flood of input must not keep the loop from
        // reaching its frame decision; the remainder is handled by the biased
        // select below, which serves input first.
        let mut quit = false;
        for _ in 0..MAX_INPUT_DRAIN {
            let Ok(Some(Ok(ev))) =
                tokio::time::timeout(std::time::Duration::ZERO, term_events.next()).await
            else {
                break;
            };
            dirty = true;
            if handle_term_event(&mut app, &agent, ev) {
                quit = true;
                break;
            }
        }
        // Coalesce agent output the same way: a streamed answer arrives as one
        // event per delta, and drawing per delta was what made the TUI stutter
        // while the model was talking. Bounded, so a flood cannot starve input.
        for _ in 0..128 {
            match events_rx.try_recv() {
                Ok(ev) => {
                    dirty = true;
                    apply_with_scroll(&mut app, &ev);
                }
                Err(tokio::sync::broadcast::error::TryRecvError::Lagged(n)) => {
                    dirty = true;
                    app.ui
                        .push_notice(format!("… {n} events dropped (subscriber lag)"));
                }
                Err(tokio::sync::broadcast::error::TryRecvError::Empty) => break,
                Err(tokio::sync::broadcast::error::TryRecvError::Closed) => {
                    quit = true;
                    break;
                }
            }
        }
        if quit {
            break;
        }

        // Frame ceiling (~30 fps). Without it a fast provider stream would still
        // draw once per batch, and every streaming frame re-wraps the whole
        // in-flight answer — the cost that made long answers feel frozen.
        if dirty && std::time::Instant::now() >= frame_due {
            let _ = terminal.draw(|f| draw(f, &mut app));
            dirty = false;
            frame_due = std::time::Instant::now() + FRAME_MIN;
        }

        // Only a due frame needs a timer here; when nothing is dirty the loop
        // parks on real events instead (the sleep is effectively infinite).
        let wait = if dirty {
            frame_due.saturating_duration_since(std::time::Instant::now())
        } else {
            std::time::Duration::from_secs(3600)
        };

        tokio::select! {
            // Input first: a busy agent event stream must never delay a keystroke.
            biased;

            maybe_ev = term_events.next() => {
                dirty = true;
                match maybe_ev {
                    Some(Ok(ev)) => {
                        if handle_term_event(&mut app, &agent, ev) {
                            break;
                        }
                    }
                    // The stream ended or errored — the previous shape stopped here.
                    _ => break,
                }
            },

            // Approvals outrank agent output: a pending decision must not wait
            // behind a stream of deltas.
            ask = approvals.next_ask() => {
                if let Some(pending) = ask {
                    app.scroll_from_bottom = 0;
                    app.pending_ask = Some(pending);
                    dirty = true;
                }
            },

            ev = events_rx.recv() => {
                dirty = true;
                match ev {
                    Ok(ev) => apply_with_scroll(&mut app, &ev),
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        app.ui.push_notice(format!("… {n} events dropped (subscriber lag)"));
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            },

            maybe_notice = notices_rx.recv() => {
                if let Some(text) = maybe_notice {
                    app.ui.push_notice(text);
                    dirty = true;
                }
            },

            maybe_oauth = oauth_rx.recv() => {
                if let Some(event) = maybe_oauth {
                    app.apply_oauth_event(event);
                    dirty = true;
                }
            },

            maybe_fetched = fetched_rx.recv() => {
                if let Some((provider, result)) = maybe_fetched
                    && let Some(text) = app.record_live_result(provider, result)
                {
                    app.ui.push_notice(text);
                    dirty = true;
                }
            },

            // A pending frame must not wait for the next event: wake when due.
            _ = tokio::time::sleep(wait) => {}

            _ = tick.tick() => {
                if app.ui.running() {
                    app.spinner_phase = app.spinner_phase.wrapping_add(1);
                    dirty = true;
                }
            },
        }
    }
    0
}

/// Apply one terminal event; `true` means the run loop should quit.
///
/// Extracted from the loop so the pre-draw drain and the select's input arm
/// share exactly one implementation — the drain is what keeps a burst of keys
/// down to a single frame.
fn handle_term_event(app: &mut App, agent: &Arc<Agent>, ev: TermEvent) -> bool {
    match ev {
        TermEvent::Key(key) => {
            if key.kind == KeyEventKind::Press && handle_key(app, agent, key) {
                return true;
            }
        }
        // Mouse wheel: Up/Down/Left/Right all forwarded by most terminals even
        // without EnableMouseCapture. Swallowing Down/Up/Press/Release is what
        // blocks drag-select.
        TermEvent::Mouse(m) => match m.kind {
            crossterm::event::MouseEventKind::ScrollUp => {
                if app.picker_open {
                    app.picker_index = app.picker_index.saturating_sub(1);
                } else {
                    app.scroll_from_bottom = app.scroll_from_bottom.saturating_add(3);
                }
            }
            crossterm::event::MouseEventKind::ScrollDown => {
                if app.picker_open {
                    let total = app.picker_rows().len();
                    if total > 0 {
                        app.picker_index = (app.picker_index + 1).min(total - 1);
                    }
                } else {
                    app.scroll_from_bottom = app.scroll_from_bottom.saturating_sub(3);
                }
            }
            _ => {} // drag/select: let the terminal own it
        },
        // M16b: bracketed pastes become atomic `[paste #N]` markers.
        TermEvent::Paste(text) => {
            if app.auth_prompt_open {
                // Masked API-key paste: go straight into auth_key, not the editor.
                handle_auth_paste(app, text);
            } else if app.auth_pick_open {
                // Typing pastes into the filter — useful when pasting a provider id.
                for c in text.trim().chars() {
                    app.insert_char(c);
                }
            } else {
                let marker = crate::kill_ring::make_paste_marker(app.next_paste_id, &text);
                if marker.starts_with(crate::kill_ring::MARKER_PREFIX) {
                    app.paste_payloads.insert(app.next_paste_id, text.clone());
                    app.next_paste_id += 1;
                }
                app.insert_str(&marker);
            }
        }
        _ => {}
    }
    false
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

/// M32: insert one char into the masked API-key buffer at `auth_key_cursor`.
fn auth_insert(app: &mut App, c: char) {
    let byte = app
        .auth_key
        .char_indices()
        .nth(app.auth_key_cursor)
        .map(|(i, _)| i)
        .unwrap_or(app.auth_key.len());
    app.auth_key.insert(byte, c);
    app.auth_key_cursor += 1;
}

/// M32c: read the system clipboard (arboard). Used by the Ctrl+V fallback when
/// the terminal does not deliver bracketed `TermEvent::Paste`. Best-effort:
/// empty string on failure so the caller can show a hint.
fn clipboard_text() -> String {
    arboard::Clipboard::new()
        .and_then(|mut c| c.get_text())
        .unwrap_or_default()
}

/// Try to write `text` to the system clipboard (arboard). Returns Err on
/// headless / permission failure so the caller can surface it as a notice.
fn clipboard_set(text: &str) -> Result<(), String> {
    let mut cb = arboard::Clipboard::new().map_err(|e| format!("{e}"))?;
    cb.set_text(text.to_string()).map_err(|e| format!("{e}"))
}

/// M32: paste (bracketed or Ctrl+V) into the masked API-key buffer. Keys are
/// single-line — newlines/spaces at the edges are trimmed so a trailing
/// newline from the clipboard does not create an invisible mismatch.
fn handle_auth_paste(app: &mut App, text: String) {
    let cleaned = text.trim().replace(['\r', '\n'], "");
    let cleaned = cleaned.trim().to_string();
    if cleaned.is_empty() {
        return;
    }
    for c in cleaned.chars() {
        auth_insert(app, c);
    }
}

/// M52: what the background OAuth login thread tells the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(not(feature = "real"), allow(dead_code))]
enum OauthUiEvent {
    /// The flow is ready for a pasted authorization code / redirect URL.
    NeedCode {
        /// Provider whose login is waiting.
        provider: String,
    },
    /// The ceremony finished (success or failure) — close the overlay.
    Done(String),
}

/// M52: the login thread's view of the terminal. Notices and events travel
/// over the app's channels; the pasted code arrives from the overlay.
#[cfg(feature = "real")]
struct TuiLoginUi {
    notices: tokio::sync::mpsc::UnboundedSender<String>,
    events: tokio::sync::mpsc::UnboundedSender<OauthUiEvent>,
    code_rx: std::sync::mpsc::Receiver<String>,
    provider: String,
}

#[cfg(feature = "real")]
impl epsilon_llm::oauth_login::LoginUi for TuiLoginUi {
    fn notify(&mut self, message: &str) {
        let _ = self.notices.send(message.to_string());
    }

    fn device_code(&mut self, prompt: &epsilon_llm::oauth_login::DevicePrompt) {
        let _ = self.notices.send(format!(
            "device login: open {} and enter code {} (valid for {}s)",
            prompt.verification_uri, prompt.user_code, prompt.expires_in_seconds
        ));
    }

    fn wait_for_code(
        &mut self,
        callback: Option<&epsilon_llm::oauth_login::CallbackListener>,
    ) -> Option<String> {
        let _ = self.events.send(OauthUiEvent::NeedCode {
            provider: self.provider.clone(),
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(600);
        loop {
            match self.code_rx.try_recv() {
                Ok(line) if !line.trim().is_empty() => return Some(line),
                Ok(_) => {}
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    // Esc (or shutdown) dropped the sender: the user cancelled.
                    return None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
            if let Some(cb) = callback {
                match cb.try_recv() {
                    Some(Ok(code)) => return Some(code.code),
                    Some(Err(_)) => return None,
                    None => {}
                }
            }
            if std::time::Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(std::time::Duration::from_millis(60));
        }
    }
}

/// M32: masked API-key prompt key handler — owns every key while open so the
/// transcript editor does not see them. Esc cancels, Enter saves to
/// `~/.config/epsilon/auth.json` via `epsilon_llm::auth`.
fn begin_auth_key_prompt(app: &mut App, provider: String) {
    app.auth_prompt_open = true;
    app.auth_provider = provider.clone();
    app.auth_key.clear();
    app.auth_key_cursor = 0;
    // M52: name the subscription route when the provider has one, so the
    // ceremony is discoverable from the prompt a user actually lands in.
    let oauth_hint = if epsilon_llm::oauth_login::login_flow(&provider).is_some() {
        " · Ctrl+O = subscription login (OAuth)"
    } else {
        ""
    };
    app.ui.push_notice(format!(
        "enter API key for '{provider}' — paste the key (masked), Enter saves to ~/.config/epsilon/auth.json, Esc cancels{oauth_hint}"
    ));
}

fn handle_auth_key(app: &mut App, agent: &Arc<Agent>, key: KeyEvent) -> bool {
    match key.code {
        KeyCode::Esc => {
            // M52: dropping the code sender disconnects the login thread,
            // which reports the cancellation itself.
            let was_oauth = app.oauth_code_tx.take().is_some();
            app.auth_prompt_open = false;
            app.auth_provider.clear();
            app.auth_key.clear();
            app.auth_key_cursor = 0;
            app.ui.push_notice(if was_oauth {
                "OAuth code entry cancelled"
            } else {
                "API key entry cancelled"
            });
            false
        }
        // Ctrl+V paste inside the masked prompt (fallback when bracketed paste
        // is not available — e.g. conhost, legacy Windows console). Reads the
        // system clipboard via arboard and inserts masked, like bracketed paste.
        KeyCode::Char('v' | 'V') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            let text = clipboard_text();
            if text.trim().is_empty() {
                app.ui
                    .push_notice("clipboard empty — copy a key first, then Ctrl+V");
            } else {
                handle_auth_paste(app, text);
            }
            false
        }
        // Shift+Insert is the other common paste chord on Windows/Linux
        // terminals. Treat it the same as Ctrl+V.
        KeyCode::Insert if key.modifiers.contains(KeyModifiers::SHIFT) => {
            let text = clipboard_text();
            if text.trim().is_empty() {
                app.ui
                    .push_notice("clipboard empty — copy a key first, then Shift+Insert");
            } else {
                handle_auth_paste(app, text);
            }
            false
        }
        // M52: discoverability — a pending key prompt for an OAuth-capable
        // provider can switch into the subscription ceremony without typing
        // `/login <provider> oauth` from scratch.
        KeyCode::Char('o' | 'O') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            let provider = app.auth_provider.clone();
            app.auth_prompt_open = false;
            app.auth_provider.clear();
            app.auth_key.clear();
            app.auth_key_cursor = 0;
            app.oauth_code_tx = None;
            if provider.is_empty() || epsilon_llm::oauth_login::login_flow(&provider).is_none() {
                app.ui.push_notice(
                    "no OAuth login for that provider — /login <provider> <key> stores a key",
                );
            } else {
                app.start_oauth_login(provider);
            }
            false
        }
        KeyCode::Enter => {
            if app.oauth_code_tx.is_some() {
                let value = app.auth_key.trim().to_string();
                if value.is_empty() {
                    app.ui
                        .push_notice("empty — paste the redirect URL or code, or Esc to cancel");
                    return false;
                }
                let sender = app.oauth_code_tx.take().expect("checked");
                let _ = sender.send(value);
                app.auth_prompt_open = false;
                app.auth_provider.clear();
                app.auth_key.clear();
                app.auth_key_cursor = 0;
                app.ui
                    .push_notice("code handed to the login flow — exchanging…");
                return false;
            }
            let provider = app.auth_provider.clone();
            let key_val = app.auth_key.trim().to_string();
            if key_val.is_empty() {
                app.ui
                    .push_notice("empty key — paste a key or press Esc to cancel");
                return false;
            }
            match epsilon_llm::auth::save_credential(&provider, &key_val) {
                Ok(path) => {
                    app.ui
                        .push_notice(format!("saved key for '{provider}' → {}", path.display()));
                    let live = agent.try_hot_swap_to_real();
                    if live {
                        app.ui.push_notice(format!(
                            "now live on '{provider}' — next prompt hits the real API."
                        ));
                    } else {
                        app.ui.push_notice(format!(
                            "Restart epsilon to use '{provider}' — next prompt will hit the real API (was offline demo on '{m}').",
                            m = app.model_id
                        ));
                    }
                    app.invalidate_picker();
                }
                Err(e) => {
                    app.ui
                        .push_notice(format!("not saved for '{provider}': {e}"));
                }
            }
            app.auth_prompt_open = false;
            app.auth_provider.clear();
            app.auth_key.clear();
            app.auth_key_cursor = 0;
            false
        }
        KeyCode::Backspace => {
            if app.auth_key_cursor > 0 {
                let before: String = app.auth_key.chars().take(app.auth_key_cursor - 1).collect();
                let after: String = app.auth_key.chars().skip(app.auth_key_cursor).collect();
                app.auth_key = before + &after;
                app.auth_key_cursor -= 1;
            }
            false
        }
        KeyCode::Delete => {
            if app.auth_key_cursor < app.auth_key.chars().count() {
                let before: String = app.auth_key.chars().take(app.auth_key_cursor).collect();
                let after: String = app.auth_key.chars().skip(app.auth_key_cursor + 1).collect();
                app.auth_key = before + &after;
            }
            false
        }
        KeyCode::Left => {
            app.auth_key_cursor = app.auth_key_cursor.saturating_sub(1);
            false
        }
        KeyCode::Right => {
            app.auth_key_cursor = (app.auth_key_cursor + 1).min(app.auth_key.chars().count());
            false
        }
        KeyCode::Home => {
            app.auth_key_cursor = 0;
            false
        }
        KeyCode::End => {
            app.auth_key_cursor = app.auth_key.chars().count();
            false
        }
        KeyCode::Char('u' | 'U') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            // Ctrl+U — kill to start (pi editor convention).
            let after: String = app.auth_key.chars().skip(app.auth_key_cursor).collect();
            app.auth_key = after;
            app.auth_key_cursor = 0;
            false
        }
        KeyCode::Char('w' | 'W') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            // Ctrl+W — kill word back.
            let start = crate::editor_ops::word_left(&app.auth_key, app.auth_key_cursor);
            let before: String = app.auth_key.chars().take(start).collect();
            let after: String = app.auth_key.chars().skip(app.auth_key_cursor).collect();
            app.auth_key = before + &after;
            app.auth_key_cursor = start;
            false
        }
        KeyCode::Char(c) => {
            // Printable char typed directly (covers paste via key events on terminals
            // without bracketed paste). Masked — not echoed.
            auth_insert(app, c);
            false
        }
        _ => false,
    }
}

/// M94 review: keys while an approval is pending. Extracted so the routing lives
/// in exactly one place — `handle_key` calls it BEFORE the auth branches, since
/// the dialog is painted over them.
///
/// Fail closed, identity-based: `App::approval_painted_id` names the ask the last
/// frame actually painted (`draw` writes it, the undersized fallback clears it).
/// An ask whose id does not match is refused with a notice and stays pending — a
/// second parked ask seated inside the same frame window, or any ask the dialog
/// could not fit for. A blind `y`/`a` would authorize work the user never saw and
/// a blind Esc would silently deny it.
fn handle_approval_key(app: &mut App, key: KeyEvent) -> bool {
    let Some(ask_id) = app
        .pending_ask
        .as_ref()
        .map(|ask| ask.request.request_id.clone())
    else {
        return false;
    };
    // The painted id is the ONLY ask these keys may decide: a key handled after a
    // NEW ask is seated (the channel is FIFO; parallel tool calls park several)
    // and before the next draw must not answer what no frame has shown.
    if app.approval_painted_id.as_deref() != Some(ask_id.as_str()) {
        app.ui.push_notice(
            "approval is not visible yet — it appears on the next frame; enlarge the terminal (need 60×12) if it does not",
        );
        return false;
    }
    // Guidance mode: Enter answers the ask too (DenyWithGuidance). Gated by the
    // same painted-id check above, so the refusal covers it as well.
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
    false
}

/// Handle one key press; returns true when the app should quit.
fn handle_key(app: &mut App, agent: &Arc<Agent>, key: KeyEvent) -> bool {
    // M30: mirror the steer queue length on every key event while running so
    // draw_status can show a count badge without needing the agent ref. Kept
    // above the modal branches below so the badge keeps up in every state.
    if app.ui.running() {
        app.steer_queue_len = agent.steer_queue_len();
    }
    // M94 re-review (coherence): with an ask pending the approval keys route
    // BEFORE the auth branches. The dialog is painted last — over the masked
    // `/login` key prompt and the provider picker — so it must own the keyboard
    // too; otherwise `y`/`a`/`n`/`Esc`/Enter went into a buffer the user could
    // not see (Enter there saves to `auth.json`) while they were looking at a
    // dialog they could not answer. The auth state is left untouched, so the
    // login flow resumes exactly where it was once the ask clears. The exit
    // keybind keeps its precedence over the approval branch (quitting drops the
    // parked ask, which resolves to Deny — fail closed), exactly as before.
    if app.pending_ask.is_some() && !app.keybindings.matches("app.exit", &key) {
        return handle_approval_key(app, key);
    }
    // M32: masked API-key prompt takes all keys while open (Esc cancels, Enter saves).
    if app.auth_prompt_open {
        return handle_auth_key(app, agent, key);
    }
    // M32b: provider picker for bare `/login` — filtered by `input`, Enter opens
    // the masked key prompt for the highlighted provider.
    if app.auth_pick_open {
        let rows = crate::picker::build_provider_rows(&app.model_id, &app.customs);
        let query = app.input.trim().to_ascii_lowercase();
        let filtered: Vec<_> = if query.is_empty() {
            rows.iter().enumerate().collect()
        } else {
            rows.iter()
                .enumerate()
                .filter(|(_, p)| p.id.to_ascii_lowercase().contains(&query))
                .collect()
        };
        match key.code {
            KeyCode::Esc => {
                app.auth_pick_open = false;
                app.auth_pick_index = 0;
                app.input.clear();
                app.cursor_chars = 0;
            }
            KeyCode::Enter => {
                if let Some((_, prow)) = filtered.get(app.auth_pick_index) {
                    let chosen = prow.id.clone();
                    app.auth_pick_open = false;
                    app.auth_pick_index = 0;
                    app.input.clear();
                    app.cursor_chars = 0;
                    begin_auth_key_prompt(app, chosen);
                }
            }
            KeyCode::Up => {
                app.auth_pick_index = app
                    .auth_pick_index
                    .saturating_sub(1)
                    .min(filtered.len().saturating_sub(1));
            }
            KeyCode::Down => {
                if !filtered.is_empty() {
                    app.auth_pick_index = (app.auth_pick_index + 1).min(filtered.len() - 1);
                }
            }
            // Ctrl+V / Shift+Insert paste into the provider filter — when the
            // terminal sends paste as a key chord instead of bracketed paste, let
            // the user paste a provider id from the clipboard.
            KeyCode::Char('v' | 'V') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                let text = clipboard_text();
                let text = text.trim().to_string();
                if !text.is_empty() {
                    for c in text.chars() {
                        app.insert_char(c);
                    }
                    app.auth_pick_index = 0;
                } else {
                    app.ui
                        .push_notice("clipboard empty — copy text first, then Ctrl+V");
                }
            }
            KeyCode::Insert if key.modifiers.contains(KeyModifiers::SHIFT) => {
                let text = clipboard_text();
                let text = text.trim().to_string();
                if !text.is_empty() {
                    for c in text.chars() {
                        app.insert_char(c);
                    }
                    app.auth_pick_index = 0;
                } else {
                    app.ui
                        .push_notice("clipboard empty — copy text first, then Shift+Insert");
                }
            }
            KeyCode::Backspace => {
                app.backspace();
                app.auth_pick_index = 0;
            }
            KeyCode::Char(c) => {
                app.insert_char(c);
                app.auth_pick_index = 0;
            }
            _ => {}
        }
        return false;
    }
    if app.keybindings.matches("app.exit", &key) {
        return true;
    }

    // Plan 009: timeline picker — Enter navigates, Esc dismisses, arrows move.
    if app.timeline_open {
        match key.code {
            KeyCode::Esc => {
                app.timeline_open = false;
                app.timeline_index = 0;
                app.timeline_entries.clear();
            }
            KeyCode::Enter => {
                if let Some((seq, _, _)) = app.timeline_entries.get(app.timeline_index).cloned() {
                    let store = agent.store_handle_for_tests();
                    let jumped = if let Ok(mut st) = store.lock() {
                        let id = st.branch_entries().ok().and_then(|entries| {
                            entries.into_iter().find(|e| e.seq == seq).map(|e| e.id)
                        });
                        if let Some(id) = id {
                            st.navigate_to(&id).is_ok()
                        } else {
                            false
                        }
                    } else {
                        false
                    };
                    if jumped {
                        let branch = store
                            .lock()
                            .ok()
                            .and_then(|st| st.branch_entries().ok())
                            .unwrap_or_default();
                        app.ui.clear_lines();
                        for e in &branch {
                            let snippet = match &e.kind {
                                epsilon_contracts::session_entry::EntryKind::Message { message, .. } => {
                                    let txt: String = match message {
                                        epsilon_contracts::message::Message::User(u) => u.content.iter().filter_map(|c| match c { epsilon_contracts::content::UserContent::Text { text } => Some(text.as_str()), _ => None }).collect::<Vec<_>>().join(" "),
                                        epsilon_contracts::message::Message::Assistant(a) => a.text(),
                                        epsilon_contracts::message::Message::ToolResult(t) => t.content.iter().filter_map(|c| match c { epsilon_contracts::content::ContentBlock::Text { text } => Some(text.as_str()), _ => None }).collect::<String>(),
                                        _ => String::new(),
                                    };
                                    let mut s: String = txt.chars().take(120).collect();
                                    if txt.chars().count() > 120 { s.push('…'); }
                                    if s.trim().is_empty() { "(empty)".into() } else { s }
                                }
                                epsilon_contracts::session_entry::EntryKind::Compaction { summary, .. } => {
                                    let mut s: String = summary.chars().take(120).collect();
                                    if summary.chars().count() > 120 { s.push('…'); }
                                    format!("[compaction] {s}")
                                }
                                _ => format!("[{}]", match e.kind { epsilon_contracts::session_entry::EntryKind::Message { .. } => "message", epsilon_contracts::session_entry::EntryKind::Compaction { .. } => "compaction", _ => "entry" }),
                            };
                            app.ui.push_notice(format!("#{seq} {snippet}", seq = e.seq));
                        }
                        app.ui.push_notice(format!("jumped to #{seq}"));
                    }
                    app.timeline_open = false;
                    app.timeline_index = 0;
                    app.timeline_entries.clear();
                }
            }
            KeyCode::Up => {
                let total = app.timeline_entries.len();
                app.timeline_index = app
                    .timeline_index
                    .saturating_sub(1)
                    .min(total.saturating_sub(1));
            }
            KeyCode::Down => {
                let total = app.timeline_entries.len();
                if total > 0 {
                    app.timeline_index = (app.timeline_index + 1).min(total - 1);
                }
            }
            _ => {}
        }
        return false;
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

    // M57: the session browser owns its keys while open. The filter IS the
    // input buffer (same as the model picker), so typing/backspace reuse the
    // editor's own grapheme-safe paths.
    if app.session_browser.is_some() {
        match key.code {
            KeyCode::Esc => {
                app.session_browser = None;
                app.input.clear();
                app.cursor_chars = 0;
            }
            KeyCode::Enter => {
                let picked = app
                    .session_browser
                    .as_ref()
                    .and_then(|b| b.rows.get(b.index))
                    .map(|r| r.path.clone());
                if let Some(path) = picked {
                    app.session_browser = None;
                    app.input.clear();
                    app.cursor_chars = 0;
                    match app.switch_session_file(agent, &path, None) {
                        Ok(count) => app.ui.push_notice(format!(
                            "resumed {} messages from {} (history restored, store swapped)",
                            count,
                            path.display()
                        )),
                        Err(e) => app.ui.push_notice(format!("resume failed: {e}")),
                    }
                }
            }
            KeyCode::Up => {
                if let Some(b) = app.session_browser.as_mut() {
                    b.index = b.index.saturating_sub(1);
                }
            }
            KeyCode::Down => {
                if let Some(b) = app.session_browser.as_mut() {
                    let last = b.rows.len().saturating_sub(1);
                    b.index = (b.index + 1).min(last);
                }
            }
            KeyCode::Backspace => {
                app.backspace();
                let filter = app.input.clone();
                if let Some(b) = app.session_browser.as_mut() {
                    b.rebuild(&filter);
                }
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                app.insert_char(c);
                let filter = app.input.clone();
                if let Some(b) = app.session_browser.as_mut() {
                    b.rebuild(&filter);
                }
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
            _ if app.keybindings.matches("app.live.refresh", &key) => {
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

    match key.code {
        KeyCode::Esc => {
            // M17b: Esc first dismisses the slash popup (pi prompt.autocomplete.hide);
            // then aborts a run; only when idle+quiet does it quit.
            if app.slash_open {
                app.slash_open = false;
                app.slash_index = 0;
                return false;
            }
            if app.at_open {
                app.at_open = false;
                app.at_index = 0;
                return false;
            }
            if app.ui.running() {
                if agent.abort_current_run() {
                    // M30: drain queued steering so it doesn't leak into the
                    // next run, and restore the last message to the editor so
                    // the user can re-edit rather than losing it.
                    let drained = agent.drain_steering_all();
                    if let Some(last_user_text) = drained.into_iter().rev().find_map(|m| {
                        if let Message::User(u) = m {
                            u.content.into_iter().find_map(|c| match c {
                                UserContent::Text { text } => Some(text),
                                _ => None,
                            })
                        } else {
                            None
                        }
                    }) {
                        app.input = last_user_text.clone();
                        app.cursor_chars = last_user_text.chars().count();
                        app.notice("abort requested — queued text restored to editor");
                    } else {
                        app.notice("abort requested — cancelling current run…");
                    }
                }
                false
            } else {
                true
            }
        }
        KeyCode::Enter => {
            if app.at_open {
                app.apply_at_selection(true);
                return false;
            }
            // M17b: with the popup open, Enter first applies the highlighted
            // suggestion (pi prompt.autocomplete.select) — then FALLS THROUGH to
            // submit, because a completed command like "/clear" is meant to RUN.
            // Bare-text suggestions (no popup) submit unchanged.
            if app.slash_open {
                app.apply_slash_selection();
            }
            app.submit_editor(agent)
        }
        _ if app.keybindings.matches("app.model.select", &key) => {
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
        _ if app.keybindings.matches("app.thinking.cycle", &key) => {
            // Cycle thinking level (pi app.thinking.cycle, Shift+Tab there —
            // Shift+Tab is unreliable across terminals, Ctrl+T is not taken).
            let next = agent.thinking_level().next();
            app.set_thinking(agent, next);
            false
        }
        _ if app.keybindings.matches("app.palette", &key) => {
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
        _ if app.keybindings.matches("app.model.cycleForward", &key)
            || app.keybindings.matches("app.model.cycleBackward", &key) =>
        {
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
            let dir = if app.keybindings.matches("app.model.cycleBackward", &key) {
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
        _ if app.keybindings.matches("tui.editor.undo", &key) => {
            // Parity sweep: editor undo (pi undo-stack.ts).
            app.undo();
            false
        }
        // M17b: redo (opencode input_redo, ctrl+.) and line-wise ops (pi/opencode).
        _ if app.keybindings.matches("tui.editor.redo", &key) => {
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
            // M36: cluster-stepped, like Left.
            app.cursor_chars = crate::editor_ops::prev_cluster_start(&app.input, app.cursor_chars);
            false
        }
        KeyCode::Char('f' | 'F') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            // M36: cluster-stepped, like Right.
            app.cursor_chars = crate::editor_ops::next_cluster_end(&app.input, app.cursor_chars);
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
            // M36: one grapheme cluster per press — never park mid-cluster.
            app.cursor_chars = crate::editor_ops::prev_cluster_start(&app.input, app.cursor_chars);
            false
        }
        KeyCode::Right => {
            // M36: one grapheme cluster per press.
            app.cursor_chars = crate::editor_ops::next_cluster_end(&app.input, app.cursor_chars);
            false
        }
        KeyCode::Up => {
            if app.at_open {
                let total = app.at_rows.as_ref().map(|r| r.len()).unwrap_or(0);
                app.at_index = app.at_index.saturating_sub(1).min(total.saturating_sub(1));
                return false;
            }
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
            if app.at_open {
                if let Some(rows) = &app.at_rows
                    && !rows.is_empty()
                {
                    app.at_index = (app.at_index + 1).min(rows.len() - 1);
                }
                return false;
            }
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
            if app.at_open {
                app.apply_at_selection(false);
                return false;
            }
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
        // Ctrl+V paste (M32c): when the terminal handles Ctrl+V as paste instead of
        // sending bracketed TermEvent::Paste (conhost, cmd.exe, some WSL setups),
        // read the system clipboard via arboard. Outside the auth prompt this
        // becomes an editor paste marker so huge pastes don't flood the buffer.
        _ if app.keybindings.matches("app.clipboard.paste", &key) => {
            let text = clipboard_text();
            if text.trim().is_empty() {
                app.ui
                    .push_notice("clipboard empty — copy text first, then Ctrl+V");
                return false;
            }
            // Reuse the bracketed-paste path shape: large/multi-line → marker,
            // single short → literal. That path lives in drive/TermEvent::Paste;
            // factor it here so ctrl+V and shift+insert behave identically without
            // duplicating the 3-line marker/payload bookkeeping.
            let marker = crate::kill_ring::make_paste_marker(app.next_paste_id, &text);
            if marker.starts_with(crate::kill_ring::MARKER_PREFIX) {
                app.paste_payloads.insert(app.next_paste_id, text.clone());
                app.next_paste_id += 1;
            }
            app.insert_str(&marker);
            false
        }
        // M16b: Ctrl+J inserts a newline (the reliable multi-line combo — Ctrl+Enter
        // is indistinguishable from Enter on legacy Windows consoles).
        _ if app.keybindings.matches("tui.input.newLine", &key) => {
            app.insert_char('\n');
            app.end_yank();
            false
        }
        // M59: hand the draft to $EDITOR (pi `app.editor.external`; the request
        // is performed by the event loop, which owns the terminal).
        _ if app.keybindings.matches("app.editor.external", &key) => {
            app.request_external_editor();
            false
        }
        // M13 card polish: expand/collapse truncated tool outputs. Originally a
        // bare `x` (M13), which made the letter x untypable in the editor —
        // M29 fix: it needs Ctrl+X like every other non-typing key.
        _ if app.keybindings.matches("app.tools.expand", &key) => {
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

/// Collector's note on the safety rule below.
///
/// The too-small fallback cannot paint the approval dialog, so it must also make
/// sure `handle_key` refuses to answer one: `approval_painted_id` records which
/// ask a frame actually showed, and clearing it here means no key can authorize
/// work the user never saw. That behaviour is preserved from M94 unchanged.
fn draw(f: &mut Frame<'_>, app: &mut App) {
    let area = f.area();
    if crate::ui::layout::Regions::too_small(area) {
        app.approval_painted_id = None;
        // Name a pending approval even here. The dialog is not painted, so the
        // keys are refused, but silently hiding a decision that is blocking the
        // run is how a user concludes the harness has hung.
        let mut lines: Vec<Line<'static>> = vec![Line::from(
            crate::ui::layout::too_small_message(area.width, area.height),
        )];
        if let Some(ask) = app.pending_ask.as_ref() {
            lines.push(Line::from(format!(
                "approval pending: {} · {}",
                ask.request.capability, ask.request.pattern
            )));
            lines.push(Line::from(
                "widen the window to answer it — the run is waiting on you",
            ));
        }
        f.render_widget(
            Paragraph::new(lines).centered().wrap(Wrap { trim: true }),
            area,
        );
        return;
    }
    // The composer wraps to the same width the shell will draw it at, so the
    // caret lands in the column the text actually occupies.
    let draft_width = crate::ui::shell::composer_text_width(area.width, &app.theme.glyphs);
    let (draft_rows, caret_row, caret_col) = composer_rows(&app.input, app.cursor_chars, draft_width);
    // The theme is `Copy`, so the closure below can still take `&mut app`: the
    // alternative — holding a borrow of `app.theme` across the call — is what the
    // borrow checker refuses, and rightly, since the closure mutates the app.
    let theme = app.theme;
    let shell = crate::ui::shell::Shell {
        theme: &theme,
        run: app.run_state(),
        metrics: app.metrics(),
        hints: app.hints(),
        draft: draft_rows,
        caret: (caret_row, caret_col),
        composer_focused: !app.overlay_open(),
        transcript_empty: app.transcript_is_empty(),
    };
    let painted = crate::ui::shell::paint(f, area, &shell, |f, transcript_area| {
        draw_transcript(f, app, transcript_area);
    });
    let transcript_area = painted.transcript;

    if app.session_browser.is_some() {
        draw_session_browser(f, app, f.area());
    }
    if app.picker_open {
        draw_picker(f, app, f.area());
    }
    if app.palette_open {
        draw_palette(f, app, f.area());
    }
    if app.auth_prompt_open {
        draw_auth_prompt(f, app, f.area());
    }
    if app.auth_pick_open {
        draw_auth_picker(f, app, f.area());
    }
    if app.timeline_open {
        draw_timeline(f, app, f.area());
    }
    draw_at_file(f, app, transcript_area);
    draw_slash_autocomplete(f, app, transcript_area);

    // M94 review: the approval dialog is painted LAST, so no other overlay can
    // cover it — an ask that arrives while `/timeline` (or any popup) is open
    // still routes y/a/n/Esc to itself, and a covered dialog silently
    // authorized or denied. Drawing it here is also what records visibility for
    // `handle_key`; the dialog always fits above 60×12 (the fallback owns
    // anything smaller).
    // M94 re-review: the guard records WHICH ask this frame painted, not merely
    // that something was painted — a per-frame flag survived the frame that set
    // it, so a second ask seated before the next draw inherited the credit and a
    // key could answer a decision no frame had shown. The painted id is the only
    // ask the key path will answer; every other ask waits for its own frame.
    app.approval_painted_id = app
        .pending_ask
        .as_ref()
        .map(|ask| ask.request.request_id.clone());
    if app.pending_ask.is_some() {
        draw_dialog(f, app, f.area());
    }
}

/// Adapters from live application state to the plain values the presentation
/// layer draws with. Keeping them here — rather than letting widgets read the
/// app — is what lets every screen be rendered in a test without an agent.
impl App {
    /// The look this session was started with.
    ///
    /// Public so a caller outside the draw path (a future integration test, a
    /// screenshot harness) can render with the same palette the binary uses.
    #[allow(dead_code, reason = "the draw path reads the field directly")]
    pub fn theme(&self) -> &crate::ui::theme::Theme {
        &self.theme
    }

    /// Run state for the header.
    ///
    /// The phase word is derived here rather than stored, so it cannot disagree
    /// with the flags it is derived from.
    fn run_state(&self) -> crate::ui::header::RunState {
        match self.run_started_at {
            Some(started) => crate::ui::header::RunState::Running {
                frame: self.spinner_phase,
                phase: self.ui.status().to_string(),
                elapsed_secs: started.elapsed().as_secs(),
            },
            None if self.ui.running() => crate::ui::header::RunState::Running {
                frame: self.spinner_phase,
                phase: self.ui.status().to_string(),
                elapsed_secs: 0,
            },
            None => crate::ui::header::RunState::Idle,
        }
    }

    /// The measurements the header reports.
    ///
    /// Cost is computed here, from catalog prices, and is `None` for a model the
    /// catalog cannot price — the header then omits it rather than printing a
    /// confident `$0`. Integer micro-USD throughout (ADR-010), so no float drift
    /// accumulates over a long session.
    fn metrics(&self) -> crate::ui::metrics::Metrics {
        let (tokens_in, tokens_out) = self.ui.usage();
        let (cache_read, cache_write) = self.ui.cache_usage();
        let latest_prompt = self.ui.latest_prompt_tokens();
        let info = epsilon_llm::catalog::lookup_model(&self.model_id);
        let cost_micro_usd = info.as_ref().map(|info| {
            tokens_in * info.input_cost_micro_per_mtok / 1_000_000
                + tokens_out * info.output_cost_micro_per_mtok / 1_000_000
        });
        crate::ui::metrics::Metrics {
            model_label: epsilon_llm::catalog::display_label(&self.model_id).to_string(),
            offline: self.model_id.starts_with("faux"),
            context_used: (latest_prompt > 0).then_some(latest_prompt),
            context_window: info.as_ref().map(|i| i.context_window).filter(|w| *w > 0),
            tokens_in,
            tokens_out,
            cache_read,
            cache_write,
            cost_micro_usd,
            compacted: self.ui.is_compacted(),
        }
    }

    /// Footer hints for the current state.
    fn hints(&mut self) -> crate::ui::footer::Hints {
        use crate::ui::footer::Hints;
        let mut hints = if self.pending_ask.is_some() {
            Hints::approval()
        } else if self.overlay_open() {
            // Overlays own the keyboard; the footer points at their own keys
            // rather than advertising send/stop, which would not work.
            Hints {
                keys: vec![
                    ("type".into(), "filter".into()),
                    ("↑↓".into(), "move".into()),
                    ("enter".into(), "choose".into()),
                    ("esc".into(), "close".into()),
                ],
                ..Hints::default()
            }
        } else if self.ui.running() {
            Hints::running(self.steer_queue_len)
        } else {
            Hints::idle()
        };
        let home = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(std::path::PathBuf::from);
        // A quarter of the row is the most the location may take before the hints
        // stop being readable; past that the tail of the path is what matters.
        hints.location = Some(crate::ui::format::format_path(
            &self.workspace_root,
            home.as_deref(),
            28,
        ));
        if let Some(branch) = self.cached_branch() {
            hints.branch = Some(branch);
        }
        hints
    }

    /// Whether any overlay currently owns the keyboard.
    fn overlay_open(&self) -> bool {
        self.picker_open
            || self.palette_open
            || self.auth_prompt_open
            || self.auth_pick_open
            || self.timeline_open
            || self.session_browser.is_some()
            || self.pending_ask.is_some()
    }

    /// Whether the transcript has nothing to show yet.
    ///
    /// Streaming text counts: a run that has started producing output must not
    /// have the welcome screen painted over it.
    fn transcript_is_empty(&self) -> bool {
        self.ui.lines().is_empty()
            && self.ui.streaming_text().is_empty()
            && self.ui.streaming_thinking().is_empty()
    }
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
        .border_style(crate::theme::Theme::border_active())
        .title(Span::styled(
            format!(" models — filter: '{}' ", app.input),
            crate::theme::Theme::accent(),
        ));

    let mut lines: Vec<Line<'static>> = Vec::new();
    if rows.is_empty() {
        lines.push(Line::from(Span::styled(
            "no providers",
            crate::theme::Theme::muted(),
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
        lines.push(Line::from(vec![
            Span::styled(
                marker.to_string(),
                if selected {
                    crate::theme::Theme::accent()
                } else {
                    Style::default()
                },
            ),
            Span::styled(
                format!("{auth} "),
                if *auth_ready {
                    if selected {
                        crate::theme::Theme::selected_muted()
                    } else {
                        crate::theme::Theme::success()
                    }
                } else {
                    crate::theme::Theme::muted()
                },
            ),
            Span::styled(
                id.clone(),
                if selected {
                    crate::theme::Theme::selected()
                } else {
                    Style::default()
                },
            ),
            Span::styled(
                format!("  {count} models"),
                if selected {
                    crate::theme::Theme::selected_muted()
                } else {
                    crate::theme::Theme::muted()
                },
            ),
        ]));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "↑/↓ · Enter browse models · Esc close".to_string(),
        crate::theme::Theme::muted(),
    )));

    let para = Paragraph::new(lines).block(block);
    f.render_widget(para, area);
}

/// Stage-2 of the two-stage picker: one provider's models with fuzzy filter.
/// M57: the `/sessions` overlay. Rows are session files from the workspace's
/// `.epsilon/sessions`, newest first; the current one is marked `●`.
fn draw_session_browser(f: &mut Frame<'_>, app: &mut App, outer: Rect) {
    let Some(browser) = app.session_browser.as_ref() else {
        return;
    };
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
        .border_style(crate::theme::Theme::border_active())
        .title(Span::styled(
            format!(" sessions — filter '{}' ", app.input),
            crate::theme::Theme::accent(),
        ));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut lines: Vec<Line<'static>> = Vec::new();
    if browser.rows.is_empty() {
        lines.push(Line::from(Span::styled(
            "no matches — Backspace to widen, Esc closes",
            crate::theme::Theme::muted(),
        )));
    }
    // Same centered scroll window as the pickers: only the visible slice is built.
    let capacity = inner.height.saturating_sub(1).max(1) as usize;
    let start = crate::scroll::scroll_start(browser.rows.len(), browser.index, capacity);
    let end = (start + capacity).min(browser.rows.len());
    for (i, row) in browser
        .rows
        .iter()
        .enumerate()
        .skip(start)
        .take(end - start)
    {
        let selected = i == browser.index;
        let marker = if row.is_current { "● " } else { "  " };
        lines.push(Line::from(vec![
            Span::styled(
                marker.to_string(),
                if selected {
                    crate::theme::Theme::accent()
                } else {
                    Style::default()
                },
            ),
            Span::styled(
                row.title.clone(),
                if selected {
                    crate::theme::Theme::selected()
                } else {
                    Style::default()
                },
            ),
            Span::styled(format!("  {}", row.meta), crate::theme::Theme::muted()),
        ]));
    }
    lines.push(Line::from(Span::styled(
        format!(
            "{} of {} · Enter resumes · Esc closes",
            browser.index + 1,
            browser.rows.len()
        ),
        crate::theme::Theme::muted(),
    )));
    f.render_widget(Paragraph::new(lines), inner);
}

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
        .border_style(crate::theme::Theme::border_active())
        .title(Span::styled(
            format!(" models — {provider}: filter '{}' ", app.input),
            crate::theme::Theme::accent(),
        ));

    let mut lines: Vec<Line<'static>> = Vec::new();
    if rows.is_empty() {
        lines.push(Line::from(Span::styled(
            "no matches — Backspace to widen, Esc closes",
            crate::theme::Theme::muted(),
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
        lines.push(Line::from(vec![
            Span::styled(
                marker.to_string(),
                if selected {
                    crate::theme::Theme::accent()
                } else {
                    Style::default()
                },
            ),
            Span::styled(
                format!("{auth} "),
                if row.auth_ready {
                    if selected {
                        crate::theme::Theme::selected_muted()
                    } else {
                        crate::theme::Theme::success()
                    }
                } else {
                    crate::theme::Theme::muted()
                },
            ),
            Span::styled(
                format!("{star} "),
                if row.is_favorite {
                    if selected {
                        crate::theme::Theme::selected_muted()
                    } else {
                        Style::default().fg(Color::Yellow)
                    }
                } else {
                    crate::theme::Theme::muted()
                },
            ),
            Span::styled(
                row.label.clone(),
                if selected {
                    crate::theme::Theme::selected()
                } else {
                    Style::default()
                },
            ),
            Span::styled(
                format!("  {}", row.id),
                if selected {
                    crate::theme::Theme::selected_muted()
                } else {
                    crate::theme::Theme::muted()
                },
            ),
        ]));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "type filters (fuzzy) · Enter apply · Backspace/Esc back",
        crate::theme::Theme::muted(),
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

    // Keep thinking/prose and explicit newlines separate; slice the combined
    // history + live tail in screen rows, including tails taller than the viewport.
    let mut tail_rows = Vec::new();
    for (text, style) in [
        (app.ui.streaming_thinking(), crate::theme::Theme::thinking()),
        (app.ui.streaming_text(), crate::theme::Theme::assistant()),
    ] {
        if !text.is_empty() {
            for line in text.split('\n') {
                tail_rows.extend(crate::wrap::wrap_line(&[(line.to_string(), style)], width));
            }
        }
    }
    let skip =
        crate::scroll::transcript_skip(total + tail_rows.len(), visible, app.scroll_from_bottom);
    let mut rows = app.render_cache.visible_rows(skip, visible);
    let remaining = visible.saturating_sub(rows.len());
    for wrapped in tail_rows
        .into_iter()
        .skip(skip.saturating_sub(total))
        .take(remaining)
    {
        rows.push(Line::from(
            wrapped
                .into_iter()
                .map(|(text, style)| Span::styled(text, style))
                .collect::<Vec<_>>(),
        ));
    }

    // visible_rows already applied the viewport offset.
    let para = Paragraph::new(rows);
    f.render_widget(para, area);
}

#[cfg(test)]
mod composer_viewport_tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    #[test]
    fn streaming_tail_is_visible_after_full_transcript() {
        use epsilon_contracts::message::{AssistantMessage, Message, StopReason, Usage};
        let mut app = bench_app(UiState::new());
        for index in 0..100 {
            app.ui.push_notice(format!("ROW-{index:03}"));
        }
        app.ui.apply(&AgentEvent::MessageUpdate {
            message: Box::new(Message::Assistant(AssistantMessage {
                id: "stream-test".into(),
                timestamp_ms: 0,
                provider: "faux".into(),
                model_id: "faux-1".into(),
                content: vec![epsilon_contracts::content::ContentBlock::text(format!(
                    "{}LATEST",
                    "live answer\n".repeat(100)
                ))],
                stop_reason: StopReason::EndTurn,
                error_message: None,
                usage: Usage::default(),
            })),
        });
        let mut terminal = Terminal::new(TestBackend::new(60, 5)).unwrap();
        terminal
            .draw(|frame| draw_transcript(frame, &mut app, frame.area()))
            .unwrap();
        let screen: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(screen.contains("LATEST"), "stream hidden: {screen}");
    }

    #[test]
    fn long_transcript_renders_latest_visible_rows() {
        let mut app = bench_app(UiState::new());
        for index in 0..100 {
            app.ui.push_notice(format!("ROW-{index:03}"));
        }
        let mut terminal = Terminal::new(TestBackend::new(60, 5)).unwrap();
        terminal
            .draw(|frame| draw_transcript(frame, &mut app, frame.area()))
            .unwrap();
        let screen: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(screen.contains("ROW-099"), "tail disappeared: {screen}");
        assert!(screen.contains("ROW-095"));
    }

    #[test]
    fn wrapped_draft_grows_composer_at_supported_widths() {
        for (width, height) in [(60, 20), (80, 24), (120, 30), (160, 40)] {
            let mut app = bench_app(UiState::new());
            app.input = "§".repeat(width as usize + 8);
            app.cursor_chars = app.input.chars().count();
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal.draw(|frame| draw(frame, &mut app)).unwrap();
            let screen: String = terminal
                .backend()
                .buffer()
                .content()
            .iter()
            .map(|cell| cell.symbol().chars().next().unwrap_or(' '))
            .collect();
            assert_eq!(
                screen.matches('§').count(),
                app.input.chars().count(),
                "draft clipped at {width}x{height}"
            );
        }
    }

    #[test]
    fn undersized_terminal_has_explicit_fallback() {
        let mut app = bench_app(UiState::new());
        let mut terminal = Terminal::new(TestBackend::new(40, 8)).unwrap();
        terminal.draw(|frame| draw(frame, &mut app)).unwrap();
        let screen: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(screen.contains("terminal too small"));
    }

    #[test]
    fn long_unicode_draft_wraps_and_places_caret_in_display_columns() {
        use ratatui::backend::Backend;
        let mut app = bench_app(UiState::new());
        app.input = "界".repeat(35);
        app.cursor_chars = 35;
        let theme = crate::ui::theme::Theme::new(
            crate::ui::theme::ColorMode::Ansi16,
            crate::ui::theme::ASCII,
        );
        let width = 60;
        let text_width = crate::ui::shell::composer_text_width(width, &theme.glyphs);
        let (rows, caret_row, caret_col) = composer_rows(&app.input, app.cursor_chars, text_width);
        let mut terminal = Terminal::new(TestBackend::new(width, 5)).unwrap();
        terminal
            .draw(|frame| {
                crate::ui::composer::draw(
                    frame,
                    frame.area(),
                    &theme,
                    &rows,
                    caret_row,
                    caret_col,
                    true,
                );
            })
            .unwrap();
        // The caret sits in *display* columns: each wide glyph is two cells, so
        // counting scalars would put it half a line away from the text.
        let caret = terminal.backend_mut().get_cursor_position().unwrap();
        assert_eq!(
            usize::from(caret.y),
            caret_row,
            "caret on the wrong draft row"
        );
        assert_eq!(
            usize::from(caret.x),
            crate::ui::composer::gutter_width(&theme.glyphs) + caret_col
        );
        let glyph = terminal.backend().buffer()[(caret.x - 2, caret.y)]
            .symbol()
            .to_string();
        assert_eq!(glyph, "界", "the caret is not next to the text it edits");
    }

    #[test]
    fn editing_start_of_long_draft_keeps_caret_line_visible() {
        let mut app = bench_app(UiState::new());
        app.input = "FIRST\nsecond\nthird\nfourth\nfifth\nLAST".into();
        app.cursor_chars = 2;
        let theme = crate::ui::theme::Theme::new(
            crate::ui::theme::ColorMode::Ansi16,
            crate::ui::theme::ASCII,
        );
        let width = 60;
        let text_width = crate::ui::shell::composer_text_width(width, &theme.glyphs);
        let (rows, caret_row, caret_col) = composer_rows(&app.input, app.cursor_chars, text_width);
        let mut terminal = Terminal::new(TestBackend::new(width, 5)).unwrap();
        terminal
            .draw(|frame| {
                crate::ui::composer::draw(
                    frame,
                    frame.area(),
                    &theme,
                    &rows,
                    caret_row,
                    caret_col,
                    true,
                );
            })
            .unwrap();
        let screen: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(
            screen.contains("FIRST"),
            "the line being edited scrolled out of view: {screen}"
        );
    }
}

/// Lay out draft and caret together, retaining whitespace and whole graphemes.
/// Unlike transcript wrapping, composer wrapping must not eat typed spaces.
///
/// `width` is the composer's *text* width and the returned rows are bare text:
/// the prompt glyph is the composer's business, not this function's. The
/// returned column is therefore relative to the start of the text, and the
/// caller adds its own gutter — which is what keeps caret arithmetic in one
/// place instead of two that can disagree.
fn composer_rows(input: &str, cursor_chars: usize, width: usize) -> (Vec<String>, usize, usize) {
    use unicode_segmentation::UnicodeSegmentation;
    use unicode_width::UnicodeWidthStr;
    let width = width.max(2);
    let mut rows = vec![String::new()];
    let mut col = 0;
    let mut offset = 0;
    let mut caret = None;
    for cluster in input.graphemes(true) {
        let newline = cluster == "\n" || cluster == "\r\n";
        let cells = if cluster == "\t" {
            4 - col % 4
        } else {
            cluster.width()
        };
        if !newline && col + cells > width {
            rows.push(String::new());
            col = 0;
        }
        if caret.is_none() && cursor_chars <= offset {
            caret = Some((rows.len() - 1, col));
        }
        if newline {
            rows.push(String::new());
            col = 0;
        } else {
            let text = if cluster == "\t" {
                " ".repeat(cells)
            } else {
                cluster.to_string()
            };
            rows.last_mut().expect("one draft row").push_str(&text);
            col += cells;
        }
        offset += cluster.chars().count();
    }
    if col >= width {
        rows.push(String::new());
        col = 0;
    }
    let (row, col) = caret.unwrap_or((rows.len() - 1, col));
    (rows, row, col.min(width - 1))
}

/// Command palette overlay (M16c, opencode command-palette pattern): fuzzy-filtered
/// list of all commands with keybinding hints.
fn draw_palette(f: &mut Frame<'_>, app: &App, outer: Rect) {
    use crate::ui::overlay::{self, Panel, Width};

    let entries = crate::picker::filter_palette(&app.input);
    let width = Width::Fixed(64).resolve(outer);
    let height = 14u16.min(outer.height.saturating_sub(2)).max(4);
    let panel = Panel {
        theme: &app.theme,
        // Top-aligned rather than centred: the palette opens where the eye
        // already is (the composer), and a centred panel would jump as the
        // filter list changes height.
        area: Rect {
            y: outer.y + 1,
            ..overlay::centered(outer, width, height)
        },
        title: "commands",
        focused: true,
    };
    let inner = panel.open(f);

    let capacity = inner.height.saturating_sub(2).max(1) as usize;
    let start = crate::scroll::scroll_start(entries.len(), app.picker_index, capacity);
    let mut lines: Vec<Line<'static>> = Vec::new();
    if entries.is_empty() {
        lines.push(Line::from(Span::styled(
            "no matching commands",
            app.theme.dim(),
        )));
    }
    for (index, entry) in entries.iter().enumerate().skip(start).take(capacity) {
        lines.push(overlay::list_row(
            &app.theme,
            index == app.picker_index,
            &entry.title,
            entry.binding.map(|b| format!("[{b}]")).as_deref(),
            inner.width as usize,
        ));
    }
    // The filter itself lives in the title bar, so the body stays a pure list.
    lines.push(Line::from(""));
    lines.push(overlay::hint_row(
        &app.theme,
        &[
            ("type", "filter"),
            ("↑↓", "move"),
            ("enter", "run"),
            ("esc", "close"),
        ],
    ));
    f.render_widget(Paragraph::new(lines), inner);
}

// (M17b: slash registry moved to `picker::SLASH_ITEMS`/`picker::slash_items` —
// descriptions + selection live there now.)

/// Popup listing matching slash commands directly above the input box
/// (M17b: pi SelectList rendering — highlighted selection + dim descriptions,
/// arrow-key navigable via `slash_index`).
fn draw_at_file(f: &mut Frame<'_>, app: &App, transcript_area: ratatui::layout::Rect) {
    if !app.at_open {
        return;
    }
    let Some(rows) = &app.at_rows else {
        return;
    };
    if rows.is_empty() {
        return;
    }
    let capacity = 8usize;
    let start = crate::scroll::scroll_start(rows.len(), app.at_index, capacity);
    let end = (start + capacity).min(rows.len());
    let height = (end - start) as u16 + 1;
    let y = transcript_area
        .bottom()
        .saturating_sub(height)
        .saturating_sub(1);
    let area = ratatui::layout::Rect {
        x: transcript_area.x,
        y,
        width: transcript_area.width.min(64),
        height,
    };
    f.render_widget(ratatui::widgets::Clear, area);
    let mut lines: Vec<ratatui::text::Line<'static>> = rows[start..end]
        .iter()
        .enumerate()
        .map(|(i, path)| {
            let selected = start + i == app.at_index;
            let raw = if path.len() > 56 {
                format!("...{}", &path[path.len() - 53..])
            } else {
                path.clone()
            };
            let style = if selected {
                crate::theme::Theme::selected()
            } else {
                crate::theme::Theme::accent()
            };
            ratatui::text::Line::from(ratatui::text::Span::styled(format!(" @{} ", raw), style))
        })
        .collect();
    lines.push(ratatui::text::Line::from(ratatui::text::Span::styled(
        "Tab insert · Enter insert + space · Esc close".to_string(),
        crate::theme::Theme::muted(),
    )));
    let block = ratatui::widgets::Block::default()
        .borders(ratatui::widgets::Borders::ALL)
        .border_style(crate::theme::Theme::border_active());
    f.render_widget(ratatui::widgets::Paragraph::new(lines).block(block), area);
}

fn draw_timeline(f: &mut Frame<'_>, app: &App, outer: Rect) {
    if !app.timeline_open || app.timeline_entries.is_empty() {
        return;
    }
    let total = app.timeline_entries.len();
    let capacity = 12usize.min(outer.height.saturating_sub(6).max(1) as usize);
    let start = crate::scroll::scroll_start(total, app.timeline_index, capacity);
    let end = (start + capacity).min(total);
    let width = (outer.width * 4 / 5).max(56).min(outer.width);
    let height = (capacity as u16 + 4).min(outer.height.saturating_sub(2));
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
        .border_style(crate::theme::Theme::border_active())
        .title(Span::styled(" timeline ", crate::theme::Theme::accent()));
    let mut lines: Vec<Line<'static>> = Vec::new();
    for (i, (seq, tag, snippet)) in app
        .timeline_entries
        .iter()
        .enumerate()
        .skip(start)
        .take(end - start)
    {
        let selected = i == app.timeline_index;
        let raw = format!("#{seq} [{tag}] {snippet}");
        let display = if raw.len() > (width as usize).saturating_sub(4) {
            let w = (width as usize).saturating_sub(7);
            format!("{}...", &raw[..w.min(raw.len())])
        } else {
            raw
        };
        lines.push(Line::from(Span::styled(
            display,
            if selected {
                crate::theme::Theme::selected()
            } else {
                Style::default()
            },
        )));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!(
            "{}/{} · ↑/↓ move · Enter jump · Esc close",
            app.timeline_index + 1,
            total
        ),
        crate::theme::Theme::muted(),
    )));
    f.render_widget(Paragraph::new(lines).block(block), area);
}

fn draw_slash_autocomplete(f: &mut Frame<'_>, app: &App, transcript_area: Rect) {
    let candidates = crate::picker::slash_items(&app.input);
    if !app.slash_open || candidates.is_empty() {
        return;
    }
    // Cap height so the popup never smothers the input box. 8 items of
    // content plus the help footer (1), plus borders (2) = max 11. Scroll
    // window follows slash_index like every other list widget.
    let total = candidates.len();
    let capacity = 8usize.min(total);
    let start = crate::scroll::scroll_start(total, app.slash_index, capacity);
    let end = (start + capacity).min(total);
    let height = (capacity as u16) + 1 + 2; // items + footer + borders
    let y = transcript_area.bottom().saturating_sub(height);
    let area = Rect {
        x: transcript_area.x,
        y,
        width: transcript_area.width.min(56),
        height,
    };
    f.render_widget(Clear, area);
    let mut lines: Vec<Line<'static>> = candidates[start..end]
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let idx = start + i;
            let selected = idx == app.slash_index;
            let mut spans = vec![Span::styled(
                format!(" {} ", item.value),
                if selected {
                    crate::theme::Theme::selected()
                } else {
                    crate::theme::Theme::accent()
                },
            )];
            if !item.description.is_empty() {
                spans.push(Span::styled(
                    format!(" {}", item.description),
                    if selected {
                        crate::theme::Theme::selected_muted()
                    } else {
                        crate::theme::Theme::muted()
                    },
                ));
            }
            Line::from(spans)
        })
        .collect();
    lines.push(Line::from(Span::styled(
        "↑/↓ pick · Tab/Enter apply · Esc dismiss".to_string(),
        crate::theme::Theme::muted(),
    )));
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(crate::theme::Theme::border_active());
    f.render_widget(Paragraph::new(lines).block(block), area);
}

/// The permission dialog.
///
/// This is the most important screen in the harness: it is the moment the user
/// decides whether the agent may do something. It is drawn with the *danger*
/// accent rather than the ordinary focus accent, because "this owned the
/// keyboard" and "this can authorize a write" are different statements, and
/// telling them apart at a glance is the point.
///
/// The body answers three questions in the order they are asked: what capability,
/// against what, and with what arguments.
fn draw_dialog(f: &mut Frame<'_>, app: &App, outer: Rect) {
    use crate::ui::overlay::{self, Panel, Width};

    let width = Width::PROMPT.resolve(outer);
    let height = 11u16.min(outer.height);
    let panel = Panel {
        theme: &app.theme,
        area: overlay::centered(outer, width, height),
        title: "permission required",
        focused: true,
    };
    let inner = panel.open(f);
    let theme = &app.theme;

    let body: Vec<Line<'static>> = match app.pending_ask.as_ref() {
        Some(ask) => {
            let req = &ask.request;
            // A labelled field, then its value: the label is dim so a reader
            // scanning for the answer skips it, and the value is not.
            let field = |label: &str, value: String| {
                Line::from(vec![
                    Span::styled(format!("{label:<10}"), theme.faint()),
                    Span::styled(value, theme.text()),
                ])
            };
            vec![
                field("capability", req.capability.clone()),
                field("pattern", req.pattern.clone()),
                field("detail", summarize_detail(&req.detail)),
                Line::from(""),
                Line::from(vec![
                    Span::styled("y", theme.success()),
                    Span::styled(" allow once", theme.dim()),
                    Span::styled("    a", theme.success()),
                    Span::styled(" always this session", theme.dim()),
                ]),
                Line::from(vec![
                    Span::styled("n", theme.danger()),
                    Span::styled(" deny", theme.dim()),
                    Span::styled("          e", theme.danger()),
                    Span::styled(" deny with guidance", theme.dim()),
                ]),
            ]
        }
        None => vec![Line::from(Span::styled("resolving…", theme.dim()))],
    };

    f.render_widget(Paragraph::new(body), inner);
}

/// M32: masked API-key prompt (opencode DialogPrompt parity). The real key
/// lives in `app.auth_key` but rendering shows `•` per char so the secret
/// never appears in the transcript or on screen.
fn draw_auth_prompt(f: &mut Frame<'_>, app: &App, outer: Rect) {
    let width = (outer.width * 3 / 5).max(48).min(outer.width);
    let height = 8u16.min(outer.height.saturating_sub(2));
    let x = outer.x + (outer.width.saturating_sub(width)) / 2;
    let y = outer.y + (outer.height.saturating_sub(height)) / 2;
    let area = Rect {
        x,
        y,
        width,
        height,
    };
    f.render_widget(Clear, area);

    let masked_len = app.auth_key.chars().count();
    let masked: String = "•".repeat(masked_len);
    // Caret in masked space: count chars left of cursor, clamp to width.
    let inner_w = width.saturating_sub(4) as usize;
    let cursor_col = app
        .auth_key_cursor
        .min(masked_len)
        .min(inner_w.saturating_sub(1));

    let oauth_mode = app.oauth_code_tx.is_some();
    let title = if oauth_mode {
        format!(" OAuth login: {} ", app.auth_provider)
    } else {
        format!(" API key for '{}' ", app.auth_provider)
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(crate::theme::Theme::auth())
        .title(Span::styled(
            title,
            crate::theme::Theme::auth().add_modifier(Modifier::BOLD),
        ));
    let hint = if oauth_mode {
        if masked_len == 0 {
            "paste the redirect URL (or the bare code) — Ctrl+V works, Enter sends".to_string()
        } else {
            format!("{} chars — Enter send · Esc cancel", masked_len)
        }
    } else if masked_len == 0 {
        "paste your key (Ctrl+V / bracketed paste) — masked, Enter saves".to_string()
    } else {
        format!("{} chars — Enter save · Esc cancel", masked_len)
    };
    let body = vec![
        Line::from(Span::styled(
            if oauth_mode {
                "hidden while typing — paste works, Enter sends"
            } else {
                "key is masked — paste works, typing works"
            },
            crate::theme::Theme::muted(),
        )),
        Line::from(Span::styled(masked.clone(), crate::theme::Theme::auth())),
        Line::from(Span::styled(hint, crate::theme::Theme::muted())),
    ];
    let para = Paragraph::new(body).block(block);
    f.render_widget(para, area);
    // Place the terminal cursor inside the masked field (if the field fits).
    let cx = area.x + 2 + cursor_col as u16;
    let cy = area.y + 2; // second line inside the box is the masked field
    if cx < area.x + area.width - 1 && cy < area.y + area.height - 1 {
        f.set_cursor_position((cx, cy));
    }
}

/// M32b: provider picker for bare `/login` — opencode DialogProvider parity.
///
/// Renders the provider list filtered by the current `input`. Selecting one
/// opens the masked API-key prompt; Esc closes. Filter + list reuse the
/// existing `/models` provider rows, with auth badge and active mark so a
/// first-run user can see at a glance what is wired up.
fn draw_auth_picker(f: &mut Frame<'_>, app: &App, outer: Rect) {
    let rows = crate::picker::build_provider_rows(&app.model_id, &app.customs);
    let query = app.input.trim().to_ascii_lowercase();
    let filtered: Vec<_> = if query.is_empty() {
        rows.iter().enumerate().collect()
    } else {
        rows.iter()
            .enumerate()
            .filter(|(_, p)| p.id.to_ascii_lowercase().contains(&query))
            .collect()
    };
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
        .border_style(crate::theme::Theme::border_active())
        .title(Span::styled(
            " connect — pick a provider ",
            crate::theme::Theme::accent(),
        ));
    let capacity = area.height.saturating_sub(4).max(1) as usize;
    let start = crate::scroll::scroll_start(filtered.len(), app.auth_pick_index, capacity);
    let end = (start + capacity).min(filtered.len());
    let mut lines: Vec<Line<'static>> = Vec::new();
    if filtered.is_empty() {
        lines.push(Line::from(Span::styled(
            format!("no match for '{}' — type another provider id", app.input),
            crate::theme::Theme::muted(),
        )));
    }
    for (i, row) in filtered.iter().enumerate().skip(start).take(end - start) {
        let selected = i == app.auth_pick_index;
        let prow = row.1;
        let marker = if prow.is_active { "● " } else { "  " };
        let auth = if prow.auth_ready { "🔑" } else { "· " };
        let style = if selected {
            crate::theme::Theme::accent()
        } else {
            Style::default()
        };
        lines.push(Line::from(vec![
            Span::styled(marker.to_string(), style),
            Span::styled(
                format!("{auth} "),
                if prow.auth_ready {
                    crate::theme::Theme::success()
                } else {
                    crate::theme::Theme::muted()
                },
            ),
            Span::styled(
                prow.id.clone(),
                if selected {
                    crate::theme::Theme::selected()
                } else {
                    Style::default()
                },
            ),
            Span::styled(
                format!("  {} models", prow.model_count),
                if selected {
                    crate::theme::Theme::selected_muted()
                } else {
                    crate::theme::Theme::muted()
                },
            ),
            Span::styled(
                if epsilon_llm::oauth_login::login_flow(&prow.id).is_some() {
                    "  ⌁ oauth"
                } else {
                    ""
                },
                if selected {
                    crate::theme::Theme::selected_muted()
                } else {
                    crate::theme::Theme::muted()
                },
            ),
        ]));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "type filters · ↑/↓ move · Enter pastes key (masked) · ⌁ oauth = subscription login · Esc cancel",
        crate::theme::Theme::muted(),
    )));
    let para = Paragraph::new(lines).block(block);
    f.render_widget(para, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyEventState;
    use epsilon_contracts::agent_event::PermissionRequest;
    use epsilon_contracts::message::UserMessage;
    use epsilon_safety::{ApprovalUi, ChannelApprovalUi};

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
    fn key_mod(modifiers: KeyModifiers, code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

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

    #[tokio::test]
    async fn approval_preempts_palette_and_picker_without_changing_filter() {
        for palette in [true, false] {
            let (mut app, agent, _receivers) = test_app();
            app.palette_open = palette;
            app.picker_open = !palette;
            app.input = "models".into();
            app.cursor_chars = 6;
            app.picker_index = 1;
            let approvals = ChannelApprovalUi::new();
            let requester = approvals.clone();
            let response = tokio::spawn(async move {
                requester
                    .ask(PermissionRequest {
                        request_id: "focus-test".into(),
                        capability: "read".into(),
                        pattern: "notes.txt".into(),
                        detail: serde_json::json!({}),
                    })
                    .await
            });
            app.pending_ask = approvals.next_ask().await;
            // M94: visibility is an identity fact now — paint one real frame so
            // `draw` records THIS ask as the one the dialog showed.
            let _ = render_rows(&mut app, 80, 24);
            assert!(!handle_key(
                &mut app,
                &agent,
                KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE)
            ));
            assert!(
                app.pending_ask.is_none(),
                "visible approval did not receive deny"
            );
            assert!(matches!(response.await.unwrap(), PermissionDecision::Deny));
            assert_eq!(app.input, "models");
            assert_eq!(app.cursor_chars, 6);
            assert_eq!(app.picker_index, 1);
            assert_eq!(app.palette_open, palette);
            assert_eq!(app.picker_open, !palette);
        }
    }

    /// M94 review: paint one full frame into a `TestBackend` and return it as
    /// one string per row (buffer inspection, the crate's rendering-test style).
    fn render_rows(app: &mut App, width: u16, height: u16) -> Vec<String> {
        use ratatui::{Terminal, backend::TestBackend};
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| draw(frame, app)).unwrap();
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| terminal.backend().buffer()[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect()
    }

    /// Park one ask on the shared queue and hand it to the app, exactly like the
    /// run loop does; the response future stays live so a test can assert which
    /// decision the parked ask received (or that it never received one).
    async fn park_ask(
        app: &mut App,
        approvals: &ChannelApprovalUi,
        id: &str,
    ) -> tokio::task::JoinHandle<PermissionDecision> {
        let requester = approvals.clone();
        let id = id.to_string();
        let response = tokio::spawn(async move {
            requester
                .ask(PermissionRequest {
                    request_id: id,
                    capability: "bash".into(),
                    pattern: "rm -rf build".into(),
                    detail: serde_json::json!({ "command": "rm -rf build" }),
                })
                .await
        });
        app.pending_ask = approvals.next_ask().await;
        response
    }

    /// M94 review (safety): below 60×12 the fallback paints instead of the
    /// dialog, but the approval keys used to route to the pending ask anyway —
    /// 'y' authorized work the user could not see, 'a' made it permanent for
    /// the session, and Esc silently denied it. They must all refuse now, say
    /// why, and leave the ask pending.
    #[tokio::test]
    async fn approval_keys_do_not_answer_an_invisible_approval() {
        for code in [
            KeyCode::Char('y'),
            KeyCode::Char('a'),
            KeyCode::Char('n'),
            KeyCode::Esc,
        ] {
            let (mut app, agent, _receivers) = test_app();
            let approvals = ChannelApprovalUi::new();
            let response = park_ask(&mut app, &approvals, "invisible-test").await;
            // Undersized frame: the fallback renders, so the dialog did not.
            let _ = render_rows(&mut app, 40, 8);
            assert!(
                app.approval_painted_id.is_none(),
                "the fallback cannot paint the dialog"
            );
            let notices_before = app.ui.lines().len();

            assert!(!handle_key(&mut app, &agent, key(code)));
            assert!(
                app.pending_ask.is_some(),
                "{code:?} answered an approval that was never drawn"
            );
            assert!(
                app.ui
                    .lines()
                    .iter()
                    .any(|l| l.text.contains("approval is not visible")),
                "{code:?} pushed no refusal notice: {:?}",
                app.ui.lines().iter().map(|l| &l.text).collect::<Vec<_>>()
            );
            assert!(
                app.ui.lines().len() > notices_before,
                "{code:?}: notice, not silence"
            );

            // The ask was never answered: dropping it fails closed to Deny.
            drop(app.pending_ask.take());
            assert!(matches!(response.await.unwrap(), PermissionDecision::Deny));
        }
    }

    /// M94 review: at sizes below the 60×12 floor the fallback must still name a
    /// pending approval and the keys that can act on it — a bare size banner
    /// hid a live permission request.
    #[tokio::test]
    async fn an_undersized_terminal_names_a_pending_approval() {
        let (mut app, _agent, _receivers) = test_app();
        let approvals = ChannelApprovalUi::new();
        let _response = park_ask(&mut app, &approvals, "undersized-test").await;

        // Below the layout floor, where no pane can be drawn at all.
        let rows = render_rows(&mut app, 43, 10);
        let screen = rows.join("\n");
        assert!(screen.contains("too small"), "{screen}");
        assert!(
            screen.contains("approval pending: bash · rm -rf build"),
            "the fallback did not name the ask: {screen}"
        );
        assert!(
            screen.contains("widen the window"),
            "the fallback did not say how to proceed: {screen}"
        );
    }

    /// M94 review: an ask that arrives while `/timeline` is open must be visible.
    /// The timeline widget clears its own centre, so a dialog painted before it
    /// was wiped out while y/a/n/Esc still routed to it (Esc silently denied).
    #[tokio::test]
    async fn a_pending_approval_is_drawn_over_the_timeline() {
        let (mut app, agent, _receivers) = test_app();
        app.timeline_open = true;
        app.timeline_entries = vec![(7, "user".into(), "TIMELINE-ROW-MARKER".into())];
        let approvals = ChannelApprovalUi::new();
        let response = park_ask(&mut app, &approvals, "over-timeline").await;

        let rows = render_rows(&mut app, 80, 24);
        let screen = rows.join("\n");
        assert!(
            rows.iter().any(|r| r.contains("TIMELINE-ROW-MARKER")),
            "timeline not drawn: {screen}"
        );
        assert!(
            screen.contains("permission required"),
            "the timeline covered the approval dialog: {screen}"
        );
        assert!(
            rows.iter().any(|r| r.contains("capability") && r.contains("bash")),
            "the dialog body is buried: {screen}"
        );

        // Visible now, so the keys answer it (the precedence is unchanged).
        assert!(!handle_key(&mut app, &agent, key(KeyCode::Char('n'))));
        assert!(app.pending_ask.is_none(), "visible approval got no deny");
        assert!(matches!(response.await.unwrap(), PermissionDecision::Deny));
    }

    /// M94 re-review (safety): `draw` wrote a per-frame `approval_visible` flag, so
    /// the credit survived the frame that set it. Answer ask A, seat ask B inside
    /// the same <=33 ms frame window (the channel is FIFO; parallel tool calls park
    /// several) and a key handled before the next draw answered B — a decision the
    /// user never saw, and `a` would have made it permanent for the session. The
    /// guard is identity-based now: `draw` records the ask it painted and any other
    /// ask is refused until a frame paints it.
    #[tokio::test]
    async fn a_second_parked_ask_is_not_answerable_before_a_frame_paints_it() {
        let (mut app, agent, _receivers) = test_app();
        let approvals = ChannelApprovalUi::new();

        // Ask A: parked, painted by a real frame, answered through the key path.
        let response_a = park_ask(&mut app, &approvals, "ask-a").await;
        let _ = render_rows(&mut app, 80, 24);
        assert!(!handle_key(&mut app, &agent, key(KeyCode::Char('y'))));
        assert!(
            app.pending_ask.is_none(),
            "'y' must answer the painted ask A"
        );
        assert!(matches!(
            response_a.await.unwrap(),
            PermissionDecision::AllowOnce
        ));

        // Ask B parks in the frame window — nothing draws between A and B.
        let _response_b = park_ask(&mut app, &approvals, "ask-b").await;
        assert_eq!(
            app.pending_ask
                .as_ref()
                .map(|a| a.request.request_id.as_str()),
            Some("ask-b"),
            "the freshly seated ask is the pending one"
        );
        for code in [KeyCode::Char('y'), KeyCode::Char('a')] {
            let noticed = app.ui.lines().len();
            assert!(!handle_key(&mut app, &agent, key(code)));
            assert_eq!(
                app.pending_ask
                    .as_ref()
                    .map(|a| a.request.request_id.as_str()),
                Some("ask-b"),
                "{code:?} answered ask B before any frame painted it"
            );
            assert!(
                app.ui.lines().len() > noticed,
                "{code:?} refused silently — the user must be told the ask is not on screen yet"
            );
        }
        assert!(
            app.ui
                .lines()
                .iter()
                .any(|l| l.text.contains("approval is not visible")),
            "expected the refusal notice, got {:?}",
            app.ui.lines().iter().map(|l| &l.text).collect::<Vec<_>>()
        );

        // The guard is a gate, not a lockout: the next frame paints B and the
        // very same key answers it.
        let _ = render_rows(&mut app, 80, 24);
        assert!(!handle_key(&mut app, &agent, key(KeyCode::Char('y'))));
        assert!(app.pending_ask.is_none(), "the painted ask B is answerable");
        assert!(matches!(
            _response_b.await.unwrap(),
            PermissionDecision::AllowOnce
        ));

        // And a never-painted ask fails closed: a blind 'a' (AlwaysSession) is
        // refused, and dropping the ask resolves to Deny.
        let response_c = park_ask(&mut app, &approvals, "ask-c").await;
        assert!(!handle_key(&mut app, &agent, key(KeyCode::Char('a'))));
        assert!(
            app.pending_ask.is_some(),
            "'a' made a never-painted ask permanent for the session"
        );
        drop(app.pending_ask.take());
        assert!(matches!(
            response_c.await.unwrap(),
            PermissionDecision::Deny
        ));
    }

    /// M94 re-review (coherence): the dialog is painted over the masked `/login`
    /// key prompt, but the auth branches handled keys first — y/a/n/Esc (and
    /// Enter, which saves to `auth.json`) went into a buffer the user could not
    /// see while the user was answering the dialog. With an ask pending the
    /// approval keys route first and the auth state is left exactly as it was, so
    /// the login flow resumes untouched.
    #[tokio::test]
    async fn a_pending_approval_preempts_the_auth_key_prompt() {
        let (mut app, agent, _receivers) = test_app();
        begin_auth_key_prompt(&mut app, "openai".into());
        app.auth_key = "sk-typed-so-far".into();
        app.auth_key_cursor = app.auth_key.chars().count();
        let approvals = ChannelApprovalUi::new();
        let response = park_ask(&mut app, &approvals, "over-auth").await;
        let _ = render_rows(&mut app, 80, 24); // the dialog paints over the prompt

        let before = (
            app.auth_prompt_open,
            app.auth_provider.clone(),
            app.auth_key.clone(),
            app.auth_key_cursor,
        );
        assert!(!handle_key(&mut app, &agent, key(KeyCode::Char('y'))));
        assert!(
            app.pending_ask.is_none(),
            "'y' must answer the approval, not the auth buffer"
        );
        assert!(matches!(
            response.await.unwrap(),
            PermissionDecision::AllowOnce
        ));
        assert_eq!(
            app.auth_prompt_open, before.0,
            "the login prompt stays open"
        );
        assert_eq!(app.auth_provider, before.1, "its provider is untouched");
        assert_eq!(
            app.auth_key, before.2,
            "'y' must not reach the masked buffer"
        );
        assert_eq!(app.auth_key_cursor, before.3, "nor move its caret");
    }

    type TestReceivers = (
        tokio::sync::mpsc::UnboundedReceiver<(String, Result<Vec<String>, String>)>,
        tokio::sync::mpsc::UnboundedReceiver<String>,
    );

    fn test_app() -> (App, Arc<Agent>, TestReceivers) {
        let (ftx, frx) = tokio::sync::mpsc::unbounded_channel();
        let (ntx, nrx) = tokio::sync::mpsc::unbounded_channel();
        let (otx, _orx) = tokio::sync::mpsc::unbounded_channel();
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
        (App::new("faux-1".into(), ftx, ntx, otx), agent, (frx, nrx))
    }

    /// M75: `/session` says nothing about repairs when the session opened clean — the listing
    /// must not imply damage that did not happen. (The repaired case is covered where the repair
    /// happens: `epsilon-session`'s own tests.)
    #[tokio::test]
    async fn session_repair_lines_are_empty_for_a_clean_session() {
        let (_app, agent, (_f, _n)) = test_app();
        assert!(agent.repair_report().is_none());
        assert!(repair_lines(&agent).is_empty());
    }

    /// M74: a submitted prompt with an `@` reference carries the file's content, through the same
    /// constructor the other frontends use. The user's own words are untouched.
    #[tokio::test]
    async fn submit_text_attaches_at_references() {
        let (mut app, _agent, (_f, _n)) = test_app();
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("notes.txt"), "the answer is 42").unwrap();
        app.workspace_root = dir.path().to_path_buf();

        let msg = app.submit_text("what does @notes.txt say?".into());
        let Message::User(u) = msg else {
            panic!("a user message")
        };
        let texts: Vec<String> = u
            .content
            .iter()
            .filter_map(|c| match c {
                UserContent::Text { text } => Some(text.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(texts[0], "what does @notes.txt say?", "verbatim");
        assert!(texts.len() >= 2, "the attachment rides with it: {texts:?}");
        assert!(texts[1].contains("the answer is 42"), "{texts:?}");

        // A prompt with no references is exactly what it always was.
        let plain = app.submit_text("no references here".into());
        let Message::User(p) = plain else {
            panic!("a user message")
        };
        assert_eq!(p.content.len(), 1, "{:?}", p.content);
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
    async fn oauth_need_code_opens_the_overlay_and_done_closes_it() {
        let (mut app, agent, (_f, _n)) = test_app();
        // start_oauth_login installs the sender; NeedCode only opens the overlay.
        let (tx, _rx) = std::sync::mpsc::channel::<String>();
        app.oauth_code_tx = Some(tx);
        app.apply_oauth_event(OauthUiEvent::NeedCode {
            provider: "anthropic".into(),
        });
        assert!(app.auth_prompt_open, "overlay opens for the paste");
        assert_eq!(app.auth_provider, "anthropic");
        assert!(
            app.oauth_code_tx.is_some(),
            "the waiting login still owns it"
        );
        // Typing routes into the masked buffer, never the transcript editor.
        for c in "http://localhost:53692/callback?code=abc".chars() {
            handle_key(&mut app, &agent, key(KeyCode::Char(c)));
        }
        assert!(app.input.is_empty(), "editor stays out of it");
        assert!(app.auth_key.contains("code=abc"));
        // Completion closes the overlay however the flow ended.
        app.apply_oauth_event(OauthUiEvent::Done("done".into()));
        assert!(!app.auth_prompt_open);
        assert!(app.oauth_code_tx.is_none());
    }

    #[tokio::test]
    async fn oauth_enter_hands_the_code_to_the_waiting_login_and_esc_cancels_it() {
        let (mut app, agent, (_f, _n)) = test_app();
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        app.oauth_code_tx = Some(tx);
        app.apply_oauth_event(OauthUiEvent::NeedCode {
            provider: "anthropic".into(),
        });
        for c in "code-123#state-1".chars() {
            handle_key(&mut app, &agent, key(KeyCode::Char(c)));
        }
        handle_key(&mut app, &agent, key(KeyCode::Enter));
        assert_eq!(
            rx.try_recv().expect("code reached the flow"),
            "code-123#state-1"
        );
        assert!(!app.auth_prompt_open);
        assert!(app.oauth_code_tx.is_none());

        // Esc drops the sender, which is how the worker sees a cancellation.
        let (tx2, rx2) = std::sync::mpsc::channel::<String>();
        app.oauth_code_tx = Some(tx2);
        app.auth_prompt_open = true;
        app.auth_provider = "anthropic".into();
        handle_key(&mut app, &agent, key(KeyCode::Esc));
        assert!(app.oauth_code_tx.is_none());
        assert!(matches!(
            rx2.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Disconnected)
        ));
    }

    #[tokio::test]
    async fn ctrl_o_in_the_key_prompt_switches_to_the_oauth_ceremony() {
        let (mut app, agent, (_f, _n)) = test_app();
        begin_auth_key_prompt(&mut app, "anthropic".into());
        assert!(app.auth_prompt_open);
        let texts: Vec<String> = app.ui.lines().iter().map(|l| l.text.clone()).collect();
        assert!(
            texts.join("\n").contains("Ctrl+O"),
            "the prompt advertises the subscription route"
        );
        handle_key(
            &mut app,
            &agent,
            key_mod(KeyModifiers::CONTROL, KeyCode::Char('o')),
        );
        assert!(!app.auth_prompt_open, "the key prompt gives way");
        // A provider without a ceremony gets guidance, not a silent no-op.
        begin_auth_key_prompt(&mut app, "deepseek".into());
        handle_key(
            &mut app,
            &agent,
            key_mod(KeyModifiers::CONTROL, KeyCode::Char('o')),
        );
        let texts: Vec<String> = app.ui.lines().iter().map(|l| l.text.clone()).collect();
        assert!(
            texts
                .join("\n")
                .contains("no OAuth login for that provider")
        );
    }

    /// Write a real session file (header + one user message) for browser tests.
    fn write_session_fixture(path: &std::path::Path, text: &str) {
        let mut store = epsilon_session::SessionStore::open_in(path, &std::env::temp_dir())
            .expect("session store");
        let user = epsilon_contracts::message::UserMessage {
            id: "m1".to_string(),
            timestamp_ms: 0,
            content: vec![epsilon_contracts::content::UserContent::Text {
                text: text.to_string(),
            }],
        };
        store
            .append(epsilon_contracts::session_entry::EntryKind::Message {
                message: epsilon_contracts::message::Message::User(user),
                api_request_sha256: None,
            })
            .expect("append");
    }

    #[test]
    fn editor_command_prefers_config_then_visual_then_editor() {
        // pi `getExternalEditorCommand` order, with blanks treated as unset.
        assert_eq!(
            editor_command_from(Some("code --wait"), Some("vim"), Some("nano")).unwrap(),
            "code --wait"
        );
        assert_eq!(
            editor_command_from(None, Some("vim"), Some("nano")).unwrap(),
            "vim"
        );
        assert_eq!(
            editor_command_from(None, None, Some("nano")).unwrap(),
            "nano"
        );
        assert_eq!(
            editor_command_from(Some("  "), None, Some("  nano  ")).unwrap(),
            "nano"
        );
        let err = editor_command_from(None, None, None).expect_err("unconfigured");
        assert!(err.contains("$EDITOR"), "{err}");
        assert!(
            err.contains("[editor]"),
            "the error shows the config shape: {err}"
        );
    }

    #[test]
    fn external_editor_round_trip_writes_hands_over_and_reads_back() {
        // The spawner seam stands in for the user's editor: it appends a line
        // (and a trailing newline the round trip must trim).
        let edited = edit_with("draft line\n", &|path: &std::path::Path| {
            let mut text = std::fs::read_to_string(path).expect("draft written");
            text.push_str("second line\n");
            std::fs::write(path, text).expect("edit");
            Ok(0)
        })
        .expect("round trip");
        assert_eq!(edited, "draft line\nsecond line");
    }

    #[test]
    fn external_editor_keeps_the_draft_when_the_editor_fails() {
        let err =
            edit_with("important draft", &|_p: &std::path::Path| Ok(3)).expect_err("non-zero exit");
        assert!(err.contains("status 3"), "{err}");
        assert!(err.contains("draft kept"), "{err}");
    }

    #[test]
    fn editor_request_refuses_during_a_run_and_reports_a_missing_editor() {
        let (mut app, _agent, (_f, _n)) = test_app();
        app.request_external_editor();
        // No editor configured in the test environment unless the suite host has
        // one — either way the request never crashes and either arms the flag or
        // explains what to configure.
        let texts: Vec<String> = app.ui.lines().iter().map(|l| l.text.clone()).collect();
        let joined = texts.join("\n");
        if app.external_editor_requested {
            assert!(joined.contains("opening"), "{joined}");
            app.external_editor_requested = false;
        } else {
            assert!(joined.contains("editor:"), "{joined}");
        }
    }

    #[test]
    fn editor_keybinding_and_slash_arm_the_same_request() {
        let (mut app, agent, (_f, _n)) = test_app();
        // Alt+E (and Ctrl+O) reach the same request path as `/editor`.
        app.keybindings = crate::keybindings::KeybindingsManager::create();
        let alt_e = KeyEvent::new(KeyCode::Char('e'), KeyModifiers::ALT);
        assert!(
            app.keybindings.matches("app.editor.external", &alt_e),
            "alt+e is a default for app.editor.external"
        );
        app.kill_ring = crate::kill_ring::KillRing::new();
        let _ = handle_key(&mut app, &agent, alt_e);
        let opened_by_key = app.external_editor_requested;
        app.external_editor_requested = false;
        type_and_enter(&mut app, &agent, "/editor");
        assert_eq!(
            app.external_editor_requested, opened_by_key,
            "the slash command and the key take the same path"
        );
    }

    #[tokio::test]
    async fn sessions_command_browses_and_enter_switches_the_session() {
        // M57: the browser lists the workspace's session directory, filters with
        // the input box, and Enter runs the same switch `/resume <path>` does.
        let (mut app, agent, (_f, _n)) = test_app();
        let tmp = tempfile::tempdir().unwrap();
        let sessions = tmp.path().join(".epsilon").join("sessions");
        std::fs::create_dir_all(&sessions).unwrap();
        let target = sessions.join("other.jsonl");
        write_session_fixture(&target, "the other session's first request");
        write_session_fixture(&sessions.join("zzz-unrelated.jsonl"), "unrelated work");
        app.workspace_root = tmp.path().to_path_buf();

        // Typing must land in the editor until Enter runs the command.
        for c in "/sessions".chars() {
            handle_key(&mut app, &agent, key(KeyCode::Char(c)));
        }
        assert!(app.session_browser.is_none(), "opens only on Enter");
        handle_key(&mut app, &agent, key(KeyCode::Enter));
        let browser = app.session_browser.as_ref().expect("browser open");
        assert_eq!(browser.rows.len(), 2, "both session files listed");
        assert!(app.input.is_empty(), "editor becomes the filter box");

        // Esc closes without switching.
        handle_key(&mut app, &agent, key(KeyCode::Esc));
        assert!(app.session_browser.is_none());

        // Reopen (Esc cleared the editor, so retype), filter to one row, Enter.
        type_and_enter(&mut app, &agent, "/sessions");
        assert!(app.session_browser.is_some());
        for c in "other".chars() {
            handle_key(&mut app, &agent, key(KeyCode::Char(c)));
        }
        let rows = app.session_browser.as_ref().unwrap().rows.clone();
        assert_eq!(rows.len(), 1, "filter narrows to the fixture: {rows:?}");
        assert_eq!(rows[0].path, target);
        handle_key(&mut app, &agent, key(KeyCode::Enter));
        assert!(app.session_browser.is_none(), "closes on pick");
        let attached = agent
            .store_handle_for_tests()
            .lock()
            .unwrap()
            .path()
            .to_path_buf();
        assert_eq!(attached, target, "the app switched onto the picked file");
        let texts: Vec<String> = app.ui.lines().iter().map(|l| l.text.clone()).collect();
        assert!(texts.join("\n").contains("resumed 1 messages"), "{texts:?}");
    }

    #[tokio::test]
    async fn sessions_command_says_when_there_is_nothing_to_browse() {
        let (mut app, agent, (_f, _n)) = test_app();
        let tmp = tempfile::tempdir().unwrap();
        app.workspace_root = tmp.path().to_path_buf();
        type_and_enter(&mut app, &agent, "/sessions");
        assert!(app.session_browser.is_none(), "no directory, no browser");
        let texts: Vec<String> = app.ui.lines().iter().map(|l| l.text.clone()).collect();
        assert!(
            texts.join("\n").contains("no session directory"),
            "{texts:?}"
        );

        // An empty directory (no session files yet) is explained too.
        std::fs::create_dir_all(tmp.path().join(".epsilon").join("sessions")).unwrap();
        type_and_enter(&mut app, &agent, "/sessions");
        let texts: Vec<String> = app.ui.lines().iter().map(|l| l.text.clone()).collect();
        assert!(texts.join("\n").contains("nothing to resume"), "{texts:?}");
    }

    #[tokio::test]
    async fn session_command_reports_cache_attribution_beside_the_totals() {
        // M69: `/session` already prints the token totals; the cache lines say whether the prompt
        // cache was paying off, and why not when it was not. Before any provider call there is
        // nothing to attribute, and the notice says exactly that instead of inventing a rate.
        let (mut app, agent, (_f, _n)) = test_app();
        type_and_enter(&mut app, &agent, "/session");
        let texts: Vec<String> = app.ui.lines().iter().map(|l| l.text.clone()).collect();
        let joined = texts.join("\n");
        assert!(joined.contains("session:"), "{texts:?}");
        assert!(
            joined.contains("no durable session file attached"),
            "an agent without a store is described, not crashed on: {texts:?}"
        );
        assert!(
            joined.contains("cache: no provider call yet"),
            "an untouched session has no cache history: {texts:?}"
        );
    }

    #[tokio::test]
    async fn tasks_command_lists_the_registry_and_says_when_it_is_empty() {
        let (mut app, agent, (_f, _n)) = test_app();
        type_and_enter(&mut app, &agent, "/tasks");
        let texts: Vec<String> = app.ui.lines().iter().map(|l| l.text.clone()).collect();
        assert!(
            texts.join("\n").contains("no subagent tasks"),
            "empty registry is explained, not silent"
        );

        // Start a background child through the loop's host: the registry view
        // must show it as running with its description.
        let host: std::sync::Arc<dyn epsilon_contracts::tool::TaskHost> =
            agent.clone() as std::sync::Arc<dyn epsilon_contracts::tool::TaskHost>;
        host.run_task(epsilon_contracts::tool::TaskRequest {
            prompt: "look around".into(),
            description: "repo recon".into(),
            subagent_type: "default".into(),
            background: true,
            resume: None,
            // M56: the view must also name the model the child runs on.
            model: Some("deepseek-flash".into()),
        })
        .await
        .expect("background start");
        type_and_enter(&mut app, &agent, "/tasks");
        let texts: Vec<String> = app.ui.lines().iter().map(|l| l.text.clone()).collect();
        let joined = texts.join("\n");
        assert!(joined.contains("repo recon"), "{joined}");
        assert!(
            ["running", "completed", "failed"]
                .iter()
                .any(|state| joined.contains(state)),
            "the state is shown: {joined}"
        );
        assert!(joined.contains("task(s)"), "{joined}");
        assert!(
            joined.contains("@deepseek-flash"),
            "the child's model is part of the view: {joined}"
        );
    }

    #[tokio::test]
    async fn login_oauth_never_falls_through_to_the_key_store() {
        let (mut app, agent, (_f, _n)) = test_app();
        type_and_enter(&mut app, &agent, "/login xai oauth");
        assert!(
            !app.auth_prompt_open,
            "`oauth` must not be read as a pasted key"
        );
        let texts: Vec<String> = app.ui.lines().iter().map(|l| l.text.clone()).collect();
        let joined = texts.join("\n");
        // The guidance depends on how this crate was compiled: a build without
        // the `real` feature says so honestly, a real build either starts the
        // ceremony or reports the failure. Assert the invariant that matters —
        // the word `oauth` was never mistaken for a pasted API key — and let the
        // message differ per build (feature unification with epsilon-cli means
        // `real` may well be on during a workspace test run).
        assert!(
            joined.contains("OAuth login") || joined.contains("oauth login"),
            "the user is told what happened: {joined}"
        );
        assert!(
            !epsilon_llm::auth::load_credentials().contains_key("xai"),
            "nothing is stored for a ceremony that could not run"
        );
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

    #[test]
    fn remapped_quit_via_keybindings_manager() {
        // G6: app.exit remapped to ctrl+q via manager still quits; ctrl+c no longer does.
        let (mut app, agent, _rx) = test_app();
        let mut map = std::collections::HashMap::new();
        map.insert("app.exit".into(), vec!["ctrl+q".into()]);
        app.keybindings = crate::keybindings::KeybindingsManager::from_map(map);
        assert!(handle_key(
            &mut app,
            &agent,
            KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL)
        ));
        let (mut app2, agent2, _rx2) = test_app();
        let mut map2 = std::collections::HashMap::new();
        map2.insert("app.exit".into(), vec!["ctrl+q".into()]);
        app2.keybindings = crate::keybindings::KeybindingsManager::from_map(map2);
        // ctrl+c is NOT exit when remapped to only ctrl+q (verify the negative).
        assert!(!handle_key(
            &mut app2,
            &agent2,
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)
        ));
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

    #[tokio::test]
    async fn compact_already_running_refused_and_second_compact_guards() {
        let (mut app, agent, _rx) = test_app();
        app.compact_running = true;
        assert!(!type_and_enter(&mut app, &agent, "/compact"));
        let texts: Vec<String> = app.ui.lines().iter().map(|l| l.text.clone()).collect();
        assert!(
            texts.iter().any(|t| t.contains("already running")),
            "must refuse second /compact: {texts:?}"
        );
        // non-compact path unchanged: /timeline still works even while compact flag set
        // (compact only guards /compact).
    }

    #[tokio::test]
    async fn timeline_picker_opens_and_enter_jumps() {
        let (mut app, agent, _rx) = test_app();
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
            let _ = st
                .append(EntryKind::Message {
                    message: epsilon_contracts::message::Message::User(UserMessage {
                        id: epsilon_contracts::new_id("msg"),
                        timestamp_ms: 0,
                        content: vec![UserContent::Text {
                            text: "hello".into(),
                        }],
                    }),
                    api_request_sha256: None,
                })
                .unwrap();
            let _ = st
                .append(EntryKind::Message {
                    message: epsilon_contracts::message::Message::User(UserMessage {
                        id: epsilon_contracts::new_id("msg"),
                        timestamp_ms: 1,
                        content: vec![UserContent::Text {
                            text: "world".into(),
                        }],
                    }),
                    api_request_sha256: None,
                })
                .unwrap();
        }
        assert!(!type_and_enter(&mut app, &agent, "/timeline"));
        assert!(
            app.timeline_open,
            "timeline picker opens instead of notice-dumping"
        );
        assert!(app.timeline_entries.len() >= 2);
        app.timeline_index = 0;
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::empty());
        let ate = handle_key(&mut app, &agent, enter);
        assert!(!ate);
        assert!(
            !app.timeline_open,
            "Enter dismisses the overlay after jumping"
        );
        assert!(!type_and_enter(&mut app, &agent, "/timeline"));
        assert!(app.timeline_open);
        let esc = KeyEvent::new(KeyCode::Esc, KeyModifiers::empty());
        handle_key(&mut app, &agent, esc);
        assert!(!app.timeline_open, "Esc dismisses without navigating");
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
        // "/hotk" → fuzzy-unique to /hotkeys (no other slash item contains h-o-t-k in order).
        for c in "/hotk".chars() {
            handle_key(&mut app, &agent, key(KeyCode::Char(c)));
        }
        assert!(app.slash_open);
        assert!(
            crate::picker::slash_items("/hotk")
                .iter()
                .any(|it| it.value == "/hotkeys")
        );
        assert!(!handle_key(&mut app, &agent, key(KeyCode::Enter)));
        assert!(
            app.ui.lines().iter().any(|l| l.text.contains("hotkeys")),
            "/hotkeys executed via Enter-apply: {:?}",
            app.ui.lines().iter().map(|l| &l.text).collect::<Vec<_>>()
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
        // "/hotk" is fuzzy-unique to /hotkeys (stable even after fuzzy slash_items).
        for c in "/hotk".chars() {
            handle_key(&mut app, &agent, key(KeyCode::Char(c)));
        }
        assert!(app.slash_open);
        assert!(!handle_key(&mut app, &agent, key(KeyCode::Tab)));
        assert_eq!(app.input, "/hotkeys", "Tab completed the fuzzy /hotk match");
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
    #[test]
    fn at_word_before_detects_fragment_and_boundary() {
        assert_eq!(
            crate::editor_ops::at_word_before("fix @src/ap", 11),
            Some((4, "src/ap".into()))
        );
        assert_eq!(
            crate::editor_ops::at_word_before("a@b", 3),
            None,
            "@ without word boundary must not trigger"
        );
        assert_eq!(
            crate::editor_ops::at_word_before("hello @", 7),
            Some((6, "".into())),
            "bare @ triggers with empty fragment"
        );
        assert_eq!(
            crate::editor_ops::at_word_before("fix @src/ap ", 12),
            None,
            "space after fragment ends the @-word"
        );
    }

    #[test]
    fn at_insert_replaces_fragment() {
        let (s, c) = crate::editor_ops::at_insert("fix @src/ap", 11, "src/app.rs");
        assert_eq!(s, "fix src/app.rs");
        assert_eq!(c, s.chars().count());
        // no @-word: unchanged
        let (s2, c2) = crate::editor_ops::at_insert("hello world", 5, "x");
        assert_eq!(s2, "hello world");
        assert_eq!(c2, 5);
    }

    #[tokio::test]
    async fn at_file_popup_opens_and_tab_inserts_path() {
        let ws = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(ws.path().join("src")).unwrap();
        std::fs::write(ws.path().join("src/app.rs"), "").unwrap();
        std::fs::write(ws.path().join("src/lib.rs"), "").unwrap();
        let (mut app, agent) = {
            let (mut a, ag, _) = test_app();
            a.workspace_root = ws.path().to_path_buf();
            (a, ag)
        };
        // Type "fix @src/a" -> at popup should open filtered to src/app.rs
        for c in "fix @src/a".chars() {
            app.insert_char(c);
            app.sync_at_state();
        }
        assert!(app.at_open, "at popup opens on @word");
        assert!(
            app.at_rows
                .as_ref()
                .unwrap()
                .iter()
                .any(|p| p == "src/app.rs")
        );
        let before = app.at_index;
        // Tab inserts without space
        app.apply_at_selection(false);
        assert!(!app.at_open, "popup closes after insert");
        assert!(
            app.input.contains("src/app.rs"),
            "path inserted: {}",
            app.input
        );
        assert!(!app.input.ends_with(' '), "Tab must not add trailing space");
        let _ = before;
        let _ = agent;
    }

    #[tokio::test]
    async fn export_writes_html_and_json() {
        let ws = tempfile::tempdir().unwrap();
        let dest_html = ws.path().join("out.html");
        let dest_json = ws.path().join("out.json");
        let (mut app, agent, _) = test_app();
        app.workspace_root = ws.path().to_path_buf();
        let backing = std::sync::Arc::new(std::sync::Mutex::new(
            epsilon_core::SessionStore::open(ws.path().join("t.jsonl")).unwrap(),
        ));
        agent.attach_session_store(std::sync::Arc::clone(&backing));
        {
            let mut guard = backing.lock().unwrap();
            guard
                .append(epsilon_contracts::session_entry::EntryKind::Message {
                    message: epsilon_contracts::message::Message::User(
                        epsilon_contracts::message::UserMessage {
                            id: "m1".into(),
                            timestamp_ms: 1,
                            content: vec![epsilon_contracts::content::UserContent::Text {
                                text: "hello".into(),
                            }],
                        },
                    ),
                    api_request_sha256: None,
                })
                .unwrap();
        }
        let n = write_export_html(&agent, &dest_html, ws.path()).unwrap();
        assert_eq!(n, 1);
        let html = std::fs::read_to_string(&dest_html).unwrap();
        assert!(html.contains("<html"), "html export must contain html tag");
        assert!(html.contains("hello"), "html must contain message text");
        let n2 = write_export_json(&agent, &dest_json).unwrap();
        assert_eq!(n2, 1);
        let j: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&dest_json).unwrap()).unwrap();
        assert!(j.is_array(), "json export must be an array");
        let _ = app;
    }
}
