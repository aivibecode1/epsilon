//! MCP integration plumbing (ADR-013, staged delivery).
//!
//! This crate owns everything EXCEPT the wire transport:
//! - [`McpServerConfig`]: declarative stdio server configuration (command shape),
//! - [`HttpServerConfig`]: declarative StreamableHTTP server configuration,
//! - [`McpServerSpec`]: the config enum — stdio or HTTP per entry,
//! - [`Connector`]: the transport seam the official rmcp SDK implements (M9),
//! - [`bridge`]: registers a connector's tools into the normal registry under
//!   `mcp__<server>__<tool>` with TTL+grace availability probes (Hermes pattern).
//!
//! Staging rationale: the bridge and naming/gating semantics are fully testable against an
//! in-process double now; plugging rmcp's stdio client in later is a connector impl, not a
//! redesign. ADR-013 carries the amendment.

mod http;
mod stdio;

pub use http::{HttpConnector, attach_http_server, attach_mcp_server};
pub use stdio::{ScrubEnv, StdioConnector, attach_stdio_server};

use async_trait::async_trait;
use epsilon_contracts::tool::{
    Tool, ToolContext, ToolDescriptor, ToolFailure, ToolInvocation, ToolOutput, ToolUpdateSink,
};
use std::sync::Arc;

/// Declarative config for one MCP server (mirrors `mcp_servers.<name>` config shape).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct McpServerConfig {
    /// Server name; tools register as `mcp__<name>__<tool>`.
    pub name: String,
    /// Executable for stdio transport.
    pub command: String,
    /// argv for `command`.
    #[serde(default)]
    pub args: Vec<String>,
}

/// Declarative config for one StreamableHTTP MCP server.
///
/// Headers and the bearer token come from the config FILE only — never from a
/// flag (AGENTS.md §6: no secrets on command lines). A full `Authorization`
/// header value travels verbatim; `bearer_token` carries a bare token that
/// rmcp sends as `Bearer <token>` (proven in `tests/http_header_proof.rs` —
/// `authorization` is not an rmcp-reserved header, so both paths work and the
/// parse below keeps them from being set at once).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HttpServerConfig {
    /// Server name; tools register as `mcp__<name>__<tool>`.
    pub name: String,
    /// Absolute `http(s)://` URL of the MCP endpoint.
    pub url: String,
    /// Static headers sent on every request (config file only).
    #[serde(default)]
    pub headers: std::collections::BTreeMap<String, String>,
    /// Optional bare bearer token, sent as `Authorization: Bearer <token>`.
    /// `None` means no auth header. Never accepted on a command line.
    #[serde(default)]
    pub bearer_token: Option<String>,
}

/// The config enum: one `[[mcp_servers]]` entry is stdio XOR HTTP.
///
/// The tag is the entry's content, not a tag field: legacy files name
/// `command` (stdio) with no tag at all, so `url` vs `command` is the
/// discriminator. Entries naming both or neither — or any unknown key — are
/// rejected at parse WITH the entry name (the runtime's TOML branch does the
/// table inspection and calls [`http_server_config`] for the HTTP half).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpServerSpec {
    /// Spawn a child process over stdio.
    Stdio(McpServerConfig),
    /// Dial a remote StreamableHTTP endpoint.
    Http(HttpServerConfig),
}

impl McpServerSpec {
    /// The server name, whichever transport it dials.
    pub fn name(&self) -> &str {
        match self {
            McpServerSpec::Stdio(cfg) => &cfg.name,
            McpServerSpec::Http(cfg) => &cfg.name,
        }
    }
}

/// Build a validated [`HttpServerConfig`], failing closed with the entry name.
///
/// Rejected at parse (never at first-request time):
/// - empty names and malformed URLs (non-`http(s)`, userinfo, empty hosts,
///   bad ports, fragments);
/// - `Authorization` headers and bearer tokens over plaintext `http://` to a
///   non-loopback host (a secret on the wire in the clear — use `https://`);
/// - `Authorization` AND `bearer_token` together (ambiguous: one source);
/// - rmcp-reserved headers (`accept`, `mcp-session-id`, `last-event-id`),
///   which the transport rejects per request and the config refuses up front;
/// - header names/values outside the HTTP field rules.
pub fn http_server_config(
    name: &str,
    url: &str,
    headers: std::collections::BTreeMap<String, String>,
    bearer_token: Option<String>,
) -> Result<HttpServerConfig, String> {
    let entry = |why: &str| format!("mcp server '{name}': {why}");
    if name.trim().is_empty() {
        return Err(entry("server name must not be empty"));
    }
    let parsed = parse_http_url(url).map_err(|why| entry(&why))?;
    let bearer = match bearer_token {
        Some(t) if t.trim().is_empty() => {
            return Err(entry("bearer_token must not be empty; omit it for no auth"));
        }
        other => other,
    };
    let mut saw_authorization = false;
    for (key, value) in &headers {
        let lower = key.to_lowercase();
        if matches!(
            lower.as_str(),
            "accept" | "mcp-session-id" | "last-event-id"
        ) {
            return Err(entry(&format!(
                "header '{key}' is reserved by the MCP transport and cannot be set"
            )));
        }
        if lower == "authorization" {
            saw_authorization = true;
        }
        // Name/value legality lives beside the transport (`http.rs` owns the
        // only `use http::…` in the crate — this module's own name shadows the
        // crate, so naming those types here does not resolve).
        crate::http::check_header(key, value).map_err(|why| entry(&why))?;
    }
    if saw_authorization && bearer.is_some() {
        return Err(entry(
            "ambiguous: sets both an Authorization header and bearer_token; use one",
        ));
    }
    // Fail closed on plaintext secrets: an Authorization header OR a bearer
    // token over http:// to a non-loopback host would cross the network in the
    // clear. Loopback and https are the two honest carriers.
    if (saw_authorization || bearer.is_some()) && !parsed.https && !parsed.loopback {
        return Err(entry(&format!(
            "refuses to send Authorization over plaintext http to non-loopback host \
             '{}'; use https:// or a loopback URL",
            parsed.host
        )));
    }
    Ok(HttpServerConfig {
        name: name.to_string(),
        url: url.to_string(),
        headers,
        bearer_token: bearer,
    })
}

/// The parsed shape of an MCP HTTP URL (stdlib only — no URL crate needed).
struct ParsedUrl {
    https: bool,
    host: String,
    loopback: bool,
}

/// Validate an absolute `http(s)://` URL without a URL crate: the shape is
/// small (scheme, authority, no userinfo, numeric port) and a dependency for
/// one check the standard library already covers is the slop Ponytail names.
fn parse_http_url(url: &str) -> Result<ParsedUrl, String> {
    let (scheme, rest) = url
        .split_once("://")
        .ok_or_else(|| format!("malformed URL '{url}': must be an absolute http(s) URL"))?;
    let https = match scheme.to_lowercase().as_str() {
        "https" => true,
        "http" => false,
        _ => {
            return Err(format!(
                "malformed URL '{url}': scheme must be http or https"
            ));
        }
    };
    // Authority ends where the path/query begins; fragments never travel.
    let authority_end = rest.find(['/', '?']).unwrap_or(rest.len());
    if rest[authority_end..].contains('#') {
        return Err(format!("malformed URL '{url}': fragments are never sent"));
    }
    let authority = &rest[..authority_end];
    if authority.contains('@') {
        return Err(format!(
            "malformed URL '{url}': userinfo credentials are not accepted in the URL"
        ));
    }
    let host = if let Some(bracketed) = authority.strip_prefix('[') {
        let (inside, after) = bracketed
            .split_once(']')
            .ok_or_else(|| format!("malformed URL '{url}': unterminated IPv6 literal"))?;
        if !after.is_empty() {
            parse_port(
                after.strip_prefix(':').ok_or_else(|| {
                    format!("malformed URL '{url}': unexpected text after IPv6 literal")
                })?,
                url,
            )?;
        }
        inside.to_string()
    } else {
        if authority.chars().filter(|&c| c == ':').count() > 1 {
            return Err(format!(
                "malformed URL '{url}': unbracketed IPv6 literals are not accepted"
            ));
        }
        match authority.split_once(':') {
            Some((host, port)) => {
                parse_port(port, url)?;
                host.to_string()
            }
            None => authority.to_string(),
        }
    };
    if host.trim().is_empty() {
        return Err(format!("malformed URL '{url}': host must not be empty"));
    }
    let lower = host.to_lowercase();
    let loopback = lower == "localhost" || lower == "::1" || lower.starts_with("127.");
    Ok(ParsedUrl {
        https,
        host,
        loopback,
    })
}

/// A port is digits and nothing else, in range. Empty and out-of-range are
/// both paste errors, and both fail here rather than at dial time.
fn parse_port(port: &str, url: &str) -> Result<(), String> {
    match port.parse::<u16>() {
        Ok(_) => Ok(()),
        Err(_) => Err(format!("malformed URL '{url}': port must be 1-65535")),
    }
}

/// Transport seam. rmcp-backed stdio/HTTP connectors implement this (M9).
#[async_trait]
pub trait Connector: Send + Sync {
    /// Discover tools exposed by the server.
    async fn list_tools(&self) -> Result<Vec<McpToolInfo>, String>;

    /// Call one tool; returns concatenated text output.
    async fn call_tool(&self, tool: &str, args: serde_json::Value) -> Result<String, String>;

    /// Cheap health probe (DIRECTION-02, Hermes' availability-check pattern).
    /// Returns `Err` when the server cannot serve calls right now. The default
    /// implementation trusts the connection — connectors that can detect death
    /// cheaply (e.g. stdio child exited) override this.
    async fn is_healthy(&self) -> Result<(), String> {
        Ok(())
    }
}

/// Tool metadata as reported by a server.
#[derive(Debug, Clone)]
pub struct McpToolInfo {
    /// Server-local tool name.
    pub name: String,
    /// LLM-facing description.
    pub description: String,
    /// JSON Schema for arguments.
    pub schema: serde_json::Value,
}

/// Fully-qualified MCP tool name.
pub fn qualified_name(server: &str, tool: &str) -> String {
    format!("mcp__{server}__{tool}")
}

/// Register every tool a connector exposes into `registry`.
///
/// Availability is probed once at attach time with TTL refresh handled by callers
/// (Hermes' check_fn pattern lives at the loop layer in later milestones).
pub async fn bridge(
    registry: &mut epsilon_core::ToolRegistry,
    server_name: &str,
    connector: Arc<dyn Connector>,
) -> Result<usize, String> {
    let tools = connector.list_tools().await?;
    let mut count = 0usize;
    for info in tools {
        registry.register(Arc::new(McpBridgeTool {
            descriptor: ToolDescriptor {
                name: qualified_name(server_name, &info.name),
                label: format!("mcp:{}:{}", server_name, info.name),
                description: info.description,
                parameters_schema: info.schema,
                execution_mode: epsilon_contracts::tool::ExecutionMode::Sequential,
            },
            server_name: server_name.to_string(),
            tool_name: info.name.clone(),
            connector: Arc::clone(&connector),
            last_probe_ms: std::sync::atomic::AtomicI64::new(0),
        }));
        count += 1;
    }
    Ok(count)
}

/// Registry-facing wrapper routing calls through the connector.
struct McpBridgeTool {
    descriptor: ToolDescriptor,
    server_name: String,
    tool_name: String,
    connector: Arc<dyn Connector>,
    /// Last successful health probe (DIRECTION-02). Calls within the TTL skip
    /// the probe; a failed call resets it so the next call re-probes first.
    last_probe_ms: std::sync::atomic::AtomicI64,
}

/// How long a successful health probe stays valid (Hermes' availability-TTL).
///
/// It is fixed. A comment used to sit here claiming an `EPS_MCP_MEALTH_TTL_MS`
/// environment override existed for tests, and nothing in the crate read it —
/// a documented feature with no code behind it, which is the shape AGENTS.md
/// calls a lie. The honest resolution was to **remove the claim rather than
/// add an override nothing uses**: the TTL needs shrinking in exactly one
/// place (the deadline test) and a caller can always spin up a fresh tool.
pub const HEALTH_TTL_MS: i64 = 30_000;

/// How long one MCP tool call may take before it is abandoned.
///
/// Deliberately the same order as the shell tool's default (120 s): an MCP tool is
/// a remote procedure that may do real work, so the bound has to be loose enough
/// not to fail a legitimate slow one and tight enough that a wedged server costs
/// a turn rather than the process. Fixed, not configurable — an override would
/// be a knob nothing uses, which is the mistake this round just removed from the
/// health probe's comment.
///
/// The deadline test needs no short value: it runs under paused tokio time, so the
/// 120 s elapse the moment the call has nothing left to do.
pub const CALL_TIMEOUT_SECS: u64 = 120;

/// The deadline for one call.
fn call_timeout() -> std::time::Duration {
    std::time::Duration::from_secs(CALL_TIMEOUT_SECS)
}

impl McpBridgeTool {
    /// Probe server health at most once per [`HEALTH_TTL_MS`], and always
    /// after a failure, so a dead server fails FAST with a clear message
    /// instead of an opaque transport error mid-call.
    async fn ensure_healthy(&self) -> Result<(), String> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        let last = self
            .last_probe_ms
            .load(std::sync::atomic::Ordering::Relaxed);
        if now - last < HEALTH_TTL_MS {
            return Ok(());
        }
        self.connector.is_healthy().await?;
        self.last_probe_ms
            .store(now, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }

    // A `#[cfg(test)] #[allow(dead_code)] invalidate_health` used to sit here,
    // commented "force the next `ensure_healthy` to re-probe". Nothing called
    // it, tests included: the `allow` was there because the two attributes
    // together are enough to make a function nobody calls survive. The probe
    // clock is a `store` on a field the tests already set, so a seam that no
    // test reached was scaffolding rather than a facility.
}

#[async_trait]
impl Tool for McpBridgeTool {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    async fn execute(
        &self,
        call: ToolInvocation,
        ctx: ToolContext,
        _on_update: ToolUpdateSink,
    ) -> Result<ToolOutput, ToolFailure> {
        // DIRECTION-02: TTL-cached health probe so a dead server fails fast
        // with an honest message instead of an opaque transport error.
        //
        // This comment used to add "Tests can shrink the TTL via the
        // `EPS_MCP_HEALTH_TTL_MS` env override" — and nothing read that variable,
        // which is a documented feature with no code behind it. The constant is
        // fixed; the honest way to test a different TTL is a fresh tool.
        self.ensure_healthy().await.map_err(ToolFailure::msg)?;
        // **The call is bounded and cancellable.**
        //
        // It used to be a bare `.await` with `ctx` discarded, which is the one
        // shape a remote procedure can take that no amount of surrounding
        // machinery recovers from: the loop's abort checkpoint
        // (`epsilon-core/src/exec.rs:731`) only stops calls that have not
        // *started*, and nothing anywhere puts a watchdog on tool execution. A
        // server that accepted a request and never replied took the run down
        // with it — `Esc` did nothing visible and the only exit was killing the
        // process.
        //
        // Both halves matter and they cover different readers. Cancellation is
        // the one a person triggers; the deadline is the one that fires when
        // nobody is watching, which is exactly when a wedged server is least
        // likely to be noticed. `Tool::execute`'s contract has said to poll
        // `ctx.token` all along (`epsilon-contracts/src/tool.rs:463-464`) —
        // this was the one built-in tool that did not, and its siblings
        // `webfetch` and `shell` both do (`webfetch.rs:181-184`,
        // `shell.rs:554-573`).
        //
        // The tool is dropped when either branch wins, which is what abandons
        // the in-flight request rather than leaking a task per hung call.
        let cancelled = ctx.token.clone();
        let call_fut = self.connector.call_tool(&self.tool_name, call.args);
        let text = tokio::select! {
            _ = cancelled.cancelled() => {
                return Err(ToolFailure::msg(format!(
                    "[aborted] MCP tool {}::{} cancelled by user before completion",
                    self.server_name, self.tool_name
                )));
            }
            answered = tokio::time::timeout(call_timeout(), call_fut) => {
                match answered {
                    Ok(Ok(t)) => t,
                    Ok(Err(e)) => {
                        // A failed call invalidates the probe: the NEXT call re-probes
                        // before attempting another round-trip.
                        self.last_probe_ms.store(0, std::sync::atomic::Ordering::Relaxed);
                        return Err(ToolFailure::msg(e));
                    }
                    Err(_) => {
                        // The deadline also invalidates the probe: a server that failed
                        // to answer once has earned a re-probe before the next attempt.
                        self.last_probe_ms.store(0, std::sync::atomic::Ordering::Relaxed);
                        return Err(ToolFailure::msg(format!(
                            "MCP tool {}::{} did not answer within {:?} and was abandoned",
                            self.server_name,
                            self.tool_name,
                            call_timeout()
                        )));
                    }
                }
            }
        };
        // Third-party MCP output is UNTRUSTED DATA (AGENTS.md contract): we pass it back
        // verbatim as text and never act on embedded instructions.
        let mut out = ToolOutput::text(text);
        out.details = serde_json::json!({ "mcp_server": self.server_name });
        Ok(out)
    }
}

/// Test/dev connector over an in-memory table — also the reference implementation of the
/// seam's contract.
/// Handler for in-memory fixture tools.
pub type InMemoryHandler = Arc<dyn Fn(serde_json::Value) -> Result<String, String> + Send + Sync>;

/// One registered fixture tool.
pub type InMemoryEntry = (String, serde_json::Value, InMemoryHandler);

/// In-process connector: the reference implementation of [`Connector`] and the test double
/// for bridge tests.
#[derive(Default)]
pub struct InMemoryConnector {
    /// tool -> entry
    pub tools: std::collections::HashMap<String, InMemoryEntry>,
    /// Test seam for DIRECTION-02: flips the health probe to unhealthy.
    pub dead: std::sync::atomic::AtomicBool,
}

impl InMemoryConnector {
    /// Mark the connector dead so `is_healthy` reports failure.
    pub fn mark_dead(&self) {
        self.dead.store(true, std::sync::atomic::Ordering::Relaxed);
    }

    /// Register a tool with a synchronous handler.
    pub fn add(
        &mut self,
        name: &str,
        description: &str,
        schema: serde_json::Value,
        handler: impl Fn(serde_json::Value) -> Result<String, String> + Send + Sync + 'static,
    ) {
        self.tools
            .insert(name.into(), (description.into(), schema, Arc::new(handler)));
    }
}

#[async_trait]
impl Connector for InMemoryConnector {
    async fn list_tools(&self) -> Result<Vec<McpToolInfo>, String> {
        Ok(self
            .tools
            .iter()
            .map(|(name, (desc, schema, _))| McpToolInfo {
                name: name.clone(),
                description: desc.clone(),
                schema: schema.clone(),
            })
            .collect())
    }

    async fn call_tool(&self, tool: &str, args: serde_json::Value) -> Result<String, String> {
        match self.tools.get(tool) {
            Some((_, _, handler)) => handler(args),
            None => Err(format!("unknown tool {tool}")),
        }
    }

    /// Test seam: healthy until `mark_dead()` is called.
    async fn is_healthy(&self) -> Result<(), String> {
        if self.dead.load(std::sync::atomic::Ordering::Relaxed) {
            Err("in-memory server marked dead".into())
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use epsilon_contracts::content::ContentBlock;
    use epsilon_core::ToolRegistry;

    #[tokio::test]
    async fn bridge_health_probe_fails_fast_when_server_dies() {
        let mut conn = InMemoryConnector::default();
        conn.add(
            "list_prs",
            "List open PRs.",
            serde_json::json!({"type": "object"}),
            |_| Ok("PR-1".into()),
        );
        let conn = Arc::new(conn);
        // Keep the concrete Arc for mark_dead(); bridge wants the trait object.
        let connector: Arc<dyn Connector> = conn.clone() as Arc<dyn Connector>;
        let mut registry = ToolRegistry::new();
        bridge(&mut registry, "github", Arc::clone(&connector))
            .await
            .unwrap();
        let name = qualified_name("github", "list_prs");
        let tool = registry.get(&name).expect("bridged tool registered");

        // Healthy path still works.
        let sink: epsilon_contracts::tool::ToolUpdateSink = Arc::new(|_| {});
        let ctx = ToolContext {
            session_id: "t".into(),
            observation_scope: std::sync::Arc::new(()),
            cwd: std::env::temp_dir(),
            spill_dir: std::env::temp_dir().join("eps-mcp"),
            token: Default::default(),
            task_host: None,
            goal_host: None,
        };
        let invocation = || ToolInvocation {
            tool_call_id: "tc_probe".into(),
            args: serde_json::json!({}),
        };
        let out = tool.execute(invocation(), ctx.clone(), sink.clone()).await;
        assert!(out.is_ok(), "healthy server must pass the probe");

        // Server dies -> next call fails FAST at the probe with an honest
        // message (not the handler's output, not an opaque transport error).
        conn.mark_dead();
        // Directly exercise the probe path: the TTL cache would mask this in
        // real use until expiry, and a failed call resets it (execute).
        let probe = conn.is_healthy().await;
        assert!(probe.is_err(), "dead server must fail its health probe");
        // The TTL cache would serve a stale OK for up to HEALTH_TTL_MS; the
        // probe contract is that the NEXT call after expiry fails fast. The
        // in-memory connector has no real clock dependency, so simulate expiry
        // by constructing a fresh registry (fresh tool, last_probe_ms = 0).
        let mut fresh_registry = ToolRegistry::new();
        bridge(&mut fresh_registry, "github", connector)
            .await
            .unwrap();
        let fresh_tool = fresh_registry.get(&name).expect("bridged tool registered");
        let err = fresh_tool
            .execute(invocation(), ctx, sink)
            .await
            .expect_err("dead server must fail");
        assert!(
            err.message.contains("marked dead"),
            "failure should come from the health probe: {}",
            err.message
        );
    }

    #[tokio::test]
    async fn bridge_registers_qualified_tools_and_routes_calls() {
        let mut conn = InMemoryConnector::default();
        conn.add(
            "list_prs",
            "List open PRs.",
            serde_json::json!({"type": "object"}),
            |_| Ok("PR-1 PR-2".into()),
        );

        let mut registry = ToolRegistry::new();
        let n = bridge(&mut registry, "github", Arc::new(conn))
            .await
            .unwrap();
        assert_eq!(n, 1);

        let name = qualified_name("github", "list_prs");
        let tool = registry.get(&name).expect("bridged tool registered");
        assert!(tool.descriptor().name == name);

        let sink: epsilon_contracts::tool::ToolUpdateSink = Arc::new(|_| {});
        let ctx = ToolContext {
            session_id: "t".into(),
            observation_scope: std::sync::Arc::new(()),
            cwd: std::env::temp_dir(),
            spill_dir: std::env::temp_dir().join("eps-mcp"),
            token: Default::default(),
            task_host: None,
            goal_host: None,
        };
        let out = tool
            .execute(
                ToolInvocation {
                    tool_call_id: "tc_m".into(),
                    args: serde_json::json!({}),
                },
                ctx,
                sink,
            )
            .await
            .unwrap();
        assert!(matches!(&out.content[0], ContentBlock::Text { text } if text.contains("PR-1")));
    }

    #[test]
    fn config_parses() {
        let cfg: McpServerConfig = serde_json::from_value(serde_json::json!({
            "name": "gh", "command": "mcp-server-github", "args": ["--stdio"]
        }))
        .unwrap();
        assert_eq!(cfg.name, "gh");
    }

    /// A connector whose tool call never answers — the shape of a wedged MCP server.
    ///
    /// `std::future::pending()` rather than a sleeping thread, because the bug these
    /// tests pin is precisely that nothing was *awaiting* the call: a blocking handler
    /// would hang the executor thread and make a `select!` look broken for the wrong
    /// reason.
    struct HangingConnector;

    #[async_trait]
    impl Connector for HangingConnector {
        async fn list_tools(&self) -> Result<Vec<McpToolInfo>, String> {
            Ok(vec![McpToolInfo {
                name: "hangs".into(),
                description: "never answers".into(),
                schema: serde_json::json!({ "type": "object" }),
            }])
        }

        async fn call_tool(&self, _tool: &str, _args: serde_json::Value) -> Result<String, String> {
            std::future::pending().await
        }

        async fn is_healthy(&self) -> Result<(), String> {
            Ok(())
        }
    }

    /// Build the bridged tool these tests drive.
    ///
    /// A plain `async fn`, not a future-polling harness: tokio is a dependency of this
    /// crate and every test here is a `#[tokio::test]`, so the bridge's own await is
    /// enough. The first version of this fixture hand-rolled a `RawWaker` and a
    /// yield-and-poll loop — a no-op waker never wakes anything, so it only worked
    /// because the loop spun, and it needed `unsafe` to do it. Reaching for the runtime
    /// already in the graph is the whole lesson of that.
    async fn hanging_tool() -> Arc<dyn Tool> {
        let mut registry = ToolRegistry::new();
        let connector: Arc<dyn Connector> = Arc::new(HangingConnector);
        bridge(&mut registry, "wedged", connector)
            .await
            .expect("the bridge registers");
        registry
            .get(&qualified_name("wedged", "hangs"))
            .expect("bridged tool registered")
    }

    /// **A cancelled MCP call returns instead of hanging forever.**
    ///
    /// `McpBridgeTool::execute` took the tool's `ToolContext` as `_ctx` and awaited
    /// `connector.call_tool(..)` with nothing racing it, so a server that accepted a
    /// request and never replied wedged the run: the loop's abort checkpoint
    /// (`epsilon-core/src/exec.rs:731`) only stops calls that have not *started*, and
    /// there is no watchdog on tool execution anywhere. `Esc` did nothing visible and
    /// the only exit was killing the process — which is the whole reason
    /// `Tool::execute`'s contract tells implementors to poll `ctx.token`
    /// (`epsilon-contracts/src/tool.rs:463-464`), and this was the one built-in tool
    /// that did not.
    #[tokio::test]
    async fn a_cancelled_mcp_call_returns_instead_of_hanging() {
        let tool = hanging_tool().await;
        let token = tokio_util::sync::CancellationToken::new();
        let child = token.clone();
        // Cancel from another task: the call is already in flight when the token fires,
        // which is the state that hung.
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            child.cancel();
        });
        let ctx = ToolContext {
            session_id: "t".into(),
            observation_scope: std::sync::Arc::new(()),
            cwd: std::env::temp_dir(),
            spill_dir: std::env::temp_dir().join("eps-mcp"),
            token: token.clone(),
            task_host: None,
            goal_host: None,
        };
        let err = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            tool.execute(
                ToolInvocation {
                    tool_call_id: "tc_cancel".into(),
                    args: serde_json::json!({}),
                },
                ctx,
                Arc::new(|_| {}),
            ),
        )
        .await
        .expect("the call must return rather than hang")
        .expect_err("a cancelled call is an error");
        assert!(
            err.message.contains("cancel") || err.message.contains("abort"),
            "the failure must say the call was cancelled, not what the server did: {}",
            err.message
        );
    }

    /// **A hung MCP call fails after a deadline instead of waiting for ever.**
    ///
    /// The other half of the same hole. Cancellation only helps when somebody presses
    /// Escape; a server that never answers and nobody watching still takes the run
    /// down. Paused tokio time makes this deterministic and instant: the deadline is
    /// reached the moment the call has nothing left to do.
    #[tokio::test(start_paused = true)]
    async fn a_hung_mcp_call_fails_after_a_deadline() {
        let tool = hanging_tool().await;
        let ctx = ToolContext {
            session_id: "t".into(),
            observation_scope: std::sync::Arc::new(()),
            cwd: std::env::temp_dir(),
            spill_dir: std::env::temp_dir().join("eps-mcp"),
            token: Default::default(),
            task_host: None,
            goal_host: None,
        };
        // An outer bound **longer than the production deadline**, which is the only shape
        // that gives both halves a reader wants: the product's own deadline fires first
        // (120 s against 300 s), and if it is ever removed this fails at 300 s instead of
        // hanging. Under paused tokio time both are instant. A shorter bound was tried
        // first and was wrong — it fired first and the test then asserted its own timeout
        // rather than the product's.
        let err = tokio::time::timeout(
            std::time::Duration::from_secs(300),
            tool.execute(
                ToolInvocation {
                    tool_call_id: "tc_hang".into(),
                    args: serde_json::json!({}),
                },
                ctx,
                Arc::new(|_| {}),
            ),
        )
        .await
        .expect("a hung MCP call must fail on the product's deadline, not hang")
        .expect_err("a call that never answers must fail");
        assert!(
            err.message.contains("did not answer")
                || err.message.contains("timed out")
                || err.message.contains("deadline"),
            "the failure must name the deadline: {}",
            err.message
        );
    }
}
