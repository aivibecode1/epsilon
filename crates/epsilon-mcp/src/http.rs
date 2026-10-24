//! The second [`Connector`]: an MCP client over rmcp 3.1's StreamableHTTP transport.
//!
//! ADR-013 amendment: same seam as [`crate::StdioConnector`], one more connector.
//! rmcp types stay inside this module (adapter rule); the rest of the crate still
//! sees only the transport-agnostic [`Connector`] seam and [`crate::McpToolInfo`].
//!
//! Three deliberate choices, each with a test behind it:
//!
//! - `from_config`, not `with_client`: the `transport-streamable-http-client-reqwest`
//!   feature unifies onto the workspace reqwest (json, stream, rustls-no-provider —
//!   no aws-lc, no native-tls, no OAuth; proven with `cargo tree -i`), so rmcp's
//!   default client IS the harness client. Its defaults are the secure ones for
//!   static secrets: no idle-connection pooling and NO redirect following, so a
//!   caller-supplied header can never be replayed to a redirect target (rmcp's own
//!   `default_http_client_does_not_leak_custom_headers_to_redirect_target`).
//!   Building our own client would need a reqwest edge plus replicated settings
//!   for zero benefit.
//! - `reinit_on_expired_session(false)`: the connector owns the ONE retry. rmcp's
//!   transparent re-init would retry the failed POST inside the worker — for
//!   tools/call that is a second execution of a possibly side-effecting tool.
//!   With it off, a 404 surfaces as `SessionExpired`; lists (read-only) do one
//!   re-handshake + one retry, calls never retry the POST.
//! - The ring provider is installed before the first transport exists: the
//!   workspace uses `rustls-no-provider`, and rmcp's client build PANICS without
//!   a process provider (`reqwest-0.13.4/src/async_impl/client.rs:2484` —
//!   epsilon-mcp's own header-proof tests tripped it before this line existed).
//!   Same one-liner as `epsilon-llm/src/http.rs:87`; idempotent, ignored error.

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;

use async_trait::async_trait;
use http::{HeaderName, HeaderValue};
use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, ClientInfo, ContentBlock, Implementation};
use rmcp::service::{RoleClient, RunningService};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;

use crate::{Connector, HttpServerConfig, McpServerSpec, McpToolInfo, ScrubEnv};

/// Install the process TLS provider before rmcp builds its default client.
///
/// See the module docs: without this, the first transport build panics instead
/// of returning an honest error. `install_default` is idempotent — a second
/// install (e.g. after epsilon-llm's) is an ignored `Err`, still usable.
fn ensure_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

/// Check one static header name/value pair against the HTTP field rules.
///
/// `pub(crate)` because `lib.rs` cannot name these types itself: this
/// module's own name shadows the `http` crate at the crate root, so the only
/// `use http::…` in the crate lives here. Returns the bare reason; the caller
/// prefixes the entry name.
pub(crate) fn check_header(name: &str, value: &str) -> Result<(), String> {
    if HeaderName::from_str(name).is_err() {
        return Err(format!("header '{name}' is not a valid HTTP header name"));
    }
    if HeaderValue::from_str(value).is_err() {
        return Err(format!(
            "header '{name}' has a value outside the HTTP field rules"
        ));
    }
    Ok(())
}

/// Connector speaking MCP over StreamableHTTP to a remote URL.
///
/// The service is held behind a mutex so a broken transport can heal lazily:
/// the next list/call re-handshakes once rather than failing forever. There is
/// no background task and no retry loop — at most one handshake plus, for
/// read-only lists only, one retry of the failed request.
pub struct HttpConnector {
    name: String,
    config: HttpServerConfig,
    service: tokio::sync::Mutex<Option<RunningService<RoleClient, ClientInfo>>>,
}

impl HttpConnector {
    /// Dial `cfg.url` and complete the MCP initialize handshake.
    ///
    /// Fails fast with an honest error naming the server when nothing answers
    /// (connection-refused is instant; no retry loop here — attach failure
    /// keeps the composition's `?` behavior at the call site).
    pub async fn connect(cfg: &HttpServerConfig) -> Result<Self, String> {
        ensure_provider();
        let service = Self::handshake(cfg).await?;
        Ok(Self {
            name: cfg.name.clone(),
            config: cfg.clone(),
            service: tokio::sync::Mutex::new(Some(service)),
        })
    }

    /// One handshake: transport config plus the initialize round-trip.
    async fn handshake(
        cfg: &HttpServerConfig,
    ) -> Result<RunningService<RoleClient, ClientInfo>, String> {
        let mut transport_config = StreamableHttpClientTransportConfig::with_uri(cfg.url.clone())
            // The connector owns the single retry (module docs); rmcp's
            // transparent re-init would replay a tools/call POST.
            .reinit_on_expired_session(false)
            .custom_headers(Self::custom_headers(cfg)?);
        if let Some(token) = cfg.bearer_token.clone() {
            // A bare token: rmcp prefixes `Bearer ` on every request
            // (proven in tests/http_header_proof.rs).
            transport_config = transport_config.auth_header(token);
        }
        let transport = StreamableHttpClientTransport::from_config(transport_config);

        // ClientInfo/Implementation are #[non_exhaustive]: default-construct, then mutate.
        let mut info = ClientInfo::default();
        info.client_info = Implementation::new("epsilon", env!("CARGO_PKG_VERSION"));

        info.serve(transport).await.map_err(|e| {
            format!(
                "mcp server '{}': http initialize with {}: {e}",
                cfg.name, cfg.url
            )
        })
    }

    /// Static headers in rmcp's shape. The config parse already validated every
    /// name and value, so this re-parse is infallible in practice — it still
    /// returns `Err` rather than panicking, because a transport constructor is
    /// not the place to discover a validator was bypassed.
    fn custom_headers(cfg: &HttpServerConfig) -> Result<HashMap<HeaderName, HeaderValue>, String> {
        let mut out = HashMap::with_capacity(cfg.headers.len());
        for (name, value) in &cfg.headers {
            let name = HeaderName::from_str(name)
                .map_err(|e| format!("mcp server '{}': header '{name}': {e}", cfg.name))?;
            let value = HeaderValue::from_str(value)
                .map_err(|e| format!("mcp server '{}': header '{name}': {e}", cfg.name))?;
            out.insert(name, value);
        }
        Ok(out)
    }

    /// Re-handshake once and swap the new service in.
    async fn reconnect(&self) -> Result<(), String> {
        let service = Self::handshake(&self.config).await.map_err(|e| {
            format!(
                "mcp server '{}': reconnect after broken transport: {e}",
                self.name
            )
        })?;
        *self.service.lock().await = Some(service);
        Ok(())
    }

    /// Heal lazily: connect when never connected or the transport closed.
    /// Exactly one handshake — never a loop.
    async fn ensure_connected(&self) -> Result<(), String> {
        let needs = match *self.service.lock().await {
            Some(ref service) => service.is_closed(),
            None => true,
        };
        if needs {
            self.reconnect().await?;
        }
        Ok(())
    }

    /// Map a listed-tools result into the seam's shape (same projection as stdio).
    fn project(tools: Vec<rmcp::model::Tool>) -> Vec<McpToolInfo> {
        tools
            .into_iter()
            .map(|t| McpToolInfo {
                name: t.name.to_string(),
                description: t.description.map(|d| d.to_string()).unwrap_or_default(),
                schema: serde_json::Value::Object((*t.input_schema).clone()),
            })
            .collect()
    }
}

/// True when the worker reports the transport is gone or the server forgot the
/// session (HTTP 404 with a session attached). Both mean "re-handshake once";
/// anything else (401 challenges, tool errors, protocol errors) is honest and
/// must NOT retry.
fn is_broken_transport(error: &str, service_closed: bool) -> bool {
    service_closed || error.to_lowercase().contains("session expired")
}

#[async_trait]
impl Connector for HttpConnector {
    /// DIRECTION-02: a closed HTTP transport means every call would fail with
    /// an opaque error; report that honestly and cheaply. No network I/O here —
    /// the next list/call heals with one handshake.
    ///
    /// Honest limitation (not a second probe): unlike a stdio child exit, a
    /// dead remote is NOT detected until a request fails — the service stays
    /// open while idle. Staleness is bounded by the bridge's `HEALTH_TTL_MS`
    /// plus the per-call honest errors below.
    async fn is_healthy(&self) -> Result<(), String> {
        let guard = self.service.lock().await;
        match guard.as_ref() {
            Some(service) if !service.is_closed() => Ok(()),
            _ => Err(format!(
                "mcp server '{}' is not connected (http transport closed); \
                 restart the session or re-attach with --mcp-http",
                self.name
            )),
        }
    }

    /// Lists are read-only, so a broken transport earns one re-handshake plus
    /// one retry of the list — never more, and never for honest errors.
    async fn list_tools(&self) -> Result<Vec<McpToolInfo>, String> {
        self.ensure_connected().await?;
        let first = {
            let guard = self.service.lock().await;
            let service = guard.as_ref().ok_or_else(|| {
                format!("mcp server '{}': transport vanished mid-list", self.name)
            })?;
            service.list_all_tools().await
        };
        match first {
            Ok(tools) => Ok(Self::project(tools)),
            Err(first_err) => {
                let message = first_err.to_string();
                let closed = self
                    .service
                    .lock()
                    .await
                    .as_ref()
                    .map(|s| s.is_closed())
                    .unwrap_or(true);
                if !is_broken_transport(&message, closed) {
                    return Err(format!("mcp server '{}': tools/list: {message}", self.name));
                }
                self.reconnect().await?;
                let retry = {
                    let guard = self.service.lock().await;
                    let service = guard.as_ref().ok_or_else(|| {
                        format!("mcp server '{}': transport vanished mid-list", self.name)
                    })?;
                    service.list_all_tools().await
                };
                retry
                    .map(Self::project)
                    .map_err(|e| format!("mcp server '{}': tools/list: {e}", self.name))
            }
        }
    }

    /// A tools/call POST is sent EXACTLY ONCE. Retrying could double-execute a
    /// side-effecting tool, so even a broken transport returns the honest error
    /// (naming the server) and lets the NEXT operation heal.
    async fn call_tool(&self, tool: &str, args: serde_json::Value) -> Result<String, String> {
        self.ensure_connected().await?;
        let Some(map) = args.as_object().cloned() else {
            return Err("MCP tool arguments must be a JSON object".into());
        };
        let mut params = CallToolRequestParams::new(tool.to_string());
        params.arguments = Some(map);

        let outcome = {
            let guard = self.service.lock().await;
            let service = guard.as_ref().ok_or_else(|| {
                format!("mcp server '{}': transport vanished mid-call", self.name)
            })?;
            service.call_tool(params).await
        };
        let result =
            outcome.map_err(|e| format!("mcp server '{}': tools/call {tool}: {e}", self.name))?;

        // Same text projection as stdio: concatenated text blocks, tool errors
        // in-band, structured-only output stringified.
        let mut text = String::new();
        for block in &result.content {
            if let ContentBlock::Text(t) = block {
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str(&t.text);
            }
        }
        if result.is_error.unwrap_or(false) {
            return Err(if text.is_empty() {
                format!("tool {tool} failed on mcp server '{}'", self.name)
            } else {
                text
            });
        }
        if text.is_empty()
            && let Some(sc) = result.structured_content.as_ref()
        {
            text = sc.to_string();
        }
        Ok(text)
    }
}

impl HttpConnector {
    /// Close the transport (rmcp deletes the server session, best-effort).
    pub async fn shutdown(self) -> Result<(), String> {
        let service = self.service.lock().await.take();
        match service {
            Some(service) => match service.cancel().await {
                Ok(_) => Ok(()),
                Err(e) => Err(format!(
                    "mcp http service shutdown (server '{}'): {e}",
                    self.name
                )),
            },
            None => Ok(()),
        }
    }
}

/// Convenience: dial an HTTP server and bridge its tools into `registry`.
///
/// Attach failure is an `Err` for the composition's `?` — the same contract as
/// [`crate::attach_stdio_server`]: one bad server refuses the launch rather
/// than silently running without its tools.
pub async fn attach_http_server(
    registry: &mut epsilon_core::ToolRegistry,
    cfg: &HttpServerConfig,
) -> Result<usize, String> {
    let connector: Arc<dyn Connector> = Arc::new(HttpConnector::connect(cfg).await?);
    crate::bridge_with_timeout(registry, &cfg.name, connector, cfg.timeout_secs).await
}

/// One entry point for both transports: stdio keeps its scrubbed spawn, HTTP
/// dials. The runtime's attach loop dispatches here per spec variant.
pub async fn attach_mcp_server(
    registry: &mut epsilon_core::ToolRegistry,
    spec: &McpServerSpec,
    scrub_env: Option<ScrubEnv>,
) -> Result<usize, String> {
    match spec {
        McpServerSpec::Stdio(cfg) => crate::attach_stdio_server(registry, cfg, scrub_env).await,
        McpServerSpec::Http(cfg) => attach_http_server(registry, cfg).await,
    }
}
