//! Pure editor operations over a `(String buffer, char-index cursor)` pair.
//!
//! Ported behaviors from pi/opencode editors: word-boundary jumps (unicode-aware via
//! `char` classification — alnum counts as word, everything else as separator),
//! kill-to-start (Ctrl+U), kill-word-backward (Ctrl+W), Home/End, forward delete.
//! The App wires these onto its `(input, cursor_chars)` state; keeping them pure
//! makes the tricky index math unit-testable.
//!
//! M36 grapheme rule: cursor positions are `char` indices, but every position a
//! function RETURNS is snapped to an extended-grapheme-cluster boundary (pi's
//! `Intl.Segmenter` parity via `unicode-segmentation`). Backspace and forward
//! delete consume a whole cluster, so ZWJ families, flags, and combining marks
//! can never be split — the pre-M36 `char`-stepping corrupted them.

use unicode_segmentation::UnicodeSegmentation;

/// Char indices of every grapheme-cluster start in `s`, with `len` as sentinel.
/// Cluster boundaries always coincide with `char` boundaries, so the mapping is exact.
fn cluster_starts(s: &str) -> Vec<usize> {
    let char_starts: Vec<usize> = s.char_indices().map(|(i, _)| i).collect();
    let mut bounds = vec![0];
    for (byte, _) in s.grapheme_indices(true) {
        if byte == 0 {
            continue;
        }
        if let Ok(pos) = char_starts.binary_search(&byte) {
            bounds.push(pos);
        }
    }
    bounds.push(s.chars().count());
    bounds
}

/// Largest cluster boundary at or before `cursor` (snaps a mid-cluster position
/// down to its cluster start; already-valid positions pass through).
pub fn floor_boundary(s: &str, cursor: usize) -> usize {
    let bounds = cluster_starts(s);
    bounds
        .iter()
        .rev()
        .find(|b| **b <= cursor)
        .copied()
        .unwrap_or(0)
}

/// Smallest cluster boundary at or after `cursor`.
pub fn ceil_boundary(s: &str, cursor: usize) -> usize {
    let bounds = cluster_starts(s);
    bounds
        .iter()
        .find(|b| **b >= cursor)
        .copied()
        .unwrap_or_else(|| s.chars().count())
}

/// Char index of the start of the cluster left of `cursor` (0 when none).
/// Backspace and Left-arrow consume/move exactly one cluster.
pub fn prev_cluster_start(s: &str, cursor: usize) -> usize {
    let bounds = cluster_starts(s);
    bounds
        .iter()
        .rev()
        .find(|b| **b < cursor)
        .copied()
        .unwrap_or(0)
}

/// Char index just past the end of the cluster at/after `cursor` (`len` at end).
/// Forward delete and Right-arrow consume/move exactly one cluster.
pub fn next_cluster_end(s: &str, cursor: usize) -> usize {
    let len = s.chars().count();
    let bounds = cluster_starts(s);
    bounds.iter().find(|b| **b > cursor).copied().unwrap_or(len)
}

/// Move `cursor` to the start of the previous word (or 0).
pub fn word_left(s: &str, cursor: usize) -> usize {
    let chars: Vec<char> = s.chars().collect();
    let mut i = cursor.min(chars.len());
    // Skip whitespace immediately left of the cursor.
    while i > 0 && chars[i - 1].is_whitespace() {
        i -= 1;
    }
    // Then skip the word itself.
    let in_word = |c: char| c.is_alphanumeric() || c == '_';
    if i > 0 && in_word(chars[i - 1]) {
        while i > 0 && in_word(chars[i - 1]) {
            i -= 1;
        }
    } else {
        // Single non-word char steps back one.
        i = i.saturating_sub(1);
    }
    // M36: the punct step can land inside a cluster (ZWJ, combining mark) —
    // snap down to the cluster start.
    floor_boundary(s, i)
}

/// Move `cursor` past the end of the next word (or len).
pub fn word_right(s: &str, cursor: usize) -> usize {
    let chars: Vec<char> = s.chars().collect();
    let mut i = cursor.min(chars.len());
    let in_word = |c: char| c.is_alphanumeric() || c == '_';
    // Skip whitespace.
    while i < chars.len() && chars[i].is_whitespace() {
        i += 1;
    }
    if i >= chars.len() {
        return i;
    }
    if in_word(chars[i]) {
        while i < chars.len() && in_word(chars[i]) {
            i += 1;
        }
    } else {
        // Punctuation run moves as one unit.
        while i < chars.len() && !in_word(chars[i]) && !chars[i].is_whitespace() {
            i += 1;
        }
    }
    // M36: punct runs can end mid-cluster — snap up to the cluster end.
    ceil_boundary(s, i)
}

/// Delete from start-of-line to `cursor`.
pub fn kill_to_start(s: &str, cursor: usize) -> (String, usize) {
    let kept: String = s.chars().skip(cursor).collect();
    (kept, 0)
}

/// Delete the word before `cursor` (Ctrl+W). Returns (buffer, new_cursor).
pub fn kill_word_back(s: &str, cursor: usize) -> (String, usize) {
    let start = word_left(s, cursor);
    let chars: Vec<char> = s.chars().collect();
    let kept: String = chars[..start]
        .iter()
        .chain(chars[cursor..].iter())
        .collect();
    (kept, start)
}

/// Delete the character under `cursor` (forward delete). No-op at end of line.
/// M36: consumes the whole grapheme cluster — never splits ZWJ/combining sequences.
pub fn delete_forward(s: &str, cursor: usize) -> (String, usize) {
    let chars: Vec<char> = s.chars().collect();
    if cursor >= chars.len() {
        return (s.to_string(), cursor);
    }
    let from = floor_boundary(s, cursor);
    let to = next_cluster_end(s, cursor);
    let kept: String = chars
        .iter()
        .enumerate()
        .filter(|(i, _)| *i < from || *i >= to)
        .map(|(_, c)| c)
        .collect();
    (kept, from)
}

// ---- M16b: atomic-segment navigation -------------------------------------------

/// Like [`word_left`] but a paste marker counts as ONE unit (pi's
/// `isAtomicSegment`; Ctrl+W removes `[paste #4 +12 lines]` whole).
pub fn word_left_atomic(s: &str, cursor: usize) -> usize {
    let chars: Vec<char> = s.chars().collect();
    let mut i = cursor.min(chars.len());
    while i > 0 && chars[i - 1].is_whitespace() {
        i -= 1;
    }
    // If text immediately left of the (whitespace-skipped) cursor ends with a
    // complete marker, jump its full length in one step.
    let before: String = chars[..i].iter().collect();
    if before.ends_with(']')
        && let Some(open) = before.rfind("[paste #")
        && crate::kill_ring::is_paste_marker(&before[open..])
    {
        return open;
    }
    word_left(s, cursor)
}

/// Like [`word_right`] but a paste marker counts as ONE unit.
pub fn word_right_atomic(s: &str, cursor: usize) -> usize {
    let chars: Vec<char> = s.chars().collect();
    let mut i = cursor.min(chars.len());
    while i < chars.len() && chars[i].is_whitespace() {
        i += 1;
    }
    let rest: String = chars[i.min(chars.len())..].iter().collect();
    if rest.starts_with("[paste #")
        && let Some(close_rel) = rest.find(']')
        && crate::kill_ring::is_paste_marker(&rest[..=close_rel])
    {
        return i + close_rel + 1;
    }
    word_right(s, cursor)
}

// ---- M17b: line-wise editing ops (opencode keybind parity) ----------------------

/// Index of the start of the line containing char position `cursor`.
/// Multi-line aware: stops at the `\n` left of the cursor.
pub fn line_home(s: &str, cursor: usize) -> usize {
    let chars: Vec<char> = s.chars().collect();
    let mut i = cursor.min(chars.len());
    while i > 0 && chars[i - 1] != '\n' {
        i -= 1;
    }
    // M36: `\r\n` is one cluster — never park between the two halves.
    floor_boundary(s, i)
}

/// Index just past the end of the line containing char position `cursor`
/// (the `\n` itself excluded). Multi-line aware.
pub fn line_end(s: &str, cursor: usize) -> usize {
    let chars: Vec<char> = s.chars().collect();
    let mut i = cursor.min(chars.len());
    while i < chars.len() && chars[i] != '\n' {
        i += 1;
    }
    // M36: stop before a `\r\n` pair, not between its halves.
    floor_boundary(s, i)
}

// ---- M17b: kill-to-line-end (Ctrl+K, opencode input_delete_to_line_end) ---------

/// Delete from `cursor` to end of line (multi-line aware: stops at `\n`).
/// Returns (buffer, new_cursor); the killed text is returned for the kill ring.
pub fn kill_to_line_end(s: &str, cursor: usize) -> (String, String, usize) {
    let chars: Vec<char> = s.chars().collect();
    let end = line_end(s, cursor);
    let killed: String = chars[cursor..end].iter().collect();
    let kept: String = chars[..cursor].iter().chain(chars[end..].iter()).collect();
    (kept, killed, cursor)
}

// ---- @-file completion (W1, pi editor.ts parity) -------------------------------

/// When `input` contains `@<fragment>` immediately left of `cursor_chars`, return
/// (byte_start_of_@, fragment_after_@). Only the last `@` considered; the range
/// must be `[\w./-]*` and stop at whitespace or start-of-line. None if no
/// `@`-word abuts the cursor. Used to trigger the at-file popup.
pub fn at_word_before(s: &str, cursor: usize) -> Option<(usize, String)> {
    let chars: Vec<char> = s.chars().collect();
    let at_byte = cursor.min(chars.len());
    if at_byte == 0 {
        return None;
    }
    // Find the nearest '@' left of the cursor that is not escaped and not mid-word
    // without a word-boundary: the char immediately after '@' (if any) up to cursor
    // must be [\w./-]* and no whitespace.
    let mut at_idx: Option<usize> = None;
    for i in (0..at_byte).rev() {
        let c = chars[i];
        if c == '@' {
            // preceding char must be start or whitespace
            if i == 0 || chars[i - 1].is_whitespace() {
                // every char after '@' up to cursor must be allowed
                let frag: String = chars[i + 1..at_byte].iter().collect();
                if frag
                    .chars()
                    .all(|ch| ch.is_alphanumeric() || "_./-".contains(ch))
                {
                    at_idx = Some(i);
                    break;
                }
                return None;
            }
            return None;
        }
        // Any char after @ must be allowed; but we scan backwards, so we
        // just check we haven't hit an invalid char without first hitting @.
        if !(c.is_alphanumeric() || "_./-".contains(c)) {
            return None;
        }
        if c.is_whitespace() {
            return None;
        }
    }
    let i = at_idx?;
    let frag: String = chars[i + 1..at_byte].iter().collect();
    // Return byte index of '@' (char index, not byte index) + fragment
    Some((i, frag))
}

/// Replace the `@<fragment>` span immediately left of `cursor` with `replacement`.
/// Returns (new_buffer, new_cursor_char_index). If no @-word abuts cursor,
/// returns the input unchanged.
pub fn at_insert(s: &str, cursor: usize, replacement: &str) -> (String, usize) {
    let Some((at_idx, _)) = at_word_before(s, cursor) else {
        return (s.to_string(), cursor);
    };
    let chars: Vec<char> = s.chars().collect();
    let before: String = chars[..at_idx].iter().collect();
    let after: String = chars[cursor..].iter().collect();
    let new_s = format!("{before}{replacement}{after}");
    let new_cursor = before.chars().count() + replacement.chars().count();
    (new_s, new_cursor)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn word_nav_respects_boundaries_and_unicode() {
        let s = "alpha beta_gamma  δέλτα";
        // End → jump back lands at start of δέλτα
        let end = s.chars().count();
        assert_eq!(word_left(s, end), 18); // start of δέλτα
        assert_eq!(word_left(s, 18), 6); // start of beta_gamma
        assert_eq!(word_left(s, 6), 0); // start of alpha
        assert_eq!(word_right(s, 0), 5);
        assert_eq!(word_right(s, 6), 16);
        assert_eq!(word_right(s, end), end);
    }

    #[test]
    fn kills_behave_and_preserve_remainder() {
        let (s, c) = kill_to_start("hello world", 6);
        assert_eq!((s.as_str(), c), ("world", 0));

        let (s, c) = kill_word_back("hello world", 11);
        assert_eq!((s.as_str(), c), ("hello ", 6));

        let (s, c) = delete_forward("abc", 1);
        assert_eq!((s.as_str(), c), ("ac", 1));
        let (s, c) = delete_forward("abc", 3);
        assert_eq!((s.as_str(), c), ("abc", 3), "no-op at EOL");
    }

    // M36: cursor movement and deletion move whole grapheme clusters — ZWJ
    // families, flags, and combining marks are never split.
    #[test]
    fn clusters_move_and_delete_whole() {
        // Family emoji: 5 chars (person + ZWJ + person + ZWJ + child), 1 cluster.
        let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";
        let s = format!("a{family}b");
        let fam_start = 1;
        let fam_end = 1 + family.chars().count();
        let end = fam_end + 1;

        assert_eq!(prev_cluster_start(&s, end), fam_end);
        assert_eq!(prev_cluster_start(&s, fam_end), fam_start);
        assert_eq!(prev_cluster_start(&s, fam_start), 0);
        assert_eq!(next_cluster_end(&s, 0), fam_start);
        assert_eq!(next_cluster_end(&s, fam_start), fam_end);
        assert_eq!(next_cluster_end(&s, fam_end), end);

        // Backspace-shaped delete removes the whole family, not one person.
        let (kept, c) = delete_forward(&s, fam_start);
        assert_eq!(kept, "ab");
        assert_eq!(c, fam_start);

        // Combining acute: e + U+0301 is one cluster.
        let acute = "cafe\u{301}";
        let len = acute.chars().count();
        assert_eq!(prev_cluster_start(acute, len), len - 2);
        assert_eq!(next_cluster_end(acute, 0), 1);

        // Regional-indicator flag: one cluster.
        let flag = "\u{1F1EB}\u{1F1F7}";
        assert_eq!(prev_cluster_start(flag, 2), 0);
        assert_eq!(next_cluster_end(flag, 0), 2);

        // Mid-cluster positions snap instead of parking inside.
        assert_eq!(floor_boundary(&s, fam_start + 2), fam_start);
        assert_eq!(ceil_boundary(&s, fam_start + 2), fam_end);
    }

    #[test]
    fn word_and_line_nav_snaps_off_cluster_interior() {
        let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";
        // Punctuation stepping over a ZWJ lands inside the cluster; word_left
        // must snap down to the cluster start instead.
        let s = format!("x {family}");
        let end = s.chars().count();
        assert_eq!(word_left(&s, end), 2);

        // CRLF is one cluster: line ends stop before the pair, never between.
        let crlf = "ab\r\ncd";
        assert_eq!(line_end(crlf, 0), 2);
        assert_eq!(line_home(crlf, 5), 4);
    }
}
