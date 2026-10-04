//! JSON-RPC framing for language servers: `Content-Length: N\r\n\r\n{json}` over stdio.
//!
//! The LSP base protocol is HTTP-like headers followed by a JSON body. This is the whole
//! implementation — a writer that computes the byte length, and a reader that trusts the header
//! only as far as it is bounded (a server that claims 4 GB must not be able to make this crate
//! allocate it). `epsilon-mcp` gets its framing from `rmcp`, so this is a deliberate mirror of
//! that behaviour with a citation rather than a dependency across the crate boundary
//! (ADR-014 amendment, M79).

use std::io;

use tokio::io::{AsyncReadExt, AsyncWrite, AsyncWriteExt};

// `read_message`'s own bound is `AsyncBufRead`, and `fill_buf`/`consume` are
// inherent to that trait's blanket impl for `BufRead` — importing it is what
// makes them reachable on a bare `&mut R`.
use tokio::io::AsyncBufReadExt as _;

/// Largest header line accepted (LSP headers are short; anything longer is a broken server).
const MAX_HEADER_LINE: usize = 8 * 1024;
/// Largest message body accepted. Generous for diagnostics, bounded for sanity.
const MAX_BODY: usize = 64 * 1024 * 1024;

/// Write one framed JSON message.
pub async fn write_message<W>(writer: &mut W, message: &serde_json::Value) -> io::Result<()>
where
    W: AsyncWrite + Unpin,
{
    let body = serde_json::to_vec(message)?;
    write_raw(writer, &body).await
}

/// Write a body that is already serialized (used by callers that build JSON by hand).
pub async fn write_raw<W>(writer: &mut W, body: &[u8]) -> io::Result<()>
where
    W: AsyncWrite + Unpin,
{
    let header = format!("Content-Length: {}\r\n\r\n", body.len());
    writer.write_all(header.as_bytes()).await?;
    writer.write_all(body).await?;
    writer.flush().await
}

/// Read one framed message. `Ok(None)` means the stream ended cleanly between messages — the
/// normal way a server exits, and not an error.
pub async fn read_message<R>(reader: &mut R) -> io::Result<Option<serde_json::Value>>
where
    R: tokio::io::AsyncBufRead + Unpin,
{
    let mut content_length: Option<usize> = None;
    loop {
        let Some(line) = read_header_line(reader).await? else {
            return Ok(None); // EOF before a complete header.
        };
        if line.len() > MAX_HEADER_LINE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "language server header line is implausibly long",
            ));
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break; // end of headers
        }
        if let Some(value) = trimmed.strip_prefix("Content-Length:") {
            let parsed = value.trim().parse::<usize>().map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "language server sent a non-numeric Content-Length",
                )
            })?;
            content_length = Some(parsed);
        }
        // Other headers (Content-Type) are ignored: only the length matters here.
    }
    let len = content_length.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "language server message had no Content-Length",
        )
    })?;
    if len > MAX_BODY {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "language server message exceeds the accepted size",
        ));
    }
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf).await?;
    let value: serde_json::Value = serde_json::from_slice(&buf).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("language server sent invalid JSON: {e}"),
        )
    })?;
    Ok(Some(value))
}

/// Read one header line, refusing to buffer more than [`MAX_HEADER_LINE`].
///
/// `AsyncBufReadExt::read_line` cannot do this: it appends to a `String` until
/// it sees a newline or EOF, so the cap could only be checked *after* the
/// allocation it exists to prevent had already happened. A language server that
/// never emits a newline — a hung one, a broken one, one that is not a language
/// server at all — could therefore make this process allocate without bound,
/// and the `MAX_HEADER_LINE` check below would arrive far too late to matter.
///
/// So the line is read in the same shape `epsilon-ext`'s frame reader uses
/// (`read_line_bytes`, `host.rs`): inspect what is buffered, and stop as soon as
/// the running total crosses the cap rather than storing the payload it refuses.
/// `None` is a clean EOF before any byte, which is how a server says it is done.
///
/// The bytes are not validated as UTF-8 here: a line is only known to be text
/// once it is complete, and deciding that per 8 KiB chunk would reject a
/// multi-byte character straddling a buffer boundary. `String::from_utf8_lossy`
/// is applied at the end, for the same reason `host.rs` applies it there.
async fn read_header_line<R>(reader: &mut R) -> io::Result<Option<String>>
where
    R: tokio::io::AsyncBufRead + Unpin,
{
    let mut buf: Vec<u8> = Vec::new();
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            return Ok(if buf.is_empty() {
                None
            } else {
                Some(String::from_utf8_lossy(&buf).into_owned())
            });
        }
        if let Some(pos) = available.iter().position(|b| *b == b'\n') {
            // The terminator is already in view, so the overrun is knowable
            // without storing the part of the line that would be refused.
            //
            // `pos + 1`, because **the terminator counts**. The check this
            // replaces was `line.len() > MAX_HEADER_LINE` on a `String` that
            // included its `\r\n`, so a line was refused when its total — CRLF
            // included — exceeded the cap. Keeping the terminator in the
            // arithmetic is what makes the cap the same number it was; the
            // off-by-one version accepted exactly one byte more than before,
            // which is a silent change to a bound nobody was told about.
            let line_len = buf.len() + pos + 1;
            if line_len > MAX_HEADER_LINE {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "language server header line is implausibly long",
                ));
            }
            buf.extend_from_slice(&available[..=pos]);
            reader.consume(pos + 1);
            break;
        }
        // No terminator in view: everything available is part of the line, so the
        // running total is the length the cap governs.
        let n = available.len();
        if buf.len() + n > MAX_HEADER_LINE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "language server header line is implausibly long",
            ));
        }
        buf.extend_from_slice(available);
        reader.consume(n);
    }
    Ok(Some(String::from_utf8_lossy(&buf).into_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::BufReader;

    #[tokio::test]
    async fn a_message_round_trips_through_the_frame() {
        let mut out: Vec<u8> = Vec::new();
        write_message(&mut out, &serde_json::json!({"jsonrpc": "2.0", "id": 1}))
            .await
            .expect("write");
        let text = String::from_utf8(out.clone()).unwrap();
        // The length must be the body's own byte count — computed here from the frame, so the test
        // cannot pass by agreeing with a wrong constant.
        let (header, body) = text.split_once("\r\n\r\n").expect("header then body");
        assert_eq!(
            header,
            format!("Content-Length: {}", body.len()),
            "advertised length must be the real body length: {text:?}"
        );
        assert!(body.starts_with('{'), "{text:?}");
        let mut reader = BufReader::new(std::io::Cursor::new(out));
        let back = read_message(&mut reader).await.unwrap().unwrap();
        assert_eq!(back["id"], 1);
    }

    #[tokio::test]
    async fn the_header_is_read_but_other_headers_are_ignored() {
        let mut bytes =
            b"Content-Length: 17\r\nContent-Type: application/vscode-jsonrpc\r\n\r\n".to_vec();
        bytes.extend_from_slice(br#"{"jsonrpc":"2.0"}"#);
        let mut reader = BufReader::new(std::io::Cursor::new(bytes));
        let msg = read_message(&mut reader).await.unwrap().unwrap();
        assert_eq!(msg["jsonrpc"], "2.0");
    }

    #[tokio::test]
    async fn a_clean_eof_between_messages_is_not_an_error() {
        let mut reader = BufReader::new(std::io::Cursor::new(Vec::new()));
        assert!(read_message(&mut reader).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn a_missing_or_broken_length_is_refused_rather_than_guessed() {
        let mut reader = BufReader::new(std::io::Cursor::new(b"X-Header: 1\r\n\r\n{}".to_vec()));
        assert!(read_message(&mut reader).await.is_err());

        let mut reader = BufReader::new(std::io::Cursor::new(
            b"Content-Length: banana\r\n\r\n{}".to_vec(),
        ));
        assert!(read_message(&mut reader).await.is_err());

        // An implausible length must be refused *before* allocating it.
        let mut reader = BufReader::new(std::io::Cursor::new(
            b"Content-Length: 999999999999\r\n\r\n".to_vec(),
        ));
        assert!(read_message(&mut reader).await.is_err());
    }

    #[tokio::test]
    async fn a_truncated_body_is_an_error_not_a_silent_empty_message() {
        let mut reader = BufReader::new(std::io::Cursor::new(
            b"Content-Length: 40\r\n\r\n{\"jsonrpc\":\"2.0\"}".to_vec(),
        ));
        assert!(read_message(&mut reader).await.is_err());
    }

    /// The cap has to bind **while** the line is being read, not once it is
    /// complete. That distinction is the whole content of this test: with
    /// `read_line`, a server that never sends a newline is buffered to EOF
    /// before any check runs, so the allocation the `MAX_HEADER_LINE` check
    /// exists to prevent has already happened by the time it is consulted.
    ///
    /// The fixture is a reader that reports a large stream but hands over one
    /// byte at a time and stops at 64 KiB — more than eight times the cap, and
    /// far past what a header line legitimately is. A bounded reader refuses at
    /// the cap; an unbounded one would keep asking for more until the fixture ran
    /// out, and the number of bytes it consumed is what the assertion reads.
    #[tokio::test]
    async fn a_server_that_never_sends_a_newline_cannot_choose_our_memory() {
        /// A reader that offers `budget` bytes of `b'x'` and then a clean EOF,
        /// handing them over in `chunk`-sized pieces, and counts what it gave.
        ///
        /// The count is the point of the fixture. It records how much the reader
        /// under test actually pulled, which is the quantity the cap is supposed
        /// to bound. An unbounded reader consumes the whole budget and then sees
        /// EOF, so it returns `Ok(None)` — *successfully*. A test that only
        /// asserted "the call returned" would therefore pass against exactly the
        /// defect it exists to catch, which is the failure mode AGENTS.md records
        /// as "a control that passes is a control you have not yet proven works".
        struct Counting {
            left: usize,
            budget: usize,
            chunk: usize,
            consumed: usize,
        }
        impl tokio::io::AsyncRead for Counting {
            fn poll_read(
                mut self: std::pin::Pin<&mut Self>,
                _cx: &mut std::task::Context<'_>,
                buf: &mut tokio::io::ReadBuf<'_>,
            ) -> std::task::Poll<std::io::Result<()>> {
                let take = self.left.min(self.chunk);
                if take == 0 {
                    // Clean EOF: no bytes, no error. An unbounded reader reaches
                    // this having consumed the whole budget, which is the
                    // assertion below.
                    return std::task::Poll::Ready(Ok(()));
                }
                let scratch = vec![b'x'; take];
                buf.put_slice(&scratch);
                self.left -= take;
                self.consumed += take;
                std::task::Poll::Ready(Ok(()))
            }
        }

        // 64 KiB of header line: eight times the cap, and a line no language
        // server has ever sent. One byte at a time, so the cap is what has to
        // stop the read rather than the fixture running out of data.
        let mut source = Counting {
            left: 64 * 1024,
            budget: 64 * 1024,
            chunk: 1,
            consumed: 0,
        };
        let mut buffered = BufReader::with_capacity(1024, &mut source);
        let outcome = read_message(&mut buffered).await;
        assert!(
            outcome.is_err(),
            "an endless header line must be refused, not reported as a clean EOF"
        );
        // The refusal happened at the cap. The allowance is one read buffer,
        // because the reader is allowed to have a chunk in hand when it notices.
        assert!(
            source.consumed <= MAX_HEADER_LINE + 1024,
            "read {} of {} bytes before refusing: the cap did not bind during the read",
            source.consumed,
            source.budget
        );
        assert!(
            source.consumed < source.budget,
            "the whole stream was consumed, so nothing bounded the read"
        );
    }

    /// A line at the cap, and one just over it. The boundary is what makes the
    /// number above a contract rather than a comment.
    #[tokio::test]
    async fn a_header_line_at_the_cap_is_accepted_and_one_byte_past_it_is_not() {
        // One construction, parameterised by the line's total length, so the two
        // cases cannot drift apart. The first version of this test built them
        // separately and the second one silently lost the `X-Pad: ` prefix — so
        // it was not one byte past the cap, it was *far* under it, and the
        // assertion that "one byte past must be refused" passed for a stream
        // that was nowhere near the boundary.
        const PREFIX: &str = "X-Pad: ";
        const TERMINATOR: &str = "\r\n";
        const BODY: &[u8] = b"{}";

        /// A message whose padded header line is exactly `total` bytes,
        /// terminator included.
        fn frame_with_header_line_of(total: usize) -> Vec<u8> {
            let pad = total - PREFIX.len() - TERMINATOR.len();
            let line = format!("{PREFIX}{}{TERMINATOR}", "p".repeat(pad));
            assert_eq!(
                line.len(),
                total,
                "the fixture must be exactly `total` bytes; it is {}",
                line.len()
            );
            let mut bytes = format!(
                "Content-Length: {len}{TERMINATOR}{line}\r\n",
                len = BODY.len()
            )
            .into_bytes();
            bytes.extend_from_slice(BODY);
            bytes
        }

        // Exactly on the cap: accepted.
        let mut reader = BufReader::new(std::io::Cursor::new(frame_with_header_line_of(
            MAX_HEADER_LINE,
        )));
        let at_cap = read_message(&mut reader).await;
        assert!(
            at_cap.is_ok(),
            "a line exactly at the cap must be accepted: {:?}",
            at_cap.err()
        );

        // One byte past: refused. This is the case that bounds the cap, and it
        // is the one a check written as `>=` would get wrong in the direction
        // that lets a hostile peer send one byte more than the limit allows.
        let mut reader = BufReader::new(std::io::Cursor::new(frame_with_header_line_of(
            MAX_HEADER_LINE + 1,
        )));
        assert!(
            read_message(&mut reader).await.is_err(),
            "a line one byte past the cap must be refused"
        );
    }
}
