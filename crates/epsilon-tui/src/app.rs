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
    /// The goal-gate runner that shares the agent's sandbox.
    ///
    /// Handed in rather than resolved here, and that is the security property rather than a
    /// convenience: a gate run outside the session's confinement is a command the agent's
    /// own `bash` is refused. `None` when the platform resolved no sandbox, in which case a
    /// gated goal cannot be evaluated and completion already refuses for it.
    pub gate_runner: Option<epsilon_runtime::GoalGateRunner>,
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
    /// Whether the harness is running **offline**, decided by the CLI (`--provider faux`, or no
    /// usable credentials), and carried rather than re-derived — the same reasoning as
    /// [`Self::theme_mode`] and [`Self::model_id`]: the frontend does not decide a second time.
    ///
    /// M233: without this the header named the *selected* model over a faux turn. Keying the
    /// offline marker off `model_id.starts_with("faux")` meant `--provider faux` beside a saved
    /// `deepseek/…` model rendered `DeepSeek V3.1` with no offline marker — **a screen naming a
    /// provider the harness is not calling**, which is the same class as the silent no-op M194
    /// closed. The headless path already said so (`provider: faux (offline scripted)`);
    /// the TUI did not.
    pub offline: bool,
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
/// How many display-steering characters this ask carries, across both fields the dialog
/// shows.
///
/// **One predicate, two callers**: the footer decides which keys to advertise, and the key
/// handler decides which to honour. Two computations of the same question is how the
/// footer ends up promising `y` while the handler refuses it — which is the defect this
/// repository tracks by name, arriving from the direction of a security fix.
pub(crate) fn ask_display_is_honest(ask: &PendingAsk) -> bool {
    // **One predicate, and it lives in `epsilon-runtime`** (M272) so the desktop window asks
    // the same question. It used to be computed here from this crate's own stripping, which
    // the window had no copy of — the same shape as A48 and `goal_command`, and the reason the
    // two frontends disagreed.
    epsilon_runtime::ask_is_display_honest(&ask.request)
}

pub(super) fn summarize_detail(detail: &serde_json::Value) -> String {
    // **One implementation for the rule, one for the fitting.** This used to hold both, and the
    // desktop window — which has no copy — rendered `request.pattern` on its permission screen
    // instead. `pattern` is the first whitespace token, so the window asked about
    // `Remove-Item` while the shell was about to be handed
    // `Remove-Item -Recurse C:\Users\me` (M271).
    //
    // The rule — *which field answers the question* — is now
    // `epsilon_runtime::summarize_request`, because that is the same in both windows. The
    // **budget is not**: a 96-column terminal row and a 520-point window pane are different
    // measurements, and a shared one would be wrong in at least one of them. So the terminal
    // fits here, and the desktop fits in points at its own draw site.
    crate::wrap::fit_width(&epsilon_runtime::summarize_request(detail), 96)
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
    /// Pending `/import` target (P-079): the path, held while the reader
    /// confirms. `None` means no question is open. Answered in
    /// `submit_editor`, next to the trust gate, for the same reason: a
    /// session swap drops the current context from view, so it asks first.
    pending_import: Option<(std::path::PathBuf, String)>,
    /// Pending `/preset` switch to full access (ADR-032): held while the
    /// reader confirms. `None` means no question is open. Answered in
    /// `submit_editor`, next to the trust and import gates: choosing it is
    /// the privilege-raising act, so it cannot be taken by a reflexive Enter.
    pending_preset: Option<String>,
    guidance_mode: bool,
    scroll_from_bottom: usize,

    /// Which rows the transcript shows.
    ///
    /// A *view* choice, not session state: Pi keeps its own `filterMode` in
    /// memory on the selector instance (`tree-selector.ts:112`), and there is no
    /// reason for a filter to outlive the process that set it. Deliberately not
    /// persisted, unlike `hide_thinking`, which is a preference the reader sets
    /// once.
    ///
    /// Applied in `RenderCache::reconcile` beside `hide_thinking` and carried in
    /// the cache key with it — a row cached under one filter and served under
    /// another is the stale-cache failure that key exists to prevent.
    pub(crate) filter: crate::state::TranscriptFilter,

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
    /// How many rows the picker showed last frame, measured at paint time.
    ///
    /// **A Page key is this many rows**, which is what pi does
    /// (`session-selector.ts:612-617` pages by `maxVisible`) and what a user
    /// expects: the page is the screenful they can see. A constant would be a
    /// whole screen on a tall terminal and a fraction of one on a short one.
    ///
    /// One when the picker has not been painted yet, so a key arriving before the
    /// first frame moves by one rather than by zero — a key that did nothing would
    /// be indistinguishable from a frozen TUI.
    picker_capacity: usize,
    recents: Vec<String>,
    /// Favorite (starred) model ids, persisted in config `[picker] favorite`
    /// (M23; opencode favorites / pi scoped-models hybrid).
    favorites: Vec<String>,
    /// Saved default model id, loaded once at startup (P-254; Ctrl+S writes
    /// it back and refreshes this). `None` means none saved. Kept in memory
    /// so the picker never hits disk mid-keypress (and tests inject without
    /// touching global state) — the favorites pattern, one field over.
    default_model: Option<String>,
    /// Config dir override for tests (Ctrl+S writes here instead of the
    /// global dir). `None` in production — always.
    pub(crate) config_dir_override: Option<std::path::PathBuf>,
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
    /// The gate runner the CLI composed with the agent's sandbox. See [`TuiConfig::gate_runner`].
    gate_runner: Option<epsilon_runtime::GoalGateRunner>,
    /// A gate-checked goal round, from the task that ran the gates back to the frame loop.
    ///
    /// **Why not awaited in place.** `drive` is async and could await it, but a gate is a
    /// shell command — `cargo test` on a real project is thirty seconds — and awaiting that
    /// in the frame loop is an unresponsive terminal for the whole run. The gate is the one
    /// thing in a goal round that is allowed to take real time, so it is the one thing that
    /// does not happen on the frame's thread.
    goal_round_tx: tokio::sync::mpsc::UnboundedSender<epsilon_runtime::GoalRound>,
    goal_round_rx: tokio::sync::mpsc::UnboundedReceiver<epsilon_runtime::GoalRound>,
    /// A round is being gate-checked, so do not start another until it answers.
    ///
    /// Without this the loop would fire a second gate run on the very next turn end, and two
    /// rounds would be spent for one continuation.
    goal_round_pending: bool,
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
    card_restale: std::collections::HashSet<u64>,
    /// The workspace path, already shortened for the footer — **one label per repertoire**.
    ///
    /// The original cache keyed on (root, home) alone and took its ellipsis from
    /// `Glyphs::detect()` at construction. That baked the *machine's* default into every
    /// frame any other repertoire drew: `…` is not in CP437 at all
    /// (`Glyphs::ASCII` uses `...`), so a window that detected Unicode and drew an ASCII
    /// frame — a golden sweep, or an `EPSILON_ASCII=1` process whose theme outlived its
    /// construction — printed the one character the ASCII repertoire exists to remove.
    /// `an_ascii_frame_draws_no_character_a_legacy_console_cannot` found it in round 105's
    /// battery: twelve rows carrying a literal `…`.
    ///
    /// Both variants are derived once, where the field is declared — the environment
    /// lookups and the path walk still happen once per repertoire, never per frame — and
    /// the draw picks by the theme's own glyphs, like every other character on screen.
    /// Index 0 is the Unicode ellipsis, index 1 the ASCII one.
    location_labels: [String; 2],
    /// The session goal as the footer shows it (`● goal 2/256`), or `None` when there is none.
    ///
    /// Cached rather than read from the store per frame: a goal changes when a person or the
    /// model changes it, and folding the session log thirty times a second to notice is the
    /// per-frame cost this crate has spent three milestones removing. `App::refresh_goal` is the
    /// only writer, called from a `/goal` command and at the end of a turn.
    goal_badge: Option<String>,
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
    /// Phase 3 (TUI as IDE): workspace tree sidebar open (F6). When open the
    /// tree owns the keyboard like every other overlay (see `overlay_open`).
    pub(crate) tree_open: bool,
    /// Cursor row within `tree_rows`.
    tree_cursor: usize,
    /// Visible rows from the last `refresh_tree`.
    tree_rows: Vec<crate::tree::TreeRow>,
    /// Collapsed directory rels; everything starts expanded.
    tree_collapsed: std::collections::BTreeSet<String>,
    /// Whether the last refresh hit the row budget (drawn as a capped note).
    tree_capped: bool,
    /// Read-only file viewer opened from the tree (Phase 3, second slice).
    viewer: Option<crate::tree::FileViewer>,
    /// Visible viewer lines measured at the last draw (page keys read it).
    viewer_capacity: usize,
    /// Rows in the `/tree` + `/timeline` picker, in display order.
    ///
    /// M196: this was `(seq, kind, snippet)` built from `branch_entries()` — the
    /// **active lane only** — so an abandoned branch was invisible no matter what
    /// the user typed. It is now flattened from `SessionStore::tree()`, so a
    /// branch the user rewound away from is still on screen, which is the whole
    /// reason pi has a tree rather than a list.
    ///
    /// The extra two fields are what turns a list into a tree:
    /// - `depth` drives the indent, so nesting is visible.
    /// - `on_active` marks a row on the lane the session is currently on, so the
    ///   flat rows are readable as "the path I am on" versus "a branch I left".
    timeline_entries: Vec<TimelineRow>,
    /// Plan 009: compact spinner — manual compaction in-flight.
    ///
    /// Shared, because the work runs on a detached task that has no `&mut App` and
    /// nobody else knows when it ends: it used to be a plain `bool`, set when the
    /// task was spawned and **never cleared**, so a session could compact exactly
    /// once and every later `/compact` refused with *"compact already running"*.
    /// An `AtomicBool` in an `Arc` is the whole fix — the flag already means "a
    /// compaction is in flight", and this is how a task says it is no longer.
    compact_running: Arc<std::sync::atomic::AtomicBool>,
    /// M16c: draft stashed when a dialog opened; restored when it closes.
    stash: Option<(String, usize)>,
    /// Parity sweep: editor undo stack (Ctrl+Z), pi undo-stack.ts semantics.
    undo: crate::undo::UndoStack,
    /// M32: masked API-key prompt overlay (`/login` without a key pasted).
    /// Masked like opencode's DialogPrompt — rendering shows • per char while
    /// the real key stays in `auth_pending.provider/auth_key`.
    auth_prompt_open: bool,
    /// M196: inline label editor opened from the `/tree` picker with `l`.
    ///
    /// A separate flag rather than a mode enum, because the picker and this
    /// prompt never coexist: opening the editor leaves the picker closed, so the
    /// two cannot fight over which one owns the keyboard. The entry being
    /// labelled is carried here because the picker's selection is gone by then.
    /// Jump-to-character mode: the direction of the NEXT printable key, or `None`
    /// when the next key is an ordinary character.
    ///
    /// M196: pi's editor has this (`editor.ts:342`) and Epsilon had only
    /// Left/Right/Home/End. A long draft with a repeated word is exactly where
    /// arrow keys stop being enough, which is why the reference has it.
    ///
    /// A `Option<JumpDir>` rather than a bool, because there are two directions
    /// and a bool makes "backward" mean "not forward" -- which is how a mode ends
    /// up doing the opposite of what the key said.
    jump_mode: Option<JumpDir>,
    /// Which entries the picker shows; cycled with `f` (pi's tree filter).
    tree_filter: TreeFilter,
    /// The picker's search text; `/` opens the field (pi's tree search).
    tree_query: String,
    /// Whether the search field has the keyboard, as opposed to the row list.
    ///
    /// Two states rather than a mode enum because the field is drawn *inside* the
    /// picker panel, not over it: a reader needs to see the rows they are
    /// filtering while they type.
    tree_searching: bool,
    /// Ids whose subtree the picker is hiding — pi's `foldedNodes`
    /// (`tree-selector.ts:121`).
    ///
    /// A set of ids rather than a boolean "hide children": a fold is *per node*,
    /// so collapsing one deep branch leaves its siblings expanded. That is the
    /// whole point of folding rather than a "collapse all" toggle.
    folded: std::collections::HashSet<String>,
    /// Every row, unfiltered, so changing the filter does not re-read the file.
    ///
    /// Filtering the *filtered* list is a bug that looks correct: pressing `f`
    /// twice narrows twice, and a row the first press removed can never come
    /// back. Keeping the full set means each press is a fresh decision from the
    /// same starting point, which is what pi does.
    timeline_all: Vec<TimelineRow>,
    label_prompt_open: bool,
    label_prompt_target: Option<String>,
    /// The text being typed. `None` while no prompt is open.
    label_prompt_text: String,
    label_prompt_cursor: usize,
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
    /// Whether the CLI decided this process runs offline (M233).
    ///
    /// Carried rather than re-derived from `model_id`, because those are different facts: a user can
    /// have a saved `deepseek/…` model **and** ask for `--provider faux`, and then the header has to
    /// name the faux row.
    offline: bool,
    /// Prompt templates discovered at startup and on `/reload`.
    ///
    /// Cached rather than re-read per submit: expansion happens on every message, and
    /// walking two directories to discover files that have not changed would tax the
    /// one path that must stay fast. `/reload` is the refresh, same as for skills.
    prompt_templates: Vec<epsilon_contracts::prompt_template::PromptTemplate>,
    /// Cached git branch + when it was read (2s TTL — no git dependency).
    branch_cache: (Option<String>, std::time::Instant),
    /// Cached `@`-completion inputs: the ignore rules, the file list they produced, and
    /// when they were read.
    ///
    /// `IgnoreSet::load` searches the workspace for `.gitignore` files, which is not work
    /// a keystroke can pay for: typing `@Cargo` re-derives the popup once per character.
    /// Without this cache each character paid a full search, and in a workspace holding a
    /// large ignored tree the popup simply never appeared — measured in a real console,
    /// which is how the regression was found (M110). The branch cache above uses the same
    /// TTL shape for the same reason; the TTL is what bounds how long an edited
    /// `.gitignore` goes unnoticed.
    ///
    /// **M194 added the file list.** M110 cached the *rules* and left the traversal to
    /// run per keystroke, on the reasoning that the rules were the expensive part. They
    /// are not: measured by mirroring `at_files::walk_inner`, one walk costs 4.2 ms in
    /// this repository and up to 9.2 ms against a large home tree, paid on every
    /// character typed after an `@` to rebuild a list that cannot have changed since the
    /// previous character. The rules and the list now expire together, so the list is
    /// never fresher than the rules that shaped it.
    at_ignore: Option<(
        epsilon_core::ignore::IgnoreSet,
        std::time::Instant,
        Vec<String>,
    )>,
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
    /// Directory scanned for session files (`.epsilon/sessions`, the root of both layouts).
    dir: std::path::PathBuf,
    /// The workspace those sessions belong to (M217) — the per-cwd subdirectory, plus the flat
    /// root's files whose header names this workspace. Carried rather than re-read so a filter
    /// keystroke cannot change which sessions the browser is looking at.
    cwd: std::path::PathBuf,
    /// The session this app is attached to (marked `●` in the list).
    current: Option<std::path::PathBuf>,
    /// Filtered rows, newest first.
    rows: Vec<crate::picker::SessionRow>,
    /// Highlighted row.
    index: usize,
    /// Rows the browser showed last frame, measured at paint time.
    ///
    /// A Page key is this many rows, which is pi's rule
    /// (`session-selector.ts:612-617` pages by `maxVisible`) and the only one a
    /// reader can predict from the screen. One until the first paint, so a key
    /// before the first frame still moves.
    capacity: usize,
    /// The session awaiting deletion confirmation (P-237).
    ///
    /// `Some(path)` means every key is intercepted until the reader answers —
    /// pi's `confirmingDeletePath` (`session-selector.ts:64`, interception at
    /// `:543-557`). The alternative, deleting on the first press, is the defect
    /// `AGENTS.md` §6 exists for: the unlink fallback is unrecoverable, so a
    /// one-keystroke guard is the *only* undo there is.
    confirming_delete: Option<std::path::PathBuf>,
    /// A refusal to explain, shown in the panel itself (P-237).
    ///
    /// A notice pushed to the transcript would sit **behind** this overlay,
    /// where the reader who just pressed the key cannot see it — which is
    /// exactly the "a bound key the UI does not act on" failure `AGENTS.md`
    /// §2.7 forbids. Pi writes its refusal into its own header
    /// (`session-selector.ts:408`); this is the same idea in the panel.
    delete_refusal: Option<String>,
}

impl SessionBrowser {
    /// Scan + filter once; callers rebuild after each filter keystroke.
    fn build(
        dir: std::path::PathBuf,
        cwd: std::path::PathBuf,
        current: Option<std::path::PathBuf>,
        filter: &str,
    ) -> Self {
        let rows = crate::picker::session_rows(&dir, &cwd, filter, current.as_deref(), now_ms());
        Self {
            dir,
            cwd,
            current,
            rows,
            index: 0,
            capacity: 1,
            confirming_delete: None,
            delete_refusal: None,
        }
    }

    /// Re-run the scan/filter with the current input as the query.
    fn rebuild(&mut self, filter: &str) {
        self.rows = crate::picker::session_rows(
            &self.dir,
            &self.cwd,
            filter,
            self.current.as_deref(),
            now_ms(),
        );
        if self.index >= self.rows.len() {
            self.index = self.index.min(self.rows.len().saturating_sub(1));
        }
    }

    /// The path a delete key press names, or `None` when it must not delete.
    ///
    /// Two refusals, both pi's (`session-selector.ts:402-413`): a row that does
    /// not exist, and the **currently attached** session. Deleting the file the
    /// agent is writing to would leave a live store pointing at a path that no
    /// longer exists, which is a worse failure than not offering the key.
    fn delete_target(&self) -> Option<std::path::PathBuf> {
        let path = self.rows.get(self.index)?.path.clone();
        if self.current.as_ref().is_some_and(|c| c == &path) {
            return None;
        }
        Some(path)
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
    /// Hand the round-driver its delivery channel.
    ///
    /// Split out from `new` for the same reason the OAuth channel is: `new` takes channels
    /// it sends on, and a channel created inside `new` would have nowhere for the frame
    /// loop to read from.
    pub(crate) fn set_goal_round_channel(
        &mut self,
        tx: tokio::sync::mpsc::UnboundedSender<epsilon_runtime::GoalRound>,
        rx: tokio::sync::mpsc::UnboundedReceiver<epsilon_runtime::GoalRound>,
    ) {
        self.goal_round_tx = tx;
        self.goal_round_rx = rx;
    }

    /// A gate-checked round, if one has arrived. Non-blocking, once per frame.
    pub(crate) fn take_goal_round(&mut self) -> Option<epsilon_runtime::GoalRound> {
        self.goal_round_rx.try_recv().ok()
    }

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
            self.default_model.as_deref(),
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

    /// Save a model id as the default (P-254): persisted immediately and
    /// reflected in memory, so the picker orders and marks it without a
    /// restart. Writes to the override dir in tests, else the global dir —
    /// a global write the tests must never make, hence the override.
    fn save_default_model(&mut self, id: &str) {
        let dir = self
            .config_dir_override
            .clone()
            .or_else(epsilon_core::config::global_dir);
        let Some(dir) = dir else {
            self.ui
                .push_notice("default not saved: no home directory resolved");
            return;
        };
        match epsilon_core::config::save_model_id_in(&dir, id) {
            Ok(_) => {
                self.default_model = Some(id.to_string());
                self.invalidate_picker();
                self.ui
                    .push_notice(format!("default model: {id} (opens second in the picker)"));
            }
            Err(e) => {
                self.ui.push_notice(format!("default not saved: {e}"));
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
    /// Submit the editor line; returns `true` when a command requested quit.
    ///
    /// M16b: paste markers expand back into their payloads here, so the model sees
    /// the real pasted text while the editor only ever held the compact marker.
    /// Discover prompt templates from the user's config and the workspace.
    ///
    /// The roots mirror skills exactly, and the reason is that a user who has learned where
    /// their skills live has learned where everything else project-scoped lives. Two
    /// directories rather than one because `.agents/` is the convention shared with other
    /// agent tools and `.epsilon/` is ours; a workspace may use either or both.
    ///
    /// # Why the project roots are gated (A32)
    ///
    /// **A template's body goes back through `parse_command`.** This is not a text
    /// substitution ending at the model — `expand_template` runs *before* the command parse
    /// in `submit_text`, so a repository shipping `.agents/prompts/setup.md` whose body is
    /// `/trust yes` answers the trust prompt that exists to ask whether to run that
    /// repository's code. `/trust` also records the config hash, so it is permanent.
    ///
    /// The gate was already declared: `.epsilon/prompts` has been listed in
    /// [`TrustRequiringResource::Prompts`] since the enum was written, and nothing in
    /// production read it. A declaration with no consumer is not a gate — this is the
    /// "faithful port, no production caller" shape the ledger has now seen seven times,
    /// where the port is the *declaration* rather than the implementation.
    ///
    /// `trust_review_pending` is the same signal the sibling `agents` reload path uses
    /// (`commands.rs`), because it is what this frontend already carries: the composition
    /// crate's `project_trusted` reaches the window, and this is its spelling here.
    pub(super) fn discover_prompt_templates(&mut self) -> usize {
        use epsilon_contracts::prompt_template as pt;

        let mut user = Vec::new();
        if let Some(dir) = epsilon_core::config::global_dir() {
            user = pt::load_from_dir(&dir.join("prompts"), pt::TemplateScope::User);
        }
        // Untrusted project: the user's own templates still load, so a personal `/deploy`
        // keeps working while the repository's cannot answer a question about the repo.
        let project = if self.trust_review_pending {
            Vec::new()
        } else {
            [
                pt::load_from_dir(
                    &self.workspace_root.join(".agents").join("prompts"),
                    pt::TemplateScope::Project,
                ),
                pt::load_from_dir(
                    &self.workspace_root.join(".epsilon").join("prompts"),
                    pt::TemplateScope::Project,
                ),
            ]
            .concat()
        };

        // Project wins on a name clash, so a repository can ship its own `review` without
        // the user editing their personal config. Explicit paths would go last; the CLI
        // has no `--prompt` flag yet, so there is nothing to append.
        self.prompt_templates = pt::merge(user, project, Vec::new());
        self.prompt_templates.len()
    }

    /// Expand `text` if it names a prompt template, else `None`.
    ///
    /// Returning the input unchanged (as `None`) is what lets a template name coexist with
    /// a slash command: a user with a `help` template who types `/help` gets whichever the
    /// lookup finds first, and a text naming neither template nor command is untouched.
    pub(super) fn expand_template(&self, text: &str) -> Option<String> {
        epsilon_contracts::prompt_template::expand_prompt_template(text, &self.prompt_templates)
    }

    /// Submit the composer. `follow_up` is Alt+Enter (P-030): when a turn
    /// is running the text queues for after it instead of steering into it;
    /// when idle it is plain Enter. Every gate above (trust, import, preset,
    /// templates, commands, shell) runs first either way — a follow-up is a
    /// submit that waits, not a way around review.
    fn submit_editor(&mut self, agent: &Arc<Agent>, follow_up: bool) -> bool {
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
        // P-079: a pending `/import` holds ALL submits until answered, like
        // the trust gate above. Only the two answers are accepted; anything
        // else is bounced with the question repeated (the gate must not eat
        // a prompt the reader meant to send).
        if let Some((abs, name)) = self.pending_import.clone() {
            match text.trim().to_ascii_lowercase().as_str() {
                "yes" | "y" => {
                    self.pending_import = None;
                    match self.switch_session_file(agent, &abs, Some(&name)) {
                        Ok(count) => self.ui.push_notice(format!(
                            "imported {count} messages from {} (store swapped)",
                            abs.display()
                        )),
                        Err(e) => self.ui.push_error(format!("import failed: {e}")),
                    }
                }
                "no" | "n" => {
                    self.pending_import = None;
                    self.ui
                        .push_notice("import cancelled — still on the current session");
                }
                _ => {
                    self.ui.push_notice(format!(
                        "Replace current session with '{}'? (yes/no)",
                        abs.display()
                    ));
                }
            }
            return false;
        }
        // P-079's sibling (ADR-032): a pending preset switch holds ALL
        // submits until answered. Only the two answers are accepted; anything
        // else repeats the question (the gate must not eat a prompt).
        if let Some(name) = self.pending_preset.clone() {
            match text.trim().to_ascii_lowercase().as_str() {
                "yes" | "y" => {
                    self.pending_preset = None;
                    if agent.set_preset(&name, &self.workspace_root) {
                        self.ui.push_notice(format!(
                            "permission preset: {name} — everything nothing forbids is now allowed"
                        ));
                    } else {
                        self.ui.push_notice(format!(
                            "preset not switched — the gate reported no change ({name})"
                        ));
                    }
                }
                "no" | "n" => {
                    self.pending_preset = None;
                    self.ui.push_notice(format!(
                        "preset switch cancelled — still on {}",
                        agent.current_preset()
                    ));
                }
                _ => {
                    self.ui.push_notice(format!(
                        "Switch permission preset to '{name}'? This allows everything nothing forbids. (yes/no)"
                    ));
                }
            }
            return false;
        }
        // Prompt templates expand BEFORE slash-command parsing, and that order is the
        // feature: a repository can ship a `review` template, and a user's personal
        // `review` template must be able to do something the built-in commands cannot.
        // Expanding afterwards would make every template name a candidate for a command
        // name, and `/help` would need to arbitrate between two things that are both
        // legitimately called `help`.
        //
        // Expansion happens on the raw text, not on a parsed command, so a template can
        // take arguments (`/review src/parser.rs`) that the command parser would otherwise
        // have consumed. A text that does not name a template is returned unchanged, so
        // every existing path below is untouched when no templates are installed.
        let text = match self.expand_template(&text) {
            Some(expanded) => expanded,
            None => text,
        };
        if let Some(command) = crate::picker::parse_command(&text) {
            return self.run_command(agent, command);
        }
        // `!` runs a shell command without a model round-trip, and `!!` runs one
        // whose output stays out of the model's context.
        // (`interactive-mode.ts:3105-3119`, `handleBashCommand` at `:6476`.)
        //
        // This routes through the real `BashTool`, so the sandbox, the timeout, the
        // output caps and the exit-code vocabulary are the SAME ones a model-run
        // command gets. A second runner here would be a second set of rules, and the
        // user would be able to run something the agent is not allowed to.
        if let Some(rest) = text.strip_prefix('!') {
            let excluded = rest.starts_with('!');
            let cmd = rest[usize::from(excluded)..].trim().to_string();
            if cmd.is_empty() {
                self.ui.push_notice(
                    "shell: type a command after ! (or !! to keep it out of context)".to_string(),
                );
                return false;
            }
            self.run_shell(agent, cmd, excluded);
            return false;
        }
        let msg = self.submit_text(text);
        if follow_up && self.ui.running() {
            agent.follow_up(msg);
            self.notice("follow-up queued (runs when this turn ends)");
        } else if self.ui.running() {
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
            let text = match login(
                &flow_provider,
                &epsilon_core::config::oauth_config(&flow_provider),
                &mut ui,
                true,
            ) {
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
        // shows the key marker or `·` but never warns at apply time). Non-blocking —
        // switching is allowed, you just can't run until the key is there. The marker is
        // the `key` glyph rather than the emoji it used to be, because an astral-plane
        // codepoint cannot be rendered by a legacy console at all (M219).
        let auth_ready = epsilon_llm::catalog::credentials_ready(&id);
        // **The session record happens inside `set_model_id`** (M303), not here.
        //
        // M299 fixed this in `apply_model`, which left `rpc_mode.rs:660` and `:722` switching a
        // live session without recording it — so a stdio-RPC client got the reported "keeps
        // reverting to another" straight back. A session file is shared by every front end, so
        // the write belongs where all of them already call. The launch path benefits for free:
        // `runtime:4874` and `:4985` both call `set_model_id`, so a resume that adopts the
        // session's model now records that adoption too, rather than leaving the session saying
        // something the next resume will read back.
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
        // Persisting to the shared config is best-effort; **the in-memory mirror is
        // not**, and it moves either way.
        //
        // Two separate defects, both found on this machine on 2026-10-04 (M299). First, M29
        // re-read the file after pushing, which makes the mirror a property of a
        // machine-wide file: any other process — or another test in this suite — that has used
        // a model puts it at the front, so the picker's first row depended on what unrelated
        // callers did. Two TUI tests asserting `recents.first()` failed purely because a real
        // user had once chosen `deepseek-flash`, and they failed on unmodified code too.
        // Second, the whole block was gated on the write **succeeding**, so an unwritable
        // config left the picker showing the previous model while the status bar already said
        // otherwise — `AGENTS.md` §2.7, a control that does not act on the thing it names.
        //
        // The mirror is therefore built from the value this call just applied, which is the
        // same answer without the dependency on anyone else.
        //
        // **What this gives up, stated rather than denied** (an independent review caught me
        // claiming both of these were free, and neither is). M29's deleted line was a genuine
        // mid-session refresh: after pushing, it re-read the file so a push made by *another*
        // process would reach this picker's ordering. That behaviour is gone. What survives is
        // weaker than the first version of this comment claimed — `push_recent_model_in` is
        // read-modify-write with **no lock and no temp-file rename**
        // (`epsilon-core/src/config.rs:590-592`), so a concurrent push is lost whenever two
        // writes interleave, which is exactly the race M29's re-read was written for. The
        // honest statement is: this process's own push is always shown, the durable file is
        // still updated, and another process's push appears on the next launch.
        //
        // **Why that trade was still worth it.** The re-read is what made two TUI tests
        // machine-dependent — they assert `recents.first()`, and the answer was whatever a real
        // user had once chosen in any process. The `is_ok()` gate was the other half, and
        // fixing only the gate would have left the test's result depending on the file it
        // claims not to depend on.
        let _ = epsilon_core::config::push_recent_model(&id);
        self.recents.retain(|r| !r.eq_ignore_ascii_case(&id));
        self.recents.insert(0, id.clone());
        self.invalidate_picker();
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
        // **No credential means no fetch, and no announcement.**
        //
        // Found by reading a real session's transcript on 2026-10-04. Opening `/model`
        // fetched for whichever stage-1 row the cursor happened to be on, announced
        // `refreshing 'openai' models in background…`, and then reported
        // `live listing failed: no API key for 'openai'` — **four visible lines about a
        // provider the reader had not chosen and had no key for**, and the "failure" was a
        // normal state. The reader was using deepseek.
        //
        // `resolve_api_key` is the same resolution the fetcher itself uses (stored
        // credential, then env var), so this cannot drift from what the request would have
        // done — it asks the real question instead of guessing with a second copy of the
        // precedence rules. `Ctrl+F` (`force`) still speaks, because there the reader is
        // asking about this provider on purpose and silence would be its own defect.
        if epsilon_llm::catalog::resolve_api_key(&provider).is_none() {
            if force {
                self.notice(format!(
                    "no API key for '{provider}' — set one with `/login {provider}`, then Ctrl+F"
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

    /// Run a `!` shell command off the key handler and show the result.
    ///
    /// **Async on purpose.** The key handler is synchronous and a shell command can
    /// take the tool's full 120s cap. Blocking here would freeze the TUI for the
    /// whole run, which is exactly the symptom this repository has been chasing
    /// since M151 — a program that looks dead.
    ///
    /// `excluded` is pi's `!!`: the output is shown to the user and kept out of the
    /// model's context, which is the whole point of the two-bang form. It is honoured
    /// by never calling `submit_text` — a shell run becomes a system line, and a
    /// system line is not history the model will be sent.
    ///
    /// The command goes through [`Agent::run_local_tool`], so it is confined,
    /// validated and gated exactly as a model-issued one is.
    ///
    /// **The exclusion is named on the running row too, not only on the result.**
    /// The running notice is the one row a reader looks at for the whole time the
    /// command takes, and a bare `running: <command>` said nothing about whether
    /// the output was about to be kept out of context. Pi marks the state on the
    /// card itself — a dimmed border for `!!` (`bash-execution.ts:36-38`) — and
    /// Epsilon's `!`/`!!` output is a system line rather than a card by design
    /// (output that never entered model context is not a tool result), so the
    /// label is where the equivalent fact lives. It is carried on the **error**
    /// path as well: a `!!` that failed lost its marker exactly when a reader
    /// most needs it, which is the failure half of `AGENTS.md` §2.7.
    fn run_shell(&mut self, agent: &Arc<Agent>, command: String, excluded: bool) {
        // Why the word, and which one: `kept out of context` is the same phrase
        // the result row has used since M52, so the two rows read as the same
        // fact rather than as two spellings of it.
        let state = if excluded {
            " (kept out of context)"
        } else {
            ""
        };
        self.notice(format!("running{state}: {command}"));
        let notices = self.notices_tx.clone();
        let agent = Arc::clone(agent);
        tokio::spawn(async move {
            let label = if excluded {
                "shell (kept out of context): "
            } else {
                "shell: "
            };
            match epsilon_core::exec::run_local(
                &agent,
                "bash",
                serde_json::json!({ "command": command }),
            )
            .await
            {
                Ok(text) => {
                    notices.send(format!("{label}{text}")).ok();
                }
                Err(e) => {
                    // The excluded label on the failure path as well, so a
                    // failed `!!` still says its output never reached the model.
                    notices.send(format!("{label}{e}")).ok();
                }
            }
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

/// Fresh sender for [`App::over_ui`]; the matching receiver is dropped immediately.
fn inert_channel<T>() -> tokio::sync::mpsc::UnboundedSender<T> {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    drop(rx);
    tx
}

/// Restores the terminal on the way out, however the way out happens.
///
/// # Why this exists
///
/// A panic used to leave the terminal in raw mode, on the alternate screen, with a
/// hidden cursor — and there is no way to tell that from the outside. The process
/// is gone, but the *console* is not: it looks exactly like a program that has
/// stopped responding, which is precisely the defect the owner reported and
/// impossible to diagnose without knowing to look for a raw-mode console.
///
/// The guard has to live in a `Drop`, not in the error path. `run` returns an
/// `i32`, so there is no `Err` to propagate, and by the time a panic has unwound
/// the stack any code after the call has already been skipped. `Drop` is the only
/// place that runs on *every* exit.
///
/// A second reason to prefer `Drop` over a panic hook alone: a hook can only print.
/// Restoring requires the `ratatui` terminal, which the hook has no handle to, and
/// printing a message to a console still in raw mode is unreadable anyway. This
/// does both — the hook leaves the terminal first, so the message is legible.
struct TerminalGuard;

/// The restore steps, in one place so the normal exit and the panic path cannot
/// drift apart.
///
/// Separate from `Drop` because the panic hook needs to run them *before* the
/// unwinding continues — a hook that only printed would leave the user looking at
/// a raw-mode console, which is where the message goes to die. Taking `&mut self`
/// rather than consuming keeps both entry points on the same function.
impl TerminalGuard {
    /// Idempotent: every step is a `let _ =`, and `Drop` runs the same calls
    /// again when the guard is dropped. Restoring twice is harmless; restoring
    /// zero times is not.
    fn restore(&mut self) {
        use crossterm::execute;
        let _ = execute!(std::io::stdout(), crossterm::event::DisableBracketedPaste);
        // `disable_raw_mode` is the one that matters: without it the user's shell
        // is left with no echo and no line editing, and the next command they type
        // appears nowhere.
        let _ = crossterm::terminal::disable_raw_mode();
        // `ratatui::restore` returns `()`, so the `let _ =` the other two steps
        // need is a `let_unit_value` lint here. It is still a discarded result,
        // just not one that needs a binding.
        ratatui::restore();
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        self.restore();
    }
}

/// Render a panic payload as text the user can read.
///
/// Factored out of the hook so it can be tested without `set_hook`, which is
/// process-global: installing it from a unit test would replace the harness's own
/// hook for every test that runs afterwards.
///
/// Every shape gets a message. A hook that unwraps without a fallback replaces a
/// readable crash report with a second crash, which is strictly worse than the
/// panic it was reporting — and an `i32` payload is what `assert_eq!` produces, so
/// it is not a hypothetical.
fn payload_message(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|s| (*s).to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .or_else(|| payload.downcast_ref::<u32>().map(|n| n.to_string()))
        .unwrap_or_else(|| "unknown panic payload".to_string())
}

/// Print a panic where the user can read it, after the terminal is usable again.
///
/// Installed *after* the guard is armed, so a panic during `drive` restores first
/// (this hook runs during unwinding, before the frames above it are dropped) and
/// the message below lands on a working console. The reverse order would print to
/// a raw-mode alternate screen, which is where a panic message disappears.
fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        TerminalGuard.restore();
        eprintln!("\nepsilon crashed: {}", payload_message(info.payload()));
        eprintln!("the terminal has been restored; this is a bug — please report it.");
        previous(info);
    }));
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
    // Armed immediately after init and before anything that can panic. A panic
    // anywhere below now leaves a usable terminal, which is the entire point.
    let guard = TerminalGuard;
    install_panic_hook();
    // Bracketed paste only — mouse capture stays OFF so drag-select copies
    // (opencode ships `useMouse: false` in `runtime.lifecycle.ts:185`; pi captures
    // clicks — `handleMouse` in `interactive-mode.ts:258` — and we deliberately do
    // not: a click this program cannot see is one it cannot act on, while capture
    // would take drag-select away from every terminal. The wheel needs no capture
    // on any terminal, so it follows focus instead). Hold Shift to select still works
    // on ConPTY, but we don't need to swallow motion just for wheel.
    let _ = execute!(std::io::stdout(), crossterm::event::EnableBracketedPaste);

    // Hidden crash trigger, for the console probe only.
    //
    // D19 is the one defect in this crate whose *symptom* cannot be reproduced by
    // any existing test, because the symptom is a broken console and a test
    // harness that breaks its own console takes every other test with it. A unit
    // test can check the mechanism; only a real conhost can check the outcome.
    //
    // Env-gated rather than bound to a key, so it cannot be typed by accident, and
    // checked once, before the event loop starts, so it cannot fire in production
    // by any route other than someone setting the variable deliberately.
    if std::env::var_os("EPSILON_TUI_CRASH").is_some() {
        panic!("deliberate crash: EPSILON_TUI_CRASH is set (probe hook)");
    }

    let code = drive(cfg, &mut terminal).await;
    let _ = execute!(std::io::stdout(), crossterm::event::DisableBracketedPaste);
    drop(guard); // restores; the explicit calls below are then redundant
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
    //
    // Only events that can change a card are worth the walk. `MessageUpdate` is
    // a streaming text delta — the event that dominates a run by two orders of
    // magnitude — and it cannot end a card. The event loop drains up to
    // `MAX_EVENT_DRAIN` of them before drawing a single frame, so asking every
    // one to re-scan the transcript was `O(events x lines)` per frame for a
    // question whose answer was already known.
    if event_can_end_a_card(ev) {
        app.restale_cards(ev);
    }
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

/// Whether applying `ev` could move a tool card from running to finished.
///
/// The rule is deliberately conservative: `true` may be returned for an event
/// that turns out not to end a card, because that costs one linear walk, while
/// `false` for an event that does end one would leave the transcript showing a
/// card as pending after it returned — the "reports success while the thing did
/// not happen" failure this repository treats as the worst one available.
///
/// So this names the arms that can carry a `ToolResult` or touch a card at all,
/// and [`AgentEvent::MessageUpdate`] is the one that is out: it is a delta of
/// assistant prose.
fn event_can_end_a_card(ev: &AgentEvent) -> bool {
    !matches!(ev, AgentEvent::MessageUpdate { .. })
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
mod session;

mod editing;

mod completion;

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
    draw_at_file, draw_auth_picker, draw_auth_prompt, draw_dialog, draw_label_prompt, draw_palette,
    draw_picker, draw_session_browser, draw_slash_autocomplete, draw_timeline, draw_transcript,
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
    // Tree sidebar, docked RIGHT with a one-column gutter: transcript | gap | tree.
    // The pane scales with the window (30% clamped to 24–38 columns) instead of a
    // fixed width, and the transcript keeps its own floor (`MIN_WIDTH`);
    // narrower than that, the tree draws as a full-area overlay after the
    // shell — one draw function, two rects.
    let side_w = (area.width * 30 / 100).clamp(24, 38);
    let sidebar_wide = app.tree_open && area.width >= side_w + 1 + crate::ui::layout::MIN_WIDTH;
    let main = if sidebar_wide {
        let chunks = ratatui::layout::Layout::default()
            .direction(ratatui::layout::Direction::Horizontal)
            .constraints([
                ratatui::layout::Constraint::Min(crate::ui::layout::MIN_WIDTH),
                ratatui::layout::Constraint::Length(1),
                ratatui::layout::Constraint::Length(side_w),
            ])
            .split(area);
        crate::tree::draw_tree(
            f,
            chunks[2],
            &app.tree_rows,
            app.tree_cursor,
            app.tree_capped,
        );
        chunks[0]
    } else {
        area
    };
    // The composer wraps to the same width the shell will draw it at, so the
    // caret lands in the column the text actually occupies.
    let draft_width = crate::ui::shell::composer_text_width(main.width, &app.theme.glyphs);
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
    app.geometry = app.measure_transcript_window(main, draft_count);
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
        // **Guidance mode is the one overlay the composer is still typing into.**
        //
        // `overlay_open()` includes `pending_ask`, which is right for every other overlay —
        // the picker and the palette own the keyboard — and wrong here. Pressing `e` hands
        // the composer to the ask, and the caret then vanished: guidance mode was the only
        // state in the application where you typed into a field with no cursor in it. A
        // writer who cannot see where they are typing deletes the wrong thing.
        composer_focused: app.guidance_mode || !app.overlay_open(),
        transcript_empty: app.transcript_is_empty(),
        // Session first, branch second: a session label is the thing a user can
        // act on, and the branch is context for it. Falls back to the branch alone.
        session_label: &session_label,
    };
    let geometry = app.geometry;
    let painted = crate::ui::shell::paint(f, main, &shell, |f, transcript_area| {
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
    // M196: its OWN `if`. This was originally written inside the auth-prompt arm
    // above, so the label editor was drawn only while an API-key prompt was open
    // -- which never happens, so the feature was on no screen at all. The state
    // tests all passed; only `the_label_prompt_and_jump_notice_draw` could see it.
    if app.label_prompt_open {
        draw_label_prompt(f, app, f.area());
    }
    if app.auth_pick_open {
        draw_auth_picker(f, app, f.area());
    }
    if app.timeline_open {
        draw_timeline(f, app, f.area());
    }
    if app.tree_open && !sidebar_wide {
        // The overlay covers the transcript: clear first, or the two texts
        // interleave on narrow screens (read on a 60x20 frame).
        f.render_widget(ratatui::widgets::Clear, f.area());
        crate::tree::draw_tree(
            f,
            f.area(),
            &app.tree_rows,
            app.tree_cursor,
            app.tree_capped,
        );
    }
    if let Some(viewer) = app.viewer.as_mut() {
        // Disjoint field borrows: the viewer read and the capacity write touch
        // different fields, so no clone crosses the frame.
        crate::tree::draw_viewer(f, f.area(), &app.theme, viewer, &mut app.viewer_capacity);
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

#[cfg(test)]
mod goal_tests;

#[cfg(test)]
mod startup_tests;

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
mod tests_support;

#[cfg(test)]
mod frame_dump;

#[cfg(test)]
mod golden;

#[cfg(test)]
mod panic_tests;

#[cfg(test)]
mod tests;

/// Move the cursor to the next occurrence of `needle`, forward or backward.
///
/// Ported from pi's `jumpToChar` (`tui/src/components/editor.ts:2126-2160`),
/// including the two details that decide whether it feels right:
///
/// - **On the current line the search starts AFTER (or BEFORE) the cursor**;
///   on other lines it starts at the beginning (or end). Without that, a jump
///   forward to the character under the cursor lands on the cursor and appears
///   to do nothing.
/// - **Forward uses the first match, backward uses the LAST** — `indexOf` vs
///   `lastIndexOf` (`editor.ts:2145`).
///
/// Returns the new cursor position, or `None` when the character does not occur
/// anywhere ahead of (or behind) the cursor. Returning `None` rather than
/// clamping matters: the caller reports "not found" instead of silently leaving
/// the cursor somewhere the reader did not ask for.
pub(crate) fn jump_to_char(
    input: &str,
    cursor_chars: usize,
    needle: char,
    dir: JumpDir,
) -> Option<usize> {
    let chars: Vec<char> = input.chars().collect();
    let total = chars.len();
    if total == 0 {
        return None;
    }
    match dir {
        JumpDir::Forward => {
            // Strictly after the cursor: `cursor + 1 ..`, so the character under
            // the cursor is not a result.
            ((cursor_chars + 1)..total).find(|&i| chars[i] == needle)
        }
        JumpDir::Backward => {
            // Strictly before it. A forward range cannot express "the last
            // match going back", so this scans downward from the cursor.
            (0..cursor_chars.min(total))
                .rev()
                .find(|&i| chars[i] == needle)
        }
    }
}

/// The number of rows from the bottom to the nearest user turn in `direction`.
///
/// pi's `scrollToPrompt` (`tui-alt-screen.ts:485-495`) walks rows from the
/// current position looking for an OSC-133 prompt marker. Epsilon emits no such
/// markers, so the same idea is expressed against the transcript's own structure:
/// a **user turn** is a prompt, and `LineKind::User` already says so.
///
/// The two details that make it feel right, both from pi's loop:
///
/// - **Start one row away from the current position** (`scrollTop + direction`),
///   not at it. A prompt the reader is already looking at is not a jump.
/// - **Give up at the ends and say so**, rather than clamping to the first or
///   last prompt. Clamping is how "you are at the top" becomes a key that
///   appears broken.
///
/// `None` means there is no prompt that way, and the caller names which way.
pub(crate) fn jump_to_prompt(
    lines: &[crate::state::TranscriptLine],
    scroll_from_bottom: usize,
    direction: i32,
) -> Option<usize> {
    let total = lines.len();
    if total == 0 {
        return None;
    }
    // The current position, measured from the bottom, as a row index.
    let here = total.saturating_sub(scroll_from_bottom + 1);
    let step = if direction < 0 { -1isize } else { 1isize };
    let mut row = here as isize + step;
    while row >= 0 && (row as usize) < total {
        if lines[row as usize].kind == crate::state::LineKind::User {
            // Convert back to "rows from the bottom", which is what the view reads.
            return Some(total.saturating_sub(row as usize + 1));
        }
        row += step;
    }
    None
}

/// Direction for an editor jump-to-character, pi's `"forward" | "backward"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum JumpDir {
    /// To the next occurrence after the cursor.
    Forward,
    /// To the previous occurrence before it.
    Backward,
}

/// Which entries the `/tree` picker shows — pi's four `FilterMode`s
/// (`tree-selector.ts:365-385`).
///
/// **`Default` is not "everything".** It hides bookkeeping entries (a label, a
/// model change, a settings record), because a reader looking for "where was
/// that?" does not want the configuration change that happened in between. That
/// is the single most surprising rule in pi's filter, and it is the reason the
/// enum exists rather than a bare `Option<Kind>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum TreeFilter {
    /// Bookkeeping entries hidden; messages, tools, and compactions shown.
    #[default]
    Default,
    /// Only what the user said.
    UserOnly,
    /// The default, minus tool results.
    NoTools,
    /// Only entries carrying a label.
    LabeledOnly,
    /// Everything, bookkeeping included.
    All,
}

impl TreeFilter {
    /// The label shown in the picker's title bar.
    pub fn label(self) -> &'static str {
        match self {
            TreeFilter::Default => "default",
            TreeFilter::UserOnly => "user only",
            TreeFilter::NoTools => "no tools",
            TreeFilter::LabeledOnly => "labelled only",
            TreeFilter::All => "all",
        }
    }

    /// The next mode, in pi's cycle order (`tree-selector.ts:1067`).
    pub fn next(self) -> Self {
        match self {
            TreeFilter::Default => TreeFilter::UserOnly,
            TreeFilter::UserOnly => TreeFilter::NoTools,
            TreeFilter::NoTools => TreeFilter::LabeledOnly,
            TreeFilter::LabeledOnly => TreeFilter::All,
            TreeFilter::All => TreeFilter::Default,
        }
    }
}

/// One row of the `/tree` + `/timeline` picker.
///
/// A named type rather than a tuple because the tuple grew four fields, and
/// `(a, b, c, d)` in a `for` loop is how `tags` and `snippets` get swapped
/// without a compiler noticing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TimelineRow {
    /// Entry sequence number, as written in the session file.
    pub seq: u64,
    /// Entry id, so a selection can navigate rather than merely display.
    pub id: String,
    /// Short kind tag (`user`, `assistant`, `label`, ...).
    pub kind: String,
    /// First line of the entry's text, already truncated for display.
    pub snippet: String,
    /// Nesting depth in the session forest; 0 is a root.
    pub depth: usize,
    /// Whether this row is on the lane the session is currently on.
    pub on_active: bool,
    /// This entry's parent, or `None` for a root.
    ///
    /// M196: needed by the filter, which re-parents a row whose parent the
    /// filter hid. Depth alone cannot answer "is my parent on screen?" — it can
    /// only say how deep I was in the full tree.
    pub parent: Option<String>,
    /// The label on this entry, if it has one.
    ///
    /// M196: without this the picker could not show a label, so a label the
    /// store held was invisible in the one view that exists to navigate them.
    pub label: Option<String>,
}
