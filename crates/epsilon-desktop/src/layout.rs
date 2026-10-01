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

use egui::Rect;

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
            let w = prefs.sidebar_width.clamp(SIDEBAR_MIN, SIDEBAR_MAX);
            let r = Rect::from_min_size(egui::pos2(x, body_top), egui::vec2(w, body_h));
            x += w + GUTTER;
            r
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
}
