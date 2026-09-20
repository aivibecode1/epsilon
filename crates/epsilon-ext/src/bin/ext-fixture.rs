//! Fixture extension child used by host integration tests.
//!
//! Default behaviour (no mode): handshake, then answer every invoke by echoing
//! `args.text` uppercased, counting `event` frames as `{"type":"log"}` receipts.
//! Integration tests depend on that default, so modes are opt-in.
//!
//! Modes are argv[1], with argv[2..] used by the modes that need a number:
//!
//! | mode | misbehaviour |
//! |---|---|
//! | `--sleep` | never answers (stalls past the invoke timeout) |
//! | `--pace` | never answers, but writes a log line every second |
//! | `--die` | exits right after handshake (crash-budget fixture) |
//! | `--exit-mid` | exits when the first invoke arrives |
//! | `--no-hello` | exits without ever writing hello |
//! | `--slow-hello` | stays alive but silent past the handshake timeout |
//! | `--garbage-hello` | first line is not JSON |
//! | `--garbage` | answers every invoke with a non-JSON line |
//! | `--tool-error` | answers with a well-formed result that reports failure |
//! | `--shape-noise` | answers with valid JSON that is not a result, then a real result |
//! | `--bad-shape` | answers with a `result` frame whose fields are missing |
//! | `--no-details` | answers with a `result` frame that has text but no details |
//! | `--wrong-types` | answers with a `result` frame whose fields have the wrong types |
//! | `--wrong-id` | answers with a result for a foreign id, then for the real one |
//! | `--late` | answers the first invoke after 12s, then behaves normally |
//! | `--huge` | writes one multi-megabyte line with no newline |
//! | `--lines N` | writes N log lines, then answers normally |
//! | `--flood N M` | writes N log lines of M bytes each, then answers normally |
//!
//! The two "not JSON" modes write text the host must never echo back into a tool result
//! (`tests/support/mod.rs` carries the same string as `GARBAGE_MARKER`, which is what the
//! tests assert is absent from host-produced text).

use std::io::{BufRead, BufWriter, Write};
use std::time::Duration;

/// The line `--garbage` and `--garbage-hello` emit. Mirrored in `tests/support/mod.rs`.
const GARBAGE: &str = "this is not json at all";

/// The message `--tool-error` reports, which the host must pass through unchanged.
const TOOL_ERROR: &str = "the fixture tool failed on purpose";

fn main() {
    let mut args = std::env::args()
        .skip(1)
        .flat_map(|a| a.split_whitespace().map(str::to_string).collect::<Vec<_>>());
    let mode = args.next().unwrap_or_default();
    // `--lines N` counts receipts; `--flood N M` counts receipts of M payload bytes each.
    let count: usize = args.next().and_then(|n| n.parse().ok()).unwrap_or(0);
    let width: usize = args.next().and_then(|n| n.parse().ok()).unwrap_or(0);

    let mut events_seen: usize = 0;
    let mut invokes_seen: usize = 0;
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut out = BufWriter::new(stdout.lock());

    if mode == "--no-hello" {
        // Die before the handshake: the host must report a failed spawn, not attach a
        // host whose every call would hang.
        std::process::exit(3);
    }
    if mode == "--garbage-hello" {
        writeln!(out, "{GARBAGE}").unwrap();
        out.flush().unwrap();
        std::process::exit(0);
    }
    if mode == "--slow-hello" {
        // Alive, but silent past the handshake window.
        std::thread::sleep(Duration::from_secs(12));
    }

    writeln!(
        out,
        r#"{{"type":"hello","name":"fixture","version":"0.1.0"}}"#
    )
    .unwrap();
    out.flush().unwrap();

    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let v: serde_json::Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if v.get("type").and_then(|t| t.as_str()) == Some("event") {
            events_seen += 1;
            let etype = v["event"]["type"].as_str().unwrap_or("?").to_string();
            writeln!(
                out,
                r#"{{"type":"log","text":"saw {etype} #{events_seen}"}}"#
            )
            .unwrap();
            out.flush().unwrap();
            continue;
        }

        let id = v.get("id").cloned().unwrap_or(serde_json::Value::Null);
        invokes_seen += 1;

        match mode.as_str() {
            "--die" => std::process::exit(7),
            "--exit-mid" => std::process::exit(9),
            "--sleep" => std::thread::sleep(Duration::from_secs(60)),
            // A frame every second, never a result: each read succeeds, so the host must
            // still stop at its own deadline rather than resetting one per line.
            "--pace" => loop {
                writeln!(out, r#"{{"type":"log","text":"still here"}}"#).unwrap();
                out.flush().unwrap();
                std::thread::sleep(Duration::from_secs(1));
            },
            "--garbage" => {
                writeln!(out, "{GARBAGE}").unwrap();
                out.flush().unwrap();
            }
            "--shape-noise" => {
                // Valid JSON, none of it a result for a live id: a bare scalar, a result
                // whose id is a number instead of a string, an unknown frame type, and an
                // object with no type at all.
                for noise in [
                    "42",
                    r#"{"type":"result","id":1,"text":"numeric id"}"#,
                    r#"{"type":"something_else","id":"i_1"}"#,
                    r#"{"id":"i_1"}"#,
                ] {
                    writeln!(out, "{noise}").unwrap();
                }
                let text = v["args"]["text"].as_str().unwrap_or("").to_uppercase();
                writeln!(
                    out,
                    r#"{{"type":"result","id":{id},"is_error":false,"text":"{text}","details":{{"fixture":true}}}}"#
                )
                .unwrap();
                out.flush().unwrap();
            }
            "--bad-shape" => {
                // A `result` with no text and no details. Accepting this as a successful
                // empty result would tell the model the tool ran and returned nothing.
                writeln!(out, r#"{{"type":"result","id":{id}}}"#).unwrap();
                out.flush().unwrap();
            }
            "--no-details" => {
                // The other half of the same frame: text is present, details is not.
                writeln!(
                    out,
                    r#"{{"type":"result","id":{id},"is_error":false,"text":"looks fine"}}"#
                )
                .unwrap();
                out.flush().unwrap();
            }
            "--wrong-types" => {
                // Fields that exist but are the wrong type. `details` matters most: the
                // tool wrapper inserts the extension's own name into it, so a non-object
                // `details` silently loses that attribution.
                writeln!(
                    out,
                    r#"{{"type":"result","id":{id},"is_error":"no","text":123,"details":"just a string"}}"#
                )
                .unwrap();
                out.flush().unwrap();
            }
            "--wrong-id" => {
                writeln!(
                    out,
                    r#"{{"type":"result","id":"i_999","is_error":false,"text":"WRONG CALL","details":{{}}}}"#
                )
                .unwrap();
                let text = v["args"]["text"].as_str().unwrap_or("").to_uppercase();
                writeln!(
                    out,
                    r#"{{"type":"result","id":{id},"is_error":false,"text":"{text}","details":{{"fixture":true}}}}"#
                )
                .unwrap();
                out.flush().unwrap();
            }
            "--tool-error" => {
                // The extension's own failure channel: a well-formed result that reports
                // the tool failed. This is `ToolFailed`, not a protocol violation.
                writeln!(
                    out,
                    r#"{{"type":"result","id":{id},"is_error":true,"text":"{TOOL_ERROR}","details":{{}}}}"#
                )
                .unwrap();
                out.flush().unwrap();
            }
            "--late" if invokes_seen == 1 => {
                // Answer, but only after the host's ten-second window has closed.
                std::thread::sleep(Duration::from_secs(12));
                let text = v["args"]["text"].as_str().unwrap_or("").to_uppercase();
                writeln!(
                    out,
                    r#"{{"type":"result","id":{id},"is_error":false,"text":"{text}","details":{{"fixture":true}}}}"#
                )
                .unwrap();
                out.flush().unwrap();
            }
            "--huge" => {
                // One line, no newline anywhere in it: `BufRead::read_line` would buffer
                // every byte of this before the host could look at it.
                let chunk = "x".repeat(64 * 1024);
                for _ in 0..64 {
                    write!(out, "{chunk}").unwrap();
                }
                out.flush().unwrap();
                std::thread::sleep(Duration::from_secs(60));
            }
            "--lines" | "--flood" => {
                // `--lines N` writes N small receipts; `--flood N M` writes N receipts of
                // M bytes each, which is what the per-invoke byte budget is about.
                let line = if mode == "--flood" {
                    format!(r#"{{"type":"log","text":"{}"}}"#, "y".repeat(width))
                } else {
                    String::new()
                };
                for i in 0..count {
                    if mode == "--flood" {
                        writeln!(out, "{line}").unwrap();
                    } else {
                        writeln!(out, r#"{{"type":"log","text":"line #{i}"}}"#).unwrap();
                    }
                }
                out.flush().unwrap();
                let text = v["args"]["text"].as_str().unwrap_or("").to_uppercase();
                writeln!(
                    out,
                    r#"{{"type":"result","id":{id},"is_error":false,"text":"{text}","details":{{"fixture":true}}}}"#
                )
                .unwrap();
                out.flush().unwrap();
            }
            _ => {
                let text = v["args"]["text"].as_str().unwrap_or("").to_uppercase();
                let reply = serde_json::json!({
                    "type": "result", "id": id, "is_error": false,
                    "text": text, "details": {"fixture": true}
                });
                writeln!(out, "{reply}").unwrap();
                out.flush().unwrap();
            }
        }
    }
}
