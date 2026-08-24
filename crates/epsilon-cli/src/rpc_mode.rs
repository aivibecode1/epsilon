//! rpc mode (ADR-011): line-delimited JSON over stdio for editor embedding.
//!
//! Protocol: each inbound line is one request `{"op": "...", ...}`; every AgentEvent
//! is streamed outbound as `{"event": {...}}` as it happens. Replies correlate by
//! arrival order per op (editors serialize requests). Same runtime + approval
//! mechanics as serve mode — the transport is the only difference.

use std::sync::Arc;

use epsilon_contracts::agent_event::PermissionDecision;
use epsilon_core::Agent;
use epsilon_safety::ChannelApprovalUi;
use serde::Deserialize;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

/// Inbound operations.
#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum Op {
    /// Start a run with a user task.
    Prompt { task: String },
    /// Queue a mid-run interjection.
    Steer { text: String },
    /// Queue post-run work.
    Followup { text: String },
    /// List pending permission asks.
    Approvals,
    /// Answer one ask.
    Approve {
        id: String,
        decision: PermissionDecision,
    },
    /// Liveness.
    Ping,
}

fn user_message(text: String) -> epsilon_contracts::message::Message {
    epsilon_contracts::message::Message::User(epsilon_contracts::message::UserMessage {
        id: epsilon_contracts::new_id("msg"),
        timestamp_ms: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0),
        content: vec![epsilon_contracts::content::UserContent::Text { text }],
    })
}

/// Run the stdio loop until stdin closes. Returns the process exit code.
pub async fn run(agent: Arc<Agent>, approvals: Arc<ChannelApprovalUi>) -> i32 {
    let stdin = tokio::io::stdin();
    let mut lines = tokio::io::BufReader::new(stdin).lines();
    let stdout = tokio::io::stdout();
    let writer = Arc::new(tokio::sync::Mutex::new(stdout));

    // Event pump: every lifecycle event becomes an outbound frame.
    let mut events_rx = agent.subscribe();
    let pump_writer = Arc::clone(&writer);
    tokio::spawn(async move {
        loop {
            match events_rx.recv().await {
                Ok(ev) => {
                    let frame = format!(
                        "{{\"event\":{}}}\n",
                        serde_json::to_string(&*ev).unwrap_or_else(|_| "null".into())
                    );
                    let mut w = pump_writer.lock().await;
                    w.write_all(frame.as_bytes()).await.ok();
                    w.flush().await.ok();
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    while let Ok(Some(line)) = lines.next_line().await {
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Op>(&line) {
            Ok(Op::Ping) => ok_json(serde_json::json!({"pong": true})),
            Ok(Op::Prompt { task }) => {
                if agent_is_busy(&agent) {
                    err_json("another run is already active")
                } else {
                    let a = Arc::clone(&agent);
                    tokio::spawn(async move {
                        let _ = a.run(user_message(task)).await;
                    });
                    ok_json(serde_json::json!({"started": true}))
                }
            }
            Ok(Op::Steer { text }) => {
                agent.steer(user_message(text));
                ok_json(serde_json::json!({"queued": "steer"}))
            }
            Ok(Op::Followup { text }) => {
                agent.follow_up(user_message(text));
                ok_json(serde_json::json!({"queued": "followup"}))
            }
            Ok(Op::Approvals) => ok_json(serde_json::json!({
                "pending": approvals.pending()
            })),
            Ok(Op::Approve { id, decision }) => {
                if approvals.resolve(&id, decision) {
                    ok_json(serde_json::json!({"resolved": true}))
                } else {
                    err_json("unknown or already resolved request id")
                }
            }
            Err(e) => err_json(format!("bad request: {e}")),
        };

        let mut w = writer.lock().await;
        w.write_all(reply.as_bytes()).await.ok();
        w.write_all(b"\n").await.ok();
        w.flush().await.ok();
    }
    0
}

fn agent_is_busy(_agent: &Arc<Agent>) -> bool {
    // Mirrors the server's active_runs guard; Agent rejects concurrent runs itself, so
    // this is advisory only (the spawned run reports AlreadyRunning through events).
    false
}

fn ok_json(v: serde_json::Value) -> String {
    format!("{{\"ok\":{v}}}")
}

fn err_json(msg: impl std::fmt::Display) -> String {
    format!(
        "{{\"error\":{}}}",
        serde_json::to_string(&msg.to_string()).unwrap_or_else(|_| "\"error\"".into())
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ops_deserialize_with_snake_tags() {
        let p: Op = serde_json::from_str(r#"{"op":"prompt","task":"x"}"#).unwrap();
        assert!(matches!(p, Op::Prompt { .. }));
        let a: Op =
            serde_json::from_str(r#"{"op":"approve","id":"p1","decision":{"type":"allow_once"}}"#)
                .unwrap();
        assert!(matches!(a, Op::Approve { .. }));
        assert!(serde_json::from_str::<Op>(r#"{"op":"nope"}"#).is_err());
    }

    #[test]
    fn replies_are_single_lines() {
        assert!(!ok_json(serde_json::json!({"a":1})).contains('\n'));
        assert!(err_json("boom").contains("\"boom\""));
    }
}
