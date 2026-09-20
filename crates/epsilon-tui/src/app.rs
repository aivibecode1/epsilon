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
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

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

// ---- OAuth login plumbing (M52) ---------------------------------------
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

/// Try to write `text` to the system clipboard (arboard). Returns Err on
/// headless / permission failure so the caller can surface it as a notice.
fn clipboard_set(text: &str) -> Result<(), String> {
    let mut cb = arboard::Clipboard::new().map_err(|e| format!("{e}"))?;
    cb.set_text(text.to_string()).map_err(|e| format!("{e}"))
}

// The keyboard model lives in `app/keys.rs`: one handler per owner, in
// precedence order. It is a child of this module, so it still reaches
// `App` and its private methods.
mod keys;
// Re-exported so the event loop, the command handlers and the tests keep
// calling these by name instead of through the module path. The precedence
// they encode is documented in `keys.rs`.
pub(crate) use keys::handle_term_event;

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
        let mut lines: Vec<Line<'static>> = vec![Line::from(crate::ui::layout::too_small_message(
            area.width,
            area.height,
        ))];
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
    let (draft_rows, caret_row, caret_col) =
        composer_rows(&app.input, app.cursor_chars, draft_width);
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
    use crate::ui::overlay::{self, Panel, Width};

    // M16c two-stage: stage 1 lists providers; stage 2 that provider's models.
    if let Some(provider) = &app.picker_provider.clone() {
        draw_picker_stage2(f, app, outer, provider);
        return;
    }
    let rows = crate::picker::build_provider_rows(&app.model_id, &app.customs)
        .into_iter()
        .map(|p| (p.id, p.model_count, p.auth_ready, p.is_active))
        .collect::<Vec<_>>();
    let width = Width::PANEL.resolve(outer);
    let height = 15u16.min(outer.height.saturating_sub(2));
    let title = format!("models — filter: '{}'", app.input);
    let panel = Panel {
        theme: &app.theme,
        // Top-aligned rather than centred, like the palette: the picker opens
        // where the composer is and stays put while the filter changes.
        area: Rect {
            y: outer.y + 1,
            ..overlay::centered(outer, width, height)
        },
        title: &title,
        focused: true,
    };
    let inner = panel.open(f);

    // M15 viewport fix: pi's centered scroll window — render only the visible slice
    // so long lists stay navigable (previously rows past the box were unreachable).
    let capacity = inner.height.saturating_sub(2).max(1) as usize; // blank line + hint footer
    let start = crate::scroll::scroll_start(rows.len(), app.picker_index, capacity);
    let end = (start + capacity).min(rows.len());

    let mut lines: Vec<Line<'static>> = Vec::new();
    if rows.is_empty() {
        lines.push(Line::from(Span::styled("no providers", app.theme.dim())));
    }
    for (i, (id, count, auth_ready, is_active)) in
        rows.iter().enumerate().skip(start).take(end - start)
    {
        let selected = i == app.picker_index;
        // The active-provider mark and the auth badge are leading columns of the
        // label; `list_row` owns the selection bar and right-aligns the count.
        let marker = if *is_active { "● " } else { "  " };
        let auth = if *auth_ready { "🔑" } else { "· " };
        lines.push(overlay::list_row(
            &app.theme,
            selected,
            &format!("{marker}{auth} {id}"),
            Some(format!("{count} models")).as_deref(),
            inner.width as usize,
        ));
    }

    lines.push(Line::from(""));
    lines.push(overlay::hint_row(
        &app.theme,
        &[("↑↓", "move"), ("enter", "browse models"), ("esc", "close")],
    ));
    f.render_widget(Paragraph::new(lines), inner);
}

/// Stage-2 of the two-stage picker: one provider's models with fuzzy filter.
/// M57: the `/sessions` overlay. Rows are session files from the workspace's
/// `.epsilon/sessions`, newest first; the current one is marked `●`.
fn draw_session_browser(f: &mut Frame<'_>, app: &mut App, outer: Rect) {
    use crate::ui::overlay::{self, Panel, Width};

    let Some(browser) = app.session_browser.as_ref() else {
        return;
    };
    let width = Width::PANEL.resolve(outer);
    let height = 15u16.min(outer.height.saturating_sub(2));
    let title = format!("sessions — filter '{}'", app.input);
    let panel = Panel {
        theme: &app.theme,
        // Top-aligned like the palette and the picker: the browser opens at the
        // composer's edge and does not jump as the filter narrows the list.
        area: Rect {
            y: outer.y + 1,
            ..overlay::centered(outer, width, height)
        },
        title: &title,
        focused: true,
    };
    let inner = panel.open(f);

    // Same centered scroll window as the pickers: only the visible slice is built.
    let capacity = inner.height.saturating_sub(1).max(1) as usize; // footer
    let start = crate::scroll::scroll_start(browser.rows.len(), browser.index, capacity);
    let end = (start + capacity).min(browser.rows.len());

    let mut lines: Vec<Line<'static>> = Vec::new();
    if browser.rows.is_empty() {
        lines.push(Line::from(Span::styled(
            "no matches — Backspace to widen, Esc closes",
            app.theme.dim(),
        )));
    }
    for (i, row) in browser
        .rows
        .iter()
        .enumerate()
        .skip(start)
        .take(end - start)
    {
        let selected = i == browser.index;
        // The current-session mark leads the title column; the age/size meta
        // stays the right-aligned column it was.
        let marker = if row.is_current { "● " } else { "  " };
        lines.push(overlay::list_row(
            &app.theme,
            selected,
            &format!("{marker}{}", row.title),
            Some(row.meta.as_str()),
            inner.width as usize,
        ));
    }
    // The position counter and the key hints share the footer line: a `hint_row`
    // carries the hints, and the counter stays where the reader looks for it.
    let mut footer = vec![Span::styled(
        format!("{} of {}", browser.index + 1, browser.rows.len()),
        app.theme.dim(),
    )];
    footer.push(Span::styled(" · ", app.theme.faint()));
    footer.extend(overlay::hint_row(&app.theme, &[("enter", "resumes"), ("esc", "closes")]).spans);
    lines.push(Line::from(footer));
    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_picker_stage2(f: &mut Frame<'_>, app: &App, outer: Rect, provider: &str) {
    use crate::ui::overlay::{self, Panel, Width};

    let rows = crate::picker::build_models_for_provider(
        provider,
        &app.input,
        &app.model_id,
        &app.live_cache,
        &app.customs,
    );
    let width = Width::PANEL.resolve(outer);
    let height = 15u16.min(outer.height.saturating_sub(2));
    let title = format!("models — {provider}: filter '{}'", app.input);
    let panel = Panel {
        theme: &app.theme,
        // Top-aligned like stage 1, so stepping between the two stages does not
        // move the frame under the reader.
        area: Rect {
            y: outer.y + 1,
            ..overlay::centered(outer, width, height)
        },
        title: &title,
        focused: true,
    };
    let inner = panel.open(f);

    let capacity = inner.height.saturating_sub(2).max(1) as usize; // blank line + hint footer
    let start = crate::scroll::scroll_start(rows.len(), app.picker_index, capacity);
    let end = (start + capacity).min(rows.len());

    let mut lines: Vec<Line<'static>> = Vec::new();
    if rows.is_empty() {
        lines.push(Line::from(Span::styled(
            "no matches — Backspace to widen, Esc closes",
            app.theme.dim(),
        )));
    }
    for (i, row) in rows.iter().enumerate().skip(start).take(end - start) {
        let selected = i == app.picker_index;
        // The active-model mark, the auth badge and the favorite star are leading
        // columns of the label; the model id stays the right-aligned meta column.
        let marker = if row.id == app.model_id { "● " } else { "  " };
        let auth = if row.auth_ready { "🔑" } else { "· " };
        // M23: ★ marks favorites (opencode's star column); Ctrl+F toggles.
        let star = if row.is_favorite { "★" } else { " " };
        lines.push(overlay::list_row(
            &app.theme,
            selected,
            &format!("{marker}{auth} {star} {}", row.label),
            Some(row.id.as_str()),
            inner.width as usize,
        ));
    }

    lines.push(Line::from(""));
    lines.push(overlay::hint_row(
        &app.theme,
        &[
            ("type", "filters (fuzzy)"),
            ("enter", "apply"),
            ("backspace/esc", "back"),
        ],
    ));
    f.render_widget(Paragraph::new(lines), inner);
}

/// M16a virtualized draw: reconcile the render cache (renders ONLY unseen lines),
/// then materialize just the visible window. Per-frame cost is O(visible), not
/// O(transcript).
fn draw_transcript(f: &mut Frame<'_>, app: &mut App, area: Rect) {
    let width = area.width.saturating_sub(1) as usize; // 1-col slack, no h-scroll
    app.render_cache
        .reconcile(app.ui.lines(), width, app.prefs.expand_outputs, &app.theme);
    let total = app.render_cache.total_rows();
    let visible = area.height as usize;

    // Keep thinking/prose and explicit newlines separate; slice the combined
    // history + live tail in screen rows, including tails taller than the viewport.
    let mut tail_rows = Vec::new();
    for (text, style) in [
        (app.ui.streaming_thinking(), app.theme.thinking()),
        (app.ui.streaming_text(), app.theme.assistant()),
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

// The tests live beside this file: `app/tests.rs` and
// `app/composer_viewport_tests.rs`, pulled in below. They stay part of this
// module so `super::*` still reaches private items.
#[cfg(test)]
mod composer_viewport_tests;

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
    let panel = crate::ui::overlay::Panel {
        theme: &app.theme,
        area,
        title: "files",
        focused: true,
    };
    let inner = panel.open(f);
    let theme = &app.theme;
    let mut lines: Vec<ratatui::text::Line<'static>> = rows[start..end]
        .iter()
        .enumerate()
        .map(|(index, path)| {
            // Trim by characters, never by bytes: a path with one non-ASCII
            // component would otherwise panic on a boundary that happens to land
            // inside a multi-byte character.
            let shown = crate::ui::format::format_path(
                std::path::Path::new(path),
                None,
                inner.width.saturating_sub(6) as usize,
            );
            crate::ui::overlay::list_row(
                theme,
                start + index == app.at_index,
                &shown,
                None,
                inner.width as usize,
            )
        })
        .collect();
    lines.push(crate::ui::overlay::hint_row(
        theme,
        &[
            ("tab", "insert"),
            ("enter", "insert + space"),
            ("esc", "close"),
        ],
    ));
    f.render_widget(ratatui::widgets::Paragraph::new(lines), inner);
}

fn draw_timeline(f: &mut Frame<'_>, app: &App, outer: Rect) {
    use crate::ui::overlay::{self, Panel, Width};

    if !app.timeline_open || app.timeline_entries.is_empty() {
        return;
    }
    let total = app.timeline_entries.len();
    let capacity = 12usize.min(outer.height.saturating_sub(6).max(1) as usize);
    let start = crate::scroll::scroll_start(total, app.timeline_index, capacity);
    let end = (start + capacity).min(total);
    // The timeline is a list panel like the others, so it takes the panel width;
    // the height still follows the visible row count.
    let width = Width::PANEL.resolve(outer);
    let height = (capacity as u16 + 4).min(outer.height.saturating_sub(2));
    let panel = Panel {
        theme: &app.theme,
        area: overlay::centered(outer, width, height),
        title: "timeline",
        focused: true,
    };
    let inner = panel.open(f);

    // A row is truncated to what the frame can actually show: the selection bar
    // and its gutter take two columns of the inner width.
    let budget = (inner.width as usize).saturating_sub(2);
    let mut lines: Vec<Line<'static>> = Vec::new();
    for (i, (seq, tag, snippet)) in app
        .timeline_entries
        .iter()
        .enumerate()
        .skip(start)
        .take(end - start)
    {
        let raw = format!("#{seq} [{tag}] {snippet}");
        let display = if raw.len() > budget {
            let w = budget.saturating_sub(3);
            format!("{}...", &raw[..w.min(raw.len())])
        } else {
            raw
        };
        lines.push(overlay::list_row(
            &app.theme,
            i == app.timeline_index,
            &display,
            None,
            inner.width as usize,
        ));
    }
    lines.push(Line::from(""));
    // The position counter and the key hints share the footer line: a `hint_row`
    // carries the hints, and the counter stays where the reader looks for it.
    let mut footer = vec![Span::styled(
        format!("{}/{}", app.timeline_index + 1, total),
        app.theme.dim(),
    )];
    footer.push(Span::styled(" · ", app.theme.faint()));
    footer.extend(
        overlay::hint_row(
            &app.theme,
            &[("↑↓", "move"), ("enter", "jump"), ("esc", "close")],
        )
        .spans,
    );
    lines.push(Line::from(footer));
    f.render_widget(Paragraph::new(lines), inner);
}

/// The slash-command popup, anchored directly above the composer.
///
/// Anchored rather than centred on purpose: it is a completion list for what the
/// user is *currently typing*, so it belongs next to the text, and a centred
/// panel would pull the eye away from the caret on every keystroke.
fn draw_slash_autocomplete(f: &mut Frame<'_>, app: &App, transcript_area: Rect) {
    use crate::ui::overlay::{self, Panel};

    let candidates = crate::picker::slash_items(&app.input);
    if !app.slash_open || candidates.is_empty() {
        return;
    }
    let total = candidates.len();
    // Eight visible rows, then the list scrolls: taller than that and the popup
    // covers the conversation it is helping with.
    let capacity = 8usize.min(total);
    let start = crate::scroll::scroll_start(total, app.slash_index, capacity);
    let height = capacity as u16 + 1 + 2; // rows + hint + frame
    let area = Rect {
        x: transcript_area.x,
        y: transcript_area.bottom().saturating_sub(height),
        width: transcript_area.width.min(56),
        height,
    };
    let panel = Panel {
        theme: &app.theme,
        area,
        title: "commands",
        focused: true,
    };
    let inner = panel.open(f);

    let mut lines: Vec<Line<'static>> = candidates[start..(start + capacity).min(total)]
        .iter()
        .enumerate()
        .map(|(offset, item)| {
            overlay::list_row(
                &app.theme,
                start + offset == app.slash_index,
                item.value,
                // The description is the hint column; `list_row` right-aligns it,
                // and an empty one means "no hint" rather than a blank column.
                Some(item.description).filter(|d| !d.is_empty()),
                inner.width as usize,
            )
        })
        .collect();
    lines.push(overlay::hint_row(
        &app.theme,
        &[("↑↓", "pick"), ("tab", "apply"), ("esc", "dismiss")],
    ));
    f.render_widget(Paragraph::new(lines), inner);
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
/// lives in `app.auth_key` but rendering shows one dot per character so the
/// secret never appears in the transcript, in a screenshot, or in a scrollback
/// buffer.
fn draw_auth_prompt(f: &mut Frame<'_>, app: &App, outer: Rect) {
    use crate::ui::overlay::{self, Panel, Width};

    let width = Width::PROMPT.resolve(outer);
    let height = 8u16.min(outer.height.saturating_sub(2)).max(5);
    let oauth_mode = app.oauth_code_tx.is_some();
    let title = if oauth_mode {
        format!("OAuth login · {}", app.auth_provider)
    } else {
        format!("API key · {}", app.auth_provider)
    };
    let panel = Panel {
        theme: &app.theme,
        area: overlay::centered(outer, width, height),
        title: &title,
        focused: true,
    };
    let inner = panel.open(f);
    let theme = &app.theme;

    let masked_len = app.auth_key.chars().count();
    let masked: String = theme.glyphs.bullet.repeat(masked_len);
    let hint = match (oauth_mode, masked_len) {
        (true, 0) => "paste the redirect URL or the bare code — Ctrl+V works, Enter sends",
        (true, _) => "Enter sends · Esc cancels",
        (false, 0) => "paste your key — it stays masked, Enter saves",
        (false, _) => "Enter saves · Esc cancels",
    };

    let body = vec![
        Line::from(Span::styled(
            if oauth_mode {
                "hidden while typing"
            } else {
                "the key is never shown, here or in the transcript"
            },
            theme.faint(),
        )),
        Line::from(Span::styled(masked, theme.warn())),
        Line::from(""),
        Line::from(Span::styled(hint, theme.dim())),
    ];
    f.render_widget(Paragraph::new(body), inner);

    // The caret is placed inside the masked field so a paste lands visibly.
    // Column arithmetic is in display cells, and the field is the second row of
    // the body (the first is the explanatory line).
    let cursor_col = app
        .auth_key_cursor
        .min(masked_len)
        .min(inner.width.saturating_sub(1) as usize);
    let x = inner.x + cursor_col as u16;
    let y = inner.y + 1;
    if x < inner.x + inner.width && y < inner.y + inner.height {
        f.set_cursor_position((x, y));
    }
}

/// M32b: provider picker for bare `/login` — opencode DialogProvider parity.
///
/// Renders the provider list filtered by the current `input`. Selecting one
/// opens the masked API-key prompt; Esc closes. Filter + list reuse the
/// existing `/models` provider rows, with auth badge and active mark so a
/// first-run user can see at a glance what is wired up.
fn draw_auth_picker(f: &mut Frame<'_>, app: &App, outer: Rect) {
    use crate::ui::overlay::{self, Panel, Width};
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
    let width = Width::PANEL.resolve(outer);
    let height = 15u16.min(outer.height.saturating_sub(2));
    let panel = Panel {
        theme: &app.theme,
        // Top-aligned like the model picker: the two are the same list at
        // different moments, and a panel that moves between them is disorienting.
        area: Rect {
            y: outer.y + 1,
            ..overlay::centered(outer, width, height)
        },
        title: "connect — pick a provider",
        focused: true,
    };
    let inner = panel.open(f);
    let theme = &app.theme;
    let capacity = inner.height.saturating_sub(2).max(1) as usize;
    let start = crate::scroll::scroll_start(filtered.len(), app.auth_pick_index, capacity);
    let end = (start + capacity).min(filtered.len());
    let mut lines: Vec<Line<'static>> = Vec::new();
    if filtered.is_empty() {
        lines.push(Line::from(Span::styled(
            format!("no match for '{}' — type another provider id", app.input),
            theme.dim(),
        )));
    }
    for (index, (_, prow)) in filtered.iter().enumerate().skip(start).take(end - start) {
        // Auth state is a word, not only a colour: a key glyph is invisible on a
        // console that cannot render it, and "ready to use" is not a detail to
        // lose to a font.
        let auth = if prow.auth_ready { "key" } else { "—" };
        let oauth = if epsilon_llm::oauth_login::login_flow(&prow.id).is_some() {
            "oauth"
        } else {
            ""
        };
        let meta = format!("{auth} · {} models {oauth}", prow.model_count)
            .trim_end()
            .to_string();
        let label = if prow.is_active {
            format!("{} (active)", prow.id)
        } else {
            prow.id.clone()
        };
        lines.push(overlay::list_row(
            theme,
            index == app.auth_pick_index,
            &label,
            Some(&meta),
            inner.width as usize,
        ));
    }
    lines.push(Line::from(""));
    lines.push(overlay::hint_row(
        theme,
        &[
            ("type", "filter"),
            ("↑↓", "move"),
            ("enter", "paste key"),
            ("esc", "cancel"),
        ],
    ));
    f.render_widget(Paragraph::new(lines), inner);
}

#[cfg(test)]
mod tests;
