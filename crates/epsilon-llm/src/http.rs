//! Minimal SSE-over-HTTP plumbing shared by the direct provider adapters.
//!
//! Port of pi's per-provider transport approach (pi-0.84.2 `packages/ai/src/api/`):
//! each provider speaks its real REST API over plain HTTP and we parse the server-sent
//! event stream ourselves — no SDK/router crate in between. The decoder follows the
//! SSE wire format pi implements (`event:`/`data:` lines, blank-line delimited,
//! multi-line data joined with `\n`, CRLF tolerated).

#![cfg(feature = "real")]

use std::sync::OnceLock;

/// One decoded server-sent event.
#[derive(Debug, Clone)]
pub struct SseEvent {
    /// `event:` field of the frame (`None` when the sender omits it — chat
    /// completions style, where the data JSON carries its own shape).
    pub name: Option<String>,
    /// `data:` payload with continuation lines joined by `\n`.
    pub data: String,
}

/// Incremental decoder: feed received lines, pull completed events out.
#[derive(Default)]
pub(crate) struct SseDecoder {
    data: Vec<String>,
    name: Option<String>,
}

impl SseDecoder {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Feed one received line (without trailing newline). Returns an event when the
    /// terminating blank line arrives.
    pub(crate) fn feed(&mut self, line: &str) -> Option<SseEvent> {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.is_empty() {
            if self.data.is_empty() {
                self.name = None;
                return None;
            }
            let ev = SseEvent {
                name: self.name.take(),
                data: self.data.join("\n"),
            };
            self.data.clear();
            return Some(ev);
        }
        // Comments (leading ':') and `id:`/`retry:` fields carry nothing we
        // need. `event:` selects the Responses dispatch (chat completions
        // frames omit it — the data JSON carries its own shape).
        if let Some(rest) = line.strip_prefix("event:") {
            let name = rest.strip_prefix(' ').unwrap_or(rest).to_string();
            if !name.is_empty() {
                self.name = Some(name);
            }
        } else if let Some(rest) = line.strip_prefix("data:") {
            self.data
                .push(rest.strip_prefix(' ').unwrap_or(rest).to_string());
        }
        None
    }
}

/// Shared HTTP client: connection pooling + a connect timeout, no overall timeout
/// (streams legitimately run for minutes).
///
/// M196: install the `ring` crypto provider before the first client exists.
///
/// The build uses reqwest's `rustls-no-provider`, so rustls has **no** default
/// provider. Without this line reqwest panics at client construction with
/// "no process-level CryptoProvider available" — a panic on the first HTTPS
/// request, in a path that is otherwise a network error. `install_default`
/// is idempotent, so calling it here on every init costs one atomic load.
///
/// `ring` rather than `aws-lc-rs` is a deliberate trade, recorded in the
/// workspace manifest: `aws-lc-rs` is a C library needing MSVC, NASM and
/// CMake+MSBuild, and `ring` does not implement P-521.
pub(crate) fn http_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        // An error here means another provider was installed first, which is
        // fine and not worth failing a user's request over -- the process-level
        // default is still usable.
        let _ = rustls::crypto::ring::default_provider().install_default();
        reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(15))
            .build()
            .expect("reqwest client builds with sane defaults")
    })
}

// A `status_error` used to live here, `#[allow(dead_code)]` and commented
// "wired into the retry-classified error path; kept for parity". Neither half
// was true: nothing in the tree called it. A suppression plus a comment
// describing a wiring that does not exist is worse than no function, because the
// next reader believes the comment. The live equivalent is
// `epsilon_tools::webfetch::status_error`, which caps its snippet at 2000
// characters rather than this one's 400 — and which actually formats the errors
// a user sees.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decoder_handles_openai_style_data_only_chunks() {
        let mut d = SseDecoder::new();
        assert!(d.feed("data: {\"a\":1}").is_none());
        assert!(d.feed("").is_some(), "blank line completes the event");
        assert!(d.feed("data: [DONE]").is_none());
        let done = d.feed("").expect("[DONE] completes as a normal event");
        assert_eq!(done.data, "[DONE]");
        assert!(d.feed("").is_none(), "empty-data events are dropped");
    }

    #[test]
    fn decoder_joins_multiline_data_and_tolerates_crlf() {
        let mut d = SseDecoder::new();
        assert!(d.feed("data: line1\r").is_none());
        assert!(d.feed("data: line2").is_none());
        assert!(d.feed(": keep-alive comment").is_none());
        let ev = d.feed("").expect("completed");
        assert_eq!(ev.data, "line1\nline2");
    }

    #[test]
    fn decoder_ignores_event_names_but_keeps_data() {
        // ponytail-audit #4: event-field plumbing removed; data must survive.
        let mut d = SseDecoder::new();
        assert!(d.feed("event: content_block_delta").is_none());
        assert!(d.feed("data: {\"type\":\"x\"}").is_none());
        let ev = d.feed("").expect("completed");
        assert_eq!(ev.data, "{\"type\":\"x\"}");
    }
}
