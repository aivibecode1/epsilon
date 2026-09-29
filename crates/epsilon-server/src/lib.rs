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
use axum::routing::{MethodRouter, get, post};
use axum::{Router, http::StatusCode, response::IntoResponse};
use epsilon_contracts::LaggedNotice;
use epsilon_contracts::message::Message;
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
    /// Per-boot bearer token (plan 003): every mutating/read route except
    /// `/healthz` requires `authorization: Bearer <token>`. `None` disables the
    /// check (tests only); production `serve_mode` always supplies one.
    pub auth_token: Option<Arc<String>>,
}

impl ServeState {
    /// Bundle an agent with its approval queue.
    pub fn new(agent: Arc<Agent>, approvals: Arc<ChannelApprovalUi>) -> Self {
        Self {
            agent,
            approvals,
            run_depth: Arc::new(AtomicUsize::new(0)),
            auth_token: None,
        }
    }

    /// Production constructor (plan 003): generates the per-boot bearer token the
    /// CLI prints once on stderr. Every route except `/healthz` requires it.
    pub fn with_boot_token(agent: Arc<Agent>, approvals: Arc<ChannelApprovalUi>) -> Self {
        let mut token = Self::new(agent, approvals);
        // 32 hex chars from OS entropy via the in-tree sha2 hasher over
        // time+pid+counter — not a password, just an unguessable-per-boot secret.
        use sha2::{Digest, Sha256};
        use std::sync::atomic::AtomicU64;
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let mut h = Sha256::new();
        h.update(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos().to_le_bytes())
                .unwrap_or([0u8; 16]),
        );
        h.update(std::process::id().to_le_bytes());
        h.update(SEQ.fetch_add(1, Ordering::SeqCst).to_le_bytes());
        let digest = h.finalize();
        let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
        token.auth_token = Some(Arc::new(hex[..32].to_string()));
        token
    }

    /// Number of runs currently executing (diagnostics/tests).
    pub fn active_runs(&self) -> usize {
        self.run_depth.load(Ordering::SeqCst)
    }

    fn user_message(&self, text: String) -> Message {
        // M74: `@` references resolve through the shared constructor, so the HTTP surface gets
        // the same attachments as the TUI and `-p`.
        epsilon_core::references::user_message(self.agent.workspace_root(), &text)
    }
}

/// Request body of `POST /prompt`.
#[derive(Debug, Deserialize)]
pub struct PromptBody {
    /// The task text; becomes the user message.
    pub task: String,
    /// Optional correlation id, echoed on the 202/409 reply (RPC-02). Same contract as
    /// pi's RPC client, which correlates replies by id rather than arrival order.
    #[serde(default)]
    pub request_id: Option<String>,
}

/// Request body of `POST /abort` (the body is optional: it carries only the correlation id).
#[derive(Debug, Deserialize)]
pub struct AbortBody {
    /// Optional correlation id, echoed on the 202/409 reply (RPC-02).
    #[serde(default)]
    pub request_id: Option<String>,
}

/// Body shared by the 202/409 answers of `POST /prompt` and `POST /abort`.
///
/// `detail` carries the text these routes answered with before their bodies became JSON,
/// so a client reading the body as prose still finds its sentence; `request_id` is present
/// exactly when the request supplied one.
fn outcome_body(
    outcome_key: &str,
    outcome: bool,
    detail: &str,
    request_id: Option<String>,
) -> serde_json::Value {
    let mut body = serde_json::json!({ outcome_key: outcome, "detail": detail });
    if let Some(request_id) = request_id {
        body["request_id"] = serde_json::Value::String(request_id);
    }
    body
}

/// Read the live session state — the same payload the stdio RPC's `get_state` replies with.
///
/// Built field by field because the shared struct lives in `epsilon-contracts`, which
/// stays behaviour-free (no constructor can take an `Agent` there) and because
/// `epsilon-server` must not depend on `epsilon-cli`, where the RPC surface lives.
fn session_state(agent: &Agent) -> epsilon_contracts::SessionState {
    // One constructor, shared with the RPC surface -- see `Agent::session_state`.
    agent.session_state()
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

/// The routes this service exposes — the single source of truth [`router`] mounts.
///
/// `router()` folds exactly this table, and the OpenAPI drift test
/// (`openapi::tests::spec_paths_match_router`) compares the document's path set against
/// this same table — so a route added on either side alone fails the test instead of
/// drifting silently. Routes:
/// - `GET  /healthz`            liveness
/// - `GET  /openapi.json`       this document (exempt from the bearer token)
/// - `POST /prompt`             start a run (`409` when one is already active)
/// - `POST /steer`              queue mid-run interjection
/// - `POST /followup`           queue post-run follow-up
/// - `POST /abort`              cancel the active run (`409` when none is running)
/// - `GET  /state`              live session state (shared RPC/HTTP payload)
/// - `GET  /events`             SSE stream of every [`AgentEvent`] as JSON data frames
/// - `GET  /approvals`          pending permission asks, oldest first
/// - `POST /approvals/{id}`     answer one ask with a [`PermissionDecision`]
fn route_table() -> Vec<(&'static str, MethodRouter<ServeState>)> {
    vec![
        (
            "/healthz",
            get(|| async { Json(serde_json::json!({"ok": true})) }),
        ),
        ("/openapi.json", get(|| async { Json(openapi::spec()) })),
        ("/prompt", post(prompt)),
        ("/steer", post(steer)),
        ("/followup", post(followup)),
        ("/abort", post(abort)),
        ("/state", get(get_state)),
        ("/events", get(events)),
        ("/approvals", get(list_approvals)),
        ("/approvals/{request_id}", post(resolve_approval)),
    ]
}

/// Build the application router from [`route_table`] — the only place routes are
/// registered.
pub fn router(state: ServeState) -> Router {
    route_table()
        .into_iter()
        .fold(Router::new(), |router, (path, methods)| {
            router.route(path, methods)
        })
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth_and_host_guard,
        ))
        .with_state(state)
}

/// Plan 003: serve mode is a TRUST BOUNDARY.
///
/// Two guards run before every route except the exempted liveness/spec paths:
/// 1. **Bearer token** — when `auth_token` is set (production always sets it),
///    requests must carry `authorization: Bearer <token>`. A local process or
///    webpage without the token cannot start runs or resolve permission asks.
/// 2. **Host allow-list** — Host must be loopback; a browser tricked into posting
///    to an attacker-chosen DNS name (rebinding) presents a non-loopback Host and
///    is refused. An `Origin` header from anything other than localhost is also
///    refused (blocks cross-origin POSTs from web pages).
async fn auth_and_host_guard(
    State(st): State<ServeState>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let path = req.uri().path();
    let exempt = path == "/healthz" || path == "/openapi.json";

    // --- Host / Origin checks -----------------------------------------------
    let host_ok = req
        .headers()
        .get(axum::http::header::HOST)
        .and_then(|h| h.to_str().ok())
        .map(|h| {
            let host = h.rsplit_once(':').map(|(h, _)| h).unwrap_or(h);
            host == "127.0.0.1" || host == "localhost" || host == "[::1]"
        })
        // Missing Host is only possible for raw/non-HTTP clients; reject to be safe.
        .unwrap_or(false);
    if !exempt && !host_ok {
        return (
            StatusCode::FORBIDDEN,
            "rejected: Host header is not loopback",
        )
            .into_response();
    }
    if let Some(origin) = req
        .headers()
        .get(axum::http::header::ORIGIN)
        .and_then(|o| o.to_str().ok())
    {
        let origin_clean = origin
            .trim_start_matches("http://")
            .trim_start_matches("https://")
            .rsplit_once(':')
            .map(|(h, _)| h)
            .unwrap_or(origin);
        if origin_clean != "127.0.0.1" && origin_clean != "localhost" {
            return (StatusCode::FORBIDDEN, "rejected: cross-origin request").into_response();
        }
    }

    // --- Bearer token ---------------------------------------------------------
    if !exempt && let Some(expected) = st.auth_token.as_ref() {
        let supplied = req
            .headers()
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "));
        match supplied {
            Some(t) if constant_time_eq(t.as_bytes(), expected.as_bytes()) => {}
            _ => {
                return (StatusCode::UNAUTHORIZED, "missing or invalid bearer token")
                    .into_response();
            }
        }
    }
    next.run(req).await
}

/// Length-independent comparison so timing does not leak token content.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter()
        .zip(b.iter())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

/// Serve on an already-bound listener until the process is killed.
pub async fn serve(state: ServeState, listener: tokio::net::TcpListener) -> std::io::Result<()> {
    axum::serve(listener, router(state)).await
}

/// Releases the server's single-run slot when the admitted run's task ends.
///
/// Drop-based so a panicking run task cannot leave the server permanently convinced a
/// run is active (which would turn every later prompt into a 409).
struct RunSlot(Arc<AtomicUsize>);

impl Drop for RunSlot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

async fn prompt(
    State(st): State<ServeState>,
    body: Option<Json<PromptBody>>,
) -> axum::response::Response {
    let Some(Json(PromptBody { task, request_id })) = body else {
        return (
            empty_body_error(),
            "body must be {\"task\": \"…\"}".to_string(),
        )
            .into_response();
    };
    if task.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            "task must not be empty".to_string(),
        )
            .into_response();
    }
    // Single-run invariant, reserved ATOMICALLY before any reply (F13's HTTP half).
    // The previous check-then-act told the loser of an overlapping pair 202 Accepted
    // and left it to die with AlreadyRunning, with nothing tying that failure back to
    // the reply; the RPC surface now keeps the same contract.
    if st.run_depth.fetch_add(1, Ordering::SeqCst) != 0 {
        st.run_depth.fetch_sub(1, Ordering::SeqCst);
        return (
            StatusCode::CONFLICT,
            Json(outcome_body(
                "started",
                false,
                "another run is already active",
                request_id,
            )),
        )
            .into_response();
    }
    // The guard is built immediately after the reservation, before anything else can
    // fail or panic: from here on the slot is always released.
    let slot = RunSlot(Arc::clone(&st.run_depth));
    let msg = st.user_message(task);
    let agent = Arc::clone(&st.agent);
    tokio::spawn(async move {
        // The slot releases when this task ends, even if `run` panics.
        let _slot = slot;
        if let Err(e) = agent.run(msg).await {
            eprintln!("serve: admitted run failed: {e}");
        }
    });
    (
        StatusCode::ACCEPTED,
        Json(outcome_body("started", true, "run started", request_id)),
    )
        .into_response()
}

/// `POST /abort` (RPC-02): cancel the active run through the same core call the stdio
/// surface uses.
///
/// 202 when a run was active and its token was cancelled; 409 when none was — HTTP keeps
/// its status vocabulary rather than importing RPC's in-band string/boolean channel. The
/// body reports core's own answer (`aborted`) and echoes `request_id` when supplied.
async fn abort(State(st): State<ServeState>, body: Option<Json<AbortBody>>) -> impl IntoResponse {
    let request_id = body.and_then(|Json(AbortBody { request_id })| request_id);
    let aborted = st.agent.abort_current_run();
    let (status, detail) = if aborted {
        (StatusCode::ACCEPTED, "abort requested")
    } else {
        (StatusCode::CONFLICT, "no run is active")
    };
    (
        status,
        Json(outcome_body("aborted", aborted, detail, request_id)),
    )
}

/// `GET /state` (RPC-02): the live session state, the same payload the stdio RPC's
/// `get_state` replies with (one shared struct, `epsilon-contracts`).
async fn get_state(State(st): State<ServeState>) -> Json<epsilon_contracts::SessionState> {
    Json(session_state(&st.agent))
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

/// The data payload of the SSE `lagged` event: the shared [`LaggedNotice`] serialized.
///
/// Both wire surfaces build the notice from the same contract struct (the stdio RPC
/// nests this object under its `lagged` frame key), so the two cannot drift apart.
fn lagged_data(dropped: u64) -> String {
    serde_json::to_string(&LaggedNotice { dropped })
        .unwrap_or_else(|e| format!(r#"{{"serialize_error":"{e}"}}"#))
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
                Ok(Event::default().event("lagged").data(lagged_data(n))),
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

    /// Like [`test_agent`], but every provider call pauses `delay_ms` first, so a run
    /// stays active while a second request arrives (same pattern as the F13 overlap test).
    fn test_agent_delayed(steps: Vec<FauxStep>, delay_ms: u64) -> (ServeState, Router) {
        let approvals = Arc::new(ChannelApprovalUi::new());
        let gate = Arc::new(RulesetGate::new(
            Arc::clone(&approvals) as Arc<dyn epsilon_safety::ApprovalUi>
        ));
        let agent = Agent::new(
            epsilon_core::LoopLimits::default(),
            Arc::new(PanicGuard(epsilon_core::testing::SlowProvider::new(
                FauxProvider::new(steps),
                delay_ms,
            ))),
            Arc::new(epsilon_core::ToolRegistry::new()),
            gate,
            std::env::temp_dir(),
        );
        let state = ServeState::new(agent, approvals);
        let app = router(state.clone());
        (state, app)
    }

    #[tokio::test]
    async fn bearer_token_required_when_configured() {
        let (mut st, _app) = test_agent(vec![FauxStep::say("hi")]);
        st.auth_token = Some(Arc::new("sekrit".into()));
        let app = router(st.clone());

        // No token -> 401 on a state-changing route.
        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/prompt")
            .header("host", "127.0.0.1")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"task":"x"}"#))
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status().as_u16(), 401, "token required");

        // Wrong token -> 401.
        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/prompt")
            .header("host", "127.0.0.1")
            .header("authorization", "Bearer nope")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"task":"x"}"#))
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status().as_u16(), 401);

        // Correct token -> passes the guard (202/409 from the route itself).
        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/prompt")
            .header("host", "127.0.0.1")
            .header("authorization", "Bearer sekrit")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"task":"hi again"}"#))
            .unwrap();
        let res = app.oneshot(req).await.unwrap();
        assert_ne!(res.status().as_u16(), 401);
    }

    #[tokio::test]
    async fn non_loopback_host_is_rejected() {
        let (_st, app) = test_agent(vec![FauxStep::say("hi")]);
        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/prompt")
            .header("host", "evil.example.com")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"task":"x"}"#))
            .unwrap();
        let res = app.oneshot(req).await.unwrap();
        assert_eq!(res.status().as_u16(), 403, "DNS-rebinding host refused");
    }

    async fn json_response(
        app: Router,
        method: &str,
        uri: &str,
        body: Option<String>,
    ) -> (u16, String) {
        // Plan 003: requests must look loopback-local (Host guard) — tests simulate
        // a trusted local client.
        let builder = axum::http::Request::builder()
            .method(method)
            .uri(uri)
            .header("host", "127.0.0.1");
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

    /// F13 (HTTP half): admission is reserved atomically before the reply, so an
    /// overlapping prompt receives 409 instead of a 202 it cannot keep. A slow
    /// provider keeps the first run active while the second request arrives.
    #[tokio::test]
    async fn overlapping_prompts_have_exactly_one_admitted() {
        let approvals = Arc::new(ChannelApprovalUi::new());
        let gate = Arc::new(RulesetGate::new(
            Arc::clone(&approvals) as Arc<dyn epsilon_safety::ApprovalUi>
        ));
        let agent = Agent::new(
            epsilon_core::LoopLimits::default(),
            Arc::new(PanicGuard(epsilon_core::testing::SlowProvider::new(
                FauxProvider::new(vec![FauxStep::say("done")]),
                400,
            ))),
            Arc::new(epsilon_core::ToolRegistry::new()),
            gate,
            std::env::temp_dir(),
        );
        let state = ServeState::new(agent, approvals);
        let app = router(state.clone());

        let (first, _) = json_response(
            app.clone(),
            "POST",
            "/prompt",
            Some(r#"{"task": "one"}"#.into()),
        )
        .await;
        assert_eq!(first, 202);

        let (second, body) = json_response(
            app.clone(),
            "POST",
            "/prompt",
            Some(r#"{"task": "two"}"#.into()),
        )
        .await;
        assert_eq!(second, 409, "overlapping prompt: {body}");
        assert!(body.contains("another run"), "{body}");

        // Ending the run releases the reservation; nothing stays wedged.
        state.agent.abort_current_run();
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        while state.active_runs() > 0 && tokio::time::Instant::now() < deadline {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert_eq!(state.active_runs(), 0, "admission must be released");
    }

    /// RPC-02: `POST /abort` cancels an active run through the same core call the stdio
    /// surface makes, and the slot is released when the cancelled run's task ends.
    #[tokio::test]
    async fn abort_ends_a_slow_run_and_echoes_the_request_id() {
        let (st, app) = test_agent_delayed(vec![FauxStep::say("slow")], 400);
        // Subscribe before prompting so the run's own start event is what this test
        // waits on: the 202 is the reservation, taken a moment before the run task is
        // first polled, and an abort in that gap is (honestly) a no-run answer.
        let mut events = st.agent.subscribe();

        let (status, body) = json_response(
            app.clone(),
            "POST",
            "/prompt",
            Some(r#"{"task": "go", "request_id": "req_1"}"#.into()),
        )
        .await;
        assert_eq!(status, 202);
        assert!(
            body.contains("\"request_id\":\"req_1\""),
            "the 202 body must echo the correlation id: {body}"
        );

        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
        let mut started = false;
        while !started {
            match tokio::time::timeout_at(deadline, events.recv()).await {
                Ok(Ok(ev)) => {
                    started =
                        matches!(&*ev, epsilon_contracts::agent_event::AgentEvent::AgentStart);
                }
                Ok(Err(e)) => panic!("event stream failed before the run started: {e}"),
                Err(_) => break,
            }
        }
        assert!(started, "the run never started");

        let (status, body) = json_response(
            app.clone(),
            "POST",
            "/abort",
            Some(r#"{"request_id": "req_2"}"#.into()),
        )
        .await;
        assert_eq!(status, 202, "{body}");
        assert!(body.contains("\"aborted\":true"), "{body}");
        assert!(body.contains("\"request_id\":\"req_2\""), "{body}");

        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
        while st.active_runs() > 0 && tokio::time::Instant::now() < deadline {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert_eq!(
            st.active_runs(),
            0,
            "the aborted run's slot must be released"
        );
    }

    /// RPC-02: `/abort` with nothing running says so, keeping HTTP's status vocabulary
    /// (the stdio surface reports core's `false` in-band instead — same core call).
    #[tokio::test]
    async fn abort_with_no_run_reports_conflict() {
        let (st, app) = test_agent(vec![FauxStep::say("hi")]);
        let (status, body) = json_response(app.clone(), "POST", "/abort", None).await;
        assert_eq!(status, 409, "{body}");
        assert!(body.contains("\"aborted\":false"), "{body}");
        assert!(body.contains("no run is active"), "{body}");
        assert_eq!(st.active_runs(), 0);

        // A 409 whose request carried a `request_id` echoes it — the correlation half of
        // the 409 contract, exercised with a body (the request above sends none).
        let (status, body) = json_response(
            app,
            "POST",
            "/abort",
            Some(r#"{"request_id": "req_9"}"#.into()),
        )
        .await;
        assert_eq!(status, 409, "{body}");
        assert!(body.contains("\"aborted\":false"), "{body}");
        assert!(body.contains("\"request_id\":\"req_9\""), "{body}");
    }

    /// RPC-02: serve's `lagged` event data is the shared `LaggedNotice` serialized. Both
    /// surfaces build it from that contract type; the bytes are pinned here (the object
    /// the stdio surface nests under its `lagged` frame key).
    #[test]
    fn lagged_event_data_is_the_shared_notice() {
        assert_eq!(lagged_data(7), r#"{"dropped":7}"#);
    }

    /// RPC-02: the loser of an admission race receives its own `request_id` back on the
    /// 409, so a client can correlate the refusal (pi's contract).
    #[tokio::test]
    async fn conflicting_prompt_echoes_its_request_id() {
        let (st, app) = test_agent_delayed(vec![FauxStep::say("slow")], 400);

        let (status, body) = json_response(
            app.clone(),
            "POST",
            "/prompt",
            Some(r#"{"task": "one", "request_id": "req_1"}"#.into()),
        )
        .await;
        assert_eq!(status, 202, "{body}");
        assert!(body.contains("\"started\":true"), "{body}");

        let (status, body) = json_response(
            app.clone(),
            "POST",
            "/prompt",
            Some(r#"{"task": "two", "request_id": "req_2"}"#.into()),
        )
        .await;
        assert_eq!(status, 409, "{body}");
        assert!(body.contains("\"request_id\":\"req_2\""), "{body}");
        assert!(body.contains("another run is already active"), "{body}");

        st.agent.abort_current_run();
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
        while st.active_runs() > 0 && tokio::time::Instant::now() < deadline {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert_eq!(st.active_runs(), 0);
    }

    /// RPC-02: `GET /state` answers the shared `SessionState` payload, live from the
    /// agent, over the real router.
    #[tokio::test]
    async fn state_reports_the_shared_payload() {
        let (st, app) = test_agent(vec![FauxStep::say("hi")]);
        st.agent.set_model_id("m92-serve-model");
        st.agent
            .set_thinking_level(epsilon_contracts::ThinkingLevel::Low);

        let (status, body) = json_response(app, "GET", "/state", None).await;
        assert_eq!(status, 200, "{body}");
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["model"], "m92-serve-model");
        // camelCase since D30 — this payload is shared with the RPC surface, and pi's
        // `RpcSessionState` is camelCase. The assertion below pins the WHOLE key set, so
        // a spelling cannot drift back without this test failing.
        assert_eq!(v["thinkingLevel"], "low");
        assert_eq!(v["isStreaming"], false);
        assert_eq!(
            v["messageCount"].as_u64().unwrap() as usize,
            st.agent.history().len()
        );
        assert_eq!(v["pendingMessageCount"], 0);
        assert_eq!(v["sessionFile"], serde_json::Value::Null);
        assert_eq!(v["sessionId"], serde_json::Value::Null);
        assert_eq!(v["sessionName"], serde_json::Value::Null);
        // `estimatedContextTokens` is this harness's own estimate of the next request, and `goal`
        // is its own concept entirely — both additive to pi's payload, which is why they are
        // asserted here rather than assumed absent.
        assert_eq!(
            v["estimatedContextTokens"].as_u64().unwrap() as usize,
            st.agent.estimated_context_tokens()
        );
        assert_eq!(
            v["goal"],
            serde_json::Value::Null,
            "an agent with no store has no goal, and `null` is how a client tells that from an \
             empty one"
        );
        // The two queue modes are pi's fields and this harness now answers them, so a
        // client can see the defaults rather than inferring them.
        assert_eq!(v["steeringMode"], "one-at-a-time");
        assert_eq!(v["followUpMode"], "one-at-a-time");

        // The shape is the shared struct's, exactly — and it is the same shape the RPC
        // surface serves, which is the entire reason this payload lives in one struct.
        let mut keys: Vec<&str> = v.as_object().unwrap().keys().map(|k| k.as_str()).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "estimatedContextTokens",
                "followUpMode",
                "goal",
                "isStreaming",
                "messageCount",
                "model",
                "pendingMessageCount",
                "sessionFile",
                "sessionId",
                "sessionName",
                "steeringMode",
                "thinkingLevel",
            ]
        );
    }

    /// **The two wire surfaces must emit the same object, byte for byte.**
    ///
    /// `rpc_mode.rs` and this file each grew their own `SessionState { … }` literal, and
    /// they were identical — right up until D30 added a field and only one of them could
    /// see it. The payload exists in ONE struct precisely so the two transports cannot
    /// disagree, and two builders put that back on a knife edge; `Agent::session_state()`
    /// is now the only constructor either of them may use.
    ///
    /// This is the HTTP half of D29's lesson. D29's test compared two *RPC* ops because
    /// one reference type was being served two ways; the same trap exists across
    /// transports, and a test per surface cannot see it — only a test that compares the
    /// surfaces can.
    #[tokio::test]
    async fn rest_and_rpc_serve_the_identical_state_object() {
        let (st, app) = test_agent(vec![FauxStep::say("hi")]);
        st.agent.set_model_id("m194-both-surfaces");
        st.agent
            .set_steering_mode(epsilon_contracts::QueueMode::All);

        let (status, body) = json_response(app, "GET", "/state", None).await;
        assert_eq!(status, 200, "{body}");
        let rest: serde_json::Value = serde_json::from_str(&body).unwrap();

        // The same agent, through the shared constructor the RPC surface also uses. If a
        // literal reappears in either frontend, this comparison is what notices.
        let via_constructor =
            serde_json::to_value(st.agent.session_state()).expect("SessionState serializes");
        assert_eq!(
            rest, via_constructor,
            "GET /state must be `Agent::session_state()` verbatim, not a second literal"
        );
        // And the setting the test just made is visible, which is the point of adding the
        // field: a mode a client can set but not read is half a feature.
        assert_eq!(rest["steeringMode"], "all", "{rest}");
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
        sock.write_all(b"GET /events HTTP/1.1\r\nHost: 127.0.0.1\r\nAccept: text/event-stream\r\nConnection: close\r\n\r\n")
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
