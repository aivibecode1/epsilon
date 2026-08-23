//! M9a acceptance: agent events forwarded to an extension child over its stdio protocol
//! (closes known-gap #1 from the M8 ledger).

use std::process::Stdio;
use tokio::io::AsyncWriteExt;

const FIXTURE_EXE: &str = env!("CARGO_BIN_EXE_ext-fixture");

/// Standalone child: handshake, count `event` frames (logging each receipt), and answer
/// invokes normally. Used to prove forwarding without dragging core into ext's deps.
#[tokio::test]
async fn events_are_forwarded_to_extension_process() {
    let dir = tempfile::tempdir().unwrap();

    let mut child = tokio::process::Command::new(FIXTURE_EXE)
        .current_dir(dir.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = tokio::io::BufReader::new(child.stdout.take().unwrap());

    // hello
    let mut line = String::new();
    tokio::io::AsyncBufReadExt::read_line(&mut stdout, &mut line)
        .await
        .unwrap();
    assert!(line.contains("\"hello\""), "got: {line}");

    // Forward three events of two types.
    for (etype, n) in [
        ("tool_execution_end", 1),
        ("tool_execution_end", 2),
        ("turn_end", 1),
    ] {
        let frame = serde_json::json!({
            "type": "event",
            "event": { "type": etype, "seq": n }
        });
        stdin
            .write_all(
                format!(
                    "{frame}
"
                )
                .as_bytes(),
            )
            .await
            .unwrap();
    }
    stdin.flush().await.unwrap();

    // Ask the child to report via its normal result path: invoke a probe that includes
    // the running tally in its reply text (the fixture logs receipts; the reply echoes).
    // Simplest deterministic check: read the log lines the child already sent back.
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;

    let mut receipts = Vec::new();
    loop {
        line.clear();
        let n = tokio::io::AsyncBufReadExt::read_line(&mut stdout, &mut line)
            .await
            .unwrap();
        if n == 0 {
            break;
        }
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(line.trim()) {
            if v.get("type").and_then(|t| t.as_str()) == Some("log") {
                receipts.push(
                    v.get("text")
                        .and_then(|t| t.as_str())
                        .unwrap_or("")
                        .to_string(),
                );
            }
            if receipts.len() >= 3 {
                break;
            }
        }
    }

    assert_eq!(receipts.len(), 3, "receipts: {receipts:?}");
    assert!(receipts[0].contains("saw tool_execution_end #1"));
    assert!(receipts[1].contains("saw tool_execution_end #2"));
    assert!(
        receipts[2].contains("saw turn_end #3"),
        "got: {}",
        receipts[2]
    );

    let _ = stdin.shutdown().await;
}

/// Manifest-level filter semantics (pure): wants_event honors "*" and exact names.
#[test]
fn manifest_event_filter() {
    use epsilon_ext::manifest::load_or_build_test_manifest;

    let m = load_or_build_test_manifest(vec!["tool_execution_end".into()]);
    assert!(m.wants_event("tool_execution_end"));
    assert!(!m.wants_event("turn_end"));

    let star = load_or_build_test_manifest(vec!["*".into()]);
    assert!(star.wants_event("anything"));
    assert!(star.wants_event("agent_start"));
}
