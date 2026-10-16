//! Integration coverage for the rmcp StreamableHTTP transport (ADR-013 amendment).
//!
//! The fixture is a REAL HTTP server speaking real MCP StreamableHTTP over a real
//! TCP socket (the DSH/opencode in-process pattern, mirroring rmcp's own
//! `test_custom_headers`): handshake, tools/list, tools/call, session expiry,
//! auth challenges and dead servers — offline, deterministic, free.
//!
//! What each test pins:
//! - attach lists + calls end-to-end through the bridge (`mcp__<server>__<tool>`);
//! - static headers arrive on EVERY request (initialize, initialized, list, call);
//! - a 404-expired session heals with exactly one re-handshake + one retry;
//! - a tools/call POST is NEVER retried (side effects must not double-execute);
//! - 401 is an honest error with no retry loop (exactly one POST);
//! - a dead server fails fast with an honest error naming the server;
//! - malformed URLs and plaintext-http Authorization are rejected AT PARSE;
//! - per-call transport errors become `ToolFailure`s naming the server.

use std::collections::{BTreeMap, HashMap};
use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;

use epsilon_core::ToolRegistry;
use epsilon_mcp::{
    Connector, HttpServerConfig, McpServerSpec, attach_http_server, attach_mcp_server,
    http_server_config, qualified_name,
};

// ---------------------------------------------------------------------------
// Fixture: a tiny StreamableHTTP MCP server with knobs.
// ---------------------------------------------------------------------------

/// One observed request.
#[derive(Debug, Clone)]
struct Observed {
    /// JSON-RPC method, or `"GET"` / `"DELETE"` for stream/session requests.
    method: String,
    /// Lowercased request headers.
    headers: HashMap<String, String>,
    /// The `mcp-session-id` the client attached, if any.
    session: Option<String>,
}

#[derive(Default)]
struct FixtureState {
    observed: Vec<Observed>,
    /// Handshakes completed (initialize POSTs answered 200).
    inits: usize,
    /// Current valid session id (`sess-<inits>`).
    session: String,
    /// When true, the next session-bearing POST answers 404 once (expiry drill).
    expire_once: bool,
    /// When true, every POST answers 401 with a `WWW-Authenticate` challenge.
    unauthorized: bool,
}

#[derive(Clone)]
struct Fixture {
    state: Arc<tokio::sync::Mutex<FixtureState>>,
}

impl Fixture {
    fn new() -> Self {
        Self {
            state: Arc::new(tokio::sync::Mutex::new(FixtureState::default())),
        }
    }

    async fn set_expire_once(&self) {
        self.state.lock().await.expire_once = true;
    }

    async fn set_unauthorized(&self) {
        self.state.lock().await.unauthorized = true;
    }

    async fn inits(&self) -> usize {
        self.state.lock().await.inits
    }

    async fn method_count(&self, method: &str) -> usize {
        self.state
            .lock()
            .await
            .observed
            .iter()
            .filter(|o| o.method == method)
            .count()
    }

    async fn sessions_for(&self, method: &str) -> Vec<Option<String>> {
        self.state
            .lock()
            .await
            .observed
            .iter()
            .filter(|o| o.method == method)
            .map(|o| o.session.clone())
            .collect()
    }

    /// Every observed request carrying `name: value` (case-insensitive name).
    async fn requests_with_header(&self, name: &str, value: &str) -> usize {
        self.state
            .lock()
            .await
            .observed
            .iter()
            .filter(|o| o.headers.get(name) == Some(&value.to_string()))
            .count()
    }

    async fn total_requests(&self) -> usize {
        self.state.lock().await.observed.len()
    }
}

fn init_result(id: &serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "serverInfo": { "name": "http-fixture", "version": "1.0.0" }
        }
    })
}

fn list_result(id: &serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "tools": [{
                "name": "echo",
                "description": "Echo back the provided text.",
                "inputSchema": {
                    "type": "object",
                    "properties": { "text": { "type": "string" } },
                    "required": ["text"]
                }
            }]
        }
    })
}

fn call_result(id: &serde_json::Value, text: &str) -> serde_json::Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "content": [{ "type": "text", "text": format!("echo: {text}") }],
            "isError": false
        }
    })
}

fn error_result(id: &serde_json::Value, message: &str) -> serde_json::Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": -32602, "message": message }
    })
}

/// One uniform response shape: every branch returns the same tuple type, so
/// axum's `IntoResponse` unifies without a per-branch array-size fight.
type PostResponse = (axum::http::StatusCode, http::HeaderMap, String);

fn respond(
    status: axum::http::StatusCode,
    session: Option<&str>,
    extra: &[(&str, &str)],
    body: String,
) -> PostResponse {
    let mut headers = http::HeaderMap::new();
    headers.insert(
        http::header::CONTENT_TYPE,
        http::HeaderValue::from_static("application/json"),
    );
    if let Some(sid) = session {
        headers.insert(
            http::HeaderName::from_static("mcp-session-id"),
            http::HeaderValue::from_str(sid).expect("fixture session is header-safe"),
        );
    }
    for (name, value) in extra {
        headers.insert(
            http::HeaderName::from_str(name).expect("fixture header name"),
            http::HeaderValue::from_str(value).expect("fixture header value"),
        );
    }
    (status, headers, body)
}

async fn post_handler(
    axum::extract::State(fixture): axum::extract::State<Fixture>,
    headers: http::HeaderMap,
    body: axum::body::Bytes,
) -> PostResponse {
    use axum::http::StatusCode;
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap_or_default();
    let method = json
        .get("method")
        .and_then(|m| m.as_str())
        .unwrap_or("")
        .to_string();
    let mut seen = HashMap::new();
    for (name, value) in headers.iter() {
        if let Ok(v) = value.to_str() {
            seen.insert(name.as_str().to_string(), v.to_string());
        }
    }
    let session = seen.get("mcp-session-id").cloned();
    let mut st = fixture.state.lock().await;
    let observe = |st: &mut FixtureState| {
        st.observed.push(Observed {
            method: method.clone(),
            headers: seen.clone(),
            session: session.clone(),
        });
    };
    // 401 short-circuits everything: the challenge is the whole answer.
    if st.unauthorized {
        observe(&mut st);
        return respond(
            StatusCode::UNAUTHORIZED,
            None,
            &[(
                "www-authenticate",
                "Bearer realm=\"fixture\", error=\"invalid_token\"",
            )],
            String::new(),
        );
    }
    if method == "initialize" {
        st.inits += 1;
        st.session = format!("sess-{}", st.inits);
        let sid = st.session.clone();
        observe(&mut st);
        let id = json.get("id").cloned().unwrap_or(serde_json::Value::Null);
        return respond(
            StatusCode::OK,
            Some(&sid),
            &[],
            init_result(&id).to_string(),
        );
    }
    // One-shot expiry drill: the next session-bearing POST 404s.
    if st.expire_once && session.is_some() {
        st.expire_once = false;
        observe(&mut st);
        return respond(StatusCode::NOT_FOUND, None, &[], String::new());
    }
    // Session gate: unknown sessions 404 exactly like a real server that
    // forgot them (this is what surfaces as `SessionExpired` client-side).
    if session.as_deref() != Some(st.session.as_str()) && method != "notifications/initialized" {
        observe(&mut st);
        return respond(StatusCode::NOT_FOUND, None, &[], String::new());
    }
    let sid = st.session.clone();
    observe(&mut st);
    if method == "notifications/initialized" {
        return respond(StatusCode::ACCEPTED, Some(&sid), &[], String::new());
    }
    let id = json.get("id").cloned().unwrap_or(serde_json::Value::Null);
    let payload = if method == "tools/list" {
        list_result(&id).to_string()
    } else if method == "tools/call" {
        let name = json
            .get("params")
            .and_then(|p| p.get("name"))
            .and_then(|n| n.as_str())
            .unwrap_or("");
        if name == "echo" {
            let text = json
                .get("params")
                .and_then(|p| p.get("arguments"))
                .and_then(|a| a.get("text"))
                .and_then(|t| t.as_str())
                .unwrap_or("");
            call_result(&id, text).to_string()
        } else {
            error_result(&id, &format!("unknown fixture tool: {name}")).to_string()
        }
    } else {
        error_result(&id, &format!("unexpected method: {method}")).to_string()
    };
    respond(StatusCode::OK, Some(&sid), &[], payload)
}

/// No SSE stream and no session delete: 405, which rmcp tolerates by skipping
/// the common stream (`ServerDoesNotSupportSse`).
async fn method_not_allowed() -> impl axum::response::IntoResponse {
    axum::http::StatusCode::METHOD_NOT_ALLOWED
}

struct Served {
    url: String,
    fixture: Fixture,
    handle: tokio::task::JoinHandle<()>,
}

async fn serve_fixture() -> Served {
    let fixture = Fixture::new();
    let app = axum::Router::new()
        .route(
            "/mcp",
            axum::routing::post(post_handler)
                .get(method_not_allowed)
                .delete(method_not_allowed),
        )
        .with_state(fixture.clone());
    let listener = tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
        .await
        .expect("bind loopback");
    let url = format!(
        "http://127.0.0.1:{}/mcp",
        listener.local_addr().unwrap().port()
    );
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Served {
        url,
        fixture,
        handle,
    }
}

fn cfg_for(served: &Served, name: &str) -> HttpServerConfig {
    http_server_config(name, &served.url, BTreeMap::new(), None).expect("valid fixture URL")
}

fn tool_context() -> epsilon_contracts::tool::ToolContext {
    epsilon_contracts::tool::ToolContext {
        session_id: "http-test".into(),
        observation_scope: std::sync::Arc::new(()),
        cwd: std::env::temp_dir(),
        spill_dir: std::env::temp_dir().join("eps-mcp-http"),
        token: Default::default(),
        task_host: None,
        goal_host: None,
    }
}

// ---------------------------------------------------------------------------
// Attach + call over HTTP.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn http_attach_lists_and_calls_through_the_bridge() {
    let served = serve_fixture().await;
    let mut registry = ToolRegistry::new();
    let bridged = attach_http_server(&mut registry, &cfg_for(&served, "fixture"))
        .await
        .expect("bridge http fixture");
    assert_eq!(bridged, 1, "fixture exposes exactly one tool");

    let name = qualified_name("fixture", "echo");
    let tool = registry.get(&name).expect("bridged tool present");
    let out = tool
        .execute(
            epsilon_contracts::tool::ToolInvocation {
                tool_call_id: "tc_http".into(),
                args: serde_json::json!({ "text": "over-http" }),
            },
            tool_context(),
            Arc::new(|_| {}),
        )
        .await
        .expect("echo call succeeds");
    assert!(
        matches!(&out.content[0], epsilon_contracts::content::ContentBlock::Text { text }
            if text.contains("echo: over-http")),
        "round-tripped text must come back, got {:?}",
        out.content
    );
    served.handle.abort();
}

#[tokio::test]
async fn static_headers_arrive_on_every_request() {
    let served = serve_fixture().await;
    let mut headers = BTreeMap::new();
    headers.insert("x-api-key".into(), "proof-key".into());
    let cfg =
        http_server_config("headers", &served.url, headers, None).expect("valid headers config");
    let mut registry = ToolRegistry::new();
    attach_http_server(&mut registry, &cfg)
        .await
        .expect("bridge");
    let tool = registry
        .get(&qualified_name("headers", "echo"))
        .expect("bridged tool present");
    tool.execute(
        epsilon_contracts::tool::ToolInvocation {
            tool_call_id: "tc_hdr".into(),
            args: serde_json::json!({ "text": "x" }),
        },
        tool_context(),
        Arc::new(|_| {}),
    )
    .await
    .expect("call succeeds");

    // initialize, notifications/initialized, tools/list, tools/call: the header
    // must be on ALL of them, not just the handshake.
    for method in [
        "initialize",
        "notifications/initialized",
        "tools/list",
        "tools/call",
    ] {
        let count = served.fixture.method_count(method).await;
        let with = served
            .fixture
            .requests_with_header("x-api-key", "proof-key")
            .await;
        assert!(count >= 1, "fixture must have seen {method}");
        assert!(
            with >= count && with > 0,
            "every {method} request must carry the static header"
        );
    }
    // No request at all may go out without it.
    assert_eq!(
        served
            .fixture
            .requests_with_header("x-api-key", "proof-key")
            .await,
        served.fixture.total_requests().await,
        "the static header must be on every request the client sent"
    );
    served.handle.abort();
}

#[tokio::test]
async fn bearer_token_arrives_as_bearer_on_every_request() {
    let served = serve_fixture().await;
    let cfg = http_server_config("bearer", &served.url, BTreeMap::new(), Some("tok-1".into()))
        .expect("valid bearer config");
    let connector = epsilon_mcp::HttpConnector::connect(&cfg)
        .await
        .expect("connect");
    connector.list_tools().await.expect("list");
    assert_eq!(
        served
            .fixture
            .requests_with_header("authorization", "Bearer tok-1")
            .await,
        served.fixture.total_requests().await,
        "auth_header must arrive as `Bearer <token>` on every request"
    );
    served.handle.abort();
}

// ---------------------------------------------------------------------------
// Session expiry: exactly one re-handshake + one retry; calls never retry.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn expired_session_heals_with_exactly_one_rehandshake_and_one_retry() {
    let served = serve_fixture().await;
    let connector = epsilon_mcp::HttpConnector::connect(&cfg_for(&served, "expiry"))
        .await
        .expect("connect");
    connector.list_tools().await.expect("first list");
    assert_eq!(served.fixture.inits().await, 1);
    assert_eq!(served.fixture.method_count("tools/list").await, 1);

    // Expire the session server-side; the next list must re-handshake ONCE and
    // retry the list ONCE — then succeed.
    served.fixture.set_expire_once().await;
    let tools = connector.list_tools().await.expect("list heals");
    assert!(tools.iter().any(|t| t.name == "echo"));
    assert_eq!(
        served.fixture.inits().await,
        2,
        "exactly one re-handshake, not zero and not a loop"
    );
    // Three list POSTs in total: the first (pre-expiry) success, the expired
    // attempt itself, and exactly one retry. The failed attempt is observed —
    // "one retry" means one POST after the failure, not one POST total.
    assert_eq!(
        served.fixture.method_count("tools/list").await,
        3,
        "the expired list must be retried exactly once"
    );
    // The retry ran under the NEW session: it was a re-handshake, not a replay.
    let sessions = served.fixture.sessions_for("tools/list").await;
    assert_eq!(sessions.len(), 3);
    assert_eq!(
        sessions[0], sessions[1],
        "the expired attempt reused the old session"
    );
    assert_ne!(
        sessions[1], sessions[2],
        "the retried list must carry the re-handshaked session"
    );
    served.handle.abort();
}

#[tokio::test]
async fn a_tools_call_post_is_never_retried() {
    let served = serve_fixture().await;
    let connector = epsilon_mcp::HttpConnector::connect(&cfg_for(&served, "no-retry"))
        .await
        .expect("connect");
    served.fixture.set_expire_once().await;
    let err = connector
        .call_tool("echo", serde_json::json!({ "text": "once" }))
        .await
        .expect_err("expired call must fail rather than replay");
    assert!(
        err.contains("no-retry"),
        "the failure must name the server, got: {err}"
    );
    assert_eq!(
        served.fixture.method_count("tools/call").await,
        1,
        "a tools/call POST is sent exactly once: retrying could double-execute a side effect"
    );
    // The NEXT list heals, proving the transport recovered without the retry.
    served.fixture.set_expire_once().await;
    let _ = connector.list_tools().await;
    assert_eq!(served.fixture.inits().await, 2);
    served.handle.abort();
}

// ---------------------------------------------------------------------------
// Honest failures: 401, dead servers, unknown tools, per-call naming.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn unauthorized_is_an_honest_error_with_no_retry_loop() {
    let served = serve_fixture().await;
    served.fixture.set_unauthorized().await;
    let err = attach_http_server(&mut registry_new(), &cfg_for(&served, "locked"))
        .await
        .expect_err("401 attach must fail");
    assert!(
        err.contains("locked") && (err.contains("401") || err.to_lowercase().contains("auth")),
        "401 must surface honestly naming the server, got: {err}"
    );
    assert_eq!(
        served.fixture.method_count("initialize").await,
        1,
        "a 401 must not trigger a retry loop"
    );
    served.handle.abort();
}

fn registry_new() -> ToolRegistry {
    ToolRegistry::new()
}

#[tokio::test]
async fn dead_server_fails_fast_with_an_honest_error() {
    // Port 9 (discard) is never listening: connection-refused, not a timeout.
    let cfg =
        http_server_config("dead", "http://127.0.0.1:9/mcp", BTreeMap::new(), None).expect("parse");
    let err = tokio::time::timeout(
        std::time::Duration::from_secs(15),
        attach_http_server(&mut registry_new(), &cfg),
    )
    .await
    .expect("attach must return, never hang")
    .expect_err("dead server must fail");
    assert!(
        err.contains("dead"),
        "the failure must name the server, got: {err}"
    );
}

#[tokio::test]
async fn unknown_tool_surfaces_as_failure_naming_server_and_tool() {
    let served = serve_fixture().await;
    let connector = epsilon_mcp::HttpConnector::connect(&cfg_for(&served, "fixture"))
        .await
        .expect("connect");
    let err = connector
        .call_tool("does_not_exist", serde_json::json!({}))
        .await
        .expect_err("unknown tool must fail in-band");
    assert!(
        err.contains("does_not_exist") && err.contains("fixture"),
        "failure must name tool and server, got: {err}"
    );
    served.handle.abort();
}

#[tokio::test]
async fn per_call_transport_error_becomes_tool_failure_naming_the_server() {
    let served = serve_fixture().await;
    let mut registry = ToolRegistry::new();
    attach_http_server(&mut registry, &cfg_for(&served, "flaky"))
        .await
        .expect("bridge");
    // Kill the server mid-session: the next call cannot be served.
    served.handle.abort();
    let tool = registry
        .get(&qualified_name("flaky", "echo"))
        .expect("bridged tool present");
    let err = tool
        .execute(
            epsilon_contracts::tool::ToolInvocation {
                tool_call_id: "tc_dead".into(),
                args: serde_json::json!({ "text": "x" }),
            },
            tool_context(),
            Arc::new(|_| {}),
        )
        .await
        .expect_err("call against a dead server must fail");
    assert!(
        err.message.contains("flaky"),
        "ToolFailure must name the server, got: {}",
        err.message
    );
}

// ---------------------------------------------------------------------------
// Parse-time rejection: malformed URLs, plaintext auth, ambiguity, SSE legacy.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn malformed_urls_are_rejected_at_parse_with_the_entry_name() {
    for bad in [
        "not a url",
        "ftp://example.com/mcp",
        "http://",
        "http:///mcp",
        "http://example.com:abc/mcp",
    ] {
        let err = http_server_config("badentry", bad, BTreeMap::new(), None)
            .expect_err(&format!("{bad} must be rejected"));
        assert!(
            err.contains("badentry"),
            "rejection must name the entry, got: {err}"
        );
    }
}

#[tokio::test]
async fn plaintext_http_authorization_to_non_loopback_is_rejected_at_parse() {
    let mut headers = BTreeMap::new();
    headers.insert("Authorization".into(), "Bearer live-secret".into());
    let err = http_server_config("plain", "http://192.168.1.10/mcp", headers, None)
        .expect_err("plaintext auth to non-loopback must fail closed");
    assert!(
        err.contains("plain") && err.to_lowercase().contains("https"),
        "rejection must name the entry and point at https, got: {err}"
    );
    // The bearer spell is the same secret by another name: same gate.
    let err = http_server_config(
        "plain-bearer",
        "http://192.168.1.10/mcp",
        BTreeMap::new(),
        Some("live-secret".into()),
    )
    .expect_err("plaintext bearer to non-loopback must fail closed");
    assert!(err.contains("plain-bearer"), "got: {err}");

    // Loopback and https are the two honest ways to carry a secret.
    http_server_config(
        "loop",
        "http://127.0.0.1:8080/mcp",
        BTreeMap::from([("Authorization".into(), "Bearer s".into())]),
        None,
    )
    .expect("loopback Authorization is allowed");
    http_server_config(
        "tls",
        "https://example.com/mcp",
        BTreeMap::new(),
        Some("s".into()),
    )
    .expect("https bearer is allowed");
}

#[tokio::test]
async fn authorization_header_and_bearer_together_are_ambiguous() {
    let mut headers = BTreeMap::new();
    headers.insert("Authorization".into(), "Bearer s".into());
    let err = http_server_config(
        "two-ways",
        "http://127.0.0.1:8080/mcp",
        headers,
        Some("s".into()),
    )
    .expect_err("two Authorization sources must be rejected as ambiguous");
    assert!(err.contains("two-ways"), "got: {err}");
}

#[tokio::test]
async fn reserved_transport_headers_are_rejected_at_parse() {
    for reserved in ["accept", "mcp-session-id", "last-event-id"] {
        let mut headers = BTreeMap::new();
        headers.insert(reserved.into(), "x".into());
        let err = http_server_config("rsv", "http://127.0.0.1:1/mcp", headers, None)
            .expect_err(&format!("{reserved} must be rejected"));
        assert!(err.contains("rsv"), "got: {err}");
    }
}

// ---------------------------------------------------------------------------
// Dispatcher: one entry point for both transports.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn attach_dispatcher_routes_http_specs() {
    let served = serve_fixture().await;
    let spec = McpServerSpec::Http(cfg_for(&served, "dispatched"));
    let mut registry = ToolRegistry::new();
    let n = attach_mcp_server(&mut registry, &spec, None)
        .await
        .expect("dispatch http");
    assert_eq!(n, 1);
    assert!(
        registry
            .get(&qualified_name("dispatched", "echo"))
            .is_some()
    );
    served.handle.abort();
}
