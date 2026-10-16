//! Open-verification (b): how `Authorization` travels over rmcp's StreamableHTTP
//! client — proved on the wire, not by assumption.
//!
//! The brief asks whether `Authorization` must travel via rmcp's `auth_header`
//! (which `bearer_auth`s a bare token) or whether it can travel as a static
//! custom header. The vendored source answers half of it:
//! `validate_custom_header` (`rmcp-3.1.4/src/transport/common/http_header.rs:31-45`)
//! rejects only `accept`, `mcp-session-id` and `last-event-id` (allowing
//! `mcp-protocol-version` through for the worker's own injection) —
//! `authorization` is NOT reserved. These tests prove the other half: what a
//! real server actually receives on a real socket in this tree's feature set.
//!
//! Proven:
//! 1. `authorization` as a custom header arrives VERBATIM (so static bearer
//!    tokens and non-Bearer schemes do not need `auth_header`);
//! 2. a bare token via `auth_header` arrives as `Bearer <token>` (rmcp prefixes
//!    it — `common/reqwest/streamable_http_client.rs:202-204`);
//! 3. a reserved header (`accept`) still fails closed with `ReservedHeaderConflict`
//!    naming the header (same shape as rmcp's own `test_custom_headers`).
//!
//! Design consequence (kept in `src/http.rs`): static headers carry a full
//! `Authorization` value verbatim with the https-unless-loopback gate at parse;
//! `bearer_token` carries a bare token via `auth_header`; setting both is
//! ambiguous and rejected at parse with the entry name.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::State;
use rmcp::ServiceExt;
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use tokio::sync::Mutex;

/// rmcp's default client is built on the workspace's `rustls-no-provider`
/// reqwest, which PANICS at client construction when no process provider is
/// installed yet (`reqwest-0.13.4/src/async_impl/client.rs:2484` — the same
/// panic `epsilon-llm/src/http.rs:74-76` guards against). The binary installs
/// it on first LLM use; tests and `HttpConnector::connect` install it before
/// the first transport exists. Idempotent: a second install is an ignored Err.
fn ensure_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

/// Headers the fixture saw, in arrival order (`name: value` lowercase-joined).
#[derive(Clone, Default)]
struct Captured {
    seen: Arc<Mutex<Vec<(String, String)>>>,
    initialized: Arc<tokio::sync::Notify>,
}

async fn fixture(
    State(state): State<Captured>,
    headers: http::HeaderMap,
    body: axum::body::Bytes,
) -> impl axum::response::IntoResponse {
    use axum::http::StatusCode;
    let mut seen = state.seen.lock().await;
    for (name, value) in headers.iter() {
        if let Ok(v) = value.to_str() {
            seen.push((name.as_str().to_string(), v.to_string()));
        }
    }
    drop(seen);
    let body_json: serde_json::Value = serde_json::from_slice(&body).unwrap_or_default();
    let method = body_json
        .get("method")
        .and_then(|m| m.as_str())
        .unwrap_or("");
    if method == "initialize" {
        let response = serde_json::json!({
            "jsonrpc": "2.0",
            "id": body_json.get("id"),
            "result": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "serverInfo": { "name": "proof-server", "version": "1.0.0" }
            }
        });
        return (
            StatusCode::OK,
            [
                (http::header::CONTENT_TYPE, "application/json"),
                (
                    http::HeaderName::from_static("mcp-session-id"),
                    "proof-session",
                ),
            ],
            response.to_string(),
        );
    }
    if method == "notifications/initialized" {
        state.initialized.notify_one();
        return (
            StatusCode::ACCEPTED,
            [
                (http::header::CONTENT_TYPE, "application/json"),
                (
                    http::HeaderName::from_static("mcp-session-id"),
                    "proof-session",
                ),
            ],
            String::new(),
        );
    }
    let response = serde_json::json!({ "jsonrpc": "2.0", "id": 1, "result": {} });
    (
        StatusCode::OK,
        [
            (http::header::CONTENT_TYPE, "application/json"),
            (
                http::HeaderName::from_static("mcp-session-id"),
                "proof-session",
            ),
        ],
        response.to_string(),
    )
}

/// One served fixture: its dialable URL, what it saw, and its task.
struct Served {
    url: String,
    state: Captured,
    handle: tokio::task::JoinHandle<()>,
}

/// Serve the fixture on loopback.
async fn serve_fixture() -> Served {
    let state = Captured::default();
    let app = axum::Router::new()
        .route("/mcp", axum::routing::post(fixture))
        .with_state(state.clone());
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
    Served { url, state, handle }
}

/// Build `config` against this fixture's real URL, preserving the caller's
/// headers and bearer token.
fn config_for(
    served: &Served,
    custom: HashMap<http::HeaderName, http::HeaderValue>,
    auth: Option<&str>,
) -> StreamableHttpClientTransportConfig {
    let mut config = StreamableHttpClientTransportConfig::with_uri(served.url.clone());
    if let Some(token) = auth {
        config = config.auth_header(token);
    }
    config.custom_headers(custom)
}

/// Dial a fresh loopback fixture with `custom` headers and an optional bearer
/// token through a real handshake; return every header the server saw.
async fn dial(
    custom: HashMap<http::HeaderName, http::HeaderValue>,
    auth: Option<&str>,
) -> Result<Vec<(String, String)>, String> {
    ensure_provider();
    let served = serve_fixture().await;
    let transport = StreamableHttpClientTransport::from_config(config_for(&served, custom, auth));
    let client = ().serve(transport).await.map_err(|e| format!("serve: {e}"))?;
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        served.state.initialized.notified(),
    )
    .await
    .map_err(|_| "handshake never completed".to_string())?;
    drop(client);
    served.handle.abort();
    Ok(served.state.seen.lock().await.clone())
}

fn header_values(seen: &[(String, String)], name: &str) -> Vec<String> {
    seen.iter()
        .filter(|(n, _)| n == name)
        .map(|(_, v)| v.clone())
        .collect()
}

/// 1. `authorization` as a custom header is NOT reserved: it arrives verbatim.
#[tokio::test]
async fn authorization_as_custom_header_arrives_verbatim() {
    let mut custom = HashMap::new();
    custom.insert(
        http::HeaderName::from_static("authorization"),
        http::HeaderValue::from_static("Bearer proof-token"),
    );
    let seen = dial(custom, None).await.expect("handshake");
    let got = header_values(&seen, "authorization");
    assert!(
        got.iter().any(|v| v == "Bearer proof-token"),
        "authorization custom header must arrive verbatim, saw: {seen:?}"
    );
}

/// 2. A bare token via `auth_header` arrives as `Bearer <token>` (rmcp prefixes).
#[tokio::test]
async fn bearer_via_auth_header_arrives_with_bearer_prefix() {
    let seen = dial(HashMap::new(), Some("proof-token-2"))
        .await
        .expect("handshake");
    let got = header_values(&seen, "authorization");
    assert!(
        got.iter().any(|v| v == "Bearer proof-token-2"),
        "auth_header token must arrive as `Bearer <token>`, saw: {seen:?}"
    );
}

/// 3. A reserved header fails closed naming the header — no request reaches us.
#[tokio::test]
async fn reserved_accept_header_fails_closed_before_any_request() {
    ensure_provider();
    let served = serve_fixture().await;
    let mut custom = HashMap::new();
    custom.insert(
        http::HeaderName::from_static("accept"),
        http::HeaderValue::from_static("text/html"),
    );
    let config = StreamableHttpClientTransportConfig::with_uri(served.url).custom_headers(custom);
    let transport = StreamableHttpClientTransport::from_config(config);
    let err = ().serve(transport).await.expect_err("accept must be rejected");
    served.handle.abort();
    assert!(
        err.to_string().contains("accept"),
        "rejection must name the reserved header, got: {err}"
    );
    assert!(
        served.state.seen.lock().await.is_empty(),
        "no request may reach the server after a reserved-header rejection"
    );
}
