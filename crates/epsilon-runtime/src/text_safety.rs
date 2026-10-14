//! Text that is safe to *show*.
//!
//! **This module moved here from epsilon-tui (M272)**, beside [crate::summarize_request]
//! and [crate::goal_command]. It is a pure function over text with no terminal and no
//! app state in it, and the desktop window has the same need the terminal had: a
//! command carrying a display-steering character must not reach an approval screen
//! unstripped. Rules that live in a frontend are rules one frontend has, and that is
//! exactly how the two windows came to disagree about what a reader was being asked
//! to approve (A48).
//!
//! # Why this exists
//!
//! A coding agent displays text it did not write: tool arguments, command lines, file
//! contents, model output, and a session replayed from disk. Two classes of character in
//! that text are dangerous in a way ordinary escaping does not cover.
//!
//! **Control characters and control sequences are both handled.** A bare control byte is
//! dropped; a full CSI sequence is dropped *whole*, because a command that colours its output
//! prints `ESC [ 0;32 m` and stripping the escape alone leaves `[0;32m` on the card — printable
//! ASCII that no other rule catches. This is the same rule pi applies to tool output
//! (`stripAnsi`, `references/pi-1.0.0/packages/coding-agent/src/utils/ansi.ts:37`, called from
//! `modes/interactive/components/bash-execution.ts:83`), for the same reason: the reader is
//! shown what the command *did*, not what its escape sequences looked like.
//!
//! **A bare escape is not assumed to start a sequence.** The introducer is dropped on its own
//! when no whole sequence follows it, so a truncated or hand-edited sequence cannot swallow
//! the rest of the line — which would be a different way of hiding text from the reader.
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

/// Does this character introduce a control sequence?
///
/// `ESC` for the 7-bit form, `CSI` (`U+009B`) for the 8-bit form pi's own matcher also accepts.
fn is_csi_introducer(c: char) -> bool {
    c == '\u{1b}' || c == '\u{9b}'
}

/// Is this byte a CSI **final** byte, the one that terminates the sequence?
///
/// pi's class, from ansi-regex: `[\dA-PR-TZcf-nq-uy=><~]`
/// (`references/pi-1.0.0/packages/coding-agent/src/utils/ansi.ts:37`).
fn is_csi_final(b: u8) -> bool {
    b.is_ascii_digit()
        || matches!(
            b,
            b'A'..=b'P'
                | b'R'..=b'T'
                | b'Z'
                | b'c'
                | b'f'..=b'n'
                | b'q'..=b'u'
                | b'y'
                | b'='
                | b'>'
                | b'<'
                | b'~'
        )
}

/// How many bytes of `rest` a CSI sequence occupies, or `None` when no whole one is there.
///
/// `rest` starts *after* an introducer. The matcher covers every form in real use, because the
/// distinguisher here is not the colour but the shape:
///
/// | form | example | what the introducer class carries |
/// |---|---|---|
/// | 6-bit private | `ESC ( B`, `ESC [ ? 25 l` | `(`, `[`, `?` |
/// | multi-group params | `ESC [ 0;32 m`, `ESC [ 1;5H` | `;` groups of digits |
/// | bare two-char | `ESC c`, `ESC M` | final byte alone |
///
/// Without this, stripping the introducer leaves the parameters on screen as `[0;32m` — a
/// display artefact that is not a glyph anybody asked for and not a control character, so
/// nothing else removes it.
fn csi_suffix_len(rest: &str) -> Option<usize> {
    let b = rest.as_bytes();
    let mut i = 0usize;
    // Intermediate bytes: pi's `[\[\]()#;?]*`. `[` is one of them, which is why the bracket
    // that follows every introducer is consumed here rather than being treated as residue.
    while i < b.len() && matches!(b[i], b'[' | b']' | b'(' | b')' | b'#' | b';' | b'?') {
        i += 1;
    }
    // Parameters: pi's `\d{1,4}(?:[;:]\d{0,4})*` — a leading group of up to four digits, then
    // any number of `;`/`:` groups of up to four more. The whole group is optional, so a
    // sequence may go straight from intermediates to its final byte.
    let first_group = i;
    while i < b.len() && i - first_group < 4 && b[i].is_ascii_digit() {
        i += 1;
    }
    if i > first_group {
        while i < b.len() && matches!(b[i], b';' | b':') {
            let group = i + 1;
            let mut j = group;
            while j < b.len() && j - group < 4 && b[j].is_ascii_digit() {
                j += 1;
            }
            i = j;
        }
    }
    // The final byte is what makes it a sequence. Without one this is not a CSI, and only the
    // introducer is dropped — swallowing what follows would eat the rest of the line.
    if i < b.len() && is_csi_final(b[i]) {
        Some(i + 1)
    } else {
        None
    }
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
///
/// A control **sequence** is removed as a unit. Stripping its introducer and leaving the
/// rest — `[0;32m` out of `ESC [ 0;32 m` — is the defect this whole method exists to avoid:
/// the parameters are printable ASCII, so no other rule catches them, and they arrive on a
/// card as though the command had printed them.
pub fn display_safe(text: &str) -> (Cow<'_, str>, Stripped) {
    let mut stripped = Stripped::default();
    // Walk twice: the first pass only measures, so the common "nothing to remove" line is
    // answered without an allocation. The introducer's sequence bytes are printable ASCII, so
    // counting the sequence here is the only place they are accounted for.
    for (i, c) in text.char_indices() {
        if is_csi_introducer(c) {
            let rest = &text[i + c.len_utf8()..];
            stripped.control += 1 + csi_suffix_len(rest).unwrap_or(0);
            continue;
        }
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
    // Byte-indexed, because a sequence must be consumed in one step: a char-at-a-time walk
    // would push its parameter and final bytes straight back out.
    let mut i = 0usize;
    while i < text.len() {
        let Some(c) = text[i..].chars().next() else {
            break;
        };
        i += c.len_utf8();
        if is_csi_introducer(c) {
            i += csi_suffix_len(&text[i..]).unwrap_or(0);
            continue;
        }
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

    /// **A coloured line reaches the card without its escape residue.**
    ///
    /// `cargo build --color=always` and `grep --color=always` wrap real text in SGR sequences.
    /// Stripping the escape *alone* left the parameters behind — `[0;32m` — and they are
    /// printable ASCII, so no other rule in this module touches them: not bidi, not a control
    /// byte, not layout. They reached the card as though the compiler had printed them.
    ///
    /// The exact output is asserted, because the failure mode is *extra* text rather than
    /// missing text and only a whole-string comparison sees it. The shapes are pi's, which
    /// strips the whole sequence for exactly this reason
    /// (`references/pi-1.0.0/packages/coding-agent/src/utils/ansi.ts:37`, reached from bash
    /// output at `modes/interactive/components/bash-execution.ts:83`).
    #[test]
    fn a_coloured_line_reaches_the_card_without_its_escape_residue() {
        // A cargo line in green, reset at the end — the shape `--color=always` emits.
        let (text, stripped) =
            display_safe("\u{1b}[0;32m   Compiling epsilon-tui v0.1.0\u{1b}[0m\n");
        assert_eq!(
            text, "   Compiling epsilon-tui v0.1.0\n",
            "the line must arrive exactly as the compiler wrote it, minus the colour"
        );
        assert_eq!(
            stripped.control, 11,
            "the two sequences must be reported as removed, so the reader is told: \
             ESC[0;32m is 7 bytes and ESC[0m is 4"
        );

        // grep's match highlighting: two sequences, a private-form reset, and a cursor move.
        let grep_line = "\u{1b}[1;31mmatch\u{1b}[0m:12\u{1b}[K";
        let (text, _) = display_safe(grep_line);
        assert_eq!(
            text, "match:12",
            "grep's colouring must not survive: {text:?}"
        );
    }

    /// **The other half of the same rule: an escape that starts nothing.**
    ///
    /// A sequence matcher that swallows until it finds a final byte deletes the rest of the
    /// line — hiding text from the reader, which is the worse failure. So only the introducer
    /// is dropped when no whole sequence follows it, and the visible tail is kept.
    #[test]
    fn a_truncated_sequence_does_not_swallow_the_line() {
        // No final byte anywhere: `[` and the digits stay, the escape does not.
        let (text, _) = display_safe("\u{1b}[0;32");
        assert_eq!(
            text, "[0;32",
            "the incomplete sequence must not eat the line"
        );

        // A bare escape followed by ordinary text is still just an escape.
        let (text, _) = display_safe("a\u{1b}bc");
        assert_eq!(
            text, "abc",
            "no sequence, so nothing but the escape is removed"
        );
    }

    /// A sequence with a 6-bit or private introducer still goes: `ESC ( B`, `ESC [ ? 25 l`.
    #[test]
    fn a_private_or_six_bit_sequence_is_removed_whole() {
        let (text, _) = display_safe("\u{1b}(Bplain\u{1b}[?25l");
        assert_eq!(
            text, "plain",
            "charset and private-mode sequences must go: {text:?}"
        );
    }

    /// The 8-bit introducer is a real CSI and is treated as one.
    #[test]
    fn the_eight_bit_csi_introducer_is_handled() {
        let (text, stripped) = display_safe("\u{9b}0;32mgreen\u{9b}0m");
        assert_eq!(text, "green", "U+009B is the 8-bit CSI: {text:?}");
        assert_eq!(
            stripped.control, 9,
            "two sequences, introducer and all: {stripped:?} -- U+009B + 0;32m is 6, U+009B + 0m is 3"
        );
    }
}
