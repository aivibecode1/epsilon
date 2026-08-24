//! Viewport math for selectable lists — a direct port of pi's `select-list` window
//! (`startIndex = clamp(selected - maxVisible/2, …)`, centered on the cursor), plus
//! the transcript follow-scroll used by the main pane.

/// Start offset of the visible window for `selected` within `total` items and a
/// viewport of `viewport` rows. Mirrors pi's select-list: keep the selection roughly
/// centered once the list overflows.
///
/// Panics-free by contract: any zero/nonsense input collapses to offset 0.
pub fn scroll_start(total: usize, selected: usize, viewport: usize) -> usize {
    if total == 0 || viewport == 0 {
        return 0;
    }
    let selected = selected.min(total - 1);
    if total <= viewport {
        return 0;
    }
    let max_start = total - viewport;
    let centered = selected.saturating_sub(viewport / 2);
    centered.min(max_start)
}

/// Transcript follow-scroll: pin an offset above the bottom when the user pages up,
/// clamp to valid range, and snap back to follow (0) when at/past the bottom.
pub fn transcript_skip(total_rows: usize, visible: usize, from_bottom: usize) -> usize {
    if total_rows <= visible {
        return 0;
    }
    let max = total_rows - visible;
    max.saturating_sub(from_bottom.min(max))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_lists_never_scroll() {
        assert_eq!(scroll_start(0, 0, 10), 0);
        assert_eq!(scroll_start(3, 2, 10), 0);
        assert_eq!(scroll_start(10, 9, 10), 0);
    }

    #[test]
    fn window_centers_selection_like_pi() {
        // 20 items, viewport 5: selecting item 10 → start ≈ 10-2 = 8
        assert_eq!(scroll_start(20, 10, 5), 8);
        // Near top: clamps to 0
        assert_eq!(scroll_start(20, 1, 5), 0);
        // Near bottom: clamps to max start
        assert_eq!(scroll_start(20, 19, 5), 15);
        // Selected beyond total is clamped, not panicking
        assert_eq!(scroll_start(20, 999, 5), 15);
    }

    #[test]
    fn transcript_follow_and_clamp() {
        assert_eq!(transcript_skip(50, 10, 0), 40, "follow mode pins the tail");
        assert_eq!(transcript_skip(50, 10, 5), 35);
        assert_eq!(
            transcript_skip(50, 10, 500),
            0,
            "deep overscroll shows the top"
        );
        assert_eq!(transcript_skip(5, 10, 3), 0, "short content never scrolls");
    }
}
