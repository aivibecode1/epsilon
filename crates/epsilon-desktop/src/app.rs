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

use std::sync::Arc;

use crate::Options;
use crate::layout::{IdeLayout, Preferences, Tier};
use crate::session::{IdeSession, RowKind, Transcript};
use crate::theme::IdeTheme;

use epsilon_contracts::agent_event::PermissionDecision;

/// What one painted frame contained.
///
/// Region name plus the text drawn into it. Rectangles come from [`IdeApp::layout`], so a
/// test can assert both *that* something is on screen and *where*.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FrameSummary {
    /// `(region, text)` in paint order.
    pub rows: Vec<(String, String)>,
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
    /// Geometry of the most recent frame, kept for hit-testing and for tests.
    layout: IdeLayout,
}

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
            transcript: Transcript::new(),
            draft: String::new(),
            placeholder: "ask anything  ·  enter sends  ·  shift+enter for a new line",
            refusal: None,
            layout,
        }
    }

    /// An application with **no** agent, for the test that asserts the absent state.
    ///
    /// Exists because "no agent attached" is a state a user can genuinely reach and a
    /// frame that draws a plausible model for it would be a lie — the M233 defect. The
    /// only way to test that is to be able to build the frame.
    pub fn without_agent(opts: Options) -> Self {
        Self {
            opts,
            theme: IdeTheme::resolve(None),
            prefs: Preferences::default(),
            session: None,
            notices: vec!["not connected to an agent".into()],
            transcript: Transcript::new(),
            draft: String::new(),
            placeholder: "no agent attached",
            refusal: None,
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

    /// Put a draft in the composer, as typing would.
    pub fn set_draft(&mut self, text: &str) {
        self.draft = text.to_string();
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

        // Through the `Arc`, and that is the whole reason `drain` takes `&self`: anything
        // else holding this session — a shortcut, a second window, a test — must not be
        // able to stop the frame from updating. An earlier version used
        // `Arc::get_mut`, which returns `None` the moment a handle exists, and the
        // symptom was a window whose status line knew a run was in flight while its
        // transcript stayed empty forever.
        if let Some(session) = self.session.as_ref() {
            session.drain(&mut self.transcript);
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
        if l.activity.width() > 0.0 {
            painter.rect_filled(l.activity, 0.0, self.theme.panel);
            painter.text(
                l.activity.center(),
                egui::Align2::CENTER_CENTER,
                "◧",
                egui::FontId::proportional(18.0),
                self.theme.dim,
            );
            summary.rows.push(("activity".into(), "◧".into()));
        }

        if l.sidebar_visible {
            self.pane(
                ui,
                &mut summary,
                l.sidebar,
                "Workspace",
                &self.sidebar_rows(),
            );
        }

        if l.center_visible {
            self.pane(
                ui,
                &mut summary,
                l.center,
                "Editor",
                &["No file open".to_string()],
            );
        }

        if l.agent_visible {
            self.agent_panel(ui, &mut summary, l.agent);
        }

        summary
    }

    /// The workspace pane's rows.
    fn sidebar_rows(&self) -> Vec<String> {
        vec![
            "Files".into(),
            "Sessions".into(),
            "Tasks".into(),
            format!("workspace  {}", display_path(&self.opts.workspace)),
        ]
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
        parts.push(self.layout.tier.name().to_string());
        parts.join("  ·  ")
    }

    /// Draw one titled pane with a list of body rows.
    fn pane(
        &self,
        ui: &mut egui::Ui,
        summary: &mut FrameSummary,
        rect: egui::Rect,
        title: &str,
        rows: &[String],
    ) {
        let painter = ui.painter();
        painter.rect_filled(rect, 4.0, self.theme.background);
        painter.rect_stroke(
            rect,
            4.0,
            egui::Stroke::new(1.0, self.theme.border),
            egui::StrokeKind::Inside,
        );
        painter.text(
            rect.left_top() + egui::vec2(12.0, 16.0),
            egui::Align2::LEFT_CENTER,
            title,
            egui::FontId::proportional(12.0),
            self.theme.dim,
        );
        summary.rows.push((title.to_lowercase(), title.to_string()));

        let mut y = rect.top() + 34.0;
        for row in rows {
            if y > rect.bottom() - 14.0 {
                // Do not draw a row that would be clipped: a half-drawn row reads as a
                // rendering fault, and "there is more below" is the layout's job.
                summary.rows.push((title.to_lowercase(), "…".to_string()));
                break;
            }
            painter.text(
                egui::pos2(rect.left() + 12.0, y),
                egui::Align2::LEFT_TOP,
                row,
                egui::FontId::proportional(13.0),
                self.theme.text,
            );
            summary.rows.push((title.to_lowercase(), row.clone()));
            y += 19.0;
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
            egui::ScrollArea::vertical()
                .id_salt("epsilon-agent-transcript")
                .stick_to_bottom(true)
                .max_height(body.height())
                .show(ui, |ui| {
                    ui.set_width(body.width());
                    self.draw_rows(ui, summary);
                });
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
    fn draw_rows(&self, ui: &mut egui::Ui, summary: &mut FrameSummary) {
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
            return;
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
            let text = format!("{marker}{body}");
            ui.colored_label(color, text.clone());
            summary.rows.push(("agent".into(), text));
        }

        // The pending approvals are drawn after the rows, where they cannot scroll out of
        // reach behind a long answer — the composer-adjacent placement opencode and DSH
        // both converged on, and the reason is not aesthetic.
        //
        // Drawn from the **approvals channel**, not from the transcript: the channel is
        // the authority on what is waiting, and a run that parks on a gate before its first
        // message has produced no rows at all. A reader in that state must still be told
        // the run is blocked on *them*.
        for ask in pending {
            let text = format!(
                "allow {} {} ?   [enter] allow   [esc] refuse",
                ask.capability, ask.pattern
            );
            ui.colored_label(theme.warn, text.clone());
            summary.rows.push(("agent".into(), text));
        }
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
    /// eframe 0.36 splits the old `update` in two: `logic` runs even while the window is
    /// hidden, so background work can keep counting, and may not paint; `ui` draws. This
    /// frontend drains its event queue in `ui`, which is deliberate — a run that finished
    /// while the window was hidden produces no `AgentEnd` on screen until the window is
    /// shown again, and `eframe` skips the whole `ui` pass while hidden. `logic` is where
    /// that belongs, and it is the next slice.
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
        assert!(workspace.contains(&"Files"), "{workspace:?}");
        assert!(workspace.contains(&"Sessions"), "{workspace:?}");
        assert!(workspace.contains(&"Tasks"), "{workspace:?}");
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
    fn a_pending_approval_outranks_the_layout_tier_in_the_status_line() {
        // A status line that shows the layout while a permission ask is unanswered is a
        // status line pointing the reader at the wrong thing.
        let app = IdeApp::without_agent(Options::for_workspace("/tmp/ws"));
        let line = app.status_line();
        assert!(line.starts_with("no agent attached"), "{line}");
        assert!(
            line.contains("no-sidebar") || line.contains("full"),
            "{line}"
        );
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
}
