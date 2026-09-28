//! How much of a pending-message queue releases at a drain point.
//!
//! **Why this lives in contracts rather than in `epsilon-core`.** It started in core,
//! where the queue that honours it lives. But `SessionState` — the payload both wire
//! surfaces serve — needs to carry the current mode, because pi's `RpcSessionState`
//! reports `steeringMode` and `followUpMode` and a client that can set a mode but not
//! read it back has half a feature. `SessionState` is in contracts, contracts may not
//! depend on core, and the architecture law forbids a library depending on CLI in the
//! other direction for the same reason. So the type moves down to where both can see it.
//! Core re-exports it, so `epsilon_core::QueueMode` keeps resolving and no caller had to
//! change.
//!
//! Ported verbatim from pi's `QueueMode`.

use serde::{Deserialize, Serialize};

/// Whether a drain releases everything queued or only the oldest message.
///
/// The `serde(rename)` on each variant is load-bearing and is **checked against
/// [`QueueMode::as_wire`] by a test**. A derived `Serialize` writes the *Rust* variant
/// name — `"All"`, `"OneAtATime"` — which is a third spelling nobody chose, and it is
/// what this type emitted before that test existed: `get_state` reported
/// `"OneAtATime"` while `set_steering_mode` accepted `"one-at-a-time"` and nothing else.
/// A client could set a mode and then read back a string it had never sent.
///
/// That is the same defect as D29 (one type, two shapes) and the same lesson: two
/// serialization paths in one type will drift unless something compares them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum QueueMode {
    /// Release everything queued.
    #[serde(rename = "all")]
    All,
    /// Release only the oldest; the rest wait for the next drain point.
    #[serde(rename = "one-at-a-time")]
    OneAtATime,
}

impl QueueMode {
    /// Every mode, in declaration order — so a listing cannot drift from the enum.
    pub const ALL: [QueueMode; 2] = [QueueMode::All, QueueMode::OneAtATime];

    /// The wire spelling, which is pi's: `"all"` and `"one-at-a-time"`.
    ///
    /// A `Display`/`as_str` pair rather than a serde `rename_all`, because the RPC layer
    /// needs to *parse* the same spelling and refuses anything else. Serde's derived
    /// `Deserialize` would accept a second spelling derived from the variant name and
    /// quietly disagree with this one — which is exactly the class of defect this project
    /// has now been bitten by twice (D26's `follow_up` alias, D29's two entry shapes).
    /// One spelling, written down once, used by both directions.
    pub fn as_wire(self) -> &'static str {
        match self {
            QueueMode::All => "all",
            QueueMode::OneAtATime => "one-at-a-time",
        }
    }

    /// Parse pi's wire spelling, or `None` for anything else.
    ///
    /// Refusing rather than defaulting is the point. A client that sent `"oneAtATime"`
    /// and got `All` back would be told a setting it did not ask for took effect, and
    /// would learn about it from the queue draining every message at once.
    pub fn from_wire(s: &str) -> Option<QueueMode> {
        match s {
            "all" => Some(QueueMode::All),
            "one-at-a-time" => Some(QueueMode::OneAtATime),
            _ => None,
        }
    }
}

/// A wire spelling no mode claims — the wording used when refusing one.
pub const KNOWN_QUEUE_MODES: &str = "`all` or `one-at-a-time`";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wire_spelling_round_trips_for_every_mode() {
        for m in QueueMode::ALL {
            assert_eq!(
                QueueMode::from_wire(m.as_wire()),
                Some(m),
                "{} must parse back to itself",
                m.as_wire()
            );
        }
    }

    /// The spellings are pi's, quoted rather than remembered: `rpc-types.ts` declares
    /// `set_steering_mode: { mode: "all" | "one-at-a-time" }`.
    #[test]
    fn the_wire_spellings_are_pis_and_nothing_else() {
        assert_eq!(QueueMode::All.as_wire(), "all");
        assert_eq!(QueueMode::OneAtATime.as_wire(), "one-at-a-time");
    }

    /// **serde and `as_wire` must not disagree.**
    ///
    /// A type with a derived `Serialize` *and* a hand-written wire spelling has two
    /// serialization paths, and they will drift unless something compares them. They did:
    /// `get_state` emitted `"OneAtATime"` while `set_steering_mode` accepted only
    /// `"one-at-a-time"`, so a client could set a mode and read back a string it had
    /// never sent. The RPC test caught it by accident; this test is what should have.
    ///
    /// The comparison is the point — a fixed literal on either side would only re-assert
    /// one of the two paths.
    #[test]
    fn serde_and_as_wire_agree_for_every_mode() {
        for m in QueueMode::ALL {
            let via_serde = serde_json::to_value(m).expect("QueueMode serializes");
            assert_eq!(
                via_serde,
                serde_json::Value::String(m.as_wire().to_string()),
                "serde writes {:?} but as_wire says {:?} — one type, two spellings",
                via_serde,
                m.as_wire()
            );
            // ...and deserialization must accept exactly what both of them wrote, or a
            // value this type produced could not be read back.
            let back: QueueMode = serde_json::from_value(via_serde).expect("must round-trip");
            assert_eq!(back, m);
        }
    }

    /// A spelling that is merely plausible must be refused, not guessed.
    ///
    /// Each of these is a spelling a reader might reasonably expect, and accepting any of
    /// them would mean reporting a setting the client did not ask for.
    #[test]
    fn plausible_but_wrong_spellings_are_refused() {
        for s in [
            "All",
            "ALL",
            "oneAtATime",
            "one_at_a_time",
            "oneatATime",
            "one-at-a-time ",
            "",
            "all ",
        ] {
            assert_eq!(QueueMode::from_wire(s), None, "{s:?} must be refused");
        }
    }
}
