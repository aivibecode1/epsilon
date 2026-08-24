//! Pure editor operations over a `(String buffer, char-index cursor)` pair.
//!
//! Ported behaviors from pi/opencode editors: word-boundary jumps (unicode-aware via
//! `char` classification — alnum counts as word, everything else as separator),
//! kill-to-start (Ctrl+U), kill-word-backward (Ctrl+W), Home/End, forward delete.
//! The App wires these onto its `(input, cursor_chars)` state; keeping them pure
//! makes the tricky index math unit-testable.

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
    i
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
    i
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
pub fn delete_forward(s: &str, cursor: usize) -> (String, usize) {
    let chars: Vec<char> = s.chars().collect();
    if cursor >= chars.len() {
        return (s.to_string(), cursor);
    }
    let kept: String = chars
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != cursor)
        .map(|(_, c)| c)
        .collect();
    (kept, cursor)
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
}
