//! The host side: spawn, handshake, invoke-with-timeout, crash accounting.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout};
use tokio::sync::Mutex;

use crate::manifest::ExtManifest;
use crate::{CRASH_BUDGET, INVOKE_TIMEOUT_SECS};

/// Largest single protocol frame accepted from a child (2 MiB, aligned with
/// `epsilon_tools::checkpoint::MAX_FILE_BYTES`).
///
/// A line is the only unit the protocol has, so a child that never sends a newline would
/// otherwise make the host buffer whatever it likes: `read_line` is happy to accumulate
/// megabytes, and the child chooses the number. The cap is far above any frame the host
/// itself sends and far above a large tool result; it exists to bound the host's memory,
/// not to police result size.
const MAX_FRAME_BYTES: usize = 2 * 1024 * 1024;

/// Most bytes of child output one invoke may consume.
///
/// `INVOKE_TIMEOUT_SECS` bounds an invoke in time; this bounds it in bytes. Without it a
/// child could keep the reader busy for the whole window — and, because chatter is skipped
/// rather than failing, busy is all it would ever be — on a stream the host can never
/// finish reading. Hitting this is a protocol violation, not a slow extension.
const MAX_INVOKE_BYTES: usize = 16 * 1024 * 1024;

/// Errors surfaced by the host. All are values — a crashing extension must degrade the
/// extension, never the harness.
#[derive(Debug)]
pub enum ExtError {
    /// The extension exceeded its per-invoke timeout.
    Timeout,
    /// The process died (crash budget tracks disabling).
    Crashed(String),
    /// Protocol violation (bad line, wrong id, malformed JSON).
    Protocol(String),
    /// The extension returned an error result for this invocation.
    ToolFailed {
        /// Error text the model should see.
        message: String,
    },
}

impl std::fmt::Display for ExtError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExtError::Timeout => write!(f, "extension invoke timed out"),
            ExtError::Crashed(d) => write!(f, "extension crashed: {d}"),
            ExtError::Protocol(d) => write!(f, "extension protocol violation: {d}"),
            ExtError::ToolFailed { message } => write!(f, "{message}"),
        }
    }
}

/// A live extension process.
pub struct ExtHost {
    name: String,
    io: Mutex<ChildIo>,
    disabled: AtomicBool,
    crashes: AtomicU32,
    /// Plan 007: the manifest's declared event interests (`["*"]` = all). Empty list
    /// means the extension receives nothing. Evaluated by `wants_event`.
    events: Vec<String>,
}

struct ChildIo {
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
    /// Diagnostics/receipts the child sends via {"type":"log"} lines (capped).
    logs: Vec<String>,
    /// Held so kill_on_drop applies when the host is dropped.
    #[allow(dead_code)]
    child: Child,
}

impl ExtHost {
    /// Spawn + handshake. Fails fast if the binary is missing or hello never arrives.
    pub async fn spawn(dir: &Path, manifest: &ExtManifest) -> Result<Arc<Self>, ExtError> {
        let bin = manifest.resolved_bin(dir);
        let mut child = tokio::process::Command::new(&bin)
            .args(&manifest.args)
            .current_dir(dir)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| ExtError::Crashed(format!("spawn {}: {e}", bin.display())))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| ExtError::Protocol("no stdin".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| ExtError::Protocol("no stdout".into()))?;
        let mut reader = BufReader::new(stdout);

        // Handshake with timeout: first line must be a hello.
        let hello = tokio::time::timeout(
            Duration::from_secs(INVOKE_TIMEOUT_SECS),
            read_line_str(&mut reader),
        )
        .await
        .map_err(|_| ExtError::Timeout)?
        .map_err(|e| ExtError::Crashed(e.to_string()))?;

        let line = hello.ok_or_else(|| ExtError::Crashed("exited before hello".into()))?;
        let v: serde_json::Value =
            serde_json::from_str(&line).map_err(|e| ExtError::Protocol(e.to_string()))?;
        if v.get("type").and_then(|t| t.as_str()) != Some("hello") {
            return Err(ExtError::Protocol("first message must be hello".into()));
        }

        Ok(Arc::new(Self {
            name: manifest.name.clone(),
            io: Mutex::new(ChildIo {
                stdin,
                stdout: reader,
                next_id: 0,
                child,
                logs: Vec::new(),
            }),
            disabled: AtomicBool::new(false),
            crashes: AtomicU32::new(0),
            events: manifest.events.clone(),
        }))
    }

    /// Extension name from its manifest.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// True once the crash budget is exhausted.
    pub fn is_disabled(&self) -> bool {
        self.disabled.load(Ordering::SeqCst)
    }

    /// Forward one agent event to the child (fire-and-forget, subject to the
    /// extension's declared manifest filter). Best-effort: event delivery must never
    /// break a run.
    ///
    /// Plan 007: the `filtered` parameter is GONE — the host now evaluates its own
    /// manifest's `events` list (captured at spawn) so a declared `["*"]` genuinely
    /// receives events and an empty list receives nothing. The old signature with a
    /// hardcoded `true` made forwarding dead code.
    pub async fn send_event(&self, event: &serde_json::Value) {
        // Event type tag lives at the top level of the payload (`event.type`).
        let tag = event
            .get("type")
            .and_then(|t| t.as_str())
            .unwrap_or("unknown");
        let wants = self.events.iter().any(|e| e == "*" || e == tag);
        if !wants {
            return;
        }
        let mut io = self.io.lock().await;
        let mut line = serde_json::to_string(&serde_json::json!({
            "type": "event",
            "event": event,
        }))
        .expect("event serializes");
        line.push('\n');
        let _ = io.stdin.write_all(line.as_bytes()).await;
        let _ = io.stdin.flush().await;
    }

    /// Log receipts the child sent via {"type":"log"} lines.
    pub async fn logs(&self) -> Vec<String> {
        self.io.lock().await.logs.clone()
    }

    /// Invoke one tool by name. Serialized per extension (extensions own their ordering);
    /// timeout enforced; crashes counted against the budget.
    pub async fn invoke(
        &self,
        tool: &str,
        call_id: &str,
        args: serde_json::Value,
    ) -> Result<(String /*text*/, serde_json::Value /*details*/), ExtError> {
        if self.disabled.load(Ordering::SeqCst) {
            return Err(ExtError::Crashed(
                "extension disabled (crash budget)".into(),
            ));
        }

        let id = {
            let mut io = self.io.lock().await;
            io.next_id += 1;
            format!("i_{}", io.next_id)
        };
        let req = serde_json::json!({
            "type": "invoke", "id": id, "call_id": call_id,
            "tool": tool, "args": args,
        });

        let result = self.write_and_read(&id, req).await;
        match result {
            Ok(ok) => Ok(ok),
            Err(ExtError::Crashed(d)) => {
                let n = self.crashes.fetch_add(1, Ordering::SeqCst) + 1;
                if n >= CRASH_BUDGET {
                    self.disabled.store(true, Ordering::SeqCst);
                }
                Err(ExtError::Crashed(d))
            }
            Err(other) => Err(other),
        }
    }

    async fn write_and_read(
        &self,
        id: &str,
        req: serde_json::Value,
    ) -> Result<(String, serde_json::Value), ExtError> {
        let mut io = self.io.lock().await;

        let mut line =
            serde_json::to_string(&req).map_err(|e| ExtError::Protocol(e.to_string()))?;
        line.push('\n');
        io.stdin
            .write_all(line.as_bytes())
            .await
            .map_err(|e| ExtError::Crashed(format!("write: {e}")))?;
        io.stdin
            .flush()
            .await
            .map_err(|e| ExtError::Crashed(format!("flush: {e}")))?;

        // The deadline is for the whole invoke, not for each line. A child that emits any
        // complete frame inside the window would otherwise restart a per-read timer and
        // hold the call open indefinitely while the documented contract promises it ends.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(INVOKE_TIMEOUT_SECS);
        let mut bytes_read = 0usize;

        loop {
            let line = tokio::time::timeout_at(deadline, read_line_bytes(&mut io.stdout))
                .await
                .map_err(|_| ExtError::Timeout)?
                .map_err(|e| ExtError::Crashed(format!("read: {e}")))?;
            let line = match line {
                Frame::Line(l) => l,
                Frame::Eof => return Err(ExtError::Crashed("stdout closed".into())),
                Frame::TooLong => {
                    return Err(ExtError::Protocol(format!(
                        "frame exceeds {MAX_FRAME_BYTES} bytes"
                    )));
                }
            };
            bytes_read += line.len();
            if bytes_read > MAX_INVOKE_BYTES {
                return Err(ExtError::Protocol(format!(
                    "child produced more than {MAX_INVOKE_BYTES} bytes for one invoke without \
                     answering it"
                )));
            }
            // Child diagnostics channel: {"type":"log","text":...} — collected, never
            // treated as a result.
            if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&line)
                && v.get("type").and_then(|t| t.as_str()) == Some("log")
            {
                if let (Some(text), true) =
                    (v.get("text").and_then(|t| t.as_str()), io.logs.len() < 1000)
                {
                    io.logs.push(text.to_string());
                }
                continue;
            }
            let v: serde_json::Value = serde_json::from_slice(&line)
                .map_err(|e| ExtError::Protocol(format!("{e} in a {}-byte line", line.len())))?;
            if v.get("type").and_then(|t| t.as_str()) != Some("result") {
                continue; // ignore non-result chatter v1
            }
            if v.get("id").and_then(|i| i.as_str()) != Some(id) {
                continue; // stale/unknown id: skip
            }
            // A result frame is validated before anything in it is believed. Accepting a
            // frame with no `text`, or with a `details` that is not an object, would turn
            // a broken child into a successful empty tool result — indistinguishable, in
            // the transcript, from a tool that legitimately returned nothing. The host
            // names the problem instead of quoting the payload (`ExtError::Protocol` goes
            // into `ToolFailure` text, which lands in the transcript).
            let text = match v.get("text") {
                Some(serde_json::Value::String(t)) => t.clone(),
                Some(other) => {
                    return Err(ExtError::Protocol(format!(
                        "result.text must be a string, got {}",
                        json_kind(other)
                    )));
                }
                None => return Err(ExtError::Protocol("result has no text".into())),
            };
            let details = match v.get("details") {
                Some(d @ serde_json::Value::Object(_)) => d.clone(),
                Some(other) => {
                    return Err(ExtError::Protocol(format!(
                        "result.details must be an object, got {}",
                        json_kind(other)
                    )));
                }
                None => return Err(ExtError::Protocol("result has no details".into())),
            };
            let is_error = match v.get("is_error") {
                Some(serde_json::Value::Bool(b)) => *b,
                Some(other) => {
                    return Err(ExtError::Protocol(format!(
                        "result.is_error must be a bool, got {}",
                        json_kind(other)
                    )));
                }
                None => return Err(ExtError::Protocol("result has no is_error".into())),
            };
            return if is_error {
                Err(ExtError::ToolFailed { message: text })
            } else {
                Ok((text, details))
            };
        }
    }
}

/// What one read off the child's stdout produced.
enum Frame {
    /// A complete line, with its terminator stripped and its JSON escapes intact.
    Line(Vec<u8>),
    /// Clean end of stream.
    Eof,
    /// A line longer than [`MAX_FRAME_BYTES`]; the rest of it is left unread on purpose.
    TooLong,
}

/// Read one protocol frame as bytes, refusing to buffer more than [`MAX_FRAME_BYTES`].
///
/// Bytes rather than `read_line`'s `String`: a line is only known to be UTF-8 once it is
/// complete, and deciding that per 8 KiB pipe chunk would reject a multi-byte character
/// that happens to straddle a buffer boundary. The cap is what keeps a child that never
/// sends a newline from choosing how much memory the harness allocates; when it trips,
/// the host stops draining the line, so callers must treat the stream as desynchronized.
async fn read_line_bytes<R: tokio::io::AsyncBufRead + Unpin>(
    reader: &mut R,
) -> std::io::Result<Frame> {
    let mut buf: Vec<u8> = Vec::new();
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            return Ok(if buf.is_empty() {
                Frame::Eof
            } else {
                // Final line with no terminator, then EOF: still a frame.
                Frame::Line(buf)
            });
        }
        if let Some(pos) = available.iter().position(|b| *b == b'\n') {
            if buf.len() + pos > MAX_FRAME_BYTES {
                // The terminator is already in view, so the overrun is knowable without
                // buffering it: the check below the loop exists for the cases this one
                // cannot see, and waiting for it here would store the payload it refuses.
                return Ok(Frame::TooLong);
            }
            buf.extend_from_slice(&available[..=pos]);
            reader.consume(pos + 1);
            break;
        }
        let n = available.len();
        buf.extend_from_slice(available);
        reader.consume(n);
        if buf.len() > MAX_FRAME_BYTES {
            // A line that never ends is refused as soon as it is longer than the cap.
            return Ok(Frame::TooLong);
        }
    }
    while matches!(buf.last(), Some(b'\r' | b'\n')) {
        buf.pop();
    }
    // Terminator and payload can arrive in one read, so the finished line is checked too.
    if buf.len() > MAX_FRAME_BYTES {
        return Ok(Frame::TooLong);
    }
    // A malformed UTF-8 line is a protocol problem the caller reports, not an I/O error.
    if std::str::from_utf8(&buf).is_err() {
        buf = String::from_utf8_lossy(&buf).into_owned().into_bytes();
    }
    Ok(Frame::Line(buf))
}

/// Name a JSON value's kind, for diagnostics that must not quote the child's payload.
fn json_kind(v: &serde_json::Value) -> &'static str {
    match v {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "a bool",
        serde_json::Value::Number(_) => "a number",
        serde_json::Value::String(_) => "a string",
        serde_json::Value::Array(_) => "an array",
        serde_json::Value::Object(_) => "an object",
    }
}

/// tokio 1.53 dropped the `next_line` convenience; read one newline-terminated line.
async fn read_line_str<R: tokio::io::AsyncBufRead + Unpin>(
    reader: &mut R,
) -> std::io::Result<Option<String>> {
    let mut buf = String::new();
    let n = reader.read_line(&mut buf).await?;
    Ok(if n == 0 {
        None
    } else {
        Some(buf.trim_end_matches(['\r', '\n']).to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The handshake still reads a `String`; the frame reader hands back bytes. Both have
    /// to agree on where a line ends, or a child's first line and its tenth would be
    /// framed differently.
    #[tokio::test]
    async fn the_frame_reader_and_the_handshake_reader_agree_on_a_line() {
        let raw = b"{\"type\":\"hello\"}\r\n{\"type\":\"result\"}\n";
        let mut a = BufReader::new(std::io::Cursor::new(raw.to_vec()));
        let mut b = BufReader::new(std::io::Cursor::new(raw.to_vec()));

        for expected in ["{\"type\":\"hello\"}", "{\"type\":\"result\"}"] {
            let via_handshake = read_line_str(&mut a).await.unwrap().unwrap();
            let via_frames = match read_line_bytes(&mut b).await.unwrap() {
                Frame::Line(l) => String::from_utf8(l).unwrap(),
                other => panic!(
                    "expected a line, got {}",
                    match other {
                        Frame::Eof => "eof",
                        _ => "too long",
                    }
                ),
            };
            assert_eq!(via_handshake, expected);
            assert_eq!(via_frames, expected);
        }
        assert!(read_line_str(&mut a).await.unwrap().is_none());
        assert!(matches!(read_line_bytes(&mut b).await.unwrap(), Frame::Eof));
    }

    /// A line with no terminator at the end of the stream is still a frame: closing stdin
    /// is a legitimate way for a child to end.
    #[tokio::test]
    async fn a_final_line_without_a_terminator_is_a_frame() {
        let mut r = BufReader::new(std::io::Cursor::new(b"{\"a\":1}".to_vec()));
        match read_line_bytes(&mut r).await.unwrap() {
            Frame::Line(l) => assert_eq!(l, b"{\"a\":1}"),
            _ => panic!("expected the unterminated line"),
        }
    }

    /// The cap has to trip on total bytes, including bytes that arrive after some complete
    /// characters — so this overruns with a multi-byte character rather than with `x`.
    #[tokio::test]
    async fn a_line_past_the_cap_is_refused_rather_than_buffered() {
        let mut raw = vec![b'a'; MAX_FRAME_BYTES + 1];
        raw.push(b'\n');
        let mut r = BufReader::new(std::io::Cursor::new(raw));
        assert!(matches!(
            read_line_bytes(&mut r).await.unwrap(),
            Frame::TooLong
        ));
    }

    /// A frame exactly at the cap is accepted: the boundary must not depend on which side
    /// of it the newline falls.
    #[tokio::test]
    async fn a_line_at_the_cap_is_still_a_frame() {
        let mut raw = vec![b'a'; MAX_FRAME_BYTES];
        raw.push(b'\n');
        let mut r = BufReader::new(std::io::Cursor::new(raw));
        match read_line_bytes(&mut r).await.unwrap() {
            Frame::Line(l) => assert_eq!(l.len(), MAX_FRAME_BYTES),
            _ => panic!("a frame at the cap must be read"),
        }
    }

    /// A reader that hands over a few bytes per poll, so the frame reader has to
    /// accumulate instead of seeing the whole line at once. Real pipes behave this way,
    /// and staying inside `AsyncRead` keeps the test free of `unsafe`.
    struct Drip {
        data: Vec<u8>,
        at: usize,
        size: usize,
    }

    impl tokio::io::AsyncRead for Drip {
        fn poll_read(
            mut self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
            buf: &mut tokio::io::ReadBuf<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            let n = (self.data.len() - self.at)
                .min(buf.remaining())
                .min(self.size);
            let at = self.at;
            buf.put_slice(&self.data[at..at + n]);
            self.at += n;
            std::task::Poll::Ready(Ok(()))
        }
    }

    /// The cap must fire while the line is still arriving. A check that only runs on the
    /// finished line would have stored the whole payload first — which is the entire cost
    /// the cap exists to avoid, and unbounded when the child chooses to keep writing.
    #[tokio::test]
    async fn a_line_past_the_cap_is_refused_while_it_is_still_arriving() {
        let endless = Drip {
            data: vec![b'a'; MAX_FRAME_BYTES * 2],
            at: 0,
            size: 4096,
        };
        let mut r = BufReader::new(endless);
        assert!(
            matches!(read_line_bytes(&mut r).await.unwrap(), Frame::TooLong),
            "an endless line must be refused before it is buffered whole"
        );
    }

    /// Invalid UTF-8 is reported by the JSON parse (a protocol violation), not as an I/O
    /// error that would read as a crash.
    #[tokio::test]
    async fn invalid_utf8_becomes_a_line_for_the_json_parser_to_refuse() {
        let mut r = BufReader::new(std::io::Cursor::new(vec![0xff, 0xfe, b'\n']));
        match read_line_bytes(&mut r).await.unwrap() {
            Frame::Line(l) => {
                assert!(serde_json::from_slice::<serde_json::Value>(&l).is_err());
            }
            _ => panic!("invalid UTF-8 must still produce a line"),
        }
    }
}
