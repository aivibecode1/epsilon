//! Putting text on the user's clipboard — the local clipboard *and* the terminal's.
//!
//! # Why there are two paths
//!
//! `arboard` reaches the clipboard of the machine the process runs on. That is
//! the right answer locally and the wrong one over SSH: the harness runs on the
//! remote box, whose "display server" is usually absent, so the call fails — or,
//! worse, succeeds against a clipboard nobody is looking at. A `/copy` that
//! reports success while nothing reached the user's own machine is a lie the UI
//! told, and it is the failure mode the skill names ("the OS clipboard doesn't
//! exist over SSH… A 'yank' that only calls them fails silently… Emit OSC 52
//! too", `references/ai-harness.md`).
//!
//! OSC 52 (`ESC ] 52 ; c ; <base64> BEL`) is the other path: a request the
//! *terminal* forwards to the clipboard of the machine the user is sitting at,
//! which is the only thing that still works across an SSH hop.
//!
//! # What this module can and cannot promise
//!
//! It cannot promise that OSC 52 worked. The sequence travels to the terminal
//! over the same pty as the frame, and a terminal that does not implement OSC 52
//! — or has it switched off, as iTerm2 does by default — discards it in silence;
//! there is no reply to wait for and no way to ask. So the two outcomes are kept
//! apart in the type system ([`Path`]) rather than collapsed into a bool: a
//! native write is something this process did, and an OSC 52 write is something
//! it asked the terminal to do. [`copy`] returns which, and [`notice`] says
//! which. That is why the terminal sentence says "sent" and not "copied".
//!
//! # Size
//!
//! Terminals cap OSC 52 payloads, and the caps are not discoverable: xterm's is a
//! configured maximum, tmux's defaults to 4 KiB, others are smaller still. What a
//! terminal does with an over-long sequence varies as much as the cap —
//! truncation is the benign case, and some discard the sequence *and* the bytes
//! around it, which shows up as a torn frame. So an over-long payload is not sent
//! at all: [`MAX_ENCODED`] is pi's ceiling (`utils/clipboard.ts:20`), and a longer
//! message reports the path that did not work rather than half of it.

use std::io::Write;

use base64::engine::Engine as _;
use base64::engine::general_purpose::STANDARD;

/// Longest base64 payload worth sending. pi's `MAX_OSC52_ENCODED_LENGTH`
/// (`packages/coding-agent/src/utils/clipboard.ts:20`), for the reason in the
/// module docs: past this the sequence is as likely to be mangled as delivered.
pub const MAX_ENCODED: usize = 100_000;

/// The reason a copy could not be completed, in words the notice can print.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// Text too long to hand to the terminal, and no local clipboard to fall
    /// back on. See [`MAX_ENCODED`].
    TooLong,
    /// The terminal write itself failed (a closed stdout, a full pipe).
    Write(String),
    /// No local clipboard, and nothing was sent to the terminal for some other
    /// reason.
    NoPath,
}

impl Refusal {
    /// The parenthetical the notice prints.
    fn detail(&self) -> String {
        match self {
            Self::TooLong => "too long to send to the terminal".to_string(),
            Self::Write(e) => format!("writing to the terminal failed: {e}"),
            Self::NoPath => "no local clipboard".to_string(),
        }
    }
}

/// Which clipboard actually received the text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Path {
    /// `arboard` placed it on this machine's clipboard. Verified as far as the
    /// crate can verify it: the call returned `Ok`.
    Local,
    /// The OSC 52 sequence was written for the terminal to forward. Sent, not
    /// confirmed — see the module docs.
    Terminal,
    /// Neither path placed the text anywhere.
    None,
}

/// The outcome of one copy: which path was taken, and why nothing was if
/// nothing was.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Copied {
    /// The path that landed.
    pub path: Path,
    /// What went wrong, when no path landed.
    pub refusal: Option<Refusal>,
}

/// A terminal multiplexer that has to be asked to pass an escape sequence on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Multiplexer {
    /// No mux between this process and the terminal.
    None,
    /// `tmux`, whose passthrough envelope doubles the escape's own introducer.
    Tmux,
    /// GNU `screen`: it passes the sequence through verbatim once wrapped.
    Screen,
}

impl Multiplexer {
    /// Which mux this process is running under, from the environment.
    ///
    /// `STY` is screen's own marker; `TMUX` is tmux's. Both are set by the mux
    /// itself in every child shell, which is why they are the detection rather
    /// than a guess.
    pub fn detect() -> Self {
        if std::env::var_os("TMUX").is_some() {
            Self::Tmux
        } else if std::env::var_os("STY").is_some() {
            Self::Screen
        } else {
            Self::None
        }
    }
}

/// The byte sequence that asks the terminal to put `text` on the clipboard.
///
/// `None` for text that must not be sent, which is two cases with one handling:
/// an empty payload (a `52;c;` with nothing after it *clears* the clipboard, so
/// "nothing to copy" must never reach the terminal as a write) and a payload over
/// [`MAX_ENCODED`].
///
/// The terminator is BEL rather than ST (`ESC \`): both are in the spec, and BEL
/// is the one every terminal that implements OSC 52 at all accepts.
pub fn sequence(text: &str, mux: Multiplexer) -> Option<String> {
    if text.is_empty() {
        return None;
    }
    let encoded = STANDARD.encode(text.as_bytes());
    if encoded.len() > MAX_ENCODED {
        return None;
    }
    let raw = format!("\x1b]52;c;{encoded}\x07");
    Some(match mux {
        Multiplexer::None => raw,
        // Inside a mux the raw escape is consumed by the mux and never reaches
        // the outer terminal, so the wrapped form is the *only* form sent:
        // emitting both would leave two identical writes and one clipboard.
        // (`Path::Terminal` still tells the truth — it claims the sequence was
        // sent, not that a terminal acted on it.)
        Multiplexer::Tmux => format!("\x1bPtmux;\x1b{raw}\x1b\\"),
        Multiplexer::Screen => format!("\x1bP{raw}\x1b\\"),
    })
}

/// Whether this process is running on a machine the user is not sitting at.
///
/// The distinction changes only *where else* the text should go: native tools
/// write the clipboard of the machine the process runs on, which over SSH is not
/// the one the user will paste from. Detected from the variables sshd sets for a
/// session (`SSH_CONNECTION`, `SSH_CLIENT`) plus mosh's (`MOSH_CONNECTION`) —
/// pi's `isRemoteSession` (`utils/clipboard.ts:22`).
pub fn is_remote() -> bool {
    ["SSH_CONNECTION", "SSH_CLIENT", "MOSH_CONNECTION"]
        .iter()
        .any(|name| std::env::var_os(name).is_some())
}

/// Write `text` to this machine's clipboard. `Err` carries arboard's reason.
pub fn set_local(text: &str) -> Result<(), String> {
    let mut clipboard = arboard::Clipboard::new().map_err(|e| e.to_string())?;
    clipboard
        .set_text(text.to_string())
        .map_err(|e| e.to_string())
}

/// Read this machine's clipboard, or `""` when there is none to read.
pub fn local_text() -> String {
    arboard::Clipboard::new()
        .and_then(|mut c| c.get_text())
        .unwrap_or_default()
}

/// Put `text` on the clipboard by whichever path can, and say which did.
///
/// Native first, then the terminal. OSC 52 is best-effort, so it must not be the
/// *only* attempt just because the harness happens to run locally — but it is
/// attempted locally too when the native path fails, because a machine with no
/// display server is exactly the case where the terminal is the only thing that
/// can still reach a clipboard.
///
/// `send` is the terminal write, injected rather than performed here so the
/// sequencing is testable without a terminal: production passes a closure that
/// writes the sequence to the same stdout the frames go to.
pub fn copy<S>(text: &str, mux: Multiplexer, remote: bool, send: S) -> Copied
where
    S: FnOnce(&str) -> std::io::Result<()>,
{
    let local = set_local(text).is_ok();
    decide(text, local, mux, remote, send)
}

/// [`copy`]'s decision, with the native result supplied rather than discovered.
///
/// Split out because the native half is the one part a test cannot place: on a
/// machine with a working clipboard `arboard` succeeds and the terminal path is
/// never reached, so a test of "the sequence is sent when only the terminal can
/// reach a clipboard" would pass by testing nothing. Passing `local` in makes
/// every branch reachable on every machine, and keeps the production path
/// (`copy`) a two-liner with no second copy of the policy in it.
fn decide<S>(text: &str, local: bool, mux: Multiplexer, remote: bool, send: S) -> Copied
where
    S: FnOnce(&str) -> std::io::Result<()>,
{
    // Remote first: over SSH the local clipboard is the wrong clipboard, so a
    // successful native write is not a reason to skip the terminal.
    if remote || !local {
        return match sequence(text, mux) {
            Some(sequence) => match send(&sequence) {
                Ok(()) => Copied {
                    path: Path::Terminal,
                    refusal: None,
                },
                Err(e) => Copied {
                    path: if local { Path::Local } else { Path::None },
                    refusal: Some(Refusal::Write(e.to_string())),
                },
            },
            // Refused by size. An empty string never gets this far: the caller
            // checks for something to copy before calling, and `sequence` says
            // `None` for it too.
            None => Copied {
                path: if local { Path::Local } else { Path::None },
                refusal: (!local).then_some(Refusal::TooLong),
            },
        };
    }
    if local {
        Copied {
            path: Path::Local,
            refusal: None,
        }
    } else {
        Copied {
            path: Path::None,
            refusal: Some(Refusal::NoPath),
        }
    }
}

/// The line the transcript shows for a copy attempt.
///
/// One function rather than a `format!` at the call site, because the promise
/// this text makes is the part that can be wrong: "copied" for a path that only
/// *sent* a request is how `/copy` used to claim success over SSH.
pub fn notice(chars: usize, copied: &Copied) -> String {
    match copied.path {
        Path::Local => format!("copied {chars} chars to the clipboard"),
        Path::Terminal => {
            format!("copied {chars} chars — sent via the terminal (OSC 52)")
        }
        Path::None => match &copied.refusal {
            Some(refusal) => format!("copy failed: {}", refusal.detail()),
            None => "copy failed: no clipboard here".to_string(),
        },
    }
}

/// Write a sequence to `writer` and flush it.
///
/// Public so the production path and its tests share one definition of "sent":
/// the event loop calls this with stdout, a test calls it with a `Vec<u8>`.
pub fn write_to<W: Write>(writer: &mut W, sequence: &str) -> std::io::Result<()> {
    writer.write_all(sequence.as_bytes())?;
    writer.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 4648's own test vectors, so the encoder is checked against the
    /// standard rather than against itself.
    #[test]
    fn the_payload_is_rfc_4648_base64() {
        for (plain, encoded) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(STANDARD.encode(plain.as_bytes()), encoded, "{plain:?}");
        }
    }

    #[test]
    fn the_sequence_is_osc_52_selection_c_with_a_bel_terminator() {
        // The whole framing, byte for byte: introducer, selection `c`
        // (the CLIPBOARD selection, not `p` or the empty default), the base64
        // payload, and BEL. A test that read only `contains("52;c;")` would
        // pass with the payload missing or the terminator wrong, and a wrong
        // terminator makes the terminal swallow the *next* frame's bytes too.
        let sequence = sequence("hello world", Multiplexer::None).expect("a sequence");
        assert_eq!(sequence, "\x1b]52;c;aGVsbG8gd29ybGQ=\x07");
        assert!(sequence.starts_with("\u{1b}]52;c;"));
        assert!(sequence.ends_with('\u{7}'));
    }

    #[test]
    fn utf8_text_survives_the_round_trip() {
        // `chars().len()` is not `bytes().len()`, and base64 encodes bytes: a
        // payload of `é`/`→`/an emoji must decode back to the same bytes.
        let text = "héllo → 🙂";
        let sequence = sequence(text, Multiplexer::None).expect("a sequence");
        let payload = sequence
            .trim_start_matches("\u{1b}]52;c;")
            .trim_end_matches('\u{7}');
        let decoded = STANDARD.decode(payload).expect("valid base64");
        assert_eq!(String::from_utf8(decoded).expect("utf-8"), text);
    }

    #[test]
    fn nothing_is_emitted_for_nothing_to_copy() {
        // An empty payload is not a no-op on the wire: `ESC ] 52 ; c ; BEL`
        // asks the terminal to *clear* the clipboard. Copying nothing must not
        // erase what the user already had.
        assert_eq!(sequence("", Multiplexer::None), None);
    }

    #[test]
    fn an_oversized_payload_is_not_sent() {
        // One byte over the encoded ceiling, and the string that produces it.
        let too_long = "a".repeat(MAX_ENCODED * 3 / 4 + 1);
        assert!(STANDARD.encode(too_long.as_bytes()).len() > MAX_ENCODED);
        assert_eq!(sequence(&too_long, Multiplexer::None), None);

        // And the largest one that is still sent, so the boundary is a boundary
        // rather than an off-by-a-lot.
        let fits = "a".repeat(MAX_ENCODED * 3 / 4);
        assert_eq!(STANDARD.encode(fits.as_bytes()).len(), MAX_ENCODED);
        assert!(sequence(&fits, Multiplexer::None).is_some());
    }

    #[test]
    fn a_multiplexer_gets_the_passthrough_envelope_and_only_the_wrapped_form() {
        let raw = "\x1b]52;c;aGk=\x07";
        assert_eq!(sequence("hi", Multiplexer::None).expect("a sequence"), raw);
        // tmux's envelope: DCS `tmux;` then the sequence with its own ESC
        // doubled, then ST. Without the doubling tmux drops it.
        assert_eq!(
            sequence("hi", Multiplexer::Tmux).expect("a sequence"),
            "\x1bPtmux;\x1b\x1b]52;c;aGk=\x07\x1b\\"
        );
        // screen wraps verbatim.
        assert_eq!(
            sequence("hi", Multiplexer::Screen).expect("a sequence"),
            "\x1bP\x1b]52;c;aGk=\x07\x1b\\"
        );
        for mux in [Multiplexer::Tmux, Multiplexer::Screen] {
            let wrapped = sequence("hi", mux).expect("a sequence");
            // One occurrence of the introducer beyond the envelope: the wrapped
            // form is the only form, never both (two writes, one clipboard).
            assert_eq!(wrapped.matches("]52;c;").count(), 1, "{mux:?}: {wrapped:?}");
        }
    }

    #[test]
    fn the_terminal_path_is_taken_when_only_the_terminal_can_reach_a_clipboard() {
        // The SSH case: no local clipboard at all. This is the branch that used
        // to end in "copied N chars" with nothing copied anywhere.
        let mut sent = Vec::new();
        let copied = decide("remote answer", false, Multiplexer::None, true, |s| {
            sent.push(s.to_string());
            Ok(())
        });
        assert_eq!(copied.path, Path::Terminal);
        assert_eq!(copied.refusal, None);
        assert_eq!(sent, vec!["\x1b]52;c;cmVtb3RlIGFuc3dlcg==\x07".to_string()]);
    }

    #[test]
    fn a_remote_session_sends_the_sequence_even_when_the_native_write_worked() {
        // The subtle one: over SSH `arboard` can succeed against the *remote*
        // machine's clipboard. Reporting only that would be true and useless —
        // the user is going to paste locally, so the terminal still has to be
        // asked. Hence `remote` rather than `!local` as the trigger.
        let mut sent = Vec::new();
        let copied = decide("answer", true, Multiplexer::None, true, |s| {
            sent.push(s.to_string());
            Ok(())
        });
        assert_eq!(copied.path, Path::Terminal);
        assert_eq!(sent.len(), 1, "the terminal must still be asked");
    }

    #[test]
    fn a_local_session_does_not_write_osc_52_when_the_native_path_worked() {
        // The negative control for the test above: locally, with a working
        // clipboard, there is nothing for the terminal to do — and a redundant
        // OSC 52 is not free, it races the native write for the same clipboard.
        let mut sent = Vec::new();
        let copied = decide("answer", true, Multiplexer::None, false, |s| {
            sent.push(s.to_string());
            Ok(())
        });
        assert_eq!(copied.path, Path::Local);
        assert!(sent.is_empty(), "no sequence may be written: {sent:?}");
    }

    #[test]
    fn a_failed_terminal_write_is_not_reported_as_a_copy() {
        // A closed stdout, a full pipe. The old code had one failure mode and
        // printed success for it; this is the one that says so.
        let copied = decide("answer", false, Multiplexer::None, true, |_| {
            Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "closed",
            ))
        });
        assert_eq!(copied.path, Path::None);
        assert!(matches!(copied.refusal, Some(Refusal::Write(_))));
        let text = notice(6, &copied);
        assert!(text.starts_with("copy failed"), "{text}");
        assert!(!text.contains("copied 6 chars"), "{text}");
    }

    #[test]
    fn an_oversized_remote_copy_reports_why_and_never_claims_success() {
        let too_long = "a".repeat(MAX_ENCODED * 3 / 4 + 1);
        let copied = decide(&too_long, false, Multiplexer::None, true, |_| {
            panic!("an oversized payload must not reach the terminal")
        });
        assert_eq!(copied.path, Path::None);
        assert_eq!(copied.refusal, Some(Refusal::TooLong));
        let text = notice(too_long.chars().count(), &copied);
        assert!(text.contains("too long to send to the terminal"), "{text}");
        assert!(!text.starts_with("copied"), "{text}");
    }

    #[test]
    fn the_notice_matches_the_path_that_actually_worked() {
        // The user-visible half of the whole change. Each sentence has to be
        // about the path that ran, and only the native path may claim the text
        // is on a clipboard the user can paste from without qualification.
        let local = Copied {
            path: Path::Local,
            refusal: None,
        };
        assert_eq!(notice(12, &local), "copied 12 chars to the clipboard");

        let terminal = Copied {
            path: Path::Terminal,
            refusal: None,
        };
        let text = notice(12, &terminal);
        assert_eq!(text, "copied 12 chars — sent via the terminal (OSC 52)");
        // "sent", not "copied … to clipboard": this module cannot verify that
        // the terminal acted, and the old wording is the bug being fixed.
        assert!(!text.contains("to the clipboard"), "{text}");

        let none = Copied {
            path: Path::None,
            refusal: Some(Refusal::NoPath),
        };
        assert!(notice(12, &none).starts_with("copy failed"), "{text}");
    }
}
