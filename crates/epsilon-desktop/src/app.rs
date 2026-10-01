//! Application state and the frame.
//!
//! # The shape of a frame
//!
//! [`IdeApp::paint`] is a **pure-ish function of `(&mut self, &egui::Context)`**
//! that returns a [`FrameSummary`] naming what it drew and where. That is not
//! instrumentation for its own sake: the terminal frontend keeps exactly this
//! shape (`render_rows` → rows), and it is what lets a test assert on *the
//! screen* rather than on state that merely ought to produce a screen. A feature
//! can be correct at every level and still never appear — that has happened in
//! this repository before, twice, and the fix was always the same.
//!
//! # What is honest about the first slice
//!
//! This draws real chrome and real geometry, and it says plainly that no agent is
//! attached yet rather than rendering plausible fake numbers. A screenshot that
//! looks finished but measures nothing is the failure mode this crate is most
//! exposed to, because a GUI is judged by how it looks.

use crate::Options;
use crate::layout::{IdeLayout, Preferences, Tier};
use crate::theme::IdeTheme;

/// What one painted frame contained.
///
/// Region name plus the text drawn into it. Rectangles come from
/// [`IdeApp::layout`], so a test can assert both *that* something is on screen and
/// *where*.
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
    /// Held for the lifetime of the window; the agent calls that need the loop
    /// borrow from it. Kept as a field rather than moved into the closure so a
    /// future pane cannot accidentally create a second runtime.
    _runtime: tokio::runtime::Runtime,
    /// Geometry of the most recent frame, kept for hit-testing and for tests.
    layout: IdeLayout,
}

impl IdeApp {
    /// Build the application for `opts`.
    ///
    /// Takes the runtime rather than making one so a caller that already has a
    /// runtime (the CLI's `--desktop` flag, a test) does not get a second.
    pub fn new(opts: Options, runtime: tokio::runtime::Runtime) -> Self {
        let theme = IdeTheme::resolve(opts.theme);
        Self {
            opts,
            theme,
            prefs: Preferences::default(),
            _runtime: runtime,
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

    /// Paint one frame and describe what it drew.
    ///
    /// This is the whole of the draw path. [`eframe::App::ui`] is a thin wrapper so a
    /// test can paint into an off-screen `egui::Context` at any size, with no window.
    pub fn paint(&mut self, ui: &egui::Ui) -> FrameSummary {
        let screen = ui.ctx().content_rect();
        self.layout = IdeLayout::new(screen, self.prefs);
        let l = self.layout;
        let mut summary = FrameSummary {
            tier: l.tier,
            size: [screen.width(), screen.height()],
            ..FrameSummary::default()
        };

        let painter = ui.painter();

        // ---- window chrome --------------------------------------------------
        if l.title.height() > 0.0 {
            painter.rect_filled(l.title, 0.0, self.theme.panel);
            painter.line_segment(
                [l.title.left_bottom(), l.title.right_bottom()],
                egui::Stroke::new(1.0, self.theme.border),
            );
            let label = self
                .opts
                .workspace
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("Epsilon")
                .to_string();
            painter.text(
                l.title.left_top() + egui::vec2(14.0, 23.0),
                egui::Align2::LEFT_CENTER,
                &label,
                egui::FontId::proportional(15.0),
                self.theme.title,
            );
            summary.rows.push(("title".into(), label));
        }

        if l.status.height() > 0.0 {
            painter.rect_filled(l.status, 0.0, self.theme.panel);
            painter.line_segment(
                [l.status.left_top(), l.status.right_top()],
                egui::Stroke::new(1.0, self.theme.border),
            );
            // The honest line: a count of what is known, never an invented number.
            let text = format!(
                "{} · no agent attached · {}",
                l.tier.name(),
                if l.sidebar_visible {
                    "sidebar open"
                } else {
                    "sidebar folded"
                }
            );
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
                &["Files", "Sessions", "Tasks"],
            );
        }

        if l.center_visible {
            self.pane(ui, &mut summary, l.center, "Editor", &["No file open"]);
        }

        if l.agent_visible {
            self.pane(
                ui,
                &mut summary,
                l.agent,
                "Agent",
                &["No agent attached yet.", "Ctrl+, settings  ·  Ctrl+P files"],
            );
        }

        summary
    }

    /// Draw one titled pane with a list of body rows.
    fn pane(
        &self,
        ui: &egui::Ui,
        summary: &mut FrameSummary,
        rect: egui::Rect,
        title: &str,
        rows: &[&str],
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
                // Do not draw a row that would be clipped: a half-drawn row reads
                // as a rendering fault, and "there is more below" is the layout's
                // job, not the pane's.
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
            summary
                .rows
                .push((title.to_lowercase(), (*row).to_string()));
            y += 19.0;
        }
    }
}

impl eframe::App for IdeApp {
    /// eframe 0.36 splits the old `update` in two: `logic` runs even while the window is
    /// hidden, so background work can keep counting, and may not paint; `ui` draws. This
    /// frontend has no background work yet, so `logic` keeps the default.
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        ui.ctx().set_visuals(self.theme.visuals());
        self.paint(ui);
    }
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
        // The 	extures_delta.clear() calls are not decoration. epaint panics in
        // Drop when a TexturesDelta goes away with unapplied deltas, and the first
        // run of this helper produced six failures whose message was about texture
        // deltas and whose real content was "the test harness throws away what the
        // frame produced". A harness that drops the frame is not measuring the frame.
        let mut warm = ctx.run_ui(input.clone(), |_| {});
        warm.textures_delta.clear();
        let mut app = IdeApp::new(
            Options::for_workspace(std::path::PathBuf::from("/tmp/example-workspace")),
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("test runtime"),
        );
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
        assert!(f.region("agent").iter().any(|t| t.contains("Ctrl+P")));
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
        // The terminal frontend's standing invariant: "nothing happened" must
        // never be a state the UI can be in. Here it means: every drawable size
        // puts *something* on the screen.
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
        // calling, and say nothing rather than guess. The same rule applies to
        // every number this crate will eventually draw.
        let f = paint_at(1440.0, 900.0);
        let status = f.region("status").join(" ");
        assert!(status.contains("no agent attached"));
        for banned in ["$0", "GPT", "Claude", "DeepSeek", "cache"] {
            assert!(
                !status.contains(banned),
                "the status line claims {banned:?} before an agent exists.\n{}",
                f.describe()
            );
        }
    }
}
