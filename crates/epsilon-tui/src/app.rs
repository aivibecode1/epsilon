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

use crate::state::{LineKind, UiState};

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
    /// Which palette to draw with (D8), already resolved by the caller: `--light`
    /// beats `EPSILON_THEME` beats `[tui] theme` in the config, and the frontend
    /// does not decide that precedence a second time. `None` means nothing was
    /// resolved, in which case [`crate::ui::theme::ThemeMode::detect`] reads the
    /// environment.
    ///
    /// Selected, never detected. There is no reliable way to ask a terminal
    /// whether its background is light — `COLORTERM` describes colour *depth*,
    /// and `OSC 11` needs a round-trip that may never come back inside a pipe,
    /// over SSH, or in `tmux` — so the answer is the user's to give.
    pub theme_mode: Option<crate::ui::theme::ThemeMode>,
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

/// The one line a permission dialog shows for a request's `detail`.
///
/// The dialog has a single row for "what is about to happen", and the answer a
/// reader needs is almost always one field of the JSON: the command, the path,
/// the URL. Printing the object instead costs the row and buries the answer —
/// `{"command":"rm -rf build"}` spends twelve columns on punctuation before the
/// part that decides the question, so on a 46-column panel the thing being
/// approved is the part that gets cut.
///
/// Preference order, not a schema: the first field present wins, and an object
/// with none of them falls back to the JSON. Nothing is invented for a missing
/// field, and the fallback is the raw object rather than a paraphrase, because
/// the value of this line is that it is the request.
fn summarize_detail(detail: &serde_json::Value) -> String {
    /// Fields that answer "what will this do", most specific first.
    const NAMED: [&str; 5] = ["command", "path", "file", "query", "url"];
    if let Some(object) = detail.as_object() {
        for key in NAMED {
            if let Some(value) = object.get(key).and_then(|v| v.as_str()) {
                return value.to_string();
            }
        }
    }
    if let Some(text) = detail.as_str() {
        return text.to_string();
    }
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
    /// D11: where the transcript's viewport sits, measured at the top of each
    /// frame so the footer's position badge and the rows beneath it agree.
    geometry: TranscriptGeometry,
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
    /// M126: per-card expansion, keyed by [`crate::state::TranscriptLine`] id.
    ///
    /// The global `prefs.expand_outputs` toggle survives beside it and still means
    /// **expand all** — a user who relies on it keeps it. These entries are the
    /// *exceptions* to it: a card the user opened while the global was off stays
    /// open, and a card they closed while it was on stays closed. That is why the
    /// map holds `false` as well as `true`; a set of "opened" ids could not
    /// express the second case and the key would silently re-expand it.
    card_open: Vec<(u64, bool)>,
    /// M126: line ids whose cached rows are stale — a card whose status or
    /// duration moved under a stable id (its result landed). Rebuilt on the next
    /// reconcile and then cleared, so a busy run re-renders the one card that
    /// changed rather than the conversation.
    card_restale: Vec<u64>,
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
    /// Cached ignore rules for `@` completion, with the time they were read.
    ///
    /// `IgnoreSet::load` searches the workspace for `.gitignore` files, which is not work
    /// a keystroke can pay for: typing `@Cargo` re-derives the popup once per character.
    /// Without this cache each character paid a full search, and in a workspace holding a
    /// large ignored tree the popup simply never appeared — measured in a real console,
    /// which is how the regression was found (M110). The branch cache above uses the same
    /// TTL shape for the same reason; the TTL is what bounds how long an edited
    /// `.gitignore` goes unnoticed.
    at_ignore: Option<(epsilon_core::ignore::IgnoreSet, std::time::Instant)>,
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
    /// An OSC 52 clipboard sequence waiting for the next frame boundary.
    ///
    /// It cannot be written where `/copy` runs: a key handler must not touch the
    /// terminal, and the sequence must not land in the middle of the frame the
    /// terminal is being handed — interleaved with ratatui's own diff, it is at
    /// best ignored and at worst a torn row. So it is parked here and written
    /// after `terminal.draw` returns, via the backend's own writer. See
    /// [`crate::clipboard`].
    osc_pending: Option<String>,
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

/// Lines describing what the last session resume repaired (M75) or kept without understanding —
/// empty when the session was both intact and entirely known.
fn repair_lines(agent: &Agent) -> Vec<String> {
    let mut lines: Vec<String> = agent
        .repair_report()
        .map(|r| {
            r.summary()
                .into_iter()
                .map(|s| format!("repair: {s}"))
                .collect()
        })
        .unwrap_or_default();
    // The one loss a transcript cannot show: a foreign entry kind is kept in the file and skipped
    // by the model view (M113), so the session reads as complete while something in it is
    // missing. The store owns the sentence (`SessionStore::preserved_unknown_notice`); this says
    // it wherever it already says what a resume had to do. No store attached — a plain `--tui`
    // run, a child agent — means nothing was read, hence nothing to report.
    if let Some(handle) = agent.session_store()
        && let Ok(store) = handle.lock()
        && let Some(notice) = store.preserved_unknown_notice()
    {
        lines.push(format!("preserved: {notice}"));
    }
    lines
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
            let b = crate::git::branch(&self.workspace_root);
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
    fn at_file_rows(&mut self, fragment: &str) -> Vec<String> {
        // The `@` list has to agree with the tools about which files exist: offering a path
        // the model's own read tool would refuse is a promise the harness cannot keep.
        // `epsilon_core::ignore` is that single answer (M99), and `epsilon-tui` already
        // depends on `epsilon-core`, so sharing it costs no new crate edge.
        //
        // What is still this module's own is the *traversal*, deliberately: it is budgeted
        // for typing latency (a few hundred rows, stop early) rather than for a tool call
        // that must see everything. One ignore answer, two walks with different budgets —
        // if a third consumer appears, the walk belongs in core beside `IgnoreSet`.
        //
        // The rules are cached, because this runs once per keystroke and loading them
        // searches the workspace. A stale cache for a few seconds is the price, and it is
        // the same price the branch cache above pays.
        const AT_IGNORE_TTL: std::time::Duration = std::time::Duration::from_secs(5);
        let fresh = matches!(&self.at_ignore, Some((_, at)) if at.elapsed() <= AT_IGNORE_TTL);
        if !fresh {
            self.at_ignore = Some((
                epsilon_core::ignore::IgnoreSet::load(
                    &self.workspace_root,
                    crate::at_files::AT_IGNORE_FILES,
                ),
                std::time::Instant::now(),
            ));
        }
        let ignore = &self
            .at_ignore
            .as_ref()
            .expect("the cache was just filled")
            .0;
        let mut budget = crate::at_files::AT_ROW_BUDGET;
        let files = crate::at_files::walk(&self.workspace_root, ignore, &mut budget);
        let needle = fragment.trim();
        if needle.is_empty() {
            let mut out = files;
            out.sort();
            out.truncate(crate::at_files::AT_ROW_BUDGET);
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

// The loop itself lives in `app/event.rs`. `run` above keeps the terminal
// lifecycle (init, bracketed paste, restore); the loop keeps everything
// between the first frame and the last.
mod event;
pub(crate) use event::drive;

/// Apply one event and maintain scroll discipline (pi scroll-view `followEnd`
/// semantics): a pinned view follows new tail rows; once the user has scrolled
/// up, appended lines shift the offset so the reading window stays FROZEN on
/// the same content instead of yanking back to the bottom mid-stream.
fn apply_with_scroll(app: &mut App, ev: &AgentEvent) {
    let before = app.ui.lines().len();
    app.ui.apply(ev);
    // A card settles *inside* its line: the result marks the call's own row
    // finished, so neither the row count nor its id changes and the cache would
    // happily keep serving "pending" for a call that has returned. Naming the
    // finished cards is how the cache learns they moved.
    app.restale_finished_cards();
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

/// Write any parked OSC 52 clipboard sequence and return what it was.
///
/// Called by the event loop immediately after `terminal.draw` returns — never
/// from inside a draw, and never from a key handler: the sequence has to travel
/// on the same stream as the frames, and interleaved with ratatui's own diff it
/// would at best be ignored. `out` is the writer that stream belongs to.
///
/// Returns the sequence so the caller can report what it sent, and so a test can
/// assert the exact bytes without owning a terminal.
pub(crate) fn flush_osc52<W: std::io::Write>(app: &mut App, out: &mut W) -> Option<String> {
    let sequence = app.osc_pending.take()?;
    // A failed write is not fatal and not silent-by-accident: `/copy` already
    // told the user which path it took, and a terminal that cannot be written to
    // is the same terminal that would have ignored the sequence.
    let _ = crate::clipboard::write_to(out, &sequence);
    Some(sequence)
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
/// Where the transcript's viewport sits on the frame being drawn (D11).
///
/// Computed **once per frame, before the chrome**, and that is the whole reason
/// this type exists. The footer has to say how far back a frozen view is, and the
/// only place that knows the answer is the transcript's own windowing — which
/// used to run *after* the footer had already been painted. Measuring first makes
/// the two read the same numbers in the same frame, instead of the footer
/// describing the frame before last.
///
/// It also keeps the cache reconciled exactly once per frame: a second reconcile
/// for the footer's benefit would be O(transcript), which is the cost the cache
/// exists to remove.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct TranscriptGeometry {
    /// The row the visible window starts at, in transcript rows.
    skip: usize,
    /// Rows the transcript holds: cached rows plus the in-flight streaming tail.
    total: usize,
    /// Rows the viewport can show. Zero means "not measured", which
    /// [`draw_transcript`] treats as nothing to draw.
    visible: usize,
}

impl TranscriptGeometry {
    /// How many rows sit below the visible window: what the footer's position
    /// badge reports.
    ///
    /// Counted rather than expressed as a percentage because every one of these
    /// rows is a row the reader must pass through to get back to live; see
    /// [`crate::ui::footer::View`] for why that is the useful quantity.
    fn rows_below(&self) -> usize {
        self.total
            .saturating_sub(self.skip)
            .saturating_sub(self.visible)
    }
}

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
        composer_rows(&app.input, app.cursor_chars, draft_width); // Before the chrome, and before the hints that describe it: measure the
    // transcript's own region, not the frame, so the footer's position badge and
    // the rows the body paints come from one arithmetic. The viewport height is
    // the transcript's, and using the frame's instead is how a "follow the tail"
    // view ends up four rows short of the tail it claims to be following.
    //
    // The split is the same one `shell::paint` will perform — same area, same
    // draft row count — and [`App::measure_transcript_window`] is the one place
    // that knows both, so the two cannot drift apart silently.
    let draft_count = draft_rows.len().max(1) as u16;
    app.geometry = app.measure_transcript_window(area, draft_count);
    // The theme is `Copy`, so the closure below can still take `&mut app`: the
    // alternative — holding a borrow of `app.theme` across the call — is what the
    // borrow checker refuses, and rightly, since the closure mutates the app.
    let theme = app.theme;
    let hint_values = app.hints();
    let shell = crate::ui::shell::Shell {
        theme: &theme,
        run: app.run_state(),
        metrics: app.metrics(),
        hints: hint_values,
        draft: draft_rows,
        caret: (caret_row, caret_col),
        composer_focused: !app.overlay_open(),
        transcript_empty: app.transcript_is_empty(),
    };
    let geometry = app.geometry;
    let painted = crate::ui::shell::paint(f, area, &shell, |f, transcript_area| {
        draw_transcript(f, app, transcript_area, geometry);
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

    // ---- M126: per-card expansion -------------------------------------------

    /// Whether one card renders its whole body.
    ///
    /// The user's own decision if they made one, the global toggle otherwise. The
    /// global toggle has to stay authoritative for cards nobody has touched, or
    /// "expand all" would only ever work on the cards that were already open.
    ///
    /// **This defers to [`crate::render_cache::RenderState::is_open`] rather than
    /// restating it.** The two answer the same question about the same card — one
    /// decides what is *drawn*, the other what a keypress *records* — and the
    /// whole per-card convention lives in their agreement: press `ctrl+x` twice
    /// and the card the first press opened must still be open after the second,
    /// which is only coherent while both halves read one rule. A second copy that
    /// agreed today is exactly the copy that would stop agreeing on the day
    /// somebody adds a third source of truth to one of them.
    fn card_is_open(&self, id: u64, has_body: bool) -> bool {
        crate::render_cache::RenderState {
            expand_all: self.prefs.expand_outputs,
            expanded: &self.card_open,
            // The key text is the renderer's business, not the rule's: it is
            // `None` here because nothing in this decision reads it, and a
            // resolved chord would only be a second thing that could disagree.
            more_key: None,
        }
        .is_open(id, has_body)
    }

    /// The card `Ctrl+X` acts on when it is not the global toggle's turn.
    ///
    /// The **newest** card with a body: the reader is at the bottom of a live
    /// transcript, so the most recent tool call is the one they just watched run.
    /// `None` when no card has a body, and then the key is purely the global
    /// toggle it always was.
    fn selected_card_id(&self) -> Option<u64> {
        self.ui
            .lines()
            .iter()
            .rev()
            .find(|l| l.card.as_ref().is_some_and(|c| c.has_body()))
            .map(|l| l.id)
    }

    /// Flip one card's own expansion state, remembering the decision.
    ///
    /// Marks the card's rows stale so exactly that card is rebuilt: the opened
    /// body is measured, wrapped and laid out against the *current* width, which a
    /// cached row from a narrower frame could not be.
    fn set_card_open(&mut self, id: u64, open: bool) {
        match self
            .card_open
            .iter_mut()
            .find(|(card_id, _)| *card_id == id)
        {
            Some(entry) => entry.1 = open,
            None => self.card_open.push((id, open)),
        }
        if !self.card_restale.contains(&id) {
            self.card_restale.push(id);
        }
    }

    /// The key text a capped card names in its `… N more lines (… to open)` row.
    ///
    /// Resolved from the live binding table here rather than inside the card
    /// renderer: a widget that read the keybindings manager would be a widget no
    /// test could aim, and resolving it is the whole point — a rebound
    /// `app.tools.expand` moves this text with it.
    fn expand_key(&self) -> Option<String> {
        self.keybindings.key_text("app.tools.expand")
    }

    /// Mark every finished card's rows as stale.
    ///
    /// Called by the event loop after each event is applied. A card's id is stable
    /// across its own status change — that is what lets the reader's open/closed
    /// choice survive the result landing — so the cache cannot notice the change
    /// from the id alone and has to be told. Listing every finished card is
    /// deliberate: the list is small, the rebuild happens once per card, and
    /// "which card changed" is not a question the reducer is asked to answer.
    pub(crate) fn restale_finished_cards(&mut self) {
        for line in self.ui.lines() {
            if line.card.as_ref().is_some_and(|c| c.ended_at_ms.is_some())
                && !self.card_restale.contains(&line.id)
            {
                self.card_restale.push(line.id);
            }
        }
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
    ///
    /// Three of the four components are charged, not two. `usage.input` excludes cached
    /// prompt tokens (the providers' own split), so uncached input, output, cache reads
    /// and cache writes are disjoint quantities and each is priced at its own rate. The
    /// terms are summed before the single division, which keeps micro-USD rounding to one
    /// truncation instead of four.
    ///
    /// When the registry publishes no cache price and the session moved cache tokens, the
    /// total is a **lower bound**, and `Metrics::cost_is_floor` says so — the header prints
    /// `$0.42+` rather than a precise-looking number that is wrong (G18).
    ///
    /// # The context number, and why it is not the last prompt
    ///
    /// `context_used` is the **session context**, mirrored from
    /// `Agent::estimated_context_tokens` — the same figure the compaction trigger reads, so
    /// the gauge and the loop cannot disagree about how full the context is. It is not
    /// `latest_prompt_tokens`: that is the size of the last *request*, which on turn 1 is
    /// the whole conversation and thereafter is only its prefix. Printing it as a share of
    /// the window was the fabrication this field replaced — it is always smaller than the
    /// context, so it always flattered.
    ///
    /// The mirror is taken once per frame by the event loop
    /// (`App::mirror_context_size`), not read here, because the draw path holds no agent:
    /// `App` is the frame's whole input, and a widget that reached for the loop's state
    /// would be a widget no test could aim.
    ///
    /// `prompt_tokens_last` stays the provider's number and stays the cache rate's
    /// denominator: a hit rate is a claim about one call, and both halves of it have to
    /// come from that call.
    fn metrics(&self) -> crate::ui::metrics::Metrics {
        let (tokens_in, tokens_out) = self.ui.usage();
        let (cache_read_session, cache_write_session) = self.ui.cache_usage();
        // The rate's numerator comes from the SAME call as its denominator, not from the
        // session totals above (G16).
        let cache_read_last = self.ui.latest_cache_read();
        let latest_prompt = self.ui.latest_prompt_tokens();
        // The context *window* is a property of the model selected now — it is a capacity,
        // not a bill — so this one lookup stays keyed on `self.model_id`. The cost below
        // deliberately does not.
        let info = epsilon_llm::catalog::lookup_model(&self.model_id);
        // M141: price each model's tokens at that model's own rates. Pricing the summed
        // totals at `self.model_id` — the model selected *now* — repriced every earlier
        // turn the moment you switched, and `/resume` did the same to replayed history
        // that was billed at other prices. The registry lookup is per model id rather
        // than one lookup for the session, so this is also what makes a resumed
        // multi-model session come back with the bill it actually incurred.
        let cost = if self.ui.usage_by_model().is_empty() {
            // No provider response yet: nothing has been billed, so there is nothing to
            // price and no model whose absence from the registry matters.
            None
        } else {
            Some(crate::ui::cost::breakdown_per_model(
                self.ui.usage_by_model(),
                |model_id| {
                    epsilon_llm::catalog::lookup_model(model_id).map(|info| {
                        crate::ui::cost::TokenPrices {
                            input: info.input_cost_micro_per_mtok,
                            output: info.output_cost_micro_per_mtok,
                            cache_read: info.cache_read_cost_micro_per_mtok,
                            cache_write: info.cache_write_cost_micro_per_mtok,
                        }
                    })
                },
            ))
        };
        let cost_micro_usd = cost.as_ref().map(crate::ui::cost::CostBreakdown::total);
        let cost_is_floor = cost.as_ref().is_some_and(|c| c.is_floor());
        // A frame that has not mirrored yet — the first one, or a bench frame —
        // still reports the provider's own count when there is one. This is a
        // fallback for a missing estimate, not a maximum: on every frame the loop
        // has drawn, the estimate is the measured prompt plus whatever was
        // appended since, so it is the larger of the two and this branch is dead.
        // An estimate is never *smaller* than the prompt it was anchored to, so
        // seeding from `latest_prompt` cannot overstate anything.
        let context_now = if self.ui.context_now > 0 {
            self.ui.context_now
        } else {
            latest_prompt
        };
        crate::ui::metrics::Metrics {
            model_label: epsilon_llm::catalog::display_label(&self.model_id).to_string(),
            offline: self.model_id.starts_with("faux"),
            context_used: (context_now > 0).then_some(context_now),
            context_window: info.as_ref().map(|i| i.context_window).filter(|w| *w > 0),
            tokens_in,
            tokens_out,
            cache_read_session,
            cache_write_session,
            cache_read_last,
            prompt_tokens_last: (latest_prompt > 0).then_some(latest_prompt),
            cost_micro_usd,
            cost_is_floor,
        }
    }

    /// One line naming what the session has cost and which part of it is the cache's doing.
    ///
    /// The header can only show a total, and a total is not actionable: `$0.31` reads the
    /// same whether the prompt cache saved 90% of your spend or none of it. The cache is
    /// the main lever a user has on cost, so `/session` shows the split that tells them
    /// whether it is working.
    ///
    /// **It says what it does not know.** Three cases, each with its own wording, because
    /// collapsing them would be the exact failure this repo keeps fixing:
    ///
    /// - no usage yet → "nothing billed yet", never `$0.00`, which is a precise-looking
    ///   number about nothing;
    /// - a model the price registry lacks → the total is a **lower bound** and is marked
    ///   with `+`, the same marker the header uses, so the two never disagree;
    /// - a genuinely free model (ollama, a `:free` row) → `$0.00` is the true answer, and
    ///   `is_floor` is what distinguishes it, so this line trusts that flag rather than
    ///   second-guessing the registry.
    fn cost_summary(&self) -> String {
        use crate::ui::cost::{TokenPrices, breakdown_per_model};
        if self.ui.usage_by_model().is_empty() {
            return "cost: nothing billed yet".to_string();
        }
        let b = breakdown_per_model(self.ui.usage_by_model(), |model_id| {
            epsilon_llm::catalog::lookup_model(model_id).map(|info| TokenPrices {
                input: info.input_cost_micro_per_mtok,
                output: info.output_cost_micro_per_mtok,
                cache_read: info.cache_read_cost_micro_per_mtok,
                cache_write: info.cache_write_cost_micro_per_mtok,
            })
        });
        let total = crate::ui::format::format_cost(b.total());
        let floor = if b.is_floor() { "+" } else { "" };
        format!(
            "cost: {total}{floor} · in {} · out {} · cache read {} · cache write {}",
            crate::ui::format::format_cost(b.input()),
            crate::ui::format::format_cost(b.output()),
            crate::ui::format::format_cost(b.cache_read()),
            crate::ui::format::format_cost(b.cache_write()),
        )
    }

    /// Mirror the agent's context estimate onto the frame's input.
    ///
    /// The draw path takes `&mut App` and nothing else, which is what lets every
    /// screen be assembled in a test with no agent. The estimate therefore has
    /// to arrive *through* the app, and this is the one place it does — called
    /// by the event loop immediately before it draws, so the number on the row
    /// describes the same history the transcript beneath it is about to show.
    ///
    /// Cheap enough for that position: the estimate is a provider-anchored
    /// figure plus a heuristic over what has been appended since, and it is
    /// taken once per frame rather than once per header fragment.
    pub(crate) fn mirror_context_size(&mut self, agent: &Agent) {
        self.ui.context_now = agent.estimated_context_tokens() as u64;
    }

    /// Footer hints for the current state.
    fn hints(&mut self) -> crate::ui::footer::Hints {
        use crate::ui::footer::{Hints, View};
        // D11: the frozen state comes from the same measurement the transcript
        // draws from, so the badge cannot describe a viewport the reader is not
        // looking at. `scroll_from_bottom > 0` *is* the state — it is what
        // `apply_with_scroll` shifts when rows arrive, which is what keeps the
        // window frozen on the same content — and the counts come from the
        // geometry measured above.
        let view = View {
            frozen: self.scroll_from_bottom > 0,
            rows_below: self.geometry.rows_below(),
            total_rows: self.geometry.total,
        };
        let mut hints = if self.pending_ask.is_some() {
            Hints::approval()
        } else if self.overlay_open() {
            // Overlays own the keyboard; the footer points at their own keys
            // rather than advertising send/stop, which would not work. These are
            // literal because an overlay is a list, not a bound action: there is
            // no binding id for "move the highlight", so there is nothing for
            // the table to resolve and nothing for it to get wrong.
            Hints {
                mode: Some("list".into()),
                keys: vec![
                    ("type".into(), "filter".into()),
                    ("↑↓".into(), "move".into()),
                    ("enter".into(), "choose".into()),
                    ("esc".into(), "close".into()),
                ],
                ..Hints::default()
            }
        } else if self.ui.running() {
            Hints::running_at(&self.keybindings, self.steer_queue_len, view)
        } else {
            Hints::idle_at(&self.keybindings, view)
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

    // ---- D11: where the view is ---------------------------------------------

    /// The transcript's region for a frame of `area` and a draft of `draft_rows`
    /// text rows.
    ///
    /// The one place that knows the split the shell will perform, exposed so the
    /// position badge, the body and the tests all measure the same rectangle. The
    /// production draw calls it once, before the chrome; a test that renders the
    /// transcript directly calls it too rather than recomputing the layout.
    fn transcript_window(area: Rect, draft_rows: u16) -> Rect {
        crate::ui::layout::Regions::split(area, draft_rows).transcript
    }

    /// Measure the viewport for a frame whose transcript region is not in hand.
    ///
    /// Used by the draw path (which must measure before the shell lays out) and by
    /// the tests that render [`draw_transcript`] on their own.
    fn measure_transcript_window(&mut self, area: Rect, draft_rows: u16) -> TranscriptGeometry {
        self.measure_transcript(Self::transcript_window(area, draft_rows))
    }

    /// Bring the render cache in line with the transcript and measure the
    /// viewport the body region will show.
    ///
    /// `area` is the **transcript's** rectangle. Runs *before* the chrome, because
    /// the footer reports the frozen view's position and a footer painted before
    /// the transcript was measured could only ever describe the previous frame.
    ///
    /// Calling it twice in one frame is free and harmless: [`crate::render_cache::RenderCache::reconcile`]
    /// is a no-op once the cache matches the transcript. The draw path calls it
    /// here, once, and the body then renders the geometry this returned.
    fn measure_transcript(&mut self, area: Rect) -> TranscriptGeometry {
        // Body width: the frame minus the gutter the decoration adds back. Wrapping
        // to the full frame width and then prefixing two columns is how a transcript
        // row ends up two cells over the edge and wraps, which turns one line into
        // two and shifts everything below it.
        let body_width = (area.width as usize).saturating_sub(crate::ui::transcript::GUTTER);
        // Taken rather than borrowed: the stale set is consumed by the reconcile
        // that rebuilds those rows, and a card left on the list would rebuild every
        // frame after.
        let restale = std::mem::take(&mut self.card_restale);
        let more_key = self.expand_key();
        let state = crate::render_cache::RenderState {
            expand_all: self.prefs.expand_outputs,
            expanded: &self.card_open,
            more_key: more_key.as_deref(),
        };
        self.render_cache
            .reconcile(self.ui.lines(), body_width, &state, &self.theme, &restale);
        // The streaming tail is not a transcript line yet, so it is counted here
        // rather than by the cache: the viewport scrolls over history *and* the
        // answer arriving under it, and a total that ignored the tail would make
        // the position badge jump by a row every time the model wrapped a line.
        let total = self.render_cache.total_rows() + self.streaming_tail_rows(body_width);
        let visible = area.height as usize;
        TranscriptGeometry {
            skip: crate::scroll::transcript_skip(total, visible, self.scroll_from_bottom),
            total,
            visible,
        }
    }

    /// The in-flight answer's and reasoning trace's wrapped rows, as a count.
    ///
    /// Cheap by construction: two `split('\n')` passes over the in-flight text,
    /// never a wrap of the transcript. [`draw_transcript`] builds the same rows
    /// through the same [`crate::wrap::wrap_line`] call, so the count the footer
    /// reports is the count the body draws.
    fn streaming_tail_rows(&self, body_width: usize) -> usize {
        [
            (self.ui.streaming_thinking(), self.theme.thinking()),
            (self.ui.streaming_text(), self.theme.assistant()),
        ]
        .into_iter()
        .filter(|(text, _)| !text.is_empty())
        .map(|(text, style)| {
            text.split('\n')
                .map(|line| crate::wrap::wrap_line(&[(line.to_string(), style)], body_width).len())
                .sum::<usize>()
        })
        .sum()
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
        border: None,
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
        border: None,
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
        border: None,
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
///
/// The last step is decoration: each visible row comes out of the cache as a bare
/// body, and [`crate::ui::transcript::decorate`] adds the two-column gutter and,
/// on a truecolor terminal, the band that reaches the right edge. That order is
/// deliberate — decoration sits OUTSIDE the cache, so switching colour mode (or
/// losing `NO_COLOR`) never invalidates a rendered row and the cache key stays
/// `(width, render state)`.
///
/// M126 adds the second half of that key: **which cards are open**. It is part of
/// the key rather than a render-time lookup because a card's rows depend on it —
/// an open card is taller, it is wrapped differently, and serving the collapsed
/// rows under an open state is the one bug the key exists to prevent. The state is
/// cheap to compare (a bool, a short list and one string) and changes only when
/// the user presses a key.
/// The measurement already happened in [`App::measure_transcript`]; this function
/// renders a [`TranscriptGeometry`] rather than deriving it a second time, so the
/// rows the reader sees are the rows the footer counted. A zero-height geometry
/// is not a small viewport — it means "not measured", and the caller below the
/// layout floor legitimately has none.
fn draw_transcript(f: &mut Frame<'_>, app: &mut App, area: Rect, geometry: TranscriptGeometry) {
    if geometry.visible == 0 {
        // No viewport, no rows. Drawing with a zero-height window would append the
        // whole streaming tail and hand ratatui more rows than the area holds.
        return;
    }
    let frame_width = area.width as usize;
    let body_width = frame_width.saturating_sub(crate::ui::transcript::GUTTER);

    // Keep thinking/prose and explicit newlines separate; slice the combined
    // history + live tail in screen rows, including tails taller than the viewport.
    // The tail is not a transcript line yet, so its kind is decided here: it is
    // the assistant's own prose (or its reasoning) arriving token by token.
    let mut tail_rows: Vec<(LineKind, Line<'static>)> = Vec::new();
    for (text, style, kind) in [
        (
            app.ui.streaming_thinking(),
            app.theme.thinking(),
            LineKind::Thinking,
        ),
        (
            app.ui.streaming_text(),
            app.theme.assistant(),
            LineKind::Assistant,
        ),
    ] {
        if !text.is_empty() {
            for line in text.split('\n') {
                for wrapped in crate::wrap::wrap_line(&[(line.to_string(), style)], body_width) {
                    tail_rows.push((
                        kind,
                        Line::from(
                            wrapped
                                .into_iter()
                                .map(|(text, style)| Span::styled(text, style))
                                .collect::<Vec<_>>(),
                        ),
                    ));
                }
            }
        }
    }
    // `total` here is the CACHED row count, which is what the tail rows are
    // skipped against: the tail is not in the cache, so it starts at that offset.
    let total = app.render_cache.total_rows();
    let skip = geometry.skip;
    let visible = geometry.visible;
    let mut rows = app.render_cache.visible_rows_with_kinds(skip, visible);
    let remaining = visible.saturating_sub(rows.len());
    rows.extend(
        tail_rows
            .into_iter()
            .skip(skip.saturating_sub(total))
            .take(remaining),
    );

    // visible_rows already applied the viewport offset; decoration runs on the
    // rows that survived, and pads to the FRAME width, not the body width, so a
    // banded row reaches the terminal edge.
    let rows: Vec<Line<'static>> = rows
        .into_iter()
        .map(|(kind, line)| {
            let body: Vec<Span<'static>> = line.spans;
            crate::ui::transcript::decorate(&app.theme, kind, body, frame_width)
        })
        .collect();

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
        border: None,
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
        border: None,
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
        border: None,
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
        border: None,
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
/// telling them apart at a glance is the point. That is now true of the code as
/// well as of this comment: the frame's colour is passed in
/// ([`crate::ui::overlay::Panel::border`]) instead of being inherited from
/// `focused: true`, which yielded the same accent every other overlay uses.
///
/// **Six rows, not eleven.** The dialog is a yes/no with one line of detail, and
/// a tall box for a small question reads as more consequential than it is —
/// which is the opposite of what the danger colour is for. The skill's own rule
/// is 5–7 rows for a confirm prompt (`references/layout.md`); the body keeps the
/// capability, the one line that says what will actually happen, and the four
/// decision keys, and drops the blank separator and the `pattern` field, which
/// restated what the detail line already shows.
fn draw_dialog(f: &mut Frame<'_>, app: &App, outer: Rect) {
    use crate::ui::overlay::{self, Panel, Width};

    /// Interior rows the body needs: capability, detail, and the two key lines.
    ///
    /// Stated once and used for the height, so the box cannot be given a size the
    /// body does not fill — an oversized dialog is how the eleven-row version
    /// happened.
    const BODY_ROWS: u16 = 4;
    const CHROME_ROWS: u16 = 2;

    let width = Width::PROMPT.resolve(outer);
    let height = (BODY_ROWS + CHROME_ROWS).min(outer.height);
    let panel = Panel {
        theme: &app.theme,
        area: overlay::centered(outer, width, height),
        title: "permission required",
        focused: true,
        // The one overlay that is not merely asking a question.
        border: Some(app.theme.danger()),
    };
    let inner = panel.open(f);
    let theme = &app.theme;

    let body: Vec<Line<'static>> = match app.pending_ask.as_ref() {
        Some(ask) => {
            let req = &ask.request;
            // A labelled field, then its value: the label is dim so a reader
            // scanning for the answer skips it, and the value is not. The label
            // column is five wide — wide enough for `cap`, narrow enough that the
            // decision keys below keep their descriptions inside the panel.
            let field = |label: &str, value: String| {
                Line::from(vec![
                    Span::styled(format!("{label:<5}"), theme.faint()),
                    Span::styled(value, theme.text()),
                ])
            };
            // The detail is the line the skill asks for: what will actually
            // happen. For a shell call that is the command, not a JSON object
            // with the command buried in it — see [`summarize_detail`].
            let detail = summarize_detail(&req.detail);
            vec![
                field("cap", req.capability.clone()),
                field("what", detail),
                Line::from(vec![
                    Span::styled("y", theme.success()),
                    Span::styled(" allow once", theme.dim()),
                    Span::styled("   a", theme.success()),
                    Span::styled(" always (session)", theme.dim()),
                ]),
                Line::from(vec![
                    Span::styled("n", theme.danger()),
                    Span::styled(" deny", theme.dim()),
                    Span::styled("         e", theme.danger()),
                    Span::styled(" deny + guidance", theme.dim()),
                ]),
            ]
        }
        // Reached only when the ask was taken between the draw call and here,
        // which the caller's `is_some` check makes a one-frame race.
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
        border: None,
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
        border: None,
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
#[cfg(test)]
mod tests_support;

#[cfg(test)]
mod golden;

#[cfg(test)]
mod tests;
