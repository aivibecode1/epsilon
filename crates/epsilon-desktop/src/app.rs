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
//! It believes exactly what the agent told it. The context, token and cost figures come
//! from the same accessors [`epsilon_core::Agent::session_state`] is built from, so the
//! three frontends cannot disagree about the same session, and a number this window shows
//! is a number the CLI would show.
//!
//! **Through the accessors, and not through `session_state` itself** — a decision, and the
//! reason is in [`IdeApp::read_metrics`]. The wire payload is right for two transports
//! that serve it once per request; on the paint thread it also costs
//! [`Agent::goal`](epsilon_core::Agent::goal), which is the read [`IdeApp::goal`] caches
//! precisely because it is expensive.
//!
//! When there is nothing to say, it says nothing: before M233's wiring landed, this pane
//! said "no agent attached" rather than inventing a cost. That rule has not changed, and a
//! test still enforces it for the case where there is genuinely no agent.
use crate::Options;
use crate::layout::{IdeLayout, Preferences, SIDEBAR_MAX, SIDEBAR_MIN, SIDEBAR_STEP, Tier};
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
    /// `(region, font size in points)` for every string drawn with
    /// [`egui::Painter::text`], in paint order.
    ///
    /// **The only record of a painter text size, and it exists because a size is not
    /// recoverable from the text.** `rows` carries *what* was drawn and `row_rects`
    /// carries *where*, so a test could ask whether the title bar drew the workspace name
    /// and where it landed, but not **how big** — which is the whole of AGENTS.md §8's
    /// "text scaling" clause. A window that scaled the widget half and left the painter
    /// half alone would pass every other test in this file.
    ///
    /// Recorded by [`paint_text`], which is the only place this crate paints a string
    /// outside a widget, so a size cannot be drawn without being recorded.
    pub text_sizes: Vec<(String, f32)>,
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

/// Give a painter-drawn region an accessible name. Nothing else.
///
/// **Why this has to exist at all.** Everything in this window is drawn with
/// [`egui::Painter`] calls on raw rectangles, and **a painter call emits no accessibility
/// node at all**. `Painter::text` puts glyphs into the tessellation and nothing into the
/// AccessKit tree, so before this function the transcript — the one region a screen-reader
/// user most needs — was invisible. No amount of toolkit capability fixes that from the
/// outside, which is what ADR-022 is about.
///
/// **What it can honestly publish.** [`egui::Response::widget_info`]
/// (`egui-0.36.2/src/response.rs:869`) is the hook that puts a node into that tree, and
/// [`egui::WidgetInfo`] (`egui-0.36.2/src/data/output.rs:563`) is the whole of what it
/// accepts: a name, a value and state. It has **no role field, no description and no
/// relations**, so `widget_info` alone cannot express an ARIA landmark.
///
/// **This paragraph said egui 0.36 could not express a landmark at all, and that was wrong**
/// (corrected 2026-10-04, M301). It is true of `WidgetInfo`, whose `WidgetType`
/// (`egui-0.36.2/src/lib.rs:623-668`) ends at `Other`, so the table in `response.rs:935-958`
/// caps out at `Role::Pane`. But roles are assignable: `Context::accesskit_node_builder` is
/// **public** (`egui-0.36.2/src/context.rs:3684-3690`) and `accesskit::Node::set_role` is
/// public, which is how `name_region` now stamps a landmark role. **The ordering is the
/// subtlety**: `fill_accesskit_node_from_widget_info` rewrites the role on the same pass, so
/// the role must be set *after* the `widget_info` call below or it is silently undone.
/// Nothing here is a `describedby` relation — egui 0.36 still cannot express one.
///
/// **The label is the caller's, and it must be bounded and truthful.** A region that
/// published its own contents would have a screen reader announce every row on every
/// repaint, which is unusable; a region that published a stale count is worse than one
/// that published nothing. Every caller therefore names the region and adds a number it
/// already has.
///
/// `Sense::empty()` on purpose: the region already owns its pointer — the scroll area
/// scrolls, the editor is read-only — and an interact sense layered on top of it would
/// take input away from what is there. [`egui::Ui::interact`] registers the widget without
/// moving the pane's cursor, so this changes no layout and paints nothing.
///
/// `&str` rather than `String`: `widget_info` takes an `Fn`, not an `FnOnce`
/// (`egui-0.36.2/src/response.rs:869`), so the label has to be cloneable to be built. One
/// `String` per frame, per region, for a name a screen reader reads once.
///
/// **Negative control.** With the `widget_info` call below commented out — everything
/// else, including every label, left exactly as it is — all eight `app::tests`
/// accessibility tests fail with the tree holding only egui's own widget names
/// (`"☰"`, `"◧"`, `"⑂"`, `"Send"`), and the other 121 lib tests still pass. That is what
/// makes them tests of the tree rather than of the intent.
///
/// Re-measured when the window chrome was named as well. Commenting out **every**
/// `name_region` call above **and** both [`name_node`] calls — no other change — gave
/// **130 passed, 6 failed**: the six tests added with that chrome, and nothing else. The
/// eight this helper already had still passed, because the control removed only the new
/// calls. The tree then held exactly `["☰", "Workspace files · 2 files", "Editor, read-only
/// · no file open", "Send", "Agent transcript · 0 entries", "◧", "⑂"]` — and the composer is
/// **absent from that list because it has no label, not because it has no node**: read by
/// its own id it was there, with `label == None`. The five warnings that build emitted
/// (`name_node` and the four new salts never used) are the same fact from the other side.
///
/// **It draws nothing.** The same control run compared the two frames' tessellations:
/// `FullOutput::shapes` held 47 entries in both and its `Debug` output was identical line
/// for line. The same comparison on the frame the chrome tests draw — an empty 1440×900
/// window — held **39 entries in both builds** and a byte-identical 154 260-character
/// `Debug` dump. `Ui::interact` writes only to `this_pass.widgets`
/// (`egui-0.36.2/src/context.rs:1248`) and `widget_info` writes only to the AccessKit node
/// builder and `Memory::widget_info` (`egui-0.36.2/src/response.rs:889`); neither builds a
/// shape. This is metadata, and the frame is the proof.
/// Give a painted region an accessible name and an ARIA landmark role.
///
/// `role` is the landmark a screen reader uses to offer a rotor of the window's regions — what
/// the WAI-ARIA Authoring Practices call landmarks, and what a sighted reader gets from the
/// layout for free. Passing [`egui::accesskit::Role::Pane`] keeps the old behaviour for a region
/// that is a container but not a landmark.
fn name_region(
    ui: &egui::Ui,
    rect: egui::Rect,
    salt: &str,
    label: &str,
    role: egui::accesskit::Role,
) {
    // `make_persistent_id` is `self.id().with(salt)` (`egui-0.36.2/src/ui.rs:884`), and
    // `pane_ui` builds each pane's id the same way, so the result is stable for as long as
    // the pane is. **Never derive the salt from a frame counter**: an id that moved every
    // repaint is a node that moves every repaint, which is worse for a screen reader than
    // no node at all.
    let response = ui.interact(rect, ui.make_persistent_id(salt), egui::Sense::empty());
    response.widget_info(|| egui::WidgetInfo {
        // `WidgetType::Panel` is what egui maps to AccessKit's `Role::Pane`
        // (`egui-0.36.2/src/response.rs:950`). That is the container role, and the role the
        // landmark call below **overrides** — kept as the starting point so a caller that
        // passes `Role::Pane` gets exactly the behaviour this helper had before landmarks.
        typ: egui::WidgetType::Panel,
        enabled: true,
        label: Some(label.to_string()),
        // `WidgetInfo` derives neither `Default` nor any of these, so "unset" is written
        // out rather than defaulted. Everything left `None` here is something egui has no
        // honest value for.
        current_text_value: None,
        prev_text_value: None,
        selected: None,
        value: None,
        text_selection: None,
        hint_text: None,
    });
    // **After** `widget_info`, never before: egui's own pass rewrites the role from
    // `WidgetType`, so a role set earlier would be overwritten every frame and the fix would
    // look like it worked on the frame it was written.
    name_role(&response, role);
}
/// Give a node egui **already built** an accessible name, and change nothing else.
///
/// The companion to [`name_region`] for the regions that are *not* painter-drawn. egui emits
/// a node for every widget it draws — the three buttons on the activity bar and the
/// composer's `TextEdit` among them — and that node already carries a role, a value, and for
/// a selected button a toggled state. A [`name_region`] beside it would be a second node for
/// one control: a screen reader announces the region twice and the first node's existence is
/// hidden behind the second.
///
/// **Why `Node::set_label` and not `Response::widget_info`.** They are not the same act.
/// `widget_info` (`egui-0.36.2/src/response.rs:869`) **writes** the node from the
/// `WidgetInfo` it is handed, and on a frame where the widget was clicked or edited it also
/// pushes an `OutputEvent` onto the context (`egui-0.36.2/src/response.rs:872-887`) — so
/// naming the composer through it would put a second, value-less copy of every edit onto the
/// queue the application already received one of. `Context::accesskit_node_builder`
/// (`egui-0.36.2/src/context.rs:3684`) gets the node egui built and touches one field on it.
/// The role, the value, the placeholder and the toggled state stay exactly as egui left them,
/// which is also what makes this safe to call beside a `Button::selected`.
///
/// **Still only a name.** `set_label` fills `accesskit::Node::label` and nothing else: no
/// description and no relation, exactly as through `WidgetInfo`. See [`name_region`] for the
/// rest of what egui 0.36 can and cannot express.
/// `name_node` writes only [`accesskit::Node::label`].
///
/// The name is what a screen reader announces, and it is the **only** field written here on
/// purpose: `egui::Response::widget_info` would also queue an `OutputEvent` on click and edit
/// frames, so naming a row through it would fire a synthetic click beside every real one. A role
/// is set separately, by [`name_role`], because a node that announces with the right words and
/// the wrong role is still announced as "button" or "group" by a reader that trusts the role
/// first.
pub(crate) fn name_node(response: &egui::Response, label: &str) {
    response.ctx.accesskit_node_builder(response.id, |node| {
        node.set_label(label.to_string());
    });
}

/// Name the vertical scrollbar **of a scroll area that drew one this frame** (M312).
///
/// **The last unnamed node in this window's accessibility tree, and the fix is a derivation
/// rather than a guess.** egui creates an `accesskit` node for every scrollbar it draws and labels
/// none of them, so `desktop-a11y-probe.ps1` exits 1 and has since the probe was written. Naming it
/// naively — unconditionally, for every scroll area — is the mistake the probe's own doc warns
/// about: `Context::accesskit_node_builder` is **get-or-create**, so a node named for a bar that
/// was never drawn is a **phantom** in the tree a reader browses.
///
/// # Why the predicate can be reproduced
///
/// egui decides to draw the bar at `egui-0.36.2/src/containers/scroll_area.rs:1256-1260`:
///
/// ```text
/// ScrollBarVisibility::VisibleWhenNeeded => content_is_too_large
/// ```
///
/// and `content_is_too_large` (`:1203-1206`) is
/// `direction_enabled[1] && (limit_rect.height().ceil() < content_size.y)` — and `limit_rect` is
/// `outer_rect` when the bars float (`:1197-1201`), where `outer_rect` is
/// `inner_rect + current_bar_use` (`:1195`) and `current_bar_use` is
/// `allocated_width * show_bars` (`:757-761`).
///
/// **So the derivation holds exactly when the scroll bars are allocated no space** — and egui's
/// floating style allocates none: `ScrollStyle::floating()` sets `floating_allocated_width: 0.0`
/// (`style.rs:648`), and `allocated_width()` (`:656-658`) returns that. With zero allocated,
/// `outer_rect == inner_rect`, `limit_rect == inner_rect`, and the published pair *is* egui's
/// threshold. The node's id is `id.with(1)` (`:1293`, `d == 1` for the vertical axis).
///
/// **The premise is the number, not the boolean, and getting that wrong is what this comment
/// records.** A first version of this function took a `floating: bool` and refused to name
/// anything when it was true — which meant it **never named anything**, because these bars float.
/// Measured on the tree that needed one: `content=[248.0 4197.0] inner_h=744.0`. The guard was
/// the bug. `the_window_allocates_no_width_to_its_scroll_bars` holds the thing that actually
/// matters, so a future style that *does* reserve width says so here first.
///
/// Should that ever happen, this predicate becomes conservative by that width rather than wrong,
/// and the bar is then named one frame late rather than never.
///
/// **What remains is one frame, and it is stated rather than hidden:** egui animates the bar in
/// (`show_bars_factor`, `:1262-1268`), so on the first frame content overflows, the bar is *not*
/// drawn yet while this predicate already says it will be. The node is then one frame early. A
/// scrollbar a reader reaches a frame before it appears is a far smaller cost than a scrollbar no
/// reader can name at all, which is the state this replaces.
///
/// Only a name, like [`name_node`]: no role, because egui already gives the bar `Role::ScrollBar`
/// through `WidgetInfo` (`scroll_area.rs:1336`).
pub(crate) fn name_scrollbar<R>(
    ctx: &egui::Context,
    out: &egui::containers::scroll_area::ScrollAreaOutput<R>,
    what: &str,
) {
    // Spelled the readable way round, and the same test: the bar is drawn when the content is
    // **taller** than the space it has. `!(a > b)` says the same thing and trips a lint nobody
    // can skim past.
    if out.content_size.y <= out.inner_rect.height().ceil() {
        return;
    }
    ctx.accesskit_node_builder(out.id.with(1usize), |node| {
        node.set_label(format!("Scroll · {what}"));
    });
}

/// Stamp an [`accesskit::Role`] onto a widget egui already built.
///
/// # Why this exists separately from [`name_node`]
///
/// `egui`'s `WidgetInfo` has no role field (`egui-0.36.2/src/data/output.rs:563-592`) and its
/// `WidgetType` stops at `Other` (`lib.rs:623-668`), so the table in `response.rs:935-958` caps
/// out at `Role::Pane` — which is why every pane here reads as a pane and nothing reads as a
/// landmark. But `Context::accesskit_node_builder` is **public** (`context.rs:3684-3690`) and
/// `accesskit::Node::set_role` is public, so a role is assignable; the only thing to get right is
/// the **ordering**, because `fill_accesskit_node_from_widget_info` rewrites the role on the same
/// pass and would undo this. Call it after the widget's own `widget_info`.
pub(crate) fn name_role(response: &egui::Response, role: egui::accesskit::Role) {
    response.ctx.accesskit_node_builder(response.id, |node| {
        node.set_role(role);
    });
}

/// How much of a transcript row's own text reaches its accessible name.
///
/// Bounded because **a name is announced every time the reader reaches the row**, and a
/// row's text is the reader's or the model's and can be any length. The bound is not a way
/// of being polite: the whole line is **already on the node**, as its value — egui puts the
/// laid-out text there for every `Label` it draws (`egui-0.36.2/src/widgets/label.rs:282`
/// → `egui-0.36.2/src/response.rs:962-964`). A name built from the entire row would have
/// the reader hear it twice, once as the name and once as the text.
const NAME_EXCERPT: usize = 80;

/// What kind of transcript row this is, in the words a reader would use for it.
///
/// The kind and never the content: [`RowKind`] carries who said it and what they said, and
/// a name is the half of that a reader navigates by. **The tool row is the one that carries
/// facts rather than a noun** — its name and whether it worked — because `▸ bash` and
/// `✓ bash` are the same row drawn twice, and a reader who cannot see which marker is on
/// screen has no way to tell a running call from a finished one.
fn row_kind_name(kind: &RowKind) -> String {
    match kind {
        RowKind::User => "You said".to_string(),
        RowKind::Assistant => AGENT_ROW_NAME.to_string(),
        RowKind::Thinking => "Reasoning".to_string(),
        RowKind::Tool { name, failed, .. } => format!(
            "Tool call · {name} · {}",
            match failed {
                None => "running",
                Some(false) => "succeeded",
                Some(true) => "failed",
            }
        ),
        // The capability and the pattern are in the row's own text, so the excerpt
        // [`row_name`] adds carries them; repeating them here would say the same words
        // twice.
        RowKind::Approval { .. } => "Permission request".to_string(),
        RowKind::Changed => "File changed".to_string(),
        RowKind::Diff => "Changed line".to_string(),
        RowKind::Notice => "Notice".to_string(),
    }
}

/// The one kind name that is a constant rather than a reading.
///
/// [`Self::draw_markdown`] is reached from the assistant row and from no other, so every
/// block it names belongs to the agent's answer. It states that here, next to the function
/// whose doc says it, rather than taking a parameter every caller would pass the same word.
const AGENT_ROW_NAME: &str = "Agent said";

/// One transcript line's accessible name: `Transcript row · {kind} · {excerpt}`.
///
/// **"Transcript row", because egui attaches these nodes flat to the window root.** The
/// transcript's own node is a *sibling* of them rather than a parent
/// (`Context::accesskit_node_builder` walks for an ancestor that already has one,
/// `egui-0.36.2/src/context.rs:607-628`, and the pane `Ui`s carry none), so a reader
/// arriving at a node out of context has nothing else to tell it which region it came from.
/// The same prefix the palette rows use, for the same reason.
///
/// `text` empty yields the kind alone. That is not a fallback: a markdown rule is drawn as
/// `--------`, which is a picture of a divider and says nothing about the conversation, so
/// the rule arm names itself and leaves the excerpt out.
fn row_name(kind: &str, text: &str) -> String {
    let excerpt = excerpt_of(text);
    if excerpt.is_empty() {
        format!("Transcript row · {kind}")
    } else {
        format!("Transcript row · {kind} · {excerpt}")
    }
}

/// `text` as **one bounded line**: newlines folded to spaces, at most [`NAME_EXCERPT`]
/// characters, an ellipsis when anything was cut.
///
/// Folded because a name is read as one utterance and a pasted prompt is several lines
/// long. Cut on a **character** boundary, never a byte one, so a row whose text is not
/// ASCII cannot panic the frame that draws it.
fn excerpt_of(text: &str) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= NAME_EXCERPT {
        return flat;
    }
    let cut: String = flat.chars().take(NAME_EXCERPT - 1).collect();
    format!("{cut}…")
}

/// What a reader did in the transcript pane during one frame.
///
/// Named rather than a tuple because the two halves are different kinds of thing — one
/// answers a gate, the other opens a review — and a caller reading
/// `if let Some((id, decision))` next to `if let Some(path)` cannot tell at a glance which
/// is which.
#[derive(Default)]
struct TranscriptClicks {
    /// A permission answered with the mouse: `(request id, decision)`.
    decision: Option<(String, PermissionDecision)>,
    /// A "changed" row clicked, as the summary text the row carries.
    change: Option<String>,
}

// ==========================================================================================
// Transcript layout — measured once per row, reused every frame
// ==========================================================================================

/// One row's height, and the word that says whether that height is still this row's.
#[derive(Clone, Copy)]
struct RowHeight {
    /// [`row_seen`] for the row this was measured from.
    seen: u64,
    /// How tall the row is, **including** the `item_spacing.y` that separates it from the
    /// next one — so a prefix sum over these is where every row actually starts.
    ///
    /// `NaN` means "not measured yet", which no layout can produce and which a real height
    /// of zero can, so the two cannot be confused.
    height: f32,
}

/// Every transcript row's height, measured once, plus the prefix sums the frame walks.
///
/// # Why this exists, with the measurement
///
/// M266 measured **104 ms/frame** at 800 rich assistant rows, and M275 measured where it
/// went: **99.481 ms of it — 95% — is egui's own text layout and shaping, per block, per
/// frame, for every row whether or not the row is on screen** (`tests/frame_cost.rs`,
/// `where_the_transcript_milliseconds_go`). The 4.919 ms we own is `plain_of` plus
/// `markup::job`. Caching those two would have bought under 5% and *looked* like progress,
/// which is the whole reason M275 exists.
///
/// The only thing that addresses the 99 ms is not building a widget for a row nobody can
/// see — and the transcript could not do that because egui's `show_rows` requires a
/// **uniform** row height and a transcript's rows are not uniform. `workspace.rs:314` gets
/// away with `show_rows` because a tree row is 18 points and always 18 points.
///
/// # Why it is safe to cache
///
/// A height depends on the row's text and on the pane's width. Both are in the fingerprint:
/// the width here, and the row's identity in [`RowHeight::seen`]. Every write to a row's
/// drawn text in `session.rs` is a **whole-`String` assignment**
/// (`session.rs:446,451,468,473` for streamed prose, `:538` for tool output), so a changed
/// row always has a different heap address — which is why the fingerprint reads pointers
/// rather than bytes. See [`row_seen`] for the argument in full.
#[derive(Default)]
struct RowLayout {
    /// The pane width the heights were measured at. A resize drops every one of them,
    /// because a paragraph that wrapped at 500 points wraps differently at 400.
    width: f32,
    /// The text scale the heights were measured at.
    ///
    /// **A zoom change is as much a re-measure as a resize, and for the same reason**: a
    /// row measured at 13 points is 26 points tall at 200%, so keeping the old heights
    /// makes the transcript's rows overlap until something else invalidates them. Without
    /// this field the window's own text-scale control would leave the transcript's geometry
    /// describing a different zoom from the one on screen.
    scale: f32,
    /// One entry per transcript row, in the transcript's order.
    rows: Vec<RowHeight>,
    /// `offsets[i]` is where row `i` starts and `offsets[n]` is the whole transcript's
    /// height. One more entry than `rows`, because the end is a position too.
    offsets: Vec<f32>,
}

impl RowLayout {
    /// Where row `i` starts, and the height of the whole transcript.
    fn at(&self, i: usize) -> f32 {
        self.offsets.get(i).copied().unwrap_or(self.total())
    }
    /// The transcript's height, which is what the scroll bar's extent comes from.
    fn total(&self) -> f32 {
        self.offsets.last().copied().unwrap_or(0.0)
    }
    /// The rows a viewport spanning `top..bottom` has to draw.
    ///
    /// **The rows must overlap the band**, not merely start inside it: the last row on
    /// screen usually starts well above the bottom edge and is mostly visible. One row of
    /// slack on each side costs two rows and buys immunity from a half-drawn edge — a
    /// rounding difference between the offset table and the clip rectangle is a visible
    /// gap at the top or bottom of the pane, and it is the kind of thing a reader blames
    /// on the data.
    fn span(&self, top: f32, bottom: f32) -> std::ops::Range<usize> {
        let n = self.rows.len();
        if n == 0 {
            return 0..0;
        }
        // The first row whose **bottom** is below `top` has already scrolled past; the
        // first row whose **start** is below `bottom` is entirely out of sight.
        let mut first = self
            .offsets
            .partition_point(|y| *y <= top)
            .saturating_sub(1);
        let mut last = self.offsets.partition_point(|y| *y < bottom);
        first = first.saturating_sub(1);
        last = (last + 1).min(n);
        first.min(last)..last
    }
}

/// Everything about a row that changes what it draws, folded into one word.
///
/// **Pointers, not bytes, and that is the whole trick.** Hashing 800 rows' text every frame
/// is the obvious way to notice that a streamed answer grew, and it costs about 100 µs a
/// frame to look at 160 KB of prose — a permanent tax to save a one-off re-measure. Reading
/// `(ptr, len)` per row touches no bytes at all: two words per row, about 2 ns each.
///
/// The claim this rests on is that **a row's text is never edited in place**. It is
/// true: every mutation of a row's drawn text in `session.rs` is a whole-`String`
/// assignment — `self.rows[i].text = text` at `session.rs:446`, `:451`, `:468` and `:473`,
/// and `*output = bounded_tail(…)` at `:538`, which returns a freshly joined `String`. A
/// freshly built string cannot share a buffer with the live one it is replacing, so a row
/// whose content changed always reports a different address.
///
/// The failure that would look like is a stale height on a row that changed, which is a
/// row drawn overlapping its neighbour — so the invariant is cited rather than assumed, and
/// [`row_offsets`] re-measures on every mismatch rather than trusting a partial one.
fn row_seen(row: &crate::session::Row) -> u64 {
    fn fold(h: &mut u64, s: &str) {
        // A string's *address* first and its length second: two different strings of the
        // same length must not fold together, and two copies of the same string never
        // share a buffer, so the address is the discriminator.
        *h = (*h ^ (s.as_ptr() as u64)).wrapping_mul(0x0100_0000_01b3);
        *h = (*h ^ (s.len() as u64)).wrapping_mul(0x0100_0000_01b3);
    }
    // A tag per kind, because two rows with identical text are not the same widget: the
    // marker is drawn from the kind, and a tool row with no arguments draws its name.
    let mut h: u64 = match &row.kind {
        RowKind::User => 1,
        RowKind::Assistant => 2,
        RowKind::Thinking => 3,
        RowKind::Tool { name, output, .. } => {
            let mut h = 4u64;
            fold(&mut h, name);
            fold(&mut h, output);
            h
        }
        RowKind::Approval { .. } => 5,
        RowKind::Changed => 6,
        RowKind::Diff => 7,
        RowKind::Notice => 8,
    };
    fold(&mut h, &row.text);
    h
}

/// The palette's search box and cursor.
///
/// A field rather than two, because "what is open" and "what is selected" are one piece of
/// state: a palette that remembered its query but forgot its selection would highlight
/// something other than the row Enter would open.
#[derive(Default)]
struct Palette {
    query: String,
    /// Index into the filtered entries.
    selected: usize,
}

/// The model picker: two stages over the shared catalog builders.
///
/// Stage one lists providers (`provider` is `None`); choosing one drills into
/// its models. The rows themselves come from `epsilon_tui::picker` — the same
/// builders the terminal draws — so the two front ends cannot disagree about
/// what a provider holds; only the pixels here are the window's own.
#[derive(Default)]
struct ModelPicker {
    query: String,
    /// Index into the filtered entries.
    selected: usize,
    /// The drilled provider, or `None` for the provider list.
    provider: Option<String>,
}

/// One drawable model-picker row, either stage.
///
/// `Clone` for the same reason as `PaletteAction`: the chosen row is read
/// after the borrow that produced it has ended.
#[derive(Clone)]
struct ModelPickerEntry {
    /// What the reader sees and types.
    label: String,
    /// Dim meta on the right (model counts, auth marks).
    meta: String,
    /// The provider id (stage one) or model id (stage two) choosing applies.
    id: String,
    /// Stage one rows drill; stage two rows apply.
    is_provider: bool,
}

/// What a palette row does when it is chosen.
///
/// An enum rather than a function pointer, deliberately: a table of `fn(&mut IdeApp)` would
/// be shorter to write and would make every entry's effect invisible at the point it is
/// listed — and a list of commands whose effects cannot be read in the list is a list
/// nobody trusts.
///
/// `Clone` because the chosen row is read after the borrow that produced it has ended, and
/// borrowing through would mean holding `self` across the action that mutates it.
#[derive(Clone)]
enum PaletteAction {
    /// Open this workspace-relative file in the editor.
    Open(String),
    /// Show this view in the sidebar.
    Sidebar(SidebarView),
    /// Walk the workspace again.
    ReloadTree,
    /// Do this to the session's goal.
    Goal(GoalCommand),
    /// Run one of the window's own slash heads, as `/head` typed in the
    /// composer would. The head, not a copy of what it does: the row must not
    /// carry a second implementation of anything.
    Verb(&'static str),
}

/// The goal verbs the window can run.
///
/// `Create` takes the palette's own query as the objective: typing the objective and
/// choosing the verb is one gesture, and it is the same one `/goal <objective>` is in the
/// terminal. A `create` with an empty query refuses and says so rather than creating a goal
/// whose objective is the empty string — a goal with nothing to aim at that runs to its cap.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum GoalCommand {
    /// Create a goal whose objective is the query that came with it.
    Create,
    /// Stop an armed goal from continuing itself.
    Pause,
    /// Arm it again.
    Resume,
    /// Remove the goal entirely.
    Clear,
}

/// One command the window offers.
///
/// **The `id` is the point of the table.** Without a stable name every consumer — the
/// palette, a future shortcut binding, a `/help` — has to match on the *label*, and a label
/// is what a reader sees and therefore what gets reworded. `opencode` builds its palette from
/// its command ids for exactly this reason.
struct Command {
    /// Stable, for bindings and help. Never shown to the reader.
    id: &'static str,
    /// What the reader sees and types.
    label: &'static str,
    /// Shown on the right, so a file and a command sharing a name are told apart.
    hint: &'static str,
    action: PaletteAction,
}

/// One row in the palette.
struct PaletteEntry {
    /// What the reader sees, and what the query matches against.
    label: String,
    /// A hint on the right, so a file and the command that mentions it are told apart.
    hint: String,
    /// The stable command id, for rows that are commands.
    ///
    /// Carried so a hotspot can name the row by `command:<id>` rather than by its label. A
    /// label is what a reader sees and therefore what gets reworded; a test that finds
    /// `goal: pause` by its text fails the day someone improves the wording, and fails for a
    /// reason that has nothing to do with what it was testing.
    command: Option<&'static str>,
    action: PaletteAction,
}

/// How many rows the palette shows before it says it is showing fewer.
///
/// A cap with a marker rather than a silent truncation: the same rule as every other list in
/// this window, and the same reason — a row that is quietly missing reads as a row that does
/// not exist.
const PALETTE_LIMIT: usize = 40;

/// The id salt the editor pane's accessibility node is registered under.
///
/// **One constant because the pane has three states** — a file, a change under review, and
/// nothing open — and each names itself separately. Three literals would be three chances
/// to spell one of them differently, and a spelling that drifts between states is a screen
/// reader that loses the pane every time the reader opens a file. The transcript's salt is
/// the literal in [`IdeApp::agent_panel`], which is also its scroll area's `id_salt`.
const EDITOR_SALT: &str = "epsilon-editor-pane";

/// The id salts the three sidebar panes name themselves under.
///
/// Distinct because all three are drawn into the **same** pane `Ui`: one salt shared by
/// the file tree, the subagent list and the session list would be one node whose name
/// changes with the activity bar, which is the "node that moves" failure `name_region`
/// is written against.
const FILES_SALT: &str = "epsilon-sidebar-files";
const TASKS_SALT: &str = "epsilon-sidebar-tasks";
const SESSIONS_SALT: &str = "epsilon-sidebar-sessions";

/// The id salts the window chrome names itself under.
///
/// **Four literals because the title bar, the status bar and the activity bar are all drawn
/// on the same root `Ui`.** One salt shared across them would be a single node whose name
/// changed with whichever region was drawn last — the same "node that moves" failure the
/// sidebar salts above exist to prevent, one level up.
const TITLE_SALT: &str = "epsilon-title-bar";
const STATUS_SALT: &str = "epsilon-status-bar";
const ACTIVITY_SALT: &str = "epsilon-activity-bar";

/// The id salt the goal band's accessibility node is registered under.
///
/// Its own for the same reason as `EDITOR_SALT`, and more sharply: the band is drawn **only
/// when the session has a goal**, into the same pane `Ui` as the transcript. A shared salt
/// would mean the transcript's node renamed itself to the goal band on every frame that had a
/// goal and back again on every frame that did not.
const GOAL_SALT: &str = "epsilon-goal-band";

/// The palette's search box, so its focus survives between frames.
fn palette_input_id() -> egui::Id {
    egui::Id::new("epsilon-palette-search")
}

/// The model picker's search box id: its own, so the two overlays never share
/// a focused widget.
fn model_input_id() -> egui::Id {
    egui::Id::new("epsilon-model-search")
}

// ---- the sidebar's two controls ------------------------------------------------------

/// The id salt the sidebar's resize handle is registered under.
///
/// **A salt of its own, not one shared with the sidebar pane**, for the reason
/// [`EDITOR_SALT`] gives: one id would be one node whose name changes as the handle is
/// dragged, and a node that renames itself every frame is worse for a reader than no node.
const SIDEBAR_HANDLE_SALT: &str = "epsilon-sidebar-handle";

/// The two hotspot ids the sidebar's controls record where they were drawn.
///
/// So a test can find the handle and press on **the coordinates the frame used**, the way
/// [`FrameSummary::hotspot`] already lets every other click in this window be tested. A
/// test that drags at a guessed `x` is testing the arithmetic rather than the control.
const SIDEBAR_HANDLE_HOTSPOT: &str = "sidebar-handle";
const SIDEBAR_TOGGLE_HOTSPOT: &str = "sidebar-toggle";

/// The chord that folds and unfolds the sidebar, as it is written on screen.
///
/// The window's bindings, read from the user's `keybindings.json` when there is one.
///
/// **Replaces a constant, and that is the point.** This used to be
/// `const SIDEBAR_TOGGLE_KEY: &str = "ctrl+b"`, hand-written and used in four hint strings,
/// with the comment above it claiming the terminal bound `ctrl+b` to page-up. **It does not** —
/// `ctrl+b` is the terminal *composer's* readline back-character, and it is not in the
/// terminal's table at all. The comment was wrong and the constant was unreachable by a user's
/// configuration.
///
/// Now: `Ctrl`+`B` is the convention every other IDE uses for "toggle the primary side bar" —
/// VS Code, Sublime and Atom all bind it there — so a reader arriving at this window has the
/// binding in their hands before they read it, and a reader who wants it elsewhere can now say so.
fn window_keys() -> crate::keys::KeyBindings {
    match epsilon_core::config::global_dir() {
        Some(dir) => crate::keys::KeyBindings::load(&dir),
        None => crate::keys::KeyBindings::defaults(),
    }
}

/// The two glyphs the title bar's toggle draws.
///
/// **A picture of the state, and never the accessible name.** `◨` is the sidebar shown and
/// `◧` the sidebar hidden — the filled half is the half that is there. [`name_node`] renames
/// the node either way, for the reason `activity_bar` records: a reader who hears a glyph has
/// learned nothing.
const SIDEBAR_SHOWN_GLYPH: &str = "◨";
const SIDEBAR_HIDDEN_GLYPH: &str = "◧";

/// Geometry of the title bar's toggle, in points.
///
/// **Un-scaled, and that is a known half-measure, not a decision.** An earlier version of this
/// comment claimed the title bar was "the one band the text-scaling work deliberately left
/// alone" and that "a control floating in a band that did not grow with the text would be the
/// half-measure that change exists to avoid" — **which describes exactly what this is.** At
/// 200% the band's height is still [`crate::layout::TITLE_H`] and this control is still 24 pt,
/// so a 30-point title line runs to 39.4 pt while the band ends at 36.
///
/// The overflow is a **descender spill into the gutter**, not onto the body: the body starts
/// at 42 pt and the panes paint *after* the chrome. Corrected 2026-10-04 against the measured
/// font metric (`row_height = 1.149 × size`, baseline `0.932 × size`, from `Ubuntu-Light.ttf`
/// hhea — OS/2 `USE_TYPO_METRICS` is not set); the version this replaces said "a 30-point line
/// overflows its band onto the body", which the arithmetic does not support.
///
/// Left unscaled deliberately for now, because the honest fix is to scale
/// [`crate::layout::TITLE_H`] and [`crate::layout::STATUS_H`] **and** their baselines together,
/// and scaling one without the other is the half-measure this comment used to deny. That is
/// the open item, not a settled design.
const SIDEBAR_TOGGLE_W: f32 = 24.0;
/// Distance from the title bar's right edge to the toggle's centre.
const SIDEBAR_TOGGLE_INSET: f32 = 22.0;
/// Below this the title bar has no room for a second control beside the workspace name, and
/// the toggle is not drawn.
///
/// **Not a silent loss.** The chord is printed in the status bar whatever the title bar can
/// afford, so nothing is advertised here that the window cannot do — it is only the pointer
/// route that is withdrawn, and only where there is nowhere to put it.
const SIDEBAR_TOGGLE_MIN_W: f32 = 320.0;

/// Every command this window offers, in the order it is listed.
///
/// One table, read by the palette and by whatever binds keys next. The goal verbs are here
/// because until this milestone the window could **watch** a goal (M258) and not **control**
/// one — the terminal's `/goal` had create, pause, resume and clear, and the desktop had
/// none of them, which is the asymmetry this whole milestone is about closing.
fn commands() -> Vec<Command> {
    fn view(id: &'static str, label: &'static str, v: SidebarView) -> Command {
        Command {
            id,
            label,
            hint: "command",
            action: PaletteAction::Sidebar(v),
        }
    }
    fn goal(id: &'static str, label: &'static str, c: GoalCommand) -> Command {
        Command {
            id,
            label,
            hint: "goal",
            action: PaletteAction::Goal(c),
        }
    }
    vec![
        view("view.files", "files", SidebarView::Files),
        view("view.subagents", "subagents", SidebarView::Tasks),
        view("view.sessions", "sessions", SidebarView::Sessions),
        Command {
            id: "tree.reload",
            label: "reload tree",
            hint: "command",
            action: PaletteAction::ReloadTree,
        },
        goal("goal.create", "goal: create", GoalCommand::Create),
        goal("goal.pause", "goal: pause", GoalCommand::Pause),
        goal("goal.resume", "goal: resume", GoalCommand::Resume),
        goal("goal.clear", "goal: clear", GoalCommand::Clear),
    ]
}

/// What the palette can reach: every file in the workspace, plus every command.
impl IdeApp {
    /// The rows the palette is currently offering.
    ///
    /// Files first, then commands, which is the order a reader means them in: typing a name
    /// wants the file, and the commands are there when the name was not what they meant.
    fn palette_entries(&self, query: &str) -> Vec<PaletteEntry> {
        let needle = query.trim().to_lowercase();
        let mut out: Vec<PaletteEntry> = self
            .tree
            .visible(&needle)
            .into_iter()
            .filter(|r| !r.is_dir)
            .map(|r| PaletteEntry {
                label: r.path.clone(),
                hint: "file".into(),
                command: None,
                action: PaletteAction::Open(r.path.clone()),
            })
            .collect();
        for cmd in commands() {
            if needle.is_empty() || cmd.label.contains(&needle) {
                out.push(PaletteEntry {
                    label: cmd.label.into(),
                    hint: cmd.hint.into(),
                    command: Some(cmd.id),
                    action: cmd.action,
                });
            }
        }
        // Every head the window answers, derived from the declaration rather
        // than listed a second time: a verb the window learns must reach the
        // palette without a second edit. Labels live here as owned strings —
        // the shared table above keeps `&'static str` for its hand-written
        // rows — and the id is the bare head, which is what
        // `run_palette_command` and the hotspot names resolve.
        for head in crate::commands::SUPPORTED {
            let label = format!("/{head}");
            if needle.is_empty() || label.contains(&needle) {
                out.push(PaletteEntry {
                    label,
                    hint: "command".into(),
                    command: Some(head),
                    action: PaletteAction::Verb(head),
                });
            }
        }
        out
    }
    /// The rows the model picker is currently offering.
    ///
    /// Providers first, then the drilled provider's models — the terminal's
    /// two stages, over the terminal's builders (`epsilon_tui::picker`), so
    /// the two front ends cannot disagree about what a provider holds. Live
    /// listings are not merged (the fetch is async; the paint thread is not),
    /// and customs come from the same loader the terminal reads.
    fn model_picker_entries(&self) -> Vec<ModelPickerEntry> {
        let Some(picker) = self.model_picker.as_ref() else {
            return Vec::new();
        };
        let current = self
            .session
            .as_ref()
            .map(|s| s.agent().model_id())
            .unwrap_or_default();
        let customs = epsilon_llm::custom::custom_providers();
        match picker.provider.clone() {
            None => {
                let needle = picker.query.trim().to_lowercase();
                epsilon_tui::picker::build_provider_rows(&current, &customs)
                    .into_iter()
                    .filter(|r| needle.is_empty() || r.id.to_lowercase().contains(&needle))
                    .map(|r| {
                        let mut label = r.id.clone();
                        if r.is_active {
                            label.push_str(" (active)");
                        }
                        let mut meta = format!("{} models", r.model_count);
                        if r.auth_ready {
                            meta.push_str(" · key");
                        }
                        ModelPickerEntry {
                            label,
                            meta,
                            id: r.id,
                            is_provider: true,
                        }
                    })
                    .collect()
            }
            Some(provider) => epsilon_tui::picker::build_models_for_provider(
                &provider,
                &picker.query,
                &current,
                &[],
                &customs,
            )
            .into_iter()
            .map(|r| {
                let mut label = r.label.clone();
                if r.id == current {
                    label.push_str(" (active)");
                }
                ModelPickerEntry {
                    label,
                    meta: if r.auth_ready {
                        "key".to_string()
                    } else {
                        String::new()
                    },
                    id: r.id,
                    is_provider: false,
                }
            })
            .collect(),
        }
    }
    /// Open the model picker on the provider list, closing the palette.
    ///
    /// One overlay at a time: two boxes reading the same arrows would make one
    /// keystroke mean two things, which is the mistake `pane_keys` exists to
    /// prevent.
    pub fn open_model_picker(&mut self) {
        self.palette = None;
        self.shortcuts_open = false;
        self.model_picker = Some(ModelPicker::default());
    }
    /// Close the model picker and hand the keyboard back to the agent pane.
    ///
    /// The same two halves as [`Self::close_palette`], for the same reason: a
    /// picker that closed without the second half leaves the window with no
    /// focused widget at all.
    fn close_model_picker(&mut self) {
        self.model_picker = None;
        self.focus = FocusTarget::Agent;
    }
    /// Read the palette's keys, and act on one.
    ///
    /// **Before the panes, and before the composer**, which is the only reason Enter and
    /// Escape mean anything here. The composer consumes both unconditionally — they are how
    /// it sends and aborts — so a palette that read them later would be reading events that
    /// had already been taken. Which is the same mistake the tab strip made in M250, one
    /// layer down: a key read after the widget that wanted it has already had it.
    fn palette_keys(&mut self, ctx: &egui::Context) {
        if self.keys.take(ctx, "window.palette") {
            // Toggling rather than only opening, because the chord people press twice
            // because it did not seem to work must not need a different one.
            // One overlay at a time: opening here closes the other two.
            if self.palette.is_some() {
                self.close_palette();
            } else {
                self.model_picker = None;
                self.shortcuts_open = false;
                self.palette = Some(Palette::default());
            }
        }
        let Some(query) = self.palette.as_ref().map(|p| p.query.clone()) else {
            return;
        };
        // **Order matters and is unchanged**: the first chord that is present wins, and each
        // is a separate table read rather than one `input_mut` closure — because a binding
        // is resolved by name now, and a name has no fixed order of its own.
        let closed = self.keys.take(ctx, "window.palette.close");
        let mut submitted = false;
        if !closed {
            submitted = self.keys.take(ctx, "window.palette.submit");
        }
        let mut step: i32 = 0;
        if self.keys.take(ctx, "window.palette.next") {
            step += 1;
        } else if self.keys.take(ctx, "window.palette.previous") {
            step -= 1;
        }
        if closed {
            self.close_palette();
            return;
        }
        let entries = self.palette_entries(&query);
        // **A click on a row opens it**, resolved against last frame's rectangles — the same
        // before-draw discipline the tab strip uses, and the same reason. Without this the
        // palette could only be used by typing, which is a worse editor than the file tree
        // it was built to replace: a reader who saw the file in the list and reached for it
        // with the mouse would get a box that does nothing.
        let (any_click, at) = ctx.input(|i| (i.pointer.any_click(), i.pointer.interact_pos()));
        if any_click
            && let Some(at) = at
            && let Some((index, _)) = self
                .palette_rects
                .iter()
                .find(|(_, r)| r.contains(at))
                .copied()
            && let Some(entry) = entries.get(index)
        {
            self.run_palette_action(entry.action.clone(), &query);
            self.close_palette();
            return;
        }
        if let Some(p) = self.palette.as_mut() {
            if step != 0 {
                let last = entries.len().saturating_sub(1) as i32;
                p.selected = (p.selected as i32 + step).clamp(0, last) as usize;
            }
            if submitted {
                p.selected = p.selected.min(entries.len().saturating_sub(1));
            }
        }
        if submitted
            && let Some(entry) = entries.get(self.palette.as_ref().map(|p| p.selected).unwrap_or(0))
        {
            self.run_palette_action(entry.action.clone(), &query);
            self.close_palette();
        }
    }

    /// Read the shortcuts reference's keys: toggle anywhere, close while open.
    ///
    /// After the palette and the model picker — an open overlay owns every
    /// key — and before the panes, for the same reason both do.
    fn shortcuts_keys(&mut self, ctx: &egui::Context) {
        if self.keys.take(ctx, "window.shortcuts") {
            // Toggling rather than only opening, like the palette: the chord
            // people press twice because it did not seem to work must not need
            // a different one. One overlay at a time, same rule as the model
            // picker.
            if self.shortcuts_open {
                self.close_shortcuts();
            } else {
                self.palette = None;
                self.model_picker = None;
                self.shortcuts_open = true;
            }
            return;
        }
        if self.shortcuts_open && self.keys.take(ctx, "window.shortcuts.close") {
            self.close_shortcuts();
        }
    }

    /// Close the shortcuts reference and hand the keyboard back to the agent pane.
    ///
    /// The same two halves as [`Self::close_palette`], for the same reason.
    fn close_shortcuts(&mut self) {
        self.shortcuts_open = false;
        self.focus = FocusTarget::Agent;
    }

    /// Every window binding with its effective chord, in table order.
    ///
    /// Effective, not default: a reader who rebound a chord is shown their own
    /// here, like the status bar and the renamed nodes do. Falls back to the
    /// table default when the id has no binding left at all.
    fn shortcut_rows(&self) -> Vec<(String, String)> {
        epsilon_contracts::keybind::WINDOW
            .iter()
            .map(|(id, defaults)| {
                let chords = self.keys.label(id).unwrap_or_else(|| defaults.join(" "));
                (id.to_string(), chords)
            })
            .collect()
    }
    /// Read the model picker's keys, and act on one.
    ///
    /// After `palette_keys` — an open palette owns every key — and before the
    /// panes, for the same reason the palette reads before the composer.
    /// Escape backs out of the model list to providers; a second one closes.
    fn model_keys(&mut self, ctx: &egui::Context) {
        if self.model_picker.is_none() {
            return;
        }
        let stage2 = self.model_picker.as_ref().and_then(|p| p.provider.clone());
        let closed = self.keys.take(ctx, "window.model.close");
        let mut submitted = false;
        if !closed {
            submitted = self.keys.take(ctx, "window.model.submit");
        }
        let mut step: i32 = 0;
        if self.keys.take(ctx, "window.model.next") {
            step += 1;
        } else if self.keys.take(ctx, "window.model.previous") {
            step -= 1;
        }
        if closed {
            if stage2.is_some() {
                if let Some(p) = self.model_picker.as_mut() {
                    p.provider = None;
                    p.query.clear();
                    p.selected = 0;
                }
                return;
            }
            self.close_model_picker();
            return;
        }
        let entries = self.model_picker_entries();
        // **A click on a row chooses it**, resolved against last frame's
        // rectangles — the same before-draw discipline as the palette, and the
        // same reason: a picker that ignores the mouse is a menu that forgot
        // half its readers.
        let (any_click, at) = ctx.input(|i| (i.pointer.any_click(), i.pointer.interact_pos()));
        if any_click
            && let Some(at) = at
            && let Some((index, _)) = self
                .model_rects
                .iter()
                .find(|(_, r)| r.contains(at))
                .copied()
            && let Some(entry) = entries.get(index)
        {
            self.choose_model_entry(entry.clone());
            return;
        }
        if let Some(p) = self.model_picker.as_mut() {
            if step != 0 {
                let last = entries.len().saturating_sub(1) as i32;
                p.selected = (p.selected as i32 + step).clamp(0, last) as usize;
            }
            if submitted {
                p.selected = p.selected.min(entries.len().saturating_sub(1));
            }
        }
        if submitted
            && let Some(entry) =
                entries.get(self.model_picker.as_ref().map(|p| p.selected).unwrap_or(0))
        {
            self.choose_model_entry(entry.clone());
        }
    }

    /// Act on a chosen model-picker row: drill into providers, apply models.
    ///
    /// Applying runs the same `run_model_argument` `/model <id>` runs — one
    /// act, two doors — so the picker and the composer cannot disagree about
    /// what switching means.
    fn choose_model_entry(&mut self, entry: ModelPickerEntry) {
        if entry.is_provider {
            self.model_picker = Some(ModelPicker {
                query: String::new(),
                selected: 0,
                provider: Some(entry.id),
            });
            return;
        }
        if let crate::commands::Outcome::Ran(said) = self.run_model_argument(Some(entry.id)) {
            self.transcript.push_notice(&said);
        }
        self.close_model_picker();
    }

    /// Close the palette and hand the keyboard back to the agent pane.
    ///
    /// **Both halves, or the window is left holding nothing.** `draw_composer` yields
    /// while the palette is open, so a palette that closed without the second half left
    /// the window with no focused widget at all — the reader types, nothing arrives, and
    /// it looks exactly like a frozen application, which is the state the composer's own
    /// focus rule exists to prevent.
    fn close_palette(&mut self) {
        self.palette = None;
        self.focus = FocusTarget::Agent;
    }
    /// Do what a palette row asked for.
    ///
    /// `query` is the palette's own text, because one command needs it: `goal: create` takes
    /// the objective the reader typed. Every other command ignores it, which is why it is passed
    /// once here rather than to the few verbs that want it.
    fn run_palette_action(&mut self, action: PaletteAction, query: &str) {
        match action {
            PaletteAction::Open(path) => self.open_file(&path),
            PaletteAction::Sidebar(view) => self.sidebar = view,
            PaletteAction::ReloadTree => {
                self.tree = crate::workspace::FileTree::load(&self.opts.workspace)
            }
            PaletteAction::Goal(cmd) => self.run_goal_command(cmd, query),
            PaletteAction::Verb(head) => {
                // The row carries the head, not a copy of what it does: this
                // runs the same dispatch the composer sends through, so the
                // two doors cannot disagree about what a verb means.
                match self.try_command(&format!("/{head}")) {
                    crate::commands::Outcome::Ran(said)
                    | crate::commands::Outcome::Refused(said) => self.transcript.push_notice(&said),
                    crate::commands::Outcome::Send(_) => {}
                }
            }
        }
    }

    /// Run one goal verb, and say what happened.
    ///
    /// **Every failure lands in the transcript.** A verb that does nothing looks exactly like a
    /// verb that worked, and a goal that refused to pause because it is not continuable leaves
    /// the reader believing it is paused — which is the "nothing happened" state this window
    /// keeps producing. The terminal's `/goal` says the same things for the same reasons.
    pub(crate) fn run_goal_command(&mut self, cmd: GoalCommand, query: &str) {
        let Some(session) = self.session.as_ref() else {
            self.transcript
                .push_notice("no agent attached, so there is no goal to change");
            return;
        };
        let agent = Arc::clone(session.agent());
        let objective = query.trim();
        let op = match cmd {
            GoalCommand::Create => {
                if objective.is_empty() {
                    self.transcript.push_notice(
                        "type the objective first, then choose `goal: create` — a goal with no \
                     objective runs to its round cap and gets nowhere",
                    );
                    return;
                }
                epsilon_contracts::goal::GoalOp::Create {
                    objective: objective.to_string(),
                    max_rounds: None,
                }
            }
            GoalCommand::Pause => epsilon_contracts::goal::GoalOp::Pause,
            GoalCommand::Resume => epsilon_contracts::goal::GoalOp::Resume,
            GoalCommand::Clear => epsilon_contracts::goal::GoalOp::Clear,
        };
        let said = format!("goal {}", said_for(cmd));
        match agent.goal_apply(None, op) {
            Ok(Some(view)) => {
                // A created goal is armed; the reader should not have to know that.
                if cmd == GoalCommand::Create || cmd == GoalCommand::Resume {
                    agent.set_goal_armed(true);
                }
                // **The cache, refreshed here.** This is the second of the two places a goal
                // can change; the first is a frame that applied events. Leaving either out
                // does not fail loudly — the band shows the previous phase and the round
                // driver spends rounds on a paused goal — which is why both are named here
                // rather than "refreshed somewhere sensible".
                self.refresh_goal();
                self.transcript.push_notice(&format!(
                    "{said} · {} · round {}/{}",
                    view.goal.status.as_str(),
                    view.goal.rounds_started,
                    view.goal.max_rounds
                ));
            }
            Ok(None) => {
                self.refresh_goal();
                self.transcript
                    .push_notice(&format!("{said} — there is no goal to change"));
            }
            Err(e) => self.transcript.push_notice(&format!("{said} refused: {e}")),
        }
    }
}

/// What a verb reads as past tense, for the notice.
fn said_for(cmd: GoalCommand) -> &'static str {
    match cmd {
        GoalCommand::Create => "created",
        GoalCommand::Pause => "paused",
        GoalCommand::Resume => "resumed",
        GoalCommand::Clear => "cleared",
    }
}

/// Which pane the keyboard is on.
///
/// **Three panes, not every control.** These are the regions a person moves between;
/// inside them the controls are ordinary egui widgets and egui's own focus handles
/// them. This is the level above that, and it is the level WCAG 2.4.3 Focus Order is
/// written about.
///
/// # Why `F6` and not `Tab`
///
/// **`Tab` must keep inserting a tab into the composer.** This window edits code, and
/// indenting with a tab is not a legacy habit here — it is the keystroke a developer
/// already has in their hands and sends dozens of times an hour. Spending it on pane
/// navigation would trade a real editing capability for a navigation nicety, and the
/// only way to recover it would be a mode switch, which is a worse trap than the one
/// it removes.
///
/// `F6` is the platform convention for cycling regions and pane groups — Windows,
/// GNOME and macOS all bind it to something of this shape — and no other key in this
/// window claims it. `Shift+F6` reverses the cycle. **This is a deliberate decision,
/// not an oversight**, and it is recorded here because a reviewer counting `Key::Tab`
/// occurrences will otherwise read it as the accessibility gap it used to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusTarget {
    /// The workspace pane: the file tree, the subagent list, the session list.
    Sidebar,
    /// The editor pane: the open file, or the change under review.
    Editor,
    /// The agent pane: the transcript, and the composer.
    Agent,
}

impl FocusTarget {
    /// Every pane, in the order they sit across the window.
    ///
    /// Left to right, so cycling forward follows the reading order a person already
    /// has of this window rather than an order invented here.
    pub const ALL: [FocusTarget; 3] = [Self::Sidebar, Self::Editor, Self::Agent];

    /// What the ring's region is called in the frame summary.
    pub fn name(self) -> &'static str {
        match self {
            Self::Sidebar => "sidebar",
            Self::Editor => "editor pane",
            Self::Agent => "agent",
        }
    }

    /// Whether this pane was actually drawn, so a ring can land on it.
    fn drawn(self, layout: &IdeLayout) -> bool {
        match self {
            Self::Sidebar => layout.sidebar_visible,
            Self::Editor => layout.center_visible,
            Self::Agent => layout.agent_visible,
        }
    }

    /// The next (`step` `1`) or previous (`step` `-1`) pane that was drawn.
    ///
    /// **Skipping rather than refusing.** A window narrow enough to have folded the
    /// sidebar still has two panes, and a ring drawn around a pane that is not on
    /// screen is the one thing an indicator must never do. When nothing at all was
    /// drawn — a window too short to give the body any height — the focus stands
    /// still, because there is nothing better to point at.
    fn cycled(from: Self, layout: &IdeLayout, step: i32) -> Self {
        let n = Self::ALL.len() as i32;
        let at = Self::ALL
            .iter()
            .position(|t| *t == from)
            .map(|i| i as i32)
            .unwrap_or(0);
        for hop in 1..=n {
            let next = ((at + step * hop) % n + n) % n;
            let candidate = Self::ALL[next as usize];
            if candidate.drawn(layout) {
                return candidate;
            }
        }
        from
    }
}

/// The desktop application.
pub struct IdeApp {
    opts: Options,
    /// The window's colours. pub(crate) so /theme and the frame tests can read what the
    /// window is actually painting; the setters beside it are the way to change it.
    pub(crate) theme: IdeTheme,
    /// What the last launch remembered, and what the reader is changing now.
    ///
    /// **Read once, at construction, and written back when it changes** — see
    /// [`Preferences::load_from`]. It was `Preferences::default()` in every constructor for
    /// the whole life of this struct, which is what made the roadmap's "no pane resizing, no
    /// collapse toggle, no persistence" true: the type existed, was correct, and nothing
    /// could reach it.
    prefs: Preferences,
    /// Where [`Self::persist_prefs`] reads and writes, or [`None`] for the product's own
    /// answer — `epsilon_core::config::global_dir`.
    ///
    /// **A field rather than a direct call, and the test seam is the point.** A test that
    /// presses `Ctrl`+`B` would otherwise write the suite's idea of a layout over the one the
    /// person running it has, in the one directory on this machine that is not disposable.
    /// A constructor that reached into `$HOME` directly would make "the window remembers"
    /// untestable, and untestable is the same as absent.
    prefs_dir: Option<std::path::PathBuf>,
    /// The last sentence [`Self::persist_prefs`] could not write, so it is said once rather
    /// than once per gesture.
    prefs_save_error: Option<String>,
    /// The harness. `None` only in the tests that assert the *absent* state.
    ///
    /// Shared, because a caller that drives a turn from outside the draw path — which is
    /// what a test does, and what an in-app shortcut will — needs a handle that outlives
    /// the borrow the frame takes.
    pub(crate) session: Option<Arc<IdeSession>>,
    /// What the launch said, in order. Shown before anything runs.
    notices: Vec<String>,
    /// The transcript, reduced from the agent's events.
    pub(crate) transcript: Transcript,
    /// The composer's draft.
    draft: String,
    /// What the composer should say when it is empty.
    placeholder: &'static str,
    /// Set when the last submit was refused, so the pane can say why.
    refusal: Option<String>,
    /// Agent events this window lost to lag, as of the last frame (M312).
    ///
    /// **A copy, refreshed in `pump`, because the count is born on the bridge task** and the
    /// status line is drawn on the render thread — the same arrangement `refusal` uses for the
    /// same reason. Reading it live from the session would mean an atomic load per segment, which
    /// is cheaper still and was rejected for a better reason: the status line must be readable
    /// **without a session** (the tests that pin it have none), and a field the window owns is
    /// what lets that be true.
    dropped_events: u64,
    /// The workspace's files, walked once and folded into a tree.
    tree: crate::workspace::FileTree,
    /// The sidebar's filter box.
    tree_query: String,
    /// The file open in the editor, if any.
    open: Option<crate::workspace::OpenFile>,
    /// A change to review in place of the file, when the reader asked for one.
    ///
    /// `None` is the ordinary state. Set by clicking a "changed" row in the transcript,
    /// cleared by opening any file — including the same one — because the tab strip and the
    /// tree are already the reader's two ways back, and a second "close" control would be
    /// chrome answering a question they had not asked.
    review: Option<crate::diff::Change>,
    /// Where each tab was drawn last frame, so a click can be applied **before** the strip
    /// is drawn again.
    ///
    /// egui hands a widget its click result while it is being drawn, so a strip that both
    /// draws and reacts to clicks cannot also redraw itself correctly in the same frame:
    /// the reader got the file but the marker still named the old one. Using last frame's
    /// rectangles puts the action first and the drawing second.
    ///
    /// M248 had this shape and read it **after** the pane that fills it, which is why it
    /// always came back empty. Same data, opposite order — the order is the whole of it.
    tab_rects: Vec<(String, egui::Rect)>,
    /// The command palette, when it is open.
    ///
    /// This window had **two** keys before it — `Enter` and `Escape`, both the composer's.
    /// Everything else was reachable only by clicking, which makes an IDE a browser. The
    /// palette is the one route to every file and every command from the keyboard, which is
    /// what every one of the four reference harnesses has and this one did not.
    palette: Option<Palette>,
    /// The model picker, open while it is `Some`. See [`ModelPicker`].
    model_picker: Option<ModelPicker>,
    /// Whether the shortcuts reference overlay is open (S-101).
    shortcuts_open: bool,
    /// Row rectangles of the last drawn model-picker frame, for clicks.
    model_rects: Vec<(usize, egui::Rect)>,
    /// A gate-checked goal round, from the task that ran the gates back to the draw path.
    ///
    /// The same shape and the same reason as the terminal's: a gate is a shell command, and
    /// the one thing in a goal round allowed to take real time is the one thing that must
    /// not happen on the thread that draws.
    goal_round_tx: tokio::sync::mpsc::UnboundedSender<epsilon_runtime::GoalRound>,
    goal_round_rx: tokio::sync::mpsc::UnboundedReceiver<epsilon_runtime::GoalRound>,
    /// A round is being gate-checked, so do not spend another until it answers.
    goal_round_pending: bool,
    /// Assistant markdown, parsed once per distinct text rather than once per frame.
    ///
    /// Measured at 800 assistant rows: **149.9 ms/frame** parsing the same unchanged text
    /// 800 times, against 15.5 ms for the same rows as plain labels. The terminal solved
    /// the same shape at M198 (`RenderCache`); this is the same rule where the crate had
    /// lost it.
    ///
    /// `RefCell` rather than `&mut` because the transcript draws inside a `ScrollArea`
    /// closure that borrows `self` — a `&mut self` field would be a self-referential
    /// borrow, and a `Mutex` would be a lock taken once per assistant row for a single
    /// thread's private cache. The borrow does not escape: [`Cache::blocks`] hands back an
    /// `Rc`, not a reference into the map.
    markdown: std::cell::RefCell<crate::markup::Cache>,
    /// How tall each transcript row is, measured once and reused — the thing that lets the
    /// frame skip every row nobody can see.
    ///
    /// `RefCell` for the same reason and on the same terms as [`Self::markdown`]: the
    /// transcript draws inside a `ScrollArea` closure that borrows `self`, so the
    /// measurement cannot take `&mut self`. The borrow does not escape.
    row_layout: std::cell::RefCell<RowLayout>,
    /// The session goal, read once per change rather than twice per frame.
    ///
    /// `Agent::goal()` walks `SessionStore::read_all()`, which **deep-clones every stored
    /// entry**, and it does that under the mutex the agent's own append path takes. Measured
    /// against a real store (`tests/frame_cost.rs`):
    ///
    /// | stored messages | `agent.goal()` | ×2, as the window calls it |
    /// |---|---|---|
    /// | 50 | 3.3 ms | 6.6 ms/frame |
    /// | 400 | 30.5 ms | **60.9 ms/frame** |
    /// | 1600 | 102.3 ms | **204.6 ms/frame** |
    ///
    /// Both callers are per-frame and neither can avoid being: the band has to say the phase
    /// and the round budget, and the round driver has to decide whether a round is allowed.
    /// The cost was never the problem — reading it *while holding the store lock* is, because
    /// that is the lock a running agent wants to append through.
    ///
    /// The terminal solved this shape at M198 and the shape is the same one: cache it and
    /// refresh at the two places a goal can change.
    pub(crate) goal: Option<epsilon_contracts::goal::GoalView>,
    /// How full the context is, what the session has cost, and how many tokens moved —
    /// read once per change rather than once per frame.
    ///
    /// **The same rule as [`Self::goal`], for the same reason.** These are three reads that
    /// all walk state the loop owns: `estimated_context_tokens` anchors over the whole
    /// transcript, `session_usage_by_model` clones a map, and this window already has the
    /// measurements for what that class of read costs per frame — `115 ms` for the markdown
    /// cache at 800 assistant rows (`tests/frame_cost.rs`) and `204 ms` for the goal read at
    /// 1600 stored messages (the table on [`Self::goal`]). None of them is allowed on the
    /// draw path.
    ///
    /// Seeded in [`IdeApp::new`] rather than lazily in `pump`, because a frame that drained
    /// nothing has learned nothing — and a window that opened onto an existing session and
    /// showed no figures until the first event arrived would be asserting the defect this
    /// field was added to fix.
    pub(crate) metrics: Option<epsilon_tui::ui::metrics::Metrics>,
    /// What a running `/compact` reported, when it reports.
    ///
    /// A channel rather than an event because `compact_now`'s success and failure are
    /// otherwise indistinguishable at the window: the reducer folds neither
    /// `CompactionStarted` nor `CompactionFinished`, so a compaction that failed would leave
    /// a reader who pressed `/compact` looking at an unchanged transcript.
    compact_tx: tokio::sync::mpsc::UnboundedSender<String>,
    compact_rx: tokio::sync::mpsc::UnboundedReceiver<String>,
    /// Where each palette row was drawn last frame, with its index, so a click can be
    /// applied **before** the rows are drawn again.
    ///
    /// The same discipline as [`IdeApp::tab_rects`], and for the same reason: egui only
    /// knows a widget was pressed once it has been drawn, so a row list that reacts to its
    /// own answer opens the file one frame after the highlight moves to it.
    palette_rects: Vec<(usize, egui::Rect)>,
    /// Open files, oldest first; the last is what the editor shows.
    tabs: Vec<String>,
    /// Which view the sidebar is showing, chosen from the activity bar.
    sidebar: SidebarView,
    /// Which pane the keyboard is on, and which one wears the ring.
    ///
    /// **Starts on the agent pane**, because that is where this window's only text
    /// input is and where [`Self::draw_composer`] already puts the keyboard when
    /// nothing else has claimed it — so the first frame behaves exactly as it did
    /// before this field existed, and every existing test still describes the
    /// product it was written for.
    focus: FocusTarget,
    /// The workspace-relative path of the file-tree row the keyboard is on.
    ///
    /// **A path, not an index.** The visible list is filtered and collapsible, so an
    /// index means something different on every frame — which is the reason
    /// [`crate::workspace::FileTree::toggle`] matches on the path too.
    tree_cursor: Option<String>,
    /// The path of the **sessions** row the keyboard is on.
    ///
    /// **This field exists because the sessions list was a keyboard trap.** The arrows and
    /// `Enter` were gated on `sidebar == SidebarView::Files`, so `F6` into a Sessions
    /// sidebar produced a focus ring over a pane where no key did anything — a pane a
    /// keyboard reader can enter and not leave, which is WCAG 2.1.2 (No Keyboard Trap).
    /// The list was clickable and nothing else, and every assertion about it in the suite
    /// was a pointer.
    ///
    /// A path, for the same reason [`Self::tree_cursor`] is one: the list is re-read from
    /// disk on every frame and a session can be deleted between two of them, so an index
    /// would name a different row than the one it meant.
    session_cursor: Option<std::path::PathBuf>,
    /// Geometry of the most recent frame, kept for hit-testing and for tests.
    layout: IdeLayout,
    /// The window's keyboard, read from the shared table
    /// (`epsilon_contracts::keybind::WINDOW`) and overridable per key.
    ///
    /// **This field exists because the window had no keyboard it could name.** Every chord
    /// was a literal `consume_key` and every on-screen hint was a hand-written string, so
    /// `keybindings.json` could not touch a single one of them — while the terminal's could.
    /// `AGENTS.md` §2.4 says one table; this is the half that was missing.
    keys: crate::keys::KeyBindings,
    /// What a close request would interrupt, once one has been caught.
    ///
    /// **`AGENTS.md` §6: never lose a user's work.** Closing the window used to end the
    /// process, so a turn in flight was killed and a reader who had not pressed `/clear`
    /// had no way to know what a durable session file does and does not cover. DSH asks the
    /// host what a quit would interrupt and treats an unknown answer as "work is running";
    /// this is the same rule, and `None` here means **no request is pending**, so the
    /// `Some` arm is the one that has to be written carefully.
    ///
    /// **Filled once, at the moment of the request**, and drawn from that snapshot rather
    /// than re-measured while the dialog is up — a dialog that changed its own warning while
    /// the reader was deciding would be worse than no dialog.
    quit_confirm: Option<QuitAsk>,
}

/// What closing the window right now would interrupt.
///
/// **An unknown answer is "work is running".** The reference does the same thing with its
/// inspection timeout, and the reason is the one in `AGENTS.md` §6: a guard that assumes the
/// best case is not a guard.
#[derive(Debug, Clone, PartialEq, Eq)]
struct QuitAsk {
    /// A turn is in flight. The agent appends durably as it goes, so this is not data loss
    /// on its own — it is a turn that stops where it is.
    running: bool,
    /// Permission asks are parked and unanswered, so **nothing is running** while the
    /// window is blocked. Quitting here discards the gate, and the tool never ran.
    pending_approvals: usize,
    /// A session file with something in it, and whether it has been `/clear`ed in this
    /// window. The transcript on screen is the only record of a conversation nobody exported.
    conversation: bool,
    /// No session at all — the window opened onto nothing, so there is nothing to lose and
    /// the dialog would be pure friction. Recorded so the "ask anyway?" path is a decision
    /// rather than a reflex.
    empty: bool,
}

impl QuitAsk {
    /// One sentence, naming what is actually at stake.
    ///
    /// **Specific beats alarming.** "Quit?" for a turn that is one tool call from finishing
    /// teaches a reader to click through it, and then it is not a guard for the case that
    /// matters.
    fn sentence(&self) -> String {
        if self.empty {
            return "Nothing is in flight. Close the window?".to_string();
        }
        let mut parts: Vec<String> = Vec::new();
        if self.running {
            parts.push("a turn is in flight and will stop where it is".to_string());
        }
        if self.pending_approvals > 0 {
            parts.push(format!(
                "{} permission ask{} waiting, and the tool never ran",
                self.pending_approvals,
                if self.pending_approvals == 1 {
                    " is"
                } else {
                    "s are"
                }
            ));
        }
        if self.conversation {
            parts.push(
                "this conversation is only on screen until the session is reopened".to_string(),
            );
        }
        if parts.is_empty() {
            return "Nothing is in flight. Close the window?".to_string();
        }
        format!("Closing now: {}.", parts.join("; "))
    }
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
///
/// # `font_size` is the size it will be **drawn** at, not a nominal one
///
/// [`AVG_ADVANCE`] is a dimensionless ratio, so it is **not** scaled: the caller passes
/// `ROW_FONT * scale`, not `ROW_FONT * scale * AVG_ADVANCE`. Scaling the ratio as well
/// would divide the budget by the scale twice, and at 200% text every painted row would be
/// truncated at half the width the reader can actually see — text lost off the right edge,
/// which is the exact failure the budget exists to prevent.
pub fn fit(text: &str, points: f32, font_size: f32) -> String {
    let budget = (points / (font_size * AVG_ADVANCE)).floor().max(4.0) as usize;
    if text.chars().count() <= budget {
        return text.to_string();
    }
    let kept: String = text.chars().take(budget.saturating_sub(1)).collect();
    format!("{kept}…")
}

/// Paint one line of text on a raw rectangle, and record the size it was painted at.
///
/// **The only way this window paints a string that is not a widget.** Every `egui` widget
/// takes its size from [`egui::Style`], so it needs no size argument; every `Painter::text`
/// call takes a number, and a number is exactly the thing AGENTS.md §8's "text scaling"
/// clause is about. Routing all of them through one function means there is one place a
/// painter text size is decided and one place it is recorded — which is what lets a test
/// ask "what size was the title bar painted at" rather than "did the title bar paint".
///
/// The size arrives already scaled, because [`text_scale`] is the only answer this crate
/// gives to "how big is text here" and multiplying it in here would let a call site
/// quietly forget.
///
/// `pub(crate)` because [`crate::workspace`] paints its three panes the same way, and a
/// second way to paint a string here would be a second place to forget the scale.
///
/// Eight arguments is eight arguments because every one of them is something the caller
/// already has and a bundle would only hide: the position, the alignment, the string, the
/// font, the colour and the region are six separate facts about one piece of text, and a
/// struct to carry them would be a second type to keep in step with this one.
#[allow(clippy::too_many_arguments)]
pub(crate) fn paint_text(
    painter: &egui::Painter,
    summary: &mut FrameSummary,
    region: &str,
    pos: egui::Pos2,
    align: egui::Align2,
    text: impl ToString,
    font: egui::FontId,
    color: egui::Color32,
) {
    // Read before the move: `FontId` is not `Copy`, and the size is the whole record.
    let size = font.size;
    painter.text(pos, align, text, font, color);
    summary.text_sizes.push((region.to_string(), size));
}

/// The scale every piece of text in this window is drawn and measured at.
///
/// # Where it comes from, and why this one
///
/// [`egui::Context::zoom_factor`], read **once per frame** in [`IdeApp::paint`] and passed
/// down to every pane. Two things make it the right number.
///
/// It is the only scale egui has. `Style` carries no zoom of its own — `egui-0.36.2`'s
/// `style.rs` has no `zoom` field at all — and `Context::begin_pass` implements zoom by
/// rescaling `screen_rect` and multiplying `pixels_per_point`
/// (`egui-0.36.2/src/context.rs:438-455`). So this crate's painter-drawn text does **not**
/// respond to it on its own, and had to be told.
///
/// It is also what a reader changes. `egui_gui_zoom` is the standard egui binding for
/// `Ctrl`+`+` / `Ctrl`+`-` (`egui-0.36.2/src/gui_zoom.rs:51-66`) and clamps to 0.5–5.0
/// there, which is why this function does not clamp: a second clamp is a second rule.
///
/// # What it scales, and what it does not
///
/// Every painter text size, every [`fit`] budget, the transcript's body size
/// ([`crate::markup::BODY_SIZE`]) and the fixed rectangles that exist only to hold text.
///
/// **It does scale `egui::Style`, and it has to.** The brief this shipped from recorded
/// that the widget half already followed `zoom_factor`, and **that is false for egui
/// 0.36.2**: `Style` has no zoom field, and `Context::begin_pass` implements zoom by
/// rescaling `screen_rect` and multiplying `pixels_per_point`
/// (`egui-0.36.2/src/context.rs:438-455`). Nothing multiplies a `FontId` by the zoom. So
/// scaling only the painter half would have grown the window's headings and its composer
/// while every widget-drawn row stayed the size it was — the half-measure this change
/// exists to end, arrived at by the same road. [`Self::scale_widget_style`] is the other
/// half.
///
/// It is applied to the **root `Ui`**, not to the `Context`, because `UiBuilder` inherits
/// its parent's style (`egui-0.36.2/src/ui_builder.rs`, `pub style: Option<Arc<Style>>`)
/// and a root `Ui` is built before `paint` runs — `ctx.style_mut` would leave this frame's
/// own `Ui` describing the old scale and the next frame's describing the new one.
///
/// Not clamped and not defaulted: a caller that passes a nonsense scale gets nonsense
/// sizes, which is honest.
pub fn text_scale(ctx: &egui::Context) -> f32 {
    ctx.zoom_factor()
}

/// `style` with every text size and every vertical measure in it multiplied by `scale`.
///
/// **The widget half of one scale**, and the reason it exists is that egui 0.36 has no
/// opinion about zoom: `Style` carries no zoom field, `Context::begin_pass` only rescales
/// `screen_rect` and `pixels_per_point` (`egui-0.36.2/src/context.rs:438-455`), and
/// nothing between them multiplies a `FontId`. Every widget this window draws — the
/// transcript's non-prose rows, `colored_label`, `selectable_label`, `TextEdit`, `Button` —
/// takes its size from [`egui::Style::text_styles`], so scaling that map is what makes them
/// answer to the same number the painter sites do.
///
/// The spacings go with the text, or the two halves disagree about how much room a row
/// takes: `interact_size.y` is the height of a button and of a `TextEdit`, `item_spacing.y`
/// is the gap between two rows the reader is reading, and `extra_text_line_spacing` is
/// additional leading between wrapped lines. Scaling the fonts without these produces a
/// window whose text is larger and whose rows are the same height, which is the same defect
/// as scaling nothing.
///
/// **`scale == 1.0` returns `style` untouched**, which is why every existing test — all of
/// which run at egui's default zoom — describes the window exactly as it was.
///
/// Only the *vertical* measures move. A window at 200% has the same width in points as it
/// did, so widening its buttons and its gaps would spend the reader's horizontal room on
/// nothing.
///
/// Applied to the **`Context`'s** style rather than to a `Ui`'s, and that is not a
/// preference: [`pane_ui`] builds every pane with [`egui::Ui::new`], and `Ui::new` falls back
/// to `ctx.global_style()` when `UiBuilder::style` is `None`
/// (`egui-0.36.2/src/ui.rs:353`). A pane therefore never reads the `Ui` it was created
/// from — it reads the Context. Setting a scaled style on the root `Ui` would reach nothing.
fn scale_widget_style(style: &mut egui::Style, scale: f32) {
    if scale == 1.0 {
        return;
    }
    for font in style.text_styles.values_mut() {
        font.size *= scale;
    }
    style.spacing.interact_size.y *= scale;
    style.spacing.item_spacing.y *= scale;
    style.spacing.extra_text_line_spacing *= scale;
}

/// Height of the editor's tab strip, before [`text_scale`].
const TAB_H: f32 = 28.0;

/// Largest width a tab may take before [`text_scale`]; the strip shares what is left
/// between its tabs.
const TAB_W: f32 = 96.0;

/// Font a tab's label is drawn at, before [`text_scale`], and the one the label's
/// [`fit`] budget assumes.
const TAB_FONT: f32 = 12.0;

/// Average glyph advance as a fraction of the font size, for proportional text.
///
/// **A ratio, so it is not scaled** — see [`fit`].
const AVG_ADVANCE: f32 = 0.55;

/// The font the sidebar's rows are drawn at, before [`text_scale`], and the one
/// [`fit`] assumes.
const ROW_FONT: f32 = 13.0;

/// How tall a pane's painted header row occupies, before [`text_scale`].
///
/// **Every pane that paints its header with the painter must move its `Ui` cursor down by
/// this (M312).** A painter-drawn header does not advance the cursor, so the first widget the
/// pane adds lands at the pane's **top** — on top of the header. Two panes did this and the
/// rendered frames showed the empty state printed *above* the pane's own title, clipped by the
/// pane's edge; the agent pane's transcript had the same fault, 42 pixels of it.
///
/// One constant, because the rule is one rule and three call sites are where it goes wrong.
const PANE_HEADER_H: f32 = 30.0;

/// Record **how far below its own top** a pane's content begins (M312).
///
/// One entry per frame, so the header/content relationship can be *asserted* rather than
/// assumed: `FrameSummary::rows` records the strings, and a line printed over the pane's title
/// records exactly the same strings — which is why this defect survived every test in the file
/// and was found by looking at a rendered frame.
///
/// **The recorded rect runs from the pane's top to the cursor, so its `height` *is* the
/// measurement.** Two earlier versions of this record measured the wrong thing and **both
/// controls left the test green**: `ui.min_rect().top()` is the `Ui`'s own top — the pane's top,
/// always clear of the band — and comparing the cursor against a constant ignored where the
/// pane's `Ui` actually begins. A rect that starts where the pane starts cannot be misread.
fn record_content_offset(ui: &egui::Ui, summary: &mut FrameSummary, rect: egui::Rect) {
    let cursor = ui.min_rect().max.y.max(rect.top());
    summary.hotspots.push((
        "pane:first-content".to_string(),
        egui::Rect::from_min_max(
            egui::pos2(rect.left(), rect.top()),
            egui::pos2(rect.left(), cursor),
        ),
    ));
}

impl IdeApp {
    /// Build the application around an already-composed session.
    pub fn new(
        opts: Options,
        session: IdeSession,
        notices: Vec<String>,
        ctx: &egui::Context,
    ) -> Self {
        // Composition's own diagnostics, after the launch's. **Appended, not merged:** the
        // launch notices come from `resolve_launch`, which ran *before* the agent was
        // composed, so this order is the order the launch happened in. Without this the
        // window showed the first half and dropped the second, and a skill that failed to
        // load or a session that migrated itself reached nobody — a window has no stderr.
        let mut notices = notices;
        notices.extend(session.notices().iter().cloned());
        // Start listening before the first frame: an event that arrives between the
        // window opening and the bridge attaching would be dropped, and the first thing a
        // user does is type.
        session.attach_bridge(ctx);
        let theme = IdeTheme::resolve(opts.theme);
        // Loaded before opts moves, so the tree is built from the workspace the caller
        // named rather than from a borrow of a moved value.
        let tree = crate::workspace::FileTree::load(&opts.workspace);
        let transcript = crate::session::Transcript::new(&opts.workspace);
        // **The remembered preferences, not the defaults**, for the seed layout as well as
        // for the struct below: a window whose first frame lays out at the default width and
        // whose second frame lays out at the remembered one spends a frame showing a layout
        // the reader did not ask for, and a test reading `layout()` before the first paint
        // would read a layout the product never draws.
        let prefs = Preferences::load_from(&opts.workspace, None);
        let layout = IdeLayout::new(
            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 820.0)),
            prefs,
        );
        // **One channel, bound once.** These fields used to read `unbounded_channel().0` and
        // `unbounded_channel().1` — which is *two different channels*, the `.0` of one call
        // and the `.1` of another. The goal round's answer went to a channel nothing read
        // from, so `take_goal_round` could only ever return `None` and **an unattended goal
        // in the window never continued itself**, which is the whole feature.
        //
        // Nothing caught it because the tests drive `start_goal_round_for_test`, which
        // bypasses the channel by design: the one test that could have shown it was the one
        // built not to. `a_goal_round_spawned_by_the_driver_really_arrives` closes that.
        // **One channel, bound once.** These fields used to read `unbounded_channel().0` and
        // `unbounded_channel().1` — which is *two different channels*, the `.0` of one call
        // and the `.1` of another. The goal round's answer went to a channel nothing read
        // from, so `take_goal_round` could only ever return `None` and **an unattended goal
        // in the window never continued itself**, which is the whole feature.
        //
        // Nothing caught it because the tests drive `start_goal_round_for_test`, which
        // bypasses the channel by design: the one test that could have shown it was the one
        // built not to. `a_goal_round_spawned_by_the_driver_really_arrives` closes that, and
        // its first version passed against the broken channel — it waited on the round
        // *count*, which `begin_goal_round` moves before it sends anything.
        let (goal_round_tx, goal_round_rx) = tokio::sync::mpsc::unbounded_channel();
        let (compact_tx, compact_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = Self {
            opts,
            theme,
            // **What the last launch left behind**, so a reader who dragged the sidebar wide
            // gets it wide again. Defaults when there is no file, when it will not parse, or
            // when there is no home directory — see `Preferences::load_from`, which is built
            // to fail that way rather than to keep the window shut.
            prefs,
            prefs_dir: None,
            prefs_save_error: None,
            session: Some(Arc::new(session)),
            notices,
            transcript,
            draft: String::new(),
            placeholder: "ask anything  ·  enter sends  ·  shift+enter for a new line",
            refusal: None,
            dropped_events: 0,
            tree,
            tree_query: String::new(),
            open: None,
            review: None,
            tab_rects: Vec::new(),
            palette: None,
            goal_round_tx,
            goal_round_rx,
            goal_round_pending: false,
            markdown: std::cell::RefCell::new(crate::markup::Cache::default()),
            row_layout: std::cell::RefCell::new(RowLayout::default()),
            goal: None,
            metrics: None,
            compact_tx,
            compact_rx,
            palette_rects: Vec::new(),
            model_picker: None,
            model_rects: Vec::new(),
            shortcuts_open: false,
            tabs: Vec::new(),
            sidebar: SidebarView::Files,
            focus: FocusTarget::Agent,
            tree_cursor: None,
            session_cursor: None,
            layout,
            keys: window_keys(),
            quit_confirm: None,
        };
        // **Once, here.** A window that opens onto an existing session has to show what
        // that session has cost without waiting for an event to arrive first: a `pump`
        // that refreshed only on `drain > 0` leaves the figures blank for exactly the
        // frames a reader looks at, and a blank gauge is read as "nothing has happened".
        app.refresh_metrics();
        app
    }
    /// An application with **no** agent, for the test that asserts the absent state.
    ///
    /// Exists because "no agent attached" is a state a user can genuinely reach and a
    /// frame that draws a plausible model for it would be a lie — the M233 defect. The
    /// only way to test that is to be able to build the frame.
    pub fn without_agent(opts: Options) -> Self {
        let tree = crate::workspace::FileTree::load(&opts.workspace);
        let transcript = crate::session::Transcript::new(&opts.workspace);
        // Bound once, as in `new` — see there for why this used to be two channels.
        let (goal_round_tx, goal_round_rx) = tokio::sync::mpsc::unbounded_channel();
        let (compact_tx, compact_rx) = tokio::sync::mpsc::unbounded_channel();
        // The same load as `new`, for the same reason — see the field below.
        let prefs = Preferences::load_from(&opts.workspace, None);
        Self {
            opts,
            theme: IdeTheme::resolve(None),
            // **The same load as `new`.** This constructor is a second way to build the
            // window, and a window that came up with a default layout while its twin
            // remembered the reader's would be an inconsistency with nothing to explain it.
            prefs,
            prefs_dir: None,
            prefs_save_error: None,
            session: None,
            notices: vec!["not connected to an agent".into()],
            transcript,
            draft: String::new(),
            placeholder: "no agent attached",
            refusal: None,
            dropped_events: 0,
            tree,
            tree_query: String::new(),
            open: None,
            review: None,
            tab_rects: Vec::new(),
            palette: None,
            goal_round_tx,
            goal_round_rx,
            goal_round_pending: false,
            markdown: std::cell::RefCell::new(crate::markup::Cache::default()),
            row_layout: std::cell::RefCell::new(RowLayout::default()),
            goal: None,
            metrics: None,
            compact_tx,
            compact_rx,
            palette_rects: Vec::new(),
            model_picker: None,
            model_rects: Vec::new(),
            shortcuts_open: false,
            tabs: Vec::new(),
            sidebar: SidebarView::Files,
            focus: FocusTarget::Agent,
            tree_cursor: None,
            session_cursor: None,
            keys: window_keys(),
            quit_confirm: None,
            layout: IdeLayout::new(
                egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 820.0)),
                prefs,
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

    /// The two keys that change the window's layout: fold the sidebar, resize it.
    ///
    /// **Both are read on every frame regardless of which pane has the keyboard**, because
    /// neither acts *on* a pane. `F6` picks which pane the arrows walk; if the arrows also
    /// moved a pane's width, one key would mean two things depending on where focus
    /// happened to be, and the reader would have to learn a mode to use a window.
    ///
    /// # Why `←` and `→` are free, and why they are here at all
    ///
    /// Nothing in this window binds them — checked, not assumed: the tree and the palette
    /// take `ArrowUp`/`ArrowDown`, and `ArrowLeft`/`ArrowRight` appear nowhere. They are
    /// bound because [`crate::layout::SIDEBAR_HANDLE_W`] is 6 points wide and WCAG 2.2
    /// SC 2.5.8 asks 24, so the drag handle is under the floor by a factor of four and
    /// something has to be over it. Arrow keys are how a splitter is resized by every
    /// screen-reader-driven UI toolkit, and they cost one line.
    ///
    /// # Why they do nothing when the sidebar has folded
    ///
    /// Because there is no sidebar to resize, and because the status bar stops printing
    /// them when `layout.sidebar_visible` is false. A key that is printed and does nothing
    /// is the `AGENTS.md` §2.7 failure in its smallest form.
    fn layout_keys(&mut self, ctx: &egui::Context) {
        if self.keys.take(ctx, "window.sidebar.toggle") {
            self.prefs.sidebar_visible = !self.prefs.sidebar_visible;
            self.persist_prefs();
        }
        // **The arrows are named bindings, matched exactly.** `KeyBindings::take` compares
        // modifiers for equality, so a `Shift+Left` press is not a resize — which is what
        // `consume_key`'s logical matching would have made of it, and the reason the two
        // are named separately rather than read as one.
        let narrower = self.keys.take(ctx, "window.sidebar.narrower");
        let wider = self.keys.take(ctx, "window.sidebar.wider");
        if (narrower || wider) && self.layout.sidebar_visible {
            let step = if wider { SIDEBAR_STEP } else { -SIDEBAR_STEP };
            self.prefs.sidebar_width =
                (self.prefs.sidebar_width + step).clamp(SIDEBAR_MIN, SIDEBAR_MAX);
            self.persist_prefs();
        }
    }

    /// Write the layout preferences out, and say so when they could not be written.
    ///
    /// **The preference has already changed by the time this runs, and it stays changed.**
    /// A window that refused to resize because the config directory was read-only would be a
    /// worse product than one that forgets: the reader asked for a layout and got the
    /// window, which is the part they cannot do without. Only the *remembering* is
    /// best-effort, exactly as [`epsilon_core::config::save_model_id_in`] is, and it says
    /// so rather than failing quietly — `AGENTS.md` §2.7 and §8.
    ///
    /// **Called at the end of a gesture, not on every frame of one.** A drag changes
    /// `sidebar_width` sixty times a second and writing a file each time would put disk I/O
    /// on the paint thread for no reader's benefit (`AGENTS.md` §8, "no blocking of the UI
    /// thread"). The three callers are a drag that stopped, a fold, and an arrow press.
    ///
    /// **The repeated-failure guard** is because a read-only directory fails on *every*
    /// gesture, and a notice per gesture is a transcript nobody can read — the same noise
    /// `session.rs`'s repaint coalescing exists to avoid.
    fn persist_prefs(&mut self) {
        match self
            .prefs
            .save_to(self.workspace(), self.prefs_dir.as_deref())
        {
            Ok(_) => self.prefs_save_error = None,
            Err(e) => {
                if self.prefs_save_error.as_deref() != Some(e.as_str()) {
                    self.notices.push(format!("window layout not saved: {e}"));
                    self.prefs_save_error = Some(e);
                }
            }
        }
    }

    /// Read the keys that move between panes and move inside the focused one.
    ///
    /// **After [`Self::palette_keys`] and before any pane is drawn.** Both halves of
    /// that sentence are load-bearing, and the second is the same lesson the palette's
    /// own doc records: `draw_composer` consumes `Enter` and `Escape`
    /// unconditionally, so a pane handler that read them after it would be reading keys
    /// that had already been taken.
    ///
    /// # Why it returns while the palette is open
    ///
    /// Not politeness — arithmetic. The palette's `ArrowDown` moves *its* selection,
    /// the composer's `Enter` sends *its* draft, and a handler in the middle would
    /// make one keystroke mean two things at once. The palette owns its keys while it
    /// is up; `AGENTS.md` §2.7 would call the alternative a silent no-op in the
    /// direction that matters.
    fn pane_keys(&mut self, ctx: &egui::Context) {
        if self.palette.is_some() || self.model_picker.is_some() || self.shortcuts_open {
            return;
        }
        // **The window's two layout keys, before every pane key.** They are read here
        // rather than inside a pane handler because neither belongs to a pane: `Ctrl`+`B`
        // moves a pane out of the way and `←`/`→` changes its width, and both work from
        // wherever the keyboard currently is. Below the palette guard, because the palette
        // owns every key while it is open — which is also why the status bar stops
        // advertising them while it is.
        self.layout_keys(ctx);
        // **Session chords, beside the layout keys.** They act on the session,
        // not on a pane, so they are read here rather than inside a pane
        // handler — the same reason as the two lines above. Below the palette
        // guard, because the palette owns every key while it is open.
        if self.keys.take(ctx, "window.session.new") {
            let outcome = self.run_new_session();
            self.notice_outcome(outcome);
        }
        if self.keys.take(ctx, "window.session.fork") {
            let outcome = self.run_fork_session("fork", "forked");
            self.notice_outcome(outcome);
        }
        // **`Shift` first, and that is no longer a workaround.** It used to be needed
        // because `InputState::consume_key` matches modifiers *logically* — extra shift and
        // alt are ignored — so a `NONE` match also swallowed `Shift+F6` and cycled forward
        // on a reverse press. `KeyBindings::take` compares modifiers for **equality**, so the
        // two ids are genuinely distinct and either order would do. The order is kept because
        // it costs nothing and because a table is free to change tomorrow.
        let back = self.keys.take(ctx, "window.focus.previous");
        let forward = self.keys.take(ctx, "window.focus.next");
        if back || forward {
            let layout = self.layout;
            self.focus = FocusTarget::cycled(self.focus, &layout, if back { -1 } else { 1 });
            // Whatever widget held the keyboard gives it up, so the pane that just took
            // it is not sharing every keystroke with a `TextEdit`. The composer's own
            // rule below takes focus straight back when the agent pane is the one that
            // won, so this is a hand-off rather than a gap.
            ctx.memory_mut(|m| {
                if let Some(id) = m.focused() {
                    m.surrender_focus(id);
                }
            });
            return;
        }
        // The agent pane's keys are the composer's, and it already has them. Reading
        // `Enter` here as well is how one press ends up sending a message *and*
        // opening a tree row.
        if self.focus == FocusTarget::Agent {
            return;
        }
        // The arrows and `Enter` belong to the **list on screen**, and the sidebar has
        // three views. A cursor walking rows the reader cannot see is a keyboard that
        // appears busy — so the gate is *which list*, and each list answers with its own
        // rows. The Sessions view is here because excluding it made a keyboard trap:
        // `F6` into that pane produced a focus ring over a list where no key did
        // anything, which is WCAG 2.1.2.
        let sidebar_has_keyboard =
            self.focus == FocusTarget::Sidebar && self.layout.sidebar_visible;
        let tree_is_here = sidebar_has_keyboard && self.sidebar == SidebarView::Files;
        let sessions_are_here = sidebar_has_keyboard && self.sidebar == SidebarView::Sessions;
        let mut step = 0i32;
        let mut opened = false;
        // `Escape` is not the tree's: it is the way **out** of a pane whatever that
        // pane is showing, and a region the reader cannot leave is the keyboard trap
        // WCAG 2.1.2 forbids.
        let home = self.keys.take(ctx, "window.pane.leave");
        if tree_is_here {
            if self.keys.take(ctx, "window.tree.next") {
                step = 1;
            } else if self.keys.take(ctx, "window.tree.previous") {
                step = -1;
            }
            opened = self.keys.take(ctx, "window.tree.open");
        }
        if step != 0 {
            self.move_tree_cursor(step);
        }
        if opened {
            self.open_tree_cursor();
        }
        if sessions_are_here {
            let mut step = 0i32;
            if self.keys.take(ctx, "window.tree.next") {
                step = 1;
            } else if self.keys.take(ctx, "window.tree.previous") {
                step = -1;
            }
            let resume = self.keys.take(ctx, "window.tree.open");
            if step != 0 {
                self.move_session_cursor(step);
            }
            if resume {
                self.open_session_cursor();
            }
        }
        if home {
            // **Consumed here rather than left for the composer.** With nothing else
            // holding the keyboard the composer's own `Escape` clears the draft, or
            // stops a run, and a reader who pressed it in the *sidebar* has not asked
            // to lose their words — `AGENTS.md` §6 would rather a stray key did
            // nothing. The request below is the same mechanism and the same widget id
            // `draw_composer` uses, so this is one focus mechanism with two callers,
            // not two that can disagree.
            self.focus = FocusTarget::Agent;
            ctx.memory_mut(|m| m.request_focus(composer_id()));
        }
    }

    /// Move the tree cursor by `step` rows through the rows currently visible.
    ///
    /// **Indexed by finding the path again**, because the visible list is filtered and
    /// collapsed: the index the cursor was at on the previous frame is a number about a
    /// list that may since have changed. A cursor that survived a filter change by
    /// staying at the same number would land on an unrelated file, which is the
    /// failure mode [`crate::workspace::FileTree::toggle` documents.
    fn move_tree_cursor(&mut self, step: i32) {
        let visible = self.tree.visible(&self.tree_query);
        if visible.is_empty() {
            self.tree_cursor = None;
            return;
        }
        let last = visible.len() as i32 - 1;
        let here = self
            .tree_cursor
            .as_deref()
            .and_then(|p| visible.iter().position(|r| r.path == p))
            .map(|i| i as i32);
        let next = match here {
            // Arriving at a list from outside takes the first row going down and the
            // last going up, which is what every list a reader already uses does.
            None if step > 0 => 0,
            None => last,
            Some(i) => (i + step).clamp(0, last),
        };
        self.tree_cursor = Some(visible[next as usize].path.clone());
    }

    /// Move the sessions cursor by `step` rows.
    ///
    /// **The same rule as [`Self::move_tree_cursor`], and it is copied rather than
    /// shared on purpose.** The tree's version takes a `&str` path out of a *filtered,
    /// collapsible* list and has to re-find it by value; this one reads a flat list
    /// straight off disk. The part that matters is the same in both: **arriving takes the
    /// first row going down and the last going up**, which is what every list a reader
    /// already uses does, and it is the part that is easy to get wrong by only handling
    /// the "already on a row" case.
    fn move_session_cursor(&mut self, step: i32) {
        let Some(session) = self.session.as_ref() else {
            self.session_cursor = None;
            return;
        };
        let sessions = session.sessions(&self.opts.workspace);
        if sessions.is_empty() {
            self.session_cursor = None;
            return;
        }
        let last = sessions.len() as i32 - 1;
        let here = self
            .session_cursor
            .as_deref()
            .and_then(|p| sessions.iter().position(|s| s.path == p))
            .map(|i| i as i32);
        let next = match here {
            None if step > 0 => 0,
            None => last,
            Some(i) => (i + step).clamp(0, last),
        };
        self.session_cursor = Some(sessions[next as usize].path.clone());
    }

    /// Resume the session under the cursor — the two lines the click path runs.
    ///
    /// **The same discipline as [`Self::open_tree_cursor`]: the keyboard must not be a
    /// second editor.** It calls the same [`Self::switch_session`] the pointer does, so a
    /// refusal reads the same sentence and lands in the same two places.
    fn open_session_cursor(&mut self) {
        let Some(path) = self.session_cursor.clone() else {
            return;
        };
        let Some(session) = self.session.as_ref() else {
            return;
        };
        // A cursor left behind by a session deleted since is not a row.
        if !session
            .sessions(&self.opts.workspace)
            .iter()
            .any(|s| s.path == path)
        {
            return;
        }
        // **Resuming the session this window is already in is skipped, because the click
        // path skips it** (`!is_current` in the pane). The comment there says why, and
        // "clicking the session you are already in is neither an error nor a spinner" is
        // true of the keyboard too — reopening the same conversation would be a
        // needless rebuild the reader did not ask for.
        if session.session_path().as_deref() == Some(path.as_path()) {
            return;
        }
        match self.switch_session(&path) {
            Ok(restored) => self
                .transcript
                .push_notice(&format!("resumed — {restored} message(s)")),
            Err(reason) => self
                .transcript
                .push_notice(&format!("cannot resume that session: {reason}")),
        }
    }

    /// Do what clicking the row under the tree cursor does.
    ///
    /// **The two lines the click path runs, not a second implementation of them.** A
    /// keyboard that opens files by a different rule from the mouse is two editors, and
    /// the second one is the one nobody tests.
    fn open_tree_cursor(&mut self) {
        let Some(path) = self.tree_cursor.clone() else {
            return;
        };
        // A cursor left behind by a filter the reader has since typed is not a row.
        if !self
            .tree
            .visible(&self.tree_query)
            .iter()
            .any(|r| r.path == path)
        {
            return;
        }
        if self.tree.is_dir(&path) {
            self.tree.toggle(&path);
        } else {
            self.open_file(&path);
        }
    }

    /// Paint the ring around whichever pane has the keyboard.
    ///
    /// **In `border_focus`, and no new colour.** That role was raised to a 3:1 floor
    /// against its own unfocused sibling precisely so this indicator would be legible
    /// — WCAG 2.4.7 Focus Visible with 1.4.11 Non-text Contrast. A colour invented
    /// here would sit outside every contrast test in `theme.rs`, which is how a role
    /// gets to 1.5:1 without anybody noticing.
    ///
    /// **Skipped while the palette is open**, because the palette owns the keyboard
    /// then and its own selected row is the indicator; a pane ring left on screen would
    /// be naming a region the reader is not typing into.
    fn draw_focus_ring(
        &self,
        painter: &egui::Painter,
        summary: &mut FrameSummary,
        layout: IdeLayout,
    ) {
        if self.palette.is_some() {
            return;
        }
        let rect = match self.focus {
            FocusTarget::Sidebar => layout.sidebar,
            FocusTarget::Editor => layout.center,
            FocusTarget::Agent => layout.agent,
        };
        // A pane the layout gave no height to has nowhere to draw a ring. Recorded
        // only once it is actually painted, so the summary cannot claim an indicator
        // nobody would see.
        if rect.width() <= 0.0 || rect.height() <= 0.0 {
            return;
        }
        painter.rect_stroke(
            rect,
            4.0,
            egui::Stroke::new(2.0, self.theme.border_focus),
            egui::StrokeKind::Inside,
        );
        summary
            .rows
            .push(("focus".into(), self.focus.name().into()));
    }

    /// Which view the sidebar is showing.
    pub fn sidebar_view(&self) -> SidebarView {
        self.sidebar
    }
    /// Put the keyboard on a pane directly.
    ///
    /// `F6` does this in the product. This exists for the test that asks whether the
    /// frame **draws** the ring, which is a different question from whether the key
    /// moves it — the same reason [`Self::set_sidebar_view`] exists.
    pub fn set_focus(&mut self, focus: FocusTarget) {
        self.focus = focus;
    }
    /// Choose the sidebar's view directly.
    ///
    /// The activity bar does this with a click; a test that has to drive three frames
    /// of hit-testing to change one field would be testing the input system instead.
    pub fn set_sidebar_view(&mut self, view: SidebarView) {
        self.sidebar = view;
    }
    /// Open the command palette directly.
    ///
    /// `Ctrl+P` does this from the keyboard; `/help` does it from the composer.
    /// A test that has to drive a chord to open one field would be testing the
    /// input system instead — the same reason [`Self::set_sidebar_view`] exists.
    pub fn open_palette(&mut self) {
        self.model_picker = None;
        self.shortcuts_open = false;
        self.palette = Some(Palette::default());
    }
    /// Say what a command did, in the transcript.
    ///
    /// The sessions button and the session chords answer through this so the
    /// doors cannot disagree about what "ran" looks like. (The composer does
    /// its own routing because it also clears the draft — a different job.)
    fn notice_outcome(&mut self, outcome: crate::commands::Outcome) {
        match outcome {
            crate::commands::Outcome::Ran(said) | crate::commands::Outcome::Refused(said) => {
                self.transcript.push_notice(&said)
            }
            crate::commands::Outcome::Send(_) => {}
        }
    }
    /// Switch the window's palette (M312).
    ///
    /// **The window had no way to change its own palette** — the colour scheme was whatever
    /// `IdeTheme::resolve(None)` picked at launch, so a user on a dark desk who wanted the light
    /// one had to edit a config file and restart. `/theme` exists in the terminal and was refused
    /// here by name, which is the honest default but also the whole gap.
    ///
    /// Bare flips brightness (`ThemeMode::flipped`, which deliberately excludes high contrast); a
    /// name selects one. Both are the terminal's behaviour, in the same shape, so the two front
    /// ends cannot disagree about what `/theme` means.
    ///
    /// **Returns the theme it replaced**, as the terminal's `Theme::set_palette` does, so the
    /// caller can name both ends of the change without a second accessor on a private field.
    pub fn set_palette(
        &mut self,
        palette: epsilon_tui::ui::theme::ThemeMode,
    ) -> crate::theme::IdeTheme {
        std::mem::replace(&mut self.theme, crate::theme::IdeTheme::from(palette))
    }

    /// The window's colours, for a caller that only needs to read them.
    ///
    /// Exists beside [`Self::set_palette`] for the same reason `transcript`, `set_focus` and
    /// `sidebar_view` do: a test has to be able to ask **what the window is painting** rather than
    /// believe a command's sentence. A command that reports a switch it did not perform is
    /// exactly the failure worth a check, and a text-only assertion cannot see it.
    pub fn theme(&self) -> &crate::theme::IdeTheme {
        &self.theme
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
        // A refusal describes a condition — "a run is already in flight" — and conditions
        // end. Nothing cleared `self.refusal` but a later successful submit, so the status bar
        // could sit beside `idle` claiming a turn was running. It clears when the window is no
        // longer running, which is exactly when it stops being true.
        if self.refusal.is_some() && !self.session.as_ref().map(|s| s.running()).unwrap_or(false) {
            self.refusal = None;
        }
        // **The lag count, taken here so the status line can show it (M312).** The counter lives
        // on the session because that is where the fact is born, and it is copied here because
        // the status line is a field of this window. One atomic load per frame, and the copy is
        // what makes the segment reachable at all: nothing in production ever bumped the
        // transcript's own counter, so `N event(s) dropped` had never appeared on a real window.
        self.dropped_events = self.session.as_ref().map_or(0, |s| s.dropped_events());
        // Through the `Arc`, and that is the whole reason `drain` takes `&self`: anything
        // else holding this session — a shortcut, a second window, a test — must not be
        // able to stop the frame from updating.
        let mut arrived = 0;
        if let Some(session) = self.session.as_ref() {
            arrived = session.drain(&mut self.transcript);
        }
        // The goal is re-read **only when something actually arrived**. Every event is a way
        // the model can have changed it (`create_goal`, `update_goal` ride inside a turn), and
        // a frame that drained nothing has learned nothing — so this is per *event batch*,
        // not per frame, and it is the only thing keeping the band off the store lock.
        //
        // **The figures refresh here for the same reason and on the same condition.** Every
        // event batch is a way the context, the token totals and the bill can have moved —
        // `MessageEnd` carries the usage that prices a turn, and every message changes what
        // the next request will carry. They ride the same `arrived` counter rather than
        // getting a second one, because a second counter would be a second rule to keep in
        // step with the first.
        if arrived > 0 {
            self.refresh_goal();
            self.refresh_metrics();
        }
        if let Some(said) = self.take_compact_notice() {
            self.transcript.push_notice(&said);
        }
        self.drive_goal_round();
        self.refresh_changed_files();
    }

    /// Re-read the session goal into [`Self::goal`].
    ///
    /// Called from the two places a goal can change and nowhere else: a frame that applied
    /// events, and [`Self::run_goal_command`] for the palette's own verbs.
    ///
    /// **Public so a caller that reaches into the agent directly can say what it did.** The
    /// three tests that set a goal up by calling `agent.goal_apply` directly are testing a
    /// route the product does not have — but they are the only thing in this crate that can,
    /// so they call this afterwards rather than being rewritten to fake a turn. A future
    /// caller that mutates the agent behind the app's back and forgets this gets a band
    /// showing the previous phase, which is a silent wrong answer and nothing else.
    pub fn refresh_goal(&mut self) {
        self.goal = self
            .session
            .as_ref()
            .and_then(|session| session.agent().goal());
    }

    /// Re-read the session's context, token and cost figures into [`Self::metrics`].
    ///
    /// Called from the two places a figure can change and nowhere else — the same shape as
    /// [`Self::refresh_goal`], and deliberately the same *two*: a frame that applied
    /// events ([`Self::pump`], when `drain` returned something), and
    /// [`Self::switch_session`], which replaces the whole history **without emitting an
    /// event**. A window that refreshed only on events would show the previous session's
    /// context and bill after the reader opened a different file, which is the silent
    /// wrong answer rather than a missing number.
    ///
    /// Public for the reason [`Self::refresh_goal`] is: a caller that mutates the agent
    /// behind the app's back has to say so, because nothing else will notice.
    pub fn refresh_metrics(&mut self) {
        self.metrics = self
            .session
            .as_ref()
            .and_then(|session| IdeApp::read_metrics(session));
    }

    /// The figures, or `None` when there is no agent to ask.
    ///
    /// So a caller can compare them with what the wire surfaces report rather than taking
    /// the title bar's word for it — see `the_window_agrees_with_the_wire_about_the_context`
    /// in `tests/window.rs`.
    pub fn metrics(&self) -> Option<&epsilon_tui::ui::metrics::Metrics> {
        self.metrics.as_ref()
    }

    /// The session's figures, in the **terminal's own vocabulary**.
    ///
    /// # Why `epsilon_tui::ui::metrics::Metrics` and not a local struct
    ///
    /// Because the alternative is a second spelling of the same numbers. This window and
    /// the terminal header answer the same three questions — how much room is left, how
    /// many tokens moved, what it cost — and every one of them has a rule attached that a
    /// second implementation would eventually get wrong: `?` rather than a fabricated `0%`,
    /// `*` on an estimate, `<1%` rather than a rounded `0%`, a trailing `+` on a cost that
    /// is a lower bound, micro-USD until the moment it becomes text. One type, one set of
    /// rules, two frontends.
    ///
    /// It is the terminal crate's type and the desktop already depends on it (ADR-020), but
    /// nothing terminal-shaped comes with it: `Metrics` is plain data with methods that
    /// format numbers, which is the same edge `theme.rs` already has.
    ///
    /// # Why not `Agent::session_state`
    ///
    /// Because that payload is built for **two transports that serve it once per request**,
    /// and this is a paint-thread read. `session_state()` reaches
    /// [`Agent::goal`](epsilon_core::Agent::goal) — which walks `SessionStore::read_all()`
    /// and deep-clones every stored entry, the 30.5 ms at 400 messages and 102.3 ms at
    /// 1600 that [`Self::goal`]'s own field doc records — and `Agent::history()`, a full
    /// deep clone of the transcript whose own doc says "one such call per drawn frame is
    /// enough to make an input-latency problem out of a rendering problem". Calling it here
    /// would reintroduce, per event batch, the exact cost this crate removed at M266.
    ///
    /// The *numbers* are still the shared ones: `SessionState::model` is `Agent::model_id`
    /// and `SessionState::estimated_context_tokens` is `Agent::estimated_context_tokens`, so
    /// reading them through the accessors cannot make this window disagree with what RPC
    /// and `GET /state` serve. `tests/window.rs` asserts that agreement against the real
    /// payload rather than trusting the comment.
    ///
    /// # Cost is priced per model, never at the current one
    ///
    /// `Agent::session_usage_by_model` rather than `Agent::session_usage`, and
    /// `breakdown_per_model` rather than `breakdown`: a token's price is a property of the
    /// model that consumed it, so pricing the summed totals at whatever model is selected
    /// now would reprice every earlier turn the instant you switched, with no visible sign
    /// (M141). The four components are disjoint and are summed before one division, which
    /// is `ui::cost`'s rule and not this window's.
    fn read_metrics(session: &IdeSession) -> Option<epsilon_tui::ui::metrics::Metrics> {
        use epsilon_tui::ui::cost::{CostBreakdown, TokenCounts, TokenPrices, breakdown_per_model};

        let agent = session.agent();
        let launch = session.launch();
        // The model the launch resolved, which is not always the model the agent will
        // send: `--provider faux` beside a saved `deepseek/...` is an ordinary state, and
        // naming the saved one would be the M233 lie in a new place. `model_suffix` already
        // resolves this the same way — see there for why it is a carried decision.
        let model_id = agent.model_id();
        let info = epsilon_llm::catalog::lookup_model(if launch.offline {
            "faux-1"
        } else {
            model_id.as_str()
        });
        let usage = agent.session_usage();
        // The agent reports `Usage` (the wire shape); the pricer wants `TokenCounts` (the
        // four disjoint buckets it multiplies). Same fields, and the split is `Usage`'s own
        // — `input` excludes cached prompt tokens, so the four do not double-charge.
        let by_model: std::collections::BTreeMap<String, TokenCounts> = agent
            .session_usage_by_model()
            .into_iter()
            .map(|(billed_on, u)| {
                (
                    billed_on,
                    TokenCounts {
                        input: u.input,
                        output: u.output,
                        cache_read: u.cache_read,
                        cache_write: u.cache_write,
                    },
                )
            })
            .collect();
        let context_used = agent.estimated_context_tokens();
        // No provider response yet means nothing has been billed, so there is nothing to
        // price and no model whose absence from the registry matters. The terminal's
        // `metrics()` makes exactly this distinction, and a `$0.00` for an empty session is
        // the precise-looking number about nothing this repository keeps refusing.
        let cost = if by_model.is_empty() {
            None
        } else {
            Some(breakdown_per_model(&by_model, |billed_on| {
                epsilon_llm::catalog::lookup_model(billed_on).map(|info| TokenPrices {
                    input: info.input_cost_micro_per_mtok,
                    output: info.output_cost_micro_per_mtok,
                    cache_read: info.cache_read_cost_micro_per_mtok,
                    cache_write: info.cache_write_cost_micro_per_mtok,
                })
            }))
        };
        Some(epsilon_tui::ui::metrics::Metrics {
            model_label: if launch.offline {
                epsilon_llm::catalog::display_label("faux-1").to_string()
            } else {
                epsilon_llm::catalog::display_label(&model_id).to_string()
            },
            offline: launch.offline,
            // **The desktop does not draw a thinking level, and says `None` rather than a
            // word (M312).** This table is built for the *terminal's* header renderer, which
            // the window reuses for its own figures; printing a level here would put a claim
            // on a surface that has no control to change it, and the terminal front end owns
            // the `Ctrl`+`T` that sets one. The desktop's own title bar carries the model.
            thinking: None,
            // `None` rather than `Some(0)`: the gauge prints `?` for an absent measurement
            // and `0%` for a measured one, and a window that printed the second where the
            // terminal prints the first would be asserting something nobody observed.
            context_used: (context_used > 0).then_some(context_used as u64),
            context_window: info.map(|i| i.context_window).filter(|w| *w > 0),
            // The provenance mark, straight from the agent's own flag: on turn one, after
            // every compaction and after `/resume` this figure is a 4-chars-per-token
            // heuristic, and the gauge has to say so rather than set it in the same
            // typeface as a provider's own count.
            context_estimated: agent.context_is_estimated(),
            tokens_in: usage.input,
            tokens_out: usage.output,
            // The session's cache traffic, which is a true answer to "how much of this
            // session came from cache" and is why these two are filled even though the
            // window does not draw `cache_gauge()` — see `Self::figures`.
            cache_read_session: usage.cache_read,
            cache_write_session: usage.cache_write,
            // **The last *call's* cache read and prompt size, deliberately zero and
            // `None`.** A hit rate is a claim about one prompt, so both halves have to come
            // from that prompt; dividing the session totals by one prompt produces a number
            // that climbs every turn and then pins itself at 100% (G16). This window keeps
            // no per-call accumulator, so it reports no rate rather than a wrong one — and
            // the reason is stated at the one place it would have been drawn.
            cache_read_last: 0,
            prompt_tokens_last: None,
            cache_reported: usage.cache_reported,
            cost_micro_usd: cost.as_ref().map(CostBreakdown::total),
            cost_is_floor: cost.as_ref().is_some_and(CostBreakdown::is_floor),
            // A compaction is an event this window's reducer folds away, so it does not
            // carry the flag — and the window does not draw the `· compacted` mark. The `*`
            // above still tells the reader the figure is estimated, which is the half of
            // that distinction a reader acts on. Recorded rather than silently `false`:
            // the next reader should know this is a gap and not an answer.
            compacted: false,
        })
    }

    /// The figures as one line, right-aligned in the title bar, or `None` to draw nothing.
    ///
    /// # Why these three and not four
    ///
    /// Context, tokens and cost are the figures `layout.rs:133` promised this bar and
    /// A38 records as absent. The terminal's fourth — the cache gauge — is **deliberately
    /// not drawn here**, and the reason is the one field above: this window keeps no
    /// per-call usage accumulator, so it can print the cache *volume* but has no honest way
    /// to print the *rate*. `cache_gauge()` in that state renders `CH —`, which the
    /// terminal defines as "the provider reports cache traffic and none of it has hit yet"
    /// — here it would mean "this window does not measure it", which is a different
    /// sentence printed with the same two glyphs. A wrong number that looks right is the
    /// failure this repository treats as worse than a missing one, so the fragment is left
    /// out until a per-call accumulator exists.
    ///
    /// # Why they are shortened rather than dropped
    ///
    /// [`fit`], which marks a cut with `…`. Dropping whole fragments from the tail is the
    /// terminal's rule, and it is the wrong one here: the fragments are the promise
    /// `layout.rs:133` makes, and a bar that silently loses its cost at some width is a bar
    /// that lies about a bill. A marked cut says the line is incomplete; a missing fragment
    /// says nothing at all. Production's minimum window is 720 points wide
    /// (`lib.rs`'s `with_min_inner_size`), where the full line fits inside the half-bar
    /// budget below; the budget exists for the tests and for anyone who ignores it.
    fn figures(&self, width: f32, scale: f32) -> Option<String> {
        let m = self.metrics.as_ref()?;
        let mut parts = vec![m.context_gauge(), Self::token_figures(m)];
        // `cost_gauge` is `None` until something has been billed, which is the honest
        // empty state and the same one the terminal header prints.
        parts.extend(m.cost_gauge());
        Some(fit(&parts.join("  ·  "), width * 0.5, 12.0 * scale))
    }

    /// The token totals in words this window's font can draw (M312).
    ///
    /// [`Metrics::token_gauge`] is `↑12k ↓3k`, and **the arrows render as two empty boxes here**:
    /// this window installs no font of its own, so egui's bundled one draws it, and that font has no
    /// `U+2191`/`U+2193`. The terminal's font does, which is why the same string is right in one
    /// front end and **two tofu boxes** in the other — visible in the title bar of every rendered
    /// frame as `ctx 1%/1.0M · □0 □0`, and the same class of defect M305 found in the sidebar hint.
    ///
    /// **The shared formatter is still used**, so the numbers are spelled exactly as the terminal
    /// spells them (`12k`, `3.4k`); only the two arrows become words. This is M305's answer again —
    /// a glyph the font cannot draw is a **missing** measurement, and a missing measurement is worse
    /// than a spelled-out one.
    pub fn token_figures(m: &epsilon_tui::ui::metrics::Metrics) -> String {
        m.token_gauge().replace('↑', "in ").replace('↓', "· out ")
    }

    /// The colour the figures are drawn in: a warning once the context is worth watching.
    ///
    /// The terminal splits the header into per-fragment roles; here the line is one string,
    /// so the pressure colours the whole thing. That is the same decision the terminal
    /// makes for a narrow row, and it errs towards saying something rather than nothing.
    fn figures_color(&self) -> egui::Color32 {
        use epsilon_tui::ui::metrics::Pressure;
        let Some(m) = self.metrics.as_ref() else {
            return self.theme.dim;
        };
        match m.pressure() {
            Pressure::Warn | Pressure::Danger => self.theme.warn,
            _ => self.theme.dim,
        }
    }

    /// Compact the session context, on a task, because the summariser is a provider call.
    ///
    /// The terminal's `/compact` (M264) and this are the same two lines of behaviour against
    /// the same `Agent::compact_now`, which is the point: a capability with one
    /// implementation behaves identically in both frontends.
    ///
    /// **Off the paint thread.** `compact_now` resends the whole history — the most expensive
    /// call in a session — and awaiting it here would freeze the window for the duration,
    /// which is the failure this window has already produced five other ways.
    ///
    /// The answer comes back on a channel, the same shape as the goal round's. It cannot come
    /// back as an event: `compact_now` emits `CompactionStarted`/`CompactionFinished`, and the
    /// window's reducer folds neither, so a failure would end with a reader who pressed
    /// `/compact` and saw nothing at all.
    pub fn run_compact(&mut self) {
        let Some(session) = self.session.as_ref() else {
            return;
        };
        let agent = Arc::clone(session.agent());
        let tx = self.compact_tx.clone();
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            eprintln!("DIAG no runtime handle");
            self.transcript
                .push_notice("compact needs the agent's runtime — it is not running here");
            return;
        };
        handle.spawn(async move {
            let mut produced = Vec::new();
            let outcome = agent.compact_now(&mut produced).await;
            tx.send(match outcome {
                Ok(_) => "context compacted".to_string(),
                Err(e) => format!("compaction failed: {e}"),
            })
            .ok();
        });
    }

    /// Drain whatever the compaction reported, if anything is waiting.
    fn take_compact_notice(&mut self) -> Option<String> {
        self.compact_rx.try_recv().ok()
    }
    /// Continue an armed goal once the agent has gone idle.
    ///
    /// **Off the paint thread, and that is the whole design.** A goal's gates are shell
    /// commands — `cargo test` on a real project is thirty seconds — and awaiting that here
    /// would freeze the window for the whole run, which is the exact failure this window has
    /// already produced in five other ways. So the round is spawned and its answer arrives
    /// on a channel the next frames drain.
    ///
    /// `pump` runs from both `paint` and `App::logic`, so this is called more than once per
    /// frame; the pending flag is what stops a second round being spent.
    fn drive_goal_round(&mut self) {
        let Some(session) = self.session.as_ref() else {
            return;
        };
        if self.goal_round_pending || session.running() {
            return;
        }
        let Some(view) = self.goal.clone() else {
            return;
        };
        if !view.is_continuable() {
            return;
        }
        // Only a turn that actually ended may start a round. Without the `armed` check a
        // window opened onto a paused goal would immediately spend a round on it.
        if !view.armed {
            return;
        }
        self.goal_round_pending = true;
        let agent = Arc::clone(session.agent());
        let runner = session.gate_runner().cloned();
        let cwd = self.opts.workspace.clone();
        let tx = self.goal_round_tx.clone();
        // `tokio::spawn` **panics** outside a runtime. The product's window runs inside one
        // (`run` builds a multi-thread runtime before the window opens), so this is not the
        // product path — but a paint with no runtime is also a test, and a round driver that
        // takes the window down rather than skipping a round is the wrong trade in both.
        //
        // **And it says so rather than returning silently.** Silence is how the round channel's
        // two-halves defect stayed invisible for six milestones: a key that does nothing, in a
        // window that looks fine. In the product this is rare, and rare is exactly when a
        // silent failure costs most — nobody looks for the one path nobody exercises.
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            self.transcript.push_notice(
                "cannot continue the goal: this window is not running on an agent runtime",
            );
            self.goal_round_pending = false;
            return;
        };
        handle.spawn(async move {
            let round = epsilon_runtime::begin_goal_round(&agent, runner.as_deref(), &cwd).await;
            tx.send(round.unwrap_or(epsilon_runtime::GoalRound::Stop))
                .ok();
        });
    }

    /// Start a round the gate check has answered for.
    ///
    /// The other half of the spawn in [`Self::drive_goal_round`]. Split so the gate's
    /// timing never reaches the draw path, and so a round starts from one place whether or
    /// not gates are involved.
    fn start_goal_round(&mut self, round: epsilon_runtime::GoalRound) -> bool {
        self.goal_round_pending = false;
        let (prompt, note) = match round {
            epsilon_runtime::GoalRound::Stop => return false,
            epsilon_runtime::GoalRound::Continue {
                prompt,
                gates_passed,
            } => {
                let note = if gates_passed > 0 {
                    format!("goal continuing — {gates_passed} gate(s) passed")
                } else {
                    "goal continuing".to_string()
                };
                (prompt, note)
            }
            epsilon_runtime::GoalRound::GateFailed { prompt, reports } => {
                // The gate's own sentence goes in front of the reader, and its output goes
                // into the round. A reader who sees "continuing" and no reason would take
                // it for progress.
                let note = format!(
                    "goal gate failed: {}",
                    reports
                        .first()
                        .map(|r| r.sentence())
                        .unwrap_or_else(|| "a gate did not pass".into())
                );
                (prompt, note)
            }
        };
        self.transcript.push_notice(&note);
        // The reader's own words for a harness-authored turn, the same echo the composer
        // gives a typed one. Deliberately **not** `goal_apply`: a round is a turn, and the
        // goal itself is changed only by the model through the `goal` tool.
        self.transcript.push_user(&prompt);
        // Submitted through the session so the transcript, the run and the bridge all see
        // it exactly as they would a typed turn.
        if let Some(session) = self.session.as_ref()
            && let Err(reason) = session.submit(&prompt)
        {
            self.transcript
                .push_notice(&format!("goal round could not start: {reason}"));
            return false;
        }
        true
    }

    /// A gate-checked round, if one has arrived. Non-blocking, once per frame.
    fn take_goal_round(&mut self) -> Option<epsilon_runtime::GoalRound> {
        self.goal_round_rx.try_recv().ok()
    }

    /// The half of [`Self::start_goal_round`] a test can drive, without the spawn.
    ///
    /// Exists because the interesting half of a goal round is what the window **does with
    /// an answer** — a gate failure has to become a notice a reader can see — and the answer
    /// otherwise arrives on a channel from a task this crate does not control. A seam rather
    /// than a `pub` on the whole method, so the production path keeps its guard.
    #[doc(hidden)]
    pub fn start_goal_round_for_test(&mut self, round: epsilon_runtime::GoalRound) -> bool {
        self.start_goal_round(round)
    }

    /// Run the command with this **id**, as choosing its palette row would.
    ///
    /// Keyed on the id rather than the label on purpose, which is the whole reason the table
    /// carries one. `query` is the palette's text, which `goal: create` takes as the
    /// objective.
    #[doc(hidden)]
    pub fn run_palette_command(&mut self, id: &str, query: &str) -> bool {
        match commands().into_iter().find(|c| c.id == id) {
            Some(cmd) => {
                self.run_palette_action(cmd.action, query);
                true
            }
            None => {
                // The derived verb rows live in `palette_entries`, not in the
                // table above: same dispatch, with the head resolved back to
                // its static spelling rather than trusted from the caller.
                match crate::commands::SUPPORTED
                    .iter()
                    .copied()
                    .find(|head| *head == id)
                {
                    Some(head) => {
                        self.run_palette_action(PaletteAction::Verb(head), query);
                        true
                    }
                    None => false,
                }
            }
        }
    }
    /// Re-read what the agent just changed: the tree, and the open file if it is one.
    ///
    /// **A window is a snapshot, and nothing told it the world had moved.** The tree was
    /// walked once in `new` and the editor holds whatever the file said when it was opened,
    /// so a file the agent has just written is absent from the tree and a file it has just
    /// edited still shows the text the edit replaced. Both are the failure this repository
    /// keeps refusing to ship: the change row appeared in the transcript and the panes did
    /// not move.
    ///
    /// Only the open file is re-read, and only when its own path is among those changed, so
    /// a file that cannot be read now really is gone rather than transiently unreadable.
    /// A file that has gone is **closed and said out loud**, because keeping its last
    /// contents on screen would be showing the reader something that is not there.
    fn refresh_changed_files(&mut self) {
        let changed = self.transcript.take_changed_paths();
        if changed.is_empty() {
            return;
        }
        self.tree = crate::workspace::FileTree::load(&self.opts.workspace);
        // A filter that was showing a file the agent has just created should keep showing
        // it, so the query survives; only the tree behind it is rebuilt.
        let Some(open) = self.open.as_ref().map(|f| f.path.clone()) else {
            return;
        };
        if !changed.contains(&open) {
            return;
        }
        match crate::workspace::OpenFile::read(&self.opts.workspace, &open) {
            Ok(file) => self.open = Some(file),
            Err(reason) => {
                self.transcript.push_notice(&format!(
                    "{open} was changed and is now unreadable: {reason}"
                ));
                self.open = None;
            }
        }
    }
    /// Paint one frame and describe what it drew.
    ///
    /// This is the whole of the draw path. [`eframe::App::ui`] is a thin wrapper so a test
    /// can paint into an off-screen `egui::Context` at any size, with no window.
    pub fn paint(&mut self, ui: &mut egui::Ui) -> FrameSummary {
        let ctx = ui.ctx();
        // **A close request is caught here, before anything is drawn.** egui flags a close
        // the *platform* raised (`ViewportInfo::close_requested`) and waits for the frame to
        // end; `ViewportCommand::CancelClose` is how the application says "not yet". DSH does
        // the same with its host inspection, and the reason it exists is `AGENTS.md` §6:
        // **never lose a user's work.** Closing the window used to end the process, so a turn
        // in flight stopped where it was and a parked permission ask was discarded.
        //
        // `close_requested` is a flag egui only ever *reads* — the backend produces it — so
        // this is also the seam a test can drive, and there is one implementation either way.
        if ctx.input(|i| i.viewport().close_requested()) {
            self.answer_close_request(ctx);
        }
        // **The dialog owns Escape while it is up, and it reads it HERE.**
        //
        // The composer takes the same chord to clear a draft, and it draws long before the
        // dialog does — so a read down there would never see the key, and Escape would
        // silently clear the reader's words instead of dismissing the question. This is the
        // ordering rule `approve_pending` already follows for the same reason, and the
        // failure when it is got wrong is invisible: the dialog simply does nothing.
        if self.quit_confirm.is_some() && self.keys.take(ctx, "window.composer.escape") {
            self.quit_confirm = None;
        }
        // **Make the zoom reachable before reading it.** The scale is plumbed everywhere, but
        // plumbing nobody can move is not a feature — it is an unreachable code path wearing a
        // feature's name. `egui::gui_zoom` supplies the clamped step (`zoom_in` / `zoom_out`);
        // the keys are bound here because this crate draws its own chrome and has no menu bar to
        // hang `zoom_menu_buttons` off. `Ctrl`+`+` / `Ctrl`+`-` / `Ctrl`+`0` is the set every
        // editor uses, and egui owns the clamping so there is one rule and not two.
        //
        // **Before** `text_scale` is read below, because this is what sets the factor it returns.
        let ctx = ui.ctx();
        // **Three bindings, read by name.** The shifted and unshifted forms of "zoom in"
        // are two chords of one id in the shared table — `plus` and `=` — because a
        // keyboard reports whichever it has and a person pressing `Ctrl`+`=` means the
        // same thing as `Ctrl`+`Shift`+`=`. Writing both here would put the decision in
        // this function; the table is where a keybinding lives.
        let zoom = (
            self.keys.take(ctx, "window.zoom.in"),
            self.keys.take(ctx, "window.zoom.out"),
            self.keys.take(ctx, "window.zoom.reset"),
        );
        if zoom.0 {
            egui::gui_zoom::zoom_in(ctx);
        }
        if zoom.1 {
            egui::gui_zoom::zoom_out(ctx);
        }
        if zoom.2 {
            ctx.set_zoom_factor(1.0);
        }
        let screen = ctx.content_rect();
        // **Once per frame, and passed on rather than re-read.** A painter that re-read
        // `zoom_factor` at twenty sites would give twenty chances to forget one, and the
        // symptom is a window whose composer grew while its transcript did not.
        let scale = text_scale(ctx);
        // The widget half, before anything is drawn, and restored at the end of the frame.
        //
        // **The Context, not this `Ui`** — see [`scale_widget_style`]: every pane `Ui` reads
        // `ctx.global_style()`, so a style set here would reach nothing at all.
        //
        // **Restored because the next frame scales the same style again**: leaving it
        // scaled would multiply the window's own text by the scale once per frame, which is
        // a window that doubles every repaint rather than one that scales.
        let base_style = ui.ctx().global_style();
        let theme = ui.ctx().theme();
        let mut scaled = base_style.as_ref().clone();
        scale_widget_style(&mut scaled, scale);
        ui.ctx().set_style_of(theme, scaled);
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
            approve_pending(&s, &self.keys, ui.ctx());
        }
        // The palette reads Enter and Escape, so it goes before the composer — which takes
        // both unconditionally, to send and to abort.
        self.palette_keys(ui.ctx());
        // The model picker reads the same keys for the same reason, after the
        // palette (an open palette owns every key) and before the composer.
        self.model_keys(ui.ctx());
        // The shortcuts reference reads two keys — toggle and close — after
        // both pickers, for the same ownership reason.
        self.shortcuts_keys(ui.ctx());
        // Pane focus, and the keys that act inside the focused pane. **After the
        // palette**, which owns those keys while it is open, and **before every pane is
        // drawn**, because the composer consumes Enter and Escape unconditionally and a
        // key read after the widget that wanted it has already been had.
        self.pane_keys(ui.ctx());
        // **The carve, after every key handler — and that placement is the difference between
        // a fold that works and a fold that is reported one frame late.** It used to run
        // first, so a key that *changes the layout* was answered by the next frame: the frame
        // that saw `Ctrl`+`B` still drew the sidebar the key had just removed, and a reader
        // (or a test reading that frame) saw a keystroke whose only visible effect was the
        // title bar's button re-labelling itself.
        //
        // The handlers above still get the **previous** frame's geometry, which is exactly
        // what they were getting before: `self.layout` is only replaced once they have read
        // it, and `pane_keys` needs the last frame's rects for the same reason the tab strip
        // does.
        self.layout = IdeLayout::new(screen, self.prefs);
        let l = self.layout;
        let mut summary = FrameSummary {
            tier: l.tier,
            size: [screen.width(), screen.height()],
            ..FrameSummary::default()
        };
        // A round whose gates were checked lands here rather than at the spawn: the check
        // ran on its own task, so its answer arrives on a later frame and a slow gate costs
        // seconds of a live window rather than seconds of a frozen one.
        while let Some(round) = self.take_goal_round() {
            self.start_goal_round(round);
        }
        // ---- window chrome --------------------------------------------------
        let painter = ui.painter();
        if l.title.height() > 0.0 {
            painter.rect_filled(l.title, 0.0, self.theme.panel);
            painter.line_segment(
                [l.title.left_bottom(), l.title.right_bottom()],
                egui::Stroke::new(1.0, self.theme.border),
            );
            // **How much of the bar's right end belongs to the sidebar toggle**, resolved
            // before anything is measured, because the title line's budget needs it. A
            // right-aligned figure painted under a control is an overlap, and overlap is
            // the one failure this crate's layout tests exist to rule out.
            //
            // **Hoisted above the title line, 2026-10-04.** It used to be computed *after*
            // the title was painted, which is why the title could not be budgeted against
            // it — see below.
            let reserve = if l.title.width() >= SIDEBAR_TOGGLE_MIN_W {
                SIDEBAR_TOGGLE_INSET + SIDEBAR_TOGGLE_W / 2.0 + 14.0
            } else {
                0.0
            };
            let mut line = self
                .opts
                .workspace
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("Epsilon")
                .to_string();
            line.push_str(&self.model_suffix());
            // **Budgeted, which this line was not.** The workspace name and the model
            // suffix were the only unbounded strings in the window that never went through
            // `fit`: every other long string does (file rows, task lines, sessions, tab
            // labels, palette rows, transcript rows), and the title bar drew on a root `Ui`
            // whose clip rect is the whole content rect, so nothing clipped it either. At
            // 200% the text is twice as wide and runs straight into the figures. The
            // figures are already `fit`-shortened by `self::figures`, so the two shorten
            // together — which is the behaviour the comment above the figures describes and
            // the title line never had.
            let line = fit(
                &line,
                (l.title.width() - 28.0 - reserve).max(0.0),
                15.0 * scale,
            );
            paint_text(
                painter,
                &mut summary,
                "title",
                l.title.left_top() + egui::vec2(14.0, 23.0),
                egui::Align2::LEFT_CENTER,
                &line,
                egui::FontId::proportional(15.0 * scale),
                self.theme.title,
            );
            // **The title bar's accessible name**, started from the string it just drew so
            // the two cannot disagree: the workspace name and, when a launch attached one,
            // the model behind it. Everything on it is a name a reader chose — the workspace
            // directory and the model label — and neither is content. Built before the row is
            // pushed so `line` is borrowed rather than cloned.
            let mut title_name = format!("Title bar · {line}");
            summary.rows.push(("title".into(), line));
            // **The rest of what `layout.rs:133` promises this bar.** Its doc reads
            // "workspace, model, context, cost"; workspace and model were on the left, and
            // context and cost were nowhere in the product's IDE at all — which is A38, and
            // the cost of it is a reader who cannot tell when to compact.
            //
            // **Right-aligned, in the bar `layout.rs` already reserves, not in a new
            // panel.** The status bar below is the wrong home for a number: it is ordered
            // "what is waiting on the user" first, and the workspace name is short and
            // fixed, so the figures have a corner of their own to sit in.
            //
            // Recorded under its own region rather than as more `"title"` rows, because
            // `FrameSummary::region` is what a test reads and "is a figure on screen" and
            // "did the bar name the workspace" are different questions.
            if let Some(figures) = self.figures(l.title.width() - reserve, scale) {
                paint_text(
                    painter,
                    &mut summary,
                    "figures",
                    l.title.right_top() + egui::vec2(-14.0 - reserve, 23.0),
                    egui::Align2::RIGHT_CENTER,
                    &figures,
                    egui::FontId::proportional(12.0 * scale),
                    self.figures_color(),
                );
                summary.rows.push(("figures".into(), figures.clone()));
                // **The figures are what the bar is for** — `layout.rs:133` promises
                // "workspace, model, context, cost" and two of those four live on the right
                // of this bar. They go into the name for the same reason they are on screen:
                // a bar that can be heard but not asked what it says is only half a bar.
                // Already `fit`-shortened by `figures`, so the name is bounded exactly as the
                // line is, and a narrow window shortens both together.
                title_name.push_str(&format!(" · {figures}"));
            }
            name_region(
                ui,
                l.title,
                TITLE_SALT,
                &title_name,
                // `Banner` is the ARIA landmark for the title bar: the region that identifies
                // the application and its context, and the one a rotor offers first.
                egui::accesskit::Role::Banner,
            );
        }
        if l.status.height() > 0.0 {
            painter.rect_filled(l.status, 0.0, self.theme.panel);
            painter.line_segment(
                [l.status.left_top(), l.status.right_top()],
                egui::Stroke::new(1.0, self.theme.border),
            );
            // **The hint is decided before the status text is painted, not after**, so the
            // status line can be budgeted against the width the hint will take. It used to be
            // the other way round, and the comment below used to explain the consequence:
            // "the status text is drawn from the left with no budget of its own, so a narrow
            // bar would run the two into each other — which is why the width guard exists".
            // **The width guard was a workaround for a missing budget, and it is now the
            // budget.** A guard withdraws the hint whenever the bar is under 420 pt; a
            // budget shortens the *text* instead, which keeps the one control a reader can
            // still learn the chord from and says which part of the line was cut.
            let hint = if self.palette.is_none() && l.status.width() >= 420.0 {
                Some(if self.layout.sidebar_visible {
                    // **The chord is read from the binding table, not typed here.** A hint
                    // that hardcodes `ctrl+b` names a key the window only *hopes* is bound;
                    // a reader who rebinds it must see their own chord or the hint is a lie.
                    let toggle = self
                        .keys
                        .label("window.sidebar.toggle")
                        .unwrap_or_else(|| "ctrl+b".into());
                    // **Spelled out, and this is not a style preference — it is what the
                    // rendered frame showed.** The first rasterised frame of this window
                    // (`audit/evidence/desktop-frames-2026-10-04/desktop-raster.png`) drew
                    // the hint as `{toggle} sidebar  ·  □/□ width`: `←` and `→` are **tofu**
                    // in egui's proportional face. Nobody had seen the frame, so nothing in
                    // the repository could know, and every test here asserts on the *string* —
                    // `hint.contains("left/right")` passes just as happily on a box as on an
                    // arrow. Words cost four characters and cannot be missing from a font.
                    format!("{toggle} sidebar  ·  left/right width")
                } else {
                    format!(
                        "{} sidebar",
                        self.keys
                            .label("window.sidebar.toggle")
                            .unwrap_or_else(|| "ctrl+b".into())
                    )
                })
            } else {
                None
            };
            let status_font = 12.0 * scale;
            // The same `AVG_ADVANCE` approximation `fit` budgets with, so the two cannot
            // disagree about how wide a string is.
            let hint_width = hint
                .as_ref()
                .map(|h| h.chars().count() as f32 * status_font * AVG_ADVANCE)
                .unwrap_or(0.0);
            let text = fit(
                &self.status_line(),
                (l.status.width() - 28.0 - hint_width).max(0.0),
                status_font,
            );
            paint_text(
                painter,
                &mut summary,
                "status",
                l.status.left_top() + egui::vec2(14.0, 12.0),
                egui::Align2::LEFT_CENTER,
                &text,
                egui::FontId::proportional(status_font),
                self.theme.dim,
            );
            summary.rows.push(("status".into(), text));
            // **The state, not the line.** `status_line` also carries the refusal text and
            // the tier, both of which are free-form or a word about the window that a reader
            // cannot act on; a name built from the whole line would be unbounded and would
            // announce a tool's own sentence every repaint. `run_state` is the bounded part
            // of it, and it is the part that changes: idle is not running is not no agent.
            name_region(
                ui,
                l.status,
                STATUS_SALT,
                &format!("Status bar · {}", self.run_state()),
                // `Status` is the ARIA landmark for a live region of the application's state.
                // It is the honest role here: what this band reports is what the harness is
                // doing right now, and it changes without the reader moving.
                egui::accesskit::Role::Status,
            );
            // **The key hints, in the band `layout.rs` reserved them for.** `layout.rs`
            // documents this bar as "goal phase, pending approvals, key hints"; the first two
            // were drawn and the third was not, which is how a bound chord stays invisible to
            // the person who has to learn it.
            //
            // **Only for keys that work on this frame.** The palette owns every key while it
            // is open, and the arrows do nothing once the sidebar has folded, so the hint
            // shrinks to the one chord that is live in either state. Advertising a dead key is
            // the `AGENTS.md` §2.7 failure in its smallest possible form.
            //
            // **Right-aligned, and only where both lines fit.** The status text is now
            // budgeted against this hint rather than merely left to collide with it — see
            // where `hint` is decided, above.
            if let Some(hint) = hint {
                paint_text(
                    painter,
                    &mut summary,
                    "hints",
                    l.status.right_top() + egui::vec2(-14.0, 12.0),
                    egui::Align2::RIGHT_CENTER,
                    &hint,
                    egui::FontId::proportional(12.0 * scale),
                    self.theme.dim,
                );
                // **Its own region, not more `"status"`.** `paint_text` records only the
                // size; a line a reader has to find inside another region's rows is a line
                // this crate's own `FrameSummary::region` cannot ask about, and
                // `at_the_default_zoom_every_size_is_exactly_what_it_was` pins the status
                // bar's sizes exactly.
                summary.rows.push(("hints".into(), hint));
            }
        }
        // **After both bands, and that placement is the borrow checker agreeing with the
        // design.** `egui::Button` has to be put through `&mut Ui` while the chrome's
        // `Painter` is still borrowed from the same `Ui`, so the toggle is registered after
        // the last painter call — which is also the last thing drawn in the title bar, and
        // therefore the thing a click in that corner reaches.
        if l.title.height() > 0.0 {
            self.sidebar_toggle(ui, &mut summary, l.title);
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
            self.activity_bar(&mut pane, &mut summary, l.activity, scale);
        }
        if l.sidebar_visible {
            let mut pane = pane_ui(ui, l.sidebar, "sidebar");
            self.workspace_pane(&mut pane, &mut summary, l.sidebar, scale);
        }
        if l.center_visible {
            let mut pane = pane_ui(ui, l.center, "center");
            self.editor_pane(&mut pane, &mut summary, l.center, scale);
        }
        if l.agent_visible {
            let mut pane = pane_ui(ui, l.agent, "agent");
            self.agent_panel(&mut pane, &mut summary, l.agent, scale);
        }
        // **After the panes**, so the handle is the last thing registered over that gutter
        // and a press beside the sidebar's edge reaches it rather than the file rows.
        self.sidebar_resize(ui, &mut summary);
        // The focus ring: **after the panes**, so it lands on top of the one-pixel
        // border it replaces, and **before the palette**, so the palette — which covers
        // the window while it is open — stays the topmost thing.
        self.draw_focus_ring(ui.painter(), &mut summary, l);
        // Drawn **last**, so it is on top of every pane rather than behind whichever one
        // happened to be laid out last.
        self.draw_palette(ui, &mut summary, scale);
        // The model picker draws beside the palette, same layer: only one of
        // the two is ever open (opening either closes the other).
        self.draw_model_picker(ui, &mut summary, scale);
        // The shortcuts reference draws on the same layer, under the same
        // one-overlay rule.
        self.draw_shortcuts(ui, &mut summary, scale);
        // **The quit dialog is above the palette, not beside it.** A close request arrives
        // from outside the window — from a button the reader pressed without touching the
        // application — so it cannot be queued behind whatever the application happens to
        // have open. A guard that waits its turn behind a palette is a guard that sometimes
        // does not run.
        //
        // It reads the keyboard **here**, after the palette and the composer have had theirs,
        // so a key that sent a prompt or opened a palette is not also read as "keep the
        // window open".
        if self.quit_confirm.is_some() {
            self.draw_quit_confirm(ui, &mut summary, scale);
        }
        ui.ctx().set_style_of(theme, base_style);
        summary
    }

    /// The palette: a search box over the window, and the rows it offers.
    ///
    /// Positioned rather than centred because the eye goes to the top for a thing you are
    /// typing into, and because a centred box covers the transcript the reader may be
    /// choosing a file *because* of.
    fn draw_palette(&mut self, ui: &mut egui::Ui, summary: &mut FrameSummary, scale: f32) {
        let Some(palette) = self.palette.as_ref() else {
            return;
        };
        self.palette_rects.clear();
        let query = palette.query.clone();
        let selected = palette.selected;
        let entries = self.palette_entries(&query);
        let shown: Vec<&PaletteEntry> = entries.iter().take(PALETTE_LIMIT).collect();

        let screen = ui.ctx().content_rect();
        let width = (screen.width() * 0.6).clamp(240.0, 720.0);
        // The box's own height is the search row plus one row per entry, so it is text
        // geometry and it scales with the text in it.
        let height = ((28.0 + shown.len() as f32 * 22.0 + 12.0) * scale).min(screen.height() * 0.6);
        let box_rect = egui::Rect::from_min_size(
            egui::pos2(screen.center().x - width / 2.0, screen.top() + 48.0),
            egui::vec2(width, height),
        );
        let theme = self.theme;
        let painter = ui.painter().with_clip_rect(screen);
        painter.rect_filled(box_rect, 6.0, theme.panel);
        painter.rect_stroke(
            box_rect,
            6.0,
            egui::Stroke::new(1.0, theme.border),
            egui::StrokeKind::Inside,
        );

        // The search box. **It must hold the keyboard**, and the composer's focus rule
        // yields only to whatever already has it, so the request is made here every frame
        // the palette is open rather than once on the frame it opened.
        let search = egui::Rect::from_min_size(
            box_rect.left_top() + egui::vec2(8.0, 8.0),
            egui::vec2(box_rect.width() - 16.0, 24.0 * scale),
        );
        let response = ui.put(
            search,
            egui::TextEdit::singleline(&mut self.palette.as_mut().expect("checked").query)
                .id(palette_input_id())
                .hint_text("files and commands")
                .desired_width(search.width() - 8.0),
        );
        response.request_focus();
        summary
            .hotspots
            .push(("palette-search".into(), response.rect));

        let mut y = search.max.y + 4.0;
        for (index, entry) in shown.iter().enumerate() {
            let row = egui::Rect::from_min_size(
                egui::pos2(box_rect.left() + 8.0, y),
                egui::vec2((box_rect.width() - 16.0).max(8.0), 20.0 * scale),
            );
            if index == selected {
                painter.rect_filled(row, 3.0, theme.selection_focus);
            }
            let color = if index == selected {
                theme.title
            } else {
                theme.text
            };
            paint_text(
                &painter,
                summary,
                "palette",
                row.left_top() + egui::vec2(4.0, 14.0 * scale),
                egui::Align2::LEFT_CENTER,
                entry.label.clone(),
                egui::FontId::proportional(13.0 * scale),
                color,
            );
            // The hint on the right, so `sessions` the file and `sessions` the command
            // are told apart before either is opened.
            paint_text(
                &painter,
                summary,
                "palette",
                row.right_top() - egui::vec2(4.0, 14.0 * scale),
                egui::Align2::RIGHT_CENTER,
                entry.hint.clone(),
                egui::FontId::proportional(11.0 * scale),
                theme.dim,
            );
            summary.rows.push(("palette".into(), entry.label.clone()));
            summary.row_rects.push(("palette".into(), row));
            self.palette_rects.push((index, row));
            // **A row is a command, and a painted row is invisible to a screen reader.**
            // The palette is drawn with `Painter::text` on rectangles, so nothing about these
            // reached the tree until now: the overlay was a picture of a list of things that
            // can be opened.
            //
            // **Named by what it is, with its hint.** `entry.hint` is the word that tells
            // `sessions` the file from `sessions` the command — the same reason it is drawn on
            // the right of the row. A name that dropped it would make those two rows say the
            // same thing, which is the ambiguity the hint exists to remove.
            //
            // **Salted by the row's identity, not by its position.** `index` is stable only
            // while the query is, and a reader typing one character would watch every node
            // below it take a new id. The command id and the workspace-relative path are the
            // same whatever order the filter puts them in.
            name_region(
                ui,
                row,
                &format!(
                    "epsilon-palette-row-{}",
                    entry.command.unwrap_or(entry.label.as_str())
                ),
                &format!("Palette row · {} · {}", entry.label, entry.hint),
                // `Dialog`: the palette is a modal surface over the window, and this is
                // the row inside it. `Region` would be true and useless.
                egui::accesskit::Role::Region,
            );
            // A command row is addressable by its **id**, so a test presses
            // `command:goal.pause` rather than the label a reader sees — a label is what
            // gets reworded, and a test keyed on it fails for a reason unrelated to what it
            // was testing.
            if let Some(id) = entry.command {
                summary.hotspots.push((format!("command:{id}"), row));
            }
            y += 22.0 * scale;
        }
        if entries.len() > PALETTE_LIMIT {
            let note = format!("… {} more — keep typing", entries.len() - PALETTE_LIMIT);
            paint_text(
                &painter,
                summary,
                "palette",
                egui::pos2(box_rect.left() + 12.0, y + 10.0 * scale),
                egui::Align2::LEFT_CENTER,
                note.clone(),
                egui::FontId::proportional(11.0 * scale),
                theme.dim,
            );
            summary.rows.push(("palette".into(), note));
        }
        if shown.is_empty() {
            let none = "nothing matches";
            paint_text(
                &painter,
                summary,
                "palette",
                egui::pos2(box_rect.left() + 12.0, search.max.y + 18.0 * scale),
                egui::Align2::LEFT_CENTER,
                none,
                egui::FontId::proportional(12.0 * scale),
                theme.dim,
            );
            summary.rows.push(("palette".into(), none.into()));
        }
        summary
            .rows
            .push(("palette".into(), "esc closes · enter opens".into()));
    }

    /// The model picker: the terminal's two stages, in the window's overlay.
    ///
    /// Positioned like the palette, for the same reason: the eye goes to the
    /// top for a thing you are typing into. Providers first; choosing one
    /// drills into its models.
    fn draw_model_picker(&mut self, ui: &mut egui::Ui, summary: &mut FrameSummary, scale: f32) {
        let Some(picker) = self.model_picker.as_ref() else {
            return;
        };
        self.model_rects.clear();
        let selected = picker.selected;
        let stage = picker.provider.clone();
        let entries = self.model_picker_entries();
        let shown: Vec<&ModelPickerEntry> = entries.iter().take(PALETTE_LIMIT).collect();

        let screen = ui.ctx().content_rect();
        let width = (screen.width() * 0.6).clamp(240.0, 720.0);
        let height = ((28.0 + shown.len() as f32 * 22.0 + 12.0) * scale).min(screen.height() * 0.6);
        let box_rect = egui::Rect::from_min_size(
            egui::pos2(screen.center().x - width / 2.0, screen.top() + 48.0),
            egui::vec2(width, height),
        );
        let theme = self.theme;
        let painter = ui.painter().with_clip_rect(screen);
        painter.rect_filled(box_rect, 6.0, theme.panel);
        painter.rect_stroke(
            box_rect,
            6.0,
            egui::Stroke::new(1.0, theme.border),
            egui::StrokeKind::Inside,
        );

        // The search box. Same contract as the palette's: it must hold the
        // keyboard, so the request is made here every frame the picker is open.
        let hint = match &stage {
            None => "providers".to_string(),
            Some(provider) => format!("models of {provider}"),
        };
        let search = egui::Rect::from_min_size(
            box_rect.left_top() + egui::vec2(8.0, 8.0),
            egui::vec2(box_rect.width() - 16.0, 24.0 * scale),
        );
        let response = ui.put(
            search,
            egui::TextEdit::singleline(&mut self.model_picker.as_mut().expect("checked").query)
                .id(model_input_id())
                .hint_text(hint)
                .desired_width(search.width() - 8.0),
        );
        response.request_focus();
        summary
            .hotspots
            .push(("model-search".into(), response.rect));

        let mut y = search.max.y + 4.0;
        for (index, entry) in shown.iter().enumerate() {
            let row = egui::Rect::from_min_size(
                egui::pos2(box_rect.left() + 8.0, y),
                egui::vec2((box_rect.width() - 16.0).max(8.0), 20.0 * scale),
            );
            if index == selected {
                painter.rect_filled(row, 3.0, theme.selection_focus);
            }
            let color = if index == selected {
                theme.title
            } else {
                theme.text
            };
            paint_text(
                &painter,
                summary,
                "models",
                row.left_top() + egui::vec2(4.0, 14.0 * scale),
                egui::Align2::LEFT_CENTER,
                entry.label.clone(),
                egui::FontId::proportional(13.0 * scale),
                color,
            );
            if !entry.meta.is_empty() {
                paint_text(
                    &painter,
                    summary,
                    "models",
                    row.right_top() - egui::vec2(4.0, 14.0 * scale),
                    egui::Align2::RIGHT_CENTER,
                    entry.meta.clone(),
                    egui::FontId::proportional(11.0 * scale),
                    theme.dim,
                );
            }
            summary.rows.push(("models".into(), entry.label.clone()));
            summary.row_rects.push(("models".into(), row));
            self.model_rects.push((index, row));
            // **A row is a model, and a painted row is invisible to a screen
            // reader.** Same arrangement as the palette: named by what it is,
            // salted by the row's identity (a provider or model id is stable
            // whatever order the filter puts it in), not by its position.
            let name = if entry.meta.is_empty() {
                format!("Model row · {}", entry.label)
            } else {
                format!("Model row · {} · {}", entry.label, entry.meta)
            };
            name_region(
                ui,
                row,
                &format!("epsilon-model-row-{}", entry.id),
                &name,
                egui::accesskit::Role::Region,
            );
            y += 22.0 * scale;
        }
        if entries.len() > PALETTE_LIMIT {
            let note = format!("… {} more — keep typing", entries.len() - PALETTE_LIMIT);
            paint_text(
                &painter,
                summary,
                "models",
                egui::pos2(box_rect.left() + 12.0, y + 10.0 * scale),
                egui::Align2::LEFT_CENTER,
                note.clone(),
                egui::FontId::proportional(11.0 * scale),
                theme.dim,
            );
            summary.rows.push(("models".into(), note));
        }
        if shown.is_empty() {
            let none = "nothing matches";
            paint_text(
                &painter,
                summary,
                "models",
                egui::pos2(box_rect.left() + 12.0, search.max.y + 18.0 * scale),
                egui::Align2::LEFT_CENTER,
                none,
                egui::FontId::proportional(12.0 * scale),
                theme.dim,
            );
            summary.rows.push(("models".into(), none.into()));
        }
        summary
            .rows
            .push(("models".into(), "esc backs out · enter chooses".into()));
    }

    /// The shortcuts reference: every window binding with its effective chord.
    ///
    /// A read-only overlay in the palette's position — the eye goes to the top
    /// for a thing you opened to read. Rows come straight from the binding
    /// table (see [`Self::shortcut_rows`]), so a rebound chord is shown
    /// rebound. No cap and no filter: a reference that hides rows is an index
    /// with pages torn out.
    fn draw_shortcuts(&mut self, ui: &mut egui::Ui, summary: &mut FrameSummary, scale: f32) {
        if !self.shortcuts_open {
            return;
        }
        let rows = self.shortcut_rows();

        let screen = ui.ctx().content_rect();
        let width = (screen.width() * 0.6).clamp(240.0, 720.0);
        let height = ((28.0 + rows.len() as f32 * 22.0 + 12.0) * scale).min(screen.height() * 0.9);
        let box_rect = egui::Rect::from_min_size(
            egui::pos2(screen.center().x - width / 2.0, screen.top() + 48.0),
            egui::vec2(width, height),
        );
        let theme = self.theme;
        let painter = ui.painter().with_clip_rect(screen);
        painter.rect_filled(box_rect, 6.0, theme.panel);
        painter.rect_stroke(
            box_rect,
            6.0,
            egui::Stroke::new(1.0, theme.border),
            egui::StrokeKind::Inside,
        );

        paint_text(
            &painter,
            summary,
            "shortcuts",
            box_rect.left_top() + egui::vec2(12.0, 18.0 * scale),
            egui::Align2::LEFT_CENTER,
            "Keyboard shortcuts".to_string(),
            egui::FontId::proportional(13.0 * scale),
            theme.title,
        );
        summary
            .rows
            .push(("shortcuts".into(), "Keyboard shortcuts".into()));

        let mut y = box_rect.top() + 32.0 * scale;
        for (id, chords) in &rows {
            let row = egui::Rect::from_min_size(
                egui::pos2(box_rect.left() + 8.0, y),
                egui::vec2((box_rect.width() - 16.0).max(8.0), 20.0 * scale),
            );
            paint_text(
                &painter,
                summary,
                "shortcuts",
                row.left_top() + egui::vec2(4.0, 14.0 * scale),
                egui::Align2::LEFT_CENTER,
                id.clone(),
                egui::FontId::proportional(13.0 * scale),
                theme.text,
            );
            paint_text(
                &painter,
                summary,
                "shortcuts",
                row.right_top() - egui::vec2(4.0, 14.0 * scale),
                egui::Align2::RIGHT_CENTER,
                chords.clone(),
                egui::FontId::proportional(11.0 * scale),
                theme.dim,
            );
            summary.rows.push(("shortcuts".into(), id.clone()));
            // **Named rows, like the palette's.** Painted text reaches no
            // screen reader on its own; the salt is the binding id, which is
            // stable whatever the table order becomes.
            name_region(
                ui,
                row,
                &format!("epsilon-shortcut-row-{id}"),
                &format!("Shortcut · {id} · {chords}"),
                egui::accesskit::Role::Region,
            );
            y += 22.0 * scale;
        }
        summary.rows.push(("shortcuts".into(), "esc closes".into()));
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
    ///
    /// **Two things are named here, and they are not the same thing.** The strip is
    /// painter-drawn and gets a [`name_region`]; the buttons are real widgets and already
    /// have nodes, so they are *renamed* rather than joined — see [`name_node`].
    fn activity_bar(
        &mut self,
        ui: &mut egui::Ui,
        summary: &mut FrameSummary,
        rect: egui::Rect,
        scale: f32,
    ) {
        let theme = self.theme;
        ui.painter().rect_filled(rect, 4.0, theme.panel);
        ui.painter().rect_stroke(
            rect,
            4.0,
            egui::Stroke::new(1.0, theme.border),
            egui::StrokeKind::Inside,
        );
        // **Before the loop, because the loop is where a click lands.** `self.sidebar` is
        // reassigned under a press, and the selection this frame painted is the one that was
        // true when the loop started — so the name has to be read from the same value the
        // highlight was.
        name_region(
            ui,
            rect,
            ACTIVITY_SALT,
            &format!("Activity bar · showing {}", self.sidebar.label()),
            // `Navigation` is the ARIA landmark for the control that moves between
            // views. The activity bar is that control, and a rotor entry for it is
            // how a reader gets to the sidebar at all.
            egui::accesskit::Role::Navigation,
        );
        let mut y = rect.top() + 10.0;
        for view in SidebarView::ALL {
            let selected = view == self.sidebar;
            let button = ui
                .put(
                    egui::Rect::from_min_size(
                        egui::pos2(rect.left() + 6.0, y),
                        egui::vec2((rect.width() - 12.0).max(8.0), 30.0 * scale),
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
            // **The tooltip above does not reach a screen reader.** `on_hover_text` paints a
            // tooltip in a layer; egui 0.36 never writes it onto the accesskit node — there
            // is no `set_description` or `set_tooltip` call anywhere in the crate, although
            // `accesskit::Node` has both (`accesskit-0.24.1/src/lib.rs:1916`, `:1962`). So
            // the node `Button` built is named `"☰"`, and a reader heard the glyph. Naming
            // the node egui already built fixes the name and keeps the selected state that
            // `WidgetInfo::selected` put there (`egui-0.36.2/src/widgets/button.rs:393`).
            name_node(&button, view.label());
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
            y += 34.0 * scale;
        }
    }

    /// The title bar's control for folding and unfolding the sidebar.
    ///
    /// **In the title bar rather than on the sidebar, because the sidebar is the thing it
    /// hides.** A control that vanishes with what it controls cannot put it back, so every
    /// version of this that lived at the sidebar's top-right corner was a one-way door. The
    /// title bar is the one band `layout.rs` draws at every window above `COMPACT`, so the
    /// control and the way back are in the same place at every size.
    ///
    /// **A real `egui::Button`, so egui builds the node.** [`name_node`] then renames it —
    /// `widgets/button.rs:391` would otherwise leave the node called `◨`, which is the
    /// activity bar's exact defect. This is the arrangement ADR-022's second amendment
    /// settles: never a second node beside egui's, because the reader then hears the control
    /// twice.
    ///
    /// **The name carries the verb *and* the chord**, because those are the two facts a
    /// reader who cannot see the glyph needs and `on_hover_text` reaches neither of them
    /// (see `activity_bar`: egui 0.36 never writes a tooltip onto the node).
    fn sidebar_toggle(&mut self, ui: &mut egui::Ui, summary: &mut FrameSummary, title: egui::Rect) {
        if title.width() < SIDEBAR_TOGGLE_MIN_W {
            return;
        }
        let theme = self.theme;
        let shown = self.prefs.sidebar_visible;
        // **The chord comes from the table**, so a reader who rebinds it is told their own
        // chord here and in the status bar, not the one the source happens to carry.
        let chord = self
            .keys
            .label("window.sidebar.toggle")
            .unwrap_or_else(|| "ctrl+b".into());
        let rect = egui::Rect::from_center_size(
            egui::pos2(title.right() - SIDEBAR_TOGGLE_INSET, title.center().y),
            egui::vec2(SIDEBAR_TOGGLE_W, SIDEBAR_TOGGLE_W),
        );
        let button = ui
            .put(
                rect,
                egui::Button::new(if shown {
                    SIDEBAR_SHOWN_GLYPH
                } else {
                    SIDEBAR_HIDDEN_GLYPH
                })
                .selected(shown)
                .fill(if shown {
                    theme.selection_focus
                } else {
                    theme.panel
                }),
            )
            // Same arrangement as the activity bar's buttons: the tooltip reaches a
            // pointer, never a screen reader, which is why `name_node` below carries the
            // chord instead.
            .on_hover_text(format!(
                "{} ({chord})",
                if shown {
                    "Hide sidebar"
                } else {
                    "Show sidebar"
                }
            ));
        let action = if shown {
            "Hide sidebar"
        } else {
            "Show sidebar"
        };
        name_node(&button, &format!("{action} · {chord}"));
        summary
            .hotspots
            .push((SIDEBAR_TOGGLE_HOTSPOT.into(), button.rect));
        // **Its own region, not more `"title"`.** The title bar's rows are the line it draws,
        // and `the_title_bar_names_the_workspace_and_the_figures_it_drew` joins them to prove
        // the bar's accessible name is that line character for character. A control in that
        // list would make the bar name itself as its own text plus a button's label — which
        // is the "a name and the screen disagreeing" failure, produced by the test's own
        // honest reading.
        summary
            .rows
            .push(("sidebar-toggle".into(), action.to_string()));
        if button.clicked() {
            self.prefs.sidebar_visible = !self.prefs.sidebar_visible;
            self.persist_prefs();
        }
    }

    /// The strip on the sidebar's right edge, and the drag that resizes it.
    ///
    /// **egui's own gesture, not one invented here.** `Ui::interact(…, Sense::drag())` plus
    /// [`egui::Response::drag_delta`] is what `egui::containers::Resize` does for its own
    /// corner (`egui-0.36.2/src/containers/resize.rs:376-401`), so this behaves the way the
    /// same gesture behaves in every other egui window.
    ///
    /// **`Sense::drag()` rather than the bare `Sense::DRAG`, and that one word is what makes
    /// the handle reachable.** `Sense::drag()` is `DRAG | FOCUSABLE`
    /// (`egui-0.36.2/src/sense.rs:64-70`), and egui builds an AccessKit node for every
    /// focusable widget (`egui-0.36.2/src/context.rs:1292-1297`). So [`name_node`] has a node
    /// to write a name into — one field, and no duplicate `OutputEvent` queued the way a
    /// `name_region` beside it would. An unnamed drag target is a control a screen reader
    /// steps over silently, which is worse than not drawing one.
    ///
    /// **Nothing is painted at rest.** The handle is the gutter the carve already left, and
    /// filling it permanently would put a 6-point bar between two panes that nobody asked
    /// for. It appears on hover or drag, in `border_focus` — which [`IdeTheme`] holds to the
    /// 3:1 state floor, so the thing that says "this is live" is not the same colour as the
    /// thing that says "this is a pane".
    fn sidebar_resize(&mut self, ui: &egui::Ui, summary: &mut FrameSummary) {
        let handle = self.layout.sidebar_handle;
        if handle.width() <= 0.0 {
            return;
        }
        let theme = self.theme;
        let response = ui.interact(
            handle,
            ui.make_persistent_id(SIDEBAR_HANDLE_SALT),
            egui::Sense::drag(),
        );
        let live = response.hovered() || response.dragged();
        if live {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
            ui.painter().rect_filled(handle, 0.0, theme.border_focus);
        }
        if response.dragged() {
            // **Applied every frame of the drag and clamped through the one rule**
            // (`Preferences::sidebar_width_clamped` does the same clamp for the layout, the
            // file and the keyboard), so a drag past either end stops at the end rather than
            // producing a sidebar the next frame has to un-draw.
            self.prefs.sidebar_width = (self.prefs.sidebar_width + response.drag_delta().x)
                .clamp(SIDEBAR_MIN, SIDEBAR_MAX);
        }
        if response.drag_stopped() {
            // **At the end of the gesture, not during it** — see `persist_prefs`.
            self.persist_prefs();
        }
        // The width is the *drawn* width, not the stored one: after a clamp at either end
        // those are different numbers, and a name that said 9999 beside a 420-point pane is
        // the "a name and the screen disagreeing" failure ADR-022 is about.
        name_node(
            &response,
            &format!(
                "Resize sidebar · {} points · drag, or use the left and right arrow keys",
                self.prefs.sidebar_width_clamped()
            ),
        );
        // **The role, which the name alone did not supply.** The node had a name and still
        // announced as `Role::Unknown`, because egui builds the node for a focusable widget
        // without a role and nothing had ever set one (`egui-0.36.2/src/context.rs:1292-1297`,
        // `response.rs:908-924`). A reader that trusts the role first has nothing to announce.
        //
        // `Role::Splitter` (`accesskit-0.24.1/src/lib.rs:189`) is precisely what this is: a
        // separator the user drags to resize one side. `Adjustment` was the alternative and is
        // wrong — that role is for a *value* being adjusted (a slider or spin box), and this
        // control adjusts the width of a pane rather than a setting.
        name_role(&response, egui::accesskit::Role::Splitter);
        summary
            .hotspots
            .push((SIDEBAR_HANDLE_HOTSPOT.into(), handle));
    }

    /// The workspace pane: whichever view the activity bar selected.
    fn workspace_pane(
        &mut self,
        ui: &mut egui::Ui,
        summary: &mut FrameSummary,
        rect: egui::Rect,
        scale: f32,
    ) {
        match self.sidebar {
            SidebarView::Files => self.files_pane(ui, summary, rect, scale),
            SidebarView::Tasks => self.tasks_pane(ui, summary, rect, scale),
            SidebarView::Sessions => self.sessions_pane(ui, summary, rect, scale),
        }
    }
    /// The sidebar's tasks view: every subagent this session has dispatched.
    ///
    /// Read from [`IdeSession`]'s agent rather than kept here, because the agent owns the
    /// registry and a copy would go stale the moment a child finished — which is exactly
    /// when a reader is looking.
    fn tasks_pane(
        &self,
        ui: &mut egui::Ui,
        summary: &mut FrameSummary,
        rect: egui::Rect,
        scale: f32,
    ) {
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
            paint_text(
                painter,
                summary,
                "tasks",
                rect.left_top() + egui::vec2(12.0, 16.0),
                egui::Align2::LEFT_CENTER,
                "Subagents",
                egui::FontId::proportional(12.0 * scale),
                theme.dim,
            );
        }
        summary.rows.push(("tasks".into(), "Subagents".into()));
        // **Move the cursor below the header (M312).** The header is painted at an absolute
        // position (`rect.left_top() + (12, 16)`), which does not move the `Ui`'s cursor — so
        // the first `colored_label` below lands at the **pane's top**, over the header. The
        // rendered frame showed it exactly that way: `none stored for this workspace yet`
        // printed *above* the word `Sessions`, both clipped by the pane edge.
        //
        // This is the same defect the transcript had (see `draw_rows`'s origin), found by
        // rendering this window and looking at it rather than by any test: every assertion here
        // reads which **strings** were drawn, and these strings were drawn, just in the wrong
        // place. Three panes, one rule — **a pane that paints its header absolutely must move
        // the cursor under it.**
        ui.add_space(PANE_HEADER_H * scale);
        record_content_offset(ui, summary, rect);
        let tasks = self
            .session
            .as_ref()
            .map(|s| s.agent().task_snapshots())
            .unwrap_or_default();
        // Named from the snapshot this pane is about to draw, not from a second read of
        // the agent. "0 tasks" is also the right answer when no agent is attached, which
        // is the state below: a name that said otherwise would have to be wrong about
        // something to stay truthful about something else.
        let count = tasks.len();
        name_region(
            ui,
            rect,
            TASKS_SALT,
            &format!("Subagents · {count} {}", Self::plural(count)),
            egui::accesskit::Role::Region,
        );
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
                let text = fit(&text, rect.width() - 16.0, ROW_FONT * scale);
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
    fn sessions_pane(
        &mut self,
        ui: &mut egui::Ui,
        summary: &mut FrameSummary,
        rect: egui::Rect,
        scale: f32,
    ) {
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
            paint_text(
                painter,
                summary,
                "sessions",
                rect.left_top() + egui::vec2(12.0, 16.0),
                egui::Align2::LEFT_CENTER,
                "Sessions",
                egui::FontId::proportional(12.0 * scale),
                theme.dim,
            );
        }
        summary.rows.push(("sessions".into(), "Sessions".into()));
        // **Under the header, not over it (M312)** — see the same line in `tasks_pane`. The
        // header is painted absolutely and the cursor is still at the pane's top, so the empty
        // state printed above the pane's own title.
        ui.add_space(PANE_HEADER_H * scale);
        record_content_offset(ui, summary, rect);
        let Some(session) = self.session.as_ref() else {
            // The pane is on screen with no list under it, and "Sessions" alone would
            // leave a reader looking for something that is not coming.
            name_region(
                ui,
                rect,
                SESSIONS_SALT,
                "Sessions · no agent attached",
                egui::accesskit::Role::Region,
            );
            let line = "no agent attached";
            ui.colored_label(theme.dim, line);
            summary.rows.push(("sessions".into(), line.to_string()));
            return;
        };
        let current = session.session_path();
        let sessions = session.sessions(&self.opts.workspace);
        // Counted here, above the empty check, so "none stored" is named as the empty
        // list it is rather than as a pane that failed to draw.
        let stored = sessions.len();
        name_region(
            ui,
            rect,
            SESSIONS_SALT,
            &format!("Sessions · {stored} stored"),
            egui::accesskit::Role::Region,
        );
        // **New session, beside the list it extends (L-113).** The reference puts
        // it on the brand row; this window has none, so it lives where the
        // sessions are. A real button, not painted text: Tab reaches it, a
        // screen reader names it, and the click runs the same `run_new_session`
        // the `/new` head runs — one act, two doors.
        // **The chord comes from the table (L-114)**, so a reader who rebinds
        // it is told their own chord here, not the default below: the tooltip
        // reaches a pointer, and the renamed node reaches a screen reader
        // (egui never writes a tooltip onto the node).
        let session_chord = self
            .keys
            .label("window.session.new")
            .unwrap_or_else(|| "ctrl+n".into());
        let new = ui
            .button("New session")
            .on_hover_text(format!("New session ({session_chord})"));
        name_node(&new, &format!("New session · {session_chord}"));
        summary.row_rects.push(("sessions-new".into(), new.rect));
        if new.clicked() {
            let outcome = self.run_new_session();
            self.notice_outcome(outcome);
        }
        if sessions.is_empty() {
            let line = "none stored for this workspace yet";
            ui.colored_label(theme.dim, line);
            summary.rows.push(("sessions".into(), line.to_string()));
            return;
        }
        let header = format!("{} stored", sessions.len());
        ui.colored_label(
            theme.dim,
            fit(&header, rect.width() - 16.0, ROW_FONT * scale),
        );
        summary.rows.push(("sessions".into(), header));
        for entry in &sessions {
            let is_current = current.as_deref() == Some(entry.path.as_path());
            // **Two markers, and they mean different things.** `●` is the session this
            // window is already in — a fact about the *session*. `▸` is where the
            // *keyboard* is, which is a fact about the *reader*. Drawing only the first
            // is what let a keyboard user move a cursor they could not see; drawing only
            // the second would hide which conversation is open.
            let on_cursor = self.session_cursor.as_deref() == Some(entry.path.as_path());
            let marker = match (is_current, on_cursor) {
                (true, true) => "●▸ ",
                (true, false) => "●  ",
                (false, true) => "  ▸ ",
                (false, false) => "   ",
            };
            // Name first, timestamp second: the name is what a reader recognises, and
            // the timestamp is what they check afterwards.
            let name = entry
                .path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| entry.path.display().to_string());
            let shown = fit(
                &format!("{marker}{name}"),
                rect.width() - 16.0,
                ROW_FONT * scale,
            );
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
        // **The second of the two places these figures can change.** `live_resume` replaces
        // the whole history and emits nothing, so this is exactly the case `pump`'s
        // `arrived > 0` cannot see — and a window that kept the old numbers would show the
        // session it just left: its context size, its token totals and its bill.
        self.refresh_metrics();
        Ok(count)
    }
    /// The file tree.
    ///
    /// Drawn with widgets rather than as text rows, so it can take clicks and a keyboard
    /// — a tree made of painted text is a picture of a tree.
    fn files_pane(
        &mut self,
        ui: &mut egui::Ui,
        summary: &mut FrameSummary,
        rect: egui::Rect,
        scale: f32,
    ) {
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
            paint_text(
                painter,
                summary,
                "workspace",
                rect.left_top() + egui::vec2(12.0, 16.0),
                egui::Align2::LEFT_CENTER,
                "Workspace",
                egui::FontId::proportional(12.0 * scale),
                theme.dim,
            );
        }
        summary.rows.push(("workspace".into(), "Workspace".into()));
        // **The tree's own count, and the word that says whether it is all of them.**
        //
        // `row_count` and `is_truncated` are the two the footer's note is built from a few
        // lines below, so the name and the drawing cannot disagree — and a truncated walk
        // named as "12 files" would claim a completeness the pane spends a whole line
        // denying.
        let listed = self.tree.row_count();
        // **Pluralised, because reading the tree is how this was found.** A screen reader
        // announces this string verbatim, and the tree dump on 2026-10-04 read
        // `Workspace files · 1 files` — a grammar bug in the one sentence this crate tells a
        // non-sighted user about the pane they are in. No assertion was looking at the wording.
        let files = if listed == 1 { "file" } else { "files" };
        let files_name = if self.tree.is_truncated() {
            format!("Workspace files · {listed} or more, list is partial")
        } else {
            format!("Workspace files · {listed} {files}")
        };
        name_region(
            ui,
            rect,
            FILES_SALT,
            &files_name,
            egui::accesskit::Role::Region,
        );
        summary.rows.push((
            "workspace".into(),
            format!("workspace  {}", display_path(&self.opts.workspace)),
        ));
        let search = egui::Rect::from_min_size(
            egui::pos2(rect.left() + 8.0, rect.top() + 28.0),
            egui::vec2((rect.width() - 16.0).max(0.0), 24.0 * scale),
        );
        if search.width() > 60.0 {
            let filter = ui.put(
                search,
                egui::TextEdit::singleline(&mut self.tree_query)
                    .hint_text("filter files")
                    .desired_width(search.width() - 8.0),
            );
            // **Named, because the tree dump on 2026-10-04 showed this input unnamed.**
            // `WidgetInfo::text_edit` fills value and hint and never `label`
            // (`egui-0.36.2/src/data/output.rs:697`), so a `TextEdit` reaches a screen reader
            // as a bare field with nothing saying what it filters. `name_node` writes the one
            // field egui leaves empty and queues no `OutputEvent`.
            name_node(
                &filter,
                &format!(
                    "Filter files in the workspace tree ({})",
                    self.tree.row_count()
                ),
            );
            // Recorded so a test can put the keyboard here. This is the window's second
            // text input, and until the composer stopped stealing focus back there was
            // no way to prove — or for a user to get — the keyboard landing here.
            summary.hotspots.push(("file-filter".into(), filter.rect));
        }
        let body = egui::Rect::from_min_max(
            egui::pos2(rect.left() + 8.0, search.max.y + 6.0),
            egui::pos2(rect.right() - 8.0, rect.bottom() - 26.0 * scale),
        );
        if body.height() > 10.0 && body.width() > 20.0 {
            let selected = self.open.as_ref().map(|f| f.path.as_str());
            let cursor = self.tree_cursor.as_deref();
            let mark = crate::workspace::TreeMark { selected, cursor };
            match crate::workspace::draw_tree(
                ui,
                &theme,
                body,
                &self.tree,
                &self.tree_query,
                mark,
                summary,
                scale,
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
        paint_text(
            painter,
            summary,
            "workspace",
            egui::pos2(rect.left() + 12.0, rect.bottom() - 14.0 * scale),
            egui::Align2::LEFT_BOTTOM,
            &note,
            egui::FontId::proportional(11.0 * scale),
            theme.dim,
        );
        summary.rows.push(("workspace".into(), note));
    }
    /// The editor pane: the open file, read-only — or the change being reviewed.
    fn editor_pane(
        &mut self,
        ui: &mut egui::Ui,
        summary: &mut FrameSummary,
        rect: egui::Rect,
        scale: f32,
    ) {
        let theme = self.theme;
        // **A tab press is applied before the strip is drawn**, against the rectangles it
        // occupied last frame. Applied afterwards — which is where egui's own answer lands,
        // because that answer only exists once the widget has been drawn — the reader got
        // the file while the marker still named the old one, in the same frame.
        if let Some(path) = self.pressed_tab(ui) {
            self.open_file(&path);
        }
        self.draw_tabs(ui, summary, rect, scale);
        let body = egui::Rect::from_min_max(
            egui::pos2(rect.left(), rect.top() + TAB_H * scale),
            egui::pos2(rect.right(), rect.bottom()),
        );
        if let Some(change) = self.review.clone() {
            crate::workspace::draw_diff(ui, &theme, body, &change, Some(summary), scale);
            // A diff is not a file, and a reader who could not tell them apart would read a
            // review as the document. `Change::summary` is the same one-line string the
            // transcript's "changed" row carries, so the two panes cannot disagree.
            name_region(
                ui,
                body,
                EDITOR_SALT,
                &format!("Change review · {}", change.summary()),
                egui::accesskit::Role::Region,
            );
            return;
        }
        match self.open.clone() {
            Some(file) => {
                crate::workspace::draw_editor(ui, &theme, body, &file, Some(summary), scale);
                name_region(
                    ui,
                    body,
                    EDITOR_SALT,
                    &format!("Editor, read-only · {} · {} lines", file.path, file.len()),
                    egui::accesskit::Role::Region,
                );
                summary.rows.push((
                    "editor".into(),
                    format!("{} · {} lines · read-only", file.path, file.len()),
                ));
                summary.rows.push(("editor".into(), "read-only".into()));
            }
            None => {
                name_region(
                    ui,
                    body,
                    EDITOR_SALT,
                    "Editor, read-only · no file open",
                    egui::accesskit::Role::Region,
                );
                let painter = ui.painter();
                painter.rect_filled(body, 4.0, theme.background);
                painter.rect_stroke(
                    body,
                    4.0,
                    egui::Stroke::new(1.0, theme.border),
                    egui::StrokeKind::Inside,
                );
                // The empty state says what to do, not merely that there is nothing.
                paint_text(
                    painter,
                    summary,
                    "editor",
                    body.center(),
                    egui::Align2::CENTER_CENTER,
                    "Pick a file on the left to read it",
                    egui::FontId::proportional(13.0 * scale),
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
    /// A tab strip drawn as painted text is a picture of a tab strip, so these are buttons.
    /// The strip **records where each tab went** and the click is applied by
    /// [`IdeApp::pressed_tab`] *before* this runs — egui only knows a button was pressed once
    /// it has been drawn, so a strip that reacted to its own answer could not also redraw
    /// itself correctly in that frame.
    ///
    /// One file means one tab and no strip: a single tab with nothing to switch to is
    /// chrome that costs a row and says nothing.
    fn draw_tabs(
        &mut self,
        ui: &mut egui::Ui,
        summary: &mut FrameSummary,
        rect: egui::Rect,
        scale: f32,
    ) {
        self.tab_rects.clear();
        if self.tabs.len() < 2 {
            return;
        }
        let theme = self.theme;
        let painter = ui.painter();
        let strip = egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), TAB_H * scale));
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
        let width = ((strip.width() - 12.0) / count - 4.0).clamp(8.0, TAB_W * scale);
        for (index, path) in self.tabs.clone().into_iter().enumerate() {
            let selected = index == self.tabs.len() - 1;
            let name = path.rsplit('/').next().unwrap_or(&path).to_string();
            // The file stem, not the whole name: `layout.rs` reads as a name and
            // `layout.rs.modified.rs` does not.
            let stem = name.rsplit_once('.').map(|(s, _)| s).unwrap_or(&name);
            // Shortened, never clipped: a narrow tab says so with an ellipsis, and the full
            // path is in the editor's own header underneath either way.
            let label = fit(stem, width - 10.0, TAB_FONT * scale);
            let tab_rect = egui::Rect::from_min_size(
                egui::pos2(
                    strip.left() + 6.0 + index as f32 * (width + 4.0),
                    strip.top() + 4.0,
                ),
                egui::vec2(width, TAB_H * scale - 9.0),
            );
            let response = ui.put(tab_rect, egui::Button::new(&label).selected(selected));
            // **The label as it was cut, not the stem it came from.** `summary.tabs` carries
            // the stem because a test asks "which file is this tab"; nothing carried what the
            // reader can actually see, and the tab is the one place this window shortens text
            // against a width the reader chose. `rows` is the record of what the frame says,
            // and a label no test can read is a label whose budget cannot be checked.
            summary.rows.push(("tabs".into(), label.clone()));
            // `tab:` rather than the bare path id: the file tree records the **same** id for the
            // same path, and `FrameSummary::hotspot` returns the first match. A test
            // asking for "alpha.rs" would get the tree row and click that, never the tab
            // it meant to press.
            summary
                .hotspots
                .push((format!("tab:{path}"), response.rect));
            summary.tabs.push((stem.to_string(), selected));
            self.tab_rects.push((path, response.rect));
        }
    }

    /// The tab the pointer clicked, resolved against where the tabs were drawn last frame.
    ///
    /// Same data as `FrameSummary::hotspots`, opposite order — and the order is the whole
    /// of it. M248 read the frame's hotspots *after* the pane that fills them and so always
    /// got an empty answer; here the reading happens *before* the strip is drawn, so the
    /// file is open and the marker already agrees by the time it is drawn.
    fn pressed_tab(&self, ui: &egui::Ui) -> Option<String> {
        let (clicked, at) = ui.input(|i| (i.pointer.any_click(), i.pointer.interact_pos()));
        // No pointer position means no hit test is possible, which is the same answer as
        // "not over a tab" — said once here rather than at both ends.
        let at = at?;
        if !clicked {
            return None;
        }
        self.tab_rects
            .iter()
            .find(|(_, r)| r.contains(at))
            .map(|(p, _)| p.clone())
    }
    /// Open `path` in the editor, or say why it could not be opened.
    ///
    /// The failure is shown in the transcript rather than swallowed: a click that appears
    /// to do nothing is the state this repository keeps failing to rule out.
    fn open_file(&mut self, path: &str) {
        // Opening a file ends a review: the reader asked to read, and leaving a diff on
        // screen because they opened the file to read it would be the window ignoring its
        // own click. The tree and the tab strip are therefore the way back from a diff,
        // which is why this pane needs no separate close control.
        self.review = None;
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
    /// What this window is doing, in the three words it is built from.
    ///
    /// **One function because two spellings would drift.** The status bar draws this string
    /// and [`Self::paint`] publishes it as the bar's accessible name, and a bar that reads
    /// "idle" above a node named "running" is the exact disagreement this window cannot
    /// have: for a reader who cannot see the bar, the node is the only account of it there
    /// is.
    /// `pub(crate)` because `/session` reports the same three words (M312) — the same rule that
    /// makes this the one function both places call, rather than a second copy of "is a turn in
    /// flight" written beside it.
    pub(crate) fn run_state(&self) -> &'static str {
        match self.session.as_ref() {
            None => "no agent attached",
            Some(s) if s.running() => "running",
            Some(_) => "idle",
        }
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
        parts.push(self.run_state().into());
        if let Some(refusal) = &self.refusal {
            parts.push(refusal.clone());
        }
        if self.dropped_events > 0 {
            // **The window's own copy, not the transcript's (M312).** This segment was drawn from
            // `self.transcript.dropped()`, whose only writer (`Transcript::note_lag`) nothing in
            // production called — so the count could only ever be zero and the segment could never
            // appear. A status bar that says a thing it can never say is worse than one that says
            // less, and the fix is to make the number real rather than to remove the sentence.
            parts.push(format!("{} event(s) dropped", self.dropped_events));
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
    ///
    /// Every block names itself [`AGENT_ROW_NAME`] and its own text, so the blocks of one
    /// answer are told apart rather than announced identically: a six-block answer is six
    /// nodes, and a reader who heard the same name six times would have learned nothing
    /// from any of them.
    ///
    /// **One argument over the clippy budget, and it is `scale`.** Every other parameter
    /// was here before text scaling; the eighth is the number every block's
    /// [`crate::markup::job_at`] needs, and reading it from the `Context` inside would
    /// give the measuring pass and the drawing pass two chances to disagree.
    #[allow(clippy::too_many_arguments)]
    fn draw_markdown(
        &self,
        ui: &mut egui::Ui,
        summary: &mut FrameSummary,
        marker: &str,
        color: egui::Color32,
        text: &str,
        measure: bool,
        scale: f32,
    ) {
        let theme = self.theme;
        // The borrow ends on this line — `blocks` is an `Rc`, not a reference into the map —
        // so nothing is held while the blocks are drawn.
        let blocks = self.markdown.borrow_mut().blocks(text);
        // A renderer that produced nothing for a non-empty answer would leave a gap in
        // the transcript, so fall back to the raw text rather than to nothing.
        if blocks.is_empty() {
            let line = format!("{marker}{text}");
            Self::row_label(
                ui,
                measure,
                egui::RichText::from(line.clone()).color(color).into(),
                &row_name(AGENT_ROW_NAME, &line),
            );
            summary.rows.push(("agent".into(), line));
            return;
        }
        for (i, block) in blocks.iter().enumerate() {
            let prefix = if i == 0 { marker } else { "  " };
            match block {
                crate::markup::Block::Heading { runs, .. } => {
                    let job = crate::markup::job_at(runs, theme.title, theme.code, scale);
                    let line = format!("{prefix}{}", plain_of(runs));
                    Self::record_block(ui, summary, egui::WidgetText::from(job), line, measure);
                }
                crate::markup::Block::Paragraph(runs) => {
                    let job = crate::markup::job_at(runs, color, theme.code, scale);
                    let line = format!("{prefix}{}", plain_of(runs));
                    Self::record_block(ui, summary, egui::WidgetText::from(job), line, measure);
                }
                crate::markup::Block::ListItem {
                    marker: bullet,
                    runs,
                } => {
                    let job = crate::markup::job_at(runs, color, theme.code, scale);
                    let line = format!("{prefix}{bullet}{}", plain_of(runs));
                    Self::record_block(ui, summary, egui::WidgetText::from(job), line, measure);
                }
                crate::markup::Block::Quote(runs) => {
                    let job = crate::markup::job_at(runs, theme.dim, theme.code, scale);
                    let line = format!("{prefix}❯ {}", plain_of(runs));
                    Self::record_block(ui, summary, egui::WidgetText::from(job), line, measure);
                }
                crate::markup::Block::Code { language, lines } => {
                    let tag = language
                        .as_deref()
                        .map(|l| format!("[{l}]"))
                        .unwrap_or_else(|| "[code]".to_string());
                    for line in lines {
                        let shown = format!("{prefix}{tag} {line}");
                        Self::row_label(
                            ui,
                            measure,
                            egui::RichText::from(shown.clone()).color(theme.code).into(),
                            &row_name(AGENT_ROW_NAME, &shown),
                        );
                        summary.rows.push(("agent".into(), shown));
                    }
                }
                crate::markup::Block::Rule => {
                    let rule = format!("{prefix}--------");
                    Self::row_label(
                        ui,
                        measure,
                        egui::RichText::from(rule.clone())
                            .color(theme.border)
                            .into(),
                        // **A rule is structure, not content.** Eight dashes is a picture of
                        // a divider; a name that reads "--------" identifies nothing in a
                        // conversation, so it is named as what it is and nothing more.
                        &row_name(&format!("{AGENT_ROW_NAME} · rule"), ""),
                    );
                    summary.rows.push(("agent".into(), rule));
                }
            }
        }
    }
    /// One transcript label, **measured** or drawn.
    ///
    /// This is the only place the two modes diverge, and it is one line wide on purpose. A
    /// transcript row's height is only knowable by laying the row out, so the frame lays
    /// out every row once ([`RowLayout`]) and then draws the two dozen that fit. If the
    /// measuring path used a *different* widget than the drawing path, the offsets would be
    /// wrong and the symptom would be rows overlapping each other — which is why both
    /// paths build the same [`egui::Label`] and differ only in what happens after the
    /// layout.
    ///
    /// The hook is [`egui::Label::layout_in_ui`]
    /// (`egui-0.36.2/src/widgets/label.rs:140`), documented as "do layout and position the
    /// galley in the ui, **without painting it or adding widget info**". That last clause is
    /// the whole reason it is the right one: a probe that painted would put 800 off-screen
    /// answers into the glyph atlas, and a probe that registered widget info would put 800
    /// nodes into the accessibility tree for rows no reader can reach — which is the
    /// opposite of what virtualising for a screen reader is for.
    ///
    /// # The name, and why `measure` is the gate on it
    ///
    /// `measure` **is** the virtualisation, and it is what makes per-row naming affordable
    /// at all. A row that is only being laid out takes the `layout_in_ui` path, which
    /// registers no node whatsoever, so [`Self::measure_rows`]'s probe publishes nothing
    /// for the ~760 rows a reader cannot see. Naming rows anywhere else — over the whole
    /// transcript, in [`Self::draw_rows`] — would put one node per row back into every
    /// frame, which is exactly the cost M285 removed.
    ///
    /// # Why [`name_node`] and not [`egui::Response::widget_info`]
    ///
    /// **Because `widget_info` cannot name this node at all.** egui has *already* built it:
    /// `Label::ui` calls `response.widget_info(…)` itself
    /// (`egui-0.36.2/src/widgets/label.rs:282`), and on a `Role::Label` node egui writes
    /// a `WidgetInfo`'s `label` into **`Node::value`, not `Node::label`**
    /// (`egui-0.36.2/src/response.rs:962-964`). Passing a name through `widget_info` would
    /// therefore overwrite the row's own text *and* still leave the accessible name empty.
    /// [`name_node`] writes the one field that is missing and leaves the role and the value
    /// exactly as egui built them — which is also why it queues no `OutputEvent` on a frame
    /// where the row was clicked (`egui-0.36.2/src/response.rs:872-887`).
    fn row_label(
        ui: &mut egui::Ui,
        measure: bool,
        text: egui::WidgetText,
        name: &str,
    ) -> egui::Response {
        if measure {
            egui::Label::new(text)
                .sense(egui::Sense::empty())
                .layout_in_ui(ui)
                .2
        } else {
            let response = ui.label(text);
            name_node(&response, name);
            response
        }
    }
    /// Draw one markdown block, **wrapped**, and record where it landed.
    ///
    /// Draw one markdown block and record **where** it landed.
    ///
    /// # The `.wrap()` that is not here
    ///
    /// A review reported that assistant prose never wraps — that `ui.label(WidgetText)` lays out
    /// on one line and clips at the pane edge — and it was the worst of the desktop's UI findings.
    /// **Measured, and it is false.** The same 12-sentence paragraph occupies **135 px with and
    /// without `.wrap()`**; a 2-sentence one occupies 30 px either way. egui 0.36's `Label` built
    /// from a `LayoutJob` wraps already, so the `.wrap()` this function first carried was a no-op
    /// that only looked like a fix.
    ///
    /// It is recorded here because **the measurement is the deliverable**: a predicted defect that
    /// turns out not to exist still has to be measured, or "we looked at it" is indistinguishable
    /// from "it was fixed".
    ///
    /// # Why the rect is recorded
    ///
    /// `row_rects` is the only record of **where** anything was painted, and the containment sweep
    /// in `app.rs` iterates it. Assistant text was pushed to `summary.rows` and never to
    /// `row_rects`, so **the window's primary content sat outside every containment check** — and no
    /// wrap-or-clip question could be asked of it at all, which is how the false finding above
    /// survived a careful reading. Recording it is what made the measurement possible.
    /// # Why the name is built here rather than in `draw_row`
    ///
    /// An assistant answer is **several rows on screen**, not one: one per markdown block,
    /// one per line of a code block. `recorded` is that block's own plain text, so it is
    /// what makes each block's node tell the reader *which paragraph* it reached — a single
    /// row-level name repeated across a six-block answer is six nodes a reader hears
    /// announce the same words for. The kind is [`AGENT_ROW_NAME`] because this function is
    /// reached from the assistant row and from no other.
    fn record_block(
        ui: &mut egui::Ui,
        summary: &mut FrameSummary,
        text: egui::WidgetText,
        recorded: String,
        measure: bool,
    ) {
        let response = Self::row_label(ui, measure, text, &row_name(AGENT_ROW_NAME, &recorded));
        summary.rows.push(("agent".into(), recorded));
        summary.row_rects.push(("agent".into(), response.rect));
    }

    /// The one line this window draws for a pending ask, and whether it can be approved at all.
    ///
    /// Extracted from the draw path for one reason: **an ask carrying a display-steering
    /// character cannot be produced by the product's own tools**, so no end-to-end frame test can
    /// reach the branch that matters. A pure function can be, and the rule is the part worth
    /// pinning — the terminal has refused to approve such an ask since M261 and this window had no
    /// check at all until M272, so the same bytes were safe in one frontend and not the other.
    ///
    /// Three things are decided here, and each was a separate defect:
    ///
    /// 1. **The request, not the pattern.** `ask.pattern` is the first whitespace token — the
    ///    thing a *rule* matches on — so it asked `allow bash echo ?` for a longer command
    ///    (M271).
    /// 2. **Stripped**, because this screen is where the decision is made. A `U+202E` reverses the
    ///    visual order of everything after it, so a command can be *displayed* as one thing and
    ///    *be* another.
    /// 3. **Says so, rather than cleaning and moving on.** On a transcript row a cleaned line is
    ///    fine — it is a report. On the screen a reader approves with, a cleaned line still means
    ///    agreeing to something not fully seen, so the terminal takes the keys away and this
    ///    window takes the button. **Refusing is not a wedge**: the caller still draws `Refuse`.
    ///
    /// The budget is this window's own. The *rule* — which field answers the question, and whether
    /// the ask can be shown — lives in `epsilon-runtime` because it is the same in both windows;
    /// the width is not, because a 96-column terminal row and a 520-point pane are different
    /// measurements and sharing one would be wrong in at least one of them (M271).
    pub(crate) fn ask_line(
        ask: &epsilon_contracts::agent_event::PermissionRequest,
        budget: f32,
        scale: f32,
    ) -> (String, bool) {
        let request = epsilon_runtime::summarize_request(&ask.detail);
        let (safe_what, what_hit) = epsilon_runtime::text_safety::display_safe(&request);
        let honest = epsilon_runtime::ask_is_display_honest(ask);
        // The budget is measured against the size the line is **drawn** at, which is the
        // scaled one — a 12-point ask in a 24-point world keeps the same characters on
        // screen only if it is measured against 24.
        let what = fit(&safe_what, budget, 12.0 * scale);
        let text = if honest {
            format!(
                "allow {} {}?   [enter] allow   [esc] refuse",
                ask.capability, what
            )
        } else {
            format!(
                "allow {} {}?   this request carries {} display-steering character(s), \
             it cannot be approved as shown   [esc] refuse",
                ask.capability,
                what,
                what_hit.total().max(1)
            )
        };
        (text, honest)
    }

    /// Text this window may draw, stripped of anything that steers the display.
    ///
    /// # Why a helper rather than the shared rule
    ///
    /// The *rule* is shared — `epsilon_runtime::text_safety::display_safe` is the one
    /// implementation, which M272 moved there precisely so the two frontends cannot disagree. What
    /// differs is the **policy around it**, and it is the same policy M261/M274 established for
    /// their surfaces: **strip everywhere, refuse only where a reader decides.**
    ///
    /// This window has exactly one such place — the permission prompt, which refuses outright
    /// because a cleaned line there still means agreeing to something not fully seen. Every other
    /// surface here is **display only**: what it hands back comes from the row's own field, not
    /// from the string it draws.
    ///
    /// # Why it borrows
    ///
    /// `display_safe` returns a `Cow` and documents the borrowed arm as "the point", because a
    /// transcript of two thousand rows must not allocate two thousand `String`s per frame to learn
    /// that nothing needed removing. **This is drawn every frame**, and M275 measured that frame
    /// at 104 ms for 800 rows — so an `into_owned()` here would buy back exactly the cost the
    /// borrow was there to avoid, on the one surface that calls this most often.
    ///
    /// # Why the count is not reported
    ///
    /// The terminal records it (`TranscriptLine::hidden`) because a transcript line has a field for
    /// it. **A frame summary does not**, and inventing one for a value that cannot cause harm is
    /// the noise this repository keeps arguing against. Recorded as a gap in the rule's reach, not
    /// a decision to skip it.
    fn safe_to_show(text: &str) -> std::borrow::Cow<'_, str> {
        epsilon_runtime::text_safety::display_safe(text).0
    }

    /// The agent panel: status, transcript, approvals, composer.
    ///
    /// Split into three regions inside one pane, and the split is decided here rather
    /// than by a nested layout because the composer must stay on screen while a long
    /// answer scrolls: a composer that scrolls away is a window that cannot answer a
    /// question it just asked.
    fn agent_panel(
        &mut self,
        ui: &mut egui::Ui,
        summary: &mut FrameSummary,
        rect: egui::Rect,
        scale: f32,
    ) {
        // Both of these are rectangles that exist **only** to hold text — the composer the
        // reader types into and the notice stack above the transcript. They scale with the
        // text in them, or a scaled window draws its transcript into a composer it has
        // outgrown. **They do not move any pane-folding threshold**: `layout.rs` folds on the
        // content rect alone (`IdeLayout::new`, which never sees a font size), and this
        // module does not touch that file.
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
            paint_text(
                painter,
                summary,
                "agent",
                rect.left_top() + egui::vec2(12.0, 16.0),
                egui::Align2::LEFT_CENTER,
                "Agent",
                egui::FontId::proportional(12.0 * scale),
                theme.dim,
            );
        }
        summary.rows.push(("agent".into(), "Agent".into()));
        // ---- header: what the launch said ------------------------------------
        let mut y = rect.top() + HEADER_H * scale;
        {
            let painter = ui.painter();
            for notice in &self.notices {
                if y > rect.bottom() - COMPOSER_H * scale {
                    break;
                }
                // **Budgeted, and the summary carries the fitted text, not the raw one.**
                // This notice stack was the last unbounded string in the window, and
                // looking at a rendered frame is the only way it shows: a real launch
                // notice — `no credentials for provider "openrouter" — set
                // OPENROUTER_API_KEY or run: epsilon auth login openrouter` — ran off
                // the right edge of the pane and stopped mid-word with **no marker**,
                // while every test still passed, because they assert on `summary.rows`
                // and this pushed the *unclipped* text.
                //
                // That is the trap this repository has been caught by repeatedly: code
                // that draws can be deleted while the row it pushed stays. `fit`
                // appends `…` and the summary records what was actually drawn, so the
                // two cannot disagree.
                let shown = fit(notice, rect.width() - 24.0, 12.0 * scale);
                paint_text(
                    painter,
                    summary,
                    "agent",
                    egui::pos2(rect.left() + 12.0, y),
                    egui::Align2::LEFT_TOP,
                    &shown,
                    egui::FontId::proportional(12.0 * scale),
                    theme.dim,
                );
                y += 17.0 * scale;
                // **The row is recorded here, not by `paint_text`** — which only pushes a
                // text *size* (`app.rs:1391`). A first version of this fix dropped this
                // line while replacing it with the fitted text, and the test failed with
                // an agent pane that had drawn nothing at all: the drawing and the record
                // are two separate acts, and this one is the record.
                summary.rows.push(("agent".into(), shown));
            }
        }
        // ---- goal banner ----------------------------------------------------
        // **A goal the window cannot see is a goal the window cannot serve.** Until this,
        // `epsilon-desktop` had no notion of a goal at all: a reader who set one in the
        // terminal, or drove an unattended goal, watched the window behave exactly as if it
        // did not exist — the same "nothing happened" shape this window has produced five
        // separate ways. It sits above the transcript because that is where DSH puts its
        // goal strip, and because a goal is the reason the conversation below it is
        // happening.
        let goal_band = 34.0 * scale;
        let goal_top = y + 4.0;
        y = match self.draw_goal_band(ui, summary, rect, goal_top, goal_band, scale) {
            Some(after) => after,
            None => goal_top,
        };
        // ---- transcript -----------------------------------------------------
        let composer_top = rect.bottom() - COMPOSER_H * scale;
        let body = egui::Rect::from_min_max(
            egui::pos2(rect.left() + 8.0, y + 4.0),
            egui::pos2(rect.right() - 8.0, composer_top - 6.0),
        );
        if body.height() > 0.0 {
            // **Laid out AT `body`, not at the pane's cursor (M312).**
            //
            // `ScrollArea::show(ui, ..)` starts from wherever the passed `Ui`'s cursor is —
            // which in this pane is the **top of the pane**, not `body.top`. The header line,
            // the notices and the goal band are painted with absolute positions, so nothing
            // moved the cursor down to where they end, and `max_height(body.height())` limits
            // the height without moving the content. Measured on the approval frame: the pane's
            // rect is `[[920, 42]-[1440, 870]]`, so `body.top` is **84**, and the scroll area's
            // clip was `[[920, 42]-…]` — the transcript was drawn **42 pixels too high**, over
            // its own header, which is why the rendered frame shows the pane's `Agent` label
            // sitting in the middle of the conversation and the first rows clipped by the pane's
            // top edge.
            //
            // Nothing caught it because every test here asserts **which** text was drawn, never
            // **where**: a row drawn in the right place and a row drawn 42 pixels too high put
            // the same strings in `FrameSummary::rows`. Only looking at the frame found it.
            let mut body_ui = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(body)
                    .layout(egui::Layout::top_down(egui::Align::LEFT)),
            );
            let out = egui::ScrollArea::vertical()
                .id_salt("epsilon-agent-transcript")
                .stick_to_bottom(true)
                .max_height(body.height())
                .show(&mut body_ui, |ui| {
                    ui.set_width(body.width());
                    self.draw_rows(ui, summary, scale)
                });
            // **The scrollbar, named when there is one (M312).** A conversation longer than the
            // pane is the case the transcript's virtualisation exists for, so this is the bar a
            // reader meets exactly when they need it. **Before** `out.inner` is taken out: the
            // output is borrowed whole here, and moving a field out of it first would leave a
            // partially-moved value to borrow.
            name_scrollbar(body_ui.ctx(), &out, "the conversation");
            let clicks = out.inner;
            // Applied here rather than inside the draw, so a click is resolved once, in
            // one place, and the draw path stays a draw path.
            if let (Some((id, decision)), Some(s)) = (clicks.decision, self.session.as_ref()) {
                s.resolve(&id, decision);
            }
            // A clicked "changed" row opens the review. It lands on the **next** frame,
            // because this pane is drawn after the editor pane — an immediate-mode frame
            // paints top to bottom, and pretending otherwise would mean moving the agent
            // panel in front of the file view to save one repaint.
            if let Some(summary_text) = clicks.change
                && let Some(change) = self
                    .transcript
                    .diffs()
                    .iter()
                    .find(|c| c.summary() == summary_text)
                    .cloned()
            {
                self.review = Some(change);
            }
            // **The transcript's accessible name — the first thing a screen reader asks for
            // and, until now, the one thing it could not get.**
            //
            // The count, and *only* the count. Publishing the transcript itself would have
            // a reader announce every row on every repaint — unusable, and the reason this
            // is a description rather than the content. The count is the transcript's own
            // `rows().len()`, **not** the number of rows the frame drew: the frame draws a
            // window of the conversation and a name that counted the window would report
            // "28 entries" for a two-hundred-entry chat every time it was at the bottom.
            //
            // "entry", not "message": `RowKind` carries tool calls, diffs and notices as
            // well as turns, and a label that overstates what it counted is a label that
            // lies.
            let entries = self.transcript.rows().len();
            name_region(
                ui,
                body,
                // The scroll area's own `id_salt`, so the node and the scrolling region
                // are one thing rather than two.
                "epsilon-agent-transcript",
                &format!(
                    "Agent transcript · {entries} {}",
                    if entries == 1 { "entry" } else { "entries" }
                ),
                // `Log` is the ARIA role for a region of past events, and a conversation is
                // exactly that. `Document` was considered and rejected: it is for a body of
                // text the reader is *editing*, and this pane is read-only. This is also the
                // single most important landmark in the window — it is the thing a reader came
                // for — so it is named before anything else a reader would reach.
                egui::accesskit::Role::Log,
            );
        }
        // ---- composer -------------------------------------------------------
        let composer = egui::Rect::from_min_size(
            egui::pos2(rect.left() + 8.0, composer_top),
            egui::vec2((rect.width() - 16.0).max(0.0), COMPOSER_H * scale - 6.0),
        );
        if composer.width() > 40.0 {
            self.draw_composer(ui, summary, composer, scale);
        } else {
            // Too narrow for a composer worth drawing; say so rather than drawing a
            // one-column text box that cannot be read or typed into.
            let line = "widen the window to type";
            let painter = ui.painter();
            paint_text(
                painter,
                summary,
                "agent",
                composer.left_top() + egui::vec2(4.0, 12.0 * scale),
                egui::Align2::LEFT_TOP,
                line,
                egui::FontId::proportional(12.0 * scale),
                theme.dim,
            );
            summary.rows.push(("agent".into(), line.into()));
        }
    }
    /// The goal strip: what the window is working toward, and how much of it is left.
    ///
    /// Returns the y the transcript may start at — the band's bottom — or `None` when there
    /// is no goal to show, which is the common case and costs nothing.
    ///
    /// **Everything on it is a fact read from the goal, never a guess.** The rounds are the
    /// only budget an unattended goal has (DSH's design, deliberate), so a reader who cannot
    /// see them has no way of knowing whether to leave it running.
    fn draw_goal_band(
        &mut self,
        ui: &mut egui::Ui,
        summary: &mut FrameSummary,
        pane: egui::Rect,
        top: f32,
        height: f32,
        scale: f32,
    ) -> Option<f32> {
        let view = self.goal.clone()?;
        let theme = self.theme;
        // The band's x comes from the **pane**, not from zero.
        //
        // It took only a width, and built the rect at `pos2(0.0, top)`, so with the sidebar
        // open the band was laid out at the window's left edge: at 1440×900 the agent pane
        // starts at x=920, which put a 520-wide goal strip across the activity bar, the file
        // filter box and the first tree rows. `with_clip_rect` *replaces* the clip rather than
        // intersecting it, so the pane's own clip did not save it. It only ever showed when a
        // goal existed, which is why every frame test — none of which seeded one — passed.
        let width = pane.width();
        let band =
            egui::Rect::from_min_size(egui::pos2(pane.left(), top), egui::vec2(width, height));
        let painter = ui.painter().with_clip_rect(band);
        painter.rect_filled(band, 4.0, theme.panel);
        painter.rect_stroke(
            band,
            4.0,
            egui::Stroke::new(1.0, theme.border),
            egui::StrokeKind::Inside,
        );

        let goal = &view.goal;
        let headline = format!(
            "goal · {} · round {}/{}",
            goal.status.as_str(),
            goal.rounds_started,
            goal.max_rounds
        );
        paint_text(
            &painter,
            summary,
            "goal",
            band.left_top() + egui::vec2(10.0, 15.0 * scale),
            egui::Align2::LEFT_CENTER,
            headline.clone(),
            egui::FontId::proportional(12.0 * scale),
            theme.title,
        );
        // The gates are the rule the goal is judged by, so their count is on the strip —
        // and "no gates" is not the same claim as "a gate passed".
        let gates = format!("{} gate(s)", goal.gates.len());
        paint_text(
            &painter,
            summary,
            "goal",
            band.right_top() - egui::vec2(10.0, 15.0 * scale),
            egui::Align2::RIGHT_CENTER,
            gates.clone(),
            egui::FontId::proportional(12.0 * scale),
            theme.dim,
        );
        // The objective, shortened rather than clipped: a goal whose text stops at the pane
        // edge with no marker is indistinguishable from a shorter goal.
        let objective = fit(&goal.objective, (width - 24.0).max(40.0), 12.0 * scale);
        paint_text(
            &painter,
            summary,
            "goal",
            band.left_top() + egui::vec2(10.0, 30.0 * scale),
            egui::Align2::LEFT_CENTER,
            objective.clone(),
            egui::FontId::proportional(12.0 * scale),
            theme.dim,
        );
        // Armed-but-stopped is worth its own words: a goal that is active and not continuing
        // looks to the reader exactly like a goal that has finished.
        let state = if view.armed {
            "continuing automatically".to_string()
        } else {
            "not armed — it will not continue".to_string()
        };
        paint_text(
            &painter,
            summary,
            "goal",
            band.right_top() - egui::vec2(10.0, 30.0 * scale),
            egui::Align2::RIGHT_CENTER,
            state.clone(),
            egui::FontId::proportional(11.0 * scale),
            if view.armed { theme.dim } else { theme.warn },
        );

        summary.rows.push(("goal".into(), headline));
        summary.rows.push(("goal".into(), objective));
        summary.rows.push(("goal".into(), gates.clone()));
        summary.rows.push(("goal".into(), state));
        summary.row_rects.push(("goal".into(), band));
        // **The band's accessible name, and it carries `armed` because that is the half of
        // the state the reader cannot see.** A goal that is active and not continuing looks
        // exactly like a goal that has finished — the comment above says so, and that is the
        // sentence this exists for. Every other fragment here is read from the same values the
        // band just drew, so the two cannot drift apart.
        //
        // **Not the objective.** It is the reader's own text, of unbounded length, drawn
        // through `fit` because it does not fit. A name is announced on every repaint; one
        // built from a paragraph is unusable, and the transcript's name is bounded for
        // exactly the same reason.
        name_region(
            ui,
            band,
            GOAL_SALT,
            &format!(
                "Goal band · {} · round {}/{} · {gates} · {}",
                goal.status.as_str(),
                goal.rounds_started,
                goal.max_rounds,
                if view.armed { "armed" } else { "not armed" },
            ),
            egui::accesskit::Role::Status,
        );
        Some(band.bottom() + 4.0)
    }

    /// Draw the transcript rows.
    ///
    /// Returns what the reader clicked: a permission answered with the mouse, or a change
    /// row opened for review. Buttons as well as keys because a gate that stops the whole
    /// run cannot be reachable only from the keyboard: the ask is drawn where a reader is
    /// already looking, so the two answers belong beside it.
    ///
    /// # Virtualised, and how
    ///
    /// **Only the rows that are on screen are drawn.** M266 measured this frame at
    /// **104 ms for 800 assistant rows** and M275 attributed **99.481 ms of it — 95% — to
    /// egui laying out and shaping every block of every row, on screen or not**. The only
    /// thing that addresses that is not building the widget.
    ///
    /// [`Self::row_offsets`] has already laid every row out once and left a prefix sum, so
    /// all this has to do is find the span the viewport covers and put the rest of the
    /// transcript's height into two [`egui::Ui::add_space`] calls — one before, one after.
    /// **The spacers are the scroll extent**: the scroll area's content height is
    /// `offsets[n]`, so the scrollbar, the wheel and `stick_to_bottom` all still see the
    /// whole conversation, and a transcript that renders fast and stops following the run
    /// is worse than a slow one.
    ///
    /// # Why the band comes from the clip rectangle
    ///
    /// `ScrollArea` already hands a caller the visible band — [`ScrollArea::show_viewport`]
    /// passes it to the closure — but only for the caller who asks. Reading it off `ui`
    /// instead costs nothing and keeps [`Self::agent_panel`] exactly as it was, and the
    /// clip rect is the truth about what will be *seen*: `ScrollArea::begin` sets the
    /// content `Ui`'s clip to the inner rect (`egui-0.36.2/src/containers/scroll_area.rs:815-825`),
    /// so `clip.min.y - max_rect.min.y` is the first visible content pixel and
    /// `clip.max.y - max_rect.min.y` the first invisible one. `max_rect.min.y` is where
    /// this cursor starts, which is the origin every offset below is counted from.
    fn draw_rows(
        &self,
        ui: &mut egui::Ui,
        summary: &mut FrameSummary,
        scale: f32,
    ) -> TranscriptClicks {
        let theme = self.theme;
        let mut clicks = TranscriptClicks::default();
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
            let response = ui.colored_label(theme.dim, line);
            // **The empty state is not a row, and it must not be named as one.** egui has
            // already built a node for this label (`Label::ui` registers one for every
            // label it draws); unnamed, a screen reader reaches a node with no name at all.
            // So it says what it is — an empty transcript and why — instead of borrowing
            // the "Transcript row ·" prefix the conversation itself uses.
            name_node(&response, &format!("Transcript empty · {line}"));
            summary.rows.push(("agent".into(), line.to_string()));
            return clicks;
        }
        // Read before the first widget: a widget's own rect is what changes the available
        // width, and the row widths are measured against it.
        let origin = ui.max_rect().min.y;
        let clip = ui.clip_rect();
        let offsets = self.row_offsets(ui, rows, scale);
        let span = offsets.span(clip.min.y - origin, clip.max.y - origin);
        // The two spacers. `add_space` advances the cursor by exactly what it is given
        // (`Layout::advance_cursor`, `egui-0.36.2/src/layout.rs:674`), and a widget's
        // allocation adds `item_spacing.y` **after** it rather than before
        // (`placer.rs:165-192`), which is why every measured height carries that one
        // spacing: row `i` then starts at `offsets[i]` on the frame, to the pixel.
        ui.add_space(offsets.at(span.start));
        for (i, row) in rows[span.clone()].iter().enumerate() {
            if let Some(changed) = self.draw_row(ui, summary, row, false, scale) {
                clicks.change = Some(changed);
            }
            // **Where the first drawn row actually landed (M312).** One entry per frame, for the
            // row that was drawn first: `FrameSummary::rows` records *which* text was drawn and
            // nothing recorded *where*, which is why a transcript drawn 42 pixels too high — over
            // its own header, with the first rows clipped by the pane edge — passed every test
            // in this file and was visible in the rendered frame in a glance. The number this
            // records is the one a person looks at.
            if i == 0 {
                summary
                    .hotspots
                    .push(("transcript:first-row".to_string(), ui.min_rect()));
            }
        }
        ui.add_space(offsets.total() - offsets.at(span.end));
        // The pending approvals are drawn after the rows, where they cannot scroll out of
        // reach behind a long answer — the composer-adjacent placement opencode and DSH
        // both converged on, and the reason is not aesthetic.
        //
        // Drawn from the **approvals channel**, not from the transcript: the channel is
        // the authority on what is waiting, and a run that parks on a gate before its first
        // message has produced no rows at all. A reader in that state must still be told
        // the run is blocked on *them*.
        let mut decided = None;
        let ask_budget = (ui.available_width() - 240.0).max(120.0);
        for ask in pending {
            let (text, honest) = Self::ask_line(&ask, ask_budget, scale);
            let response = ui.colored_label(theme.warn, text.clone());
            // **A pending ask is drawn outside the virtualised span** — it sits under the
            // rows on purpose, so it cannot scroll out of reach behind a long answer — and
            // it is the one transcript line that can stop the run. Naming it costs one
            // field on a node egui already built, and it is named as the permission request
            // it is rather than as text, because the words under it are the tool's.
            name_node(&response, &row_name("Permission request", &text));
            summary.rows.push(("agent".into(), text));
            summary.row_rects.push(("agent".into(), ui.min_rect()));
            // The two answers, drawn where the ask is drawn. `horizontal` keeps them on
            // one line with the ask; a run that is blocked on this row and has no way to
            // unblock it is the window's worst state, so the buttons are not a
            // convenience.
            ui.horizontal(|ui| {
                if honest
                    && ui
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
        clicks.decision = decided;
        clicks
    }

    /// Answer a close request. Returns `true` when the window may close.
    ///
    /// **The one place that decision is made**, and it is a function rather than an inline
    /// check because `ViewportInfo::close_requested` is a flag egui only ever *reads* — the
    /// platform produces it — so a test cannot raise it through `Context`. Putting the answer
    /// behind a callable seam is what makes the guard testable **without** giving the test a
    /// second implementation to pass against.
    ///
    /// **Fails closed, twice over.** An answer this code cannot compute is treated as "work
    /// is running" (see [`QuitAsk`]), and a request that arrives while the dialog is already
    /// up is cancelled again rather than re-arming — so holding the window's close button does
    /// not fall through into a close.
    pub fn answer_close_request(&mut self, ctx: &egui::Context) -> bool {
        if self.quit_confirm.is_none() {
            self.quit_confirm = Some(self.inspect_quit());
        }
        // **Cancel the platform close here, not in `paint`.** eframe closes a
        // viewport whose close was not cancelled in the same frame
        // (`epi_integration.rs`), so a refusal recorded anywhere else is a
        // dialog over a window that is already gone. Sending it here keeps the
        // decision and its enforcement in the one function tests drive — the
        // reason `answer_close_request` exists at all.
        //
        // Deliberately no silent path, even for an empty window: a tested
        // product rule says closing is the reader's decision, and overturning
        // it solo would trade D-031's second half for that rule. The tension
        // is recorded in D-031 itself.
        ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        false
    }

    /// What closing the window right now would interrupt.
    ///
    /// **Read once, when the request arrives, and never again while the dialog is up** — a
    /// warning that changes under the reader is worse than no warning, because the thing it
    /// said a moment ago is the thing they decided about.
    ///
    /// Fails closed: a window with no session attached is the *one* case that reports
    /// `empty`, and a session whose state cannot be read is not that case.
    fn inspect_quit(&self) -> QuitAsk {
        match self.session.as_ref() {
            None => QuitAsk {
                running: false,
                pending_approvals: 0,
                conversation: false,
                empty: true,
            },
            Some(session) => {
                let rows = self.transcript.rows().len();
                QuitAsk {
                    running: session.running(),
                    pending_approvals: session.pending_approvals().len(),
                    // "Something is on screen the reader would want again." The session
                    // file is durable, so this is not a claim that work is lost — it is the
                    // claim that `/clear` was not pressed, which is what the reference asks
                    // about too.
                    conversation: rows > 0,
                    empty: false,
                }
            }
        }
    }

    /// Ask, when there is something to lose. Returns `true` when the reader chose to close.
    ///
    /// **The safe answer is the one a stray key gets.** `Enter` and `Escape` both keep the
    /// window, because a dialog that a reflexive `Enter` dismisses destructively is the
    /// reflexive-`Enter` case `AGENTS.md` §6 and DSH's `RiskConfirmation` both exist to
    /// prevent. Quitting still has a keyboard route — the buttons are real egui buttons, so
    /// `Tab` reaches them and `Space` presses one — and it is deliberately the longer way.
    fn draw_quit_confirm(&mut self, ui: &mut egui::Ui, summary: &mut FrameSummary, scale: f32) {
        let Some(ask) = self.quit_confirm.clone() else {
            return;
        };
        let theme = self.theme;
        let screen = ui.ctx().content_rect();
        let text = ask.sentence();
        let width = 460.0 * scale;
        let height = 96.0 * scale;
        let box_rect = egui::Rect::from_center_size(
            screen.center(),
            egui::vec2(width.min(screen.width()), height.min(screen.height())),
        );
        // **Painted with the painter, like the palette**, rather than through
        // `egui::Modal`. The modal is the right widget for an application that owns a
        // viewport; this window draws its own chrome into whatever `Ui` the host hands it,
        // and a `Modal` opened from a bare `Context` in a test had nothing to lay out in.
        // One idiom for every overlay is also the reason the palette's tests can read its
        // rectangles: a second mechanism would be a second thing that can silently not draw.
        let painter = ui.painter().with_clip_rect(screen);
        painter.rect_filled(box_rect, 6.0, theme.panel);
        painter.rect_stroke(
            box_rect,
            6.0,
            egui::Stroke::new(1.0, theme.border),
            egui::StrokeKind::Inside,
        );
        // **No scrim, and that is a measured choice rather than a missing nicety.** A
        // translucent dim over the window was the obvious way to say "the window
        // underneath is not in charge", and it rendered as a **blank white PNG** from the
        // offscreen rasteriser — every other screen in the showcase is fine, and only the one
        // with a blend pass came out empty. A guard whose screen cannot be looked at is a
        // guard nobody has reviewed, so the scrim goes: the box has a border, a strong title
        // and a centred position, and those carry the same meaning without a dependency on
        // how the frame happens to be captured. (`audit/evidence/desktop-frames-2026-10-05-m316/13-quit-guard.png`
        // is the blank one, kept because a defect found by looking is evidence.)

        // **Painted, not `egui::Label`.** Every other string in this window goes through
        // `paint_text`, and that is not tidiness: a first version used two wrapping
        // `Label`s here and the offscreen rasteriser produced a **blank white PNG** for this
        // screen alone, while the other twelve rendered normally. A screen nobody can look at
        // is a screen nobody has reviewed, so the dialog uses the same paint path as the rest
        // of the window — one mechanism rather than two.
        paint_text(
            ui.painter(),
            summary,
            "agent",
            box_rect.left_top() + egui::vec2(12.0, 12.0),
            egui::Align2::LEFT_TOP,
            "Close this window?".to_string(),
            egui::FontId::proportional(14.0 * scale),
            theme.title,
        );
        let body_text = fit(&text, box_rect.width() - 24.0, 12.0 * scale);
        paint_text(
            ui.painter(),
            summary,
            "agent",
            box_rect.left_top() + egui::vec2(12.0, 34.0 * scale),
            egui::Align2::LEFT_TOP,
            body_text.clone(),
            egui::FontId::proportional(12.0 * scale),
            theme.text,
        );
        // **Named, and named as a dialog.** An unnamed box of strings is a group a reader
        // arrives at with no idea what it is — the same failure the composer's unnamed
        // `TextEdit` was, and the reason [`name_region`] exists.
        let name = format!("Close this window? {text}");
        name_region(
            ui,
            box_rect,
            "quit-confirm",
            &name,
            egui::accesskit::Role::Dialog,
        );
        // **One node for the whole dialog, and no node per line.** The painter path builds
        // no `Response` to name — that is what `paint_text` is — so the lines are carried by
        // the single `Dialog` region above, which is also what a reader wants: one thing to
        // arrive at, saying what the window is asking and why.
        summary
            .rows
            .push(("agent".into(), "Close this window?".into()));
        summary.rows.push(("agent".into(), text.clone()));

        // **Keep working is the first button**, so the leftmost control and the default tab
        // stop are both the answer that loses nothing.
        let buttons = egui::Rect::from_min_size(
            box_rect.left_bottom() + egui::vec2(12.0, -34.0 * scale),
            egui::vec2(box_rect.width() - 24.0, 24.0 * scale),
        );
        // **Painted at absolute positions, not laid out.** This window draws its chrome with
        // the painter rather than in a `Ui`'s flow — the palette above does the same, and for
        // the same reason: a `Ui` that lays out children will not put them where the painter
        // put the box. The buttons are `ui.put` into their own rect, which is both absolute
        // and a real widget, so they still take focus and still answer the keyboard.
        let keep_rect = egui::Rect::from_min_size(
            buttons.left_top(),
            egui::vec2(buttons.width() * 0.5, buttons.height()),
        );
        let quit_rect = egui::Rect::from_min_size(
            egui::pos2(buttons.left_top().x + buttons.width() * 0.5, buttons.top()),
            egui::vec2(buttons.width() * 0.5, buttons.height()),
        );
        let keep_button = ui
            .put(
                keep_rect,
                egui::Button::new("Keep working").min_size(egui::vec2(120.0, 24.0 * scale)),
            )
            .on_hover_text("Stay open. Escape does the same.");
        let quit_button = ui
            .put(
                quit_rect,
                egui::Button::new("Close anyway").min_size(egui::vec2(120.0, 24.0 * scale)),
            )
            .on_hover_text("Stop the turn and close the window.");
        // **Keep working is the first control**, so the leftmost button and the default tab
        // stop are both the answer that loses nothing.
        keep_button.request_focus();
        let keep = keep_button.clicked();
        let quit = quit_button.clicked();
        // Escape is read at the **top of `paint`**, before the composer takes it; see the
        // note there. Reading it here would find nothing, and the dialog would appear not to
        // answer the keyboard at all.
        if keep {
            self.quit_confirm = None;
            return;
        }
        if quit {
            self.quit_confirm = None;
            // **A second close request is the one we mean.** The first was refused; sending
            // it again with nothing pending is how the application says yes.
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    /// Draw one transcript row, and report a change row the reader clicked.
    ///
    /// **The same function measures and draws** — `measure` says which. That is the whole
    /// reason [`Self::row_offsets`] can trust the heights it caches: there is no second
    /// implementation of a row to drift from this one, only a flag on the one place the
    /// two paths differ.
    fn draw_row(
        &self,
        ui: &mut egui::Ui,
        summary: &mut FrameSummary,
        row: &crate::session::Row,
        measure: bool,
        scale: f32,
    ) -> Option<String> {
        let theme = self.theme;
        // **Markers this window's font can draw (M312).** They were `▸`/`✓`/`✗`, and the rendered
        // frame showed **both finished tool calls as an empty box** — `U+2713`/`U+2717` are
        // Dingbats, and egui's bundled font has no Dingbats (the same font that turned the token
        // arrows into boxes, and the same answer: the title bar's `↑`/`↓` are Arrows and are also
        // missing). `▸`, `±`, `◨` and `›` are all covered, so this is not "no symbols" — it is
        // **two missing blocks**.
        //
        // The spellings are the terminal's own ASCII repertoire (`tick: "+"`, `cross: "x"`,
        // `cursor_bar: ">"`, `crates/epsilon-tui/src/ui/theme.rs:340-358`), so both front ends
        // draw the same outcome the same way rather than this window inventing a third set.
        // Meaning is not carried by shape alone: the row's **accessible name** already says
        // `Tool call`, `ok` or `failed` (`row_kind_name`), which is WCAG 2.2 SC 1.4.1's "not by
        // shape or colour alone" satisfied by text rather than by a third glyph.
        let (marker, color) = match &row.kind {
            RowKind::User => ("› ", theme.user),
            RowKind::Assistant => ("  ", theme.text),
            RowKind::Thinking => ("~ ", theme.thinking),
            RowKind::Tool { failed, .. } => match failed {
                None => ("> ", theme.accent),
                Some(false) => ("+ ", theme.success),
                Some(true) => ("x ", theme.danger),
            },
            RowKind::Approval { .. } => ("! ", theme.warn),
            RowKind::Changed => ("± ", theme.accent),
            RowKind::Diff => ("  ", theme.dim),
            RowKind::Notice => ("  ", theme.dim),
        };
        // A tool row with no arguments yet still has to say which tool it is: an
        // empty row with a marker is a mark the reader has to decode.
        //
        // **Stripped here, once, for every row kind.** A `U+202E` reverses the visual order
        // of everything after it, so a message, a tool's arguments or a file's contents can
        // be *displayed* as one thing and *be* another. The terminal has cleaned all of these
        // since M260 (the transcript), M273 (tool cards) and M274 (its panels); this window
        // cleaned only its permission prompt until now. One call here covers every kind,
        // and `draw_markdown` inherits it because the assistant's prose is rendered *from*
        // this text rather than beside it.
        let body: std::borrow::Cow<'_, str> = match &row.kind {
            // **The tool's name belongs on the row, not only in its accessible name (M312).**
            // A rendered frame showed `+ {"command": "cargo test -p epsilon-session"}` — the
            // arguments of a tool the reader was never told the name of, while the row's
            // accessible name said `Tool call · bash · …` and no sighted reader could see it.
            // The arguments are still JSON, which is the terminal's tool *card* to improve on
            // (M273) and not this fix; **which tool ran** is the half that was missing entirely.
            RowKind::Tool { name, .. } if !row.text.is_empty() => {
                format!("{name} {}", Self::safe_to_show(&row.text)).into()
            }
            RowKind::Tool { name, .. } => format!("{name}…").into(),
            _ if row.text.is_empty() => "".into(),
            _ => Self::safe_to_show(&row.text),
        };
        // **Only the assistant's text is interpreted.** A user who types `**hi**`
        // must see `**hi**`; a diff line is a diff line. Rendering is an
        // interpretation, and applying it to text that is not prose makes the
        // window lie about what was said.
        if matches!(row.kind, RowKind::Assistant) {
            self.draw_markdown(ui, summary, marker, color, &body, measure, scale);
            return None;
        }
        // **What this row is, in the words a reader would use for it.** Read from the same
        // `row.kind` the marker above was read from, so the name and the glyph cannot
        // disagree about which row this is, and applied to the **sanitised** `body` and
        // `text` below — never to raw model or tool output, which is exactly what
        // [`Self::safe_to_show`] exists to strip. Below the assistant branch because
        // `draw_markdown` names its own blocks.
        let kind = row_kind_name(&row.kind);
        let text = format!("{marker}{body}");
        let painted = Self::row_label(
            ui,
            measure,
            egui::RichText::from(text.clone()).color(color).into(),
            // **From `body`, not from `text`.** The marker is a glyph, and the kind name
            // above has already answered the question it answers: `› fix the build` is a
            // row the reader typed, and `▸ …` is a tool call, which the name says in words
            // a screen reader can read out.
            &row_name(&kind, &body),
        );
        // **What the tool printed, under the call that printed it.**
        //
        // This is the difference between a window that runs tools and a window you can
        // work in: `▸ bash cargo test` followed by `✓` tells you nothing about whether
        // the suite passed, and the terminal has shown it since M63. Each line is its own
        // widget because this is a log, not prose — wrapping it into the row above would
        // turn `FAILURES:` into a sentence.
        //
        // Dimmed, so a wall of compiler output cannot out-shout the conversation it
        // is part of, and drawn last so it lands under the row that produced it.
        //
        // **Each line is also recorded in `summary.rows`.** Painting without recording is
        // how this feature would have shipped unverified: `FrameSummary::describe` is the
        // only window anyone has into what the window shows, so a line the summary does
        // not carry is a line no frame test can ever see. The first version drew
        // correctly and the frame test still failed — not because the drawing was wrong,
        // but because the instrument could not see it.
        if let RowKind::Tool { name, output, .. } = &row.kind
            && !output.trim().is_empty()
        {
            // **Each output line is its own node and its own name.** Naming them all
            // "Tool call · bash · succeeded" would have a reader hear the same words once
            // per line of compiler output, which is the wall of noise the dimming below is
            // there to prevent — and the line itself is the only thing that tells one
            // output line from another.
            let output_kind = format!("{name} output");
            for line in output.lines() {
                // **Tool output is the highest-value text in the window and the least
                // controlled**: for `read` it is a file's contents, and a file can contain
                // anything the model has read, including a name it was told to render.
                let shown = format!("    {}", Self::safe_to_show(line));
                Self::row_label(
                    ui,
                    measure,
                    egui::RichText::from(shown.clone()).color(theme.dim).into(),
                    &row_name(&output_kind, &shown),
                );
                summary.rows.push(("agent".into(), shown));
            }
        }
        // **A change row is a link.** The row under it says "open the file", and until
        // this existed there was nothing to open: a review instruction with no control
        // beside it is the "nothing happened" state reached by asking the agent to do
        // its job. Alloc rather than position, so the rectangle and the click cannot
        // drift apart.
        //
        // **`allocate_space`, not `allocate_exact_size`, while measuring.** Both advance the
        // cursor identically, so the height is the same either way; only the click target
        // and its hotspot differ, and a hotspot for a row nobody can click is a rectangle a
        // test would keep re-finding on a frame it is not there.
        let response = if matches!(row.kind, RowKind::Changed) {
            if measure {
                ui.allocate_space(painted.rect.size());
                None
            } else {
                let (rect, response) =
                    ui.allocate_exact_size(painted.rect.size(), egui::Sense::click());
                summary
                    .hotspots
                    .push((format!("change:{}", row.text), rect));
                // **The node this click target already has, named and roled.** `Sense::click()`
                // is focusable, so egui allocated an AccessKit node for this row and set no
                // role on it (`egui-0.36.2/src/context.rs:1292-1297`) — the same shape the file
                // tree had at M301, found here by reading `name_role`'s contract rather than by
                // another dump. A reader could focus the row and be told nothing, which for a
                // control whose entire purpose is "open this file" is the worst case of the
                // three.
                name_node(&response, &format!("Open the change · {}", row.text));
                name_role(&response, egui::accesskit::Role::Link);
                Some(response)
            }
        } else {
            None
        };
        summary.rows.push(("agent".into(), text));
        summary.row_rects.push(("agent".into(), painted.rect));
        response
            .filter(egui::Response::clicked)
            .map(|_| row.text.clone())
    }

    /// Every transcript row's height and where it starts, measuring whatever has not been
    /// measured yet.
    ///
    /// Called once per frame from [`Self::draw_rows`], before any row is drawn. A frame
    /// where nothing changed is a fold over `(ptr, len)` per row and a prefix sum — no
    /// shaping, no widget, no allocation after the first. A frame where a row changed
    /// measures **that row**, which is what a streaming answer costs: one row, once per
    /// update, which is the row the reader is watching grow anyway.
    ///
    /// **The probe is one `Ui`, reused for every row.** It is built from the *same*
    /// `max_rect.min`, width, layer and style as the transcript's content `Ui`, so a row
    /// measured in it wraps at exactly the width it will be drawn at — a measurement at the
    /// wrong width is not a cheap measurement, it is a wrong one, and the symptom is rows
    /// overlapping once the pane is narrowed. `Sense::empty()` on the probe keeps it out
    /// of the accessibility tree: a container node per off-screen row would be published to
    /// the screen reader for content no reader can reach.
    ///
    /// A width change drops every height, because a paragraph that wrapped at 500 points
    /// does not wrap the same way at 400 and no narrower change is visible until it is
    /// late. Re-measuring 2,000 rows on a window drag is the honest cost of that.
    fn row_offsets(
        &self,
        ui: &egui::Ui,
        rows: &[crate::session::Row],
        scale: f32,
    ) -> std::cell::Ref<'_, RowLayout> {
        let width = ui.available_width();
        {
            let mut layout = self.row_layout.borrow_mut();
            let mut stale = (layout.width - width).abs() >= 0.5 || layout.scale != scale;
            if stale {
                layout.rows.clear();
            }
            layout.width = width;
            layout.scale = scale;
            for (i, row) in rows.iter().enumerate() {
                let seen = row_seen(row);
                match layout.rows.get_mut(i) {
                    Some(cached) if cached.seen == seen => {}
                    Some(cached) => {
                        *cached = RowHeight {
                            seen,
                            height: f32::NAN,
                        };
                        stale = true;
                    }
                    None => {
                        layout.rows.push(RowHeight {
                            seen,
                            height: f32::NAN,
                        });
                        stale = true;
                    }
                }
            }
            if layout.rows.len() > rows.len() {
                // The transcript trimmed (`MAX_ROWS` drops the oldest). Keeping the tail's
                // heights is right and keeping the head's is not, so this is a truncation
                // and not a clear: `rows` only ever grows at the end.
                layout.rows.truncate(rows.len());
                stale = true;
            }
            if stale {
                self.measure_rows(ui, rows, &mut layout, scale);
                layout.offsets.clear();
                layout.offsets.reserve(rows.len() + 1);
                layout.offsets.push(0.0);
                let mut running = 0.0f32;
                for i in 0..layout.rows.len() {
                    running += layout.rows[i].height;
                    layout.offsets.push(running);
                }
            }
        }
        self.row_layout.borrow()
    }

    /// Lay out every unmeasured row into one throwaway probe and keep the height.
    fn measure_rows(
        &self,
        ui: &egui::Ui,
        rows: &[crate::session::Row],
        layout: &mut RowLayout,
        scale: f32,
    ) {
        let mut probe = egui::Ui::new(
            ui.ctx().clone(),
            ui.id().with("epsilon-row-probe"),
            egui::UiBuilder::new()
                .max_rect(egui::Rect::from_min_size(
                    ui.max_rect().min,
                    egui::vec2(ui.available_width(), 0.0),
                ))
                .layer_id(ui.layer_id())
                .layout(*ui.layout())
                .sense(egui::Sense::empty()),
        );
        let mut sink = FrameSummary::default();
        for (i, row) in rows.iter().enumerate() {
            if !layout.rows[i].height.is_nan() {
                continue;
            }
            // **The probe's cursor has to be folded into its `min_rect` before and after
            // each row**, and the `add_space(0.0)` is how. `min_rect` grows only where a
            // widget actually landed, so without this the second row would measure from
            // the first row's last pixel rather than from where the first row's trailing
            // `item_spacing` put it — and every row after it would be a pixel short, which
            // is a drift of ten pixels over eight hundred rows and a visible staircase at
            // the bottom of the transcript.
            probe.add_space(0.0);
            let before = probe.min_rect().max.y;
            sink.rows.clear();
            sink.row_rects.clear();
            sink.hotspots.clear();
            self.draw_row(&mut probe, &mut sink, row, true, scale);
            probe.add_space(0.0);
            layout.rows[i].height = probe.min_rect().max.y - before;
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
            // **Say why, rather than returning nothing.** This is the invariant the whole
            // project is named for: "nothing happened" must never be a state the UI can be in.
            // A reader who presses Enter mid-turn and watches the composer stay put has no way
            // to tell a refused send from a dropped key.
            //
            // The sentence is `IdeSession::submit`'s, written long ago and stored in
            // `self.refusal` for the status bar — but this guard runs **one line earlier** and
            // returned before `submit` could ever be reached, so the branch that produces the
            // message was unreachable and the message never reached a reader. The guard and the
            // refusal asked the same question twice; now the guard answers it.
            self.refusal =
                Some("a run is already in flight — wait for it, or press Esc to stop it".into());
            self.transcript
                .push_notice("a run is already in flight — wait for it, or press Esc to stop it");
            return None;
        }
        let text = self.draft.trim().to_string();
        if text.is_empty() {
            return None;
        }
        // **A command is not a prompt.** Before this, anything starting with `/` went
        // straight to the model as text, so a reader who knew the terminal asked the model to
        // compact and got a plausible paragraph back. `try_command` answers with what the
        // window did, and the third outcome — refused *by name* — is the one that matters:
        // a window with three commands is smaller, but one that forwards the rest silently is
        // lying. See `crate::commands`.
        match self.try_command(&text) {
            crate::commands::Outcome::Send(text) => {
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
            crate::commands::Outcome::Ran(said) => {
                self.draft.clear();
                self.transcript.push_notice(&said);
                None
            }
            crate::commands::Outcome::Refused(said) => {
                // **The draft is deliberately left as it was typed.** Clearing it would make a
                // refused command indistinguishable from one that ran, and the reader's words
                // are still the right words — they want to edit the argument, or send it as a
                // prompt on purpose.
                self.transcript.push_notice(&said);
                None
            }
        }
    }
    /// The composer's text box, and the two ways to send.
    ///
    /// `Enter` sends and `Shift+Enter` inserts a newline, which is what every editor a
    /// developer already has in their hands does.
    fn draw_composer(
        &mut self,
        ui: &mut egui::Ui,
        summary: &mut FrameSummary,
        rect: egui::Rect,
        scale: f32,
    ) {
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
        //
        // **`Shift+Enter` is a newline, and no longer needs a hand-written guard to prove it.**
        //
        // M314 fixed this by hand: egui's `consume_key` matches modifiers *logically*
        // (`egui-0.36.2/src/input_state/mod.rs:717-718`), so a plain `Enter` read also
        // swallowed `Shift+Enter` and the draft was **sent** — the opposite of what this
        // composer advertises. The fix was a block that scanned the event stream for a
        // `Shift`+`Enter` and declined to claim it.
        //
        // **That block is deleted.** `KeyBindings::take` compares modifiers for *equality*,
        // so `window.composer.submit` — spelled `enter`, no modifiers — does not match
        // `Shift+Enter` and does not consume it, which leaves the event in the stream for the
        // `TextEdit` drawn two lines below to turn into a newline. The general rule replaced
        // the special case, and the test that proved the special case
        // (`shift_enter_is_a_newline_in_the_composer_not_a_send`) still holds.
        //
        // What the guard taught is kept in `KeyBindings::take`, where it belongs: **a binding
        // is a choice by a person, so it means one specific set of modifiers.**
        let sent = self.keys.take(ui.ctx(), "window.composer.submit");
        let escaped = self.keys.take(ui.ctx(), "window.composer.escape");
        let editor = egui::Rect::from_min_size(
            rect.min,
            egui::vec2(rect.width(), rect.height() - 28.0 * scale),
        );
        let response = ui.put(
            editor,
            egui::TextEdit::multiline(&mut self.draft)
                .id(composer_id())
                .hint_text(hint)
                .desired_width(editor.width() - 8.0)
                .font(egui::TextStyle::Monospace),
        );
        // **egui already built this node; it just had no name.** `WidgetInfo::text_edit`
        // (`egui-0.36.2/src/data/output.rs:697`) fills the value and the hint and never the
        // label, so the composer's node reached the tree as a `MultilineTextInput` with a
        // value and a placeholder and nothing to call it — a reader arriving at it heard an
        // unlabelled field. Naming it is one field on the node egui made, not a second node;
        // see [`name_node`]. The value and the hint are left exactly as egui set them, which
        // is the whole reason that function does not go through `WidgetInfo`.
        name_node(&response, "Composer");
        let send_clicked = ui
            .put(
                egui::Rect::from_min_size(
                    rect.right_bottom() + egui::vec2(-64.0 * scale, -24.0 * scale),
                    egui::vec2(64.0 * scale, 22.0 * scale),
                ),
                egui::Button::new("Send"),
            )
            .clicked();
        if sent || send_clicked {
            self.send_draft();
        }
        // Where the keyboard is, which decides two things: whether the composer may take
        // it, and what `Escape` means. Measured from egui's own focus state rather than
        // tracked here, so it cannot disagree with the widget that holds it.
        let focused_elsewhere = ui
            .ctx()
            .memory(|m| m.focused())
            .is_some_and(|id| id != response.id);
        if escaped {
            if focused_elsewhere {
                // Escape brings the keyboard home instead of silently doing nothing.
                // Before this, a reader who clicked into the file filter could not get
                // back to the composer from the keyboard at all, and `Escape` — which
                // does clear a draft, in the box they could no longer see — appeared
                // broken.
                response.request_focus();
            } else if running {
                if let Some(s) = self.session.as_ref() {
                    s.abort();
                }
            } else {
                self.draft.clear();
            }
        }
        // **Take the keyboard — unless something else already has it.** Without this the
        // window opens with nothing focused, every keystroke goes nowhere, and it looks
        // exactly like a frozen application, which is the failure this repository has
        // chased across four audits in the terminal frontend and the end-to-end window
        // test reproduced here on its first run.
        //
        // It used to be unconditional, on the stated ground that this window had exactly
        // one text input. The file filter is the second one, so the exception the comment
        // promised is the code above: a composer that steals the keyboard back from
        // whichever control the reader is actually using is a composer nobody else can
        // type into.
        //
        // **The pane clause arrived with `F6`, and it is not optional.** Without it this
        // rule fires on every frame in which nothing else holds the keyboard — which is
        // exactly what the sidebar and the editor leave behind — so `F6` would move the
        // ring to the sidebar and the very next line would take the keyboard straight
        // back. The reader would see the sidebar marked and every keystroke land in the
        // composer: a focus indicator naming the wrong region, which is worse than none.
        //
        // **One authority asked one more question.** The alternative — a second focus
        // mechanism for panes, unaware of egui's — is the disagreement this comment
        // exists to prevent, and it is what would have produced the above.
        if !response.has_focus()
            && !focused_elsewhere
            && self.palette.is_none()
            && self.focus == FocusTarget::Agent
        {
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
///
/// **`keys` is a parameter rather than read from the app** because this is a free function and
/// because the two ids it consults — `window.approval.allow` and `window.approval.deny` — are
/// **deliberately different ids from the composer's `Enter`/`Escape`.** The same chord answers a
/// different action depending on which surface holds the keyboard, and which one that is is
/// decided by the drawing order above, not by the table.
pub fn approve_pending(
    session: &IdeSession,
    keys: &crate::keys::KeyBindings,
    ctx: &egui::Context,
) -> Option<(String, bool)> {
    let ask = session.pending_approvals().into_iter().next()?;
    if keys.take(ctx, "window.approval.allow") {
        let decision = PermissionDecision::AllowOnce;
        let resolved = session.resolve(&ask.request_id, decision);
        return Some((ask.request_id, resolved));
    }
    if keys.take(ctx, "window.approval.deny") {
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

    /// **The strip itself — and this is the assertion that fails when it is removed.**
    ///
    /// Verified by control: with `safe_to_show` stubbed to `text.to_string()` it fails on
    /// `left: "ls\u{202e} -R /tmp"`.
    #[test]
    fn an_override_never_survives_into_a_row() {
        assert_eq!(
            IdeApp::safe_to_show("claude\u{202E}-3-opus"),
            "claude-3-opus"
        );
        assert_eq!(
            IdeApp::safe_to_show("src/\u{2066}secrets.rs"),
            "src/secrets.rs"
        );
        // Ordinary text is untouched: an ordinary conversation must never be altered for this.
        assert_eq!(
            IdeApp::safe_to_show("cargo test -p epsilon-desktop"),
            "cargo test -p epsilon-desktop"
        );
    }

    /// Every **route** untrusted text takes into this window goes through the strip.
    ///
    /// The window had none anywhere except its permission prompt (M272), so a message, a
    /// tool's arguments, its output or a diff could each be displayed in an order they were
    /// not in — and `U+202E` reverses the visual order of everything after it.
    ///
    /// **On the frame, and it discriminates only because of the neighbours.** M274 measured that
    /// a frame cannot see a display-steering character at all: `row_text` reads egui's *cell
    /// symbols* and one never becomes a cell, so "the character is absent" is true either way.
    /// What *is* observable is what the removal leaves behind — stripped, `ls -R /tmp` keeps
    /// its space; unstripped, the dropped cell closes the gap and the row reads `ls-R /tmp`.
    #[test]
    fn a_transcript_row_comes_out_stripped_on_the_frame() {
        let mut app = crate::session::Transcript::new(std::path::Path::new("."));
        app.push_notice("ls\u{202E} -R /tmp");
        app.push_user("ls\u{202E} -R /tmp");
        let mut harness = Harness::sized(std::env::temp_dir(), 1440.0, 900.0);
        harness.app.seed_transcript(app);

        let screen = harness.paint().describe();
        assert!(
            screen.contains("ls -R /tmp"),
            "the row must reach the screen with its neighbours where they belong.\n{screen}"
        );
        assert!(
            !screen.contains("ls-R /tmp"),
            "and the gap an unstripped override leaves behind must not be on screen.\n{screen}"
        );
    }

    /// **A command carrying a display-steering character cannot be approved from this window.**
    ///
    /// `U+202E` (RIGHT-TO-LEFT OVERRIDE) reverses the visual order of everything after it, so
    /// a command can be *displayed* as one thing and *be* another. The terminal has refused
    /// `y`/`a` on such an ask since M261, leaving it parked with `n` still working — **refusing
    /// is not a wedge**, because the direction that cannot run the wrong thing must always be
    /// available. This window had no check at all, so the same bytes were safe in one frontend
    /// and not the other.
    ///
    /// **A unit test, and the reason is a finding rather than a preference.** `ChannelApprovalUi`
    /// has no way to *park* an ask — only the agent can produce one, and the agent runs whatever
    /// the model wrote, which the offline provider does not put a `U+202E` in. Two attempts to
    /// reach this through a painted frame both failed the same way: the prompt is drawn from
    /// `pending_approvals()`, so a seeded transcript row has nothing to draw against (M270 hit
    /// exactly that trap). So the rule is pinned where it is written. What is lost is the
    /// *drawn* assertion; what is kept is the part that decides whether `Allow` exists.
    #[test]
    fn a_command_with_a_bidi_override_cannot_be_approved_in_the_window() {
        use epsilon_contracts::agent_event::PermissionRequest;

        let hostile = PermissionRequest {
            request_id: "ask-hostile".into(),
            capability: "bash".into(),
            pattern: "ls".into(),
            detail: serde_json::json!({ "command": "ls\u{202E} -R /" }),
        };
        let (text, honest) = IdeApp::ask_line(&hostile, 400.0, 1.0);

        assert!(
            !text.contains('\u{202E}'),
            "the override must never reach the screen it is trying to lie on: {text}"
        );
        assert!(
            !honest,
            "an ask carrying display-steering characters cannot be approved: {text}"
        );
        assert!(
            text.contains("cannot be approved as shown"),
            "and the window must say so, not merely clean the line and let it be approved: {text}"
        );
        assert!(
            text.contains("[esc] refuse"),
            "refusing must stay available on the line itself: refusing is not a wedge: {text}"
        );
    }

    /// An ordinary ask is untouched — **a guard that fired on every ask would be worse than
    /// none**, since the button would simply always be missing.
    #[test]
    fn an_ordinary_command_is_still_approvable_and_shows_whole() {
        use epsilon_contracts::agent_event::PermissionRequest;

        let ordinary = PermissionRequest {
            request_id: "ask-fine".into(),
            capability: "bash".into(),
            pattern: "echo".into(),
            detail: serde_json::json!({ "command": "echo epsilon-ready" }),
        };
        let (text, honest) = IdeApp::ask_line(&ordinary, 400.0, 1.0);
        assert!(honest, "an ordinary command must remain approvable: {text}");
        assert!(text.contains("allow"), "{text}");
        assert!(
            text.contains("epsilon-ready"),
            "and shows the whole command: {text}"
        );
    }

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
        // **Bounded, like every other window this module paints.** `without_agent` resolves
        // the global config directory itself, so without this a frame painted here would
        // describe the layout the person running the suite happens to have — measured, not
        // assumed: a settings file keyed on this fixture's workspace turns this helper's
        // tests red, and `bounded` is what makes them describe the product instead.
        let _settings = bounded(&mut app);
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

    /// **The figures are the terminal's own strings, not a second spelling of them.**
    ///
    /// The expected value here is built from `epsilon_tui::ui::metrics`'s formatters, so this
    /// test cannot pass against a window that invented its own `ctx`/`↑`/`$` shapes — and the
    /// property it states is the one that matters: a reader moving between the two frontends
    /// reads the same glyphs for the same numbers.
    ///
    /// **Negative control.** With `figures()` replaced by a literal `"ctx 24%/200k"` the second
    /// assertion fails on the missing `↑12k ↓3.4k` and the `$1.42`; with `painter.text` for
    /// the figures removed, `f.region("figures")` is empty and the first assertion fails.
    /// With `figures()` returning the string unshortened, the third assertion fails on a line
    /// that does not end in `…` — which is the failure the budget exists to prevent, and the
    /// one a screenshot shows first.
    #[test]
    fn the_figures_line_says_what_the_terminal_header_says() {
        use epsilon_tui::ui::metrics::Metrics;
        let (dir, _keep) = scratch_workspace("figures");
        let mut h = Harness::sized(dir.clone(), 1440.0, 900.0);

        // **Nothing at all before there is an agent.** A window with no session has no context,
        // no tokens and no bill, and a figure for any of them would be invented. This is the
        // same rule as `the_status_line_does_not_invent_a_provider_or_a_cost`, one region on.
        let bare = h.paint();
        assert!(
            !bare.drew("figures"),
            "a window with no agent must draw no figures at all.\n{}",
            bare.describe()
        );

        let mut m = Metrics::idle();
        m.context_used = Some(48_000);
        m.context_window = Some(200_000);
        m.tokens_in = 12_000;
        m.tokens_out = 3_400;
        m.cost_micro_usd = Some(1_420_000);
        let expected = format!(
            "{}  ·  {}  ·  {}",
            m.context_gauge(),
            // **The token gauge in this window's spelling, not the terminal's (M312).** The shared
            // string is `↑12k ↓3.4k` and this window's font draws the arrows as two empty boxes —
            // the frame showed `ctx 1%/1.0M · □0 □0` in the title bar of every rendered window.
            // Everything else is still the terminal's vocabulary, which is what this assertion was
            // written to hold: a window that invents its own numbers is the failure, and the
            // numbers are still the shared formatter's, only the two arrows are spelled out.
            IdeApp::token_figures(&m),
            m.cost_gauge().expect("a priced session has a cost")
        );
        h.app.metrics = Some(m);

        let full = h.paint();
        let drawn = full.region("figures").join(" ");
        assert_eq!(
            drawn,
            expected,
            "the title bar must speak the terminal's vocabulary, not a private one.\n{}",
            full.describe()
        );

        // A bar too narrow for the whole line says so, rather than clipping the glyphs off the
        // end. A figure half-printed is worse than a missing one, because nobody notices it.
        let mut narrow = Harness::sized(dir, 420.0, 900.0);
        narrow.app.metrics = h.app.metrics.clone();
        let cut = narrow.paint();
        let cut_line = cut.region("figures").join(" ");
        assert!(
            cut_line.chars().count() < drawn.chars().count(),
            "a narrow title bar must shorten the figures, not overflow it: {cut_line:?}"
        );
        assert!(
            cut_line.ends_with('…'),
            "and a shortened figure must be marked as shortened: {cut_line:?}"
        );
    }

    #[test]
    fn the_status_line_does_not_lead_with_window_geometry() {
        // `full` is a word about the window's size that a reader cannot act on, and it used
        // to be on every status line in the product. What a reader must do comes first;
        // geometry comes last, and only when geometry has actually taken something away.
        let mut app = IdeApp::without_agent(Options::for_workspace("/tmp/ws"));
        // Bounded for the same reason as `paint_at`: the seed layout this reads comes from
        // the remembered preferences, and a test must describe the product rather than the
        // machine it runs on.
        let _settings = bounded(&mut app);
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

    /// A transcript that has just recorded `paths` changing, for tests that need a window to
    /// notice an agent edit without running one.
    fn changed(dir: &std::path::Path, paths: &[(&str, bool)]) -> Transcript {
        let mut t = Transcript::new(dir.to_path_buf());
        for (path, created) in paths {
            t.push_change(&crate::diff::Change {
                path: (*path).to_string(),
                lines: Vec::new(),
                created: *created,
                coarse: false,
            });
        }
        t
    }

    /// A file the agent created reaches the tree.
    ///
    /// The window walked the workspace once, at launch, and nothing told it the world had
    /// moved: a turn that writes three new files produced a transcript that said so, and a
    /// tree that did not have them. The reader is looking for the file the agent just made and
    /// it is not there — which is the same "nothing happened" state this crate keeps refusing
    /// to ship, reached by asking the agent to do its job.
    #[test]
    fn a_file_the_agent_created_appears_in_the_tree() {
        let (dir, _keep) = scratch_workspace("created");
        let mut h = Harness::new(dir.clone());
        std::fs::write(dir.join("gamma.rs"), "fn gamma() {}\n").expect("a new file");
        h.app.seed_transcript(changed(&dir, &[("gamma.rs", true)]));
        let f = h.paint();

        let files = f
            .region("workspace")
            .iter()
            .find(|r| r.contains("files"))
            .copied()
            .unwrap_or_default();
        assert!(
            files.contains("3 files"),
            "the tree must include the file the agent just wrote, says {files:?}.\n{}",
            f.describe()
        );
    }

    /// A file the agent edited shows its **new** contents.
    ///
    /// The editor holds whatever the file said when it was opened. After an edit the reader
    /// was looking at the text the edit replaced — the worst kind of stale, because it looks
    /// current and is confidently wrong.
    #[test]
    fn an_edited_file_shows_its_new_contents_without_being_reopened() {
        let (dir, _keep) = scratch_workspace("edited");
        let mut h = Harness::new(dir.clone());
        h.app.open_file("alpha.rs");
        std::fs::write(dir.join("alpha.rs"), "fn alpha() { /* rewritten */ }\n").expect("write");
        h.app.seed_transcript(changed(&dir, &[("alpha.rs", false)]));
        let f = h.paint();

        let editor = f.region("editor");
        assert!(
            editor.iter().any(|r| r.contains("rewritten")),
            "the editor must show the text the agent wrote, not the text it replaced: \
             {editor:?}.\n{}",
            f.describe()
        );
    }

    /// A file the agent deleted is closed and **said out loud**.
    ///
    /// Keeping its last contents on screen would be showing the reader something that is not
    /// there. A missing file is not an error to swallow either — it is the ordinary result of
    /// a refactor, and the window has to name it.
    #[test]
    fn a_deleted_file_is_closed_and_named_rather_than_left_on_screen() {
        let (dir, _keep) = scratch_workspace("deleted");
        let mut h = Harness::new(dir.clone());
        h.app.open_file("beta.rs");
        std::fs::remove_file(dir.join("beta.rs")).expect("remove");
        h.app.seed_transcript(changed(&dir, &[("beta.rs", false)]));
        let f = h.paint();

        assert!(
            !f.region("editor").iter().any(|r| r.contains("beta.rs")),
            "a deleted file must not still be presented as open: {:?}.\n{}",
            f.region("editor"),
            f.describe()
        );
        assert!(
            f.region("agent").iter().any(|r| r.contains("beta.rs")),
            "and the reader must be told which file went, in words they can act on.\n{}",
            f.describe()
        );
    }

    /// The file filter takes the keyboard, and `Escape` brings it back.
    ///
    /// The composer requested focus unconditionally, on the stated ground that this window had
    /// exactly one text input — and the file filter is the second one. So the filter could be
    /// clicked, focused, and then had the keyboard pulled straight back out of it on the very
    /// next frame: a control that takes focus and cannot be typed into. `Escape` was worse,
    /// because it cleared a draft in a box the reader could no longer see, which reads as a
    /// broken key rather than a focus change.
    #[test]
    fn the_file_filter_takes_the_keyboard_and_escape_brings_it_back() {
        let (dir, _keep) = scratch_workspace("focus");
        let mut h = Harness::new(dir);

        let first = h.paint();
        let filter = first
            .hotspot("file-filter")
            .expect("the filter records where it was drawn");

        // Click it, then type. The text must land in the filter, not the draft.
        h.click(filter.center());
        h.type_text("alpha");
        assert_eq!(
            h.app.tree_query, "alpha",
            "typing must reach the control the reader clicked"
        );
        assert!(
            h.app.draft.is_empty(),
            "the composer must not have taken the keyboard back: {:?}",
            h.app.draft
        );

        // Escape hands the keyboard back, so the reader is never trapped.
        h.press(egui::Key::Escape);
        h.type_text("hello");
        assert_eq!(
            h.app.draft, "hello",
            "after Escape the keys must go to the composer again"
        );
    }

    /// Clicking something, then typing, still types into the composer.
    ///
    /// M249 stopped the composer from stealing the keyboard unconditionally, which traded one
    /// silent failure for a possible new one: **egui gives a clicked `Button` keyboard focus**,
    /// and this window is full of buttons. If a tab took the keyboard, a reader who clicked a
    /// file and then typed would watch nothing arrive — which is the same "the window ignored
    /// me" state M249 was fixing, arriving by the door the fix opened.
    ///
    /// So both kinds of clickable row are asserted here: a tree row (a bare click allocation)
    /// and a tab (a real `Button`). Neither may take the keyboard from the composer.
    #[test]
    fn clicking_a_file_then_typing_still_reaches_the_composer() {
        let (dir, _keep) = scratch_workspace("click-file");
        let mut h = Harness::new(dir.clone());
        let first = h.paint();
        let row = first
            .hotspot(&crate::workspace::hotspot_id("alpha.rs"))
            .expect("the tree records where each row was drawn");

        h.click(row.center());
        assert!(h.app.open.is_some(), "the click must still open the file");
        h.type_text("hello");
        assert_eq!(
            h.app.draft, "hello",
            "clicking a file must not take the keyboard away from the composer"
        );
    }

    #[test]
    fn clicking_a_tab_then_typing_still_reaches_the_composer() {
        let (dir, _keep) = scratch_workspace("click-tab");
        let mut h = Harness::new(dir);
        h.app.open_file("alpha.rs");
        h.app.open_file("beta.rs");
        let first = h.paint();
        let tab = first
            .hotspot("tab:alpha.rs")
            .expect("the strip records each tab");

        h.click(tab.center());
        assert_eq!(
            h.app.open.as_ref().map(|f| f.path.as_str()),
            Some("alpha.rs"),
            "the click must still switch the editor"
        );
        h.type_text("hello");
        assert_eq!(
            h.app.draft, "hello",
            "clicking a tab must not take the keyboard away from the composer"
        );
    }

    /// A change with real lines in it, for tests that review a diff rather than assert on a
    /// summary.
    fn edited(dir: &std::path::Path, path: &str) -> Transcript {
        let mut t = Transcript::new(dir.to_path_buf());
        t.push_change(&crate::diff::Change {
            path: path.to_string(),
            lines: vec![
                crate::diff::DiffLine::Context("fn alpha() {".into()),
                crate::diff::DiffLine::Removed("    old();".into()),
                crate::diff::DiffLine::Added("    fresh();".into()),
                crate::diff::DiffLine::Context("}".into()),
            ],
            created: false,
            coarse: false,
        });
        t
    }

    /// The rectangle of the first clickable "changed" row in a frame.
    fn change_row(f: &FrameSummary) -> egui::Rect {
        f.hotspots
            .iter()
            .find(|(k, _)| k.starts_with("change:"))
            .map(|(_, r)| *r)
            .expect("a changed row is something to click")
    }

    /// Clicking a "changed" row opens the change, in the pane, as a review.
    ///
    /// The transcript row used to say *"… more changed lines — open the file"* with nothing to
    /// open: a review instruction beside no control. And a report is not a review anyway — the
    /// rows are capped, interleaved with the conversation, and gone when the transcript trims,
    /// so the whole change was never on screen at once.
    #[test]
    fn clicking_a_changed_row_opens_the_change_for_review() {
        let (dir, _keep) = scratch_workspace("review");
        let mut h = Harness::new(dir.clone());
        h.app.seed_transcript(edited(&dir, "alpha.rs"));
        let row = change_row(&h.paint());
        h.click(row.center());
        let after = h.paint();

        let editor = after.region("editor");
        assert!(
            editor.iter().any(|t| t.contains("+1") && t.contains("−1")),
            "the review must say how much changed: {editor:?}.\n{}",
            after.describe()
        );
        assert!(
            editor.iter().any(|t| t.contains("-    old();")),
            "the removed line must be on screen, not merely counted: {editor:?}"
        );
        assert!(
            editor.iter().any(|t| t.contains("+    fresh();")),
            "and so must the added one: {editor:?}"
        );
    }

    /// A coarse change says so, rather than presenting a block replacement as a line diff.
    #[test]
    fn a_coarse_change_is_labelled_as_one() {
        let (dir, _keep) = scratch_workspace("coarse");
        let mut h = Harness::new(dir.clone());
        let mut t = Transcript::new(dir.clone());
        t.push_change(&crate::diff::Change {
            path: "alpha.rs".into(),
            lines: vec![crate::diff::DiffLine::Added("whatever".into())],
            created: false,
            coarse: true,
        });
        h.app.seed_transcript(t);
        let row = change_row(&h.paint());
        h.click(row.center());
        let after = h.paint();

        assert!(
            after
                .region("editor")
                .iter()
                .any(|t| t.contains("replaced as a block")),
            "a diff that is not line-accurate must say so.\n{}",
            after.describe()
        );
    }

    /// Opening a file ends a review.
    ///
    /// One way back, not two: the reader clicks a file to read it, and leaving a diff on screen
    /// because they did would be the window ignoring its own click. No separate "close" control,
    /// because the tree and the tab strip are already the answer.
    #[test]
    fn opening_a_file_ends_the_review() {
        let (dir, _keep) = scratch_workspace("endreview");
        let mut h = Harness::new(dir.clone());
        h.app.open_file("alpha.rs");
        h.app.seed_transcript(edited(&dir, "alpha.rs"));
        let row = change_row(&h.paint());
        h.click(row.center());
        assert!(
            h.paint()
                .region("editor")
                .iter()
                .any(|t| t.contains("-    old();")),
            "precondition: the review is on screen"
        );

        h.app.open_file("beta.rs");
        let after = h.paint();
        assert!(
            !after.region("editor").iter().any(|t| t.contains("old();")),
            "opening a file must show that file, not the diff.\n{}",
            after.describe()
        );
        assert!(
            after.region("editor").iter().any(|t| t.contains("beta.rs")),
            "and it must be the file that was asked for.\n{}",
            after.describe()
        );
    }

    /// Did the frame draw this exact palette row?
    ///
    /// A helper rather than four spellings of one lookup. `region` yields `&str`, and a
    /// palette test that passes because `alpha` matched `alpha.rs.bak` has measured nothing
    /// — so the comparison is exact, by name.
    fn palette_has(f: &FrameSummary, label: &str) -> bool {
        f.region("palette").contains(&label)
    }

    /// Every rectangle the frame **stroked** in `color`.
    ///
    /// The focus ring is the one thing in this window whose entire job is to be seen,
    /// so a test that asks "does the frame say the sidebar has focus" has measured the
    /// wrong thing: `FrameSummary::rows` is filled in by the code that draws, and code
    /// that draws can be deleted while the row it pushed stays. This reads the shape
    /// list a renderer would have been handed, which is one step closer to the pixels
    /// than anything else this crate can reach — and on this machine it is the last
    /// step before the GPU that exists (`a_frame_tessellates_into_geometry_a_renderer_
    /// could_draw`).
    fn strokes_in(full: &egui::FullOutput, color: egui::Color32) -> Vec<egui::Rect> {
        full.shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::Rect(r) if r.stroke.color == color && r.stroke.width > 0.0 => {
                    Some(r.rect)
                }
                _ => None,
            })
            .collect()
    }

    /// Is the one rectangle that looks like `want` the only focus ring on the frame?
    ///
    /// **Exactly one.** A ring that accumulates — one per pane ever focused — is not a
    /// focus indicator, it is decoration, and it leaves the reader guessing which of
    /// several lit borders is the live one.
    fn only_ring_is(full: &egui::FullOutput, color: egui::Color32, want: egui::Rect) -> bool {
        let rings = strokes_in(full, color);
        rings.len() == 1
            && (rings[0].min - want.min).length() < 0.5
            && (rings[0].max - want.max).length() < 0.5
    }

    /// Where a pane was drawn, named — so a test can say *which pane* rather than
    /// carrying a rectangle the layout may move under it.
    fn pane_rect(layout: &IdeLayout, pane: &str) -> egui::Rect {
        match pane {
            "sidebar" => layout.sidebar,
            "center" => layout.center,
            "agent" => layout.agent,
            other => panic!("{other} is not a pane this window has"),
        }
    }

    /// `Shift+Enter` puts a **newline** in the draft, which is what the composer's own
    /// placeholder promises — `ask anything · enter sends · shift+enter for a new line`.
    ///
    /// **Found by reading the promise against the code, not by a failing test.** egui's
    /// `InputState::consume_key` matches modifiers **logically**, so the composer's
    /// `consume_key(NONE, Enter)` also claimed `Shift+Enter` and the draft was **sent**.
    /// The sentence on screen and the behaviour under the hand disagreed, and nothing in
    /// the suite pressed the chord — every composer test pressed plain `Enter`.
    ///
    /// **Driven through the real key path** with a real `Modifiers::SHIFT`, because the
    /// whole defect lives in which modifiers egui is told about.
    #[test]
    fn shift_enter_is_a_newline_in_the_composer_not_a_send() {
        let (ws, _keep) = scratch_workspace("shift-enter");
        let (session, notices) = faux_session(&ws);
        let mut h = Harness::with_session(ws, session, notices);
        // One frame first: the composer takes its focus with `request_focus`, and egui
        // honours that on the **next** frame. Pressing on frame one is pressing at a
        // widget that does not yet hold the keyboard.
        let _ = h.paint();
        // **Typed first, on purpose.** If the composer does not hold the keyboard then
        // this fails on `"a\n"` rather than on the newline, and the reader learns the
        // problem is focus and not the chord. A test that pressed only `Shift+Enter`
        // would report the same empty draft for two different reasons.
        h.type_text("a");
        let _ = h.paint();
        assert_eq!(
            h.app.draft,
            "a",
            "the typed character must be in the draft before any key is pressed. \
             egui focus: {:?}",
            h.ctx.memory(|m| m.focused())
        );

        let frame = h.chord(egui::Modifiers::SHIFT, egui::Key::Enter);
        assert_eq!(
            h.app.draft,
            "a\n",
            "shift+enter must leave a newline in the draft, not send it. \
             The placeholder promises exactly this.\negui focus: {:?}\nrunning: {:?}\nframe: {}",
            h.ctx.memory(|m| m.focused()),
            h.app.session.as_ref().map(|s| s.running()),
            frame.describe()
        );
        assert!(
            !h.app.session.as_ref().map(|s| s.running()).unwrap_or(false),
            "and nothing may be sent — this run was not asked for"
        );
        // The sentence that promised this is still on screen, and now true. A first
        // version of this test asserted the draft row carried the terminal's `▸` cursor
        // marker — which the **terminal** draws and this window never did, so it failed
        // against a build that had already fixed the defect.
        assert!(
            frame
                .region("composer")
                .iter()
                .any(|t| t.contains("shift+enter for a new line")),
            "the placeholder still promises it, so it must still say it. {}",
            frame.describe()
        );
    }

    /// The **sessions** sidebar answers the keyboard — a list a keyboard reader can enter
    /// and use.
    ///
    /// **It used to be a keyboard trap, and that is the WCAG it breaks.** `pane_keys`
    /// gated its arrows and `Enter` on `SidebarView::Files`, so `F6` into a Sessions
    /// sidebar drew a focus ring over a list where no key did anything at all. The list
    /// was clickable and nothing else, and every assertion about it in the suite was a
    /// pointer.
    ///
    /// Asserted on **the drawn frame**, twice over: that the cursor is *visible* (a
    /// cursor a reader cannot see is not a cursor), and that `Enter` resumes the row it
    /// is on. The first assertion is the one a field holding a path could not satisfy.
    ///
    /// The walk is a loop rather than a fixed number of presses **because the list is
    /// newest-first and this window's own session is in it** — one press would land on a
    /// row chosen by clock time rather than by anything the test states. Walking down
    /// until the wanted row is under the cursor is what a reader does, and it encodes no
    /// guess about sort order.
    #[test]
    fn the_sessions_sidebar_answers_the_keyboard_and_shows_where_it_is() {
        let (ws, _keep) = scratch_workspace("sessions-keys");

        // A second, older session with a message in it, so `Enter` has somewhere to go.
        // It lives in the sessions root — the flat legacy half of the discovery
        // union — because no session has ever lived directly in a workspace root,
        // and a fixture anywhere else would pass against a listing that cannot
        // see real files (proven when the root fix turned this green by moving
        // nothing else).
        let sessions_root = epsilon_runtime::sessions_root(&ws);
        std::fs::create_dir_all(&sessions_root).expect("sessions root");
        let older = sessions_root.join("older.jsonl");
        let mut store =
            epsilon_session::SessionStore::open_in(older.clone(), &ws).expect("a store opens");
        store
            .append(epsilon_contracts::session_entry::EntryKind::Message {
                message: epsilon_contracts::message::Message::Assistant(
                    epsilon_contracts::message::AssistantMessage {
                        id: "m-old".into(),
                        timestamp_ms: 0,
                        provider: "faux".into(),
                        model_id: "faux-1".into(),
                        content: vec![epsilon_contracts::content::ContentBlock::text(
                            "a memorable line from last time",
                        )],
                        stop_reason: epsilon_contracts::message::StopReason::EndTurn,
                        error_message: None,
                        usage: Default::default(),
                    },
                ),
                api_request_sha256: None,
            })
            .expect("the message lands");
        drop(store);
        let (session, notices) = faux_session(&ws);
        let mut h = Harness::with_session(ws, session, notices);

        h.app.set_sidebar_view(SidebarView::Sessions);
        // Walk the focus ring onto the sidebar, the way a reader would. This frame is
        // also the "before" one, so the assertion below is made on a painted screen
        // rather than on a field.
        let before = h.chord(egui::Modifiers::NONE, egui::Key::F6);
        assert_eq!(
            h.app.focus,
            FocusTarget::Sidebar,
            "one F6 from the agent pane lands on the sidebar"
        );

        assert!(
            before
                .region("sessions")
                .iter()
                .any(|t| t.contains("older")),
            "the stored session must be listed before the keyboard is asked to move.\n{}",
            before.describe()
        );
        assert!(
            !before.region("sessions").iter().any(|t| t.contains('▸')),
            "and no cursor exists yet — arriving is what puts one there.\n{}",
            before.describe()
        );

        // Walk down until the wanted row is under the keyboard. Bounded, and the bound
        // is a failure rather than a silent exit.
        for _ in 0..8 {
            if h.app.session_cursor.as_deref() == Some(older.as_path()) {
                break;
            }
            let frame = h.chord(egui::Modifiers::NONE, egui::Key::ArrowDown);
            assert!(
                frame.region("sessions").iter().any(|t| t.contains('▸')),
                "the keyboard cursor must be DRAWN, or it is a cursor nobody can see.\n{}",
                frame.describe()
            );
        }
        assert_eq!(
            h.app.session_cursor.as_deref(),
            Some(older.as_path()),
            "the cursor must be able to reach the row the reader is looking for"
        );

        // and `Enter` does what clicking that row does.
        let resumed = h.chord(egui::Modifiers::NONE, egui::Key::Enter);
        assert!(
            resumed
                .region("agent")
                .iter()
                .any(|t| t.contains("a memorable line from last time")),
            "Enter must resume the row the cursor is on, through the same call the click \
             path uses.\n{}",
            resumed.describe()
        );
    }

    /// `F6` walks the ring left-to-right across the panes, and `Shift+F6` walks it back.
    ///
    /// **Driven through the real key path** — `egui::Event::Key` into `paint` — because
    /// a pane-cycling rule that is correct in a function and never called is not a rule.
    /// **Asserted on the painted shapes**, for the reason [`strokes_in`] gives: the
    /// alternative is reading a field, and a field can hold `Sidebar` for ever while the
    /// window shows nothing at all.
    ///
    /// The order asserted is the one the panes are laid out in — sidebar, editor,
    /// agent, then wrapping — because WCAG 2.4.3 asks for an order that follows the
    /// visual one and this window has only one.
    #[test]
    fn f6_walks_the_focus_ring_across_the_panes_and_shift_f6_walks_it_back() {
        let (dir, _keep) = scratch_workspace("pane-cycle");
        let mut h = Harness::new(dir);
        let ring = h.app.theme.border_focus;

        // A window opens on the agent pane, because that is where its only text input
        // is and where the composer's focus rule has always put the keyboard.
        let first = h.full_frame();
        let agent = h.app.layout.agent;
        assert!(
            only_ring_is(&first, ring, agent),
            "the window must open with the agent pane marked. Stroked {:?}, expected {agent:?}",
            strokes_in(&first, ring)
        );

        for (press, pane) in [
            ("F6", "sidebar"),
            ("F6", "center"),
            ("F6", "agent"),
            ("Shift+F6", "center"),
            ("Shift+F6", "sidebar"),
            ("Shift+F6", "agent"),
        ] {
            let (modifiers, key) = if press.starts_with("Shift") {
                (egui::Modifiers::SHIFT, egui::Key::F6)
            } else {
                (egui::Modifiers::NONE, egui::Key::F6)
            };
            let (_frame, full) = h.chord_full(modifiers, key);
            let want = pane_rect(&h.app.layout, pane);
            assert!(
                only_ring_is(&full, ring, want),
                "{press} must leave the ring on the {pane} pane at {want:?}. Stroked {:?}",
                strokes_in(&full, ring)
            );
        }
    }

    /// The ring is **painted**, not merely decided.
    ///
    /// **This is the assertion the other one cannot make.** Above, `F6` could be
    /// deleted and the ring painted on whatever the last value was, and the walk would
    /// fail — but the *painting* could be deleted on its own and every other pane test
    /// in the crate would stay green. Here the pane is set directly, with no key
    /// involved, so the only thing that can put the rectangle on the frame is the
    /// drawing.
    ///
    /// Negative control run: with the `rect_stroke` in [`IdeApp::draw_focus_ring`]
    /// commented out, this fails with `Stroked []`; restored, it passes.
    #[test]
    fn the_focus_ring_is_painted_and_the_unfocused_panes_carry_nothing() {
        let (dir, _keep) = scratch_workspace("pane-ring");
        let mut h = Harness::new(dir);
        let ring = h.app.theme.border_focus;

        for (focus, pane) in [
            (FocusTarget::Sidebar, "sidebar"),
            (FocusTarget::Editor, "center"),
            (FocusTarget::Agent, "agent"),
        ] {
            h.app.set_focus(focus);
            let full = h.full_frame();
            let want = pane_rect(&h.app.layout, pane);
            assert!(
                only_ring_is(&full, ring, want),
                "the {pane} pane must wear the ring and nothing else may. Stroked {:?}, \
                 expected exactly {want:?}",
                strokes_in(&full, ring)
            );
        }
    }

    /// `ArrowDown` walks the file tree and `Enter` opens the row under the cursor.
    ///
    /// **The tree rows were click-only until now**: `allocate_exact_size(…,
    /// Sense::click())` with `painter.text`, so a reader using nothing but the
    /// keyboard could not reach a file at all except through `Ctrl+P` — and the tree is
    /// the pane that disappears first as the window narrows, which is where a keyboard
    /// reader is most likely to be.
    ///
    /// Asserted on the frame twice over: the cursor's rectangle is recorded as
    /// `tree-cursor:<path>` when the row is drawn (so the *mark* is on screen), and the
    /// editor's own rows carry the file's **text**, so "it opened" is not merely "the
    /// field was set".
    #[test]
    fn arrow_keys_walk_the_tree_and_enter_opens_the_row_under_the_cursor() {
        let (dir, _keep) = scratch_workspace("tree-keys");
        let mut h = Harness::new(dir);
        h.app.set_focus(FocusTarget::Sidebar);

        let before = h.paint();
        assert!(
            before.hotspot("tree-cursor:alpha.rs").is_none(),
            "a tree nobody has walked has no cursor.\n{}",
            before.describe()
        );

        let first = h.press(egui::Key::ArrowDown);
        assert!(
            first.hotspot("tree-cursor:alpha.rs").is_some(),
            "one ArrowDown must mark the first row, and mark it **on the frame**.\n{}",
            first.describe()
        );
        assert!(
            first.hotspot("tree-cursor:beta.rs").is_none(),
            "and only one row at a time.\n{}",
            first.describe()
        );
        let opened = h.press(egui::Key::Enter);
        assert!(
            opened
                .region("editor")
                .iter()
                .any(|t| t.contains("fn alpha() {}")),
            "Enter must open the row under the cursor, the way a click on it does.\n{}",
            opened.describe()
        );

        // And the cursor really walks: the next row is a different file, with different
        // contents, so "it opened *a* file" cannot pass for "it opened *the* one".
        let _ = h.press(egui::Key::ArrowDown);
        let second = h.press(egui::Key::Enter);
        assert!(
            second
                .region("editor")
                .iter()
                .any(|t| t.contains("fn beta() {}")),
            "the second ArrowDown must move the cursor, and Enter must follow it.\n{}",
            second.describe()
        );
    }

    /// `Escape` in the sidebar brings the keyboard home **without eating the draft**.
    ///
    /// The trap: the composer's own `Escape` clears the draft when nothing else holds
    /// the keyboard, and leaving the sidebar's key to fall through to it means a reader
    /// who pressed `Escape` to get out of the sidebar loses their words — `AGENTS.md`
    /// §6 calls that data loss, and it is the kind no failure message would explain.
    #[test]
    fn escape_leaves_the_sidebar_for_the_composer_and_leaves_the_draft_alone() {
        let (dir, _keep) = scratch_workspace("tree-escape");
        let mut h = Harness::new(dir);
        h.app.set_draft("half a thought");
        h.app.set_focus(FocusTarget::Sidebar);
        let _ = h.paint();

        let (home, full) = h.chord_full(egui::Modifiers::NONE, egui::Key::Escape);
        assert_eq!(
            h.app.draft,
            "half a thought",
            "Escape in the sidebar is a navigation key, not an undo.\n{}",
            home.describe()
        );
        let ring = h.app.theme.border_focus;
        assert!(
            only_ring_is(&full, ring, h.app.layout.agent),
            "and the ring must come home with the keyboard. Stroked {:?}",
            strokes_in(&full, ring)
        );
        h.type_text("!");
        assert_eq!(
            h.app.draft, "half a thought!",
            "the keys must reach the composer"
        );
    }

    /// The palette keeps its own keys, and the tree cursor does not move underneath it.
    ///
    /// **The trap this closes, named.** `ArrowDown` means "next palette row" while the
    /// palette is open and "next tree row" when the sidebar has focus. Both handlers
    /// read the same event, so a pane handler placed in the middle moves two things with
    /// one keystroke: the palette opens the wrong file, and the reader has no way to
    /// tell which of the two answers they got.
    ///
    /// Set up by walking the tree **first**, so the cursor has somewhere to *not* move:
    /// a test that starts with no cursor proves only that a cursor was never created.
    #[test]
    fn the_palette_owns_its_arrows_and_the_tree_cursor_stays_where_it_was() {
        let (dir, _keep) = scratch_workspace("palette-vs-tree");
        let mut h = Harness::new(dir);
        h.app.set_focus(FocusTarget::Sidebar);
        let _ = h.paint();
        let _ = h.press(egui::Key::ArrowDown);
        let _ = h.press(egui::Key::Enter);
        assert_eq!(
            h.app.open.as_ref().map(|f| f.path.as_str()),
            Some("alpha.rs"),
            "the setup: the tree cursor is on alpha.rs and alpha.rs is open"
        );

        let opened = h.chord(egui::Modifiers::CTRL, egui::Key::P);
        assert!(
            opened.drew("palette"),
            "Ctrl+P must still open the palette with the sidebar focused.\n{}",
            opened.describe()
        );
        // **The palette's own arrow.** One `ArrowDown` from its selection of zero.
        let moved = h.press(egui::Key::ArrowDown);
        assert!(
            moved.hotspot("tree-cursor:alpha.rs").is_some(),
            "the tree cursor must not move while the palette owns the arrows.\n{}",
            moved.describe()
        );
        let chosen = h.press(egui::Key::Enter);
        assert!(
            chosen
                .region("editor")
                .iter()
                .any(|t| t.contains("fn beta() {}")),
            "Enter must open the **palette's** selection — the one ArrowDown moved — not \
             the tree's.\n{}",
            chosen.describe()
        );
    }
    /// `Ctrl+P` opens a palette that lists files and commands.
    ///
    /// The window had **two** keys before this — `Enter` and `Escape`, both the composer's.
    /// Every file was reachable only by clicking through the tree, and the tree is the pane
    /// that disappears first as the window narrows. A reader who cannot type a name has to
    /// know where the file is already, which is the browser, not the IDE.
    #[test]
    fn control_p_opens_a_palette_of_files_and_commands() {
        let (dir, _keep) = scratch_workspace("palette");
        let mut h = Harness::new(dir);
        let _ = h.paint();

        let opened = h.chord(egui::Modifiers::CTRL, egui::Key::P);
        assert!(
            palette_has(&opened, "alpha.rs"),
            "the palette must list the workspace's files.\n{}",
            opened.describe()
        );
        assert!(
            palette_has(&opened, "reload tree"),
            "and the window's own commands, not only files.\n{}",
            opened.describe()
        );
    }

    /// The palette offers every verb the window answers — derived, not listed.
    ///
    /// The command table below is hand-written for views and goals, but the
    /// slash verbs come from the shared `SUPPORTED` declaration: a verb the
    /// window learns must reach the palette without a second edit, or the
    /// palette is a menu that forgot the kitchen grew. RED first: twelve
    /// answered verbs, eight offered rows.
    #[test]
    fn the_palette_offers_every_verb_the_window_answers() {
        use crate::commands::SUPPORTED;
        let (dir, _keep) = scratch_workspace("palette-verbs");
        let mut h = Harness::new(dir);
        let _ = h.paint();
        let opened = h.chord(egui::Modifiers::CTRL, egui::Key::P);
        for head in SUPPORTED {
            let label = format!("/{head}");
            assert!(
                palette_has(&opened, &label),
                "the palette must offer {label}.\n{}",
                opened.describe()
            );
        }
    }

    /// Choosing a verb row runs it through the same dispatch the composer uses.
    ///
    /// The row must not carry its own copy of what `/help` does: it runs the
    /// head, and the head decides. `/help` opens the palette, so running it
    /// from the palette leaves the palette open on the commands.
    #[test]
    fn choosing_a_verb_row_runs_it() {
        let (dir, _keep) = scratch_workspace("palette-verb-run");
        let mut h = Harness::new(dir);
        let _ = h.paint();
        assert!(
            h.app.run_palette_command("help", ""),
            "the palette must resolve the help verb"
        );
        assert!(
            h.app.palette.is_some(),
            "running /help from the palette opens the palette"
        );
    }

    /// `/help` opens the palette — the window's honest equivalent of the terminal's
    /// command reference.
    ///
    /// The palette lists every file and every command the window answers, so it is
    /// what "show me what I can do" means here. Before this arm, `/help` was
    /// refused by name like every other unbuilt verb; the refusal was honest, and
    /// this is the verb that replaces it.
    #[test]
    fn slash_help_opens_the_palette() {
        let (dir, _keep) = scratch_workspace("palette-help");
        let mut h = Harness::new(dir);
        let _ = h.paint();
        assert!(h.app.palette.is_none(), "the palette starts closed");

        let outcome = h.app.try_command("/help");
        let crate::commands::Outcome::Ran(said) = outcome else {
            panic!("/help must run, not refuse or send: {outcome:?}");
        };
        assert!(
            said.contains("palette"),
            "the answer names what opened: {said:?}"
        );
        assert!(h.app.palette.is_some(), "/help opens the palette");

        let opened = h.paint();
        assert!(
            palette_has(&opened, "reload tree"),
            "and the opened palette lists the window's commands.\n{}",
            opened.describe()
        );
    }

    /// `/new` starts a session file and switches to it.
    ///
    /// Same act as the terminal's (M221): a file beside the others, created
    /// under a lease, handed over through `switch_session` — the path the
    /// sidebar's clicks already take. The previous session is kept, not
    /// replaced. Refuses while a run is in flight, like the terminal's
    /// "new: busy" (which is itself unpinned there — this guard mirrors it
    /// rather than re-proving it).
    #[test]
    fn slash_new_starts_a_session_file_and_switches_to_it() {
        let (ws, _keep) = scratch_workspace("slash-new");
        let (session, notices) = faux_session(&ws);
        let mut h = Harness::with_session(ws.clone(), session, notices);
        let before = h
            .app
            .session
            .as_ref()
            .and_then(|s| s.session_path())
            .expect("the attached session holds a file");
        let listed_before = h
            .app
            .session
            .as_ref()
            .map(|s| s.sessions(&ws).len())
            .unwrap_or(0);

        let outcome = h.app.try_command("/new");
        let crate::commands::Outcome::Ran(said) = outcome else {
            panic!("/new must run, not refuse or send: {outcome:?}");
        };
        assert!(
            said.contains("new session"),
            "the answer names what happened: {said:?}"
        );

        let after = h
            .app
            .session
            .as_ref()
            .and_then(|s| s.session_path())
            .expect("still attached after the switch");
        assert_ne!(before, after, "/new must move to a new file, not stay put");
        assert!(
            after.exists(),
            "the new session file exists: {}",
            after.display()
        );
        let listed_after = h
            .app
            .session
            .as_ref()
            .map(|s| s.sessions(&ws).len())
            .unwrap_or(0);
        assert!(
            listed_after > listed_before,
            "the previous session is kept: {listed_before} -> {listed_after}"
        );
    }

    /// `/new` with no agent names what's missing instead of doing half the verb.
    ///
    /// Creating the file without a session to switch would leave a lease nobody
    /// holds and a file nobody opens — half a verb. The `/session` command's
    /// precedent ("no durable session file attached") is the shape: name the
    /// missing piece in words.
    #[test]
    fn slash_new_without_an_agent_names_what_is_missing() {
        let (ws, _keep) = scratch_workspace("slash-new-no-agent");
        let mut h = Harness::new(ws);
        let outcome = h.app.try_command("/new");
        let crate::commands::Outcome::Ran(said) = outcome else {
            panic!("/new without an agent must answer, not refuse or send: {outcome:?}");
        };
        assert!(
            said.contains("no agent"),
            "names the missing piece: {said:?}"
        );
    }

    /// `/resume <path>` switches to that session file.
    ///
    /// Same act as the terminal's path form: refuse while busy, name a path
    /// with no file in words, otherwise hand over through `switch_session` —
    /// the path the sidebar's clicks already take. Resume is a handover, not
    /// a replacement: the previous session stays stored.
    #[test]
    fn slash_resume_with_a_path_switches_to_that_file() {
        let (ws, _keep) = scratch_workspace("slash-resume");
        // A second session with a message in it, in the sessions root where
        // discovery reads (flat legacy half).
        let sessions_root = epsilon_runtime::sessions_root(&ws);
        std::fs::create_dir_all(&sessions_root).expect("sessions root");
        let other = sessions_root.join("other.jsonl");
        let mut store =
            epsilon_session::SessionStore::open_in(other.clone(), &ws).expect("a store opens");
        store
            .append(epsilon_contracts::session_entry::EntryKind::Message {
                message: epsilon_contracts::message::Message::Assistant(
                    epsilon_contracts::message::AssistantMessage {
                        id: "m-other".into(),
                        timestamp_ms: 0,
                        provider: "faux".into(),
                        model_id: "faux-1".into(),
                        content: vec![epsilon_contracts::content::ContentBlock::text(
                            "a line from the other session",
                        )],
                        stop_reason: epsilon_contracts::message::StopReason::EndTurn,
                        error_message: None,
                        usage: Default::default(),
                    },
                ),
                api_request_sha256: None,
            })
            .expect("the message lands");
        drop(store);
        let (session, notices) = faux_session(&ws);
        let mut h = Harness::with_session(ws, session, notices);

        let outcome = h.app.try_command(&format!("/resume {}", other.display()));
        let crate::commands::Outcome::Ran(said) = outcome else {
            panic!("/resume <path> must run, not refuse or send: {outcome:?}");
        };
        assert!(
            said.contains("resumed"),
            "the answer names what happened: {said:?}"
        );
        let after = h
            .app
            .session
            .as_ref()
            .and_then(|s| s.session_path())
            .expect("still attached after the switch");
        assert_eq!(
            after.file_name(),
            other.file_name(),
            "resume moves to the named file"
        );
        assert!(after.exists(), "and that file exists: {}", after.display());
    }

    /// A resumed session with a truncated reply shows the notice (P-127, pixels).
    ///
    /// The reducer tests pin the row; this pins the row reaching the screen
    /// through the path a reader actually takes — resume a file, look at it.
    #[test]
    fn a_resumed_truncated_reply_reaches_the_screen() {
        let (ws, _keep) = scratch_workspace("truncated-screen");
        let sessions_root = epsilon_runtime::sessions_root(&ws);
        std::fs::create_dir_all(&sessions_root).expect("sessions root");
        let other = sessions_root.join("cut.jsonl");
        let mut store =
            epsilon_session::SessionStore::open_in(other.clone(), &ws).expect("a store opens");
        store
            .append(epsilon_contracts::session_entry::EntryKind::Message {
                message: epsilon_contracts::message::Message::Assistant(
                    epsilon_contracts::message::AssistantMessage {
                        id: "m-cut".into(),
                        timestamp_ms: 0,
                        provider: "faux".into(),
                        model_id: "faux-1".into(),
                        content: vec![epsilon_contracts::content::ContentBlock::text(
                            "the cut answer",
                        )],
                        stop_reason: epsilon_contracts::message::StopReason::MaxTokens,
                        error_message: None,
                        usage: Default::default(),
                    },
                ),
                api_request_sha256: None,
            })
            .expect("the message lands");
        drop(store);
        let (session, notices) = faux_session(&ws);
        let mut h = Harness::with_session(ws, session, notices);

        let outcome = h.app.try_command(&format!("/resume {}", other.display()));
        let crate::commands::Outcome::Ran(_) = outcome else {
            panic!("/resume <path> must run, not refuse or send: {outcome:?}");
        };
        let frame = h.paint();
        let agent = frame.region("agent").join("\n");
        assert!(
            agent.contains("Response was truncated"),
            "the notice must reach the screen, not just the rows.\n{agent}"
        );
    }

    /// A resumed session with a stopped reply shows the notice (C-110, pixels).
    ///
    /// Same arrangement as the truncation screen test: the reducer tests pin
    /// the row, this pins the row reaching the agent region through resume.
    #[test]
    fn a_resumed_stopped_reply_reaches_the_screen() {
        let (ws, _keep) = scratch_workspace("stopped-screen");
        let sessions_root = epsilon_runtime::sessions_root(&ws);
        std::fs::create_dir_all(&sessions_root).expect("sessions root");
        let other = sessions_root.join("stopped.jsonl");
        let mut store =
            epsilon_session::SessionStore::open_in(other.clone(), &ws).expect("a store opens");
        store
            .append(epsilon_contracts::session_entry::EntryKind::Message {
                message: epsilon_contracts::message::Message::Assistant(
                    epsilon_contracts::message::AssistantMessage {
                        id: "m-stop".into(),
                        timestamp_ms: 0,
                        provider: "faux".into(),
                        model_id: "faux-1".into(),
                        content: vec![epsilon_contracts::content::ContentBlock::text(
                            "the stopped answer",
                        )],
                        stop_reason: epsilon_contracts::message::StopReason::Aborted,
                        error_message: None,
                        usage: Default::default(),
                    },
                ),
                api_request_sha256: None,
            })
            .expect("the message lands");
        drop(store);
        let (session, notices) = faux_session(&ws);
        let mut h = Harness::with_session(ws, session, notices);

        let outcome = h.app.try_command(&format!("/resume {}", other.display()));
        let crate::commands::Outcome::Ran(_) = outcome else {
            panic!("/resume <path> must run, not refuse or send: {outcome:?}");
        };
        let frame = h.paint();
        let agent = frame.region("agent").join("\n");
        assert!(
            agent.contains("Response was stopped"),
            "the notice must reach the screen, not just the rows.\n{agent}"
        );
    }

    /// Bare `/resume` opens the sessions browser — the path form's partner.
    ///
    /// The terminal's bare form opens its session browser; here the browser is
    /// the sessions sidebar, so bare `/resume` is `/sessions` by another
    /// spelling. It needs no agent: the pane paints its own attached-or-not
    /// state.
    #[test]
    fn slash_resume_bare_opens_the_sessions_browser() {
        let (ws, _keep) = scratch_workspace("slash-resume-bare");
        let (session, notices) = faux_session(&ws);
        let mut h = Harness::with_session(ws, session, notices);
        let outcome = h.app.try_command("/resume");
        let crate::commands::Outcome::Ran(said) = outcome else {
            panic!("bare /resume must run: {outcome:?}");
        };
        assert!(
            said.contains("sessions"),
            "the answer names where to look: {said:?}"
        );
        assert!(
            matches!(h.app.sidebar, SidebarView::Sessions),
            "bare /resume shows the sessions browser"
        );
    }

    /// `/resume` on the session already open says so instead of rebuilding it.
    ///
    /// The click path skips a resume of the current file — "a needless rebuild
    /// the reader did not ask for" — and the typed command follows it, but says
    /// so: a typed line that silently does nothing is the state this UI refuses.
    #[test]
    fn slash_resume_on_the_current_session_says_so() {
        let (ws, _keep) = scratch_workspace("slash-resume-current");
        let (session, notices) = faux_session(&ws);
        let mut h = Harness::with_session(ws, session, notices);
        let current = h
            .app
            .session
            .as_ref()
            .and_then(|s| s.session_path())
            .expect("the attached session holds a file");

        let outcome = h.app.try_command(&format!("/resume {}", current.display()));
        let crate::commands::Outcome::Ran(said) = outcome else {
            panic!("/resume <current> must answer, not refuse or send: {outcome:?}");
        };
        assert!(
            said.contains("already in"),
            "the answer names the no-op: {said:?}"
        );
        let still = h
            .app
            .session
            .as_ref()
            .and_then(|s| s.session_path())
            .expect("still attached");
        assert_eq!(still, current, "and nothing moved");
    }

    /// `/resume` at a path with no file names it rather than attempting it.
    ///
    /// The terminal's wording ("no file at ... — check the path"), because a
    /// reader who mistyped wants the path they typed, not a lease error about
    /// a file that was never there.
    #[test]
    fn slash_resume_at_a_missing_file_names_it() {
        let (ws, _keep) = scratch_workspace("slash-resume-missing");
        let (session, notices) = faux_session(&ws);
        let mut h = Harness::with_session(ws.clone(), session, notices);
        let before = h
            .app
            .session
            .as_ref()
            .and_then(|s| s.session_path())
            .expect("the attached session holds a file");

        let outcome = h.app.try_command("/resume nope.jsonl");
        let crate::commands::Outcome::Ran(said) = outcome else {
            panic!("/resume <missing> must answer, not refuse or send: {outcome:?}");
        };
        assert!(
            said.contains("no file at") && said.contains("nope.jsonl"),
            "the answer names the path that is not there: {said:?}"
        );
        let still = h
            .app
            .session
            .as_ref()
            .and_then(|s| s.session_path())
            .expect("still attached");
        assert_eq!(still, before, "and nothing moved");
    }

    /// `/resume <path>` with no agent names what's missing.
    #[test]
    fn slash_resume_with_a_path_and_no_agent_names_what_is_missing() {
        let (ws, _keep) = scratch_workspace("slash-resume-no-agent");
        let mut h = Harness::new(ws);
        let outcome = h.app.try_command("/resume some.jsonl");
        let crate::commands::Outcome::Ran(said) = outcome else {
            panic!("/resume <path> without an agent must answer: {outcome:?}");
        };
        assert!(
            said.contains("no agent"),
            "names the missing piece: {said:?}"
        );
    }

    /// `/fork` branches the current session file — and stays where it is.
    ///
    /// Same act as the terminal's: the store's `fork_branch_at`, not a file
    /// copy (a copy duplicates abandoned branches under one identity). The
    /// fork is stored and named; switching to it is `/resume`'s job, exactly
    /// as the terminal says "open with --session" rather than moving.
    #[test]
    fn slash_fork_branches_the_current_file_and_stays_put() {
        let (ws, _keep) = scratch_workspace("slash-fork");
        let (session, notices) = faux_session(&ws);
        let mut h = Harness::with_session(ws.clone(), session, notices);
        // Give the attached session a leaf to branch from.
        {
            let store = h
                .app
                .session
                .as_ref()
                .and_then(|s| s.agent().session_store())
                .expect("attached store");
            let mut guard = store.lock().expect("store lock");
            guard
                .append(epsilon_contracts::session_entry::EntryKind::Message {
                    message: epsilon_contracts::message::Message::Assistant(
                        epsilon_contracts::message::AssistantMessage {
                            id: "m-fork".into(),
                            timestamp_ms: 0,
                            provider: "faux".into(),
                            model_id: "faux-1".into(),
                            content: vec![epsilon_contracts::content::ContentBlock::text(
                                "the line being forked",
                            )],
                            stop_reason: epsilon_contracts::message::StopReason::EndTurn,
                            error_message: None,
                            usage: Default::default(),
                        },
                    ),
                    api_request_sha256: None,
                })
                .expect("the message lands");
        }
        let before = h
            .app
            .session
            .as_ref()
            .and_then(|s| s.session_path())
            .expect("the attached session holds a file");
        let listed_before = h
            .app
            .session
            .as_ref()
            .map(|s| s.sessions(&ws).len())
            .unwrap_or(0);

        let outcome = h.app.try_command("/fork");
        let crate::commands::Outcome::Ran(said) = outcome else {
            panic!("/fork must run, not refuse or send: {outcome:?}");
        };
        assert!(
            said.contains("forked session"),
            "the answer names what happened: {said:?}"
        );
        let still = h
            .app
            .session
            .as_ref()
            .and_then(|s| s.session_path())
            .expect("still attached");
        assert_eq!(still, before, "forking stores; it does not move");
        let listed_after = h
            .app
            .session
            .as_ref()
            .map(|s| s.sessions(&ws).len())
            .unwrap_or(0);
        assert_eq!(
            listed_after,
            listed_before + 1,
            "one branch file added beside the others"
        );
    }

    /// `/clone` is the same branch under its own name.
    ///
    /// The terminal's `/fork` and `/clone` share `fork_branch_at` and differ
    /// only in the verb they report; the window does the same through one
    /// helper, so this test pins the shared path once more under the second
    /// spelling rather than re-proving the mechanics.
    #[test]
    fn slash_clone_branches_under_its_own_name() {
        let (ws, _keep) = scratch_workspace("slash-clone");
        let (session, notices) = faux_session(&ws);
        let mut h = Harness::with_session(ws, session, notices);
        {
            let store = h
                .app
                .session
                .as_ref()
                .and_then(|s| s.agent().session_store())
                .expect("attached store");
            let mut guard = store.lock().expect("store lock");
            guard
                .append(epsilon_contracts::session_entry::EntryKind::Message {
                    message: epsilon_contracts::message::Message::Assistant(
                        epsilon_contracts::message::AssistantMessage {
                            id: "m-clone".into(),
                            timestamp_ms: 0,
                            provider: "faux".into(),
                            model_id: "faux-1".into(),
                            content: vec![epsilon_contracts::content::ContentBlock::text(
                                "the line being cloned",
                            )],
                            stop_reason: epsilon_contracts::message::StopReason::EndTurn,
                            error_message: None,
                            usage: Default::default(),
                        },
                    ),
                    api_request_sha256: None,
                })
                .expect("the message lands");
        }

        let outcome = h.app.try_command("/clone");
        let crate::commands::Outcome::Ran(said) = outcome else {
            panic!("/clone must run, not refuse or send: {outcome:?}");
        };
        assert!(
            said.contains("cloned session"),
            "the answer uses the verb that was typed: {said:?}"
        );
    }

    /// Bare `/export` writes HTML into `.epsilon/exports/` — the terminal's default.
    ///
    /// Same formats, same folder, same words: the spelling is shared and the
    /// rendering is shared, so this pins the window's half (workspace root,
    /// notices) through the real dispatch.
    #[test]
    fn slash_export_writes_html_by_default() {
        let (ws, _keep) = scratch_workspace("slash-export");
        let (session, notices) = faux_session(&ws);
        let mut h = Harness::with_session(ws.clone(), session, notices);
        {
            let store = h
                .app
                .session
                .as_ref()
                .and_then(|s| s.agent().session_store())
                .expect("attached store");
            let mut guard = store.lock().expect("store lock");
            guard
                .append(epsilon_contracts::session_entry::EntryKind::Message {
                    message: epsilon_contracts::message::Message::Assistant(
                        epsilon_contracts::message::AssistantMessage {
                            id: "m-export".into(),
                            timestamp_ms: 0,
                            provider: "faux".into(),
                            model_id: "faux-1".into(),
                            content: vec![epsilon_contracts::content::ContentBlock::text(
                                "a line worth exporting",
                            )],
                            stop_reason: epsilon_contracts::message::StopReason::EndTurn,
                            error_message: None,
                            usage: Default::default(),
                        },
                    ),
                    api_request_sha256: None,
                })
                .expect("the message lands");
        }

        let outcome = h.app.try_command("/export");
        let crate::commands::Outcome::Ran(said) = outcome else {
            panic!("/export must run, not refuse or send: {outcome:?}");
        };
        assert!(
            said.contains("exported html"),
            "the answer names format and file: {said:?}"
        );
        let exports = ws.join(".epsilon").join("exports");
        let files: Vec<_> = std::fs::read_dir(&exports)
            .expect("exports dir")
            .filter_map(|e| e.ok().map(|e| e.path()))
            .collect();
        assert_eq!(
            files.len(),
            1,
            "one default file beside no others: {files:?}"
        );
        let body = std::fs::read_to_string(&files[0]).expect("read back");
        assert!(
            body.contains("a line worth exporting"),
            "the export carries the conversation"
        );
    }

    /// `/export json` writes JSON where told, and `/export` with no agent answers.
    #[test]
    fn slash_export_json_and_no_agent() {
        let (ws, _keep) = scratch_workspace("slash-export-json");
        let (session, notices) = faux_session(&ws);
        let mut h = Harness::with_session(ws.clone(), session, notices);

        let outcome = h.app.try_command("/export json out.json");
        let crate::commands::Outcome::Ran(said) = outcome else {
            panic!("/export json must run: {outcome:?}");
        };
        assert!(
            said.contains("exported json"),
            "the answer names format and file: {said:?}"
        );
        let body = std::fs::read_to_string(ws.join("out.json")).expect("read back");
        let v: serde_json::Value = serde_json::from_str(&body).expect("valid json");
        assert!(v.is_array(), "json export must be an array");

        let (ws2, _keep2) = scratch_workspace("slash-export-no-agent");
        let mut bare = Harness::new(ws2);
        let outcome = bare.app.try_command("/export");
        let crate::commands::Outcome::Ran(said) = outcome else {
            panic!("/export without an agent must answer: {outcome:?}");
        };
        assert!(
            said.contains("no agent"),
            "names the missing piece: {said:?}"
        );
    }

    /// `/model <id>` switches through the shared `set_model_id` — the same
    /// machinery as the terminal, session record included (M303).
    ///
    /// Bare names the current model; the picker half stays honestly out of
    /// reach (there is no model picker in this window yet, and a bare
    /// `/model` that opened nothing would be the silent no-op). RED first:
    /// the honest refusal.
    #[test]
    fn slash_model_switches_and_names() {
        let (ws, _keep) = scratch_workspace("slash-model");
        let (session, notices) = faux_session(&ws);
        let mut h = Harness::with_session(ws, session, notices);

        // Bare opens the picker now (round 21 built it); the running model is
        // marked active inside, which is the informational half the old
        // "names the current model" answer carried.
        let outcome = h.app.try_command("/model");
        let crate::commands::Outcome::Ran(said) = outcome else {
            panic!("bare /model must answer, not refuse or send: {outcome:?}");
        };
        assert!(
            said.contains("pick a provider"),
            "bare /model opens the picker: {said:?}"
        );
        assert!(
            h.app.model_picker.is_some(),
            "and the picker is open afterwards"
        );
        h.app.close_model_picker();

        let outcome = h.app.try_command("/model zzz-test-model");
        let crate::commands::Outcome::Ran(said) = outcome else {
            panic!("/model <id> must run, not refuse or send: {outcome:?}");
        };
        assert!(
            said.contains("zzz-test-model"),
            "the answer names what it switched to: {said:?}"
        );
        let after = h
            .app
            .session
            .as_ref()
            .map(|s| s.agent().model_id())
            .unwrap_or_default();
        assert_eq!(
            after, "zzz-test-model",
            "the agent runs the named model afterwards"
        );

        let outcome = h.app.try_command("/model zzz-test-model");
        let crate::commands::Outcome::Ran(said) = outcome else {
            panic!("repeating the model must answer: {outcome:?}");
        };
        assert!(
            said.contains("already"),
            "switching to the running model says so: {said:?}"
        );
    }

    /// `/model` with no agent answers rather than refusing.
    ///
    /// Bare still opens the picker (browsing needs no agent — the catalog is
    /// static); only applying needs one, and that refusal names it.
    #[test]
    fn slash_model_without_an_agent_names_what_is_missing() {
        let (ws, _keep) = scratch_workspace("slash-model-no-agent");
        let mut h = Harness::new(ws);
        let outcome = h.app.try_command("/model");
        let crate::commands::Outcome::Ran(said) = outcome else {
            panic!("bare /model without an agent must answer: {outcome:?}");
        };
        assert!(
            said.contains("pick a provider"),
            "bare /model opens the picker: {said:?}"
        );
        assert!(
            h.app.model_picker.is_some(),
            "and the picker is open afterwards"
        );
        h.app.close_model_picker();

        let outcome = h.app.try_command("/model zzz-test-model");
        let crate::commands::Outcome::Ran(said) = outcome else {
            panic!("/model <id> without an agent must answer: {outcome:?}");
        };
        assert!(
            said.contains("no agent"),
            "names the missing piece: {said:?}"
        );
    }

    /// The model picker lists providers (stage one).
    ///
    /// Over the shared builders, so this pins the window drawing what the
    /// terminal draws from — not the catalog data itself, which is the
    /// builders' own tests' job.
    #[test]
    fn model_picker_lists_providers() {
        let (ws, _keep) = scratch_workspace("model-providers");
        let mut h = Harness::new(ws);
        let _ = h.app.try_command("/model");
        let frame = h.paint();
        let rows = frame.region("models").join("\n");
        assert!(
            rows.contains("anthropic"),
            "the provider list must name a bundled provider.\n{rows}"
        );
    }

    /// Typing in the picker filters; gibberish matches nothing.
    #[test]
    fn typing_in_the_model_picker_filters() {
        let (ws, _keep) = scratch_workspace("model-filter");
        let mut h = Harness::new(ws);
        let _ = h.app.try_command("/model");
        // One frame with the picker open before typing: the search box takes
        // focus when it is drawn, and text typed before that lands nowhere.
        let _ = h.paint();
        h.type_text("anthropic");
        let filtered = h.paint();
        let rows = filtered.region("models").join("\n");
        assert!(
            rows.contains("anthropic"),
            "the typed provider must remain: {rows:?}"
        );
        assert!(
            !rows.contains("openai"),
            "a provider that does not match must be gone, not merely further down: {rows:?}"
        );

        h.type_text("zzz-no-such-provider");
        let empty = h.paint();
        let rows = empty.region("models").join("\n");
        assert!(
            rows.contains("nothing matches"),
            "gibberish must say so: {rows:?}"
        );
    }

    /// Enter on a provider drills into its models (stage two).
    #[test]
    fn choosing_a_provider_drills_into_its_models() {
        let (ws, _keep) = scratch_workspace("model-drill");
        let mut h = Harness::new(ws);
        let _ = h.app.try_command("/model");
        // One frame with the picker open before typing: the search box takes
        // focus when it is drawn, and text typed before that lands nowhere.
        let _ = h.paint();
        h.type_text("anthropic");
        let _ = h.paint();
        let _ = h.press(egui::Key::Enter);
        assert_eq!(
            h.app.model_picker.as_ref().and_then(|p| p.provider.clone()),
            Some("anthropic".to_string()),
            "Enter on a provider row must drill, not apply"
        );
        let frame = h.paint();
        let rows = frame.region("models").join("\n");
        assert!(
            !rows.contains("nothing matches"),
            "the drilled provider must offer models: {rows:?}"
        );
    }

    /// Enter on a model applies it through the shared switch.
    #[test]
    fn choosing_a_model_applies_it() {
        let (ws, _keep) = scratch_workspace("model-apply");
        let (session, notices) = faux_session(&ws);
        let mut h = Harness::with_session(ws, session, notices);
        let before = h
            .app
            .session
            .as_ref()
            .map(|s| s.agent().model_id())
            .unwrap_or_default();
        let _ = h.app.try_command("/model");
        // One frame with the picker open before typing: the search box takes
        // focus when it is drawn, and text typed before that lands nowhere.
        let _ = h.paint();
        h.type_text("anthropic");
        let _ = h.paint();
        let _ = h.press(egui::Key::Enter);
        let _ = h.paint();
        // Whatever the builders put first is what Enter must apply: the
        // window applies the row, it does not choose the model.
        let first = h.app.model_picker_entries().first().map(|e| e.id.clone());
        let _ = h.press(egui::Key::Enter);
        assert!(h.app.model_picker.is_none(), "applying closes the picker");
        let after = h
            .app
            .session
            .as_ref()
            .map(|s| s.agent().model_id())
            .unwrap_or_default();
        assert_eq!(
            Some(after.clone()),
            first,
            "the agent runs the row Enter applied (was {before})"
        );
    }

    /// Escape backs out of the model list, then closes.
    #[test]
    fn escape_backs_out_of_models_then_closes() {
        let (ws, _keep) = scratch_workspace("model-escape");
        let mut h = Harness::new(ws);
        let _ = h.app.try_command("/model");
        // One frame with the picker open before typing: the search box takes
        // focus when it is drawn, and text typed before that lands nowhere.
        let _ = h.paint();
        h.type_text("anthropic");
        let _ = h.paint();
        let _ = h.press(egui::Key::Enter);
        assert!(
            h.app
                .model_picker
                .as_ref()
                .and_then(|p| p.provider.clone())
                .is_some(),
            "precondition: drilled into a provider"
        );
        let _ = h.press(egui::Key::Escape);
        assert_eq!(
            h.app.model_picker.as_ref().and_then(|p| p.provider.clone()),
            None,
            "first Escape backs out to providers, picker still open"
        );
        assert!(h.app.model_picker.is_some(), "still open after backing out");
        let _ = h.press(egui::Key::Escape);
        assert!(h.app.model_picker.is_none(), "second Escape closes");
    }

    /// Model rows are named for screen readers.
    #[test]
    fn model_rows_are_named_for_screen_readers() {
        let (ws, _keep) = scratch_workspace("model-a11y");
        let mut h = Harness::new(ws);
        let _ = h.app.try_command("/model");
        let frame = h.paint();
        assert!(
            h.node_named("Model row").starts_with("Model row"),
            "a screen reader must reach the rows by name.\n{}",
            frame.describe()
        );
    }

    /// `Ctrl+/` opens the shortcuts reference, and again closes it (S-101).
    ///
    /// The window's chords live in tooltips and status hints; nothing lists
    /// them all. RED first: no overlay, no rows.
    #[test]
    fn control_slash_toggles_the_shortcuts_reference() {
        let (ws, _keep) = scratch_workspace("shortcuts-toggle");
        let mut h = Harness::new(ws);
        let _ = h.paint();
        let _ = h.chord(egui::Modifiers::CTRL, egui::Key::Slash);
        assert!(
            h.app.shortcuts_open,
            "Ctrl+/ must open the shortcuts reference"
        );
        let frame = h.paint();
        let rows = frame.region("shortcuts").join("\n");
        assert!(
            rows.contains("window.palette"),
            "the reference must list the window's bindings.\n{rows}"
        );
        // The chord rides the accessibility node (summary rows carry ids, as
        // the palette's do) — which is also the stronger claim: a screen
        // reader hears the effective chord.
        let chord = h.app.keys.label("window.palette").unwrap_or_default();
        assert!(
            !chord.is_empty(),
            "the palette binding must have an effective chord"
        );
        let named = h.node_named("Shortcut · window.palette");
        assert!(
            named.contains(&chord),
            "the node must carry the effective chord, not a hardcoded one.\n{named}"
        );

        let _ = h.chord(egui::Modifiers::CTRL, egui::Key::Slash);
        assert!(
            !h.app.shortcuts_open,
            "a second Ctrl+/ must close it when already in front"
        );
    }

    /// Escape closes the shortcuts reference.
    #[test]
    fn escape_closes_the_shortcuts_reference() {
        let (ws, _keep) = scratch_workspace("shortcuts-escape");
        let mut h = Harness::new(ws);
        let _ = h.paint();
        let _ = h.chord(egui::Modifiers::CTRL, egui::Key::Slash);
        assert!(h.app.shortcuts_open, "precondition: open");
        let _ = h.press(egui::Key::Escape);
        assert!(!h.app.shortcuts_open, "Escape must close the reference");
    }

    /// Shortcut rows are named for screen readers.
    #[test]
    fn shortcut_rows_are_named_for_screen_readers() {
        let (ws, _keep) = scratch_workspace("shortcuts-a11y");
        let mut h = Harness::new(ws);
        let _ = h.paint();
        let _ = h.chord(egui::Modifiers::CTRL, egui::Key::Slash);
        let frame = h.paint();
        assert!(
            h.node_named("Shortcut").starts_with("Shortcut"),
            "a screen reader must reach the rows by name.\n{}",
            frame.describe()
        );
    }

    /// `/fork` on an empty session names it rather than branching nothing.
    ///
    /// The terminal's words ("nothing to fork — session is empty"), because a
    /// reader who just opened the window has an empty session and the message
    /// must explain the no-op rather than print a path.
    #[test]
    fn slash_fork_on_an_empty_session_names_it() {
        let (ws, _keep) = scratch_workspace("slash-fork-empty");
        let (session, notices) = faux_session(&ws);
        let mut h = Harness::with_session(ws, session, notices);
        let outcome = h.app.try_command("/fork");
        let crate::commands::Outcome::Ran(said) = outcome else {
            panic!("/fork on empty must answer, not refuse or send: {outcome:?}");
        };
        assert!(
            said.contains("nothing to fork"),
            "names the empty session: {said:?}"
        );
    }

    /// `/fork` with no agent names what's missing.
    #[test]
    fn slash_fork_without_an_agent_names_what_is_missing() {
        let (ws, _keep) = scratch_workspace("slash-fork-no-agent");
        let mut h = Harness::new(ws);
        let outcome = h.app.try_command("/fork");
        let crate::commands::Outcome::Ran(said) = outcome else {
            panic!("/fork without an agent must answer: {outcome:?}");
        };
        assert!(
            said.contains("no agent"),
            "names the missing piece: {said:?}"
        );
    }

    /// The sessions pane lists the stored session — it did not, until the root fix.
    ///
    /// `IdeSession::sessions` passed the workspace root where
    /// `list_sessions_for_cwd` needs the sessions root, so it read a directory
    /// that never exists and the pane always showed "none stored for this
    /// workspace yet" — with real sessions on disk. The `/new` test pins the
    /// API count; this pins the visible symptom, through the real draw path.
    #[test]
    fn the_sessions_pane_lists_the_stored_session() {
        let (ws, _keep) = scratch_workspace("sessions-list");
        let (session, notices) = faux_session(&ws);
        let mut h = Harness::with_session(ws, session, notices);
        h.app.set_sidebar_view(SidebarView::Sessions);
        let frame = h.paint();
        let rows = frame.region("sessions").join(" ");
        assert!(
            rows.contains("1 stored"),
            "the pane must count the stored session.\n{}",
            frame.describe()
        );
        assert!(
            !rows.contains("none stored"),
            "and not show the empty state alongside it.\n{}",
            frame.describe()
        );
    }

    /// Clicking New session starts a session file — the `/new` head behind a button.
    ///
    /// The reference puts it on the brand row; this window has none, so it
    /// lives atop the sessions list. A real button, not painted text: Tab
    /// reaches it, a screen reader names it, and the click runs the same
    /// `run_new_session` the `/new` head runs — one act, two doors. RED first:
    /// no button, no rect, no node.
    #[test]
    fn clicking_new_session_starts_a_session_file() {
        let (ws, _keep) = scratch_workspace("sessions-new-button");
        let (session, notices) = faux_session(&ws);
        let mut h = Harness::with_session(ws.clone(), session, notices);
        h.app.set_sidebar_view(SidebarView::Sessions);
        let before = h.paint();
        let rect = before
            .row_rects
            .iter()
            .find(|(region, _)| region == "sessions-new")
            .map(|(_, rect)| *rect)
            .expect("the New session button records where it was drawn");
        assert!(
            h.node_named("New session").starts_with("New session"),
            "a screen reader must reach the button by its name.\n{}",
            before.describe()
        );

        let listed_before = h
            .app
            .session
            .as_ref()
            .map(|s| s.sessions(&ws).len())
            .unwrap_or(0);
        let after = h.click(rect.center());
        let listed_after = h
            .app
            .session
            .as_ref()
            .map(|s| s.sessions(&ws).len())
            .unwrap_or(0);
        assert_eq!(
            listed_after,
            listed_before + 1,
            "clicking New session stores one branch file beside the others.\n{}",
            after.describe()
        );
    }

    /// The New session button names its chord — hover text and reader name.
    ///
    /// L-114: the reference shows the effective keybinding as trailing text on
    /// hover or keyboard focus. The tooltip reaches a pointer; the renamed
    /// accesskit node reaches a screen reader (egui never writes a tooltip
    /// onto the node). Both read the chord from the binding table, so a
    /// rebound chord is announced rebound. RED first: "New session" alone.
    #[test]
    fn new_session_button_names_its_chord() {
        let (ws, _keep) = scratch_workspace("sessions-new-chord");
        let (session, notices) = faux_session(&ws);
        let mut h = Harness::with_session(ws, session, notices);
        h.app.set_sidebar_view(SidebarView::Sessions);
        let frame = h.paint();
        assert!(
            h.node_named("New session").contains("ctrl+n"),
            "the accessible name must carry the effective chord.\n{}",
            frame.describe()
        );
    }

    /// `Ctrl+N` starts a new session (S-103) — the `/new` head behind a chord.
    ///
    /// Driven through the real key path with real modifiers, because the whole
    /// defect class here is chords that parse in the table and never fire.
    /// RED first: the chord falls through to whatever holds the keyboard.
    #[test]
    fn control_n_starts_a_new_session() {
        let (ws, _keep) = scratch_workspace("chord-new");
        let (session, notices) = faux_session(&ws);
        let mut h = Harness::with_session(ws.clone(), session, notices);
        let before = h
            .app
            .session
            .as_ref()
            .and_then(|s| s.session_path())
            .expect("the attached session holds a file");
        let listed_before = h
            .app
            .session
            .as_ref()
            .map(|s| s.sessions(&ws).len())
            .unwrap_or(0);

        let frame = h.chord(egui::Modifiers::CTRL, egui::Key::N);
        let after = h
            .app
            .session
            .as_ref()
            .and_then(|s| s.session_path())
            .expect("still attached after the chord");
        assert_ne!(
            before,
            after,
            "Ctrl+N must move to a new file.\n{}",
            frame.describe()
        );
        assert!(
            after.exists(),
            "and that file exists.\n{}",
            frame.describe()
        );
        let listed_after = h
            .app
            .session
            .as_ref()
            .map(|s| s.sessions(&ws).len())
            .unwrap_or(0);
        assert_eq!(
            listed_after,
            listed_before + 1,
            "one session file added, the previous one kept.\n{}",
            frame.describe()
        );
    }

    /// `Ctrl+Alt+F` forks the session (S-107).
    ///
    /// Same shape: the `/fork` head behind the reference's chord, driven
    /// through the real key path. The session needs a leaf to branch from.
    #[test]
    fn control_alt_f_forks_the_session() {
        let (ws, _keep) = scratch_workspace("chord-fork");
        let (session, notices) = faux_session(&ws);
        let mut h = Harness::with_session(ws.clone(), session, notices);
        {
            let store = h
                .app
                .session
                .as_ref()
                .and_then(|s| s.agent().session_store())
                .expect("attached store");
            let mut guard = store.lock().expect("store lock");
            guard
                .append(epsilon_contracts::session_entry::EntryKind::Message {
                    message: epsilon_contracts::message::Message::Assistant(
                        epsilon_contracts::message::AssistantMessage {
                            id: "m-chord".into(),
                            timestamp_ms: 0,
                            provider: "faux".into(),
                            model_id: "faux-1".into(),
                            content: vec![epsilon_contracts::content::ContentBlock::text(
                                "the line being forked",
                            )],
                            stop_reason: epsilon_contracts::message::StopReason::EndTurn,
                            error_message: None,
                            usage: Default::default(),
                        },
                    ),
                    api_request_sha256: None,
                })
                .expect("the message lands");
        }
        let listed_before = h
            .app
            .session
            .as_ref()
            .map(|s| s.sessions(&ws).len())
            .unwrap_or(0);

        let frame = h.chord(egui::Modifiers::CTRL | egui::Modifiers::ALT, egui::Key::F);
        let listed_after = h
            .app
            .session
            .as_ref()
            .map(|s| s.sessions(&ws).len())
            .unwrap_or(0);
        assert_eq!(
            listed_after,
            listed_before + 1,
            "Ctrl+Alt+F stores one branch file beside the others.\n{}",
            frame.describe()
        );
    }

    /// Typing filters it, and `Enter` opens what is highlighted.
    ///
    /// The whole point is reaching a file **by name**, so the test types a name rather than
    /// clicking a row: a palette that opens correctly and cannot be searched is a menu.
    #[test]
    fn typing_in_the_palette_filters_and_enter_opens_the_file() {
        let (dir, _keep) = scratch_workspace("palette-filter");
        let mut h = Harness::new(dir);
        let _ = h.paint();
        let _ = h.chord(egui::Modifiers::CTRL, egui::Key::P);
        h.type_text("beta");

        let filtered = h.paint();
        let rows = filtered.region("palette");
        assert!(
            palette_has(&filtered, "beta.rs"),
            "the typed name must be found: {rows:?}"
        );
        assert!(
            !palette_has(&filtered, "alpha.rs"),
            "and a row that does not match must be gone, not merely further down: {rows:?}"
        );

        let opened = h.press(egui::Key::Enter);
        assert!(
            h.app.open.is_some_and(|f| f.path == "beta.rs"),
            "Enter must open the highlighted row.\n{}",
            opened.describe()
        );
        assert!(
            !palette_has(&opened, "beta.rs"),
            "and the palette must close once it has done its job.\n{}",
            opened.describe()
        );
    }

    /// `Escape` closes it, and does **not** clear the draft.
    ///
    /// This is the collision that decides the order things happen in. The composer takes
    /// `Escape` unconditionally — it is how a run is aborted — so a palette that read it later
    /// would be reading an event already taken. Reading it first is right; what has to be
    /// checked is that doing so did not steal the composer's meaning.
    #[test]
    fn escape_closes_the_palette_and_leaves_the_draft_alone() {
        let (dir, _keep) = scratch_workspace("palette-esc");
        let mut h = Harness::new(dir);
        let _ = h.paint();
        h.app.set_draft("half a thought");
        let _ = h.chord(egui::Modifiers::CTRL, egui::Key::P);

        let closed = h.press(egui::Key::Escape);
        assert!(h.app.palette.is_none(), "Escape must close the palette");
        assert!(
            !closed.drew("palette"),
            "and it must be gone from the frame.\n{}",
            closed.describe()
        );
        assert_eq!(
            h.app.draft, "half a thought",
            "Escape closed the palette, not the draft"
        );
    }

    /// A palette row can run a **command**, not only open a file.
    ///
    /// A palette that only opens files is a file finder wearing a command palette's name. The
    /// commands are the window's own: there is no command table in this crate to draw on yet,
    /// and three entries that demonstrably work beat thirty that do not.
    #[test]
    fn a_palette_row_can_switch_the_sidebar() {
        let (dir, _keep) = scratch_workspace("palette-cmd");
        let mut h = Harness::new(dir);
        let _ = h.paint();
        let _ = h.chord(egui::Modifiers::CTRL, egui::Key::P);
        h.type_text("sessions");

        let _ = h.paint();
        let _ = h.press(egui::Key::Enter);
        assert_eq!(
            h.app.sidebar,
            crate::app::SidebarView::Sessions,
            "the palette must be able to reach the window's commands"
        );
    }

    /// A frame becomes **real geometry**, not just text in a summary.
    ///
    /// Everything else in this file asserts on `FrameSummary`, which is a test instrument: it
    /// records what the draw path *said*. This is the one test that goes one step further and
    /// tessellates the frame's shapes the way the renderer does, so it fails if the draw path
    /// produced nothing drawable at all.
    ///
    /// It exists because of a measurement, not a worry. Run on this machine for real, the
    /// window **opened and painted and showed nothing**: a full-window red rectangle never
    /// reached the screen, because this laptop's GPU device is lost and no wgpu backend can
    /// present. So on this machine the tessellation step is the **last** point where a defect
    /// is visible to a test, and everything after it — the GPU — is not ours to check.
    ///
    /// The number is a floor, not a target: an empty window tessellates to almost nothing, and
    /// this window's chrome, tree, editor and transcript do not.
    #[test]
    fn a_frame_tessellates_into_geometry_a_renderer_could_draw() {
        let (dir, _keep) = scratch_workspace("tessellate");
        let mut h = Harness::new(dir);
        let full = h.full_frame();
        assert!(
            !full.shapes.is_empty(),
            "the draw path produced no shapes at all"
        );

        let meshes = h.ctx.tessellate(full.shapes.clone(), 1.0);
        let vertices: usize = meshes
            .iter()
            .map(|p| match &p.primitive {
                egui::epaint::Primitive::Mesh(m) => m.vertices.len(),
                // Text is uploaded as a texture callback rather than tessellated, which
                // is exactly why vertex count alone could not be the only measure.
                egui::epaint::Primitive::Callback(_) => 0,
            })
            .sum();
        assert!(
            vertices > 1000,
            "a full window must tessellate to real geometry; it produced {vertices} vertices \
             across {} shape(s)",
            full.shapes.len()
        );
    }

    /// **The window's pixels, rasterised offscreen, are not blank.**
    ///
    /// `#[ignore]`d on purpose, and for the reason `frame_dump.rs` is: this needs a
    /// **wgpu adapter**, which a CI runner does not have, and a test that fails because
    /// the machine has no GPU is a test that gets deleted rather than read. It is run by
    /// hand with `--ignored`, which is what produced
    /// `audit/evidence/desktop-frames-2026-10-04/`.
    ///
    /// **Why this test exists at all, and it is not another frame assertion.** Everything
    /// else in this module checks what the window *recorded* — `FrameSummary`'s rows, the
    /// AccessKit tree, the tessellated vertex count. All three stop one step short of an
    /// image. And the window itself cannot supply one on this machine: **no wgpu backend
    /// presents**, measured and re-measured, with the content rect filled solid red and
    /// still blank (`audit/evidence/desktop-window-measurement-2026-10-03.md`). So for the
    /// whole life of this crate **no one had looked at what it draws**, and
    /// `AGENTS.md` §13 asks for exactly that.
    ///
    /// This renders the frame's own tessellation through the same `egui_wgpu::Renderer`
    /// `eframe` uses, into an offscreen texture. It is the swapchain that was failing, not
    /// a GPU.
    ///
    /// **The assertion is the one that matters: the image is not blank.** A rasterizer that
    /// runs, picks an adapter, and returns a field of clear colour passes every other
    /// check in this crate — which is precisely the shape of the failure that has been
    /// happening. So it counts pixels that differ from the clear colour and requires a real
    /// number of them.
    #[test]
    #[ignore = "needs a wgpu adapter; a CI runner has none. Run with --ignored."]
    fn the_window_rasterises_to_an_image_that_is_not_blank() {
        let (dir, _keep) = scratch_workspace("raster");
        let mut h = Harness::new(dir.clone());
        // A window with something in it. A blank IDE and a blank *renderer* produce the
        // same image, so the fixture has to be one where "is anything there" has an answer.
        let mut t = Transcript::new(dir);
        t.push_user("fix the failing test in epsilon-core");
        t.apply(&epsilon_contracts::agent_event::AgentEvent::MessageEnd {
            message: Box::new(epsilon_contracts::message::Message::Assistant(
                epsilon_contracts::message::AssistantMessage {
                    id: "m1".into(),
                    timestamp_ms: 0,
                    provider: "faux".into(),
                    model_id: "faux-1".into(),
                    content: vec![epsilon_contracts::content::ContentBlock::text(
                        "The guard compared against the **pre**-image, so it could never pass \
                         after a real write. Recording the post-image fixes it at the cause \
                         rather than by weakening the check.",
                    )],
                    stop_reason: epsilon_contracts::message::StopReason::EndTurn,
                    error_message: None,
                    usage: Default::default(),
                },
            )),
        });
        t.push_change(&crate::diff::Change {
            path: "src/lib.rs".into(),
            lines: vec![
                crate::diff::DiffLine::Removed("assert!(matches!(guard))".into()),
                crate::diff::DiffLine::Added(
                    "assert!(matches!(guard), \"the guard must hold\")".into(),
                ),
            ],
            created: false,
            coarse: false,
        });
        h.app.seed_transcript(t);

        // **Not `Harness::full_frame`**: that clears `textures_delta` on purpose, and the
        // font atlas lives in it. Upload it and get geometry pointing at glyphs that were
        // never sent, and the frame renders with the text missing — which for a probe
        // whose entire job is to show the text is the worst available outcome.
        let mut input = h.input.clone();
        input.events = Vec::new();
        let clear = h.app.theme.background;
        let full = h.ctx.run_ui(input, |ui| {
            h.app.paint(ui);
        });

        let size = full.pixels_per_point
            * full.shapes[..]
                .iter()
                .fold(egui::Rect::ZERO, |acc, s| acc.union(s.clip_rect))
                .size();
        let (w, h_px) = (size.x.round() as u32, size.y.round() as u32);
        assert!(
            w > 0 && h_px > 0,
            "the frame reported a {w}x{h_px} size, so there is nothing to rasterise"
        );

        let rgba = crate::frame_raster::rasterize(&h.ctx, full, clear)
            .expect("an offscreen render needs a wgpu adapter; without one this machine cannot produce a frame at all");
        assert_eq!(
            rgba.len(),
            (w as usize) * (h_px as usize) * 4,
            "read back {} bytes for a {w}x{h_px} frame",
            rgba.len()
        );

        // **The assertion that is the whole point.** Pixels that differ from the clear
        // colour by more than a rounding step. A blank render scores zero here; a blank
        // *window* and a broken rasterizer both score zero, which is the honest
        // ambiguity — but it is an ambiguity between two things a person can now look at,
        // rather than between two things nobody could see.
        let bg = clear.to_array();
        let inked = rgba
            .chunks_exact(4)
            .filter(|p| (0..3).any(|i| (p[i] as i32 - bg[i] as i32).abs() > 4))
            .count();
        let total = w as usize * h_px as usize;
        let share = inked as f64 / total as f64;
        eprintln!(
            "frame_raster: {w}x{h_px}, {inked}/{total} pixels inked ({:.1}%)",
            share * 100.0
        );
        assert!(
            share > 0.02,
            "only {inked}/{total} pixels ({:.2}%) differ from the clear colour. That is a \
             blank image: the rasterizer ran and drew nothing, which is the failure this test \
             exists to catch and which no other check in this crate could see.",
            share * 100.0
        );

        // **And keep the picture**, so a person can look at it. `EPSILON_FRAME_DIR` is the
        // same variable `epsilon-tui`'s `frame_dump` reads, so one directory can hold both
        // front ends' output.
        if let Some(dir) = std::env::var_os("EPSILON_FRAME_DIR") {
            let path = std::path::Path::new(&dir).join("desktop-raster.png");
            crate::frame_raster::write_png(&path, w, h_px, &rgba).expect("writing the frame");
            eprintln!("frame_raster: wrote {}", path.display());
        }
    }

    /// **The transcript starts below its own header (M312).**
    ///
    /// Found by **rendering the window and looking at it** — the frame dump this milestone
    /// added shows the pane's `Agent` label sitting in the middle of the conversation, with the
    /// first rows clipped by the pane's top edge. The cause: `ScrollArea::show(ui, ..)` starts
    /// from the passed `Ui`'s cursor, which in that pane is the pane's **top**, while the
    /// header, notices and goal band are painted at absolute positions and never move it. The
    /// transcript was drawn **42 pixels too high**.
    ///
    /// **Why no test caught it.** Every other assertion in this file reads
    /// `FrameSummary::rows`, which records *which* strings were drawn. A row drawn in the right
    /// place and a row drawn 42 pixels too high put **identical strings** in that list. Position
    /// was not recorded anywhere, so the only instrument that could see it was a person looking
    /// at a picture — which is why `transcript:first-row` now records the first drawn row's
    /// rectangle, once per frame, and this asserts on it.
    ///
    /// Asserted as geometry rather than as a magic number: the first row must start **below the
    /// pane's header band** and must not be clipped by the pane's own top edge. Both halves
    /// matter — a transcript drawn under the header but outside the clip is a different bug.
    #[test]
    fn the_transcript_starts_below_its_own_header_and_is_not_clipped() {
        let (ws, _keep) = scratch_workspace("transcript-origin");
        let mut h = Harness::new(ws.clone());
        let mut t = Transcript::new(ws);
        t.push_user("the first thing said");
        t.apply(&epsilon_contracts::agent_event::AgentEvent::MessageEnd {
            message: Box::new(epsilon_contracts::message::Message::Assistant(
                epsilon_contracts::message::AssistantMessage {
                    id: "m1".into(),
                    timestamp_ms: 0,
                    provider: "faux".into(),
                    model_id: "faux-1".into(),
                    content: vec![epsilon_contracts::content::ContentBlock::text("an answer")],
                    stop_reason: epsilon_contracts::message::StopReason::EndTurn,
                    error_message: None,
                    usage: Default::default(),
                },
            )),
        });
        h.app.seed_transcript(t);

        let f = h.paint();
        let first_row = f
            .hotspots
            .iter()
            .find(|(id, _)| id == "transcript:first-row")
            .map(|(_, r)| *r)
            .expect("the first drawn transcript row must be recorded");

        // **Above the header line it belongs under.** The pane starts at the same y in every
        // frame, so the relationship is what carries the meaning, not a coordinate.
        let header_bottom = 42.0 + 34.0;
        assert!(
            first_row.top() >= header_bottom,
            "the transcript is drawn over its own header: the first row starts at y={:.1} and the \
             header band ends at {header_bottom:.1}. This is the defect the rendered frame showed \
             — the pane's `Agent` label appeared in the middle of the conversation.\n{}",
            first_row.top(),
            f.describe()
        );
        assert!(
            first_row.top() >= 42.0,
            "and it must not start above the pane's own edge, where it would be clipped: \
             y={:.1}",
            first_row.top()
        );
    }

    /// **A pane's content starts under its own header (M312).**
    ///
    /// The same defect as the transcript's, in the two sidebar panes, and found the same way:
    /// **the rendered frame**. The sessions pane printed `none stored for this workspace yet`
    /// *above* the word `Sessions`, both clipped by the pane's top edge; the subagents pane had
    /// the same shape waiting.
    ///
    /// **Why nothing caught it.** Both panes' headers are painted with the painter at an
    /// absolute position, which does not advance the `Ui` cursor — so the first label the pane
    /// adds lands at the pane's **top**, on top of the header. Every assertion in the file reads
    /// `FrameSummary::rows`, which records the strings: these strings *were* drawn, just in the
    /// wrong place.
    ///
    /// **So this asserts the position** — from `pane:first-content`, recorded where the pane's
    /// content is about to be laid out — against the header band, rather than checking that some
    /// text exists. A first version of this test checked the strings, and would therefore have
    /// passed against the defect it was written for.
    #[test]
    fn a_sidebar_pane_draws_its_content_under_its_own_header() {
        for view in [SidebarView::Sessions, SidebarView::Tasks] {
            let (ws, _keep) = scratch_workspace("pane-header");
            let (session, notices) = faux_session(&ws);
            let mut h = Harness::with_session(ws, session, notices);
            h.app.set_sidebar_view(view);
            let f = h.paint();

            let content = f
                .hotspots
                .iter()
                .find(|(id, _)| id == "pane:first-content")
                .map(|(_, r)| *r)
                .expect("the pane must record where its content lands");
            assert!(
                content.height() >= PANE_HEADER_H,
                "the {view:?} pane starts its content {:.1} points below its own top, inside the \
                 {PANE_HEADER_H:.1}-point header band — the pane's title and its first line \
                 overlap, which is what the rendered frame showed. A pane paints its header with \
                 the painter, which does not move the cursor, so it has to move it itself.\n{}",
                content.height(),
                f.describe()
            );
        }
    }

    /// **An open file draws its lines on separate lines (M312).**
    ///
    /// Found by **rendering the editor pane and looking at it**: a five-line file appeared as
    /// **one squashed band of overlapping glyphs**. `ScrollArea::show_rows` is virtualised — it
    /// decides which lines to call the closure for — and this loop painted every row with the
    /// painter while allocating **nothing**, so the cursor never advanced and every visible line
    /// landed on the same `y`.
    ///
    /// **Asserted on the rects, not the text**, and that is the whole point. `FrameSummary::rows`
    /// recorded `    1 fn alpha() {` and every other line correctly, because every one of them
    /// *was* drawn. The defect was entirely in *where*, and `row_rects` — which the loop already
    /// filled in, with five identical rects — is the one record of *where* this crate keeps.
    #[test]
    fn each_line_of_an_open_file_gets_its_own_line() {
        let (dir, _keep) = scratch_workspace("editor-rows");
        std::fs::write(
            dir.join("alpha.rs"),
            "fn alpha() {\n    let one = 1;\n    let two = 2;\n    let three = 3;\n    \
             let four = 4;\n}\n",
        )
        .expect("a file with six lines");
        let mut h = Harness::new(dir);
        h.app.open_file("alpha.rs");
        let f = h.paint();

        let rects: Vec<egui::Rect> = f
            .row_rects
            .iter()
            .filter(|(region, _)| region == "editor")
            .map(|(_, r)| *r)
            .collect();
        assert!(
            rects.len() >= 5,
            "the editor drew {} row(s) for a six-line file:\n{}",
            rects.len(),
            f.describe()
        );
        for (i, pair) in rects.windows(2).enumerate() {
            assert!(
                pair[1].top() > pair[0].top(),
                "line {} and line {} were drawn at the same height (y={:.1} and y={:.1}) — the \
                 file is one band of overlapping text rather than lines.\n{}",
                i + 1,
                i + 2,
                pair[0].top(),
                pair[1].top(),
                f.describe()
            );
        }
    }

    /// **A change under review draws its lines on separate lines (M312).**
    ///
    /// The editor's defect, in the pane beside it, found by rendering both and looking. A
    /// four-line diff drew its two context lines as one band of overlapping text while the added
    /// and removed lines — which are drawn with their own offsets — stayed apart, which is why the
    /// frame looked *almost* right.
    ///
    /// Asserted on `row_rects` for the same reason as the editor's: the strings were all correct.
    #[test]
    fn a_change_under_review_draws_its_lines_on_separate_lines() {
        let (dir, _keep) = scratch_workspace("review-rows");
        let mut h = Harness::new(dir);
        let mut t = Transcript::new(std::env::temp_dir());
        t.push_user("tighten the guard");
        t.push_change(&crate::diff::Change {
            path: "src/lib.rs".into(),
            lines: vec![
                crate::diff::DiffLine::Context("pub fn check(state: &State) -> bool {".into()),
                crate::diff::DiffLine::Context("    let guard = state.guard();".into()),
                crate::diff::DiffLine::Removed("    assert!(matches!(guard))".into()),
                crate::diff::DiffLine::Added(
                    "    assert!(matches!(guard), \"the guard must hold\")".into(),
                ),
                crate::diff::DiffLine::Context("}".into()),
            ],
            created: false,
            coarse: false,
        });
        h.app.seed_transcript(t);
        let change = h
            .app
            .transcript
            .diffs()
            .first()
            .cloned()
            .expect("the seeded change");
        h.app.review = Some(change);

        let f = h.paint();
        let rects: Vec<egui::Rect> = f
            .row_rects
            .iter()
            .filter(|(region, _)| region == "editor")
            .map(|(_, r)| *r)
            .collect();
        assert!(
            rects.len() >= 4,
            "the review drew {} row(s) for a five-line diff:\n{}",
            rects.len(),
            f.describe()
        );
        for (i, pair) in rects.windows(2).enumerate() {
            assert!(
                pair[1].top() > pair[0].top(),
                "diff line {} and {} were drawn at the same height (y={:.1} and y={:.1}) — the \
                 review is a band of overlapping text.\n{}",
                i + 1,
                i + 2,
                pair[0].top(),
                pair[1].top(),
                f.describe()
            );
        }
    }

    /// **A tool row says which tool it was, and marks it with a glyph this window can draw (M312).**
    ///
    /// Found by rendering a conversation with tool calls in it — the transcript's commonest
    /// content, and the only state nobody had drawn. Two defects in one row: the marker was an
    /// **empty box** (`U+2713`/`U+2717` are Dingbats, which egui's bundled font has not got, the
    /// same two missing blocks that turned the token arrows into boxes), and the row showed the
    /// tool's **arguments without its name** — `{"command": "cargo test"}` from a reader who was
    /// never told `bash` ran. The name was in the row's *accessible* name, so a screen-reader user
    /// knew and a sighted one did not, which is the wrong way round for a pane.
    ///
    /// Asserted on the **drawn text**, which is where both were wrong. The glyph *coverage* cannot
    /// be asserted from code at all — that is what looking at the frame is for, and
    /// `10-tool-calls.png` is the evidence.
    #[test]
    fn a_tool_row_names_its_tool_and_marks_it_with_a_drawable_glyph() {
        use epsilon_contracts::agent_event::AgentEvent;
        let (ws, _keep) = scratch_workspace("tool-row");
        let mut h = Harness::new(ws.clone());
        let mut t = Transcript::new(ws);
        t.apply(&AgentEvent::ToolExecutionStart {
            tool_call_id: "c1".into(),
            tool_name: "bash".into(),
            args: serde_json::json!({ "command": "cargo test" }),
        });
        t.apply(&AgentEvent::ToolExecutionEnd {
            tool_call_id: "c1".into(),
            tool_name: "bash".into(),
            is_error: false,
        });
        h.app.seed_transcript(t);

        let ok = h.paint();
        let line = ok.region("agent").join(" ");
        assert!(
            line.contains("+ bash") && line.contains("cargo test"),
            "a finished tool call must name the tool it ran **and** what it ran — a reader who \
             cannot see which tool produced a result cannot trust it.\n{}",
            ok.describe()
        );
        assert!(
            !line.contains('✓') && !line.contains('▸'),
            "**and the markers must be ones this window's font actually has.** U+2713 and U+25B8 \
             rendered as empty boxes in the frame; the terminal's own ASCII repertoire is `+` and \
             `>`.\n{line}"
        );

        // And the failed marker differs, so the two outcomes are not told apart by colour alone.
        let mut failed = Harness::new(std::env::temp_dir());
        let mut t = Transcript::new(std::env::temp_dir());
        t.apply(&AgentEvent::ToolExecutionStart {
            tool_call_id: "c2".into(),
            tool_name: "write".into(),
            args: serde_json::json!({ "path": "src/lib.rs" }),
        });
        t.apply(&AgentEvent::ToolExecutionEnd {
            tool_call_id: "c2".into(),
            tool_name: "write".into(),
            is_error: true,
        });
        failed.app.seed_transcript(t);
        let bad = failed.paint();
        let bad_line = bad.region("agent").join(" ");
        assert!(
            bad_line.contains("x write"),
            "a failed tool call is marked differently and names its tool too.\n{}",
            bad.describe()
        );
    }

    /// One painted frame with the harness's input, keeping everything the renderer would be handed.
    fn paint_once(h: &mut Harness) -> egui::FullOutput {
        let mut input = h.input.clone();
        input.events = Vec::new();
        h.ctx.run_ui(input, |ui| {
            h.app.paint(ui);
        })
    }

    /// `a` with `b`'s textures added on top — so a frame that carries no atlas can still be drawn.
    ///
    /// The first frame after a font change is the one carrying the atlas; every later frame from the
    /// same context has an empty `textures_delta`, and a fresh `egui_wgpu` renderer has nothing to
    /// draw glyphs from. This is what makes "render twice" work, and it is why every dump in this
    /// module went blank until it was written down.
    fn merge_atlas(mut a: egui::TexturesDelta, b: egui::TexturesDelta) -> egui::TexturesDelta {
        // `TexturesDelta` implements `Drop`, so neither side can be *moved* out of — they are taken
        // by value and drained through the borrow.
        for (id, image) in b.set.iter() {
            if !a.set.contains_key(id) {
                a.set.insert(*id, image.clone());
            }
        }
        a.free.extend(b.free.iter().copied());
        a
    }

    /// **The scrollbar is named when there is one, and absent when there is not (M312).**
    ///
    /// The last unnamed node in this window's tree, and the fix could have made things worse:
    /// `Context::accesskit_node_builder` is **get-or-create**, so naming a bar that was never
    /// drawn puts a **phantom** in the tree a reader browses. The probe's own doc warns about
    /// exactly this, so the second half of this test matters more than the first — a pane that
    /// fits must contribute **no** node at all.
    ///
    /// The predicate is egui's own threshold reproduced from what `ScrollAreaOutput` publishes
    /// (`scroll_area.rs:1203-1206`, `:1256-1260`), which is only equivalent while the bars are
    /// not floating (`style.rs:595` has `floating: false`, and this window never changes it).
    /// **`the_window_does_not_float_its_scroll_bars` holds that premise**, so a future change to
    /// it fails a test instead of silently producing a bar that is not there.
    ///
    /// Driven on the **file tree**, which is the pane the probe found unnamed: a workspace with
    /// more files than fit gets a bar, and a small one does not.
    #[test]
    fn a_scrollbar_is_named_only_when_there_is_one() {
        // A workspace too tall for its pane: 200 files, a pane that fits a few dozen.
        let dir = std::env::temp_dir().join("epsilon-tabs-scrollbar-overflow");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch workspace");
        for i in 0..200 {
            std::fs::write(dir.join(format!("file-{i:03}.txt")), "x").expect("a file");
        }
        let mut h = Harness::new(dir.clone());
        let nodes = h.named_nodes();
        assert!(
            nodes.iter().any(|(l, _)| l.starts_with("Scroll")),
            "a workspace that overflows must publish a **named** scrollbar. Nodes: {:?}",
            nodes.iter().map(|(l, _)| l).collect::<Vec<_>>()
        );
        assert!(
            nodes.iter().any(|(l, _)| l == "Scroll · the file tree"),
            "and the name says what it scrolls: {:?}",
            nodes.iter().map(|(l, _)| l).collect::<Vec<_>>()
        );

        // **And the control**: a workspace that fits contributes nothing. This is the half that
        // would have been broken by naming a bar unconditionally.
        let small = std::env::temp_dir().join("epsilon-tabs-scrollbar-fits");
        let _ = std::fs::remove_dir_all(&small);
        std::fs::create_dir_all(&small).expect("scratch workspace");
        std::fs::write(small.join("only.rs"), "fn only() {}").expect("one file");
        let mut h = Harness::new(small);
        let nodes = h.named_nodes();
        assert!(
            !nodes.iter().any(|(l, _)| l.starts_with("Scroll")),
            "a workspace that fits must publish **no** scrollbar node: a node named for a bar that \
             was never drawn is a phantom in the tree a reader browses. Nodes: {:?}",
            nodes.iter().map(|(l, _)| l).collect::<Vec<_>>()
        );
    }

    /// **The premise the scrollbar derivation rests on (M312).**
    ///
    /// `name_scrollbar` reproduces egui's own threshold from `ScrollAreaOutput`, and it is only
    /// equivalent to it while `limit_rect == inner_rect`. That holds while the scroll bars are
    /// **allocated no width**: `outer_rect = inner_rect + current_bar_use` (`scroll_area.rs:1195`)
    /// and `current_bar_use = allocated_width * show_bars` (`:757-761`), so zero allocated means
    /// zero difference.
    ///
    /// **This test exists because a first version asserted the wrong number.** It asked whether
    /// the bars were *floating* and refused to name anything when they were — and they float, so
    /// the function never named anything and the probe stayed red. The boolean is not the premise;
    /// `allocated_width()` is, and egui's floating style sets it to zero (`style.rs:648`).
    ///
    /// So the premise is a number, read the way the drawing code reads it, and asserted.
    #[test]
    fn the_window_allocates_no_width_to_its_scroll_bars() {
        let dir = std::env::temp_dir().join("epsilon-tabs-scrollbar-premise");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch workspace");
        std::fs::write(dir.join("a.rs"), "fn a() {}").expect("a file");
        let h = Harness::new(dir);
        // Ask a `Ui` the question the drawing code asks, rather than reading the style struct:
        // the number that matters is the one the layout will actually use.
        let mut allocated = f32::NAN;
        h.ctx
            .run_ui(egui::RawInput::default(), |ui| {
                allocated = ui.spacing().scroll.allocated_width();
            })
            .drop_without_applying_deltas();
        assert_eq!(
            allocated, 0.0,
            "the scrollbar naming reproduces egui's threshold from the published output, which is \
             only egui's threshold while the bars reserve no width. These reserve {allocated}; if \
             that changes, `name_scrollbar` must account for it rather than comparing against \
             `inner_rect` alone."
        );
    }

    /// Rasterise `h`'s next frame and write it as `name`. Returns 1 on success.
    ///
    /// **Not `Harness::full_frame`**, which clears `textures_delta` on purpose and takes the font
    /// atlas with it — the frame then renders with every glyph missing, which for a probe whose
    /// whole job is to show the text is the worst available outcome. The raster test above says
    /// so at its own call site; this repeats it because the mistake is one line away.
    fn save_frame(h: &mut Harness, dir: &std::path::Path, name: &str) -> usize {
        // **The visuals, exactly as `eframe::App::ui` sets them (M312).** `set_visuals` lives in
        // `ui()` and nowhere else — `paint` draws the panes from `theme` itself — so a harness
        // that calls `paint` directly paints **every** widget with egui's default (dark) visuals
        // whatever the theme says. The light-palette frame proved it: the panes came out light and
        // both text inputs came out as **black boxes**, which reads as a defect in the light theme
        // and is not one. A frame dump that is not the frame the product draws is worse than no
        // dump, so this mirrors the production call.
        h.ctx.set_visuals(h.app.theme.visuals());

        // **Two passes, and the atlas is carried from the first into the second.** Both facts
        // are about what a *fresh* rasteriser needs, and both have produced a blank white PNG:
        //
        // 1. `rasterize` builds a new `egui_wgpu` renderer every call, so it needs the font
        //    atlas **in the frame it is given**. Only a frame whose text changed carries it in
        //    `textures_delta`; every later frame from the same context has none. Hence "render
        //    twice" — and it is also what makes the second frame the *steady state*, which is the
        //    state a reader is in (a scroll area's stick-to-bottom needs a prior frame to be
        //    stuck from, so a long conversation dumped on its first frame shows the top).
        // 2. `Harness::paint` goes through `full_frame`, which clears `textures_delta` on
        //    purpose — the same trap, one call away in the harness's API.
        let first = paint_once(h);
        let mut second = paint_once(h);
        second.textures_delta = merge_atlas(first.textures_delta, second.textures_delta);

        let clear = h.app.theme.background;
        let size = second.pixels_per_point
            * second.shapes[..]
                .iter()
                .fold(egui::Rect::ZERO, |acc, s| acc.union(s.clip_rect))
                .size();
        let (w, h_px) = (size.x.round() as u32, size.y.round() as u32);
        if w == 0 || h_px == 0 {
            eprintln!("desktop_frames: {name} reported {w}x{h_px}, so nothing was written");
            return 0;
        }
        let Ok(rgba) = crate::frame_raster::rasterize(&h.ctx, second, clear) else {
            eprintln!("desktop_frames: {name} could not be rasterised (no adapter)");
            return 0;
        };
        let path = dir.join(name);
        if crate::frame_raster::write_png(&path, w, h_px, &rgba).is_ok() {
            eprintln!("desktop_frames: wrote {}", path.display());
            1
        } else {
            0
        }
    }

    /// **Every state worth looking at, rendered to a file so a person can look at it (M312).**
    ///
    /// The raster test above renders **one** window — a conversation with a change in it — and
    /// its image from M305 is what the desktop has been judged on ever since. Four states nobody
    /// has seen are the ones a reviewer asks about: **a permission ask**, the sessions list, a
    /// **narrow window** where the layout must give something up, and the first-run transcript.
    ///
    /// **This is the desktop's counterpart of `epsilon-tui`'s `frame_dump`**, and it reads the
    /// same `EPSILON_FRAME_DIR`, so one directory holds both front ends. It is `#[ignore]`d
    /// because it needs a wgpu adapter, exactly like the raster test.
    ///
    /// **Nothing is asserted but "the frame has pixels".** An assertion a frame can make is that
    /// it is not blank; everything past that is a person looking at it, which is the entire
    /// reason these files are written to disk. With `EPSILON_FRAME_DIR` unset it does nothing and
    /// succeeds, so it is safe to leave in the tree.
    #[test]
    #[ignore = "needs a wgpu adapter; run with --ignored and EPSILON_FRAME_DIR set"]
    fn the_windows_a_person_has_not_seen_are_rendered_for_one_to_see() {
        let Some(dir) = std::env::var_os("EPSILON_FRAME_DIR") else {
            eprintln!("desktop_frames: EPSILON_FRAME_DIR is not set, so nothing is written");
            return;
        };
        let dir = std::path::PathBuf::from(dir);
        let mut written = 0usize;

        // 1. A permission ask. **The state the product exists to get right**: the reader is told
        //    what is about to happen and shown the keys. Nobody has looked at this window drawn.
        {
            use epsilon_contracts::agent_event::{AgentEvent, PermissionRequest};
            let (ws, _keep) = scratch_workspace("frames-approval");
            let mut h = Harness::new(ws.clone());
            let mut t = Transcript::new(ws.clone());
            t.push_user("delete the build directory");
            t.apply(&AgentEvent::PermissionRequested {
                request: PermissionRequest {
                    request_id: "ask-1".into(),
                    capability: "bash".into(),
                    pattern: "rm -rf target".into(),
                    detail: serde_json::json!({ "command": "rm -rf target" }),
                },
            });
            h.app.seed_transcript(t);
            written += save_frame(&mut h, &dir, "01-permission-ask.png");
        }

        // 2. A narrow window: the layout tier has to drop a pane, and say which one.
        {
            let (ws, _keep) = scratch_workspace("frames-narrow");
            let mut h = Harness::new(ws.clone());
            h.input.screen_rect = Some(egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(620.0, 460.0),
            ));
            let mut t = Transcript::new(ws);
            t.push_user("what changed in the frame?");
            h.app.seed_transcript(t);
            written += save_frame(&mut h, &dir, "02-narrow-window.png");
        }

        // 3. The sessions list, with the sidebar on it.
        {
            let (ws, _keep) = scratch_workspace("frames-sessions");
            let (session, notices) = faux_session(&ws);
            let mut h = Harness::with_session(ws, session, notices);
            h.app.set_sidebar_view(SidebarView::Sessions);
            written += save_frame(&mut h, &dir, "03-sessions.png");
        }

        // 4. The first run: no transcript, nothing chosen, the empty state a new user meets.
        {
            let (ws, _keep) = scratch_workspace("frames-empty");
            let mut h = Harness::new(ws.clone());
            h.app.seed_transcript(Transcript::new(ws));
            written += save_frame(&mut h, &dir, "04-empty-transcript.png");
        }

        // 5. A file open in the editor. **The editor pane is the largest surface in this window
        //    and the one no rendered frame has ever shown** — the tab strip has a test that
        //    asserts the field, and every frame so far had only the editor's *empty* state on it.
        {
            let (ws, _keep) = scratch_workspace("frames-editor");
            std::fs::write(
                ws.join("alpha.rs"),
                "fn alpha() {\n    // a comment long enough to need eliding in a narrow pane\n    \
                 let total = compute_the_total(&items, &rates);\n    report(total)\n}\n",
            )
            .expect("a file with something in it");
            let mut h = Harness::new(ws.clone());
            h.app.open_file("alpha.rs");
            written += save_frame(&mut h, &dir, "05-file-open.png");
        }

        // 6. A change under review. **The review is what the `changed` row opens**, and it is a
        //    surface a reader reaches only after clicking through two other things.
        {
            let (ws, _keep) = scratch_workspace("frames-review");
            let mut h = Harness::new(ws.clone());
            let mut t = Transcript::new(ws);
            t.push_user("tighten the guard in lib.rs");
            t.push_change(&crate::diff::Change {
                path: "src/lib.rs".into(),
                lines: vec![
                    crate::diff::DiffLine::Context("pub fn check(state: &State) -> bool {".into()),
                    crate::diff::DiffLine::Removed("    assert!(matches!(guard))".into()),
                    crate::diff::DiffLine::Added(
                        "    assert!(matches!(guard), \"the guard must hold\")".into(),
                    ),
                    crate::diff::DiffLine::Context("}".into()),
                ],
                created: false,
                coarse: false,
            });
            h.app.seed_transcript(t);
            if let Some(change) = h.app.transcript.diffs().first().cloned() {
                h.app.review = Some(change);
            }
            written += save_frame(&mut h, &dir, "06-change-review.png");
        }

        // 7. **A permission ask with its buttons — the state the whole product exists for, and
        //    the one nobody had ever seen drawn.** The frame above (01) shows the ask *without*
        //    them, because its window has no session and the pane says "not connected to an
        //    agent". A previous milestone recorded that reaching the drawn buttons was not
        //    possible: `ChannelApprovalUi` has no way to *park* an ask, only the agent can produce
        //    one — and the drawer reads `pending_approvals()`, so a seeded transcript row has
        //    nothing behind it.
        //
        //    It **is** reachable, and the existing `channel_ui` test shows how: spawn the ask and
        //    leave it parked. The runtime is the other half — `ask_line` and the round driver both
        //    use `try_current`, which degrades **quietly** without a context, and this is exactly
        //    the trap that file's doc records.
        {
            use epsilon_contracts::agent_event::PermissionRequest;
            let (ws, _keep) = scratch_workspace("frames-approval-live");
            let (session, notices) = faux_session(&ws);
            let runtime = session.runtime_handle();
            let mut h = Harness::with_session(ws, session, notices);

            let ask = PermissionRequest {
                request_id: "ask-live".into(),
                capability: "bash".into(),
                pattern: "rm -rf target".into(),
                detail: serde_json::json!({ "command": "rm -rf target" }),
            };
            let ui = Arc::clone(h.app.session.as_ref().expect("a session").approvals());
            // `ask` is a trait method on `ApprovalUi`; without the trait in scope the park is a
            // compile error rather than a finding, which is at least honest.
            use epsilon_safety::ApprovalUi as _;
            let _guard = runtime.enter();
            tokio::spawn(async move {
                // Left parked on purpose: this is the state a blocked run is in.
                let _ = ui.ask(ask).await;
            });
            while h
                .app
                .session
                .as_ref()
                .expect("a session")
                .pending_approvals()
                .is_empty()
            {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }

            let mut t = Transcript::new(std::env::temp_dir());
            t.push_user("delete the build directory");
            h.app.seed_transcript(t);

            // **Render FIRST, then read the accessibility tree — in that order, always.** `a11y_frame`
            // clears `textures_delta` on purpose, and the font atlas lives in it, so asking for
            // the tree first and rasterising after produces **a blank white image**: the frame
            // that recorded the buttons was the frame that destroyed the glyphs. The raster test's
            // own comment says this and this ordering is the other half of it.
            written += save_frame(&mut h, &dir, "07-approval-with-buttons.png");

            // **Asserted on the accessibility tree, not on `FrameSummary::rows`.** The buttons
            // are egui widgets, and `rows` records only *painted* text — so the first version of
            // this assertion looked for the word `Allow` in a list of painted strings and could
            // not have passed. The node is the better question anyway: it is what the platform
            // receives, and an approval the reader cannot reach by name is not an approval.
            let nodes = h.named_nodes();
            assert!(
                nodes.iter().any(|(label, _)| label == "Allow"),
                "**the Allow button must be on screen** — this is the interaction the product is \
                 built around, and the first version of this check looked for it in the painted \
                 text where a button's name never appears. Nodes: {:?}",
                nodes.iter().map(|(l, _)| l).collect::<Vec<_>>()
            );
            assert!(
                nodes.iter().any(|(label, _)| label == "Refuse"),
                "and refusing must be one click away from allowing: {:?}",
                nodes.iter().map(|(l, _)| l).collect::<Vec<_>>()
            );
            drop(_guard);
        }

        // 8. **The light palette.** `AGENTS.md` §8 asks for AA contrast "in every theme", and the
        //    window has had a light one since the theme module grew it — with **no rendered frame
        //    of it anywhere**, so the one claim nobody could check was the one about a theme a
        //    person would choose on a bright desk.
        {
            let (ws, _keep) = scratch_workspace("frames-light");
            let mut h = Harness::new(ws.clone());
            h.app.theme = crate::theme::IdeTheme::from(epsilon_tui::ui::theme::ThemeMode::Light);
            let mut t = Transcript::new(ws);
            t.push_user("what does the contrast look like here?");
            h.app.seed_transcript(t);
            written += save_frame(&mut h, &dir, "08-light-palette.png");
        }

        // 9. **200% text.** Required by the same section, measured once at M305 for the title
        //    bar's collision, and never looked at as a picture.
        {
            let (ws, _keep) = scratch_workspace("frames-zoom");
            std::fs::write(
                ws.join("alpha.rs"),
                "fn alpha() {\n    let total = compute(&items);\n    report(total)\n}\n",
            )
            .expect("a file to open");
            let mut h = Harness::new(ws.clone());
            h.app.open_file("alpha.rs");
            h.at_zoom(2.0);
            written += save_frame(&mut h, &dir, "09-text-200-percent.png");
        }

        // 10. **A conversation with tool calls in it — the transcript's commonest content, and the
        //     only state nobody had ever drawn.** Every frame so far showed prompts and answers;
        //     a reader spends most of their time looking at what the agent *did*. One call
        //     succeeded and one failed, because the failed marker is the one that has to be right.
        {
            use epsilon_contracts::agent_event::AgentEvent;
            let (ws, _keep) = scratch_workspace("frames-tools");
            let mut h = Harness::new(ws.clone());
            let mut t = Transcript::new(ws);
            t.push_user("run the suite and tell me what broke");
            t.apply(&AgentEvent::ToolExecutionStart {
                tool_call_id: "c1".into(),
                tool_name: "bash".into(),
                args: serde_json::json!({ "command": "cargo test -p epsilon-session" }),
            });

            t.apply(&AgentEvent::ToolExecutionEnd {
                tool_call_id: "c1".into(),
                tool_name: "bash".into(),
                is_error: false,
            });
            t.apply(&AgentEvent::ToolExecutionStart {
                tool_call_id: "c2".into(),
                tool_name: "write".into(),
                args: serde_json::json!({ "path": "src/lib.rs" }),
            });
            t.apply(&AgentEvent::ToolExecutionEnd {
                tool_call_id: "c2".into(),
                tool_name: "write".into(),
                is_error: true,
            });
            t.apply(&AgentEvent::MessageStart {
                message: Box::new(epsilon_contracts::message::Message::Assistant(
                    epsilon_contracts::message::AssistantMessage {
                        id: "m2".into(),
                        timestamp_ms: 0,
                        provider: "faux".into(),
                        model_id: "faux-1".into(),
                        content: vec![epsilon_contracts::content::ContentBlock::text(
                            "The session crate is green. The write failed because the path does \
                             not exist, so nothing changed on disk.",
                        )],
                        stop_reason: epsilon_contracts::message::StopReason::EndTurn,
                        error_message: None,
                        usage: Default::default(),
                    },
                )),
            });
            t.apply(&AgentEvent::MessageEnd {
                message: Box::new(epsilon_contracts::message::Message::Assistant(
                    epsilon_contracts::message::AssistantMessage {
                        id: "m2".into(),
                        timestamp_ms: 0,
                        provider: "faux".into(),
                        model_id: "faux-1".into(),
                        content: vec![epsilon_contracts::content::ContentBlock::text(
                            "The session crate is green. The write failed because the path does \
                             not exist, so nothing changed on disk.",
                        )],
                        stop_reason: epsilon_contracts::message::StopReason::EndTurn,
                        error_message: None,
                        usage: Default::default(),
                    },
                )),
            });
            h.app.seed_transcript(t);
            written += save_frame(&mut h, &dir, "10-tool-calls.png");
        }

        // 11. **A long conversation — the state the virtualisation claim is about.** M285 measured that
        //     800 rich rows cost only the rows that are drawn, and every test since has been a
        //     *number*; no frame has ever shown what a long transcript actually looks like, which
        //     is where a scroll offset that is a row too far, a sticky-to-bottom that does not
        //     stick, or a composer that has been pushed off the pane would show.
        {
            use epsilon_contracts::agent_event::AgentEvent;
            let (ws, _keep) = scratch_workspace("frames-long");
            let mut h = Harness::new(ws.clone());
            let mut t = Transcript::new(ws);
            for i in 0..120 {
                t.push_user(&format!("question {i}: what does the layout module own?"));
                t.apply(&AgentEvent::MessageEnd {
                    message: Box::new(epsilon_contracts::message::Message::Assistant(
                        epsilon_contracts::message::AssistantMessage {
                            id: format!("m{i}"),
                            timestamp_ms: 0,
                            provider: "faux".into(),
                            model_id: "faux-1".into(),
                            content: vec![epsilon_contracts::content::ContentBlock::text(
                                "It owns the spatial model: the header, the transcript, the \
                                 composer and the footer, and it decides what a too-small \
                                 terminal gives up first.",
                            )],
                            stop_reason: epsilon_contracts::message::StopReason::EndTurn,
                            error_message: None,
                            usage: Default::default(),
                        },
                    )),
                });
            }
            h.app.seed_transcript(t);
            // `save_frame` renders twice on purpose, so this is the **steady state**: a scroll
            // area's stick-to-bottom is "keep it at the bottom *as content grows*", which needs a
            // prior frame to be stuck from, and the first frame after seeding 120 pairs shows the
            // **top** of the conversation — indistinguishable from a broken stick. Dumped once,
            // this frame read as a defect. It is not one: at the tail, the newest exchange is
            // fully visible above the composer, and 71 shapes are drawn for 240 rows, which is
            // the virtualisation claim as a picture rather than as a number.
            written += save_frame(&mut h, &dir, "11-long-conversation.png");
        }

        // 12. **The high-contrast palette — the one mode that paints its own page (M312).**
        //     `AGENTS.md` §8 asks for high-contrast support and the window had none; this is the
        //     only frame that can show it, because a terminal has no background to show and a
        //     window does. The numbers are held by tests in the terminal's theme; what a frame
        //     adds is whether the whole window reads as one system rather than a page with some
        //     black on it.
        {
            let (ws, _keep) = scratch_workspace("frames-contrast");
            let mut h = Harness::new(ws.clone());
            h.app.theme =
                crate::theme::IdeTheme::from(epsilon_tui::ui::theme::ThemeMode::HighContrast);
            let mut t = Transcript::new(ws);
            t.push_user("what does the contrast look like here?");
            t.push_notice("Pure white on black: 21:1, and every role clears 7:1.");
            t.push_notice("failed gate: cargo test -p epsilon-session");
            h.app.seed_transcript(t);
            written += save_frame(&mut h, &dir, "12-high-contrast.png");
        }

        // 13. The quit guard — **deliberately not rendered here, and the reason is the
        // finding.**
        //
        // Added to this showcase, it produced a **blank white PNG** with three different
        // dialog bodies (an `egui::Modal`, two wrapping `Label`s, and two `paint_text`
        // calls all rasterised to the identical 7,059-byte image — so the output did not
        // depend on the dialog at all), and once the extra priming paint was removed the
        // whole test process died with `STATUS_STACK_BUFFER_OVERRUN`.
        //
        // **A screen that cannot be looked at, and a renderer that crashes on it, are two
        // findings — and neither is a reason to ship the frame.** The guard itself is tested
        // through the real draw path (`closing_the_window_asks_before_it_loses_a_turn` and its
        // three siblings assert the dialog's rows on the frame and that the frame emits no
        // `ViewportCommand::Close`), and the buttons are real egui widgets, so the keyboard
        // and pointer routes are covered. What is **not** verified is how it looks, and
        // `docs/STATUS.md` records that as open rather than letting this comment stand in
        // for evidence.

        eprintln!(
            "desktop_frames: wrote {written} frame(s) to {}",
            dir.display()
        );
    }

    /// A palette row can be **clicked**, not only typed at and confirmed.
    ///
    /// A palette whose rows can only be reached by typing is a worse editor than the file tree
    /// it was built to replace: a reader who sees the file in the list and reaches for it with
    /// the mouse gets a box that does nothing, which is the "the window ignored me" state this
    /// window has now produced three separate ways.
    #[test]
    fn clicking_a_palette_row_opens_it() {
        let (dir, _keep) = scratch_workspace("palette-click");
        let mut h = Harness::new(dir);
        let _ = h.paint();
        let open = h.chord(egui::Modifiers::CTRL, egui::Key::P);

        // The row must be somewhere a pointer can reach: its rectangle is recorded, not just
        // its text.
        let row = open
            .row_rects
            .iter()
            .find(|(region, _)| region == "palette")
            .map(|(_, r)| *r)
            .expect("palette rows record where they were drawn");

        let after = h.click(row.center());
        assert!(
            h.app.open.is_some(),
            "clicking a file row must open that file.\n{}",
            after.describe()
        );
        assert!(
            h.app.palette.is_none(),
            "and the palette must close, the same as pressing Enter"
        );
    }

    /// The row that was clicked is the row that opens — not the first, and not the selected one.
    ///
    /// One row would satisfy "clicking a row opens it". Three, with the click on the last, is
    /// the claim.
    #[test]
    fn clicking_a_later_row_opens_that_row_and_not_the_first() {
        let (dir, _keep) = scratch_workspace("palette-click3");
        let mut h = Harness::new(dir.clone());
        // A third file, so "the last row" is genuinely not the first.
        std::fs::write(dir.join("gamma.rs"), "fn gamma() {}\n").expect("a third file");
        h.app.tree = crate::workspace::FileTree::load(&dir);
        let _ = h.paint();
        let _ = h.chord(egui::Modifiers::CTRL, egui::Key::P);
        h.type_text("rs");

        let open = h.paint();
        let rows: Vec<egui::Rect> = open
            .row_rects
            .iter()
            .filter(|(region, _)| region == "palette")
            .map(|(_, r)| *r)
            .collect();
        assert!(rows.len() >= 3, "expected several rows, got {}", rows.len());

        let last = rows[rows.len() - 1];
        let after = h.click(last.center());
        assert!(
            h.app.open.is_some(),
            "clicking a row must open something.\n{}",
            after.describe()
        );
        assert_ne!(
            h.app.open.as_ref().map(|f| f.path.as_str()),
            Some("alpha.rs"),
            "the first row opened: the click landed on the wrong rectangle"
        );
        assert!(
            after.region("editor").iter().any(|t| t.contains("·")),
            "and the editor must now be showing the file that was clicked.\n{}",
            after.describe()
        );
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
        /// The directory this window reads and writes its layout settings in.
        ///
        /// **Held, because it has to outlive the app** — the window writes into it on every
        /// gesture. See [`bounded`].
        settings: tempfile::TempDir,
    }

    /// Point `app` at a throwaway settings directory, and load whatever is (not) in it.
    ///
    /// **`IdeApp::without_agent` resolves `~/.config/epsilon`.** Without this, the first test
    /// that pressed `Ctrl`+`B` would write the suite's idea of a layout over the one the
    /// person running the suite has, in the one directory on this machine that is not
    /// disposable — and the rest of the run would then describe a layout nobody chose.
    /// `bounded` is what makes "the window remembers" testable rather than merely asserted.
    ///
    /// The [`tempfile::TempDir`] is returned rather than dropped, because a settings
    /// directory that vanished before the app was dropped would make the write fail — which
    /// is the other half of the same mistake, and quieter.
    fn bounded(app: &mut IdeApp) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("a settings directory");
        app.prefs_dir = Some(dir.path().to_path_buf());
        app.prefs = Preferences::load_from(app.workspace(), app.prefs_dir.as_deref());
        dir
    }

    impl Harness {
        fn new(workspace: std::path::PathBuf) -> Self {
            Self::sized(workspace, 1440.0, 900.0)
        }
        /// A harness around a window that has a **real agent**, which [`Self::sized`]
        /// cannot build — it is deliberately the agent-less state, for the test that
        /// asserts the agent-less state.
        ///
        /// The context is built first and then handed to `IdeApp::new`, because
        /// `attach_bridge` keeps that one and the frame is painted through it: a bridge
        /// attached to a different context would repaint a surface nobody is looking at,
        /// which is a passing test for a window that does not update.
        fn with_session(
            workspace: std::path::PathBuf,
            session: crate::session::IdeSession,
            notices: Vec<String>,
        ) -> Self {
            let ctx = egui::Context::default();
            let mut app = IdeApp::new(Options::for_workspace(workspace), session, notices, &ctx);
            let settings = bounded(&mut app);
            Self {
                ctx,
                app,
                settings,
                input: egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1440.0, 900.0),
                    )),
                    // **`focused: true`, and it is load-bearing.** `RawInput` defaults it
                    // to false and egui **drops keyboard events for an unfocused window**
                    // (`tests/window.rs` says the same about its own harness). This
                    // harness built the default, so nothing typed into the composer ever
                    // reached it and no in-crate test could have noticed: the composer's
                    // `Shift+Enter` promise, and the `Ctrl`+`+` zoom keys, were both
                    // untestable here rather than unverified because of it.
                    focused: true,
                    ..Default::default()
                },
            }
        }
        fn sized(workspace: std::path::PathBuf, w: f32, h: f32) -> Self {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(w, h),
                )),
                // Focused for the same reason as `with_session` above.
                focused: true,
                ..Default::default()
            };
            let mut app = IdeApp::without_agent(Options::for_workspace(workspace));
            let settings = bounded(&mut app);
            Self {
                ctx: egui::Context::default(),
                app,
                input,
                settings,
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
        /// Raise a close request the way the window's close button does, and report whether
        /// the application let it through.
        ///
        /// **Goes through [`IdeApp::answer_close_request`], the same function the platform
        /// flag reaches**, because `ViewportInfo::close_requested` is a flag egui only ever
        /// *reads* — a bare `Context` cannot raise it, and a test that poked the field
        /// instead would be testing a path no window takes.
        fn request_close(&mut self) -> bool {
            self.app.answer_close_request(&self.ctx)
        }

        /// What the application asked the **window** to do on the last painted frame.
        ///
        /// **This is how a test sees that a close was refused.** The application never closes
        /// a window itself — it sends `ViewportCommand::Close` and lets the window honour it —
        /// so "did the window close" is a question about the commands the frame emitted, not
        /// about any field the application holds. Without it, a test asserting the dialog
        /// appeared would pass against a build that asked and then closed anyway.
        fn viewport_commands(&mut self) -> Vec<egui::ViewportCommand> {
            let mut full = self.ctx.run_ui(self.input.clone(), |ui| {
                let _ = self.app.paint(ui);
            });
            full.textures_delta.clear();
            std::mem::take(
                &mut full
                    .viewport_output
                    .get_mut(&egui::ViewportId::ROOT)
                    .expect("the root viewport is always present")
                    .commands,
            )
        }

        /// True when the application answered a close request with "yes".
        fn close_was_accepted(&self, commands: &[egui::ViewportCommand]) -> bool {
            commands
                .iter()
                .any(|c| matches!(c, egui::ViewportCommand::Close))
        }

        /// The window at `zoom`, with the **same content rect** as it had at 1.0.
        ///
        /// `Context::set_zoom_factor` does two things at the start of the next pass
        /// (`egui-0.36.2/src/context.rs:438-450`): it rescales `screen_rect` by
        /// `old / new` and raises `pixels_per_point`. So a bare `set_zoom_factor(2.0)`
        /// halves the content rect and drops a 1440×900 window into a lower layout tier —
        /// the title bar and the status bar stop being drawn at all — and a test comparing
        /// the two frames would be comparing two different layouts rather than two
        /// different font sizes.
        ///
        /// **The order below is the whole trick, and it is not obvious.** egui rescales
        /// the rect it has *stored* from the previous pass, not the one the next pass
        /// carries (`context.rs:444-447` reads `viewport.input`), so the window has to be
        /// taught its enlarged rect **before** the zoom changes. Do it the other way round
        /// and the frame under test is laid out against 720×450 — a different layout, a
        /// different tier, a different set of panes — and the comparison silently measures
        /// something else.
        fn at_zoom(&mut self, zoom: f32) {
            let rect = self
                .input
                .screen_rect
                .expect("every harness frame is given a screen rect");
            let widened = egui::Rect::from_min_size(rect.min, rect.size() * zoom);
            self.input.screen_rect = Some(widened);
            let mut warm = self.ctx.run_ui(self.input.clone(), |_| {});
            warm.textures_delta.clear();
            self.ctx.set_zoom_factor(zoom);
            let mut warm = self.ctx.run_ui(self.input.clone(), |_| {});
            warm.textures_delta.clear();
            // Hand the harness back the window it started with: this pass ended with a
            // content rect of exactly `rect`, and the frame under test must be laid out
            // against those same points.
            self.input.screen_rect = Some(rect);
        }
        /// One frame painted **with AccessKit switched on**, keeping the tree update.
        ///
        /// This is the accessibility counterpart of [`Self::full_frame`], and it exists
        /// because the whole claim in ADR-022 is that egui can carry a name to a screen
        /// reader. Reading it back is therefore the only assertion worth making: a test
        /// that read a field in `IdeApp` would pass with the metadata computed and never
        /// attached, which is the failure this milestone exists to close.
        ///
        /// [`egui::Context::enable_accesskit`] (`egui-0.36.2/src/context.rs:3701`) turns on
        /// the per-pass state that `Context::end_pass` turns into
        /// `PlatformOutput::accesskit_update` (`egui-0.36.2/src/context.rs:2710`) — the
        /// **same** `accesskit::TreeUpdate` the platform adapter hands to the OS. So this
        /// is the real tree, not a proxy for it.
        fn a11y_frame(&mut self) -> egui::FullOutput {
            self.ctx.enable_accesskit();
            let full = self.full_frame();
            assert!(
                full.platform_output.accesskit_update.is_some(),
                "egui produced no AccessKit tree update. The node was never built, so \
                 nothing below this assertion would be measuring the screen."
            );
            full
        }
        /// Every named node in this frame's accessibility tree: `(label, role)`.
        ///
        /// Only nodes with a **label**, because that is what a screen reader announces and
        /// therefore what the accessible name means in practice. `Role` is named through
        /// `egui::accesskit` (egui re-exports it at `egui-0.36.2/src/lib.rs:434`) so this
        /// crate needs no direct dependency on it.
        fn named_nodes(&mut self) -> Vec<(String, egui::accesskit::Role)> {
            let full = self.a11y_frame();
            full.platform_output
                .accesskit_update
                .as_ref()
                .expect("checked by a11y_frame")
                .nodes
                .iter()
                .filter_map(|(_, node)| node.label().map(|l| (l.to_string(), node.role())))
                .collect()
        }
        /// The node egui registered under `id`, if this frame produced one.
        ///
        /// `TreeUpdate::nodes` is a `Vec` of pairs rather than a map, so this is the search
        /// egui's own ids are meant to be looked up with (`Id::accesskit_id`,
        /// `egui-0.36.2/src/id.rs:103`). Cloned, because the update is dropped with the frame.
        fn node_for(&mut self, id: egui::Id) -> Option<egui::accesskit::Node> {
            let want = id.accesskit_id();
            let full = self.a11y_frame();
            full.platform_output
                .accesskit_update
                .as_ref()
                .expect("checked by a11y_frame")
                .nodes
                .iter()
                .find(|(node_id, _)| *node_id == want)
                .map(|(_, node)| node.clone())
        }
        /// The composer's own node, found by the widget id [`composer_id`] gives it.
        ///
        /// **By id, not by name.** "The node called Composer" is satisfied by a second node
        /// added beside egui's, which is the one thing this arrangement must never become.
        fn composer_node(&mut self) -> egui::accesskit::Node {
            self.node_for(composer_id())
                .expect("egui builds a node for every TextEdit, the composer included")
        }
        /// The label of the one node that starts with `prefix`, or a failure naming
        /// everything that *is* in the tree.
        ///
        /// The failure message matters: "no such node" on its own is the difference
        /// between an id that moved, a region that stopped being drawn and a call that was
        /// deleted, and all three look identical from inside a `contains`.
        fn node_named(&mut self, prefix: &str) -> String {
            let all = self.named_nodes();
            let hit: Vec<_> = all
                .iter()
                .filter(|(label, _)| label.starts_with(prefix))
                .collect();
            assert!(
                !hit.is_empty(),
                "no accessible name starts with {prefix:?}.\nthe tree held: {:#?}",
                all.iter().map(|(l, _)| l).collect::<Vec<_>>()
            );
            hit[0].0.clone()
        }
        /// Scroll the transcript **the way a reader does**: the pointer over the pane
        /// and a wheel notch.
        ///
        /// Both halves are needed and neither is decoration. `ScrollArea::end` only takes a
        /// wheel when the pointer is inside the scroll area's own rect
        /// (`egui-0.36.2/src/containers/scroll_area.rs:1215-1219`), and the notch is an
        /// `egui::Event` rather than a written offset because the scroll position is the
        /// one thing a test must not be able to set directly: a test that assigns the offset
        /// proves the drawing code honours an offset the product has no way to produce.
        ///
        /// **`delta` follows egui's own sign**: positive `y` scrolls *up*, towards the start
        /// of the transcript. A notch below eight points is taken whole on the frame it
        /// arrives (`WheelState`, `egui-0.36.2/src/input_state/wheel_state.rs:140-150`), so
        /// the caller gets the number of points it asked for rather than a smoothed
        /// fraction of it — which is what makes a test that counts notches meaningful.
        fn wheel_over_the_agent_pane(&mut self, delta: f32) -> FrameSummary {
            let over = self.app.layout().agent.center();
            let mut input = self.input.clone();
            input.events = vec![
                egui::Event::PointerMoved(over),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0.0, delta),
                    modifiers: egui::Modifiers::default(),
                    phase: egui::TouchPhase::Move,
                },
            ];
            let mut out = FrameSummary::default();
            let mut full = self.ctx.run_ui(input, |ui| out = self.app.paint(ui));
            full.textures_delta.clear();
            out
        }
        /// Type `text` one character per event, the way a keyboard delivers it.
        fn type_text(&mut self, text: &str) {
            let events: Vec<egui::Event> = text
                .chars()
                .map(|c| egui::Event::Text(c.to_string()))
                .collect();
            let mut input = self.input.clone();
            input.events = events;
            let mut full = self.ctx.run_ui(input, |ui| {
                self.app.paint(ui);
            });
            full.textures_delta.clear();
        }
        /// Press and release one key, and return the frame it produced.
        fn press(&mut self, key: egui::Key) -> FrameSummary {
            self.chord(egui::Modifiers::NONE, key)
        }
        /// Press and release one key with modifiers, and return the frame it produced.
        fn chord(&mut self, modifiers: egui::Modifiers, key: egui::Key) -> FrameSummary {
            let mut input = self.input.clone();
            input.events = vec![egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            }];
            let mut out = FrameSummary::default();
            let mut full = self.ctx.run_ui(input, |ui| out = self.app.paint(ui));
            full.textures_delta.clear();
            out
        }
        /// Press one key and keep **both** answers: what the frame said and what the
        /// renderer would have been handed.
        ///
        /// [`Self::chord`] throws the shapes away, which is right for "what does the
        /// frame say" and useless for "was the focus ring painted" — and two frames
        /// would answer a different question, because the ring on frame *n+1* is the
        /// ring frame *n* produced, and a test cannot then tell whether the key did it.
        fn chord_full(
            &mut self,
            modifiers: egui::Modifiers,
            key: egui::Key,
        ) -> (FrameSummary, egui::FullOutput) {
            let mut input = self.input.clone();
            input.events = vec![egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            }];
            let mut out = FrameSummary::default();
            let mut full = self.ctx.run_ui(input, |ui| out = self.app.paint(ui));
            full.textures_delta.clear();
            (out, full)
        }
        /// One painted frame, keeping everything the renderer would have been handed.
        fn full_frame(&mut self) -> egui::FullOutput {
            let mut input = self.input.clone();
            input.events = Vec::new();
            let mut full = self.ctx.run_ui(input, |ui| {
                self.app.paint(ui);
            });
            full.textures_delta.clear();
            full
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
        /// Press at `from`, move through `to`, release — and return the frame the release
        /// produced.
        ///
        /// **Four frames, and the middle one is the whole trick.** egui decides a press has
        /// become a drag from the pointer having *moved* past a threshold since the press,
        /// so a press and a release delivered on adjacent frames resolve as a click, and
        /// [`egui::Response::drag_delta`] is zero for the entire gesture. A single-frame
        /// helper would therefore resize nothing and look like a working harness — the same
        /// trap `click` records above, one gesture further on.
        ///
        /// The move is delivered as its own frame so the drag has a frame to be *decided* in,
        /// which is what a hand does. Intermediate positions are not interpolated: one hop is
        /// what a person produces in a single frame, and interpolating would let a test
        /// prove a clamp by teleporting rather than by dragging.
        fn drag(&mut self, from: egui::Pos2, to: egui::Pos2) -> FrameSummary {
            let button = |pos: egui::Pos2, pressed: bool| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            let feed = |h: &mut Harness, events: Vec<egui::Event>| {
                let mut input = h.input.clone();
                input.events = events;
                let mut full = h.ctx.run_ui(input, |ui| {
                    h.app.paint(ui);
                });
                full.textures_delta.clear();
            };
            feed(self, vec![egui::Event::PointerMoved(from)]);
            feed(
                self,
                vec![egui::Event::PointerMoved(from), button(from, true)],
            );
            feed(self, vec![egui::Event::PointerMoved(to)]);
            let mut out = FrameSummary::default();
            let mut input = self.input.clone();
            input.events = vec![
                egui::Event::PointerMoved(to),
                egui::Event::PointerMoved(to),
                button(to, false),
            ];
            let mut full = self.ctx.run_ui(input, |ui| out = self.app.paint(ui));
            full.textures_delta.clear();
            out
        }
    }

    /// **Closing the window asks first, and a close request does not close the window.**
    ///
    /// `AGENTS.md` §6: never lose a user's work. Closing used to end the process, so a turn
    /// in flight stopped where it was and a parked permission ask was thrown away — and a
    /// reader who had not pressed `/clear` had no way to know which of those two had happened.
    ///
    /// **Two assertions, because either alone is a test that cannot see the thing.** The
    /// dialog appearing would pass against a build that asks and then closes anyway; the
    /// window still being open is a claim about the commands the frame emitted.
    #[test]
    fn closing_the_window_asks_before_it_loses_a_turn() {
        let (ws, _keep) = scratch_workspace("quit-guard");
        let (session, notices) = faux_session(&ws);
        let mut h = Harness::with_session(ws, session, notices);
        // Say something, so the conversation is not empty and the warning is a real one.
        h.type_text("hello");
        let _ = h.paint();

        assert!(
            !h.request_close(),
            "a close request must be refused while there is something to lose"
        );
        let frame = h.paint();
        assert!(
            h.app.quit_confirm.is_some(),
            "and the refusal must be a question, not a silent block.\n{}",
            frame.describe()
        );
        assert!(
            frame
                .region("agent")
                .iter()
                .any(|t| t.contains("Close this window?")),
            "which must be on screen and named.\n{}",
            frame.describe()
        );

        // **The window is still open**, and that is the part that matters.
        let commands = h.viewport_commands();
        assert!(
            !h.close_was_accepted(&commands),
            "the frame must not ask the window to close. It asked for: {commands:?}"
        );
    }

    /// **A second close request does not fall through into a close.**
    ///
    /// A guard that can be beaten by holding the window's close button is not a guard, and
    /// this is the case a reader would find before a test did.
    #[test]
    fn a_second_close_request_does_not_fall_through() {
        let (ws, _keep) = scratch_workspace("quit-guard-2");
        let (session, notices) = faux_session(&ws);
        let mut h = Harness::with_session(ws, session, notices);
        assert!(!h.request_close());
        let _ = h.paint();
        assert!(
            !h.request_close(),
            "a second request while the dialog is up is still refused"
        );
        let frame = h.paint();
        assert!(
            h.app.quit_confirm.is_some(),
            "the dialog stays up rather than re-arming"
        );
        let commands = h.viewport_commands();
        assert!(
            !h.close_was_accepted(&commands),
            "and the window is still open after two requests.\n{}",
            frame.describe()
        );
    }

    /// Refusing a close cancels the platform close — not just records a dialog.
    ///
    /// The production hole this closes: nothing ever sent `CancelClose`, and
    /// eframe closes a viewport whose close was not cancelled in the same
    /// frame — so the dialog, the default-safe buttons and the second-request
    /// logic could never execute outside a test. This asserts the command a
    /// frame carries, through the same `request_close` seam the platform flag
    /// reaches.
    #[test]
    fn refusing_a_close_cancels_the_platform_close() {
        let (ws, _keep) = scratch_workspace("quit-cancel");
        let (session, notices) = faux_session(&ws);
        let mut h = Harness::with_session(ws, session, notices);
        h.type_text("hello");
        let _ = h.paint();

        assert!(
            !h.request_close(),
            "a close with a conversation on screen is refused"
        );
        let commands = h.viewport_commands();
        assert!(
            commands
                .iter()
                .any(|c| matches!(c, egui::ViewportCommand::CancelClose)),
            "refusing must cancel the platform close, or the dialog is drawn over a window that is already gone: {commands:?}"
        );
        assert!(
            !h.close_was_accepted(&commands),
            "and must not ask for the close in the same breath"
        );
    }

    /// A typed draft keeps its guard.
    ///
    /// An unsent draft is work even with no agent to send it to, and closing
    /// over it would be the silent data loss `AGENTS.md` §6 exists to prevent.
    /// There is no silent path that a draft could slip through: every close is
    /// asked about, empty window or not, by tested product rule.
    #[test]
    fn a_typed_draft_keeps_its_guard() {
        let (ws, _keep) = scratch_workspace("quit-draft");
        let mut h = Harness::new(ws);
        // Set, not typed: the typing path is tested elsewhere, and what this
        // decides on is the draft's content, however it got there.
        h.app.draft = "unsent words".to_string();
        let _ = h.paint();
        assert!(
            !h.request_close(),
            "an unsent draft is work, even with no agent to send it to"
        );
        assert!(
            h.app.quit_confirm.is_some(),
            "and the refusal must be a question"
        );
    }

    /// **`Escape` keeps the window, and the dialog goes away.**
    ///
    /// **The safe answer is the one a stray key gets**, which is the reflexive-`Enter` case
    /// `AGENTS.md` §6 and DSH's `RiskConfirmation` both exist to prevent.
    #[test]
    fn escape_keeps_the_window_open() {
        let (ws, _keep) = scratch_workspace("quit-guard-3");
        let (session, notices) = faux_session(&ws);
        let mut h = Harness::with_session(ws, session, notices);
        assert!(!h.request_close());
        let _ = h.paint();
        h.press(egui::Key::Escape);
        let frame = h.paint();
        assert!(
            h.app.quit_confirm.is_none(),
            "escape dismisses the question.\n{}",
            frame.describe()
        );
        let commands = h.viewport_commands();
        assert!(
            !h.close_was_accepted(&commands),
            "and the window is still open"
        );
    }

    /// **A window with nothing in flight is not told a turn is running.**
    ///
    /// The other arm of the same rule, and the one that keeps the guard usable: a dialog that
    /// cries wolf for an empty window teaches a reader to click through it, and then it is not
    /// a guard for the case that matters.
    #[test]
    fn an_empty_window_says_so_and_still_asks() {
        let (dir, _keep) = scratch_workspace("quit-empty");
        let mut h = Harness::new(dir);
        assert!(
            !h.request_close(),
            "even an empty window is asked about, not closed"
        );
        let frame = h.paint();
        assert!(
            h.app.quit_confirm.is_some(),
            "the question is still asked — closing is the reader's decision.\n{}",
            frame.describe()
        );
        assert!(
            frame
                .region("agent")
                .iter()
                .any(|t| t.contains("Nothing is in flight")),
            "and it must not invent work to justify itself.\n{}",
            frame.describe()
        );
    }
    /// **The warning names what is actually at stake.**
    ///
    /// **Specific beats alarming.** "Quit?" for a turn one tool call from finishing teaches a
    /// reader to click through it, and then it is not a guard for the case that matters.
    #[test]
    fn the_warning_names_what_is_actually_at_stake() {
        let ask = QuitAsk {
            running: true,
            pending_approvals: 0,
            conversation: true,
            empty: false,
        };
        let text = ask.sentence();
        assert!(text.contains("a turn is in flight"), "{text}");
        assert!(text.contains("only on screen"), "{text}");

        let idle = QuitAsk {
            running: false,
            pending_approvals: 0,
            conversation: false,
            empty: true,
        };
        assert!(
            idle.sentence().contains("Nothing is in flight"),
            "an empty window must not be told a turn is running: {}",
            idle.sentence()
        );
    }

    /// **A parked permission ask is named as such**, and it is the case that reads as "nothing
    /// is happening": the run is blocked on the reader, so `running` is **false**, and a
    /// warning that only looked at `running` would say there was nothing at stake.
    #[test]
    fn a_parked_ask_is_named_as_such() {
        let ask = QuitAsk {
            running: false,
            pending_approvals: 1,
            conversation: false,
            empty: false,
        };
        let text = ask.sentence();
        assert!(text.contains("1 permission ask is waiting"), "{text}");
        assert!(text.contains("the tool never ran"), "{text}");

        let two = QuitAsk {
            pending_approvals: 2,
            ..ask
        };
        assert!(
            two.sentence().contains("2 permission asks are waiting"),
            "pluralisation is a detail a reader notices: {}",
            two.sentence()
        );
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
    /// **Two files of the same name are two rows a reader can tell apart (M312).**
    ///
    /// Found by reading the platform tree this repository had already captured: the UIA dump
    /// (`audit/evidence/desktop-uia-tree-2026-10-04.txt`) carries **two `TreeItem`s named
    /// `File · README.md`** — one at the workspace root, one under `agents/`. The rows were
    /// named from `Row::name`, which is the last segment.
    ///
    /// This is not a cosmetic naming preference. A screen-reader rotor or a search returns the
    /// names, so a reader asking for "README.md" got two identical entries and no way to choose
    /// — while a sighted reader resolves the same two rows instantly from the indentation. The
    /// eye has information the name does not carry, which is the whole of the defect.
    ///
    /// The assertion is on uniqueness over the **published node names**, not on a substring: a
    /// test that checked "the name contains README.md" would have passed against the broken code,
    /// which is the mistake M305 documents — sixteen assertions on this crate's hint string all
    /// passed while it drew two tofu boxes.
    #[test]
    fn a_file_row_is_named_by_its_path_so_two_of_the_same_name_differ() {
        // **Its own workspace, not `scratch_workspace`.** Adding two `README.md` files to the
        // shared fixture moved four other tests' file counts and tree positions — a fixture
        // shared by every desktop test is the wrong place to put a fact only this test needs.
        let dir = std::env::temp_dir().join("epsilon-tabs-tree-names");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("agents")).expect("scratch workspace");
        std::fs::write(dir.join("README.md"), "# top\n").expect("top readme");
        std::fs::write(dir.join("agents").join("README.md"), "# agents\n").expect("nested readme");
        let _keep = TempWorkspace(dir.clone());
        let mut h = Harness::new(dir);
        let f = h.paint();

        let readmes: Vec<String> = h
            .named_nodes()
            .into_iter()
            .map(|(label, _)| label)
            .filter(|l| l.contains("README.md"))
            .collect();
        assert_eq!(
            readmes.len(),
            2,
            "the workspace holds two README.md files, so both must be published: {readmes:?}\n{}",
            f.describe()
        );
        let unique: std::collections::BTreeSet<&String> = readmes.iter().collect();
        assert_eq!(
            unique.len(),
            readmes.len(),
            "two rows a reader cannot tell apart are one row twice: {readmes:?}"
        );
        assert!(
            readmes.iter().any(|l| l == "File · agents/README.md")
                && readmes.iter().any(|l| l == "File · README.md"),
            "each name identifies its own file, and the top-level one is unchanged: {readmes:?}"
        );
        // The *drawn* row keeps the short name — indenting `agents/README.md` under its own
        // folder would be noise in a tree. `workspace::tests` pins that on the row itself,
        // because this frame summary does not carry the tree's per-row text.
    }

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
        // **`tab:alpha.rs`, not the bare path id.** The tree records the same id for the same
        // path and `hotspot` returns the first match, so this test was clicking the *file
        // tree row* — which opens alpha.rs too, so it passed, and was named for a control it
        // never touched. A test can be green and still be about the wrong thing.
        let tab = before
            .hotspot("tab:alpha.rs")
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
                .hotspot(&format!("tab:{path}"))
                .unwrap_or_else(|| panic!("f{index}.rs has no tab rect"));
            assert!(
                r.right() <= strip.right() + 0.5 && r.width() > 0.0,
                "tab {index} ({path}) is drawn at {r:?}, outside the pane {strip:?}"
            );
        }
    }

    /// **A startup diagnostic composition emitted is on the screen.**
    ///
    /// The defect this closes: `build_runtime_and_gate_runner` announced what it did with
    /// `eprintln!`, and the window captured only `Launch::notices` — which
    /// `resolve_launch` fills *before* composition runs. So a skill that failed to load,
    /// an extension that was skipped, a session that migrated or repaired itself reached a
    /// terminal and nobody else. `AGENTS.md` §2.7 and §8 call that a silent failure, and a
    /// missing feature is the better of the two.
    ///
    /// **A frame assertion, not a state one.** `IdeSession::notices` holding the line proves
    /// only that the vector was filled; this proves a person would read it. M196 and M248
    /// in this repository both shipped features with correct state and nothing drawn.
    ///
    /// **The trigger is a resumed session**, because it announces itself on every launch
    /// that resumes and needs no project trust to do it — a `.epsilon/SYSTEM.md` would have
    /// depended on the user's trust record for a temp directory, which is exactly the kind
    /// of environmental dependence that turns a green test into a coin flip.
    ///
    /// **Negative control.** With `IdeApp::new` not appending the session's notices, the
    /// agent pane contains the launch's own line and the resumed line is absent — the
    /// assertion below names a substring nothing else produces.
    #[test]
    fn a_startup_diagnostic_composition_emitted_is_painted_in_the_window() {
        let ws = tempfile::tempdir().expect("a workspace");
        // A prior session with one message in it, so resuming has something to restore and
        // says so. Dropped before composition: the store holds a lease, and a test that
        // contended with itself would wait for it.
        let prior = ws.path().join("prior.jsonl");
        let mut store =
            epsilon_session::SessionStore::open_in(&prior, ws.path()).expect("the prior session");
        store
            .append(epsilon_contracts::session_entry::EntryKind::Message {
                message: epsilon_contracts::message::Message::Assistant(
                    epsilon_contracts::message::AssistantMessage {
                        id: "prior-1".into(),
                        timestamp_ms: 0,
                        provider: "faux".into(),
                        model_id: "faux-1".into(),
                        content: vec![epsilon_contracts::content::ContentBlock::text(
                            "something said last time",
                        )],
                        stop_reason: epsilon_contracts::message::StopReason::EndTurn,
                        error_message: None,
                        usage: Default::default(),
                    },
                ),
                api_request_sha256: None,
            })
            .expect("the message lands in the log");
        drop(store);

        // **Multi-thread, for the same reason the end-to-end fixture is:** composition and
        // the frame's own spawns both need a runtime that makes progress with nobody
        // inside `block_on`.
        let runtime = std::sync::Arc::new(
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .expect("a test runtime"),
        );
        let launch = epsilon_runtime::resolve_launch(&epsilon_runtime::LaunchRequest {
            workspace: ws.path().to_path_buf(),
            provider: epsilon_runtime::ProviderChoice::Faux,
            resume: epsilon_runtime::Resume::Path(prior.clone()),
            ..epsilon_runtime::LaunchRequest::for_workspace(ws.path())
        })
        .expect("a faux launch always resolves");
        let launch_notices = launch.notices.clone();
        assert!(
            !launch
                .notices
                .iter()
                .any(|n| n.contains("resumed 1 entries")),
            "the launch's own notices cannot contain this: the resume happens later, inside \
             composition. If they can, this test is asserting on the wrong vector."
        );
        let session = runtime
            .block_on(crate::session::IdeSession::start(
                std::sync::Arc::clone(&runtime),
                launch,
            ))
            .expect("composing an agent over a stored session");

        let mut h = Harness::with_session(ws.path().to_path_buf(), session, launch_notices);
        let frame = h.paint();
        let agent = frame.region("agent").join("\n");

        assert!(
            agent.contains("resumed 1 entries"),
            "a session that restored entries announced it, and the window is the only surface \
             that can show it.\n{}",
            frame.describe()
        );
    }

    /// A startup notice longer than its pane is **marked as cut**, not clipped.
    ///
    /// **Found by looking at a rendered frame**, which is the only way this one shows.
    /// [`03-sessions.png`] carried the real launch notice — `no credentials for provider
    /// "openrouter" — set OPENROUTER_API_KEY or run: epsilon auth login openrouter` — and
    /// it ran past the right edge of the agent pane, stopping mid-word with no `…` at all.
    /// Every test passed, because they assert on `FrameSummary::rows` and the notice stack
    /// pushed the **unclipped** text: the drawing could have been deleted and the row would
    /// still be there.
    ///
    /// So the assertion here is deliberately about a notice **too long for the pane**, and
    /// it is the `…` that matters: a notice that fits must still read whole, and one that
    /// does not must say so.
    #[test]
    fn a_launch_notice_too_long_for_its_pane_is_marked_rather_than_clipped() {
        let long = "no credentials for provider \"openrouter\" — set OPENROUTER_API_KEY or run: \
                    epsilon auth login openrouter, which is a deliberately long sentence so \
                    that it cannot possibly fit the agent pane at 1440 points wide";
        let (ws, _keep) = scratch_workspace("notice-budget");
        let (session, notices) = faux_session(&ws);
        let mut h = Harness::with_session(ws, session, notices);
        // **Pushed here, not handed to the constructor**, so the test is about the
        // *drawing* and not about the launch path that would supply this text. The frame
        // below must therefore contain this sentence and no other notice is relevant.
        h.app.notices = vec![long.to_string()];
        let frame = h.paint();
        let agent = frame.region("agent").join("\n");

        assert!(
            agent.contains('…'),
            "a notice that cannot fit must end in the marker, so a reader can tell a cut \
             sentence from a whole one.\n{}",
            frame.describe()
        );
        assert!(
            !agent.contains("which is a deliberately long sentence"),
            "and the text past the budget must not still be reported as drawn — the summary \
             and the pixels are the same string now.\n{}",
            frame.describe()
        );
        assert!(
            agent.contains("no credentials for provider"),
            "the beginning still reads, or the budget ate the sentence instead of cutting it.\n{}",
            frame.describe()
        );
    }

    // ---- ADR-022: the painter path carries names into the AccessKit tree ------------
    //
    // Every assertion below reads `FullOutput::platform_output.accesskit_update` — the
    // `accesskit::TreeUpdate` egui hands to the platform adapter, read back through the
    // **real** draw path (`IdeApp::paint`). Nothing here reads a field on `IdeApp`, which
    // is the whole point: a region could compute the perfect label, store it, and never
    // attach it, and every field-level test in this repository would still be green.

    /// **The transcript reaches a screen reader**, with a bounded name and a count.
    ///
    /// This is the one that matters. The transcript is the reason ADR-022 exists, and it
    /// was the region with nothing at all in the tree before `name_region`.
    ///
    /// **The name is a count, not the content.** A screen reader that announced every row
    /// on every repaint would be unusable, so the assertion is that the name says *how
    /// many* and that it says so with the number the frame actually drew — a label built
    /// from a different counter would drift from the screen, which is the same lie in a
    /// new place.
    ///
    /// **Negative control.** With `name_region`'s `widget_info` call commented out this
    /// fails with `no accessible name starts with "Agent transcript". the tree held:
    /// ["☰", "◧", "Send", "⑂"]` — the only nodes left are egui's own. The 121 lib tests
    /// that do not concern accessibility still pass, which is the other half of the proof:
    /// the control broke the accessibility claim and nothing else.
    #[test]
    fn the_transcript_publishes_a_bounded_name_with_the_number_of_entries() {
        let (dir, _keep) = scratch_workspace("a11y-transcript");
        let mut h = Harness::new(dir.clone());
        let mut t = Transcript::new(dir);
        for i in 0..8 {
            t.push_user(&format!("question {i}"));
        }
        h.app.seed_transcript(t);

        let label = h.node_named("Agent transcript");
        assert_eq!(
            label, "Agent transcript · 8 entries",
            "the name must count what the pane drew and nothing more."
        );
        assert!(
            !label.contains("question 0"),
            "the transcript text is not in the name; announcing every row on every repaint \
             is unusable.\ngot {label:?}"
        );
    }

    /// An empty transcript says so, and says it truthfully rather than skipping the node.
    ///
    /// "The node is there and its count is zero" is a different answer from "there is no
    /// node", and only the first one tells a reader the pane is empty rather than broken.
    #[test]
    fn an_empty_transcript_still_publishes_its_name() {
        let (dir, _keep) = scratch_workspace("a11y-empty");
        let mut h = Harness::new(dir);
        assert_eq!(
            h.node_named("Agent transcript"),
            "Agent transcript · 0 entries"
        );
    }

    /// **The name is bounded**, however long the conversation is.
    ///
    /// The property the transcript name is chosen for, asserted at a size where a name
    /// built from content would be obvious: 400 entries must still produce a label a
    /// reader hears once.
    #[test]
    fn the_transcript_name_does_not_grow_with_the_conversation() {
        let (dir, _keep) = scratch_workspace("a11y-long");
        let mut h = Harness::new(dir.clone());
        let mut t = Transcript::new(dir);
        for i in 0..400 {
            t.push_user(&format!("a reasonably long question number {i}"));
        }
        h.app.seed_transcript(t);

        let label = h.node_named("Agent transcript");
        assert_eq!(label, "Agent transcript · 400 entries");
        assert!(
            label.chars().count() < 40,
            "a name that scales with the conversation is not a name, it is the transcript: \
             {label:?}"
        );
    }

    /// **The id does not move between frames.**
    ///
    /// A node whose id changed on every repaint would be a node a screen reader could never
    /// keep up with — worse than none, because it is announced and then lost. Two frames
    /// of the same window must produce the same node, and this is the assertion that fails
    /// if a salt is ever derived from a frame counter.
    #[test]
    fn a_region_keeps_one_node_across_frames() {
        let (dir, _keep) = scratch_workspace("a11y-stable");
        let mut h = Harness::new(dir.clone());
        let mut t = Transcript::new(dir);
        t.push_user("hello");
        h.app.seed_transcript(t);

        let first = h.node_named("Agent transcript");
        let second = h.node_named("Agent transcript");
        assert_eq!(
            first, second,
            "the same region named itself differently on the next frame."
        );
    }

    /// **The editor pane names whichever of its three states it is in.**
    ///
    /// A reader who cannot tell a file from a diff under review reads a patch as the
    /// document, and the empty pane is indistinguishable from one that failed to draw.
    #[test]
    fn the_editor_pane_names_the_file_it_is_showing() {
        let (dir, _keep) = scratch_workspace("a11y-editor");
        let mut h = Harness::new(dir);
        // Empty first: the pane is on screen with nothing in it before anything is opened.
        assert_eq!(
            h.node_named("Editor, read-only"),
            "Editor, read-only · no file open"
        );

        h.app.open_file("alpha.rs");
        assert_eq!(
            h.node_named("Editor, read-only"),
            "Editor, read-only · alpha.rs · 1 lines",
            "the name carries the path and the length the header row shows, so the two \
             cannot disagree."
        );
    }

    /// A change under review is named as a change, and not as a file.
    #[test]
    fn the_editor_pane_names_a_change_under_review_as_a_change() {
        let (dir, _keep) = scratch_workspace("a11y-diff");
        let mut h = Harness::new(dir.clone());
        h.app.seed_transcript(changed(&dir, &[("alpha.rs", false)]));
        // The same assignment the "changed" row's click makes on the next frame.
        h.app.review = h.app.transcript.diffs().first().cloned();
        assert!(
            h.app.review.is_some(),
            "the change row must have been recorded; otherwise this test is asserting on \
             an editor that was never opened for review."
        );

        let label = h.node_named("Change review");
        assert!(
            label.starts_with("Change review · alpha.rs"),
            "a review must never be named as a file. got {label:?}"
        );
        assert!(
            !label.contains("Editor, read-only"),
            "the two states share one id, so the name has to say which one this frame is."
        );
    }

    /// **The sidebar panes name themselves**, including the count each already has.
    ///
    /// **One frame per view, because the activity bar shows one at a time.** The first
    /// version of this test read one frame's tree and asserted all three names in it; it
    /// failed with a tree holding `Workspace files · 2 files` and neither of the others,
    /// which is the design working — `workspace_pane` dispatches to whichever view is
    /// selected, and only that one is ever drawn. An assertion that had passed would have
    /// been asserting on a window that does not exist.
    ///
    /// Each view also keeps its own node, which is the half that is easy to lose: all three
    /// are drawn into the same pane `Ui`, so one salt shared by all three would collapse
    /// them into a single node whose name changed with the activity bar.
    #[test]
    fn the_sidebar_panes_name_themselves_with_the_counts_they_already_have() {
        let (dir, _keep) = scratch_workspace("a11y-sidebar");
        let mut h = Harness::new(dir);

        // `scratch_workspace` writes two files, and the walk counts files.
        h.app.sidebar = SidebarView::Files;
        let files = h.node_named("Workspace files");
        assert!(
            files.starts_with("Workspace files · ") && files.ends_with(" files"),
            "the file tree must say how much it found: {files:?}"
        );
        assert!(
            !files.contains("partial"),
            "two files is not a partial list: {files:?}"
        );

        h.app.sidebar = SidebarView::Tasks;
        assert_eq!(h.node_named("Subagents"), "Subagents · 0 tasks");

        h.app.sidebar = SidebarView::Sessions;
        assert_eq!(h.node_named("Sessions"), "Sessions · no agent attached");

        // And each one is a node of its own, rather than the same node renamed.
        for (view, prefix) in [
            (SidebarView::Files, "Workspace files"),
            (SidebarView::Tasks, "Subagents"),
            (SidebarView::Sessions, "Sessions"),
        ] {
            h.app.sidebar = view;
            let nodes = h.named_nodes();
            let mine: Vec<_> = nodes
                .iter()
                .filter(|(l, _)| l.starts_with(prefix))
                .collect();
            assert_eq!(
                mine.len(),
                1,
                "{prefix:?} must own exactly one node; the tree held {:#?}",
                nodes.iter().map(|(l, _)| l).collect::<Vec<_>>()
            );
        }
    }

    /// **The claim is bounded: a name and a role, not a description, not a relation.**
    ///
    /// **This test asserted the opposite for a while, and its own doc comment was wrong.**
    /// It used to require `Role::Pane` — "egui 0.36 cannot express ARIA landmarks through
    /// this API at all" — which was a true statement about `WidgetInfo` mistaken for a
    /// statement about egui. `Context::accesskit_node_builder` is public
    /// (`egui-0.36.2/src/context.rs:3684-3690`) and `accesskit::Node::set_role` is public, so
    /// ADR-022's "a name and nothing more" was narrower than the truth (M301).
    ///
    /// **The shape of the mistake is the point.** A test that documents a limitation pins it:
    /// correcting the limitation makes the test fail, which is exactly what happened, and the
    /// tempting response is to widen the assertion instead of reading why. What is asserted
    /// now is the property that was always wanted — *the transcript is reachable by role*, not
    /// *the transcript is a pane* — so it survives the next role change and fails if the role
    /// is dropped.
    ///
    /// What is still bounded is real: `WidgetInfo` has no description and no relations, and
    /// nothing here claims otherwise.
    #[test]
    fn a_named_region_carries_the_role_it_claims() {
        let (dir, _keep) = scratch_workspace("a11y-role");
        let mut h = Harness::new(dir);
        let nodes = h.named_nodes();

        assert!(
            nodes
                .iter()
                .any(|(label, role)| label.starts_with("Agent transcript")
                    && *role == egui::accesskit::Role::Log),
            "the transcript is a Log - the region a reader came for, reachable from the \
             rotor rather than by arrowing through every other pane. tree: {nodes:?}"
        );

        let full = h.a11y_frame();
        let update = full
            .platform_output
            .accesskit_update
            .as_ref()
            .expect("a11y_frame asserts this");
        assert!(
            update.tree.is_some(),
            "the tree update must carry its tree root."
        );
    }

    /// A real agent over the offline provider, for the one state the agent-less window
    /// cannot show.
    ///
    /// **Composition, not a hand-built session.** `resolve_launch` + [`IdeSession::start`]
    /// is the product's own path, so "idle" here is what the status bar would say about a
    /// real session rather than what a fake could be made to say.
    /// **Every sentence this crate hands a screen reader has to read right.**
    ///
    /// Found by dumping the AccessKit tree on 2026-10-04 and *reading* it, which is the
    /// accessibility counterpart of rendering a frame and looking at the screen
    /// (`AGENTS.md` §5, §13). Two defects surfaced that no assertion was looking at, because
    /// every existing test asserted **that a node existed with some name**, never **what the
    /// name said**:
    ///
    /// * the file-tree pane read `Workspace files · 1 files`;
    /// * the tree's filter box was a `TextInput` with an **empty** label — a bare field a
    ///   screen reader could not say anything about, because `WidgetInfo::text_edit` fills
    ///   value and hint and never `label`.
    ///
    /// This pins the wording, not just the presence.
    #[test]
    fn the_accessibility_names_read_correctly_and_the_filter_is_named() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("only.md"), "x").unwrap();
        let (session, _n) = faux_session(dir.path());
        let mut h = Harness::with_session(dir.path().to_path_buf(), session, Vec::new());
        let names = h
            .named_nodes()
            .into_iter()
            .map(|(l, _)| l)
            .collect::<Vec<_>>();

        assert!(
            names.iter().any(|n| n == "Workspace files · 1 file"),
            "the count must be pluralised — `1 files` is what a screen reader announced on \
             2026-10-04. Names now: {names:?}"
        );
        assert!(
            names
                .iter()
                .any(|n| n.starts_with("Filter files in the workspace tree")),
            "the tree filter is a text field and must be nameable; an unnamed `TextInput` \
             announces as a bare box. Names now: {names:?}"
        );
        assert!(
            !names.iter().any(|n| n.contains(" 1 files")),
            "no name may carry a plural bug: {names:?}"
        );
    }

    /// **No node in this window has an unknown role.**
    ///
    /// The dump read on 2026-10-04 carried one, and it was real. egui allocates an AccessKit node for
    /// every **focusable** widget and fills it with `fill_accesskit_node_common`, which sets disabled,
    /// bounds and actions but no role (`egui-0.36.2/src/context.rs:1292-1297`,
    /// `response.rs:908-924`); the node starts as `Default::default()` (`context.rs:605`) and
    /// `Unknown` is `#[default]` (`accesskit-0.24.1/src/lib.rs:63-64`). `Sense::click()` is focusable
    /// (`egui-0.36.2/src/sense.rs:60`), so every drawn file-tree row allocated a node that announced
    /// as nothing at all.
    ///
    /// **The name used to claim two properties and the body checked one.** It said
    /// "unnamed_and_unroled" while filtering only on the role — an independent review caught that,
    /// in a commit whose whole subject is that naming and roleling are *separate* absences, and
    /// where the resize handle had a full name and still announced as unknown. It is now
    /// `no_node_in_the_window_is_unroled`, which is what it asserts. The naming half is covered
    /// by `the_accessibility_names_read_correctly_and_the_filter_is_named`.
    ///
    /// Asserted as a property over the **whole** tree rather than "the row I fixed", because a test
    /// aimed at one node passes with the other still broken — which is exactly what happened here.
    /// A node whose role is `Unknown` is by definition one a screen reader cannot describe, so none
    /// may exist.
    #[test]
    fn no_node_in_the_window_is_unroled() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("only.md"), "x").unwrap();
        let (session, _n) = faux_session(dir.path());
        let mut h = Harness::with_session(dir.path().to_path_buf(), session, Vec::new());

        let update = h
            .a11y_frame()
            .platform_output
            .accesskit_update
            .clone()
            .expect("checked by a11y_frame");
        let unknown: Vec<_> = update
            .nodes
            .iter()
            .filter(|(_, n)| n.role() == egui::accesskit::Role::Unknown)
            .map(|(id, n)| (*id, n.label().map(str::to_string)))
            .collect();

        assert!(
            unknown.is_empty(),
            "every node a screen reader can reach needs a role and a name; these are \
         `Role::Unknown`: {unknown:#?}\nall named nodes: {:?}",
            h.named_nodes()
        );
    }

    /// **The window publishes landmarks, not just names.**
    ///
    /// # The claim this corrects
    ///
    /// `name_region`'s own doc comment said *"Nothing here is an ARIA landmark … egui 0.36 cannot
    /// express either through this API."* **The first half was true and the conclusion was wrong.**
    /// It is true of `WidgetInfo`, which has no role field, but `Context::accesskit_node_builder` is
    /// public (`egui-0.36.2/src/context.rs:3684-3690`) and `accesskit::Node::set_role` is public — so
    /// `name_region` now stamps a role after its own `widget_info` call, because egui's pass
    /// rewrites the role otherwise.
    ///
    /// # Why landmarks are the difference and not a nicety
    ///
    /// Every region in this window already had a **name**. What it did not have was a role, and a
    /// role is what a screen reader's rotor is built from: with every pane reading as an anonymous
    /// container, the fastest route to the transcript was arrow keys through everything else. A
    /// landmark is the mechanism the WAI-ARIA Authoring Practices give for exactly this, and the
    /// transcript — the thing a reader opened the window for — was the one that had none.
    ///
    /// Asserted on the **roles actually published**, not on the call sites: a test that counted
    /// `name_region` calls would pass if every one of them passed the wrong role.
    #[test]
    fn the_window_publishes_landmarks_a_reader_can_rotate_through() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("only.md"), "x").unwrap();
        let (session, _n) = faux_session(dir.path());
        let mut h = Harness::with_session(dir.path().to_path_buf(), session, Vec::new());

        let update = h
            .a11y_frame()
            .platform_output
            .accesskit_update
            .clone()
            .expect("checked by a11y_frame");
        let roles: std::collections::BTreeSet<egui::accesskit::Role> =
            update.nodes.iter().map(|(_, n)| n.role()).collect();

        // The four the window is actually built from, each named for what it is.
        for (role, why) in [
            (egui::accesskit::Role::Log, "the transcript"),
            (egui::accesskit::Role::Navigation, "the activity bar"),
            (egui::accesskit::Role::Banner, "the title bar"),
            (
                egui::accesskit::Role::Status,
                "the status bar and the goal band",
            ),
        ] {
            assert!(
                roles.contains(&role),
                "{why} must publish its landmark role ({role:?}); this tree has {roles:?}"
            );
        }
        // And the control that was `Role::Unknown` with a full name is a splitter now.
        assert!(
            roles.contains(&egui::accesskit::Role::Splitter),
            "the sidebar resize handle is a splitter, not an anonymous box; this tree has {roles:?}"
        );
    }

    fn faux_session(dir: &std::path::Path) -> (crate::session::IdeSession, Vec<String>) {
        let runtime = std::sync::Arc::new(
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .expect("a test runtime"),
        );
        let launch = epsilon_runtime::resolve_launch(&epsilon_runtime::LaunchRequest {
            workspace: dir.to_path_buf(),
            provider: epsilon_runtime::ProviderChoice::Faux,
            ..epsilon_runtime::LaunchRequest::for_workspace(dir)
        })
        .expect("a faux launch always resolves");
        let notices = launch.notices.clone();
        let session = runtime
            .block_on(crate::session::IdeSession::start(
                std::sync::Arc::clone(&runtime),
                launch,
            ))
            .expect("composing an agent over a fresh session");
        (session, notices)
    }

    /// A goal as `draw_goal_band` reads one, so the band's name can be checked against the
    /// same fields the band paints. `GoalSnapshot` has no `Default`, which is why this is a
    /// function and not a struct literal at each use.
    fn goal_view(armed: bool) -> epsilon_contracts::goal::GoalView {
        epsilon_contracts::goal::GoalView {
            goal: epsilon_contracts::goal::GoalSnapshot {
                id: "goal-1".into(),
                revision: 1,
                objective: "ship the thing".into(),
                status: epsilon_contracts::goal::GoalStatus::Active,
                max_rounds: 8,
                rounds_started: 2,
                blocker: None,
                gates: vec![epsilon_contracts::goal::GoalGate::new("cargo test")],
                created_at_ms: 0,
                updated_at_ms: 0,
            },
            armed,
        }
    }

    /// **The title bar names the workspace and the figures, out of the text it drew.**
    ///
    /// Two of the four things `layout.rs:133` promises this bar are the workspace name and
    /// the model; the other two — context and cost — are the figures that were added
    /// later, and they are the reason a reader opens the window at all. The name is built
    /// from the drawn line rather than from a second reading of the same state, so the two
    /// cannot drift, and the test asserts exactly that: it compares the name against what
    /// the frame recorded.
    #[test]
    fn the_title_bar_names_the_workspace_and_the_figures_it_drew() {
        let (dir, _keep) = scratch_workspace("a11y-title");
        let mut h = Harness::new(dir);

        // No agent, so no figures: the bar names the workspace and says there is no agent
        // rather than inventing a cost.
        let frame = h.paint();
        let drawn = frame.region("title").join("");
        assert!(!drawn.is_empty(), "{}\n", frame.describe());
        assert_eq!(
            h.node_named("Title bar"),
            format!("Title bar · {drawn}"),
            "the name must be the line the bar drew, not a second spelling of it."
        );

        // **The same bar with figures on it.** `metrics` is the field `IdeApp::figures`
        // reads, set here rather than through an agent so the assertion stays about the
        // name.
        h.app.metrics = Some(epsilon_tui::ui::metrics::Metrics {
            context_used: Some(12_000),
            context_window: Some(200_000),
            tokens_in: 1_234,
            tokens_out: 56,
            cost_micro_usd: Some(4_200),
            ..Default::default()
        });
        let frame = h.paint();
        let figures = frame.region("figures").join("");
        assert!(
            !figures.is_empty(),
            "the figures were not drawn, so this test would be asserting on a name nothing \
             put there.\n{}",
            frame.describe()
        );
        assert!(
            h.node_named("Title bar")
                .ends_with(&format!(" · {figures}")),
            "the name must carry the figures the bar drew, character for character."
        );
    }

    /// **The status bar names the state, and the state is the only part that changes.**
    ///
    /// `status_line` also carries the refusal text — a tool's own sentence, unbounded — and
    /// the layout tier, which is a word about the window's size. Neither belongs in a name
    /// that is announced on every repaint; `run_state` is the bounded part, and it is the
    /// part a reader needs: *is something running*.
    #[test]
    fn the_status_bar_names_the_state_it_is_in() {
        let (dir, _keep) = scratch_workspace("a11y-status");
        let mut h = Harness::new(dir.clone());
        assert_eq!(
            h.node_named("Status bar"),
            "Status bar · no agent attached",
            "the agent-less window must say that, not 'idle' — a name that reports a state \
             the window is not in is worse than no name."
        );

        // **A different state is a different name.** Same window, a session attached and
        // nothing in flight: `idle` where the first frame said the agent was missing.
        let (session, notices) = faux_session(&dir);
        let mut h = Harness::with_session(dir, session, notices);
        assert_eq!(
            h.node_named("Status bar"),
            "Status bar · idle",
            "a real session with no turn in flight is idle."
        );
    }

    /// **The activity bar names the strip, and its buttons are renamed rather than joined.**
    ///
    /// Two different problems, two different answers. The strip is painter-drawn and had no
    /// node at all. The three buttons are real widgets, so egui had already put them in the
    /// tree — **named with the glyph they draw**, because `Button` names itself with its
    /// text (`egui-0.36.2/src/widgets/button.rs:401`) and `on_hover_text` never reaches a
    /// node (egui 0.36 contains no `set_description` or `set_tooltip` call at all, although
    /// `accesskit::Node` has both). Adding a node beside them would announce the control
    /// twice and hide the first one.
    #[test]
    fn the_activity_bar_names_the_strip_and_renames_the_buttons_egui_already_had() {
        let (dir, _keep) = scratch_workspace("a11y-activity");
        let mut h = Harness::new(dir);
        assert_eq!(h.node_named("Activity bar"), "Activity bar · showing files");

        let nodes = h.named_nodes();
        for view in SidebarView::ALL {
            let mine: Vec<_> = nodes.iter().filter(|(l, _)| l == view.label()).collect();
            assert_eq!(
                mine.len(),
                1,
                "{:?} must own exactly one node, carrying the name a reader would say for \
                 it.\nthe tree held: {:#?}",
                view.label(),
                nodes.iter().map(|(l, _)| l).collect::<Vec<_>>()
            );
        }
        assert!(
            !nodes.iter().any(|(l, _)| l == "☰" || l == "◧" || l == "⑂"),
            "a glyph is a picture of a view. It is still what is drawn; it must not be what \
             the node is called.\nthe tree held: {nodes:?}"
        );

        // The strip answers a question none of the buttons answers: which view is showing.
        h.app.sidebar = SidebarView::Tasks;
        assert_eq!(
            h.node_named("Activity bar"),
            "Activity bar · showing subagents"
        );
    }

    /// **The goal band names itself and says whether it is armed.**
    ///
    /// Armed is the half of the state a reader cannot infer from the rest: an active goal
    /// that is not continuing looks exactly like one that has finished, which is why the
    /// band draws the words "not armed — it will not continue" on it.
    #[test]
    fn the_goal_band_says_whether_it_is_armed() {
        let (dir, _keep) = scratch_workspace("a11y-goal");
        let mut h = Harness::new(dir.clone());

        // **No goal, no band, no node.** `draw_goal_band` returns before it draws anything
        // when the session has no goal, and a node for a region that is not on screen is a
        // region a reader cannot find.
        let nodes = h.named_nodes();
        assert!(
            !nodes.iter().any(|(l, _)| l.starts_with("Goal band")),
            "a session with no goal must publish no band.\nthe tree held: {nodes:?}"
        );

        h.app.goal = Some(goal_view(false));
        let label = h.node_named("Goal band");
        assert_eq!(
            label, "Goal band · active · round 2/8 · 1 gate(s) · not armed",
            "the name is built from the same fields the band paints: phase, round budget and \
             gates, plus the one word it cannot leave out."
        );
        assert!(
            !label.contains("ship the thing"),
            "the objective is the reader's own text and can be any length; a name is \
             announced on every repaint.\ngot {label:?}"
        );

        h.app.goal.as_mut().expect("seeded above").armed = true;
        assert_eq!(
            h.node_named("Goal band"),
            "Goal band · active · round 2/8 · 1 gate(s) · armed",
            "arming the goal is the state change a reader must be able to hear."
        );
    }

    /// **Every palette row is reachable by name, and the search box is still unnamed.**
    ///
    /// The rows are drawn with `Painter::text` on rectangles, so before this they were a
    /// picture of a list of things that can be opened. The second half of the test is the
    /// rule that keeps it from becoming two pictures: egui **already** emits a node for the
    /// palette's search `TextEdit`, and it stays exactly as egui built it — unnamed, with
    /// its hint as the placeholder. One node per region; the reader hears the palette once.
    #[test]
    fn the_palette_names_every_row_it_offers() {
        let (dir, _keep) = scratch_workspace("a11y-palette");
        let mut h = Harness::new(dir);
        h.chord(egui::Modifiers::CTRL, egui::Key::P);

        // A file row and a command row, told apart by the same hint the row draws.
        assert_eq!(
            h.node_named("Palette row · alpha.rs"),
            "Palette row · alpha.rs · file"
        );
        assert_eq!(
            h.node_named("Palette row · goal: pause"),
            "Palette row · goal: pause · goal"
        );

        let search = h
            .node_for(palette_input_id())
            .expect("egui builds a node for every TextEdit, the palette's search box included");
        assert_eq!(
            search.label(),
            None,
            "the search box already had a node. Naming it a second time is how a region ends \
             up announced twice."
        );
        assert_eq!(
            search.placeholder(),
            Some("files and commands"),
            "the hint is egui's placeholder and must survive: it is what the field says about \
             itself."
        );
    }

    /// **The composer is egui's node, with a name on it and its value left alone.**
    ///
    /// `WidgetInfo::text_edit` (`egui-0.36.2/src/data/output.rs:697`) fills the value and
    /// the hint and **never** the label, so the composer's node reached every screen reader
    /// as an unlabelled `MultilineTextInput`. This reads the node by its own id, which is
    /// the only way to say "this node, and not another one with the same name".
    #[test]
    fn the_composer_is_egui_s_own_node_with_a_name_on_it() {
        let (dir, _keep) = scratch_workspace("a11y-composer");
        let mut h = Harness::new(dir);

        let empty = h.composer_node();
        assert_eq!(
            empty.role(),
            egui::accesskit::Role::MultilineTextInput,
            "egui maps a multiline `TextEdit` to `MultilineTextInput` \
             (egui-0.36.2/src/widgets/text_edit/builder.rs:1001); anything else means the \
             mapping changed."
        );
        assert_eq!(empty.label(), Some("Composer"));
        assert_eq!(
            empty.value(),
            Some(""),
            "the value is egui's and a name must not cost the field its text."
        );

        // **The name is stable and the value is not** — the composer reports what is in it,
        // and that is what changes while the reader types.
        h.type_text("hello");
        let typed = h.composer_node();
        assert_eq!(typed.label(), Some("Composer"));
        assert_eq!(typed.value(), Some("hello"));

        let named = h.named_nodes();
        assert_eq!(
            named.iter().filter(|(l, _)| l == "Composer").count(),
            1,
            "one node per region: two would announce the composer twice.\nthe tree held: \
             {named:?}"
        );
    }

    /// **A "file changed" row is a named link, not an anonymous click target.**
    ///
    /// This row is the one control in the transcript whose entire purpose is to open
    /// something, and it is the only row that allocates a response for its click
    /// (`Sense::click()`, `app.rs:4609`). Being focusable meant egui built a node for it and
    /// set **no role** on it — the exact shape the file tree had at M301, so a reader could
    /// focus the row and be told nothing.
    ///
    /// Asserted on both fields because they are separately absent-able: a node can carry
    /// the words and still announce as an unknown box, which is what this row did until the
    /// role was added — the name alone would have left it half-fixed.
    #[test]
    fn a_file_changed_row_is_a_named_link_for_its_click_target() {
        let (dir, _keep) = scratch_workspace("a11y-change");
        let mut h = Harness::new(dir.clone());
        let mut t = Transcript::new(dir);
        t.push_user("fix the build");
        t.push_change(&crate::diff::Change {
            path: "src/main.rs".into(),
            lines: vec![crate::diff::DiffLine::Added("let x = 1;".into())],
            created: false,
            coarse: false,
        });
        h.app.seed_transcript(t);

        let nodes = h.named_nodes();
        let change: Vec<_> = nodes
            .iter()
            .filter(|(l, _)| l.starts_with("Open the change · "))
            .collect();
        assert_eq!(
            change.len(),
            1,
            "one click target, one node.\nthe tree held: {nodes:?}"
        );
        assert_eq!(
            change[0].0,
            // The row's own text, which is `path · +n −m`. Asserted verbatim rather than
            // with a prefix match: the counts are what tell a reader whether activating
            // this row is worth it, so a test that accepted any suffix would not notice
            // them going missing.
            "Open the change · src/main.rs · +1 −0",
            "the name must say what activating this row does, and on what"
        );
        assert_eq!(
            change[0].1,
            egui::accesskit::Role::Link,
            "a row that opens a file is a link; naming it without a role leaves a reader \
             with an unknown box that carries words"
        );
    }

    // ======================================================================================
    // Transcript rows: the rows on screen are named, and the rows off screen are absent
    // ======================================================================================

    /// **Every row on screen is in the tree with a name, and two rows are told apart.**
    ///
    /// This is ADR-022's last open item. The transcript was named and counted while its
    /// rows were not, so a screen reader could find the conversation and not read it.
    ///
    /// **The node named is egui's own, and that is asserted too.** `Label::ui` already puts
    /// a node in the tree for every label it draws (`egui-0.36.2/src/widgets/label.rs:282`)
    /// — unnamed, and carrying the row's own text as its *value*. A change that published a
    /// second node beside it would satisfy every name assertion here and announce the
    /// conversation twice, so the third half of this test counts the nodes carrying the
    /// row's text and requires exactly one.
    #[test]
    fn a_visible_transcript_row_is_named_and_it_is_egui_s_own_node() {
        let (dir, _keep) = scratch_workspace("a11y-row");
        let mut h = Harness::new(dir.clone());
        let mut t = Transcript::new(dir);
        t.push_user("fix the build");
        t.push_user("and run the tests");
        t.push_notice("provider: faux");
        h.app.seed_transcript(t);

        let nodes = h.named_nodes();
        let mut mine: Vec<&str> = nodes
            .iter()
            .filter(|(l, _)| l.starts_with("Transcript row · "))
            .map(|(l, _)| l.as_str())
            .collect();
        // Sorted, because `TreeUpdate::nodes` is a `HashMap`'s iteration order
        // (`Context::end_pass`, `egui-0.36.2/src/context.rs:2699-2705`) and this is about
        // which rows are named, not about the order the tree happens to walk them in.
        mine.sort_unstable();
        assert_eq!(
            mine,
            vec![
                "Transcript row · Notice · provider: faux",
                "Transcript row · You said · and run the tests",
                "Transcript row · You said · fix the build",
            ],
            "a name must say what kind of row this is and enough of it to be recognisable, and \
             two different rows must not share one.\nthe tree held: {nodes:?}"
        );

        let full = h.a11y_frame();
        let update = full
            .platform_output
            .accesskit_update
            .as_ref()
            .expect("a11y_frame asserts this");
        let carrying: Vec<_> = update
            .nodes
            .iter()
            .filter(|(_, n)| n.value().is_some_and(|v| v.contains("fix the build")))
            .collect();
        // **Exactly one *named* node for the row.** A second named node for one row is the
        // failure `name_node` exists to prevent: the reader hears the row twice and the
        // first node's existence is hidden behind the second.
        assert_eq!(
            carrying.iter().filter(|(_, n)| n.label().is_some()).count(),
            1,
            "one name per row.\nthe nodes carrying this row: {:#?}",
            carrying
                .iter()
                .map(|(_, n)| (n.role(), n.label(), n.value()))
                .collect::<Vec<_>>()
        );
        // **The other node holding this text is egui's selection run, and it is not a
        // duplicate.** `selectable_labels` is on, so `Label::ui` runs
        // `LabelSelectionState::label_text_selection`
        // (`egui-0.36.2/src/widgets/label.rs:309-317`), which builds a `Role::TextRun` per
        // galley row so the text can be selected. It carried this row's text before this
        // change and it is a different thing — a run of selectable text, whose content *is*
        // its value — so naming it too would announce the row twice. Pinned rather than
        // assumed: a change that made it a second `Role::Label` would be the bug.
        assert!(
            carrying
                .iter()
                .filter(|(_, n)| n.label().is_none())
                .all(|(_, n)| n.role() == egui::accesskit::Role::TextRun),
            "the unnamed nodes holding this row's text must all be egui's selection runs.\n\
             {:#?}",
            carrying
                .iter()
                .map(|(_, n)| (n.role(), n.label(), n.value()))
                .collect::<Vec<_>>()
        );
        let row = &carrying
            .iter()
            .find(|(_, n)| n.label().is_some())
            .expect("asserted above")
            .1;
        assert_eq!(
            row.role(),
            egui::accesskit::Role::Label,
            "egui builds this node (Label::ui); naming it must not change what it is."
        );
        assert_eq!(
            row.label(),
            Some("Transcript row · You said · fix the build")
        );
    }

    /// **A row off screen is absent — and this is the test that keeps virtualisation honest.**
    ///
    /// Naming rows is one AccessKit node per row per frame, which is why it was blocked
    /// until M285 virtualised the transcript. The cheap way to do it and the correct way to
    /// do it differ by exactly one thing — *where* the naming call sits — and the symptom of
    /// getting it wrong is invisible in a test that only looks for a name that should be
    /// there. So this asserts the other direction: a transcript of 200 rows must **not**
    /// publish 200 names.
    #[test]
    fn a_transcript_row_off_screen_is_not_in_the_tree() {
        const ROWS: usize = 200;
        let (dir, _keep) = scratch_workspace("a11y-row-offscreen");
        let mut h = Harness::new(dir.clone());
        let mut t = Transcript::new(dir);
        for i in 0..ROWS {
            t.push_user(&format!("question {i}"));
        }
        h.app.seed_transcript(t);

        // Two frames, for the reason the scrolling test gives: `stick_to_bottom` is
        // applied at the end of a frame, so the first is laid out from offset zero.
        h.paint();
        let frame = h.paint();
        assert!(
            frame
                .region("agent")
                .iter()
                .any(|t| t.contains(&format!("question {}", ROWS - 1))),
            "the bottom of the transcript is not what the frame shows, so 'the first row is \
             absent' would be true of a window that drew nothing.\n{}",
            frame.describe()
        );

        let named = h.named_nodes();
        let rows: Vec<&str> = named
            .iter()
            .filter(|(l, _)| l.starts_with("Transcript row · "))
            .map(|(l, _)| l.as_str())
            .collect();
        // No `question 0` in a string of `question 199`: the digit after the space is
        // never a zero for a multi-digit row, so this cannot match by accident.
        assert!(
            !rows.iter().any(|l| l.contains("question 0")),
            "row 0 is off screen and must not be named. A node for a row no reader can reach \
             is a node the screen reader has to walk past on every repaint, and it is one \
             node per row per frame.\nthe tree held {rows:?}"
        );
        assert!(
            rows.iter()
                .any(|l| l.contains(&format!("question {}", ROWS - 1))),
            "the last row is on screen, so it must be named. Without this half the absence \
             above would also be satisfied by a transcript that names nothing at all."
        );
        assert!(
            rows.len() < ROWS / 4,
            "a {ROWS}-row transcript published {} row names. Naming every row rather than \
             every *drawn* row puts one node per row back into every frame, which is the \
             cost M285 removed and the reason this was blocked until then.\nthe tree held \
             {rows:?}",
            rows.len()
        );
    }

    /// **The fixture ADR-022 names — 800 *rich* rows — with both numbers measured here.**
    ///
    /// ADR-022 records the cost of naming every drawn row as *"800 rich rows → 44 lines
    /// drawn → 53 named nodes … and it is in a test, not in a comment."* **It was not in a
    /// test.** The bounding tests used 200- and 400-row transcripts of *one-line user
    /// rows*, which is a different fixture twice over: a user row never wraps, so nothing
    /// exercised the multi-line layout the claim is about, and it names as
    /// `Transcript row · You said · …` rather than the assistant role.
    ///
    /// **Two of the three numbers are also inconsistent with each other under either
    /// reading.** `region("agent")` counts *wrapped lines*; a node is built per *row*, so a
    /// transcript of multi-line rows must name **fewer** nodes than it draws lines. "44
    /// lines drawn → 53 named nodes" has more of the second than the first, which no build
    /// of this window produces. So the claim was not merely unpinned — it was not true.
    ///
    /// It is replaced by a measurement rather than deleted, and the property asserted is
    /// the one the sentence was reaching for: **the node count tracks the rows on screen,
    /// not the length of the conversation.** That is why the fixture is measured at two
    /// sizes. Quadrupling the transcript from 200 rows to 800 must not quadruple the
    /// nodes, and a frame cost that grew with the conversation is exactly what M285
    /// removed.
    #[test]
    fn eight_hundred_rich_rows_pay_only_for_the_rows_that_are_drawn() {
        /// One paragraph per row, distinct per index.
        ///
        /// Through `apply` rather than `push_user` because a *rich* row is the one that
        /// wraps: this is the fixture whose drawn-line count is several times its row
        /// count, and it names as `Agent said` rather than `You said`.
        fn conversation(rows: usize, workspace: std::path::PathBuf) -> Transcript {
            let mut t = Transcript::new(workspace);
            for i in 0..rows {
                t.apply(&epsilon_contracts::agent_event::AgentEvent::MessageEnd {
                    message: Box::new(epsilon_contracts::message::Message::Assistant(
                        epsilon_contracts::message::AssistantMessage {
                            id: format!("rich-{i}"),
                            timestamp_ms: 0,
                            provider: "faux".into(),
                            model_id: "faux-1".into(),
                            content: vec![epsilon_contracts::content::ContentBlock::text(format!(
                                "paragraph {i} of a long conversation, written so that it \
                                     wraps across several drawn lines rather than fitting on \
                                     one, which is what makes the drawn-line count and the \
                                     row count different numbers and what this measurement \
                                     is actually about."
                            ))],
                            stop_reason: epsilon_contracts::message::StopReason::EndTurn,
                            error_message: None,
                            usage: Default::default(),
                        },
                    )),
                });
            }
            t
        }

        /// `(lines drawn, row names published)` for a transcript of `rows` rich rows.
        fn measure(rows: usize, tag: &str) -> (usize, Vec<String>) {
            let (dir, _keep) = scratch_workspace(tag);
            let mut h = Harness::new(dir.clone());
            h.app.seed_transcript(conversation(rows, dir));
            // Two frames: `stick_to_bottom` is applied at the end of a pass, so the first
            // is laid out from offset zero. The reason the sibling test gives.
            h.paint();
            let f = h.paint();
            // The tail must be on screen, or every assertion below is satisfied by a
            // window that drew nothing at all.
            assert!(
                f.region("agent")
                    .iter()
                    .any(|l| l.contains(&format!("paragraph {}", rows - 1))),
                "{rows} rich rows: the bottom of the transcript is not what the frame shows, \
                 so the bounds below would be vacuous.\n{}",
                f.describe()
            );
            let lines = f.region("agent").len();
            let names: Vec<String> = h
                .named_nodes()
                .into_iter()
                .map(|(l, _)| l)
                .filter(|l| l.starts_with("Transcript row · "))
                .collect();
            (lines, names)
        }

        let (lines_200, names_200) = measure(200, "rich-rows-200");
        let (lines_800, names_800) = measure(800, "rich-rows-800");

        // **The claim, and the only half that needed pinning: the cost tracks the screen,
        // not the transcript.** Four times the conversation, the same work — measured at
        // 15 row names either way on this machine. A frame cost that grew with the
        // conversation is exactly what M285 removed.
        assert!(
            names_800.len() < names_200.len() * 2,
            "quadrupling the transcript from 200 to 800 rich rows took the published names \
             from {} to {}. Naming every row rather than every *drawn* row makes this frame \
             cost grow with the conversation, which is the cost M285 removed.",
            names_200.len(),
            names_800.len()
        );
        assert!(
            names_800.len() < 800 / 4,
            "an 800-row transcript published {} row names; one per row is one node per row \
             per frame.",
            names_800.len()
        );

        // **The off-screen half is genuinely absent, by name.**
        //
        // No `paragraph 0` in a string of `paragraph 799`: the digit after the space is
        // never a zero for a multi-digit row, so this cannot match by accident. It holds
        // of the *drawn* half too, not only of the tree — a window that drew all 800 and
        // named a few would satisfy the bound above and fail here.
        assert!(
            !names_800.iter().any(|l| l.contains("paragraph 0 ")),
            "the first paragraph is off screen and must not be named.\nthe tree held {:?}",
            names_800
        );

        // Both counts are returned rather than discarded, because **the recorded line
        // count is not the number the old ADR sentence was reaching for.** Measured here:
        // 17 recorded lines against 15 names at *both* sizes, and the two extra lines are
        // pane chrome (`Agent`, and the greeting) rather than wrapped prose. See the note
        // on `FrameSummary` below.
        assert!(
            lines_200 > names_200.len() && lines_800 > names_800.len(),
            "the agent region carries chrome beyond its rows, so its recorded lines exceed \
             its named rows: {lines_200}/{}/{lines_800}/{}",
            names_200.len(),
            names_800.len()
        );
    }

    /// **`FrameSummary::region` counts recorded *blocks*, not painted lines.**
    ///
    /// A sibling test above justifies counting this region as "the lines are the rows".
    /// **That is true for one-line user rows and false for the assistant rows that make a
    /// real conversation long.** `record_block` (`app.rs:3952-3962`) is called once per
    /// **markdown block**, and a paragraph that wraps over six painted lines contributes
    /// **one** recorded line and one named node. The code block is the arm that makes
    /// "per block" the rule rather than "one line per answer": it records **one line per
    /// line of code** (`app.rs:3836-3845`).
    ///
    /// It is recorded here because it is why the ADR-022 figure "800 rich rows → 44 lines
    /// drawn → 53 named nodes" could not have been reproduced from this tree even by
    /// someone who meant to: there is no counter in the product that reports painted lines,
    /// and the count that does exist moves by block. The claim was measured against a
    /// number the window does not produce.
    ///
    /// **The fixture is most of this test, and it was wrong the first time.** Four
    /// adjacent `ContentBlock::text` values parse as **one** paragraph — markdown has no
    /// blank line between them — so the first version of this test recorded a single
    /// concatenated line and failed with `left: 1, right: 2`. The frame dump is what showed
    /// it: four paragraphs ran together into one entry with no separator at all. **Blocks
    /// are separated by a blank line, not by being different content blocks**, and the last
    /// assertion below is the one the wrong fixture would have failed.
    #[test]
    fn the_agent_region_records_one_line_per_markdown_block_not_per_painted_line() {
        let (dir, _keep) = scratch_workspace("blocks-not-lines");
        let mut h = Harness::new(dir.clone());
        let mut t = Transcript::new(dir);
        t.apply(&epsilon_contracts::agent_event::AgentEvent::MessageEnd {
            message: Box::new(epsilon_contracts::message::Message::Assistant(
                epsilon_contracts::message::AssistantMessage {
                    id: "blocks-1".into(),
                    timestamp_ms: 0,
                    provider: "faux".into(),
                    model_id: "faux-1".into(),
                    // Blank lines between the blocks, which is what separates them. Every
                    // paragraph here is long enough to wrap, so recorded lines and painted
                    // lines part company.
                    content: vec![epsilon_contracts::content::ContentBlock::text(
                        "first paragraph, long enough that it wraps past the width of the \
                         pane and onto a second painted line\n\nsecond paragraph, which the \
                         first must not swallow\n\n- a list item\n\n\
                         ```rust\nlet x = 1;\nlet y = 2;\n```",
                    )],
                    stop_reason: epsilon_contracts::message::StopReason::EndTurn,
                    error_message: None,
                    usage: Default::default(),
                },
            )),
        });
        h.app.seed_transcript(t);

        h.paint();
        let f = h.paint();
        let region = f.region("agent");

        // **Each block is its own recorded entry.**
        for expected in ["first paragraph", "second paragraph", "a list item"] {
            assert!(
                region.iter().any(|l| l.contains(expected)),
                "no recorded line carries {expected:?}.\n{}",
                f.describe()
            );
        }
        // **And a code block records one line per line of code** — the arm that makes
        // "per block" the rule rather than "one entry per answer".
        for code in ["let x = 1;", "let y = 2;"] {
            assert!(
                region.iter().any(|l| l.contains(code)),
                "{code:?} is missing, so a code block is recorded as one entry rather than \
                 one per line.\n{}",
                f.describe()
            );
        }
        // **The assertion the wrong fixture would have failed**, stated directly: no entry
        // carries two blocks. One line holding both paragraphs is exactly what four
        // adjacent content blocks produced.
        assert!(
            !region
                .iter()
                .any(|l| l.contains("first paragraph") && l.contains("second paragraph")),
            "two blocks were recorded as one entry, so this region is counting something \
             other than blocks.\n{}",
            f.describe()
        );
    }

    /// **The chrome lines are budgeted like everything else in the window.**
    ///
    /// These were the **only two long strings the window drew without a budget**: file rows,
    /// task lines, sessions, tab labels, palette rows and transcript rows all go through
    /// [`fit`], and the chrome draws on the root `Ui` whose clip rect is the whole content
    /// rect (`app.rs:2458`), so nothing clipped them either. The title line therefore ran
    /// into the figures on its right, whose reserve is a fixed 48 pt — and at 200% the same
    /// line is twice as wide, so it collided at a size where it had not collided at all.
    ///
    /// **At 200%, which is where the defect is reachable.** A 1440-point window at zoom 1
    /// has room for the longest workspace name a scratch directory can produce, so a test
    /// pinned there would pass against the unbudgeted code. The zoom is what makes the same
    /// string twice as wide, and `at_zoom` keeps the content rect identical so this compares
    /// two font sizes rather than two layouts.
    ///
    /// **Both halves matter.** The line must be *marked* as cut (`fit` ends with `…`; a
    /// silently shortened value is the failure `fit` exists to prevent), and it must be
    /// *shorter than the zoomed-out line*, which is what "budgeted by the font size it is
    /// drawn at" means rather than by a fixed character count.
    ///
    /// **What is not pinned here, stated rather than left for a reader to assume:** the
    /// status bar's half of the same fix. Its line is arbitrary length in the product —
    /// `status_line` carries a tool's own refusal sentence — but `paint` clears
    /// `self.refusal` on a window with no session (`app.rs:1898-1899`), so driving a long
    /// one from a frame test needs a composed agent rather than a fixture. The status bar
    /// now takes the identical [`fit`] call against the identical `AVG_ADVANCE`
    /// approximation, and that the two agree is what the title half above demonstrates.
    #[test]
    fn the_title_line_is_budgeted_and_says_when_it_was_cut() {
        let long = "a-workspace-directory-name-that-no-window-is-wide-enough-to-show-in-full";
        let (dir, _keep) = scratch_workspace(long);
        let mut h = Harness::new(dir);

        let frame = h.paint();
        let title = frame.region("title").join("");
        assert!(
            !title.is_empty(),
            "the title bar drew nothing, so this asserts on an empty string.\n{}",
            frame.describe()
        );
        // **At 100% it fits, and must not be cut anyway.** A budget that is too aggressive
        // is the same defect as one that is missing: a reader who believes a truncated
        // workspace name is the whole one is misled about where they are.
        assert!(
            !title.ends_with('…'),
            "a 1440-point bar has room for this name and cut it anyway: {title:?}"
        );

        h.at_zoom(2.0);
        let zoomed = h.paint();
        let title2 = zoomed.region("title").join("");
        assert!(
            title2.ends_with('…'),
            "at 200% the same line is twice as wide and must be shortened **and marked**: \
             {title2:?}\n{}",
            zoomed.describe()
        );
        assert!(
            title2.chars().count() < title.chars().count(),
            "doubling the font size did not shorten the line, so the budget is not being \
             measured in points: {title2:?} against {title:?}"
        );
        // **The cut is honest about what it cut.** The workspace name is at the front, so a
        // budgeted line keeps it and loses the end — never the part that identifies the
        // workspace.
        assert!(
            title2.starts_with("epsilon-tabs-a-workspace"),
            "the budget cut the front of the line rather than the end: {title2:?}"
        );
    }

    /// **The status bar's dropped-events segment appears, because it can (M312).**
    ///
    /// It used to be drawn from `self.transcript.dropped()`, whose only writer —
    /// `Transcript::note_lag` — **no production code called**. The number on screen was therefore
    /// always zero and the segment could never appear: a control that does not do what it names,
    /// which `AGENTS.md` §2.7 rules out and which a read-only sweep found by grepping callers.
    ///
    /// **Asserted on the frame, and driven through the whole path**: the count is written on the
    /// session (where the bridge writes it), read by `pump`, carried in the status line, and only
    /// then asserted. Setting `IdeApp::dropped_events` directly would have been a test of the copy
    /// rather than of the path — and `pump` overwrites that field every frame, so it would have
    /// failed for a reason unrelated to the defect.
    #[test]
    fn the_status_bar_says_how_many_events_were_dropped_when_there_were_some() {
        let (dir, _keep) = scratch_workspace("dropped-events");
        let (session, notices) = faux_session(&dir);
        let mut h = Harness::with_session(dir, session, notices);

        let quiet = h.paint().region("status").join("");
        assert!(
            !quiet.contains("event(s) dropped"),
            "a window that has dropped nothing must not say it has: {quiet:?}"
        );

        h.app
            .session
            .as_ref()
            .expect("a session")
            .note_dropped_events_for_tests(5);
        let loud = h.paint().region("status").join("");
        assert!(
            loud.contains("5 event(s) dropped"),
            "the count the bridge recorded must reach the reader. The status bar drew: {loud:?}"
        );
        // **The other half of the run state is still there.** A status line that replaced the
        // bar with a number would satisfy the assertion above and be worse than useless.
        assert!(
            loud.contains("idle"),
            "the segment was added without displacing the state beside it: {loud:?}"
        );
    }

    /// **A name identifies a row; it is not the row.**
    ///
    /// Both halves matter and they are opposites. A name that carried the whole line would
    /// have the reader hear it twice — once as the name and once as the text the node
    /// already carries as its value — and a name of `…` would identify nothing. So the
    /// bound is asserted, and so is that bounding the name **cost the row nothing**.
    ///
    /// Two lines, because the second is a pasted prompt and a name is read as one
    /// utterance: a newline in a name is an announcement the reader did not ask for.
    #[test]
    fn a_long_transcript_row_is_named_in_a_bounded_number_of_characters() {
        let (dir, _keep) = scratch_workspace("a11y-row-bounded");
        let mut h = Harness::new(dir.clone());
        let prompt = format!("first line\n{}", "a very long continuation ".repeat(40));
        let mut t = Transcript::new(dir);
        t.push_user(&prompt);
        h.app.seed_transcript(t);

        let label = h.node_named("Transcript row · You said");
        assert!(
            label.chars().count() <= "Transcript row · You said · ".chars().count() + NAME_EXCERPT,
            "the name must be bounded: the row it identifies is {} characters long and every \
             character of it would be announced again on every repaint.\ngot {label:?}",
            prompt.len()
        );
        assert!(
            label.ends_with('…'),
            "a truncated name must say it was truncated, or a reader cannot tell a short row \
             from a clipped one.\ngot {label:?}"
        );
        assert!(
            !label.contains('\n'),
            "a name is read as one utterance and a pasted prompt is several lines.\ngot \
             {label:?}"
        );

        // **Nothing was lost.** The whole line is still on the node, as the value egui put
        // there; the name is an addition, not a replacement.
        let full = h.a11y_frame();
        let update = full
            .platform_output
            .accesskit_update
            .as_ref()
            .expect("a11y_frame asserts this");
        let value = update
            .nodes
            .iter()
            .find(|(_, n)| n.label() == Some(label.as_str()))
            .and_then(|(_, n)| n.value())
            .expect("the node that carries this name");
        assert!(
            value.contains("a very long continuation"),
            "the row's own text must survive on the node: bounding the name is not permission \
             to throw the row away.\nvalue: {value:?}"
        );
    }

    /// **An empty transcript names its empty state, and does not pretend to be content.**
    ///
    /// The empty state is a `Label` like any other, so egui has already built it a node —
    /// unnamed. And it is **not a row**: a name of `Transcript row · no agent attached`
    /// would tell a reader the conversation contains a row that says it has none.
    #[test]
    fn an_empty_transcript_names_its_empty_state_rather_than_a_row() {
        let (dir, _keep) = scratch_workspace("a11y-row-empty");
        let mut h = Harness::new(dir);

        let named = h.named_nodes();
        assert!(
            named
                .iter()
                .any(|(l, _)| l == "Transcript empty · no agent attached"),
            "the window with no agent says so on screen; its node must say it too.\nthe tree \
             held {named:?}"
        );
        assert!(
            !named
                .iter()
                .any(|(l, _)| l.starts_with("Transcript row · ")),
            "a transcript with nothing in it has no rows to name, and a node that claims one \
             is a row a reader cannot find.\nthe tree held {named:?}"
        );
    }

    // ======================================================================================
    // Virtualisation: the transcript draws the rows that are on screen and no others
    // ======================================================================================

    /// A transcript longer than the pane lays out **strictly fewer** rows than it has.
    ///
    /// This is the property, and it is the one M266 said the transcript did not have: the
    /// transcript built a text widget for every block of every row, on screen or not, and
    /// that is **95% of a 104 ms frame** (`docs/milestones/M275-phase-breakdown.md`).
    ///
    /// The count is [`FrameSummary::region`], not a counter this change added — the
    /// transcript records every line it paints, so the lines are the rows. Asserting on a
    /// number the code maintains itself would pass against a transcript that painted
    /// everything and reported otherwise.
    ///
    /// **"The lines are the rows" holds for this fixture and not in general.** The rows
    /// here are one-line user rows, which is what makes it true. An assistant answer is
    /// recorded **per markdown block**, one entry however far the block wrapped, so the
    /// region is not a painted-line counter. That is what
    /// `the_agent_region_records_blocks_and_not_painted_lines` pins, and it is why the
    /// "800 rich rows → 44 lines drawn" figure in ADR-022 was not reproducible from this
    /// tree.
    #[test]
    fn a_transcript_longer_than_the_pane_lays_out_strictly_fewer_rows_than_it_has() {
        const ROWS: usize = 400;
        let (dir, _keep) = scratch_workspace("virtualised");
        let mut h = Harness::new(dir.clone());
        let mut t = Transcript::new(dir);
        for i in 0..ROWS {
            t.push_user(&format!("question {i}"));
        }
        h.app.seed_transcript(t);

        let f = h.paint();
        // The pane's own title is one of the agent region's lines.
        let laid_out = f.region("agent").len();
        assert!(
            laid_out > 1,
            "the transcript drew nothing at all.\n{}",
            f.describe()
        );
        assert!(
            laid_out < ROWS,
            "a {ROWS}-row transcript laid out {laid_out} rows. Nothing is virtualised, and \
             this frame is the 104 ms M266 measured.\n{}",
            f.describe()
        );
        // Tighter than "fewer than the total", because a change that skipped *some* rows
        // would satisfy the loose version while leaving a 200-row frame. At 1440x900 the
        // pane is about 700 points tall; a 400-row conversation cannot fit in it and this
        // says so without pinning a font metric.
        assert!(
            laid_out < 120,
            "the pane should be holding about two dozen rows, not {laid_out}.\n{}",
            f.describe()
        );
    }

    /// **Scrolling reaches the beginning of the conversation.**
    ///
    /// Virtualising a scroll area can silently shorten it: keep only the rows that fit, and
    /// the scrollbar measures the visible rows instead of the transcript, and every row
    /// above the first is simply gone with nothing on screen to say so. The only claim that
    /// survives that is "the first row is still reachable", so that is the assertion — and
    /// it is driven through egui's own wheel, the way a reader scrolls, rather than by
    /// writing an offset into the scroll area from outside.
    #[test]
    fn the_first_row_of_a_long_transcript_is_reachable_by_scrolling_up() {
        const ROWS: usize = 120;
        let (dir, _keep) = scratch_workspace("reachable");
        let mut h = Harness::new(dir.clone());
        let mut t = Transcript::new(dir);
        for i in 0..ROWS {
            t.push_user(&format!("question {i}"));
        }
        h.app.seed_transcript(t);

        // Starts at the bottom, because that is where `stick_to_bottom` puts it, so the
        // first row is *not* on screen yet. If it were, this test would prove nothing.
        //
        // **Two frames, not one.** `stick_to_bottom` is applied at the *end* of a frame
        // (`egui-0.36.2/src/containers/scroll_area.rs:1282-1285` writes the offset into
        // the scroll area's state, which the next frame then lays out against), so the
        // first frame of a fresh transcript is drawn from offset zero. That is egui's
        // behaviour and it was this window's before the change; a test that asserted on
        // one frame would be asserting on the frame counter.
        h.paint();
        let first = h.paint();
        assert!(
            first
                .region("agent")
                .iter()
                .any(|t| t.contains(&format!("question {}", ROWS - 1))),
            "the bottom of the transcript is not what the frame shows.\n{}",
            first.describe()
        );
        assert!(
            !first
                .region("agent")
                .iter()
                .any(|t| t.contains("question 0")),
            "the whole conversation fitted, so scrolling would prove nothing: use more rows."
        );

        let mut reached = false;
        for _ in 0..500 {
            let f = h.wheel_over_the_agent_pane(7.0);
            if f.region("agent").iter().any(|t| t.contains("question 0")) {
                reached = true;
                break;
            }
        }
        assert!(
            reached,
            "500 wheel notches over a {ROWS}-row conversation and row 0 was never drawn: the \
             scroll extent is short of the transcript, so the rows above the first are gone.\n{}",
            h.paint().describe()
        );
    }

    /// The transcript **follows the run**: a row appended while the reader is at the bottom
    /// is on screen on the next frame.
    ///
    /// The behaviour `stick_to_bottom` buys, and the one a virtualised transcript is most
    /// able to lose: the scroll extent is now a number this window maintains
    /// ([`RowLayout::offsets`]) rather than a sum of widget rectangles egui took for
    /// itself, and a transcript that renders fast and stops following the run is worse
    /// than a slow one. Two assertions because they are two facts — the *first* frame after
    /// a long conversation opens at the bottom, and a *later* frame follows a new row down.
    #[test]
    fn the_transcript_follows_a_running_conversation_to_the_bottom() {
        const ROWS: usize = 300;
        let (dir, _keep) = scratch_workspace("follows");
        let mut h = Harness::new(dir.clone());
        let mut t = Transcript::new(dir);
        for i in 0..ROWS {
            t.push_user(&format!("question {i}"));
        }
        h.app.seed_transcript(t);

        // Two frames, for the reason in the scrolling test above: `stick_to_bottom` is applied
        // at the end of a frame, so the first one is laid out from offset zero.
        h.paint();
        let opening = h.paint();
        assert!(
            opening
                .region("agent")
                .iter()
                .any(|t| t.contains(&format!("question {}", ROWS - 1))),
            "a {ROWS}-row conversation must open at its last row.\n{}",
            opening.describe()
        );
        assert!(
            !opening
                .region("agent")
                .iter()
                .any(|t| t.contains("question 0")),
            "and not at its first, or `stick_to_bottom` is not following anything."
        );

        h.app.transcript.push_user("the run kept going");
        let next = h.paint();
        assert!(
            next.region("agent")
                .iter()
                .any(|t| t.contains("the run kept going")),
            "the row the run just appended is not on the next frame: the transcript stopped \
             following it.\n{}",
            next.describe()
        );
    }

    /// Every row of a conversation is drawn by **some** scroll position.
    ///
    /// The two end-to-end tests above walk the scrollbar to its two extremes, which is
    /// what a reader does and what proves the extent is right. This one is the property
    /// underneath them and it is checked directly, because a span calculation that skipped
    /// a row *in the middle* would still pass both: at the top and at the bottom every row
    /// is either on screen or accounted for by the extent.
    ///
    /// A sweep rather than a sample: every pixel of the transcript's height is a viewport
    /// position, and the union of what those positions draw is the set of reachable rows.
    #[test]
    fn every_row_of_the_transcript_is_drawn_for_some_scroll_position() {
        // Mixed heights on purpose: a tool-output row is many lines and an assistant line
        // is one, and a span calculation that assumes one or the other is wrong for the
        // other.
        let heights = [18.0f32, 41.0, 12.5, 96.0, 18.0, 7.5, 63.0, 18.0, 33.0];
        let mut layout = RowLayout {
            width: 500.0,
            // The fixture is a pure `span`/`total` probe and never measures a row, so the
            // text scale it was built at is the default one.
            scale: 1.0,
            rows: heights
                .iter()
                .map(|h| RowHeight {
                    seen: 0,
                    height: *h,
                })
                .collect(),
            offsets: Vec::new(),
        };
        let mut running = 0.0f32;
        layout.offsets.push(running);
        for h in heights {
            running += h;
            layout.offsets.push(running);
        }
        let total = layout.total();
        assert!(
            total > 200.0,
            "the fixture must be taller than one viewport or the sweep proves nothing"
        );

        let mut drawn = vec![false; heights.len()];
        // A viewport as tall as the whole pane: one position of it per pixel, from well
        // above the first row to well below the last.
        let viewport = 700.0;
        let mut y = -400.0f32;
        while y <= total {
            for i in layout.span(y, y + viewport) {
                drawn[i] = true;
            }
            y += 1.0;
        }
        let missed: Vec<usize> = drawn
            .iter()
            .enumerate()
            .filter(|(_, d)| !**d)
            .map(|(i, _)| i)
            .collect();
        assert!(
            missed.is_empty(),
            "rows {missed:?} are never drawn at any scroll position, so they cannot be \
             reached.\noffsets: {:?}",
            layout.offsets
        );
    }

    // -----------------------------------------------------------------------
    // Text scaling — AGENTS.md §8's "honour text scaling".
    //
    // **What the whole group is a control for.** Before it, this window drew text two ways:
    // real egui widgets, whose size comes from `egui::Style`, and `Painter::text` on raw
    // rectangles with a number baked in at the call site. Nothing in the crate ever read a
    // scale, so `ctx.zoom_factor()` was `1.0` forever and neither half answered to it. The
    // tests below drive the real draw path at a real zoom and read the **size the painter
    // was handed**, because that is the only question a reader of the screen can answer.
    // -----------------------------------------------------------------------

    /// The sizes every painter string was drawn at, `region` then points, at `zoom`.
    fn sizes_at(zoom: f32, dir: &std::path::Path) -> Vec<(String, f32)> {
        let mut h = Harness::sized(dir.to_path_buf(), 1440.0, 900.0);
        h.at_zoom(zoom);
        h.paint().text_sizes
    }

    #[test]
    fn every_painter_string_is_drawn_at_the_scaled_size() {
        let (dir, _keep) = scratch_workspace("zoom-painter");
        let plain = sizes_at(1.0, &dir);
        let doubled = sizes_at(2.0, &dir);
        // **A count check first, so a shorter list cannot pass by matching.** Scaling
        // geometry can change which rows of a virtualised pane are on screen; if that
        // happened here the pairing below would be comparing different strings, and the
        // failure would read as a font regression instead of a layout one.
        assert!(
            !plain.is_empty(),
            "no painter text at all: this frame would pass every assertion below vacuously"
        );
        assert_eq!(
            plain.len(),
            doubled.len(),
            "the same strings were painted at both zooms.\n1.0: {plain:?}\n2.0: {doubled:?}"
        );
        for ((region, one), (region2, two)) in plain.iter().zip(&doubled) {
            assert_eq!(
                region, region2,
                "paint order changed: {plain:?} vs {doubled:?}"
            );
            assert_eq!(
                *two,
                one * 2.0,
                "{region} was drawn at {two} points at 200% and {one} at 100%"
            );
        }
    }

    #[test]
    fn at_the_default_zoom_every_size_is_exactly_what_it_was() {
        // **A scaling change that quietly nudges the default is a regression**, and this is
        // the assertion that catches it: the sizes below are the literals this window
        // shipped before `text_scale` existed, to the bit.
        let (dir, _keep) = scratch_workspace("zoom-default");
        let sizes = sizes_at(1.0, &dir);
        let sizes_of = |region: &str| -> Vec<f32> {
            sizes
                .iter()
                .filter(|(r, _)| r == region)
                .map(|(_, s)| *s)
                .collect()
        };
        assert_eq!(sizes_of("title"), vec![15.0], "{sizes:?}");
        assert_eq!(sizes_of("status"), vec![12.0], "{sizes:?}");
        assert_eq!(sizes_of("workspace"), vec![12.0, 11.0], "{sizes:?}");
        assert_eq!(sizes_of("editor"), vec![13.0], "{sizes:?}");
        assert_eq!(sizes_of("tree"), vec![13.0, 13.0], "{sizes:?}");
        // Every painter string the agent pane drew, whatever they are: two at 12 points,
        // because the pane's own heading and the notice stack below it are both 12 and
        // both were 12 before this change.
        assert!(sizes_of("agent").iter().all(|s| *s == 12.0), "{sizes:?}");
        assert!(
            sizes_of("agent").len() >= 2,
            "the agent pane paints its heading and its notices: {sizes:?}"
        );
    }

    #[test]
    fn the_text_holding_geometry_grows_with_the_text_it_holds() {
        // **The rectangles that exist only to hold text.** A scaled composer in an unscaled
        // rectangle is the half-measure this whole change exists to prevent: the feature
        // looks like it works because the text got bigger, and the transcript behind it is
        // now clipped by a composer that did not.
        let (dir, _keep) = scratch_workspace("zoom-geometry");
        let mut h = Harness::sized(dir.clone(), 1440.0, 900.0);
        h.at_zoom(1.0);
        let one = h.paint();
        let filter_one = one.hotspot("file-filter").expect("the filter box is drawn");
        let activity_one = one
            .hotspot("tree:files")
            .expect("the activity bar draws a button");

        let mut h = Harness::sized(dir, 1440.0, 900.0);
        h.at_zoom(2.0);
        let two = h.paint();
        let filter_two = two.hotspot("file-filter").expect("the filter box is drawn");
        let activity_two = two
            .hotspot("tree:files")
            .expect("the activity bar draws a button");

        assert_eq!(
            filter_two.height(),
            filter_one.height() * 2.0,
            "the file filter's box is the height of one line of text"
        );
        assert_eq!(
            activity_two.height(),
            activity_one.height() * 2.0,
            "an activity-bar button is as tall as the glyph in it"
        );
    }

    #[test]
    fn a_row_is_cut_sooner_when_the_text_is_bigger() {
        // **`fit` must measure at the size it draws at.** The budget is
        // `points / (font_size * AVG_ADVANCE)`, so passing an unscaled font size with a
        // scaled width keeps the same number of characters — which is not "a little more
        // text than fits", it is text running off the right edge of the tab, silently.
        let dir = std::env::temp_dir().join("epsilon-zoom-fit");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch workspace");
        // A name far longer than a tab, so the budget is what decides what is on screen.
        let long = "a-very-long-module-name-that-no-tab-can-show.rs";
        std::fs::write(dir.join(long), "fn main() {}\n").expect("write the file");
        // **Enough tabs that the strip's own maximum is not what is being measured.**
        // `TAB_W` scales with the text, so while the clamp binds, a tab is twice as wide
        // at 200% and holds exactly as many characters — the budget is decided by the
        // clamp rather than by the font, and the test would compare two equal labels and
        // call it a pass.
        let mut others = Vec::new();
        for i in 0..7 {
            let name = format!("f{i}.rs");
            std::fs::write(dir.join(&name), "fn main() {}\n").expect("write the file");
            others.push(name);
        }
        let _keep = TempWorkspace(dir.clone());

        let labels_at = |zoom: f32| -> Vec<String> {
            let mut h = Harness::sized(dir.clone(), 1440.0, 900.0);
            // More than one open file, because one open file is one tab and no strip — and
            // more than about six, because past that the strip's maximum tab width takes
            // over from the font.
            h.app.open_file(long);
            for name in &others {
                h.app.open_file(name);
            }
            h.at_zoom(zoom);
            h.paint()
                .region("tabs")
                .into_iter()
                .map(str::to_string)
                .collect()
        };
        let plain = labels_at(1.0);
        let doubled = labels_at(2.0);
        let shown = |labels: &[String]| -> String {
            labels
                .iter()
                .find(|l| l.starts_with('a'))
                .unwrap_or_else(|| panic!("no tab for the long name in {labels:?}"))
                .clone()
        };
        let at_one = shown(&plain);
        let at_two = shown(&doubled);
        assert!(
            at_one.ends_with('…') && at_two.ends_with('…'),
            "the fixture must actually be cut at both zooms: {at_one:?} / {at_two:?}"
        );
        assert!(
            at_two.chars().count() < at_one.chars().count(),
            "doubling the font must spend more of the budget: {at_one:?} then {at_two:?}"
        );
        // The ellipsis is the reader being told the label is partial. Losing it would make
        // the cut value indistinguishable from the whole one.
        assert!(at_two.ends_with('…'), "{at_two:?}");
    }

    #[test]
    fn a_fit_budget_follows_the_size_it_is_given_and_nothing_else() {
        // **The rule `fit` is asked to keep**, stated separately from the draw path so a
        // failure says whether the budget or the caller is wrong. `AVG_ADVANCE` is a ratio
        // and is *not* scaled; scaling it as well would divide the budget by the scale
        // twice and cut every row at half the width the reader can see.
        let long: String = "x".repeat(20);
        let at_13 = fit(&long, 200.0, ROW_FONT);
        let at_26 = fit(&long, 200.0, ROW_FONT * 2.0);
        let count = |s: &str| s.chars().count();
        // 200 / (13 * 0.55) = 27.9 → 27, so twenty characters fit whole.
        assert_eq!(
            at_13, long,
            "twenty characters fit in 200 points at 13pt: {at_13}"
        );
        assert!(
            count(&at_26) < count(&at_13),
            "the same width at twice the size holds fewer characters: {at_26}"
        );
        // 200 / (26 * 0.55) = 13.98 → 13, minus the ellipsis.
        assert_eq!(
            count(&at_26),
            13,
            "the budget is `points / (size * AVG_ADVANCE)`: {at_26}"
        );
    }

    #[test]
    fn the_scale_is_read_from_the_context_rather_than_assumed() {
        // The number this crate scales by is egui's, read once per frame. A window that
        // used its own would be a second scale, and the two would disagree the first time
        // a reader pressed `Ctrl`+`+`.
        let ctx = egui::Context::default();
        assert_eq!(text_scale(&ctx), 1.0, "egui starts at 1.0");
        ctx.set_zoom_factor(1.75);
        let mut warm = ctx.run_ui(egui::RawInput::default(), |_| {});
        warm.textures_delta.clear();
        assert_eq!(text_scale(&ctx), 1.75, "and takes the value it is given");
    }

    #[test]
    fn the_transcript_is_re_measured_when_the_text_scale_changes() {
        // **The cache this protects.** `RowLayout` keeps every row's height and only drops
        // them when the pane's width moves, because a paragraph that wrapped at 500 points
        // wraps differently at 400. A text-scale change has exactly the same effect and was
        // not in the fingerprint: rows measured at 13 points were still the row heights at
        // 200%, so the transcript's own spacers described a window one zoom smaller than
        // the one on screen, and rows were drawn on top of each other.
        let rows = 40;
        let mut transcript = Transcript::default();
        for i in 0..rows {
            transcript.push_user(&format!(
                "message {i} — a sentence long enough to occupy a line of its own"
            ));
        }
        let (dir, _keep) = scratch_workspace("zoom-remeasure");

        let drawn_heights_at = |zoom: f32| -> Vec<f32> {
            let mut h = Harness::sized(dir.clone(), 1440.0, 900.0);
            h.app.seed_transcript(transcript.clone());
            h.at_zoom(zoom);
            h.paint()
                .row_rects
                .into_iter()
                .filter(|(region, _)| region == "agent")
                .map(|(_, r)| r.height())
                .collect()
        };
        let one = drawn_heights_at(1.0);
        let two = drawn_heights_at(2.0);
        assert!(
            one.len() >= 2,
            "the fixture must draw several rows: {one:?}"
        );
        // The transcript's body text is a `LayoutJob`, so it scales with the window rather
        // than with `egui::Style` — a row that did not grow would mean the scaling never
        // reached `markup::BODY_SIZE`.
        assert!(
            two[0] > one[0],
            "a transcript row is taller at 200%: {one:?} then {two:?}"
        );
        // And the geometry the frame *thinks* it has: consecutive rows must not overlap,
        // which is exactly what a stale offset table produces.
        let mut h = Harness::sized(dir, 1440.0, 900.0);
        h.app.seed_transcript(transcript);
        h.at_zoom(2.0);
        let frame = h.paint();
        let rects: Vec<egui::Rect> = frame
            .row_rects
            .iter()
            .filter(|(region, _)| region == "agent")
            .map(|(_, r)| *r)
            .collect();
        for pair in rects.windows(2) {
            assert!(
                pair[1].min.y >= pair[0].max.y - 0.5,
                "two transcript rows overlap at 200%, which is what a stale height cache \
                 produces: {:?} then {:?}",
                pair[0],
                pair[1]
            );
        }
    }

    // ======================================================================================
    // Layout agency: a pane that can be resized, folded, and found again next launch
    //
    // `Preferences` was `Preferences::default()` in both constructors for the whole life of
    // this file. Every test below drives the real draw path, because the failure this closes
    // is precisely the one that passes on state: a stored width that no frame ever used, and
    // a stored flag nothing read.
    // ======================================================================================

    /// **A drag on the handle widens the sidebar, and it stops at both ends.**
    ///
    /// The handle is found through [`FrameSummary::hotspot`] rather than through arithmetic,
    /// so the pointer presses where the frame actually drew it. The width is then read off
    /// **the file rows' own rectangles**: a row's right edge *is* the sidebar's right edge as
    /// a reader sees it, so that is the assertion that the drag reached the screen rather than
    /// only the field.
    ///
    /// **Both clamps, because one of them is invisible on its own.** A drag that overshot
    /// `SIDEBAR_MAX` and kept going would still look like a working handle; the failure is a
    /// sidebar that has eaten the editor, which is what `SIDEBAR_MAX` exists to prevent.
    #[test]
    fn dragging_the_sidebar_handle_resizes_it_and_stops_at_both_ends() {
        let (dir, _keep) = scratch_workspace("resize");
        let mut h = Harness::new(dir);

        let before = h.paint();
        let handle = before
            .hotspot(SIDEBAR_HANDLE_HOTSPOT)
            .expect("the sidebar's right edge is drawn as a drag target");
        assert_eq!(
            handle.width(),
            crate::layout::SIDEBAR_HANDLE_W,
            "the handle is the gutter and not wider — a wider strip would take clicks away \
             from the file rows beside it: {handle:?}"
        );
        let row_before = before
            .hotspot("tree:alpha.rs")
            .expect("the scratch workspace has a file in its tree")
            .right();

        h.drag(
            handle.center(),
            egui::pos2(handle.center().x + 90.0, handle.center().y),
        );
        let after = h.paint();
        let row_after = after
            .hotspot("tree:alpha.rs")
            .expect("the tree is still drawn after the drag")
            .right();
        assert!(
            row_after - row_before > 80.0,
            "a 90-point drag moved the file rows by {} points.\n{}",
            row_after - row_before,
            after.describe()
        );

        // **Far past the ceiling, in one hop.** The pointer is not walking there a notch at a
        // time: a real overshoot is one fast movement, and a clamp that only held for slow
        // drags would be no clamp at all.
        //
        // **Both the drawn width and the stored one, and the second is not redundant.**
        // `IdeLayout::new` clamps on the way to the screen, so a drag that left 4 354 in
        // `prefs` would still *draw* 420 — and that is precisely why the clamp has to be at
        // the drag as well: the stored value is what gets written to disk, and a reader who
        // released at the ceiling and moved the pointer back would watch the sidebar jump.
        let handle = after.hotspot(SIDEBAR_HANDLE_HOTSPOT).expect("still drawn");
        h.drag(
            handle.center(),
            egui::pos2(handle.center().x + 4000.0, handle.center().y),
        );
        assert_eq!(
            h.app.layout().sidebar.width(),
            crate::layout::SIDEBAR_MAX,
            "a drag past the ceiling must stop at it, not eat the editor.\n{}",
            h.paint().describe()
        );
        assert_eq!(
            h.app.prefs.sidebar_width,
            crate::layout::SIDEBAR_MAX,
            "and it must *stop* at it, rather than running on and being clamped only on the \
             way to the screen: releasing past the end and coming back would otherwise snap."
        );

        let handle = h
            .paint()
            .hotspot(SIDEBAR_HANDLE_HOTSPOT)
            .expect("still drawn");
        h.drag(
            handle.center(),
            egui::pos2(handle.center().x - 4000.0, handle.center().y),
        );
        assert_eq!(
            h.app.layout().sidebar.width(),
            crate::layout::SIDEBAR_MIN,
            "a drag past the floor must stop at it: below this a file name and its expander \
             stop reading."
        );
        assert_eq!(
            h.app.prefs.sidebar_width,
            crate::layout::SIDEBAR_MIN,
            "and it must stop there too, not run on to a negative width."
        );
    }

    /// **`Ctrl`+`B` folds the sidebar away and brings it back, from the keyboard.**
    ///
    /// **`chord`, not a call into the handler.** A test that sets `prefs.sidebar_visible`
    /// itself would pass against a window where the key was bound to nothing, and `AGENTS.md`
    /// §2.7 calls that the failure this whole feature has to avoid.
    ///
    /// The frame is the assertion at both ends: the activity bar and the sidebar stop drawing
    /// when folded, the tier says `no-sidebar`, and the title bar's own control reports the
    /// action it would now take. A fold that changed a bool and not the window would leave
    /// every one of those unchanged.
    #[test]
    fn the_sidebar_folds_and_unfolds_from_the_keyboard() {
        let (dir, _keep) = scratch_workspace("fold");
        let mut h = Harness::new(dir);

        let open = h.paint();
        assert_eq!(open.tier, Tier::Full);
        assert!(
            open.region("activity").iter().any(|t| t.contains("files")),
            "the sidebar and its activity bar are on screen to begin with.\n{}",
            open.describe()
        );

        let folded = h.chord(egui::Modifiers::CTRL, egui::Key::B);
        assert_eq!(
            folded.tier,
            Tier::NoSidebar,
            "the window folded the sidebar.\n{}",
            folded.describe()
        );
        assert!(
            folded.region("activity").is_empty(),
            "the activity bar belongs to the sidebar, so it goes with it.\n{}",
            folded.describe()
        );
        assert!(
            folded.hotspot(SIDEBAR_HANDLE_HOTSPOT).is_none(),
            "a handle for a sidebar that is not on screen is a control a reader cannot find."
        );
        assert!(
            folded.region("sidebar-toggle").contains(&"Show sidebar"),
            "the control now says it will show it — a fold with no way back is a one-way \
             door.\n{}",
            folded.describe()
        );

        let back = h.chord(egui::Modifiers::CTRL, egui::Key::B);
        assert_eq!(
            back.tier,
            Tier::Full,
            "the same key brought it back.\n{}",
            back.describe()
        );
        assert!(back.drew("activity"), "{}", back.describe());
    }

    /// **What the window wrote is what the next window starts from.**
    ///
    /// **The window is the subject, not the store.** A test that saved a `Preferences`, read
    /// it back with `load_in` and called the feature proved the file format works — which
    /// `the_layout_comes_back_exactly_as_it_went_in` in `layout.rs` already covers. What this
    /// has to rule out is the window failing to *use* it, which is the shape every defect in
    /// this file's history has taken: `Preferences` was constructed correctly and read by
    /// nothing.
    ///
    /// So: a real [`Harness`] resizes and folds, a **second** window is pointed at the same
    /// settings directory the way a relaunch would be, and what it starts from is asserted —
    /// including the width the layout it carves on its first frame.
    #[test]
    fn what_the_window_wrote_is_what_the_next_window_starts_from() {
        let (dir, _keep) = scratch_workspace("remember");
        let mut first = Harness::new(dir.clone());

        // A reader's two gestures, through the real input path.
        let handle = first
            .paint()
            .hotspot(SIDEBAR_HANDLE_HOTSPOT)
            .expect("the handle is drawn");
        first.drag(
            handle.center(),
            egui::pos2(handle.center().x - 40.0, handle.center().y),
        );
        first.chord(egui::Modifiers::CTRL, egui::Key::B);
        let written = first.app.prefs;
        assert!(
            !written.sidebar_visible
                && (written.sidebar_width - crate::layout::SIDEBAR_DEFAULT).abs() > 1.0,
            "the fixture must actually change both fields, or the relaunch below proves \
             nothing: {written:?}"
        );
        assert_eq!(
            std::fs::read_dir(first.settings.path())
                .expect("the settings directory")
                .count(),
            1,
            "a gesture must write the settings file, and only it: {:?}",
            first.settings.path()
        );

        // The relaunch. `bounded` is what a constructor does — resolve the directory, load
        // what is there — pointed at the directory the first window wrote into.
        let mut second = Harness::new(dir.clone());
        second.app.prefs_dir = Some(first.settings.path().to_path_buf());
        second.app.prefs =
            Preferences::load_from(second.app.workspace(), second.app.prefs_dir.as_deref());

        let reopened = second.paint();
        assert_eq!(
            reopened.tier,
            Tier::NoSidebar,
            "the next window started with the sidebar folded and must draw it folded.\n{}",
            reopened.describe()
        );
        assert!(
            reopened.region("activity").is_empty(),
            "which means no activity bar either — a folded sidebar that still drew its strip \
             would be half a preference.\n{}",
            reopened.describe()
        );

        // And the width, on a third window that does **not** inherit the fold, so the two
        // fields are proved independently rather than by one flag hiding the other.
        let mut third = Harness::new(dir);
        third.app.prefs_dir = Some(first.settings.path().to_path_buf());
        third.app.prefs = Preferences {
            sidebar_visible: true,
            ..Preferences::load_from(third.app.workspace(), third.app.prefs_dir.as_deref())
        };
        third.app.layout = crate::layout::IdeLayout::new(
            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1440.0, 900.0)),
            third.app.prefs,
        );
        let third_frame = third.paint();
        assert!(
            (third.app.layout().sidebar.width() - written.sidebar_width).abs() < 0.01,
            "and at the width the reader left it ({}), not at the default ({}).\n{}",
            third.app.layout().sidebar.width(),
            crate::layout::SIDEBAR_DEFAULT,
            third_frame.describe()
        );
    }

    /// **Both new controls are named, and neither is named by its glyph.**
    ///
    /// `AGENTS.md` §8 and ADR-022: a control a screen reader cannot name is skipped in
    /// silence, which is worse than not drawing one. Both halves of that are checked —
    /// that a node exists with a name that says what the control does *and* which key does
    /// it, and that the name is not the `◨` egui would have left there.
    #[test]
    fn the_resize_handle_and_the_fold_control_are_named_for_a_screen_reader() {
        let (dir, _keep) = scratch_workspace("a11y-layout");
        let mut h = Harness::new(dir);

        assert_eq!(
            h.node_named("Resize sidebar"),
            format!(
                "Resize sidebar · {} points · drag, or use the left and right arrow keys",
                crate::layout::SIDEBAR_DEFAULT
            ),
            "the name reports the width on screen and the keyboard route to it."
        );
        // **Read from the table, not from the source text.** These are the assertions
        // that a chord printed on screen is the chord the window acts on, and a test that
        // compared the two to a hand-written literal would keep passing after someone
        // renamed the binding.
        let toggle = crate::keys::KeyBindings::defaults()
            .label("window.sidebar.toggle")
            .expect("the window declares a sidebar chord");
        assert_eq!(
            h.node_named("Hide sidebar"),
            format!("Hide sidebar · {toggle}"),
            "the control says what it does *now* and which key does it, because the glyph \
             reaches neither a reader nor `on_hover_text`."
        );

        // **The same name changes with the state it reports**, which is what makes it worth
        // reading: a control named "sidebar" whatever the state is describes nothing.
        h.chord(egui::Modifiers::CTRL, egui::Key::B);
        assert_eq!(
            h.node_named("Show sidebar"),
            format!("Show sidebar · {toggle}"),
            "with the sidebar folded the control now offers to show it."
        );
        // And the handle is gone with it — a node for a control that is not on screen is a
        // control a reader cannot find.
        let nodes = h.named_nodes();
        assert!(
            !nodes.iter().any(|(l, _)| l.starts_with("Resize sidebar")),
            "a folded sidebar must publish no handle node.\nthe tree held: {nodes:?}"
        );
    }

    /// **The status bar prints the layout keys, and only while they work.**
    ///
    /// `AGENTS.md` §2.7: a key the window advertises and does not act on is a lie. The
    /// palette owns every key while it is open, and the arrows do nothing once the sidebar
    /// has folded, so both of those states must lose the hint rather than keep it.
    #[test]
    fn the_status_bar_prints_the_layout_keys_only_while_they_work() {
        let (dir, _keep) = scratch_workspace("hints");
        let mut h = Harness::new(dir);

        let open = h.paint();
        let hint = open.region("hints").join(" ");
        let toggle = crate::keys::KeyBindings::defaults()
            .label("window.sidebar.toggle")
            .expect("the window declares a sidebar chord");
        assert!(
            hint.contains(&toggle) && hint.contains("left/right"),
            "with the sidebar up, both keys are live and both are printed: {hint:?}"
        );

        h.chord(egui::Modifiers::CTRL, egui::Key::P);
        let palette = h.paint();
        assert!(
            palette.region("hints").is_empty(),
            "the palette owns every key while it is open, so no key may be advertised: {:?}",
            palette.region("hints")
        );
        h.press(egui::Key::Escape);

        h.chord(egui::Modifiers::CTRL, egui::Key::B);
        let folded = h.paint();
        let hint = folded.region("hints").join(" ");
        assert!(
            hint.contains(&toggle) && !hint.contains("left/right"),
            "with the sidebar folded there is nothing to resize, so the arrows are not \
             printed: {hint:?}"
        );
    }
}
