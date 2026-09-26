//! Finding text in the transcript you can see.
//!
//! ## What this replaced
//!
//! `/search <query>` scanned the **session log on disk** and printed its hits as transcript
//! notices:
//!
//! ```text
//! • 3 hits for 'error':
//! • #41 user: …the error is in the parser…
//! • #88 assistant: …three errors reported…
//! ```
//!
//! Which is a *list of places you could go*, printed into the one place you already were. The
//! view did not move, nothing was highlighted, the matches were prose rather than the rows on
//! screen, and the whole thing needed a session log attached \u2014 so it printed
//! "no session log attached (search unavailable this run)" in exactly the situations where a
//! user most wants it. pi searches the rendered screen
//! (`packages/tui/src/alt-screen-search.ts:22-30`), which is the thing a reader is actually
//! looking at.
//!
//! ## The scope, honestly
//!
//! This searches the **rendered transcript**, not the session log. That is a deliberate
//! trade: the log reaches further back than the viewport ever holds, and `/search` used to
//! find a message from fifty turns ago that has long since scrolled off. What it could not
//! do was show you the thing it found.
//!
//! The query is matched case-insensitively against each line's own text. There is no
//! incremental re-filter as you type, no next/previous scrollbar, and no match highlighting
//! inside the matched row yet. The three that matter \u2014 it moves the view, it says which
//! match you are on, and it works with no session attached \u2014 are here; the rest is the
//! obvious next slice rather than something half-built here.

/// A search over the rendered transcript.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TranscriptSearch {
    /// The text being looked for.
    pub query: String,
    /// Transcript **line** indices of the matching lines, ascending.
    pub matches: Vec<usize>,
    /// Which match is current. Always `< matches.len()` while the search is open.
    pub current: usize,
    /// The view offset to restore when the search closes.
    ///
    /// Navigation deliberately moves the view. Leaving it wherever the last match put it
    /// would mean closing the search is itself an action the user has to undo, and the
    /// alternative \u2014 snapping back to the tail \u2014 throws away the position they had
    /// scrolled to before they started looking.
    pub saved_scroll_from_bottom: usize,
}

impl TranscriptSearch {
    /// A search over `lines`, with the first match current.
    ///
    /// Returns `None` when the query is blank, so a caller can tell "nothing to search for"
    /// apart from "searched and found nothing" \u2014 which are different messages and used to
    /// look the same because both printed a notice.
    pub fn open(
        lines: &[impl AsRef<str>],
        query: &str,
        saved_scroll_from_bottom: usize,
    ) -> Option<Self> {
        let query = query.trim();
        if query.is_empty() {
            return None;
        }
        let needle = query.to_lowercase();
        let matches = lines
            .iter()
            .enumerate()
            .filter(|(_, l)| l.as_ref().to_lowercase().contains(&needle))
            .map(|(i, _)| i)
            .collect();
        Some(Self {
            query: query.to_string(),
            matches,
            current: 0,
            saved_scroll_from_bottom,
        })
    }

    /// Move to the next match, wrapping. Returns the line index, or `None` if there are none.
    ///
    /// Named `next_match` rather than `next`: a bare `next` on a type that is not an
    /// `Iterator` reads like one at the call site, and clippy is right to say so —
    /// `search.next()` looks like it is pulling from the match list and returns the line
    /// index instead.
    pub fn next_match(&mut self) -> Option<usize> {
        if self.matches.is_empty() {
            return None;
        }
        self.current = (self.current + 1) % self.matches.len();
        self.current_line()
    }

    /// Move to the previous match, wrapping. `None` if there are none.
    pub fn previous_match(&mut self) -> Option<usize> {
        if self.matches.is_empty() {
            return None;
        }
        self.current = (self.current + self.matches.len() - 1) % self.matches.len();
        self.current_line()
    }

    /// The transcript line the current match is on.
    pub fn current_line(&self) -> Option<usize> {
        self.matches.get(self.current).copied()
    }

    /// How many matches there are, and where in that list we are: `3 of 7`, or `no matches`.
    ///
    /// One of these, never both, never neither. A counter that reads `4 of 4` while the
    /// viewport is somewhere else is worse than no counter, so the two numbers are always
    /// produced together.
    pub fn position(&self) -> String {
        if self.matches.is_empty() {
            "no matches".to_string()
        } else {
            format!("{} of {}", self.current + 1, self.matches.len())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::TranscriptSearch;

    fn lines() -> Vec<String> {
        [
            "first line",
            "the error is here",
            "third",
            "another error",
            "fifth",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect()
    }

    #[test]
    fn a_search_finds_every_matching_line_in_order() {
        let s = TranscriptSearch::open(&lines(), "error", 0).expect("a non-empty query");
        assert_eq!(s.matches, vec![1, 3], "both errors, in transcript order");
    }

    #[test]
    fn matching_ignores_case_because_the_reader_does_not() {
        // A search that is case-sensitive finds "Error" when you typed "error" and tells you
        // there are no matches, which is indistinguishable from the text not being there.
        let upper: Vec<String> = vec!["Error in module".to_string()];
        let s = TranscriptSearch::open(&upper, "error", 0).expect("a match");
        assert_eq!(s.matches, vec![0]);
    }

    #[test]
    fn a_blank_query_is_not_a_search() {
        // `None` rather than a search with zero matches, so the caller can say "usage" and
        // "no matches" — different answers that used to look identical.
        assert!(TranscriptSearch::open(&lines(), "   ", 0).is_none());
    }

    #[test]
    fn a_query_with_no_match_is_a_search_with_no_matches() {
        let mut s = TranscriptSearch::open(&lines(), "absent", 0).expect("searchable");
        assert!(s.matches.is_empty());
        assert_eq!(s.position(), "no matches");
        assert_eq!(s.next_match(), None, "stepping an empty list cannot panic");
    }

    #[test]
    fn next_and_previous_wrap_rather_than_stop() {
        // Stopping at the end is right for a list you are editing and wrong for a search:
        // "find the next one" should never be the thing that fails.
        let mut s = TranscriptSearch::open(&lines(), "error", 0).expect("two matches");
        assert_eq!(s.next_match(), Some(3), "second match");
        assert_eq!(s.next_match(), Some(1), "wraps to the first");
        assert_eq!(s.previous_match(), Some(3), "and back again");
    }

    #[test]
    fn the_counter_says_where_you_are_and_how_many_there_are() {
        let mut s = TranscriptSearch::open(&lines(), "error", 0).expect("two matches");
        assert_eq!(s.position(), "1 of 2");
        s.next_match();
        assert_eq!(s.position(), "2 of 2");
    }

    #[test]
    fn closing_restores_the_view_the_user_came_from() {
        // A search that relocates the viewport and leaves it there makes *closing* the thing
        // you have to undo.
        let s = TranscriptSearch::open(&lines(), "error", 17).expect("two matches");
        assert_eq!(s.saved_scroll_from_bottom, 17);
    }
}
