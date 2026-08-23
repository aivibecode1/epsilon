//! Fixture extension child used by host integration tests.
//!
//! Modes (argv[1]): none = echo uppercase | `--sleep` = stall past invoke timeout |
//! `--die` = exit right after handshake (crash-budget fixture).

use std::io::{BufRead, BufWriter, Write};

fn main() {
    let mode = std::env::args().nth(1).unwrap_or_default();
    let mut events_seen: usize = 0;
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut out = BufWriter::new(stdout.lock());

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
        match mode.as_str() {
            "--die" => std::process::exit(7),
            "--sleep" => std::thread::sleep(std::time::Duration::from_secs(60)),
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
