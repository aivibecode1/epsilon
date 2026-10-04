//! Where each region of the IDE window sits, and how the layout gives way.
//!
//! # Why this is a pure function and not something the draw code decides
//!
//! A layout that is computed *while drawing* is a layout nobody can test: it only
//! exists on a screen, at the one size the author happened to run, and "the sidebar
//! covers the transcript on a narrow window" is found by a user rather than by a
//! failing assertion. So the geometry here is a pure function of (`available rect`,
//! [`Preferences`]) and every degradation rule below has a test that fails if the rule
//! changes.
//!
//! This is the same discipline the terminal frontend uses for its own layout
//! (`epsilon_tui::ui::layout`), and it exists for the same reason. It has already paid
//! for itself: the first version of this file overlapped the title bar with the
//! activity bar at every ordinary window size, and made the editor pane a fixed width
//! that never grew. Reading the arithmetic did not find either. Painting frames did.
//!
//! # The degradation order, and why it is this order
//!
//! A desktop window can be resized to anything, and the failure mode to avoid is
//! *silent overlap* — two panes drawn on top of each other, which reads as a
//! rendering bug rather than as a window that is too small. So the rules give way in
//! reverse order of importance:
//!
//! 1. **Below [`NARROW`] the agent panel takes the whole body.** The agent is the
//!    product; the editor is a view onto files. In a narrow window the right answer is
//!    to show the product and let the user widen the window.
//! 2. **Below [`MEDIUM`] the sidebar folds away.** A 200-point file tree in an
//!    800-point window costs more than it gives.
//! 3. **Below [`SHORT`] the status bar folds into the title bar.** Nothing is lost;
//!    the same text is drawn, one line instead of two.
//! 4. **Below [`COMPACT`] the title bar goes too.** The operating system's own title
//!    bar already names the app; repeating it inside the window is the one piece of
//!    chrome that is pure duplication.
//!
//! # What the user gets to decide
//!
//! Every rule above is the window overruling the user, and an IDE that only ever gives
//! way is a picture of an IDE. So [`Preferences`] is **read from disk at construction and
//! written back when it changes**, and the sidebar has the two controls every other IDE
//! has: a drag handle on its right edge ([`SIDEBAR_HANDLE_W`]) and a chord that folds it
//! away. Neither rule above changes — a window too narrow still folds the sidebar, and a
//! reader who has dragged it to [`SIDEBAR_MAX`] in a wide window still sees it fold in a
//! narrow one. That is the whole contract: the window wins when it must, and otherwise
//! the width on screen is the width the reader chose.

use egui::Rect;
use std::path::{Path, PathBuf};

/// Width below which the sidebar folds away.
///
/// 860 is the sum of what a three-pane window actually needs: the activity bar (48),
/// the sidebar floor (200), the editor floor (340), the agent floor (320), and three
/// 6-point gutters between them. A window exactly this narrow is not one anybody
/// works in, which is the point of the threshold.
pub const MEDIUM: f32 = 860.0;

/// Width below which the agent panel takes the whole body.
pub const NARROW: f32 = 680.0;
/// Height below which the status bar folds into the title bar.
pub const SHORT: f32 = 520.0;
/// Height below which the title bar is dropped entirely.
pub const COMPACT: f32 = 360.0;

/// Default sidebar width, and the bounds it is clamped to.
///
/// The floor is the width at which a file name plus its expander still reads; the
/// ceiling stops a drag from eating the editor.
pub const SIDEBAR_DEFAULT: f32 = 264.0;
/// Smallest sidebar worth showing.
pub const SIDEBAR_MIN: f32 = 200.0;
/// Largest sidebar worth showing.
pub const SIDEBAR_MAX: f32 = 420.0;
/// Smallest editor pane worth showing.
pub const CENTER_MIN: f32 = 340.0;
/// Smallest agent panel worth showing.
pub const AGENT_MIN: f32 = 320.0;
/// The agent panel's comfortable width, and the point past which it stops growing.
pub const AGENT_DEFAULT: f32 = 520.0;
/// Largest agent panel worth showing; extra width goes to the editor.
pub const AGENT_MAX: f32 = 900.0;
/// Width of the activity bar (the icon strip).
pub const ACTIVITY_BAR: f32 = 48.0;
/// Height of the window's status line.
pub const TITLE_H: f32 = 36.0;
/// Height of the bottom bar.
pub const STATUS_H: f32 = 24.0;

/// Gaps between regions, in points.
///
/// One number rather than four: a layout with per-edge padding drifts, and the drift
/// is invisible until someone resizes the window.
pub const GUTTER: f32 = 6.0;

/// Width of the strip on the sidebar's right edge that a reader drags to resize it.
///
/// **Exactly [`GUTTER`], on purpose.** The handle sits in the gap the carve already
/// leaves between the sidebar and the editor, so a strip wider than the gutter would
/// reach into a pane and take pointer input away from it — the last few points of every
/// file row, or the first few of the editor. WCAG 2.2 SC 2.5.8 asks 24×24 points for a
/// pointer target and this is 6 wide, so the deviation is real and it is written down
/// rather than left to be discovered: the alternatives were a control that silently eats
/// clicks, and a second `GUTTER` rule that would move every pane. The keyboard route
/// (`←` / `→`, at [`SIDEBAR_STEP`]) is the mitigation, and it is the reason that binding
/// is not a convenience.
pub const SIDEBAR_HANDLE_W: f32 = 6.0;

/// How far one `←` or `→` press moves the sidebar.
///
/// Wider than the one point a spinner would use and narrower than a file name, so a
/// reader holding the key reaches the end of the range in about a second and a dozen
/// presses do not cross the window.
pub const SIDEBAR_STEP: f32 = 16.0;

/// What the user asked for, as opposed to what the window can afford.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Preferences {
    /// Width of the sidebar, clamped into `[SIDEBAR_MIN, SIDEBAR_MAX]` on use.
    pub sidebar_width: f32,
    /// Whether the sidebar is wanted at all.
    pub sidebar_visible: bool,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            sidebar_width: SIDEBAR_DEFAULT,
            sidebar_visible: true,
        }
    }
}

/// The file `workspace`'s window settings live in, inside `dir`.
///
/// **A file of its own, not a table in `config.toml`.**
/// [`epsilon_core::config::save_model_id_in`] merges into the shared file because a model id
/// is a fact every frontend reads and one of them writes; a pane width is a fact about this
/// window alone, and a separate file is what makes "the fields this change added, and
/// nothing else" true of the bytes on disk rather than merely of the diff.
///
/// **Nothing in it is a credential.** `AGENTS.md` §6 forbids writing a secret in plaintext;
/// this file holds one float and one bool, both of which the window already shows on screen
/// and both of which are re-derivable by deleting the file.
///
/// # Why it is named per workspace rather than one file for the machine
///
/// **One file was measured, and it made nine tests describe the reader's window rather than
/// the product's.** `IdeApp::without_agent` and `IdeApp::new` resolve the directory
/// themselves, so `tests/window.rs` — which a writer of `app.rs` may not edit — inherits
/// whatever the person running the suite has resized. With a single `desktop.json` in the
/// global directory saying the sidebar was folded, **9 tests in this crate went red: 1 in
/// the lib and 8 in `tests/window.rs`**. Every workspace those tests use is a throwaway the
/// suite created itself, so naming the file after the workspace removes the dependence
/// rather than relocating it.
///
/// **It is also the better product answer, not only the safer one.** A monorepo's file tree
/// wants a wide sidebar and a dotfiles repository's does not; one number cannot be right for
/// both, and a reader who had to re-drag the sidebar in every repository is a reader who
/// stops trusting the window to remember anything.
///
/// **The digest is over the canonical path**, so one directory reached two ways — a symlink,
/// a trailing separator — is one window's settings and not two. `canonicalize` failing is
/// not an error: the raw path is used, because the worst case of a missing canonical form is
/// a second set of preferences and the worst case of refusing is the window not opening.
fn settings_path(dir: &Path, workspace: &Path) -> PathBuf {
    let canonical = workspace
        .canonicalize()
        .unwrap_or_else(|_| workspace.to_path_buf());
    // Lower-cased because Windows paths are case-insensitive: `C:\repo` and `c:/repo` are
    // one directory, and two windows' settings for one directory is a bug with no symptom.
    let digest =
        epsilon_core::config::sha256_hex(canonical.to_string_lossy().to_lowercase().as_bytes());
    let name = canonical
        .file_name()
        .and_then(|n| n.to_str())
        .filter(|n| !n.is_empty())
        .unwrap_or("workspace");
    dir.join(format!("{name}-{}.json", &digest[..16]))
}

impl Preferences {
    /// The width the window will actually draw, clamped into the readable range.
    ///
    /// **One clamp, here, because there are now three callers** — the layout that carves
    /// the pane, the drag that moves it, and the file that carries it between launches.
    /// Three `clamp` calls are three chances to disagree about what "the sidebar is five
    /// points wide" means, and the disagreement is invisible until a pane is unreadable.
    pub fn sidebar_width_clamped(&self) -> f32 {
        self.sidebar_width.clamp(SIDEBAR_MIN, SIDEBAR_MAX)
    }

    /// The preferences the last launch of this workspace left behind, or the defaults.
    ///
    /// **`dir` is [`Option`] so the product and a test say the same thing.** `None` is the
    /// product's answer — the resolved global config directory — and spelling that as `None`
    /// rather than as a second function means there is exactly one load and one fallback in
    /// this module, whichever side is calling.
    ///
    /// **A missing, unreadable, malformed or out-of-range file resolves to the defaults and
    /// is never an error.** `AGENTS.md` §2.6 is about a refusal path that does not end the
    /// process, and a window that refused to open because a sixty-byte preferences file was
    /// truncated would be the worst answer available: the reader loses the agent over a pane
    /// width. So every failure here — no directory, no file, no permission, no parse — lands
    /// on [`Preferences::default`], and the file is rewritten the next time a preference
    /// changes.
    ///
    /// The same rule [`epsilon_core::config::load_model_id_in`] states: **configuration is
    /// advisory at read time and must never block startup.**
    pub fn load_from(workspace: &Path, dir: Option<&Path>) -> Self {
        match dir {
            Some(dir) => Self::load_in(dir, workspace),
            None => match epsilon_core::config::global_dir() {
                Some(dir) => Self::load_in(&dir, workspace),
                None => Self::default(),
            },
        }
    }

    /// [`Self::load_from`] against one directory, for a caller that knows where its settings
    /// are.
    pub fn load_in(dir: &Path, workspace: &Path) -> Self {
        let Ok(raw) = std::fs::read_to_string(settings_path(dir, workspace)) else {
            return Self::default();
        };
        // A parse failure is not distinguished from a missing file, on purpose: both mean
        // "this window starts from the defaults", and a caller cannot act on the difference.
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) else {
            return Self::default();
        };
        Self::from_json(&value).unwrap_or_default()
    }

    /// Read the two fields out of a parsed settings file.
    ///
    /// **All of it or none of it.** A file missing either key is treated as unreadable
    /// rather than half-applied, which is the rule `config::parse_model_id` already sets
    /// for the model id. Extra keys a later build wrote are simply not read, so the file
    /// survives a downgrade intact.
    fn from_json(value: &serde_json::Value) -> Option<Self> {
        let width = value.get("sidebar_width")?.as_f64()? as f32;
        let visible = value.get("sidebar_visible")?.as_bool()?;
        Some(Self {
            // Clamped on the way in as well as on the way out: a file hand-edited to
            // `9999` must not put a sidebar on screen that eats the editor until the
            // first drag.
            sidebar_width: width.clamp(SIDEBAR_MIN, SIDEBAR_MAX),
            sidebar_visible: visible,
        })
    }

    /// Write these preferences for `workspace` into `dir`, or into the product's own
    /// directory when it is [`None`], creating it if needed.
    ///
    /// **Best-effort by contract**, exactly as [`epsilon_core::config::save_model_id_in`] is:
    /// the error is returned for a caller that wants to warn, and is never fatal. Only the
    /// *remembering* is best-effort — the caller has already applied the preference by the
    /// time it gets here, and a window that refused to resize because the config directory
    /// was read-only would be a worse product than one that forgets.
    pub fn save_to(&self, workspace: &Path, dir: Option<&Path>) -> Result<PathBuf, String> {
        let dir = match dir {
            Some(dir) => dir.to_path_buf(),
            None => epsilon_core::config::global_dir()
                .ok_or_else(|| "no home directory resolved".to_string())?,
        };
        self.save_in(&dir, workspace)
    }

    /// [`Self::save_to`] against one directory.
    ///
    /// **Keys this build does not know about survive**, because the file is read back as a
    /// [`serde_json::Map`] and only these two entries are replaced. That is the same
    /// round-trip `save_model_id_in` does through a `toml::Table`, and for the same reason:
    /// a build that knows more must not lose what this one wrote.
    pub fn save_in(&self, dir: &Path, workspace: &Path) -> Result<PathBuf, String> {
        std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
        let path = settings_path(dir, workspace);
        let mut table: serde_json::Map<String, serde_json::Value> = std::fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default();
        // A width JSON cannot represent (a NaN, which `clamp` passes through) is refused
        // rather than written as `null`, because a file that reads back as "unreadable"
        // is a self-inflicted reset on the next launch.
        let width = serde_json::Number::from_f64(f64::from(self.sidebar_width_clamped()))
            .ok_or_else(|| "refusing to persist a width JSON cannot hold".to_string())?;
        table.insert("sidebar_width".into(), serde_json::Value::Number(width));
        table.insert(
            "sidebar_visible".into(),
            serde_json::Value::Bool(self.sidebar_visible),
        );
        let text = serde_json::to_string_pretty(&serde_json::Value::Object(table))
            .map_err(|e| format!("serialize desktop settings: {e}"))?;
        std::fs::write(&path, text).map_err(|e| format!("write {}: {e}", path.display()))?;
        Ok(path)
    }
}

/// Which degradation rule is in force.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tier {
    /// Everything is shown.
    #[default]
    Full,
    /// The sidebar folded away.
    NoSidebar,
    /// The agent panel took the body.
    AgentOnly,
    /// The status bar folded into the title bar.
    CompactHeight,
    /// The title bar was dropped too.
    ChromeOnly,
}

impl Tier {
    /// A stable name, for a test that asserts on the tier rather than on a rect.
    pub fn name(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::NoSidebar => "no-sidebar",
            Self::AgentOnly => "agent-only",
            Self::CompactHeight => "compact-height",
            Self::ChromeOnly => "chrome-only",
        }
    }
}

/// The rectangles a frame draws into.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IdeLayout {
    /// The window's own status line: workspace, model, context, cost.
    pub title: Rect,
    /// The icon strip.
    pub activity: Rect,
    /// The file / session / task tree.
    pub sidebar: Rect,
    /// The strip on the sidebar's right edge that resizes it. `Rect::ZERO` when the
    /// sidebar is not drawn.
    ///
    /// **Geometry, so it is computed here rather than in the draw code.** A handle drawn
    /// from arithmetic the reader of this module cannot see is exactly the "the sidebar
    /// overlaps the transcript" class of defect this file exists to rule out, and a test
    /// can only rule out what it can read.
    pub sidebar_handle: Rect,
    /// The editor or diff.
    pub center: Rect,
    /// The transcript, composer and approvals.
    pub agent: Rect,
    /// The bottom bar: goal phase, pending approvals, key hints.
    pub status: Rect,
    /// Whether the sidebar was actually drawn.
    pub sidebar_visible: bool,
    /// Whether the agent panel was actually drawn.
    pub agent_visible: bool,
    /// Whether the center pane was actually drawn.
    pub center_visible: bool,
    /// Which degradation rule fired, for the test that names each one.
    pub tier: Tier,
}

impl IdeLayout {
    /// Compute the frame's geometry for `available` under `prefs`.
    ///
    /// Regions that are not drawn come back **zero-width**, never a zero-width rect at
    /// some stale offset: a caller that forgets to check a `*_visible` flag then draws
    /// nothing rather than drawing a hairline or, worse, a full-width pane on top of
    /// another.
    pub fn new(available: Rect, prefs: Preferences) -> Self {
        let hidden = Rect::ZERO;
        // A window can report a non-positive rect — a monitor unplugged mid-drag on
        // Windows reports zero, and some maximized-window configurations report
        // negative. Normalise once, here, rather than guarding every subtraction below;
        // the first version produced a negative-width `title` at `-10x900` and the
        // degenerate-window test caught it.
        let avail_w = available.width().max(0.0);
        let avail_h = available.height().max(0.0);

        // ---- horizontal tier -------------------------------------------------
        //
        // Measured against the minimum widths each pane needs to be readable, not
        // against round numbers.
        let (sidebar_visible, agent_visible, center_visible) = if avail_w < NARROW {
            // Not enough for a file tree AND an agent panel: show the agent.
            (false, true, false)
        } else if avail_w < MEDIUM {
            (false, true, true)
        } else {
            (
                prefs.sidebar_visible,
                true,
                // A sidebar the user asked for still yields to an editor that cannot
                // show a line of code. Clamping happens here, once.
                (avail_w - ACTIVITY_BAR - GUTTER * 2.0) >= SIDEBAR_MIN + CENTER_MIN,
            )
        };

        // ---- vertical tier ----------------------------------------------------
        let want_status = avail_h >= SHORT;
        let want_title = avail_h >= COMPACT;

        // ---- carve ------------------------------------------------------------
        //
        // Every height is subtracted explicitly, including the gap that belongs to each
        // band, and the body starts *below* the title.
        let title_h = if want_title { TITLE_H } else { 0.0 };
        let status_h = if want_status { STATUS_H } else { 0.0 };
        let gap_title = if want_title { GUTTER } else { 0.0 };
        let gap_status = if want_status { GUTTER } else { 0.0 };
        let body_top = available.min.y + title_h + gap_title;
        let body_h = (avail_h - title_h - gap_title - status_h - gap_status).max(0.0);
        let right = available.min.x + avail_w;

        let title = if want_title {
            Rect::from_min_size(
                egui::pos2(available.min.x, available.min.y),
                egui::vec2(avail_w, title_h),
            )
        } else {
            hidden
        };

        let status = if want_status {
            Rect::from_min_size(
                egui::pos2(available.min.x, available.min.y + avail_h - status_h),
                egui::vec2(avail_w, status_h),
            )
        } else {
            hidden
        };

        // Horizontal carve, left to right, advancing `x` past each pane **and** the
        // gutter after it. Then one split of what is left, rather than three
        // subtractions from three different copies of the width.
        let mut x = available.min.x;

        let activity = if sidebar_visible {
            let r = Rect::from_min_size(egui::pos2(x, body_top), egui::vec2(ACTIVITY_BAR, body_h));
            x += ACTIVITY_BAR + GUTTER;
            r
        } else {
            hidden
        };

        let sidebar = if sidebar_visible {
            let w = prefs.sidebar_width_clamped();
            let r = Rect::from_min_size(egui::pos2(x, body_top), egui::vec2(w, body_h));
            x += w + GUTTER;
            r
        } else {
            hidden
        };

        // The grab strip, centred on the gutter the sidebar already gave up. **Never wider
        // than [`GUTTER`]** — see [`SIDEBAR_HANDLE_W`] — so it cannot overlap a pane, and
        // the no-overlap invariant the tests below assert keeps holding with it on screen.
        let sidebar_handle = if sidebar_visible {
            Rect::from_center_size(
                egui::pos2(sidebar.right() + GUTTER / 2.0, sidebar.center().y),
                egui::vec2(SIDEBAR_HANDLE_W, sidebar.height()),
            )
        } else {
            hidden
        };

        // The editor absorbs the leftover: the sidebar has a natural width (a path)
        // and the agent panel has a comfortable one, so a wider window widens the
        // *file view*, not the transcript.
        //
        // The gutter between the two is subtracted here rather than at the call site,
        // because the first version put them flush against each other — the pane
        // borders then read as one line and the only test that noticed was the
        // overlap check, which saw two rects sharing an edge.
        let rest = (right - x).max(0.0);
        let (center_w, agent_w) = if !agent_visible {
            (rest, 0.0)
        } else if !center_visible {
            (0.0, rest)
        } else {
            split(rest)
        };

        let center = if center_visible {
            Rect::from_min_size(egui::pos2(x, body_top), egui::vec2(center_w, body_h))
        } else {
            hidden
        };

        let agent = if agent_visible {
            let w = if center_visible { agent_w } else { rest };
            Rect::from_min_size(egui::pos2(right - w, body_top), egui::vec2(w, body_h))
        } else {
            hidden
        };

        let tier = if !want_title {
            Tier::ChromeOnly
        } else if !want_status {
            Tier::CompactHeight
        } else if !center_visible {
            Tier::AgentOnly
        } else if !sidebar_visible {
            Tier::NoSidebar
        } else {
            Tier::Full
        };

        Self {
            title,
            activity,
            sidebar,
            sidebar_handle,
            center,
            agent,
            status,
            sidebar_visible,
            agent_visible,
            center_visible,
            tier,
        }
    }
}

/// Split `rest` between the editor and the agent panel, honouring both minimums.
///
/// Two rules, and each one exists because breaking it produced a window that was wrong
/// rather than merely tight:
///
/// * **Both minimums first.** Below `CENTER_MIN + AGENT_MIN` there is no layout that
///   gives both panes enough room, so the width is shared in proportion to the
///   minimums. Neither pane overlaps the other and neither is starved to zero, which is
///   the best available answer and is what a window this small deserves.
/// * **Above that, the agent panel takes its default and the editor takes the rest**,
///   bounded by [`AGENT_MAX`] so a 4K window does not produce a transcript 1,900 points
///   wide.
fn split(rest: f32) -> (f32, f32) {
    if rest <= 0.0 {
        return (0.0, 0.0);
    }
    let both = CENTER_MIN + AGENT_MIN + GUTTER;
    if rest >= both {
        let agent = AGENT_DEFAULT.clamp(AGENT_MIN, (rest - CENTER_MIN - GUTTER).min(AGENT_MAX));
        (rest - agent - GUTTER, agent)
    } else {
        let agent = (rest * (AGENT_MIN / both)).round();
        (rest - agent - GUTTER, agent)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn avail(w: f32, h: f32) -> Rect {
        Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(w, h))
    }

    #[test]
    fn a_comfortable_window_shows_every_region() {
        let l = IdeLayout::new(avail(1440.0, 900.0), Preferences::default());
        assert_eq!(l.tier, Tier::Full);
        assert!(l.sidebar_visible && l.center_visible && l.agent_visible);
        assert!(l.title.height() > 0.0 && l.status.height() > 0.0);
    }

    #[test]
    fn every_region_fits_inside_the_window_and_none_of_them_overlaps_another() {
        // The property the whole module exists for: silent overlap reads as a
        // rendering bug, so it is asserted rather than eyeballed.
        for w in [
            1400.0, 1100.0, 1000.0, 900.0, 859.0, 700.0, 679.0, 640.0, 1024.0, 768.0,
        ] {
            for h in [900.0, 600.0, 519.0, 400.0, 359.0, 300.0] {
                let l = IdeLayout::new(avail(w, h), Preferences::default());
                let drawn: Vec<(&str, Rect)> = [
                    ("title", l.title),
                    ("activity", l.activity),
                    ("sidebar", l.sidebar),
                    ("center", l.center),
                    ("agent", l.agent),
                    ("status", l.status),
                ]
                .into_iter()
                .filter(|(_, r)| r.width() > 0.0 && r.height() > 0.0)
                .collect();

                assert!(
                    !drawn.is_empty(),
                    "at {w}x{h} nothing at all is drawn, which is the 'nothing happened' \
                     state this layout must never be able to be in (tier {})",
                    l.tier.name()
                );

                for (name, r) in &drawn {
                    assert!(
                        r.min.x >= -0.01
                            && r.min.y >= -0.01
                            && r.max.x <= w + 0.01
                            && r.max.y <= h + 0.01,
                        "{name} at {w}x{h} escapes the window: {r:?}"
                    );
                }

                for i in 0..drawn.len() {
                    for j in (i + 1)..drawn.len() {
                        let (an, a) = drawn[i];
                        let (bn, b) = drawn[j];
                        assert!(
                            !a.intersects(b),
                            "{an} and {bn} overlap at {w}x{h} (tier {}):\n  {an} {a:?}\n  {bn} {b:?}",
                            l.tier.name()
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn the_title_bar_ends_where_the_body_begins() {
        // The first version started the body `GUTTER` below the *window* top regardless
        // of whether a title was drawn, so every ordinary window had the title bar and
        // the activity bar on top of each other.
        for h in [900.0, 600.0, 400.0] {
            let l = IdeLayout::new(avail(1440.0, h), Preferences::default());
            if l.title.height() > 0.0 {
                assert!(
                    l.activity.min.y >= l.title.max.y,
                    "at h={h} the body starts at {} while the title ends at {}",
                    l.activity.min.y,
                    l.title.max.y
                );
            }
        }
    }

    #[test]
    fn the_sidebar_folds_before_the_agent_panel_does() {
        // The order matters: the agent is the product, the file tree is a view.
        let narrow = IdeLayout::new(avail(MEDIUM - 1.0, 900.0), Preferences::default());
        assert_eq!(narrow.tier, Tier::NoSidebar);
        assert!(narrow.agent_visible && narrow.center_visible);

        let very_narrow = IdeLayout::new(avail(NARROW - 1.0, 900.0), Preferences::default());
        assert_eq!(very_narrow.tier, Tier::AgentOnly);
        assert!(very_narrow.agent_visible && !very_narrow.center_visible);
    }

    #[test]
    fn a_collapsed_region_is_zero_rather_than_a_hairline() {
        let l = IdeLayout::new(avail(700.0, 900.0), Preferences::default());
        assert!(!l.sidebar_visible);
        assert_eq!(l.sidebar, Rect::ZERO, "a hidden region must not draw");
        assert_eq!(l.activity, Rect::ZERO);
    }

    #[test]
    fn a_short_window_folds_the_status_bar_and_then_the_title_bar() {
        let short = IdeLayout::new(avail(1440.0, SHORT - 1.0), Preferences::default());
        assert_eq!(short.tier, Tier::CompactHeight);
        assert_eq!(short.status, Rect::ZERO);
        assert!(short.title.height() > 0.0, "the title survives first");

        let tiny = IdeLayout::new(avail(1440.0, COMPACT - 1.0), Preferences::default());
        assert_eq!(tiny.tier, Tier::ChromeOnly);
        assert_eq!(tiny.title, Rect::ZERO);
        assert!(tiny.agent.height() > 0.0, "the body still draws");
    }

    #[test]
    fn the_sidebar_is_clamped_to_the_range_that_stays_readable() {
        let prefs = Preferences {
            sidebar_width: 5.0,
            ..Preferences::default()
        };
        let l = IdeLayout::new(avail(1600.0, 900.0), prefs);
        assert!(
            (l.sidebar.width() - SIDEBAR_MIN).abs() < 0.01,
            "a 5pt sidebar is not readable: {:?}",
            l.sidebar
        );

        let prefs = Preferences {
            sidebar_width: 4000.0,
            ..Preferences::default()
        };
        let l = IdeLayout::new(avail(1600.0, 900.0), prefs);
        assert!(
            (l.sidebar.width() - SIDEBAR_MAX).abs() < 0.01,
            "a 4000pt sidebar eats the editor: {:?}",
            l.sidebar
        );
    }

    #[test]
    fn widening_the_window_widens_the_file_view_not_the_transcript() {
        // Both widths are chosen above `AGENT_DEFAULT` so the panel has reached its
        // comfortable size; below that it is still growing, which is a different rule.
        let narrow = IdeLayout::new(avail(1600.0, 900.0), Preferences::default());
        let wide = IdeLayout::new(avail(2600.0, 900.0), Preferences::default());
        assert_eq!(narrow.sidebar.width(), wide.sidebar.width());
        assert_eq!(
            narrow.agent.width(),
            wide.agent.width(),
            "the transcript must not grow with the window: {:?} vs {:?}",
            narrow.agent,
            wide.agent
        );
        assert_eq!(
            wide.center.width() - narrow.center.width(),
            1000.0,
            "all 1000 extra points should have gone to the editor: {:?} vs {:?}",
            narrow.center,
            wide.center
        );
    }

    #[test]
    fn a_window_too_small_for_both_panes_still_gives_each_one_something() {
        // Between `MEDIUM` and the point where both minimums fit, the split is shared
        // rather than one pane winning outright — two panes that do not overlap and are
        // both usable is the honest answer to a window that is too narrow.
        for w in [680.0, 700.0, 760.0, 820.0] {
            let l = IdeLayout::new(avail(w, 900.0), Preferences::default());
            if l.center_visible {
                assert!(
                    l.center.width() > 0.0 && l.agent.width() > 0.0,
                    "at {w} one pane collapsed: center {:?} agent {:?}",
                    l.center,
                    l.agent
                );
            }
        }
    }

    #[test]
    fn a_degenerate_window_does_not_panic_and_still_draws_something() {
        // A zero-sized or negative rect reaches here when a monitor is unplugged
        // mid-drag on Windows. It must not panic, and it must not draw a negative
        // width that the tessellator would reject.
        for (w, h) in [
            (0.0, 0.0),
            (1.0, 1.0),
            (-10.0, 900.0),
            (1440.0, -10.0),
            (-5.0, -5.0),
        ] {
            let l = IdeLayout::new(avail(w, h), Preferences::default());
            for (name, r) in [
                ("title", l.title),
                ("activity", l.activity),
                ("sidebar", l.sidebar),
                ("center", l.center),
                ("agent", l.agent),
                ("status", l.status),
            ] {
                assert!(
                    r.width() >= 0.0 && r.height() >= 0.0,
                    "{name} {r:?} is negative at {w}x{h}"
                );
            }
        }
    }

    #[test]
    fn a_preference_to_hide_the_sidebar_is_honoured_at_every_size() {
        let prefs = Preferences {
            sidebar_visible: false,
            ..Preferences::default()
        };
        for w in [2400.0, 1400.0, 900.0, 700.0, 600.0] {
            let l = IdeLayout::new(avail(w, 900.0), prefs);
            assert!(!l.sidebar_visible, "at {w} the sidebar came back uninvited");
            assert_eq!(l.sidebar, Rect::ZERO);
        }
    }

    // ---- the handle, and the store behind it --------------------------------------------

    /// **The handle is the gutter, and it overlaps nothing that is drawn.**
    ///
    /// A strip wider than [`GUTTER`] would reach into the sidebar's file rows and the
    /// editor's text, and the reader would lose the last few points of both. So this asserts
    /// the *overlap* rather than the width alone: the layout tests above already know how to
    /// check that two rects share no space, and the handle is now one of the rects a frame
    /// puts under a pointer.
    #[test]
    fn the_resize_handle_sits_in_the_gutter_and_overlaps_no_pane() {
        for w in [2600.0, 1600.0, 1440.0, 1000.0, 900.0] {
            let l = IdeLayout::new(avail(w, 900.0), Preferences::default());
            assert!(
                l.sidebar_visible,
                "at {w} the sidebar folded, so there is no handle"
            );
            assert_eq!(
                l.sidebar_handle.width(),
                GUTTER,
                "at {w} the handle must be exactly the gutter",
            );
            for (name, r) in [
                ("title", l.title),
                ("activity", l.activity),
                ("sidebar", l.sidebar),
                ("center", l.center),
                ("agent", l.agent),
                ("status", l.status),
            ] {
                if r.width() > 0.0 && r.height() > 0.0 {
                    // **Shrunk by a hair, and that is the whole point of it.** egui's
                    // `Rect::intersects` counts a *shared edge* as an intersection
                    // (`emath-0.36.2/src/rect.rs:250-255`) — the pane-overlap test above gets
                    // away with it only because `GUTTER` keeps every pair apart. The handle
                    // is supposed to share its two edges, with the sidebar and with the
                    // editor it sits between; what must not happen is reaching into either.
                    assert!(
                        !l.sidebar_handle.intersects(r.shrink(0.01)),
                        "at {w} the resize handle reaches into the {name} pane: {:?} vs {r:?}",
                        l.sidebar_handle
                    );
                }
            }
        }
    }

    /// **A sidebar that is not drawn has no handle, not a handle at a stale offset.**
    ///
    /// The same rule every other hidden region follows above, and the same reason: a caller
    /// that forgot to check `sidebar_visible` gets nothing rather than a control over an
    /// editor that is not there.
    #[test]
    fn a_folded_sidebar_publishes_no_handle() {
        let collapsed = IdeLayout::new(
            avail(1440.0, 900.0),
            Preferences {
                sidebar_visible: false,
                ..Preferences::default()
            },
        );
        assert_eq!(collapsed.sidebar_handle, Rect::ZERO);
        // And at a width that folds it by itself, which is a different reason to be absent.
        let narrow = IdeLayout::new(avail(MEDIUM - 1.0, 900.0), Preferences::default());
        assert_eq!(narrow.sidebar_handle, Rect::ZERO);
    }

    /// **A layout that goes out and comes back is the layout that went in.**
    ///
    /// The store's whole job, and the test `ide.app`'s relaunch test builds on: it asserts
    /// the two fields and nothing else, so a store that quietly dropped one of them — or
    /// rounded the other into a range the reader never chose — fails here.
    #[test]
    fn a_layout_comes_back_exactly_as_it_went_in() {
        let dir = tempfile::tempdir().expect("a settings directory");
        let ws = tempfile::tempdir().expect("a workspace");
        let prefs = Preferences {
            sidebar_width: 331.5,
            sidebar_visible: false,
        };
        let path = prefs.save_in(dir.path(), ws.path()).expect("save");
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .expect("a file name")
            .to_string();
        assert!(
            name.ends_with(".json") && name.contains('-'),
            "the file is this window's own, named after the workspace so one window's layout \
             is not another's: {name}"
        );
        assert_eq!(
            path.parent(),
            Some(dir.path()),
            "and it sits directly in the config directory beside `auth.json` and \
             `config.toml`, rather than inside the shared config file: {}",
            path.display()
        );
        assert_eq!(Preferences::load_in(dir.path(), ws.path()), prefs);

        // And the second field on its own, because a boolean is the field most likely to be
        // forgotten by a writer that only remembers about numbers.
        let other = Preferences {
            sidebar_width: 200.0,
            sidebar_visible: true,
        };
        other.save_in(dir.path(), ws.path()).expect("save again");
        assert_eq!(Preferences::load_in(dir.path(), ws.path()), other);
    }

    /// **Two workspaces do not share one layout, and one workspace always finds its own.**
    ///
    /// The property behind [`settings_path`]'s per-workspace name, and it is a *product*
    /// property as much as a test-hygiene one: a monorepo's file tree wants a wide sidebar
    /// and a dotfiles repository's does not, so a reader who resized the window in the first
    /// must not find it already resized in the second.
    #[test]
    fn each_workspace_keeps_its_own_layout() {
        let dir = tempfile::tempdir().expect("a settings directory");
        let one = tempfile::tempdir().expect("the first workspace");
        let two = tempfile::tempdir().expect("the second workspace");
        Preferences {
            sidebar_width: 400.0,
            sidebar_visible: true,
        }
        .save_in(dir.path(), one.path())
        .expect("save the first");
        assert_eq!(
            Preferences::load_in(dir.path(), two.path()),
            Preferences::default(),
            "the second workspace has never been resized, and inheriting the first one's \
             width is the bug this naming exists to prevent"
        );
        assert_eq!(
            Preferences::load_in(dir.path(), one.path()).sidebar_width,
            400.0,
            "and the first still finds its own."
        );
    }

    /// **A settings file that cannot be read gives the defaults, and does not fail.**
    ///
    /// Every way it can go wrong, one at a time: absent, unparseable, truncated mid-write,
    /// the right shape with the wrong types, and missing one of the two keys. `AGENTS.md`
    /// §2.6 is about a refusal path that does not end the process, and a window that would
    /// not open over a pane width is that failure at its worst.
    #[test]
    fn a_settings_file_that_cannot_be_read_gives_the_defaults() {
        let dir = tempfile::tempdir().expect("a settings directory");
        let ws = tempfile::tempdir().expect("a workspace");
        let path = settings_path(dir.path(), ws.path());

        // Absent — the ordinary first launch.
        assert_eq!(
            Preferences::load_in(dir.path(), ws.path()),
            Preferences::default()
        );

        for (what, body) in [
            ("not json at all", "this is not json"),
            ("truncated mid-write", "{\n  \"sidebar_width\": 30"),
            ("a json value rather than an object", "[1, 2, 3]"),
            (
                "the wrong types",
                r#"{"sidebar_width": "wide", "sidebar_visible": true}"#,
            ),
            ("one key short", r#"{"sidebar_width": 300.0}"#),
        ] {
            std::fs::write(&path, body).expect("write a broken file");
            assert_eq!(
                Preferences::load_in(dir.path(), ws.path()),
                Preferences::default(),
                "{what} must not stop the window opening, and must not half-apply: {body}"
            );
        }
    }

    /// **A stored width outside the readable range is clamped on the way in, not on the way
    /// out.**
    ///
    /// A file hand-edited to `9999` — or written by a build with a wider range — must not put
    /// a sidebar on screen that has eaten the editor until the reader happens to drag it.
    /// Clamping at load makes the in-memory value honest, and `IdeLayout::new` clamps again
    /// because a `Preferences` can also be built in a test.
    #[test]
    fn a_stored_width_outside_the_readable_range_lands_inside_it() {
        let dir = tempfile::tempdir().expect("a settings directory");
        let ws = tempfile::tempdir().expect("a workspace");
        for (stored, expected) in [
            (9999.0, SIDEBAR_MAX),
            (1.0, SIDEBAR_MIN),
            (-40.0, SIDEBAR_MIN),
        ] {
            std::fs::write(
                settings_path(dir.path(), ws.path()),
                format!(r#"{{"sidebar_width": {stored}, "sidebar_visible": true}}"#),
            )
            .expect("write");
            let read = Preferences::load_in(dir.path(), ws.path());
            assert_eq!(read.sidebar_width, expected, "stored {stored}");
            assert_eq!(read.sidebar_width_clamped(), expected);
        }
    }

    /// **Saving keeps the keys this build does not own.**
    ///
    /// The round-trip `config::save_model_id_in` does through a `toml::Table`, for the same
    /// reason: a build that knows more must not lose what this one wrote, and a future field
    /// added to this struct must not need a migration to survive a downgrade.
    #[test]
    fn a_save_keeps_the_keys_it_does_not_own() {
        let dir = tempfile::tempdir().expect("a settings directory");
        let ws = tempfile::tempdir().expect("a workspace");
        let path = settings_path(dir.path(), ws.path());
        std::fs::write(
            &path,
            r#"{"sidebar_width": 300.0, "sidebar_visible": true, "written_by": "a later build"}"#,
        )
        .expect("write");
        Preferences {
            sidebar_width: 280.0,
            sidebar_visible: true,
        }
        .save_in(dir.path(), ws.path())
        .expect("save");
        let raw = std::fs::read_to_string(&path).expect("read back");
        assert!(
            raw.contains("a later build"),
            "an unknown key was dropped on save: {raw}"
        );
        assert_eq!(
            Preferences::load_in(dir.path(), ws.path()).sidebar_width,
            280.0
        );
    }
}
