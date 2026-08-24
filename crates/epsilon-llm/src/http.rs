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
pub(crate) struct SseEvent {
    /// `data:` payload with continuation lines joined by `\n`.
    #[allow(dead_code)] // anthropic-style frames key off data.type; kept for parity
    pub event: Option<String>,
    /// `data:` payload with continuation lines joined by `\n`.
    pub data: String,
}

/// Incremental decoder: feed received lines, pull completed events out.
#[derive(Default)]
pub(crate) struct SseDecoder {
    event: Option<String>,
    data: Vec<String>,
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
                self.event = None;
                return None;
            }
            let ev = SseEvent {
                event: self.event.take(),
                data: self.data.join("\n"),
            };
            self.data.clear();
            return Some(ev);
        }
        // Comments (leading ':') and `id:`/`retry:` fields carry nothing we need.
        if let Some(rest) = line.strip_prefix("event:") {
            self.event = Some(rest.trim_start().to_string());
        } else if let Some(rest) = line.strip_prefix("data:") {
            self.data
                .push(rest.strip_prefix(' ').unwrap_or(rest).to_string());
        }
        None
    }
}

/// Shared HTTP client: connection pooling + a connect timeout, no overall timeout
/// (streams legitimately run for minutes).
pub(crate) fn http_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(15))
            .build()
            .expect("reqwest client builds with sane defaults")
    })
}

/// Render a failed HTTP response as an in-band error string (status + trimmed body),
/// pi's error-path convention: never panic, describe honestly.
#[allow(dead_code)] // wired into the retry-classified error path; kept for parity
pub(crate) fn status_error(status: u16, body: &str) -> String {
    let head: String = body.chars().take(400).collect();
    format!("HTTP {status}: {head}")
}

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
    fn decoder_captures_event_names_for_anthropic_style_frames() {
        let mut d = SseDecoder::new();
        assert!(d.feed("event: content_block_delta").is_none());
        assert!(d.feed("data: {\"type\":\"x\"}").is_none());
        let ev = d.feed("").expect("completed");
        assert_eq!(ev.event.as_deref(), Some("content_block_delta"));
    }
}
