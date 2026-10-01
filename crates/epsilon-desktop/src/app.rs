//! Application state and the frame.
//!
//! # The shape of a frame
//!
//! [`IdeApp::paint`] is a **pure-ish function of `(&mut self, &egui::Ui)`** that returns a
//! [`FrameSummary`] naming what it drew. That is not instrumentation for its own sake: the
//! terminal frontend keeps exactly this shape (`render_rows` → rows), and it is what lets a
//! test assert on *the screen* rather than on state that merely ought to produce a screen.
//! A feature can be correct at every level and still never appear — that has happened in
//! this repository twice, and the fix was always the same.
//!
//! # What this pane believes
//!
//! It believes exactly what the agent told it. Every number drawn here comes from
//! [`epsilon_core::Agent::session_state`], which is the same constructor the HTTP surface
//! and the stdio RPC surface use — so the three frontends cannot disagree about the same
//! session, and a number this window shows is a number the CLI would show.
//!
//! When there is nothing to say, it says nothing: before M233's wiring landed, this pane
//! said "no agent attached" rather than inventing a cost. That rule has not changed, and a
//! test still enforces it for the case where there is genuinely no agent.
use crate::Options;
use crate::layout::{IdeLayout, Preferences, Tier};
use crate::session::{IdeSession, RowKind, Transcript};
use crate::theme::IdeTheme;
use epsilon_contracts::agent_event::PermissionDecision;
use std::sync::Arc;
/// What one painted frame contained.
///
/// Region name plus the text drawn into it. Rectangles come from [`IdeApp::layout`], so a
/// test can assert both *that* something is on screen and *where*.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FrameSummary {
    /// `(region, text)` in paint order.
    pub rows: Vec<(String, String)>,
    /// `(name, selected)` for each open tab, so a test can say which one is showing.
    pub tabs: Vec<(String, bool)>,
    /// `(id, rect)` for every thing this frame made clickable.
    ///
    /// Present because a painted row has no response of its own, so anything that needs
    /// to know where one was drawn — a test pressing a tree row, a keyboard shortcut
    /// moving the caret to the selection — cannot recover it after the fact. Cheap to
    /// record while the rectangle is in scope, and impossible to reconstruct from the
    /// text alone because indentation is not geometry.
    pub hotspots: Vec<(String, egui::Rect)>,
    /// `(region, rect)` for every painted row, so a test can ask **where** it landed.
    ///
    /// Added after a measurement rather than a review: the panes used to share one `Ui`,
    /// so the transcript was drawn at `[0,142]` while its pane was at `[920,42]` — on top
    /// of the file tree — and 103 tests passed, because `rows` records *what* was drawn
    /// and nothing recorded *where*. `hotspots` above had the same reason to exist.
    pub row_rects: Vec<(String, egui::Rect)>,
    /// Which degradation rule was in force.
    pub tier: Tier,
    /// Window size the frame was painted at, for the assertion message.
    pub size: [f32; 2],
}

impl FrameSummary {
    /// Every line drawn into `region`, in order.
    pub fn region(&self, region: &str) -> Vec<&str> {
        self.rows
            .iter()
            .filter(|(r, _)| r == region)
            .map(|(_, t)| t.as_str())
            .collect()
    }
    /// Where a clickable thing was drawn, by id.
    pub fn hotspot(&self, id: &str) -> Option<egui::Rect> {
        self.hotspots.iter().find(|(k, _)| k == id).map(|(_, r)| *r)
    }
    /// Whether `region` drew anything at all.
    pub fn drew(&self, region: &str) -> bool {
        self.rows.iter().any(|(r, _)| r == region)
    }
    /// A one-line rendering of the frame, for a test failure message.
    ///
    /// A failing assertion that prints `"the sidebar did not draw"` is actionable.
    /// One that prints a `FrameSummary` debug dump is not, quite.
    pub fn describe(&self) -> String {
        let mut out = format!(
            "frame at {}x{} (tier {}):\n",
            self.size[0],
            self.size[1],
            self.tier.name()
        );
        for (region, text) in &self.rows {
            out.push_str(&format!("  [{region}] {text}\n"));
        }
        out
    }
}
/// A `Ui` of its own, clipped to `rect`.
///
/// Every pane gets one. The alternative — sharing the root `Ui` — makes the panes a
/// linked list: each starts where the last one stopped, so the file tree decides where
/// the conversation is drawn. This returns a `Ui` with its own cursor starting at
/// `rect.min`, and a clip rect that stops a long line from painting into the pane next
/// door. A painter's clip is not decoration here: `OpenFile` deliberately keeps whole
/// lines, so a 400-character line in a 500-pixel editor was a 400-character line drawn
/// across the transcript.
fn pane_ui(ui: &egui::Ui, rect: egui::Rect, id: &str) -> egui::Ui {
    egui::Ui::new(
        ui.ctx().clone(),
        ui.id().with(id),
        egui::UiBuilder::new()
            .max_rect(rect)
            .layer_id(ui.layer_id())
            .layout(egui::Layout::top_down(egui::Align::LEFT)),
    )
}

/// The desktop application.
pub struct IdeApp {
    opts: Options,
    theme: IdeTheme,
    prefs: Preferences,
    /// The harness. `None` only in the tests that assert the *absent* state.
    ///
    /// Shared, because a caller that drives a turn from outside the draw path — which is
    /// what a test does, and what an in-app shortcut will — needs a handle that outlives
    /// the borrow the frame takes.
    session: Option<Arc<IdeSession>>,
    /// What the launch said, in order. Shown before anything runs.
    notices: Vec<String>,
    /// The transcript, reduced from the agent's events.
    transcript: Transcript,
    /// The composer's draft.
    draft: String,
    /// What the composer should say when it is empty.
    placeholder: &'static str,
    /// Set when the last submit was refused, so the pane can say why.
    refusal: Option<String>,
    /// The workspace's files, walked once and folded into a tree.
    tree: crate::workspace::FileTree,
    /// The sidebar's filter box.
    tree_query: String,
    /// The file open in the editor, if any.
    open: Option<crate::workspace::OpenFile>,
    /// Open files, oldest first; the last is what the editor shows.
    tabs: Vec<String>,
    /// Which view the sidebar is showing, chosen from the activity bar.
    sidebar: SidebarView,
    /// Geometry of the most recent frame, kept for hit-testing and for tests.
    layout: IdeLayout,
}
/// Shorten `text` so it fits `points` at `font_size`, marking that it was cut.
///
/// # Why this exists rather than letting the widget clip
///
/// egui clips a label that overflows **silently**. A row that ends mid-word with no marker
/// is indistinguishable from the whole value, and the two failures are opposites: a reader
/// who believes a truncated model name is the full one is misled about what ran; a reader
/// who believes a truncated description is complete is misled about what the agent is doing.
/// This is the same rule this repository keeps arriving at — **a partial value must say it
/// is partial** — and a diff line cut without a marker is the identical mistake elsewhere.
///
/// # The budget is an approximation, and deliberately so
///
/// It assumes an average advance of `AVG_ADVANCE` ems, which holds to within about a
/// fifth for proportional text and is wrong for a run of narrow glyphs. That is acceptable
/// *because the result is only ever shortened, never lengthened*: the budget is
/// deliberately conservative, so the worst case is a row a little shorter than it could have
/// been. A budget that could drop a readable character would not be acceptable.
pub fn fit(text: &str, points: f32, font_size: f32) -> String {
    let budget = (points / (font_size * AVG_ADVANCE)).floor().max(4.0) as usize;
    if text.chars().count() <= budget {
        return text.to_string();
    }
    let kept: String = text.chars().take(budget.saturating_sub(1)).collect();
    format!("{kept}…")
}

/// Height of the editor's tab strip.
const TAB_H: f32 = 28.0;

/// Largest width a tab may take; the strip shares what is left between its tabs.
const TAB_W: f32 = 96.0;

/// Font a tab's label is drawn at, and the one the label's [`fit`] budget assumes.
const TAB_FONT: f32 = 12.0;

/// Average glyph advance as a fraction of the font size, for proportional text.
const AVG_ADVANCE: f32 = 0.55;

/// The font the sidebar's rows are drawn at, and the one [`fit`] assumes.
const ROW_FONT: f32 = 13.0;

impl IdeApp {
    /// Build the application around an already-composed session.
    pub fn new(
        opts: Options,
        session: IdeSession,
        notices: Vec<String>,
        ctx: &egui::Context,
    ) -> Self {
        // Start listening before the first frame: an event that arrives between the
        // window opening and the bridge attaching would be dropped, and the first thing a
        // user does is type.
        session.attach_bridge(ctx);
        let theme = IdeTheme::resolve(opts.theme);
        // Loaded before opts moves, so the tree is built from the workspace the caller
        // named rather than from a borrow of a moved value.
        let tree = crate::workspace::FileTree::load(&opts.workspace);
        let transcript = crate::session::Transcript::new(&opts.workspace);
        let layout = IdeLayout::new(
            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 820.0)),
            Preferences::default(),
        );
        Self {
            opts,
            theme,
            prefs: Preferences::default(),
            session: Some(Arc::new(session)),
            notices,
            transcript,
            draft: String::new(),
            placeholder: "ask anything  ·  enter sends  ·  shift+enter for a new line",
            refusal: None,
            tree,
            tree_query: String::new(),
            open: None,
            tabs: Vec::new(),
            sidebar: SidebarView::Files,
            layout,
        }
    }
    /// An application with **no** agent, for the test that asserts the absent state.
    ///
    /// Exists because "no agent attached" is a state a user can genuinely reach and a
    /// frame that draws a plausible model for it would be a lie — the M233 defect. The
    /// only way to test that is to be able to build the frame.
    pub fn without_agent(opts: Options) -> Self {
        let tree = crate::workspace::FileTree::load(&opts.workspace);
        let transcript = crate::session::Transcript::new(&opts.workspace);
        Self {
            opts,
            theme: IdeTheme::resolve(None),
            prefs: Preferences::default(),
            session: None,
            notices: vec!["not connected to an agent".into()],
            transcript,
            draft: String::new(),
            placeholder: "no agent attached",
            refusal: None,
            tree,
            tree_query: String::new(),
            open: None,
            tabs: Vec::new(),
            sidebar: SidebarView::Files,
            layout: IdeLayout::new(
                egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 820.0)),
                Preferences::default(),
            ),
        }
    }
    /// The geometry of the last painted frame.
    pub fn layout(&self) -> &IdeLayout {
        &self.layout
    }
    /// The workspace this window is bound to.
    pub fn workspace(&self) -> &std::path::Path {
        &self.opts.workspace
    }
    /// The transcript, for a caller that wants to read rather than draw.
    pub fn transcript(&self) -> &Transcript {
        &self.transcript
    }
    /// The harness this window is driving, if it has one.
    ///
    /// Public because it is the only way to drive a turn from outside the draw path, and
    /// a test that has to go through a frame to submit a prompt is a test of the frame.
    pub fn session(&self) -> Option<&Arc<IdeSession>> {
        self.session.as_ref()
    }
    /// What the composer currently holds.
    pub fn draft(&self) -> &str {
        &self.draft
    }
    /// Replace the transcript.
    ///
    /// For a test that needs a particular history without running a turn to produce it.
    /// `None` clears it.
    pub fn seed_transcript(&mut self, transcript: Transcript) {
        self.transcript = transcript;
    }
    /// Put a draft in the composer, as typing would.
    pub fn set_draft(&mut self, text: &str) {
        self.draft = text.to_string();
    }
    /// Which view the sidebar is showing.
    pub fn sidebar_view(&self) -> SidebarView {
        self.sidebar
    }
    /// Choose the sidebar's view directly.
    ///
    /// The activity bar does this with a click; a test that has to drive three frames
    /// of hit-testing to change one field would be testing the input system instead.
    pub fn set_sidebar_view(&mut self, view: SidebarView) {
        self.sidebar = view;
    }
    /// Drain the agent's event queue into the transcript.
    ///
    /// Called from [`IdeApp::paint`] **and** from [`eframe::App::logic`], and both call
    /// sites are needed. `logic` is the one that matters for correctness — eframe skips
    /// `ui` entirely while the window is hidden, so a run that finished in the background
    /// would otherwise never reach the screen. `paint` is the one that matters for tests,
    /// which drive the draw path directly.
    ///
    /// Twice in a frame is not twice the work: the drain is a queue read, and the second
    /// call finds it empty. Making it conditional on *how* the frame was reached would
    /// leave two paths that can disagree about what is on screen.
    fn pump(&mut self) {
        // Through the `Arc`, and that is the whole reason `drain` takes `&self`: anything
        // else holding this session — a shortcut, a second window, a test — must not be
        // able to stop the frame from updating.
        if let Some(session) = self.session.as_ref() {
            session.drain(&mut self.transcript);
        }
    }
    /// Paint one frame and describe what it drew.
    ///
    /// This is the whole of the draw path. [`eframe::App::ui`] is a thin wrapper so a test
    /// can paint into an off-screen `egui::Context` at any size, with no window.
    pub fn paint(&mut self, ui: &mut egui::Ui) -> FrameSummary {
        let screen = ui.ctx().content_rect();
        self.layout = IdeLayout::new(screen, self.prefs);
        let l = self.layout;
        let mut summary = FrameSummary {
            tier: l.tier,
            size: [screen.width(), screen.height()],
            ..FrameSummary::default()
        };
        // The drain lives in `pump`, which both `paint` and `eframe::App::logic` call — see
        // there for why neither call site can be the only one.
        self.pump();
        // **Approvals, before anything else can want a key.** While a run is parked on a
        // gate, `Enter` and `Escape` belong to that gate. The composer below reads the same
        // two keys to send and to abort, so whichever ran first decided what those keys
        // meant — and the ask was painted with `[enter] allow [esc] refuse` while the
        // composer consumed both. `approve_pending` existed, was correct, had a test, and
        // **had no caller in the crate**: the window drew a gate it could not open.
        if let Some(s) = self.session.clone() {
            approve_pending(&s, ui.ctx());
        }
        // ---- window chrome --------------------------------------------------
        let painter = ui.painter();
        if l.title.height() > 0.0 {
            painter.rect_filled(l.title, 0.0, self.theme.panel);
            painter.line_segment(
                [l.title.left_bottom(), l.title.right_bottom()],
                egui::Stroke::new(1.0, self.theme.border),
            );
            let mut line = self
                .opts
                .workspace
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("Epsilon")
                .to_string();
            line.push_str(&self.model_suffix());
            painter.text(
                l.title.left_top() + egui::vec2(14.0, 23.0),
                egui::Align2::LEFT_CENTER,
                &line,
                egui::FontId::proportional(15.0),
                self.theme.title,
            );
            summary.rows.push(("title".into(), line));
        }
        if l.status.height() > 0.0 {
            painter.rect_filled(l.status, 0.0, self.theme.panel);
            painter.line_segment(
                [l.status.left_top(), l.status.right_top()],
                egui::Stroke::new(1.0, self.theme.border),
            );
            let text = self.status_line();
            painter.text(
                l.status.left_top() + egui::vec2(14.0, 12.0),
                egui::Align2::LEFT_CENTER,
                &text,
                egui::FontId::proportional(12.0),
                self.theme.dim,
            );
            summary.rows.push(("status".into(), text));
        }
        // ---- panes ----------------------------------------------------------
        //
        // **Each pane gets its own `Ui`, pinned to its own rect.** They used to share the
        // root `Ui`, whose cursor walks: whichever pane drew first moved it, and the next
        // pane started wherever the previous one stopped. Measured, not reasoned — with
        // the agent pane at `[920,42]-[1440,870]`, the transcript laid itself out at
        // `[0,142]-[504,817]`, which is on top of the file tree, leaving the right-hand
        // pane empty. Every pane painted text correctly and the conversation was invisible
        // behind the sidebar.
        //
        // `UiBuilder::max_rect` alone does not fix it: `scope_builder` confines what is
        // drawn and still advances the parent's cursor to the scope's edge. A fresh `Ui`
        // has its own cursor and its own clip rect, which is what a pane is.
        if l.activity.width() > 0.0 {
            let mut pane = pane_ui(ui, l.activity, "activity");
            self.activity_bar(&mut pane, &mut summary, l.activity);
        }
        if l.sidebar_visible {
            let mut pane = pane_ui(ui, l.sidebar, "sidebar");
            self.workspace_pane(&mut pane, &mut summary, l.sidebar);
        }
        if l.center_visible {
            let mut pane = pane_ui(ui, l.center, "center");
            self.editor_pane(&mut pane, &mut summary, l.center);
        }
        if l.agent_visible {
            let mut pane = pane_ui(ui, l.agent, "agent");
            self.agent_panel(&mut pane, &mut summary, l.agent);
        }
        summary
    }
    /// The icon strip that chooses what the sidebar shows.
    ///
    /// Two views, because that is what an agent IDE needs to switch between: the **files**
    /// the agent can act on, and the **subagents** it is already running. One pane trying
    /// to hold both would put a task list under a file tree, and a reader would scroll past
    /// a thousand paths to find out whether anything was still running.
    ///
    /// Drawn as widgets rather than painted, because it is clickable, and a painted strip
    /// is a picture of a control.
    fn activity_bar(&mut self, ui: &mut egui::Ui, summary: &mut FrameSummary, rect: egui::Rect) {
        let theme = self.theme;
        ui.painter().rect_filled(rect, 4.0, theme.panel);
        ui.painter().rect_stroke(
            rect,
            4.0,
            egui::Stroke::new(1.0, theme.border),
            egui::StrokeKind::Inside,
        );
        let mut y = rect.top() + 10.0;
        for view in SidebarView::ALL {
            let selected = view == self.sidebar;
            let button = ui
                .put(
                    egui::Rect::from_min_size(
                        egui::pos2(rect.left() + 6.0, y),
                        egui::vec2((rect.width() - 12.0).max(8.0), 30.0),
                    ),
                    egui::Button::new(view.glyph())
                        .selected(selected)
                        .fill(if selected {
                            theme.selection_focus
                        } else {
                            theme.panel
                        }),
                )
                // The glyph is a picture of the view; the label is its name. Both existed and
                // only the glyph reached the screen, so the one control in this window that
                // chooses what everything else shows was three unlabelled symbols. The name is
                // a tooltip rather than a second line of text because the bar is 48 pixels
                // wide, and 48 pixels is not enough to set a word in.
                .on_hover_text(view.label());
            summary
                .hotspots
                .push((crate::workspace::hotspot_id(view.key()), button.rect));
            summary.rows.push((
                "activity".into(),
                format!("{} {}", view.glyph(), view.label()),
            ));
            if button.clicked() {
                self.sidebar = view;
            }
            y += 34.0;
        }
    }
    /// The workspace pane: whichever view the activity bar selected.
    fn workspace_pane(&mut self, ui: &mut egui::Ui, summary: &mut FrameSummary, rect: egui::Rect) {
        match self.sidebar {
            SidebarView::Files => self.files_pane(ui, summary, rect),
            SidebarView::Tasks => self.tasks_pane(ui, summary, rect),
            SidebarView::Sessions => self.sessions_pane(ui, summary, rect),
        }
    }
    /// The sidebar's tasks view: every subagent this session has dispatched.
    ///
    /// Read from [`IdeSession`]'s agent rather than kept here, because the agent owns the
    /// registry and a copy would go stale the moment a child finished — which is exactly
    /// when a reader is looking.
    fn tasks_pane(&self, ui: &mut egui::Ui, summary: &mut FrameSummary, rect: egui::Rect) {
        let theme = self.theme;
        {
            let painter = ui.painter();
            painter.rect_filled(rect, 4.0, theme.background);
            painter.rect_stroke(
                rect,
                4.0,
                egui::Stroke::new(1.0, theme.border),
                egui::StrokeKind::Inside,
            );
            painter.text(
                rect.left_top() + egui::vec2(12.0, 16.0),
                egui::Align2::LEFT_CENTER,
                "Subagents",
                egui::FontId::proportional(12.0),
                theme.dim,
            );
        }
        summary.rows.push(("tasks".into(), "Subagents".into()));
        let tasks = self
            .session
            .as_ref()
            .map(|s| s.agent().task_snapshots())
            .unwrap_or_default();
        if tasks.is_empty() {
            // The empty state says how to make one, because "no tasks" on its own reads
            // as "subagents are broken".
            let line = if self.session.is_some() {
                "none yet — the model starts them with the `task` tool"
            } else {
                "no agent attached"
            };
            ui.colored_label(theme.dim, line);
            summary.rows.push(("tasks".into(), line.to_string()));
            return;
        }
        let header = Self::task_header(&tasks);
        ui.colored_label(theme.dim, header.clone());
        summary.rows.push(("tasks".into(), header));
        for task in &tasks {
            // Both lines are elided to the pane: a model name cut mid-token is
            // indistinguishable from the whole model.
            for (color, text) in Self::task_lines(task, &theme) {
                let text = fit(&text, rect.width() - 16.0, ROW_FONT);
                ui.colored_label(color, text.clone());
                summary.rows.push(("tasks".into(), text));
            }
        }
    }
    /// The count line above the subagent list.
    ///
    /// `pluralise` because "1 task(s), 1 running" is the kind of row a reader stops reading,
    /// and it costs four characters to not do that.
    pub fn task_header(tasks: &[epsilon_contracts::tool::TaskSnapshot]) -> String {
        let running = tasks
            .iter()
            .filter(|t| t.state == epsilon_contracts::tool::TaskState::Running)
            .count();
        format!(
            "{} {}, {running} running",
            tasks.len(),
            Self::plural(tasks.len())
        )
    }
    fn plural(n: usize) -> &'static str {
        if n == 1 { "task" } else { "tasks" }
    }
    /// One task as the two lines it draws, coloured.
    ///
    /// Extracted from the draw path because **this is the part that was written and never
    /// checked**: the pane reached `Agent::task_snapshots()` and drew three markers, and no test
    /// asserted that a `Running` child looks different from a `Failed` one. A live delegation is
    /// a race — it can finish before the first frame — so the state machine is tested here and
    /// the *acquisition* stays where `epsilon-core`'s own `task_scenarios.rs` already covers it.
    pub fn task_lines(
        task: &epsilon_contracts::tool::TaskSnapshot,
        theme: &IdeTheme,
    ) -> Vec<(egui::Color32, String)> {
        use epsilon_contracts::tool::TaskState;
        let (marker, color) = match task.state {
            TaskState::Running => ("▸ ", theme.accent),
            TaskState::Completed => ("✓ ", theme.success),
            TaskState::Failed => ("✗ ", theme.danger),
        };
        vec![
            (color, format!("{marker}{}", task.description)),
            // The model and the turn count are what a reader cannot work out from the
            // description: a delegation that silently ran on a different model from the one
            // that was asked for is exactly the sort of thing worth catching.
            (
                theme.dim,
                format!(
                    "   {} · {} turn(s){}",
                    task.subagent_type,
                    task.turns,
                    task.model
                        .as_deref()
                        .map(|m| format!(" · {m}"))
                        .unwrap_or_default()
                ),
            ),
        ]
    }
    /// The conversations stored for this workspace, and a real switch.
    ///
    /// This view exists **only because it can switch**. A list of session files that could
    /// not be opened would be a list of files with extra steps, and a "Sessions" control
    /// that looks switchable and is not is the kind of thing this project keeps refusing to
    /// ship. The handover underneath is [`IdeSession::resume`], the same `live_resume` the
    /// terminal's `/resume` uses: release the lease, open the other file, repair it,
    /// re-derive the model view, restore the history, re-attach.
    fn sessions_pane(&mut self, ui: &mut egui::Ui, summary: &mut FrameSummary, rect: egui::Rect) {
        let theme = self.theme;
        {
            let painter = ui.painter();
            painter.rect_filled(rect, 4.0, theme.background);
            painter.rect_stroke(
                rect,
                4.0,
                egui::Stroke::new(1.0, theme.border),
                egui::StrokeKind::Inside,
            );
            painter.text(
                rect.left_top() + egui::vec2(12.0, 16.0),
                egui::Align2::LEFT_CENTER,
                "Sessions",
                egui::FontId::proportional(12.0),
                theme.dim,
            );
        }
        summary.rows.push(("sessions".into(), "Sessions".into()));
        let Some(session) = self.session.as_ref() else {
            let line = "no agent attached";
            ui.colored_label(theme.dim, line);
            summary.rows.push(("sessions".into(), line.to_string()));
            return;
        };
        let current = session.session_path();
        let sessions = session.sessions(&self.opts.workspace);
        if sessions.is_empty() {
            let line = "none stored for this workspace yet";
            ui.colored_label(theme.dim, line);
            summary.rows.push(("sessions".into(), line.to_string()));
            return;
        }
        let header = format!("{} stored", sessions.len());
        ui.colored_label(theme.dim, fit(&header, rect.width() - 16.0, ROW_FONT));
        summary.rows.push(("sessions".into(), header));
        for entry in &sessions {
            let is_current = current.as_deref() == Some(entry.path.as_path());
            let marker = if is_current { "● " } else { "  " };
            // Name first, timestamp second: the name is what a reader recognises, and
            // the timestamp is what they check afterwards.
            let name = entry
                .path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| entry.path.display().to_string());
            let shown = fit(&format!("{marker}{name}"), rect.width() - 16.0, ROW_FONT);
            let response = ui.selectable_label(is_current, shown.clone());
            summary.rows.push(("sessions".into(), shown.clone()));
            // **The age, on its own line.** The list is newest-first, so a reader who
            // wants the newest already has it — what they do not have is *evidence*.
            // "2h ago" is what turns an ordering into something they can check, and it
            // is the difference between a list and a judgement.
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            if let Some(age) = epsilon_core::human_age_ms(now, entry.modified_ms) {
                let line = format!("   {age} ago");
                ui.colored_label(theme.dim, line.clone());
                summary.rows.push(("sessions".into(), line));
            }
            // Clicking the session you are already in is neither an error nor a
            // spinner: it is the same conversation, and doing nothing is right.
            if response.clicked() && !is_current {
                match self.switch_session(&entry.path) {
                    Ok(restored) => summary.rows.push((
                        "sessions".into(),
                        format!("resumed {name} — {restored} message(s)"),
                    )),
                    Err(reason) => {
                        // **Two places, because only one of them is a screen.** The
                        // transcript is painted, so a refusal lands where the reader is
                        // already looking; `FrameSummary` is test instrumentation and
                        // paints nothing at all. The comment this replaced claimed the
                        // refusal was on screen while the only code that carried it was
                        // a vector a test reads and a user never does.
                        let text = format!("cannot resume {name}: {reason}");
                        self.transcript.push_notice(&text);
                        summary.rows.push(("sessions".into(), text));
                    }
                }
            }
        }
    }
    /// Resume `path` and rebuild the transcript from what came back.
    ///
    /// The rebuild is not optional. `live_resume` restores history onto the agent and
    /// **nothing emits an event for it**, so a window that only reduced events would switch
    /// files and show an empty pane — "nothing happened", reached by opening something.
    pub fn switch_session(&mut self, path: &std::path::Path) -> Result<usize, String> {
        let session = self
            .session
            .as_ref()
            .ok_or_else(|| "no agent attached".to_string())?;
        session.resume(path)?;
        let history = session.agent().history();
        let count = history.len();
        self.transcript = Transcript::from_messages(&history);
        Ok(count)
    }
    /// The file tree.
    ///
    /// Drawn with widgets rather than as text rows, so it can take clicks and a keyboard
    /// — a tree made of painted text is a picture of a tree.
    fn files_pane(&mut self, ui: &mut egui::Ui, summary: &mut FrameSummary, rect: egui::Rect) {
        let theme = self.theme;
        {
            let painter = ui.painter();
            painter.rect_filled(rect, 4.0, theme.background);
            painter.rect_stroke(
                rect,
                4.0,
                egui::Stroke::new(1.0, theme.border),
                egui::StrokeKind::Inside,
            );
            painter.text(
                rect.left_top() + egui::vec2(12.0, 16.0),
                egui::Align2::LEFT_CENTER,
                "Workspace",
                egui::FontId::proportional(12.0),
                theme.dim,
            );
        }
        summary.rows.push(("workspace".into(), "Workspace".into()));
        summary.rows.push((
            "workspace".into(),
            format!("workspace  {}", display_path(&self.opts.workspace)),
        ));
        let search = egui::Rect::from_min_size(
            egui::pos2(rect.left() + 8.0, rect.top() + 28.0),
            egui::vec2((rect.width() - 16.0).max(0.0), 24.0),
        );
        if search.width() > 60.0 {
            ui.put(
                search,
                egui::TextEdit::singleline(&mut self.tree_query)
                    .hint_text("filter files")
                    .desired_width(search.width() - 8.0),
            );
        }
        let body = egui::Rect::from_min_max(
            egui::pos2(rect.left() + 8.0, search.max.y + 6.0),
            egui::pos2(rect.right() - 8.0, rect.bottom() - 26.0),
        );
        if body.height() > 10.0 && body.width() > 20.0 {
            let selected = self.open.as_ref().map(|f| f.path.as_str());
            match crate::workspace::draw_tree(
                ui,
                &theme,
                body,
                &self.tree,
                &self.tree_query,
                selected,
                &mut summary.hotspots,
            ) {
                Some(crate::workspace::TreeClick::File(path)) => self.open_file(&path),
                Some(crate::workspace::TreeClick::Directory(path)) => self.tree.toggle(&path),
                None => {}
            }
        }
        // The footer of the pane states what the walk found, and says **partial** when it
        // stopped early. A tree that silently ends looks like a workspace with no more
        // files in it, which is a claim the pane cannot support.
        let note = if self.tree.is_truncated() {
            format!(
                "{}+ files · list is partial (raise the budget in code)",
                self.tree.row_count()
            )
        } else {
            format!("{} files", self.tree.row_count())
        };
        let painter = ui.painter();
        painter.text(
            egui::pos2(rect.left() + 12.0, rect.bottom() - 14.0),
            egui::Align2::LEFT_BOTTOM,
            &note,
            egui::FontId::proportional(11.0),
            theme.dim,
        );
        summary.rows.push(("workspace".into(), note));
    }
    /// The editor pane: the open file, read-only.
    fn editor_pane(&mut self, ui: &mut egui::Ui, summary: &mut FrameSummary, rect: egui::Rect) {
        let theme = self.theme;
        // Before the body is read, so a tab press shows its file in the same frame the
        // click landed — the alternative is a frame that shows the file you just left.
        if let Some(path) = self.draw_tabs(ui, summary, rect) {
            self.open_file(&path);
        }
        let body = egui::Rect::from_min_max(
            egui::pos2(rect.left(), rect.top() + TAB_H),
            egui::pos2(rect.right(), rect.bottom()),
        );
        match self.open.clone() {
            Some(file) => {
                crate::workspace::draw_editor(ui, &theme, body, &file, Some(summary));
                summary.rows.push((
                    "editor".into(),
                    format!("{} · {} lines · read-only", file.path, file.len()),
                ));
                summary.rows.push(("editor".into(), "read-only".into()));
            }
            None => {
                let painter = ui.painter();
                painter.rect_filled(body, 4.0, theme.background);
                painter.rect_stroke(
                    body,
                    4.0,
                    egui::Stroke::new(1.0, theme.border),
                    egui::StrokeKind::Inside,
                );
                // The empty state says what to do, not merely that there is nothing.
                painter.text(
                    body.center(),
                    egui::Align2::CENTER_CENTER,
                    "Pick a file on the left to read it",
                    egui::FontId::proportional(13.0),
                    theme.dim,
                );
                summary
                    .rows
                    .push(("editor".into(), "Pick a file on the left to read it".into()));
            }
        }
    }

    /// The tab strip: every open file, and which one is showing.
    ///
    /// A tab strip drawn as painted text is a picture of a tab strip. These are buttons, so
    /// they take clicks — and, like the file tree, the strip **returns** what was pressed
    /// rather than leaving the frame to go looking for it afterwards. Reading a frame's
    /// hotspots after the pane that fills them has not run yet yields an empty list every
    /// time, which is how a strip that looks clickable turns out not to be.
    ///
    /// One file means one tab and no strip: a single tab with nothing to switch to is
    /// chrome that costs a row and says nothing.
    fn draw_tabs(
        &mut self,
        ui: &mut egui::Ui,
        summary: &mut FrameSummary,
        rect: egui::Rect,
    ) -> Option<String> {
        if self.tabs.len() < 2 {
            return None;
        }
        let theme = self.theme;
        let painter = ui.painter();
        let strip = egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), TAB_H));
        painter.rect_filled(strip, 0.0, theme.panel);
        painter.line_segment(
            [strip.left_bottom(), strip.right_bottom()],
            egui::Stroke::new(1.0, theme.border),
        );
        summary
            .rows
            .push(("tabs".into(), format!("{} open", self.tabs.len())));

        // The strip shares its width between every tab rather than giving each a fixed one.
        // With a fixed width the tenth open file is drawn past the right edge, where it
        // cannot be clicked and no test sees it — an open file that cannot be reached is
        // worse than a short tab label. Sharing has no threshold at which that happens.
        let count = self.tabs.len() as f32;
        let width = ((strip.width() - 12.0) / count - 4.0).clamp(8.0, TAB_W);
        let mut pressed = None;
        for (index, path) in self.tabs.clone().into_iter().enumerate() {
            let selected = index == self.tabs.len() - 1;
            let name = path.rsplit('/').next().unwrap_or(&path).to_string();
            // The file stem, not the whole name: `layout.rs` reads as a name and
            // `layout.rs.modified.rs` does not.
            let stem = name.rsplit_once('.').map(|(s, _)| s).unwrap_or(&name);
            // Shortened, never clipped: a narrow tab says so with an ellipsis, and the full
            // path is in the editor's own header underneath either way.
            let label = fit(stem, width - 10.0, TAB_FONT);
            let response = ui.put(
                egui::Rect::from_min_size(
                    egui::pos2(
                        strip.left() + 6.0 + index as f32 * (width + 4.0),
                        strip.top() + 4.0,
                    ),
                    egui::vec2(width, TAB_H - 9.0),
                ),
                egui::Button::new(label).selected(selected),
            );
            summary
                .hotspots
                .push((crate::workspace::hotspot_id(&path), response.rect));
            summary.tabs.push((stem.to_string(), selected));
            if response.clicked() {
                pressed = Some(path);
            }
        }
        pressed
    }
    /// Open `path` in the editor, or say why it could not be opened.
    ///
    /// The failure is shown in the transcript rather than swallowed: a click that appears
    /// to do nothing is the state this repository keeps failing to rule out.
    fn open_file(&mut self, path: &str) {
        match crate::workspace::OpenFile::read(&self.opts.workspace, path) {
            Ok(file) => {
                self.open = Some(file);
                // Re-opening a file already open **moves it to the front** rather than
                // adding a second tab for it. Two tabs showing the same path is a tab
                // strip that cannot answer "which one am I editing".
                self.tabs.retain(|p| p != path);
                self.tabs.push(path.to_string());
            }
            Err(reason) => self
                .transcript
                .push_notice(&format!("cannot open {reason}")),
        }
    }

    /// What to show after the workspace name: the provider this window is calling.
    ///
    /// **Never a guess.** With no session there is nothing to name, and with one the name
    /// comes from the launch the CLI's own resolver produced — including the offline
    /// marker, which is a carried decision rather than something re-derived from the model
    /// id. A model id beside `--provider faux` must still read as offline here, exactly as
    /// it does in the terminal header.
    fn model_suffix(&self) -> String {
        let Some(session) = self.session.as_ref() else {
            return "  ·  no agent attached".into();
        };
        let launch = session.launch();
        let label = if launch.offline {
            epsilon_llm::catalog::display_label("faux-1")
        } else {
            epsilon_llm::catalog::display_label(&launch.model_id)
        };
        let offline = if launch.offline { "  ·  offline" } else { "" };
        format!("  ·  {label}{offline}")
    }
    /// The bottom bar's one line.
    ///
    /// Priority order matters more than the wording: what is *waiting on the user* outranks
    /// what the harness is doing, which outranks what the layout is doing. A status line
    /// that shows the layout tier while a permission ask sits unanswered is a status line
    /// telling the user to look at the wrong thing.
    fn status_line(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        let pending = self
            .session
            .as_ref()
            .map(|s| s.pending_approvals().len())
            .unwrap_or(0);
        if pending > 0 {
            parts.push(format!(
                "{pending} permission ask{} waiting",
                if pending == 1 { "" } else { "s" }
            ));
        }
        match self.session.as_ref() {
            None => parts.push("no agent attached".into()),
            Some(s) => {
                if s.running() {
                    parts.push("running".into());
                } else {
                    parts.push("idle".into());
                }
            }
        }
        if let Some(refusal) = &self.refusal {
            parts.push(refusal.clone());
        }
        if self.transcript.dropped() > 0 {
            parts.push(format!("{} event(s) dropped", self.transcript.dropped()));
        }
        // Only when the layout has actually given something up. `full` is a word about the
        // window's size that a reader cannot act on; `no-sidebar` explains a sidebar that
        // has just disappeared, which is the one a reader *does* need explained.
        if self.layout.tier != Tier::Full {
            parts.push(self.layout.tier.name().to_string());
        }
        parts.join("  ·  ")
    }
    /// Draw an assistant's markdown as blocks rather than as one flat line.
    ///
    /// Everything else in the transcript draws as a single coloured label; this is the
    /// one place that interprets, and it is reached only from the assistant row. The
    /// visible text of every block is pushed into the frame summary, so a test can
    /// assert that `**bold**` reached the reader as *bold* and not as asterisks.
    fn draw_markdown(
        &self,
        ui: &mut egui::Ui,
        summary: &mut FrameSummary,
        marker: &str,
        color: egui::Color32,
        text: &str,
    ) {
        let theme = self.theme;
        let blocks = crate::markup::render(text);
        // A renderer that produced nothing for a non-empty answer would leave a gap in
        // the transcript, so fall back to the raw text rather than to nothing.
        if blocks.is_empty() {
            let line = format!("{marker}{text}");
            ui.colored_label(color, line.clone());
            summary.rows.push(("agent".into(), line));
            return;
        }
        for (i, block) in blocks.iter().enumerate() {
            let prefix = if i == 0 { marker } else { "  " };
            match block {
                crate::markup::Block::Heading { runs, .. } => {
                    let job = crate::markup::job(runs, theme.title, theme.code);
                    let line = format!("{prefix}{}", plain_of(runs));
                    ui.label(egui::WidgetText::from(job));
                    summary.rows.push(("agent".into(), line));
                }
                crate::markup::Block::Paragraph(runs) => {
                    let job = crate::markup::job(runs, color, theme.code);
                    let line = format!("{prefix}{}", plain_of(runs));
                    ui.label(egui::WidgetText::from(job));
                    summary.rows.push(("agent".into(), line));
                }
                crate::markup::Block::ListItem {
                    marker: bullet,
                    runs,
                } => {
                    let job = crate::markup::job(runs, color, theme.code);
                    let line = format!("{prefix}{bullet}{}", plain_of(runs));
                    ui.label(egui::WidgetText::from(job));
                    summary.rows.push(("agent".into(), line));
                }
                crate::markup::Block::Quote(runs) => {
                    let job = crate::markup::job(runs, theme.dim, theme.code);
                    let line = format!("{prefix}❯ {}", plain_of(runs));
                    ui.label(egui::WidgetText::from(job));
                    summary.rows.push(("agent".into(), line));
                }
                crate::markup::Block::Code { language, lines } => {
                    let tag = language
                        .as_deref()
                        .map(|l| format!("[{l}]"))
                        .unwrap_or_else(|| "[code]".to_string());
                    for line in lines {
                        let shown = format!("{prefix}{tag} {line}");
                        ui.colored_label(theme.code, shown.clone());
                        summary.rows.push(("agent".into(), shown));
                    }
                }
                crate::markup::Block::Rule => {
                    ui.colored_label(theme.border, format!("{prefix}--------"));
                    summary
                        .rows
                        .push(("agent".into(), format!("{prefix}--------")));
                }
            }
        }
    }
    /// The agent panel: status, transcript, approvals, composer.
    ///
    /// Split into three regions inside one pane, and the split is decided here rather
    /// than by a nested layout because the composer must stay on screen while a long
    /// answer scrolls: a composer that scrolls away is a window that cannot answer a
    /// question it just asked.
    fn agent_panel(&mut self, ui: &mut egui::Ui, summary: &mut FrameSummary, rect: egui::Rect) {
        const COMPOSER_H: f32 = 92.0;
        const HEADER_H: f32 = 34.0;
        let theme = self.theme;
        // Scoped: `Ui::painter()` borrows the `Ui`, and the scroll area below needs it
        // mutably. Holding the painter across that is the classic egui borrow conflict.
        {
            let painter = ui.painter();
            painter.rect_filled(rect, 4.0, theme.background);
            painter.rect_stroke(
                rect,
                4.0,
                egui::Stroke::new(1.0, theme.border),
                egui::StrokeKind::Inside,
            );
            painter.text(
                rect.left_top() + egui::vec2(12.0, 16.0),
                egui::Align2::LEFT_CENTER,
                "Agent",
                egui::FontId::proportional(12.0),
                theme.dim,
            );
        }
        summary.rows.push(("agent".into(), "Agent".into()));
        // ---- header: what the launch said ------------------------------------
        let mut y = rect.top() + HEADER_H;
        {
            let painter = ui.painter();
            for notice in &self.notices {
                if y > rect.bottom() - COMPOSER_H {
                    break;
                }
                painter.text(
                    egui::pos2(rect.left() + 12.0, y),
                    egui::Align2::LEFT_TOP,
                    notice,
                    egui::FontId::proportional(12.0),
                    theme.dim,
                );
                summary.rows.push(("agent".into(), notice.clone()));
                y += 17.0;
            }
        }
        // ---- transcript -----------------------------------------------------
        let composer_top = rect.bottom() - COMPOSER_H;
        let body = egui::Rect::from_min_max(
            egui::pos2(rect.left() + 8.0, y + 4.0),
            egui::pos2(rect.right() - 8.0, composer_top - 6.0),
        );
        if body.height() > 0.0 {
            // max_height rather than a hard rect: the scroll area is handed the
            // Ui and lays out inside the space the composer left, which is the only way
            // three regions share one pane without one of them clipping the others.
            let decided = egui::ScrollArea::vertical()
                .id_salt("epsilon-agent-transcript")
                .stick_to_bottom(true)
                .max_height(body.height())
                .show(ui, |ui| {
                    ui.set_width(body.width());
                    self.draw_rows(ui, summary)
                });
            // Applied here rather than inside the draw, so the click is resolved once,
            // in one place, and the draw path stays a draw path.
            if let (Some((id, decision)), Some(s)) = (decided.inner, self.session.as_ref()) {
                s.resolve(&id, decision);
            }
        }
        // ---- composer -------------------------------------------------------
        let composer = egui::Rect::from_min_size(
            egui::pos2(rect.left() + 8.0, composer_top),
            egui::vec2((rect.width() - 16.0).max(0.0), COMPOSER_H - 6.0),
        );
        if composer.width() > 40.0 {
            self.draw_composer(ui, summary, composer);
        } else {
            // Too narrow for a composer worth drawing; say so rather than drawing a
            // one-column text box that cannot be read or typed into.
            let line = "widen the window to type";
            let painter = ui.painter();
            painter.text(
                composer.left_top() + egui::vec2(4.0, 12.0),
                egui::Align2::LEFT_TOP,
                line,
                egui::FontId::proportional(12.0),
                theme.dim,
            );
            summary.rows.push(("agent".into(), line.into()));
        }
    }
    /// Draw the transcript rows.
    ///
    /// Returns the permission decision a reader made with the mouse, which the caller
    /// applies. Buttons as well as keys because a gate that stops the whole run cannot be
    /// reachable only from the keyboard: the ask is drawn where a reader is already
    /// looking, so the two answers belong beside it.
    fn draw_rows(
        &self,
        ui: &mut egui::Ui,
        summary: &mut FrameSummary,
    ) -> Option<(String, PermissionDecision)> {
        let theme = self.theme;
        let pending: Vec<_> = self
            .session
            .as_ref()
            .map(|s| s.pending_approvals())
            .unwrap_or_default();
        let rows = self.transcript.rows();
        if rows.is_empty() && pending.is_empty() {
            let line = match self.session {
                None => "no agent attached",
                Some(_) => "nothing yet — type below to start",
            };
            ui.colored_label(theme.dim, line);
            summary.rows.push(("agent".into(), line.to_string()));
            return None;
        }
        for row in rows {
            let (marker, color) = match &row.kind {
                RowKind::User => ("› ", theme.user),
                RowKind::Assistant => ("  ", theme.text),
                RowKind::Thinking => ("~ ", theme.thinking),
                RowKind::Tool { failed, .. } => match failed {
                    None => ("▸ ", theme.accent),
                    Some(false) => ("✓ ", theme.success),
                    Some(true) => ("✗ ", theme.danger),
                },
                RowKind::Approval { .. } => ("! ", theme.warn),
                RowKind::Changed => ("± ", theme.accent),
                RowKind::Diff => ("  ", theme.dim),
                RowKind::Notice => ("  ", theme.dim),
            };
            // A tool row with no arguments yet still has to say which tool it is: an
            // empty row with a marker is a mark the reader has to decode.
            let body = if row.text.is_empty() {
                match &row.kind {
                    RowKind::Tool { name, .. } => format!("{name}…"),
                    _ => String::new(),
                }
            } else {
                row.text.clone()
            };
            // **Only the assistant's text is interpreted.** A user who types `**hi**`
            // must see `**hi**`; a diff line is a diff line. Rendering is an
            // interpretation, and applying it to text that is not prose makes the
            // window lie about what was said.
            if matches!(row.kind, RowKind::Assistant) {
                self.draw_markdown(ui, summary, marker, color, &body);
                continue;
            }
            let text = format!("{marker}{body}");
            let painted = ui.colored_label(color, text.clone());
            summary.rows.push(("agent".into(), text));
            summary.row_rects.push(("agent".into(), painted.rect));
        }
        // The pending approvals are drawn after the rows, where they cannot scroll out of
        // reach behind a long answer — the composer-adjacent placement opencode and DSH
        // both converged on, and the reason is not aesthetic.
        //
        // Drawn from the **approvals channel**, not from the transcript: the channel is
        // the authority on what is waiting, and a run that parks on a gate before its first
        // message has produced no rows at all. A reader in that state must still be told
        // the run is blocked on *them*.
        let mut decided = None;
        for ask in pending {
            let text = format!(
                "allow {} {} ?   [enter] allow   [esc] refuse",
                ask.capability, ask.pattern
            );
            ui.colored_label(theme.warn, text.clone());
            summary.rows.push(("agent".into(), text));
            summary.row_rects.push(("agent".into(), ui.min_rect()));
            // The two answers, drawn where the ask is drawn. `horizontal` keeps them on
            // one line with the ask; a run that is blocked on this row and has no way to
            // unblock it is the window's worst state, so the buttons are not a
            // convenience.
            ui.horizontal(|ui| {
                if ui
                    .button("Allow")
                    .on_hover_text("Run this now. Enter does the same.")
                    .clicked()
                {
                    decided = Some((ask.request_id.clone(), PermissionDecision::AllowOnce));
                }
                if ui
                    .button("Refuse")
                    .on_hover_text("Decline it. Escape does the same.")
                    .clicked()
                {
                    decided = Some((ask.request_id.clone(), PermissionDecision::Deny));
                }
            });
            summary
                .hotspots
                .push((format!("approval:allow:{}", ask.request_id), ui.min_rect()));
        }
        decided
    }
    /// Send whatever the composer holds.
    ///
    /// Extracted from the draw path because **the interesting part is not the drawing.**
    /// The key handling that decides *whether* to send is two `consume_key` calls; what
    /// happens next — refusing a second turn, clearing the draft, putting the reader's own
    /// words on screen — is a rule, and a rule is worth a test. The composer calls this;
    /// so does [`tests::sending_the_draft_submits_it_and_shows_the_prompt`].
    ///
    /// Returns what was sent, or `None` when there was nothing to send or a turn is
    /// already in flight.
    pub fn send_draft(&mut self) -> Option<String> {
        if self.session.as_ref().map(|s| s.running()).unwrap_or(false) {
            return None;
        }
        let text = self.draft.trim().to_string();
        if text.is_empty() {
            return None;
        }
        self.refusal = self.session.as_ref().and_then(|s| s.submit(&text).err());
        if self.refusal.is_some() {
            return None;
        }
        self.draft.clear();
        // The user's own words, on screen at the moment they are sent.
        //
        // `AgentEvent` has no "you said this" — the prompt is an input, not an output, so
        // no event carries it — and a frontend that waits for one shows an empty transcript
        // until the model answers. The reader pressed Enter and nothing appeared, which is
        // the state this whole repository keeps trying to rule out.
        self.transcript.push_user(&text);
        Some(text)
    }
    /// The composer's text box, and the two ways to send.
    ///
    /// `Enter` sends and `Shift+Enter` inserts a newline, which is what every editor a
    /// developer already has in their hands does.
    fn draw_composer(&mut self, ui: &mut egui::Ui, summary: &mut FrameSummary, rect: egui::Rect) {
        let running = self.session.as_ref().map(|s| s.running()).unwrap_or(false);
        let hint = if running {
            "running — esc stops"
        } else {
            self.placeholder
        };
        // **Consume the keys first, act on them last.** The order is the whole trick and
        // getting it wrong is invisible until someone types and presses Enter:
        //
        // * Consuming `Enter` *before* the text edit is drawn is what stops Enter becoming
        //   a newline. Doing it after would put a blank line in the draft every time.
        // * Acting on it *after* the text edit is drawn is what makes the draft contain
        //   the words typed on this very frame. Doing it before submits an empty prompt,
        //   which is what the first version did.
        let sent = ui
            .ctx()
            .input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
        let escaped = ui
            .ctx()
            .input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        let editor =
            egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), rect.height() - 28.0));
        let response = ui.put(
            editor,
            egui::TextEdit::multiline(&mut self.draft)
                .id(composer_id())
                .hint_text(hint)
                .desired_width(editor.width() - 8.0)
                .font(egui::TextStyle::Monospace),
        );
        let send_clicked = ui
            .put(
                egui::Rect::from_min_size(
                    rect.right_bottom() + egui::vec2(-64.0, -24.0),
                    egui::vec2(64.0, 22.0),
                ),
                egui::Button::new("Send"),
            )
            .clicked();
        if sent || send_clicked {
            self.send_draft();
        }
        if escaped {
            if running {
                if let Some(s) = self.session.as_ref() {
                    s.abort();
                }
            } else {
                self.draft.clear();
            }
        }
        // **Take the keyboard.** Without this the window opens with nothing focused, every
        // keystroke goes nowhere, and it looks exactly like a frozen application — which is
        // the failure this repository has chased across four audits in the terminal
        // frontend, and the end-to-end window test reproduced it here on its first run by
        // synthesizing nine characters and watching none of them arrive.
        //
        // Unconditional, because there is exactly one text input in this window and a
        // composer that cannot be typed into is not a composer. When a second input
        // arrives this becomes "unless something else has claimed focus", which is a
        // one-line change and a comment here rather than a rewrite.
        if !response.has_focus() {
            response.request_focus();
        }
        summary.rows.push(("composer".into(), hint.to_string()));
    }
}
/// The composer's widget id.
///
/// Named rather than derived from a position, because the composer is the one widget whose
/// identity has to survive the window being resized — a per-position id would drop the
/// caret and the draft's scroll state on every resize.
fn composer_id() -> egui::Id {
    egui::Id::new("epsilon-composer")
}
/// The visible text of a block's runs, for the frame summary.
///
/// The summary is what tests assert on, so it must carry **what the reader sees** — not
/// the markup, and not a reconstruction that could disagree with the drawing.
fn plain_of(runs: &[crate::markup::Run]) -> String {
    runs.iter().map(|r| r.text.as_str()).collect()
}
/// What the sidebar shows, chosen from the activity bar.
///
/// Two, not three: **files** and **tasks** are what this window can act on and show. A
/// "sessions" view needs the ability to *switch* the session — which is a lease handover,
/// not a list — so it will be added when it can do the thing a reader expects rather than
/// only look like it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidebarView {
    /// The workspace's files.
    Files,
    /// The subagents this session has dispatched.
    Tasks,
    /// Conversations stored for this workspace. Switching is a real handover.
    Sessions,
}
impl SidebarView {
    /// Every view, in bar order.
    pub const ALL: [SidebarView; 3] = [
        SidebarView::Files,
        SidebarView::Tasks,
        SidebarView::Sessions,
    ];
    /// A stable key, for the hotspot id a test presses.
    pub fn key(self) -> &'static str {
        match self {
            Self::Files => "files",
            Self::Tasks => "tasks",
            Self::Sessions => "sessions",
        }
    }
    /// What the button says.
    pub fn label(self) -> &'static str {
        match self {
            Self::Files => "files",
            Self::Tasks => "subagents",
            Self::Sessions => "sessions",
        }
    }
    /// The glyph on the button.
    pub fn glyph(self) -> &'static str {
        match self {
            Self::Files => "◧",
            Self::Tasks => "⑂",
            Self::Sessions => "☰",
        }
    }
}
/// A path, shortened for a narrow pane: the last two components, with the root elided.
fn display_path(path: &std::path::Path) -> String {
    let text = path.display().to_string();
    let parts: Vec<&str> = text.split(['/', '\\']).filter(|s| !s.is_empty()).collect();
    if parts.len() <= 2 {
        return text;
    }
    format!("…/{}", parts[parts.len() - 2..].join("/"))
}
impl eframe::App for IdeApp {
    /// eframe 0.36 splits the old `update` in two, and **the split is not a detail**:
    /// `ui` does not run at all while the window is hidden.
    ///
    /// A turn that finishes in the background therefore produces no `AgentEnd` on screen
    /// — no status change, no `idle` — until the window is shown again, which reads as
    /// "still working" for as long as the window is in the background. That is the shape
    /// of the freeze this repository has chased across four audits, so the drain has to
    /// happen here too. It takes `&mut self` and must not paint, which is exactly what
    /// draining is.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.pump();
        let _ = ctx;
    }
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        ui.ctx().set_visuals(self.theme.visuals());
        self.paint(ui);
    }
}
/// Answer a pending approval from the keyboard, if one is waiting.
///
/// Split out from [`IdeApp::paint`] because it is the part with a policy — Enter allows,
/// Escape refuses — and a policy is worth one test that names it rather than one that
/// pokes at pixels.
pub fn approve_pending(session: &IdeSession, ctx: &egui::Context) -> Option<(String, bool)> {
    let ask = session.pending_approvals().into_iter().next()?;
    if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)) {
        let decision = PermissionDecision::AllowOnce;
        let resolved = session.resolve(&ask.request_id, decision);
        return Some((ask.request_id, resolved));
    }
    if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
        let decision = PermissionDecision::Deny;
        let resolved = session.resolve(&ask.request_id, decision);
        return Some((ask.request_id, resolved));
    }
    None
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{MEDIUM, NARROW};
    /// Paint a frame at `w` x `h` with no window, and describe it.
    ///
    /// egui renders into an off-screen context, so this is a measurement of the real
    /// draw path rather than of a stand-in for it — the same "render a frame and look at
    /// it" discipline the terminal frontend uses, without a terminal.
    ///
    /// The `textures_delta.clear()` calls are not decoration. epaint panics in `Drop`
    /// when a `TexturesDelta` goes away with unapplied deltas, and the first run of this
    /// helper produced six failures whose message was about texture deltas and whose real
    /// content was "the test harness throws away what the frame produced". A harness that
    /// drops the frame is not measuring the frame.
    fn paint_at(w: f32, h: f32) -> FrameSummary {
        let ctx = egui::Context::default();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(w, h),
            )),
            ..Default::default()
        };
        // Two passes: the first teaches egui the input shape, the second paints
        // against a settled layout. One pass is enough in practice, and two makes
        // the result independent of that being true.
        //
        // `run_ui` hands the callback a root `Ui` covering the whole viewport, which
        // is exactly what `IdeApp::ui` receives from eframe -- so the test drives the
        // same entry point the shipped window does, not a stand-in for it.
        let mut warm = ctx.run_ui(input.clone(), |_| {});
        warm.textures_delta.clear();
        let mut app = IdeApp::without_agent(Options::for_workspace(std::path::PathBuf::from(
            "/tmp/example-workspace",
        )));
        let mut out = FrameSummary::default();
        let mut full = ctx.run_ui(input, |ui| out = app.paint(ui));
        full.textures_delta.clear();
        out
    }
    #[test]
    fn a_comfortable_window_draws_every_pane_with_a_title_and_a_body() {
        let f = paint_at(1440.0, 900.0);
        assert_eq!(f.tier, Tier::Full);
        for region in [
            "title",
            "status",
            "activity",
            "workspace",
            "editor",
            "agent",
        ] {
            assert!(f.drew(region), "no {region} drawn.\n{}", f.describe());
        }
        // The pane title is drawn *in* the pane, so it is part of what the region
        // contains -- the assertion is on the body rows, not on an exact list.
        let workspace = f.region("workspace");
        assert!(workspace.contains(&"Workspace"), "{workspace:?}");
        assert!(
            workspace.iter().any(|t| t.starts_with("workspace  ")),
            "the pane names the workspace it is bound to: {workspace:?}"
        );
        // The file count, and the word that says the list is only what the walk found.
        // A tree whose footer said nothing would let a truncated walk look like a
        // repository with no more files in it.
        assert!(
            workspace.iter().any(|t| t.contains("files")),
            "the pane says how much it found: {workspace:?}"
        );
        // The editor's empty state says what to do, not merely that there is nothing.
        assert!(
            f.region("editor").iter().any(|t| t.contains("Pick a file")),
            "{:?}",
            f.region("editor")
        );
        assert!(
            f.drew("composer"),
            "the agent panel has no composer.\n{}",
            f.describe()
        );
        // With no agent attached, the composer's placeholder says exactly that. It does
        // not advertise a key binding that would do nothing, and it does not show the
        // wired app's hint -- the two states are different and the difference is on screen.
        assert!(
            f.region("composer").iter().any(|t| t.contains("no agent")),
            "an agent-less composer must say so.\n{}",
            f.describe()
        );
    }
    #[test]
    fn narrowing_the_window_folds_the_sidebar_and_then_the_editor() {
        let medium = paint_at(MEDIUM - 1.0, 900.0);
        assert_eq!(medium.tier, Tier::NoSidebar);
        assert!(
            !medium.drew("workspace"),
            "the sidebar must be gone.\n{}",
            medium.describe()
        );
        assert!(
            medium.drew("editor"),
            "the editor must survive first.\n{}",
            medium.describe()
        );
        let narrow = paint_at(NARROW - 1.0, 900.0);
        assert_eq!(narrow.tier, Tier::AgentOnly);
        assert!(!narrow.drew("editor"));
        assert!(narrow.drew("agent"));
    }
    #[test]
    fn a_short_window_folds_the_status_bar_then_the_title_bar() {
        let short = paint_at(1440.0, crate::layout::SHORT - 1.0);
        assert_eq!(short.tier, Tier::CompactHeight);
        assert!(!short.drew("status"));
        assert!(short.drew("title"));
        let tiny = paint_at(1440.0, crate::layout::COMPACT - 1.0);
        assert_eq!(tiny.tier, Tier::ChromeOnly);
        assert!(!tiny.drew("title"));
        assert!(
            tiny.drew("agent"),
            "the body still draws.\n{}",
            tiny.describe()
        );
    }
    #[test]
    fn no_frame_at_any_size_is_empty() {
        // The terminal frontend's standing invariant: "nothing happened" must never be a
        // state the UI can be in. Here it means: every drawable size puts *something* on
        // the screen.
        for w in [
            1600.0, 1440.0, 1200.0, 1000.0, 900.0, 860.0, 700.0, 680.0, 640.0,
        ] {
            for h in [1000.0, 800.0, 600.0, 520.0, 400.0, 360.0] {
                let f = paint_at(w, h);
                assert!(
                    !f.rows.is_empty(),
                    "an empty frame at {w}x{h} is the frozen-window state"
                );
                assert!(
                    f.drew("agent"),
                    "the agent panel is the product; it is missing at {w}x{h}\n{}",
                    f.describe()
                );
            }
        }
    }
    #[test]
    fn the_workspace_name_is_on_screen() {
        let f = paint_at(1440.0, 900.0);
        assert!(
            f.region("title")
                .iter()
                .any(|t| t.contains("example-workspace")),
            "the title bar must name the workspace.\n{}",
            f.describe()
        );
    }
    #[test]
    fn the_status_line_does_not_invent_a_provider_or_a_cost() {
        // M233 in the terminal: the header must name what the process is actually
        // calling, and say nothing rather than guess. The same rule applies to every
        // number this crate will eventually draw.
        let f = paint_at(1440.0, 900.0);
        let status = f.region("status").join(" ");
        assert!(status.contains("no agent attached"), "{status}");
        for banned in ["$0", "GPT", "Claude", "DeepSeek", "cache"] {
            assert!(
                !status.contains(banned),
                "the status line claims {banned:?} before an agent exists.\n{}",
                f.describe()
            );
        }
        assert!(
            f.region("title")
                .iter()
                .any(|t| t.contains("no agent attached")),
            "and the title says so too, rather than naming a model.\n{}",
            f.describe()
        );
    }
    #[test]
    fn the_status_line_does_not_lead_with_window_geometry() {
        // `full` is a word about the window's size that a reader cannot act on, and it used
        // to be on every status line in the product. What a reader must do comes first;
        // geometry comes last, and only when geometry has actually taken something away.
        let app = IdeApp::without_agent(Options::for_workspace("/tmp/ws"));
        let line = app.status_line();
        assert!(line.starts_with("no agent attached"), "{line}");
        assert!(
            !line.contains("full"),
            "an undegraded layout has nothing to explain, so it says nothing: {line}"
        );
    }

    /// A layout that **has** given something up says so, because that is the one case
    /// where the reader needs the reason a pane is missing.
    #[test]
    fn a_degraded_layout_says_which_pane_it_dropped() {
        let (dir, _keep) = scratch_workspace("tier");
        let mut h = Harness::sized(dir, 600.0, 700.0);
        let line = {
            let _ = h.paint();
            h.app.status_line()
        };
        assert!(
            !line.contains("full"),
            "a 600-pixel window is not a full layout: {line}"
        );
        assert!(
            line.contains("chrome-only")
                || line.contains("compact-height")
                || line.contains("no-sidebar")
                || line.contains("agent-only"),
            "a window too small for the full layout must say which tier it fell to: {line}"
        );
    }
    /// The task pane renders three states, and that is the part that was written and
    /// never checked.
    ///
    /// A live delegation is a race -- it can finish before the first frame -- so the state
    /// machine is tested here and the *acquisition* stays where `epsilon-core`'s own
    /// `task_scenarios.rs` already covers it. What this pins is the thing the pane is for:
    /// a reader must be able to tell a running child from a failed one at a glance.
    #[test]
    fn each_subagent_state_reads_differently_and_names_its_model() {
        use epsilon_contracts::tool::{TaskSnapshot, TaskState};
        let theme = IdeTheme::from(epsilon_tui::ThemeMode::Dark);
        let base = TaskSnapshot {
            id: "task-1".into(),
            description: "audit the auth tests".into(),
            subagent_type: "reviewer".into(),
            model: Some("fast-model".into()),
            state: TaskState::Running,
            output: String::new(),
            turns: 3,
            usage: None,
        };
        let mut rendered: Vec<Vec<String>> = Vec::new();
        for state in [TaskState::Running, TaskState::Completed, TaskState::Failed] {
            let mut snapshot = base.clone();
            snapshot.state = state;
            rendered.push(
                IdeApp::task_lines(&snapshot, &theme)
                    .into_iter()
                    .map(|(_, text)| text)
                    .collect(),
            );
        }
        let markers: Vec<String> = rendered
            .iter()
            .map(|lines| lines[0].chars().take(2).collect())
            .collect();
        assert_eq!(
            markers,
            vec!["▸ ".to_string(), "✓ ".to_string(), "✗ ".to_string()],
            "a running child and a failed one must not read the same: {markers:?}"
        );
        // Every state names the model it ran on. A delegation that silently ran on a
        // different model from the one asked for is invisible unless the model is on the row.
        for lines in &rendered {
            assert!(
                lines[1].contains("fast-model") && lines[1].contains("3 turn(s)"),
                "every row carries its model and turn count: {:?}",
                lines[1]
            );
        }
    }
    /// A model the snapshot does not know is omitted rather than printed as an empty gap.
    #[test]
    fn a_subagent_with_no_recorded_model_shows_no_model_rather_than_a_blank() {
        use epsilon_contracts::tool::{TaskSnapshot, TaskState};
        let theme = IdeTheme::from(epsilon_tui::ThemeMode::Dark);
        let snapshot = TaskSnapshot {
            id: "task-2".into(),
            description: "no model recorded".into(),
            subagent_type: "default".into(),
            model: None,
            state: TaskState::Completed,
            output: String::new(),
            turns: 0,
            usage: None,
        };
        let detail = IdeApp::task_lines(&snapshot, &theme)[1].1.clone();
        assert!(
            !detail.ends_with(" · "),
            "a trailing separator is a value that is missing, printed as if it were empty: {detail}"
        );
    }
    #[test]
    fn one_task_is_not_one_tasks() {
        use epsilon_contracts::tool::TaskSnapshot;
        let header = IdeApp::task_header(&[TaskSnapshot {
            id: "t".into(),
            description: "d".into(),
            subagent_type: "s".into(),
            model: None,
            state: epsilon_contracts::tool::TaskState::Running,
            output: String::new(),
            turns: 0,
            usage: None,
        }]);
        assert!(header.starts_with("1 task,"), "{header}");
        assert!(!header.contains("(s)"), "{header}");
    }

    /// A row that does not fit is **marked as cut**, never silently clipped.
    ///
    /// egui clips an overflowing label without a word. A truncated model name and the whole
    /// model name look identical on screen, and the two failures are opposites: one misleads
    /// a reader about what ran, the other about what the agent is doing.
    #[test]
    fn a_row_that_does_not_fit_is_marked_rather_than_clipped() {
        assert_eq!(fit("short", 200.0, ROW_FONT), "short");

        let long = "a".repeat(200);
        let cut = fit(&long, 200.0, ROW_FONT);
        assert!(cut.ends_with('…'), "a cut row must say so: {cut:?}");
        assert!(
            cut.chars().count() < long.chars().count(),
            "and must actually be shorter"
        );
        // Never empty: a budget of zero is a value that vanished rather than one that was
        // shortened, which is the failure this is here to prevent.
        assert!(fit(&"x".repeat(50), 1.0, ROW_FONT).chars().count() >= 4);
    }

    /// The budget only ever **shortens**. A number that could drop a readable character
    /// would be worse than no elision at all.
    #[test]
    fn the_budget_never_returns_more_than_was_put_in() {
        for width in [1.0f32, 12.0, 47.0, 120.0, 264.0, 900.0] {
            for n in [0usize, 1, 5, 39, 40, 41, 200] {
                let text = "m".repeat(n);
                let out = fit(&text, width, ROW_FONT);
                assert!(
                    out.chars().count() <= text.chars().count().max(4),
                    "width {width}, {n} chars: {:?} is longer than what went in",
                    out
                );
            }
        }
    }

    /// Every painted transcript row lands **inside the agent pane**.
    ///
    /// This is the test the shared-`Ui` defect needed and did not have. The panes drew
    /// correct text and a suite of 103 tests passed, because `rows` records what was drawn
    /// and nothing recorded where: the transcript was laid out at `[0,142]` while the
    /// agent pane was at `[920,42]`, so the conversation was painted on top of the file
    /// tree and the right-hand pane stood empty. The panes themselves never overlapped —
    /// `layout.rs` proves that — which is exactly why the gap survived a passing sweep.
    #[test]
    fn the_transcript_is_drawn_inside_the_agent_pane_and_nowhere_else() {
        let (dir, _keep) = scratch_workspace("inside");
        let mut h = Harness::new(dir.clone());
        let mut t = Transcript::new(dir.clone());
        for i in 0..8 {
            t.push_user(&format!("question {i}"));
        }
        h.app.seed_transcript(t);
        let f = h.paint();

        assert!(
            f.row_rects.iter().any(|(r, _)| r == "agent"),
            "the probe needs painted agent rows to measure.\n{}",
            f.describe()
        );
        let pane = h.app.layout.agent;
        for (region, rect) in &f.row_rects {
            assert!(
                rect.left() >= pane.left() - 0.5
                    && rect.right() <= pane.right() + 0.5
                    && rect.top() >= pane.top() - 0.5
                    && rect.bottom() <= pane.bottom() + 0.5,
                "a {region} row was painted at {rect:?}, outside the agent pane {pane:?}."
            );
        }
    }

    /// An editor shows the file **from the beginning**, because that is what opening a file
    /// means.
    ///
    /// It used to draw `lines[len - visible..]` — the tail, with no scrollbar — so opening a
    /// 900-line source file showed its last forty lines and offered no way to reach line 1.
    /// The assertion is on the *first drawn line number*, not on "a line is drawn": a test that
    /// only checks the pane is non-empty passed against the broken version for exactly as long
    /// as the broken version existed.
    #[test]
    fn the_editor_starts_at_the_first_line_of_the_file() {
        let (dir, _keep) = scratch_workspace("editor");
        let long: String = (1..=900).map(|i| format!("line {i}\n")).collect();
        std::fs::write(dir.join("long.rs"), long).expect("a long file");
        let mut h = Harness::new(dir);
        h.app.open_file("long.rs");
        let f = h.paint();

        let first = f
            .region("editor")
            .iter()
            .find(|r| r.contains("line "))
            .copied()
            .unwrap_or_else(|| {
                panic!("the editor drew no file content.\n{}", f.describe());
            });
        assert!(
            first.contains("    1 line 1"),
            "the editor must open at line 1, drew {first:?}.\n{}",
            f.describe()
        );
        assert!(
            !f.region("editor").iter().any(|r| r.contains("line 900")),
            "the whole file fits in a 900-line window, which it does not; the pane is \
             showing a tail again."
        );
    }

    /// Every line the editor draws lands inside the editor pane.
    ///
    /// `OpenFile` keeps whole lines on purpose, so a 400-character line is 400 characters
    /// wide. With no clip it painted straight across the transcript beside it. The clip is
    /// the fix; this is what would notice it going away.
    #[test]
    fn editor_lines_are_drawn_inside_the_editor_pane() {
        let (dir, _keep) = scratch_workspace("clip");
        let wide: String = (1..=200)
            .map(|i| format!("let x{i} = \"{}\";\n", "z".repeat(300)))
            .collect();
        std::fs::write(dir.join("wide.rs"), wide).expect("a wide file");
        let mut h = Harness::new(dir);
        h.app.open_file("wide.rs");
        let f = h.paint();

        let pane = h.app.layout.center;
        let lines: Vec<_> = f.row_rects.iter().filter(|(r, _)| r == "editor").collect();
        assert!(
            !lines.is_empty(),
            "no editor lines were drawn.\n{}",
            f.describe()
        );
        for (_, rect) in lines {
            assert!(
                rect.left() >= pane.left() - 0.5 && rect.right() <= pane.right() + 0.5,
                "an editor line was drawn at {rect:?}, outside the editor pane {pane:?}"
            );
        }
    }

    /// Non-ASCII text is cut on a character boundary, so the row never ends in the
    /// replacement character that a byte slice would leave behind.
    #[test]
    fn a_cut_row_never_ends_in_half_a_character() {
        let text = "日本語のテキスト".repeat(20);
        for width in [20.0f32, 60.0, 130.0, 260.0] {
            let cut = fit(&text, width, ROW_FONT);
            assert!(!cut.contains('\u{FFFD}'), "width {width}: {cut:?}");
        }
    }

    #[test]
    fn a_long_path_is_shortened_rather_than_overflowing_the_pane() {
        let short = display_path(std::path::Path::new("/a/b"));
        assert_eq!(short, "/a/b");
        let long = display_path(std::path::Path::new("/home/user/code/project"));
        assert_eq!(long, "…/code/project");
        assert!(
            long.chars().count() < 24,
            "a shortened path is still too long: {long}"
        );
    }

    /// An app that outlives one frame, so a test can click something and then look at
    /// what the **next** frame drew.
    ///
    /// Every other helper here paints one frame and throws the app away. That is fine for
    /// "what does a frame contain" and useless for "what does a click do", because a click
    /// needs a pointer that moves, then a button that goes down and up, and then a frame
    /// that acts on it. This is why the tab strip shipped with hotspots recorded for a test
    /// that did not exist: nothing here could produce a click at all.
    struct Harness {
        ctx: egui::Context,
        app: IdeApp,
        input: egui::RawInput,
    }

    impl Harness {
        fn new(workspace: std::path::PathBuf) -> Self {
            Self::sized(workspace, 1440.0, 900.0)
        }
        fn sized(workspace: std::path::PathBuf, w: f32, h: f32) -> Self {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(w, h),
                )),
                ..Default::default()
            };
            Self {
                ctx: egui::Context::default(),
                app: IdeApp::without_agent(Options::for_workspace(workspace)),
                input,
            }
        }
        /// One frame, with no pointer.
        fn paint(&mut self) -> FrameSummary {
            let mut out = FrameSummary::default();
            let mut full = self
                .ctx
                .run_ui(self.input.clone(), |ui| out = self.app.paint(ui));
            full.textures_delta.clear();
            out
        }
        /// Put the pointer on `p`, press, then release on a later frame — and return the
        /// frame the release produced.
        ///
        /// Three frames, not one, and each one paints. A press and a release delivered in
        /// the same frame arrive at a widget that egui has not yet registered the pointer
        /// against, so nothing registers a click; the version this replaced looked like a
        /// working harness and silently clicked nothing. A real pointer press is also held
        /// across frames, so this is the shape a person produces anyway.
        fn click(&mut self, p: egui::Pos2) -> FrameSummary {
            let button = |pressed: bool| egui::Event::PointerButton {
                pos: p,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            // Every frame paints, including the ones whose output is thrown away. egui
            // resolves a click against widgets that existed when the press landed, and a
            // frame that builds no widgets registers nothing for the press to land on —
            // which is why the first version of this helper looked like a working harness
            // and clicked nothing.
            let feed = |h: &mut Harness, events: Vec<egui::Event>| {
                let mut input = h.input.clone();
                input.events = events;
                let mut full = h.ctx.run_ui(input, |ui| {
                    h.app.paint(ui);
                });
                full.textures_delta.clear();
            };
            feed(self, vec![egui::Event::PointerMoved(p)]);
            feed(self, vec![egui::Event::PointerMoved(p), button(true)]);
            let mut out = FrameSummary::default();
            let mut input = self.input.clone();
            input.events = vec![egui::Event::PointerMoved(p), button(false)];
            let mut full = self.ctx.run_ui(input, |ui| out = self.app.paint(ui));
            full.textures_delta.clear();
            out
        }
    }

    /// A workspace on disk with two readable files, removed when the test ends.
    fn scratch_workspace(name: &str) -> (std::path::PathBuf, TempWorkspace) {
        let dir = std::env::temp_dir().join(format!("epsilon-tabs-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch workspace");
        std::fs::write(dir.join("alpha.rs"), "fn alpha() {}\n").expect("alpha");
        std::fs::write(dir.join("beta.rs"), "fn beta() {}\n").expect("beta");
        (dir.clone(), TempWorkspace(dir))
    }

    /// Deletes its directory on drop, so a failing assertion cannot leave a workspace
    /// behind for the next run to trip over.
    struct TempWorkspace(std::path::PathBuf);

    impl Drop for TempWorkspace {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Two open files draw a strip, and the one the editor is showing is the marked one.
    ///
    /// This is the frame-level assertion the strip needs. Asserting on `app.tabs` alone
    /// would pass with a strip that draws nothing, which is exactly what the first version
    /// of this feature did: the field existed, the tabs were correct, and nothing had ever
    /// been on screen.
    #[test]
    fn two_open_files_draw_a_tab_strip_and_mark_the_one_being_shown() {
        let (dir, _keep) = scratch_workspace("strip");
        let mut h = Harness::new(dir);
        h.app.open_file("alpha.rs");
        h.app.open_file("beta.rs");
        let f = h.paint();

        assert_eq!(
            f.tabs,
            vec![("alpha".to_string(), false), ("beta".to_string(), true)],
            "the strip shows every open file and marks the current one.\n{}",
            f.describe()
        );
        assert!(
            f.drew("tabs"),
            "the strip itself must be a region, not just a field.\n{}",
            f.describe()
        );
    }

    /// One open file is one tab with nothing to switch to, so no strip is drawn.
    #[test]
    fn a_single_open_file_draws_no_strip() {
        let (dir, _keep) = scratch_workspace("one");
        let mut h = Harness::new(dir);
        h.app.open_file("alpha.rs");
        let f = h.paint();

        assert!(f.tabs.is_empty(), "{:?}", f.tabs);
        assert!(!f.drew("tabs"), "{}\n", f.describe());
    }

    /// Clicking a tab switches the editor to that file.
    ///
    /// Driven by coordinates taken from the frame's own hotspots, so the test presses
    /// where the tab was actually drawn rather than where the arithmetic says it should
    /// be. This is the test the feature did not have: `draw_tabs` used to record hotspots
    /// and then be ignored, so the strip was drawn, was clickable-looking, and did nothing.
    #[test]
    fn clicking_a_tab_switches_the_editor_to_that_file() {
        let (dir, _keep) = scratch_workspace("switch");
        let mut h = Harness::new(dir);
        h.app.open_file("alpha.rs");
        h.app.open_file("beta.rs");

        let before = h.paint();
        assert!(
            before
                .region("editor")
                .iter()
                .any(|t| t.contains("beta.rs")),
            "beta.rs should be showing first: {:?}",
            before.region("editor")
        );
        let tab = before
            .hotspot(&crate::workspace::hotspot_id("alpha.rs"))
            .expect("the strip records where each tab was drawn");

        let after = h.click(tab.center());
        assert!(
            after
                .region("editor")
                .iter()
                .any(|t| t.contains("alpha.rs")),
            "clicking the alpha tab must switch the editor to it, got {:?}.\n{}",
            after.region("editor"),
            after.describe()
        );
        // The strip is still there afterwards, still with two tabs, still marking the new
        // current file — a switch that empties the strip has switched the whole pane.
        assert_eq!(
            after.tabs,
            vec![("beta".to_string(), false), ("alpha".to_string(), true)],
            "the strip survives the switch and re-marks the current file.\n{}",
            after.describe()
        );
    }

    /// Every open tab stays inside the strip, however many files are open.
    ///
    /// A fixed tab width pushed the tenth file past the right edge of the pane, where it
    /// could not be clicked and no assertion could see it. Sharing the width has no
    /// threshold at which that starts happening.
    #[test]
    fn every_open_tab_stays_inside_the_pane_however_many_are_open() {
        let (dir, _keep) = scratch_workspace("many");
        let mut h = Harness::new(dir.clone());
        for i in 0..12 {
            std::fs::write(dir.join(format!("f{i}.rs")), "fn f() {}\n").expect("file");
            h.app.open_file(&format!("f{i}.rs"));
        }
        let f = h.paint();
        assert_eq!(f.tabs.len(), 12, "every open file has a tab: {:?}", f.tabs);

        let strip = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(f.size[0], f.size[1]));
        for (index, path) in (0..12).map(|i| format!("f{i}.rs")).enumerate() {
            let r = f
                .hotspot(&crate::workspace::hotspot_id(&path))
                .unwrap_or_else(|| panic!("f{index}.rs has no tab rect"));
            assert!(
                r.right() <= strip.right() + 0.5 && r.width() > 0.0,
                "tab {index} ({path}) is drawn at {r:?}, outside the pane {strip:?}"
            );
        }
    }
}
