//! Text that is safe to *show*.
//!
//! # Why this exists
//!
//! A coding agent displays text it did not write: tool arguments, command lines, file
//! contents, model output, and a session replayed from disk. Two classes of character in
//! that text are dangerous in a way ordinary escaping does not cover.
//!
//! **Control characters** are already handled — ratatui drops them, so an escape sequence
//! in tool output cannot drive the terminal.
//!
//! **Bidirectional overrides are not.** `U+202E` (RIGHT-TO-LEFT OVERRIDE) reverses the
//! visual order of everything after it, so a command can be *displayed* as one thing and
//! *be* another. In a harness where a reader approves a command after reading it, that is
//! not a rendering bug; it is the approval prompt lying. It costs eleven characters.
//!
//! ```
//! // "delete" with RLO around "px" displays as "dpxelete" or worse, while the bytes run
//! // the other way. Stripping the override makes the line say what it does.
//! ```
//!
//! The characters stripped here are `U+202A`–`U+202E` (the embedding/override family),
//! `U+2066`–`U+2069` (isolates), the directional marks `U+200E`/`U+200F`/`U+061C`, and every
//! C0/C1 control except tab, newline and carriage return.
//!
//! # What this does *not* do
//!
//! **It does not delete what the tool actually received.** The session log keeps the exact
//! bytes; this only governs what the window paints. Stripping in the transcript does not
//! change a command, an argument, or a file. The line also reports *how many* characters
//! were removed, so the reader is told rather than left to assume the text was whole —
//! a silent strip is this repository's own recurring failure, and it would be the same
//! failure wearing a security hat.

use std::borrow::Cow;

/// Characters that steer display without contributing a glyph.
///
/// Split into "bidi" and "control" only so the count reported back to the reader can say
/// which happened; both are removed for the same reason and with the same consequence.
fn is_bidi(c: char) -> bool {
    matches!(c,
        '\u{202A}'..='\u{202E}'   // LRE, RLE, PDF, LRO, RLO
        | '\u{2066}'..='\u{2069}' // LRI, RLI, FSI, PDI
        | '\u{200E}' | '\u{200F}' // LRM, RLM
        | '\u{061C}') // ARABIC LETTER MARK
}

/// A control character that is not layout.
///
/// Tab, newline and carriage return are layout. Everything else below `U+0020`, and
/// everything in the C1 block, is either an escape the terminal might honour or a byte
/// that renders as nothing.
fn is_hidden_control(c: char) -> bool {
    c.is_control() && !matches!(c, '\t' | '\n' | '\r')
}

/// How much was removed, so the reader can be told.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Stripped {
    /// Bidirectional overrides and marks.
    pub bidi: usize,
    /// Other control characters.
    pub control: usize,
}

impl Stripped {
    /// Whether anything was removed.
    pub fn any(&self) -> bool {
        self.bidi + self.control > 0
    }

    /// The total removed.
    pub fn total(&self) -> usize {
        self.bidi + self.control
    }
}

/// Strip display-steering characters, borrowing when there is nothing to do.
///
/// The `Cow::Borrowed` fast path is the point: the overwhelming majority of lines contain
/// none of these, and a transcript of two thousand rows must not allocate two thousand
/// `String`s per frame to learn that nothing needed removing.
pub fn display_safe(text: &str) -> (Cow<'_, str>, Stripped) {
    let mut stripped = Stripped::default();
    for c in text.chars() {
        if is_bidi(c) {
            stripped.bidi += 1;
        } else if is_hidden_control(c) {
            stripped.control += 1;
        }
    }
    if stripped.bidi == 0 && stripped.control == 0 {
        return (Cow::Borrowed(text), stripped);
    }
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if is_bidi(c) || is_hidden_control(c) {
            continue;
        }
        out.push(c);
    }
    (Cow::Owned(out), stripped)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_is_borrowed_and_untouched() {
        let (text, stripped) = display_safe("fn main() {}");
        assert!(
            matches!(text, Cow::Borrowed(_)),
            "the common path must not allocate"
        );
        assert_eq!(text, "fn main() {}");
        assert!(!stripped.any());
    }

    /// The property that matters: what is drawn is what the command does.
    #[test]
    fn a_right_to_left_override_cannot_reorder_what_the_reader_is_shown() {
        // "ls" then RLO then " -R /" : visually the tail is reversed.
        let hostile = "ls\u{202E} -R /tmp";
        let (safe, stripped) = display_safe(hostile);
        assert!(
            !safe.contains('\u{202E}'),
            "the override must not reach the screen"
        );
        assert_eq!(stripped.bidi, 1);
        assert_eq!(
            safe, "ls -R /tmp",
            "and the remaining text is the real command"
        );
    }

    #[test]
    fn layout_characters_survive() {
        let (text, stripped) = display_safe("a\tb\nc\r\nd");
        assert_eq!(
            text, "a\tb\nc\r\nd",
            "tab and newline are layout, not smuggling"
        );
        assert!(!stripped.any());
    }

    #[test]
    fn other_control_characters_are_stripped_and_counted() {
        let (text, stripped) = display_safe("bell\u{7}end\u{1b}");
        assert_eq!(text, "bellend");
        assert_eq!(stripped.control, 2);
        assert_eq!(stripped.bidi, 0, "and they are not counted as bidi");
    }
}
