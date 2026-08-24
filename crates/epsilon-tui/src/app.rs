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
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use crate::state::{LineKind, UiState};

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

struct App {
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
}

impl App {
    #[allow(clippy::too_many_arguments)]
    fn new(
        model_id: String,
        fetched_tx: tokio::sync::mpsc::UnboundedSender<Result<Vec<String>, String>>,
        notices_tx: tokio::sync::mpsc::UnboundedSender<String>,
    ) -> Self {
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
        }
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
        self.ui.push_local_input(&text);
        Message::User(UserMessage {
            id: new_id("msg"),
            timestamp_ms: now_ms(),
            content: vec![UserContent::Text { text }],
        })
    }

    /// Submit the editor line; returns `true` when a command requested quit.
    fn submit_editor(&mut self, agent: &Arc<Agent>) -> bool {
        let text = std::mem::take(&mut self.input);
        self.cursor_chars = 0;
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
        }
    }

    /// Apply the currently highlighted picker row.
    fn pick_selected(&mut self, agent: &Arc<Agent>) {
        let rows =
            crate::picker::build_rows(&self.input, &self.model_id, &self.recents, &self.live_cache);
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
        let Some(fetcher) = self.live.as_ref() else {
            self.notice("live listing unavailable: built without --features real");
            return;
        };
        let rows =
            crate::picker::build_rows(&self.input, &self.model_id, &self.recents, &self.live_cache);
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
}

/// Run the interactive frontend until the user quits. Returns the process exit code.
pub async fn run(cfg: TuiConfig) -> i32 {
    let mut terminal = match ratatui::try_init() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("terminal init failed: {e}");
            return 1;
        }
    };
    let code = drive(cfg, &mut terminal).await;
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

    if let Some(task) = initial_task {
        let msg = app.submit_text(task);
        app.spawn_run(Arc::clone(&agent), msg);
    }

    let mut term_events = EventStream::new();
    let mut tick = tokio::time::interval(std::time::Duration::from_millis(TICK_MS));

    loop {
        let _ = terminal.draw(|f| draw(f, &app));

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
                            app.ui.push_notice(format!("live listing: {n} models merged into /models"));
                        }
                        Err(e) => app.ui.push_notice(format!("live listing failed: {e}")),
                    }
                    app.fetching_hint = None;
                }
            },

            maybe_ev = term_events.next() => {
                let Some(Ok(ev)) = maybe_ev else { break };
                if let TermEvent::Key(key) = ev
                    && key.kind == KeyEventKind::Press
                    && handle_key(&mut app, &agent, key)
                {
                    break;
                }
            },

            _ = tick.tick() => {},
        }
    }
    0
}

/// Apply one event; any newly appended transcript row resets follow-scroll pinning.
fn apply_with_scroll(app: &mut App, ev: &AgentEvent) {
    let before = app.ui.lines().len();
    app.ui.apply(ev);
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
                let total = crate::picker::build_rows(
                    &app.input,
                    &app.model_id,
                    &app.recents,
                    &app.live_cache,
                )
                .len();
                app.picker_index = app
                    .picker_index
                    .saturating_sub(1)
                    .min(total.saturating_sub(1));
            }
            KeyCode::Down => {
                let total = crate::picker::build_rows(
                    &app.input,
                    &app.model_id,
                    &app.recents,
                    &app.live_cache,
                )
                .len();
                if total > 0 {
                    app.picker_index = (app.picker_index + 1).min(total - 1);
                }
            }
            KeyCode::Backspace => {
                app.backspace();
                let total = crate::picker::build_rows(
                    &app.input,
                    &app.model_id,
                    &app.recents,
                    &app.live_cache,
                )
                .len();
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

fn style_for(kind: LineKind) -> Style {
    match kind {
        LineKind::User => Style::default().fg(Color::Cyan),
        LineKind::Assistant => Style::default(),
        LineKind::Thinking => Style::default().fg(Color::DarkGray),
        LineKind::ToolCall => Style::default().fg(Color::Yellow),
        LineKind::ToolResultOk => Style::default().fg(Color::Green),
        LineKind::ToolResultErr => Style::default().fg(Color::Red),
        LineKind::System => Style::default().fg(Color::DarkGray),
    }
}

fn transcript_rows(app: &App) -> Vec<Line<'static>> {
    let mut rows: Vec<Line<'static>> = Vec::with_capacity(app.ui.lines().len() + 2);
    for l in app.ui.lines() {
        // M13 card polish: expandable rows show their full payload when toggled.
        if app.prefs.expand_outputs
            && let Some(full) = l.full.as_ref()
        {
            rows.push(Line::from(Span::styled(full.clone(), style_for(l.kind))));
            continue;
        }
        if l.code {
            rows.push(Line::from(Span::styled(
                l.text.clone(),
                Style::default().fg(Color::DarkGray),
            )));
            continue;
        }
        rows.push(Line::from(Span::styled(l.text.clone(), style_for(l.kind))));
    }
    let streaming = app.ui.streaming_text();
    if !streaming.is_empty() {
        rows.push(Line::from(Span::styled(
            format!("{streaming}▌"),
            Style::default().add_modifier(Modifier::DIM),
        )));
    }
    rows
}

fn draw(f: &mut Frame<'_>, app: &App) {
    let [transcript_area, input_area, status_area] = Layout::vertical([
        Constraint::Min(3),
        Constraint::Length(3),
        Constraint::Length(1),
    ])
    .areas(f.area());

    draw_transcript(f, app, transcript_area);
    draw_input(f, app, input_area);
    draw_status(f, app, status_area);

    if app.picker_open {
        draw_picker(f, app, f.area());
    }
    if app.pending_ask.is_some() {
        draw_dialog(f, app, f.area());
    }
}

/// `/models` overlay: filter-driven list with provider grouping hints and an auth
/// column. The active model floats to the top (picker::build_rows).
fn draw_picker(f: &mut Frame<'_>, app: &App, outer: Rect) {
    let rows = crate::picker::build_rows(&app.input, &app.model_id, &app.recents, &app.live_cache);
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
    let mut last_provider = String::new();
    for (i, row) in rows.iter().enumerate() {
        // Group separators between distinct providers/groups.
        if row.provider != last_provider {
            last_provider = row.provider.clone();
            lines.push(Line::from(Span::styled(
                format!("─ {last_provider} "),
                Style::default().fg(Color::DarkGray),
            )));
        }
        let selected = i == app.picker_index;
        let marker = match row.id == app.model_id {
            true => "● ",
            false => "  ",
        };
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
                format!("  · {}", row.id),
                Style::default().fg(Color::DarkGray),
            ),
        ]));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "↑/↓ move · type to filter · Ctrl+F fetch live · Enter apply · Esc cancel",
        Style::default().fg(Color::DarkGray),
    )));

    let para = Paragraph::new(lines).block(block);
    f.render_widget(para, area);
}

fn draw_transcript(f: &mut Frame<'_>, app: &App, area: Rect) {
    let rows = transcript_rows(app);
    let total = rows.len();
    let visible = area.height as usize;

    // Follow mode pins the tail; PageUp/PageDown pin an offset above the bottom.
    let skip = if total > visible {
        (total - visible).saturating_sub(app.scroll_from_bottom.min(total - visible))
    } else {
        0
    };

    let para = Paragraph::new(rows)
        .wrap(Wrap { trim: false })
        .scroll((skip as u16, 0));
    f.render_widget(para, area);
}

fn draw_input(f: &mut Frame<'_>, app: &App, area: Rect) {
    let title = if app.guidance_mode {
        "guidance for denial — Enter sends, Esc cancels"
    } else if app.ui.running() {
        "input — typing during a run queues steering"
    } else {
        "input"
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
    let para = Paragraph::new(Line::from(vec![
        Span::styled("> ", Style::default().fg(prompt_color)),
        Span::raw(app.input.clone()),
    ]))
    .block(frame_box);
    f.render_widget(para, area);

    let col = area
        .x
        .saturating_add(2)
        .saturating_add(app.cursor_chars.min(inner_width.saturating_sub(1)) as u16);
    f.set_cursor_position((col, area.y + 1));
}

fn draw_status(f: &mut Frame<'_>, app: &App, area: Rect) {
    let (tin, tout) = app.ui.usage();
    let model = epsilon_llm::catalog::display_label(&app.model_id);
    // Cost from catalog placeholder prices when the model is known (ADR-010 item 5;
    // integer micro-USD, no float drift). Unknown/custom ids omit cost honestly.
    let cost = match epsilon_llm::catalog::lookup_model(&app.model_id) {
        Some(info) => {
            let micro = tin * info.input_cost_micro_per_mtok / 1_000_000
                + tout * info.output_cost_micro_per_mtok / 1_000_000;
            format!(" · ${:.4}", micro as f64 / 1_000_000.0)
        }
        None => String::new(),
    };
    let right = format!("{model}{cost} · tokens {tin}/{tout}");
    let left_len = app.ui.status().chars().count();
    let pad = (area.width as usize)
        .saturating_sub(left_len + right.chars().count())
        .max(1);
    let line = Line::from(vec![
        Span::styled(
            app.ui.status().to_string(),
            Style::default().fg(Color::DarkGray),
        ),
        Span::raw(" ".repeat(pad)),
        Span::styled(right, Style::default().fg(Color::DarkGray)),
    ]);
    f.render_widget(line, area);
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
}
