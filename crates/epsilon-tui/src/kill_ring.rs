//! Editor v2 primitives (M16b): the Emacs kill-ring (pi `kill-ring.ts` port) and
//! atomic paste markers (pi `editor.ts` `[paste #N +L lines]` convention).
//!
//! Pure functions/types only; the App wires them onto its buffer like every other
//! editor op.

/// Ring buffer of killed text. Consecutive kills accumulate into one entry
/// (Emacs rule: Ctrl+W Ctrl+W extends the previous kill rather than stacking).
pub struct KillRing {
    ring: Vec<String>,
    max: usize,
}

/// Depth cap (handoff requirement: ≥10).
const KILL_RING_MAX: usize = 16;

impl KillRing {
    /// Empty ring.
    pub fn new() -> Self {
        Self {
            ring: Vec::new(),
            max: KILL_RING_MAX,
        }
    }

    /// Push killed text. `accumulate` merges with the most recent entry instead of
    /// creating a new one; `prepend` controls merge order for backward kills.
    pub fn push(&mut self, text: String, accumulate: bool, prepend: bool) {
        if text.is_empty() {
            return;
        }
        if accumulate && let Some(last) = self.ring.pop() {
            let merged = if prepend { text + &last } else { last + &text };
            self.ring.push(merged);
        } else {
            self.ring.push(text);
        }
        if self.ring.len() > self.max {
            // Drop the OLDEST entries beyond capacity.
            let overflow = self.ring.len() - self.max;
            self.ring.drain(..overflow);
        }
    }

    /// Most recent entry without consuming it.
    pub fn peek(&self) -> Option<&String> {
        self.ring.last()
    }

    /// Rotate (pi/Emacs yank-pop): move last to front so repeated Ctrl+Y cycles.
    pub fn rotate(&mut self) {
        if self.ring.len() > 1 {
            let last = self.ring.pop().unwrap();
            self.ring.insert(0, last);
        }
    }

    /// Number of live entries.
    pub fn len(&self) -> usize {
        self.ring.len()
    }

    /// True when nothing has been killed yet.
    pub fn is_empty(&self) -> bool {
        self.ring.is_empty()
    }
}

impl Default for KillRing {
    fn default() -> Self {
        Self::new()
    }
}

// ---- paste markers ---------------------------------------------------------------

/// Marker prefix used in the buffer; matches pi's editor convention.
pub const MARKER_PREFIX: &str = "[paste #";

/// True when `segment` is exactly one paste marker (`[paste #N]`,
/// `[paste #N +L lines]`, `[paste #N C chars]`). Mirrors pi's single-segment test.
pub fn is_paste_marker(segment: &str) -> bool {
    let Some(body) = segment
        .strip_prefix(MARKER_PREFIX)
        .and_then(|r| r.strip_suffix(']'))
    else {
        return false;
    };
    let mut parts = body.splitn(2, ' ');
    let id = parts.next().unwrap_or("");
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    match parts.next() {
        None => true,
        Some(d) => {
            // "+L lines" or "C chars" with digit heads.
            (d.starts_with('+')
                && d.ends_with(" lines")
                && d[1..d.len() - 6].chars().all(|c| c.is_ascii_digit()))
                || (d.ends_with(" chars")
                    && d[..d.len() - 6].chars().all(|c| c.is_ascii_digit())
                    && !d[..d.len() - 6].is_empty())
        }
    }
}

/// Build a marker label from pasted content (pi editor.ts lines 21–45 shape):
/// multi-line pastes count lines, single-line big pastes count chars.
pub fn make_paste_marker(id: usize, text: &str) -> String {
    let line_count = text.lines().count();
    if line_count > 1 {
        format!("[paste #{id} +{line_count} lines]")
    } else if text.chars().count() > 40 {
        format!("[paste #{id} {} chars]", text.chars().count())
    } else {
        text.to_string()
    }
}

/// Expand markers back into their stored payloads on submit.
///
/// `payloads` maps marker id → original pasted text. Unknown ids expand to empty
/// string (defensive; shouldn't happen because ids are session-local).
pub fn expand_markers(
    buffer: &str,
    payloads: &std::collections::BTreeMap<usize, String>,
) -> String {
    let mut out = String::with_capacity(buffer.len());
    let bytes = buffer.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'['
            && let Some(end_rel) = buffer[i..].find(']')
        {
            let candidate = &buffer[i..=i + end_rel];
            if is_paste_marker(candidate) {
                let body = &candidate[MARKER_PREFIX.len()..candidate.len() - 1];
                let id: usize = body
                    .split_whitespace()
                    .next()
                    .and_then(|d| d.parse().ok())
                    .unwrap_or(0);
                out.push_str(payloads.get(&id).map(String::as_str).unwrap_or(""));
                i += candidate.len();
                continue;
            }
        }
        let ch_len = buffer[i..].chars().next().map(char::len_utf8).unwrap_or(1);
        out.push_str(&buffer[i..i + ch_len]);
        i += ch_len;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor_ops;

    #[test]
    fn kill_ring_pushes_accumulates_and_caps_depth() {
        let mut kr = KillRing::new();
        assert!(kr.is_empty());
        kr.push("alpha ".into(), false, false);
        kr.push("beta ".into(), true, false); // forward accumulation appends
        assert_eq!(kr.peek(), Some(&"alpha beta ".to_string()));
        kr.push("pre-".into(), true, true); // backward accumulation prepends
        assert_eq!(kr.peek(), Some(&"pre-alpha beta ".to_string()));

        for i in 0..32 {
            kr.push(format!("x{i}"), false, false);
        }
        assert_eq!(kr.len(), KILL_RING_MAX, "depth capped");
    }

    #[test]
    fn kill_ring_rotate_cycles_entries() {
        let mut kr = KillRing::new();
        kr.push("a".into(), false, false);
        kr.push("b".into(), false, false);
        kr.rotate();
        assert_eq!(kr.peek(), Some(&"a".to_string()), "last moved to front");
        kr.rotate();
        assert_eq!(kr.peek(), Some(&"b".to_string()));
    }

    #[test]
    fn paste_markers_round_trip_through_expand() {
        let mut payloads = std::collections::BTreeMap::new();
        payloads.insert(1, "line1\nline2\nline3".to_string());
        let marker = make_paste_marker(1, "line1\nline2\nline3");
        assert_eq!(marker, "[paste #1 +3 lines]");
        assert!(is_paste_marker(&marker));

        let expanded = expand_markers(&format!("before {marker} after"), &payloads);
        assert_eq!(expanded, "before line1\nline2\nline3 after");
    }

    #[test]
    fn char_count_markers_for_long_single_lines() {
        let long = "x".repeat(120);
        let marker = make_paste_marker(2, &long);
        assert_eq!(marker, "[paste #2 120 chars]");
        assert!(is_paste_marker(&marker));
    }

    #[test]
    fn short_text_inserts_verbatim_no_marker() {
        assert_eq!(make_paste_marker(3, "short"), "short");
    }

    #[test]
    fn non_marker_brackets_pass_through_expand() {
        let payloads = std::collections::BTreeMap::new();
        let out = expand_markers("array[0] and [not a marker]", &payloads);
        assert_eq!(out, "array[0] and [not a marker]");
    }

    #[test]
    fn word_nav_treats_marker_as_one_unit() {
        // The whole point of atomic markers: navigation over the marker text lands
        // at its start as one "word".
        let buf = "explain [paste #4 +12 lines] please";
        let end = buf.chars().count();
        let after_please = editor_ops::word_left(buf, end);
        let before_marker = editor_ops::word_left_atomic(buf, after_please);
        let marker_start = buf.find("[paste").unwrap();
        assert_eq!(
            before_marker, marker_start,
            "Ctrl+W from past the marker jumps its full length in one step"
        );
        // And forward nav clears the whole marker in one step too.
        assert_eq!(
            editor_ops::word_right_atomic(buf, marker_start),
            buf.find("] ").unwrap() + 1,
            "word_right_atomic lands just past the closing bracket"
        );
    }
}
