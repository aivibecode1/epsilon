//! Local HTTP/SSE frontend (M9b, ADR-011).
//!
//! One session runtime beneath all frontends: this crate exposes that runtime over
//! HTTP — SSE for live [`AgentEvent`]s, small REST endpoints for prompts, steering,
//! and permission decisions. Composition (which provider/tools/ruleset) stays in the
//! CLI; everything here is agnostic to those choices.
//!
//! Permission asks park in the shared [`ChannelApprovalUi`]; remote clients discover
//! them via `GET /approvals` and answer via `POST /approvals/{id}` — the same mechanism
//! the TUI dialog drives locally.

use std::convert::Infallible;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use axum::Json;
use axum::extract::{Path, State};
use axum::response::sse::{Event, KeepAlive, Sse};
mod openapi;
use axum::routing::{get, post};
use axum::{Router, http::StatusCode, response::IntoResponse};
use epsilon_contracts::content::UserContent;
use epsilon_contracts::message::{Message, UserMessage};
use epsilon_contracts::new_id;
use epsilon_core::Agent;
use epsilon_safety::ChannelApprovalUi;
use futures_util::Stream;
use serde::Deserialize;

/// Shared handle every route operates on.
#[derive(Clone)]
pub struct ServeState {
    /// The single agent instance behind this server.
    pub agent: Arc<Agent>,
    /// Where permission asks park until a client answers them.
    pub approvals: Arc<ChannelApprovalUi>,
    run_depth: Arc<AtomicUsize>,
}

impl ServeState {
    /// Bundle an agent with its approval queue.
    pub fn new(agent: Arc<Agent>, approvals: Arc<ChannelApprovalUi>) -> Self {
        Self {
            agent,
            approvals,
            run_depth: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Number of runs currently executing (diagnostics/tests).
    pub fn active_runs(&self) -> usize {
        self.run_depth.load(Ordering::SeqCst)
    }

    fn user_message(&self, text: String) -> Message {
        Message::User(UserMessage {
            id: new_id("msg"),
            timestamp_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
            content: vec![UserContent::Text { text }],
        })
    }
}

/// Request body of `POST /prompt`.
#[derive(Debug, Deserialize)]
pub struct PromptBody {
    /// The task text; becomes the user message.
    pub task: String,
}

/// Request body of `POST /steer` and `POST /followup`.
#[derive(Debug, Deserialize)]
pub struct TextBody {
    /// Interjection text.
    pub text: String,
}

fn empty_body_error() -> StatusCode {
    StatusCode::UNPROCESSABLE_ENTITY
}

/// Build the application router.
///
/// Routes:
/// - `GET  /healthz`            liveness
/// - `POST /prompt`             start a run (`409` when one is already active)
/// - `POST /steer`              queue mid-run interjection
/// - `POST /followup`           queue post-run follow-up
/// - `GET  /events`             SSE stream of every [`AgentEvent`] as JSON data frames
/// - `GET  /approvals`          pending permission asks, oldest first
/// - `POST /approvals/{id}`     answer one ask with a [`PermissionDecision`]
pub fn router(state: ServeState) -> Router {
    Router::new()
        .route(
            "/healthz",
            get(|| async { Json(serde_json::json!({"ok": true})) }),
        )
        .route("/openapi.json", get(|| async { Json(openapi::spec()) }))
        .route("/prompt", post(prompt))
        .route("/steer", post(steer))
        .route("/followup", post(followup))
        .route("/events", get(events))
        .route("/approvals", get(list_approvals))
        .route("/approvals/{request_id}", post(resolve_approval))
        .with_state(state)
}

/// Serve on an already-bound listener until the process is killed.
pub async fn serve(state: ServeState, listener: tokio::net::TcpListener) -> std::io::Result<()> {
    axum::serve(listener, router(state)).await
}

async fn prompt(State(st): State<ServeState>, body: Option<Json<PromptBody>>) -> impl IntoResponse {
    let Some(Json(PromptBody { task })) = body else {
        return (
            empty_body_error(),
            "body must be {\"task\": \"…\"}".to_string(),
        );
    };
    if task.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            "task must not be empty".to_string(),
        );
    }
    // Single-run invariant mirrors Agent's own guard; fail fast with 409 instead of
    // accepting work that would die with AlreadyRunning.
    if st.active_runs() > 0 {
        return (
            StatusCode::CONFLICT,
            "another run is already active".to_string(),
        );
    }
    let msg = st.user_message(task);
    let depth = Arc::clone(&st.run_depth);
    let agent = Arc::clone(&st.agent);
    depth.fetch_add(1, Ordering::SeqCst);
    tokio::spawn(async move {
        let _ = agent.run(msg).await;
        depth.fetch_sub(1, Ordering::SeqCst);
    });
    (StatusCode::ACCEPTED, "run started".to_string())
}

async fn steer(State(st): State<ServeState>, body: Option<Json<TextBody>>) -> impl IntoResponse {
    let Some(Json(TextBody { text })) = body else {
        return (
            empty_body_error(),
            "body must be {\"text\": \"…\"}".to_string(),
        );
    };
    st.agent.steer(st.user_message(text));
    (StatusCode::ACCEPTED, "steering queued".to_string())
}

async fn followup(State(st): State<ServeState>, body: Option<Json<TextBody>>) -> impl IntoResponse {
    let Some(Json(TextBody { text })) = body else {
        return (
            empty_body_error(),
            "body must be {\"text\": \"…\"}".to_string(),
        );
    };
    st.agent.follow_up(st.user_message(text));
    (StatusCode::ACCEPTED, "follow-up queued".to_string())
}

async fn list_approvals(State(st): State<ServeState>) -> impl IntoResponse {
    Json(serde_json::json!({ "pending": st.approvals.pending() }))
}

async fn resolve_approval(
    State(st): State<ServeState>,
    Path(request_id): Path<String>,
    Json(decision): Json<epsilon_contracts::agent_event::PermissionDecision>,
) -> impl IntoResponse {
    if st.approvals.resolve(&request_id, decision) {
        (StatusCode::OK, "resolved".to_string())
    } else {
        (
            StatusCode::NOT_FOUND,
            "unknown or already resolved".to_string(),
        )
    }
}

async fn events(
    State(st): State<ServeState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let rx = st.agent.subscribe();
    // Every recv outcome resolves this unfold step: events and lag notices yield frames,
    // closure ends the stream — no loop needed (clippy: never_loops).
    let stream = futures_util::stream::unfold(rx, |mut rx| async move {
        match rx.recv().await {
            Ok(ev) => {
                let data = serde_json::to_string(&*ev)
                    .unwrap_or_else(|e| format!(r#"{{"type":"serialize_error","detail":"{e}"}}"#));
                Some((Ok(Event::default().data(data)), rx))
            }
            Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => Some((
                Ok(Event::default()
                    .event("lagged")
                    .data(format!(r#"{{"dropped":{n}}}"#))),
                rx,
            )),
            Err(tokio::sync::broadcast::error::RecvError::Closed) => None,
        }
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use epsilon_llm::{FauxProvider, FauxStep, PanicGuard};
    use epsilon_safety::RulesetGate;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    fn test_agent(steps: Vec<FauxStep>) -> (ServeState, Router) {
        let approvals = Arc::new(ChannelApprovalUi::new());
        let gate = Arc::new(RulesetGate::new(
            Arc::clone(&approvals) as Arc<dyn epsilon_safety::ApprovalUi>
        ));
        let agent = Agent::new(
            epsilon_core::LoopLimits::default(),
            Arc::new(PanicGuard(FauxProvider::new(steps))),
            Arc::new(epsilon_core::ToolRegistry::new()),
            gate,
            std::env::temp_dir(),
        );
        let state = ServeState::new(agent, approvals);
        let app = router(state.clone());
        (state, app)
    }

    async fn json_response(
        app: Router,
        method: &str,
        uri: &str,
        body: Option<String>,
    ) -> (u16, String) {
        let builder = axum::http::Request::builder().method(method).uri(uri);
        let req = match body {
            Some(b) => builder
                .header("content-type", "application/json")
                .body(Body::from(b))
                .unwrap(),
            None => builder.body(Body::empty()).unwrap(),
        };
        let res = app.oneshot(req).await.unwrap();
        let status = res.status().as_u16();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    #[tokio::test]
    async fn healthz_reports_ok() {
        let (_st, app) = test_agent(vec![FauxStep::say("hi")]);
        let (status, body) = json_response(app, "GET", "/healthz", None).await;
        assert_eq!(status, 200);
        assert!(body.contains("\"ok\":true"), "{body}");
    }

    #[tokio::test]
    async fn prompt_rejects_empty_and_conflicts_then_completes() {
        let (st, app) = test_agent(vec![FauxStep::say("done")]);
        let (status, _) = json_response(
            app.clone(),
            "POST",
            "/prompt",
            Some(r#"{"task": "  "}"#.into()),
        )
        .await;
        assert_eq!(status, 400);

        let (status, _) = json_response(
            app.clone(),
            "POST",
            "/prompt",
            Some(r#"{"task": "go"}"#.into()),
        )
        .await;
        assert_eq!(status, 202);

        while st.active_runs() > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        let (status, _) = json_response(
            app.clone(),
            "POST",
            "/prompt",
            Some(r#"{"task": "again"}"#.into()),
        )
        .await;
        assert_eq!(status, 202, "slot freed after completion");
    }

    #[tokio::test]
    async fn approval_round_trip_over_rest_unblocks_a_bash_ask() {
        // Default ruleset sends bash to ask; the faux model requests bash immediately.
        let steps = vec![
            FauxStep::call(
                "tc_1",
                "bash",
                serde_json::json!({"command": "echo served"}),
            ),
            FauxStep::say("approved run finished"),
        ];
        let approvals_src = Arc::new(ChannelApprovalUi::new());
        let gate = Arc::new(RulesetGate::new(
            Arc::clone(&approvals_src) as Arc<dyn epsilon_safety::ApprovalUi>
        ));
        let registry = Arc::new(epsilon_tools::builtin_registry(
            &epsilon_tools::Confinement::Auto,
        ));
        let agent = Agent::new(
            epsilon_core::LoopLimits::default(),
            Arc::new(PanicGuard(FauxProvider::new(steps))),
            registry,
            gate,
            std::env::temp_dir(),
        );
        let st = ServeState::new(Arc::clone(&agent), Arc::clone(&approvals_src));
        let app = router(st.clone());

        let (status, _) = json_response(
            app.clone(),
            "POST",
            "/prompt",
            Some(r#"{"task": "run bash"}"#.into()),
        )
        .await;
        assert_eq!(status, 202);

        // Ask parks; REST resolves it.
        let mut pending = Vec::new();
        for _ in 0..200 {
            let (_, body) = json_response(app.clone(), "GET", "/approvals", None).await;
            pending = serde_json::from_str::<serde_json::Value>(&body).unwrap()["pending"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            if !pending.is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        assert!(!pending.is_empty(), "ask never surfaced over REST");
        let request_id = pending[0]["request_id"].as_str().unwrap().to_string();

        let (status, _) = json_response(
            app.clone(),
            "POST",
            &format!("/approvals/{request_id}"),
            Some(r#"{"type":"allow_once"}"#.into()),
        )
        .await;
        assert_eq!(status, 200);

        while st.active_runs() > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        let transcript: Vec<String> = agent
            .history()
            .iter()
            .filter_map(|m| match m {
                Message::ToolResult(t) => Some(
                    t.content
                        .iter()
                        .map(|c| match c {
                            epsilon_contracts::content::ContentBlock::Text { text } => text.clone(),
                            _ => String::new(),
                        })
                        .collect::<String>(),
                ),
                _ => None,
            })
            .collect();
        assert!(
            transcript.iter().any(|t| t.contains("served")),
            "bash output missing after approval: {transcript:?}"
        );
    }

    #[tokio::test]
    async fn sse_streams_live_events_over_a_real_socket() {
        let (st, app) = test_agent(vec![FauxStep::say("hello over sse")]);
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind");
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(serve(st.clone(), listener));

        // Subscribe BEFORE prompting so no event is missed: wait for the handshake
        // headers, which proves the events handler is now streaming.
        let mut sock = tokio::net::TcpStream::connect(addr).await.expect("connect");
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        sock.write_all(b"GET /events HTTP/1.1\r\nHost: t\r\nAccept: text/event-stream\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();

        let mut buf = Vec::new();
        while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
            let mut chunk = [0u8; 1024];
            let n = tokio::time::timeout(std::time::Duration::from_secs(10), sock.read(&mut chunk))
                .await
                .expect("handshake timed out")
                .expect("socket read during handshake");
            assert!(n > 0, "server closed during SSE handshake");
            buf.extend_from_slice(&chunk[..n]);
        }

        let (status, _) = json_response(
            app.clone(),
            "POST",
            "/prompt",
            Some(r#"{"task": "sse please"}"#.into()),
        )
        .await;
        assert_eq!(status, 202);

        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
        let needle = b"agent_end";
        while !buf.windows(needle.len()).any(|w| w == needle) {
            if tokio::time::Instant::now() > deadline {
                panic!(
                    "no agent_end frame within timeout; got {}",
                    String::from_utf8_lossy(&buf)
                );
            }
            let mut chunk = [0u8; 4096];
            let n = tokio::time::timeout(std::time::Duration::from_secs(5), sock.read(&mut chunk))
                .await
                .unwrap_or_else(|_| {
                    panic!(
                        "read timed out mid-stream; buffered so far: {:?}",
                        String::from_utf8_lossy(&buf)
                    )
                })
                .expect("socket read");
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
        }
        let text = String::from_utf8_lossy(&buf).into_owned();
        assert!(text.contains("200 OK"), "missing HTTP status: {text}");
        assert!(text.contains("text/event-stream"), "{text}");
        assert!(
            text.contains("hello over sse"),
            "assistant text must ride an SSE data frame: {text}"
        );
        server.abort();
        let _ = server.await;
    }
}
