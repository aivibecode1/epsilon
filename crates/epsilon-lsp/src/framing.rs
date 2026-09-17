//! JSON-RPC framing for language servers: `Content-Length: N\r\n\r\n{json}` over stdio.
//!
//! The LSP base protocol is HTTP-like headers followed by a JSON body. This is the whole
//! implementation — a writer that computes the byte length, and a reader that trusts the header
//! only as far as it is bounded (a server that claims 4 GB must not be able to make this crate
//! allocate it). `epsilon-mcp` gets its framing from `rmcp`, so this is a deliberate mirror of
//! that behaviour with a citation rather than a dependency across the crate boundary
//! (ADR-014 amendment, M79).

use std::io;

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWrite, AsyncWriteExt};

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
        let mut line = String::new();
        let n = reader.read_line(&mut line).await?;
        if n == 0 {
            return Ok(None); // EOF before a complete header.
        }
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

/// Build the frame bytes for tests and fixtures (the write path without a writer).
pub fn frame_bytes(body: &[u8]) -> Vec<u8> {
    let mut out = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
    out.extend_from_slice(body);
    out
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
}
