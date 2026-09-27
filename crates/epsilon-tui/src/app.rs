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

    /// An open transcript search, or `None`.
    ///
    /// `None` is the common case and is the whole point: this is state, not a mode, so
    /// every other key path keeps treating the app as a normal transcript until something
    /// actually opens a search.
    search: Option<crate::search::TranscriptSearch>,

    /// A transcript line the view must scroll to at the next measurement.
    ///
    /// A search cannot move the view when the key is handled: the line-to-row mapping lives
    /// in the render cache, and the cache is reconciled *during* the draw, so at key time it
    /// is empty and every lookup misses. That was the first implementation, and the frame
    /// test caught it at once — `row_of_line` returning `None` silently meant "do nothing".
    ///
    /// So the search records its target here and [`App::measure_transcript`] applies it once
    /// the offsets exist. Deliberately *not* cleared on a miss: the first jump of a search
    /// happens before any draw has reconciled the cache, so dropping it there would lose
    /// precisely the jump that matters most.
    pending_jump: Option<usize>,

    /// Height of the transcript viewport in rows, recorded at each frame.
    ///
    /// The transcript search has to place a match inside the viewport, and the viewport's
    /// height is only known while a frame is being measured — which is *after* a key is
    /// handled. Recording it here is what lets a search move the view at all; without it the
    /// match can only be scrolled to the very top, which hides whatever the reader was
    /// looking at one line above it.
    viewport_rows: usize,
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
mod editor;
use editor::*;

mod login;
use login::*;

mod view;

mod commands;

mod panels;
// The panel surface, named rather than glob-imported: `draw()` is the only caller, and a list
// keeps the set of things drawn over the transcript readable in one place.
use panels::{
    draw_at_file, draw_auth_picker, draw_auth_prompt, draw_dialog, draw_palette, draw_picker,
    draw_session_browser, draw_slash_autocomplete, draw_timeline, draw_transcript,
};

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
    /// What the welcome panel calls this session: its name when it has one, otherwise the
    /// git branch, otherwise nothing. Never a placeholder — an empty label drops the row,
    /// and a row that says "session: -" is worse than no row.
    fn session_label(app: &App) -> String {
        app.session_name.clone().unwrap_or_default()
    }

    let theme = app.theme;
    // Owned, because `shell` borrows it and `app` is used again in the closure below.
    let session_label = session_label(app);
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
        // Session first, branch second: a session label is the thing a user can
        // act on, and the branch is context for it. Falls back to the branch alone.
        session_label: &session_label,
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
    } else if app.search.is_some() {
        // M157: a search that moves the view without saying so is the same defect M155
        // was about — the action happens, the user is not told. The panel names the query,
        // which match is current, and what the keys do.
        draw_search(f, app, f.area());
    }
}

/// The open search: the query, where you are in the matches, and the keys.
///
/// Drawn last, over the transcript, for the same reason the permission dialog is: the
/// thing it reports has to be visible while it is being reported. A counter in the footer
/// would be out of the way exactly when a user is pressing `n` through a list.
fn draw_search(f: &mut Frame<'_>, app: &App, outer: Rect) {
    use crate::ui::overlay::{self, Panel, Width};
    use ratatui::text::{Line, Span};

    let Some(search) = app.search.as_ref() else {
        return;
    };
    let theme = &app.theme;
    // query row + position row + one key row, plus the panel's two border rows.
    let width = Width::PROMPT.resolve(outer);
    let height = (3 + 2).min(outer.height);
    let panel = Panel {
        theme,
        area: overlay::centered(outer, width, height),
        title: "search",
        focused: true,
        border: Some(theme.accent()),
    };
    let inner = panel.open(f);
    let body = vec![
        Line::from(vec![
            Span::styled("find  ", theme.faint()),
            Span::styled(search.query.clone(), theme.text()),
        ]),
        Line::from(vec![
            Span::styled("      ", theme.faint()),
            // One of these, never both and never neither: a counter reading `4 of 4`
            // while the viewport is somewhere else is worse than no counter at all.
            Span::styled(
                search.position(),
                if search.matches.is_empty() {
                    theme.danger()
                } else {
                    theme.text()
                },
            ),
        ]),
    ];
    f.render_widget(ratatui::widgets::Paragraph::new(body), inner);
    if let Some(hint_row) = inner.height.checked_sub(3).map(|y| Rect {
        x: inner.x,
        y: inner.y + y,
        width: inner.width,
        height: 1,
    }) {
        f.render_widget(
            crate::ui::overlay::hint_row(
                theme,
                &[("n", "next"), ("p", "previous"), ("esc", "close")],
            ),
            hint_row,
        );
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

#[cfg(test)]
#[cfg(test)]
mod tests_support;

#[cfg(test)]
mod golden;

#[cfg(test)]
mod tests;
