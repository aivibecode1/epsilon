//! OAuth login ceremonies — the half M46/M51 deliberately left open.
//!
//! Ported from pi-0.84.2 `packages/ai/src/auth/oauth/*.ts` (login half):
//! `pkce.ts` (verifier/challenge), `device-code.ts` (RFC 8628 polling),
//! `anthropic.ts` (Claude Pro/Max authorization-code + PKCE),
//! `openrouter.ts` (PKCE → permanent API key) and `xai.ts` (device flow).
//!
//! Two invariants carried over from M46/M51, unchanged:
//!
//! 1. **No embedded client ids.** pi hardcodes one per provider (Anthropic's
//!    is base64-obfuscated to dodge secret scanners). Epsilon resolves the
//!    id from `<PROVIDER>_OAUTH_CLIENT_ID` in the environment, then from
//!    `[oauth.<provider>] client_id` in the global `config.toml`, and
//!    otherwise fails tellingly, naming both ways to set it.
//! 2. **Transport split.** Pure helpers ([`generate_pkce`], [`parse_authorization_input`],
//!    [`CallbackListener`], [`poll_device_code`]) compile and test offline;
//!    the exchanges and the [`login`] orchestrator need the `real` feature
//!    for `reqwest` like every other network call.
//!
//! The orchestrator is deliberately **synchronous** and drives its own
//! current-thread runtime: both frontends run it off the UI thread (the CLI
//! before its main runtime exists, the TUI on a worker thread), so blocking
//! here is the honest shape.

use std::path::PathBuf;
use std::time::Duration;

/// RFC 7636 §4.1 verifier length in bytes (pi `pkce.ts` uses 32 → 43 chars
/// base64url, inside the 43–128 allowed range).
pub const PKCE_VERIFIER_BYTES: usize = 32;

/// pi `device-code.ts`: minimum poll interval, regardless of what the server
/// asks for.
pub const DEVICE_MIN_INTERVAL_MS: u64 = 1_000;

/// RFC 8628 §3.2: use 5 seconds when the server omits `interval`.
pub const DEVICE_DEFAULT_INTERVAL_SECS: u64 = 5;

/// RFC 8628 §3.5: `slow_down` without a server interval adds 5 seconds.
pub const DEVICE_SLOW_DOWN_INCREMENT_MS: u64 = 5_000;

/// How the flow obtains the user's authorization.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginKind {
    /// Authorization-code + PKCE: browser URL, code comes back via the local
    /// callback listener or a paste.
    Pkce,
    /// RFC 8628 device flow: short code shown to the user, we poll.
    Device,
}

/// What the device flow's token must be exchanged into before it can be spent
/// (M55). Most providers hand back the spendable token directly; GitHub's
/// device flow yields a long-lived *GitHub* token, and the Copilot bearer the
/// proxy accepts is minted from it at `copilot_internal/v2/token` — pi
/// `refreshGitHubCopilotAccessToken`, which the login and every later refresh
/// both go through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MintKind {
    /// The device token is the credential (xAI).
    None,
    /// Device token → Copilot bearer (GitHub Copilot).
    GitHubCopilot,
}

/// Token-endpoint body encoding (pi per-provider: JSON for Anthropic's
/// `postJson`, form-encoded for the rest).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExchangeStyle {
    /// `application/json` body.
    Json,
    /// `application/x-www-form-urlencoded` body.
    Form,
}

/// One provider's login ceremony (pi's per-file constants, minus the client id).
#[derive(Debug, Clone, Copy)]
pub struct LoginFlow {
    /// Provider id as stored in `auth.json`.
    pub provider: &'static str,
    /// Human label for pickers (`pi` `OAuthAuth.name`).
    pub label: &'static str,
    /// Which ceremony this provider speaks.
    pub kind: LoginKind,
    /// Env var carrying the OAuth client id (pi hardcodes it; we refuse to).
    pub client_id_env: &'static str,
    /// Scope string sent to the authorization endpoint.
    pub scopes: &'static str,
    /// Token endpoint (authorization-code exchange or device poll).
    pub token_url: &'static str,
    /// Authorization endpoint (`kind == Pkce`), empty for device flows.
    pub authorize_url: &'static str,
    /// Device-authorization endpoint (`kind == Device`), empty otherwise.
    pub device_url: &'static str,
    /// Loopback port for the callback listener (`kind == Pkce`); `0` binds an
    /// ephemeral port (pi `openrouter.ts` `server.listen(0, …)`).
    pub callback_port: u16,
    /// Callback path (`kind == Pkce`).
    pub callback_path: &'static str,
    /// Token-endpoint body encoding.
    pub style: ExchangeStyle,
    /// `true` when the authorization request must carry `state` (pi sends the
    /// verifier there for Anthropic; OpenRouter's endpoint takes no state).
    pub use_state: bool,
    /// `true` when the exchange returns a permanent API key instead of a
    /// token pair (pi `openrouter.ts` reads `body.key`).
    pub returns_key: bool,
    /// Extra exchange the device token needs before it is spendable (M55).
    pub mint: MintKind,
    /// `User-Agent` the provider's device endpoints expect (pi sends its
    /// editor identity to GitHub); empty when the provider wants none.
    pub user_agent: &'static str,
}

impl LoginFlow {
    /// Redirect URI for a fixed-port flow; `None` when the port is ephemeral
    /// (the caller substitutes the bound port). Only used by the login path.
    #[cfg_attr(not(feature = "real"), allow(dead_code))]
    fn redirect_uri(&self) -> Option<String> {
        if self.callback_port == 0 {
            None
        } else {
            Some(format!(
                "http://localhost:{}{}",
                self.callback_port, self.callback_path
            ))
        }
    }
}

/// Login-capable providers with a wire Epsilon can actually use today.
///
/// `openai-codex` stays absent on purpose: pi reaches it through the ChatGPT
/// backend, and neither models.dev nor the reference tree publishes that
/// model list — inventing ids and prices would break the "verify against the
/// source" rule (see `plans/018-github-copilot-and-codex-wires.md`). Store a
/// codex token the day the model data lands. `github-copilot` joined in M55
/// once its wire + catalog rows existed.
pub const LOGIN_FLOWS: &[LoginFlow] = &[
    LoginFlow {
        provider: "anthropic",
        label: "Anthropic (Claude Pro/Max)",
        kind: LoginKind::Pkce,
        client_id_env: "ANTHROPIC_OAUTH_CLIENT_ID",
        scopes: "org:create_api_key user:profile user:inference user:sessions:claude_code user:mcp_servers user:file_upload",
        token_url: "https://platform.claude.com/v1/oauth/token",
        authorize_url: "https://claude.ai/oauth/authorize",
        device_url: "",
        callback_port: 53692,
        callback_path: "/callback",
        style: ExchangeStyle::Json,
        use_state: true,
        returns_key: false,
        mint: MintKind::None,
        user_agent: "",
    },
    LoginFlow {
        provider: "openrouter",
        label: "OpenRouter",
        kind: LoginKind::Pkce,
        client_id_env: "OPENROUTER_OAUTH_CLIENT_ID",
        scopes: "",
        token_url: "https://openrouter.ai/api/v1/auth/keys",
        authorize_url: "https://openrouter.ai/auth",
        device_url: "",
        callback_port: 0,
        callback_path: "/oauth/callback",
        style: ExchangeStyle::Json,
        use_state: false,
        returns_key: true,
        mint: MintKind::None,
        user_agent: "",
    },
    LoginFlow {
        provider: "xai",
        label: "xAI (Grok)",
        kind: LoginKind::Device,
        client_id_env: "XAI_OAUTH_CLIENT_ID",
        scopes: "openid profile email offline_access grok-cli:access api:access",
        token_url: "https://auth.x.ai/oauth2/token",
        authorize_url: "",
        device_url: "https://auth.x.ai/oauth2/device/code",
        callback_port: 0,
        callback_path: "",
        style: ExchangeStyle::Form,
        use_state: false,
        returns_key: false,
        mint: MintKind::None,
        user_agent: "",
    },
    // M55: GitHub Copilot. Device flow on github.com (or the enterprise host),
    // then the mint that turns the GitHub token into the Copilot bearer. The
    // client id is pi's `Iv1.…`, resolved from the env/config like every other
    // ceremony (see `docs/qa/08-oauth-login-client-ids.md`); the enterprise
    // domain comes from `[oauth.github-copilot] enterprise_domain`.
    LoginFlow {
        provider: "github-copilot",
        label: "GitHub Copilot (subscription)",
        kind: LoginKind::Device,
        client_id_env: "GITHUB_COPILOT_OAUTH_CLIENT_ID",
        scopes: "read:user",
        token_url: "https://github.com/login/oauth/access_token",
        authorize_url: "",
        device_url: "https://github.com/login/device/code",
        callback_port: 0,
        callback_path: "",
        style: ExchangeStyle::Form,
        use_state: false,
        returns_key: false,
        mint: MintKind::GitHubCopilot,
        user_agent: "GitHubCopilotChat/0.35.0",
    },
];

/// Look up a provider's login ceremony.
pub fn login_flow(provider: &str) -> Option<LoginFlow> {
    LOGIN_FLOWS.iter().find(|f| f.provider == provider).copied()
}

/// Providers that can log in via OAuth, for pickers and guidance text.
pub fn oauth_providers() -> Vec<&'static str> {
    LOGIN_FLOWS.iter().map(|f| f.provider).collect()
}

/// Is `value` a flag-shaped login argument rather than a pasted key?
///
/// `epsilon auth login <provider> --oauth` and the TUI's
/// `/login <provider> oauth` both need to tell "start the ceremony" apart
/// from "here is an API key" without a second parser.
///
/// Deliberately NOT named `is_oauth_token`: that name is taken by the wire
/// layer's token-shape predicate (`providers::anthropic::is_oauth_token`),
/// which answers a different question — does this *key* look like an OAuth
/// access token.
pub fn is_oauth_ceremony_arg(value: &str) -> bool {
    matches!(value.trim(), "oauth" | "--oauth")
}

// ---------------------------------------------------------------------------
// PKCE
// ---------------------------------------------------------------------------

/// A generated PKCE pair (pi `generatePKCE`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pkce {
    /// The `code_verifier` sent at exchange time (never leaves the machine
    /// until then).
    pub verifier: String,
    /// The `code_challenge` (S256 of the verifier) sent in the auth URL.
    pub challenge: String,
}

/// base64url without padding (RFC 7636 Appendix A encoding).
pub fn base64url(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// S256 challenge: base64url(SHA-256(verifier)).
pub fn s256_challenge(verifier: &str) -> String {
    use sha2::{Digest, Sha256};
    base64url(&Sha256::digest(verifier.as_bytes()))
}

/// Generate a fresh PKCE pair from the OS CSPRNG.
///
/// Fails only when the platform randomness source is unavailable (getrandom),
/// which is a real error and never silently downgraded to a weak source.
pub fn generate_pkce() -> Result<Pkce, String> {
    let mut bytes = [0u8; PKCE_VERIFIER_BYTES];
    getrandom::fill(&mut bytes).map_err(|e| format!("no OS randomness for PKCE: {e}"))?;
    let verifier = base64url(&bytes);
    let challenge = s256_challenge(&verifier);
    Ok(Pkce {
        verifier,
        challenge,
    })
}

/// A random `state` value (base64url of 16 CSPRNG bytes) for flows that send
/// one; also used as the OpenRouter callback-path suffix so a stale browser
/// tab cannot post into a fresh listener.
pub fn random_state() -> Result<String, String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|e| format!("no OS randomness for OAuth state: {e}"))?;
    Ok(base64url(&bytes))
}

// ---------------------------------------------------------------------------
// Manual paste parsing (pi `parseAuthorizationInput`)
// ---------------------------------------------------------------------------

/// What the user pasted: the code, and the state when the paste carried one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AuthorizationInput {
    /// The authorization code.
    pub code: Option<String>,
    /// The `state` parameter, when present.
    pub state: Option<String>,
}

/// Parse a pasted redirect URL, `code#state`, `code=…&state=…` query string,
/// or a bare code. pi tolerates all four because the callback page may be
/// unreachable (ssh, other machine) and users copy whatever the address bar
/// shows.
pub fn parse_authorization_input(input: &str) -> AuthorizationInput {
    let value = input.trim();
    if value.is_empty() {
        return AuthorizationInput::default();
    }
    if value.starts_with("http://") || value.starts_with("https://") {
        // A redirect URL may carry a fragment (`#…`) — strip it before the
        // query parse or it glues itself onto the last parameter (pi's
        // `new URL(value)` drops it for the same reason).
        let without_fragment = value.split('#').next().unwrap_or(value);
        let query = without_fragment
            .split_once('?')
            .map(|(_, q)| q)
            .unwrap_or("");
        let params = parse_query(query);
        let get = |name: &str| {
            params
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.clone())
        };
        return AuthorizationInput {
            code: get("code"),
            state: get("state"),
        };
    }
    // `code#state` (pi's second branch) — Anthropic's out-of-band format.
    if let Some((code, state)) = value.split_once('#') {
        return AuthorizationInput {
            code: non_empty(code),
            state: non_empty(state),
        };
    }
    if value.contains("code=") {
        let params = parse_query(value.trim_start_matches('?'));
        let get = |name: &str| {
            params
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.clone())
        };
        return AuthorizationInput {
            code: get("code"),
            state: get("state"),
        };
    }
    AuthorizationInput {
        code: non_empty(value),
        state: None,
    }
}

fn non_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Parse `a=1&b=2` (percent-decoding `+` as space, like a form body would).
fn parse_query(query: &str) -> Vec<(String, String)> {
    query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            (percent_decode(k), percent_decode(v))
        })
        .collect()
}

/// Percent-decode a query component (`+` → space, `%XX` → byte). Invalid
/// escapes pass through verbatim rather than failing the whole login.
fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                let hex = &value[i + 1..i + 3];
                match u8::from_str_radix(hex, 16) {
                    Ok(b) => {
                        out.push(b);
                        i += 3;
                    }
                    Err(_) => {
                        out.push(bytes[i]);
                        i += 1;
                    }
                }
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

// ---------------------------------------------------------------------------
// Callback listener (pi's `http.createServer`, std::net here)
// ---------------------------------------------------------------------------

/// A one-shot loopback HTTP listener that captures the OAuth redirect.
///
/// pi binds the same way (`127.0.0.1`, fixed port for Anthropic, port 0 for
/// OpenRouter). The listener accepts in a background thread, answers the
/// browser with a small HTML page, and hands the `(code, state)` pair to the
/// caller exactly once. A `timeout` bounds how long the thread lives so a
/// forgotten login cannot leak a listener forever.
pub struct CallbackListener {
    rx: std::sync::mpsc::Receiver<Result<CallbackCode, String>>,
    port: u16,
}

/// The code + state captured from the browser redirect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallbackCode {
    /// `code` query parameter.
    pub code: String,
    /// `state` query parameter (`""` when the flow does not send one).
    pub state: String,
}

impl CallbackListener {
    /// Bind `host:port` (port `0` = ephemeral) and start accepting.
    ///
    /// `expected_state` is compared against the redirect's `state` — a
    /// mismatch is answered with an error page and reported, never accepted
    /// (pi `anthropic.ts` state check).
    pub fn bind(
        host: &str,
        port: u16,
        path: &str,
        expected_state: &str,
        timeout: Duration,
    ) -> Result<Self, String> {
        use std::io::Read as _;
        use std::net::TcpListener;

        let listener = TcpListener::bind((host, port)).map_err(|e| {
            format!("cannot bind the OAuth callback listener on {host}:{port}: {e}")
        })?;
        let port = listener
            .local_addr()
            .map_err(|e| format!("callback listener has no local address: {e}"))?
            .port();
        listener
            .set_nonblocking(true)
            .map_err(|e| format!("cannot make the callback listener non-blocking: {e}"))?;
        let (tx, rx) = std::sync::mpsc::channel();
        let path = path.to_string();
        let expected_state = expected_state.to_string();
        std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + timeout;
            loop {
                if std::time::Instant::now() >= deadline {
                    let _ = tx.send(Err(format!(
                        "timed out after {}s waiting for the browser callback",
                        timeout.as_secs()
                    )));
                    return;
                }
                match listener.accept() {
                    Ok((mut stream, _addr)) => {
                        // Accepted sockets inherit non-blocking mode on
                        // Windows, so a single `read` can come back
                        // `WouldBlock` (or partial) — loop until the request
                        // line is in hand or the short deadline passes.
                        let mut request = String::new();
                        let read_deadline = std::time::Instant::now() + Duration::from_millis(500);
                        loop {
                            let mut buf = [0u8; 4096];
                            match stream.read(&mut buf) {
                                Ok(0) => break,
                                Ok(n) => {
                                    request.push_str(&String::from_utf8_lossy(&buf[..n]));
                                    if request.contains("\r\n") || request.contains('\n') {
                                        break;
                                    }
                                }
                                Err(ref e)
                                    if e.kind() == std::io::ErrorKind::WouldBlock
                                        || e.kind() == std::io::ErrorKind::TimedOut =>
                                {
                                    if std::time::Instant::now() >= read_deadline {
                                        break;
                                    }
                                    std::thread::sleep(Duration::from_millis(5));
                                }
                                Err(_) => break,
                            }
                        }
                        let line = request.lines().next().unwrap_or("");
                        match parse_callback_request_line(line, &path) {
                            Ok(Ok(params)) => {
                                let get = |name: &str| {
                                    params
                                        .iter()
                                        .find(|(k, _)| k == name)
                                        .map(|(_, v)| v.clone())
                                        .unwrap_or_default()
                                };
                                let error = get("error");
                                if !error.is_empty() {
                                    let _ = respond(&mut stream, 400, &error_page(&error));
                                    continue;
                                }
                                let code = get("code");
                                let state = get("state");
                                if code.is_empty() {
                                    let _ = respond(
                                        &mut stream,
                                        400,
                                        &error_page("missing code parameter"),
                                    );
                                    continue;
                                }
                                if !expected_state.is_empty() && state != expected_state {
                                    let _ = respond(
                                        &mut stream,
                                        400,
                                        &error_page("state mismatch — login not accepted"),
                                    );
                                    continue;
                                }
                                let _ = respond(&mut stream, 200, &success_page());
                                let _ = tx.send(Ok(CallbackCode { code, state }));
                                return;
                            }
                            Ok(Err(status)) => {
                                let _ = respond(
                                    &mut stream,
                                    status,
                                    &error_page("callback route not found"),
                                );
                            }
                            Err(message) => {
                                let _ = respond(&mut stream, 400, &error_page(&message));
                            }
                        }
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(40));
                    }
                    Err(e) => {
                        let _ = tx.send(Err(format!("callback listener failed: {e}")));
                        return;
                    }
                }
            }
        });
        Ok(Self { rx, port })
    }

    /// The port actually bound (useful for ephemeral-port flows).
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Wait up to `timeout` for the redirect. `None` = nothing arrived in
    /// time (the caller falls back to the paste prompt).
    pub fn recv_timeout(&self, timeout: Duration) -> Option<Result<CallbackCode, String>> {
        self.rx.recv_timeout(timeout).ok()
    }

    /// Non-blocking peek, for UIs that serve two input sources at once.
    pub fn try_recv(&self) -> Option<Result<CallbackCode, String>> {
        self.rx.try_recv().ok()
    }
}

/// Parse one HTTP request line: `GET /callback?code=..&state=.. HTTP/1.1`.
///
/// `Ok(Ok(params))` = a request for `path`; `Ok(Err(status))` = wrong route
/// (404); `Err(_)` = malformed request line (400).
pub fn parse_callback_request_line(
    line: &str,
    path: &str,
) -> Result<Result<Vec<(String, String)>, u16>, String> {
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let target = parts.next().unwrap_or("");
    if method.is_empty() || target.is_empty() {
        return Err("malformed HTTP request line".to_string());
    }
    let (target_path, query) = match target.split_once('?') {
        Some((p, q)) => (p, q),
        None => (target, ""),
    };
    if target_path != path {
        return Ok(Err(404));
    }
    Ok(Ok(parse_query(query)))
}

fn respond(stream: &mut std::net::TcpStream, status: u16, body: &str) -> std::io::Result<()> {
    use std::io::Write as _;
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        _ => "Not Found",
    };
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes())?;
    stream.flush()
}

fn success_page() -> String {
    "<!doctype html><html><body style=\"font-family:system-ui;padding:2rem\">\
     <h2>Epsilon: authentication complete</h2>\
     <p>You can close this window and return to the terminal.</p></body></html>"
        .to_string()
}

fn error_page(message: &str) -> String {
    format!(
        "<!doctype html><html><body style=\"font-family:system-ui;padding:2rem\">\
         <h2>Epsilon: authentication failed</h2><p>{message}</p></body></html>"
    )
}

// ---------------------------------------------------------------------------
// Device flow (RFC 8628 polling, pi `device-code.ts`)
// ---------------------------------------------------------------------------

/// One poll's verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DevicePoll<T> {
    /// Server says keep waiting (`authorization_pending`).
    Pending,
    /// Server says slow down; `Some(secs)` when it supplied a new interval.
    SlowDown(Option<u64>),
    /// Token obtained.
    Complete(T),
    /// Terminal failure with a telling message.
    Failed(String),
}

/// Poll a device-authorization token endpoint until it completes, fails, or
/// the device-code lifetime runs out — pi `pollOAuthDeviceCodeFlow`
/// semantics verbatim: minimum 1s interval, wait before the first poll,
/// `slow_down` honours a server-provided interval else adds 5s, and the
/// timeout message distinguishes "saw at least one `slow_down`" (usually
/// clock drift on WSL/VMs) from a plain timeout.
pub async fn poll_device_code<T, F, Fut>(
    interval_secs: Option<u64>,
    expires_in_secs: u64,
    mut poll: F,
) -> Result<T, String>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = DevicePoll<T>>,
{
    let deadline = tokio::time::Instant::now() + Duration::from_secs(expires_in_secs);
    let mut interval_ms = std::cmp::max(
        DEVICE_MIN_INTERVAL_MS,
        interval_secs.unwrap_or(DEVICE_DEFAULT_INTERVAL_SECS) * 1000,
    );
    let mut slow_downs = 0u32;
    // pi waits one interval before the first poll (`waitBeforeFirstPoll`).
    tokio::time::sleep(Duration::from_millis(interval_ms)).await;
    loop {
        if tokio::time::Instant::now() >= deadline {
            break;
        }
        match poll().await {
            DevicePoll::Complete(value) => return Ok(value),
            DevicePoll::Failed(message) => return Err(message),
            DevicePoll::SlowDown(server_interval) => {
                slow_downs += 1;
                interval_ms = match server_interval {
                    Some(secs) if secs > 0 => std::cmp::max(DEVICE_MIN_INTERVAL_MS, secs * 1000),
                    _ => std::cmp::max(
                        DEVICE_MIN_INTERVAL_MS,
                        interval_ms + DEVICE_SLOW_DOWN_INCREMENT_MS,
                    ),
                };
            }
            DevicePoll::Pending => {}
        }
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        tokio::time::sleep(std::cmp::min(Duration::from_millis(interval_ms), remaining)).await;
    }
    Err(if slow_downs > 0 {
        "device flow timed out after one or more slow_down responses — this is usually clock drift in a WSL or VM environment; sync the clock and retry".to_string()
    } else {
        "device flow timed out".to_string()
    })
}

/// A parsed device-authorization response (pi `parseDeviceCode`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceCodeInfo {
    /// Opaque code we poll with.
    pub device_code: String,
    /// Short code the user types in the browser.
    pub user_code: String,
    /// Where the user types it.
    pub verification_uri: String,
    /// `verification_uri_complete` when the server offers a pre-filled link.
    pub verification_uri_complete: Option<String>,
    /// Poll interval the server asked for.
    pub interval_seconds: Option<u64>,
    /// Lifetime of the device code.
    pub expires_in_seconds: u64,
}

/// Parse a device-authorization response body. Rejects a non-http(s)
/// `verification_uri` (pi refuses to hand an arbitrary scheme to the
/// browser opener).
pub fn parse_device_code(body: &str) -> Result<DeviceCodeInfo, String> {
    let v: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| format!("device authorization returned invalid JSON: {e}"))?;
    let required = |name: &str| -> Result<String, String> {
        v.get(name)
            .and_then(|x| x.as_str())
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.to_string())
            .ok_or_else(|| format!("device authorization response missing {name}"))
    };
    // pi `parseDeviceCode` reads the fields in this order (`device_code`,
    // `user_code`, `verification_uri`, `expires_in`) — a malformed response
    // names the first missing field, not an arbitrary one.
    let user_code = required("user_code")?;
    let verification_uri = required("verification_uri")?;
    if !verification_uri.starts_with("http://") && !verification_uri.starts_with("https://") {
        return Err(format!(
            "refusing untrusted verification_uri '{verification_uri}' (http/https only)"
        ));
    }
    let expires_in = v
        .get("expires_in")
        .and_then(|x| x.as_u64())
        .filter(|n| *n > 0)
        .ok_or_else(|| "device authorization response missing expires_in".to_string())?;
    Ok(DeviceCodeInfo {
        device_code: required("device_code")?,
        user_code,
        verification_uri,
        verification_uri_complete: v
            .get("verification_uri_complete")
            .and_then(|x| x.as_str())
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.to_string()),
        interval_seconds: v.get("interval").and_then(|x| x.as_u64()),
        expires_in_seconds: expires_in,
    })
}

// ---------------------------------------------------------------------------
// Client-id resolution
// ---------------------------------------------------------------------------

/// `[oauth.<provider>]` from the global `config.toml`, resolved by the caller.
///
/// **M224: the value arrives; this crate no longer reads the file.** Two functions here each parsed
/// `config.toml` for one key — `client_id_from_config` and `enterprise_domain_from_config` — which
/// is two reads of one file per login, two places that could disagree about the same table, and the
/// reason `epsilon-llm` carries a `toml` dependency. The parse now lives in `epsilon-core::config`,
/// which is the one reader of that file, and this type lives *here* because `epsilon-core` depends
/// on `epsilon-llm` and not the other way round (M223's asymmetry, which decides where a value type
/// can live and who fills it).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OAuthConfig {
    /// `[oauth.<provider>] client_id`. Never a built-in default: pi embeds its own registration per
    /// provider and Epsilon will not ship someone else's credential.
    pub client_id: Option<String>,
    /// `[oauth.<provider>] enterprise_domain` — GitHub Copilot Business/Enterprise accounts run the
    /// ceremony on the customer's host and mint on `api.<host>`.
    pub enterprise_domain: Option<String>,
}

/// Resolve a flow's OAuth client id: environment first, then the caller's configured value.
///
/// Never a built-in default: pi embeds its own registration per provider and
/// Epsilon will not ship someone else's credential. The error names both
/// ways to set it, plus the exact TOML snippet.
pub fn resolve_client_id(flow: &LoginFlow, config: &OAuthConfig) -> Result<String, String> {
    if let Ok(value) = std::env::var(flow.client_id_env)
        && !value.trim().is_empty()
    {
        return Ok(value.trim().to_string());
    }
    if let Some(id) = config
        .client_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return Ok(id.to_string());
    }
    Err(format!(
        "no OAuth client id for '{}' — either export {} or add\n    [oauth.{}]\n    client_id = \"…\"\nto ~/.config/epsilon/config.toml (pi embeds its own id; Epsilon never ships another project's credential)",
        flow.provider, flow.client_id_env, flow.provider
    ))
}

// ---------------------------------------------------------------------------
// Authorization URL
// ---------------------------------------------------------------------------

/// Build the browser URL for a PKCE flow (pi per-provider parameter sets).
///
/// `redirect_uri` is passed in because ephemeral-port flows only know it
/// after the listener binds.
pub fn authorize_url(
    flow: &LoginFlow,
    client_id: &str,
    redirect_uri: &str,
    pkce: &Pkce,
    state: &str,
) -> String {
    let mut params: Vec<(String, String)> = vec![
        ("client_id".into(), client_id.to_string()),
        ("response_type".into(), "code".into()),
        ("redirect_uri".into(), redirect_uri.to_string()),
        ("code_challenge".into(), pkce.challenge.clone()),
        ("code_challenge_method".into(), "S256".into()),
    ];
    if !flow.scopes.is_empty() {
        params.push(("scope".into(), flow.scopes.to_string()));
    }
    if flow.use_state {
        params.push(("state".into(), state.to_string()));
    }
    // Anthropic's endpoint also wants `code=true` (pi `anthropic.ts`); it is
    // harmless for the others, so only send it where pi does.
    if flow.provider == "anthropic" {
        params.insert(0, ("code".into(), "true".into()));
    }
    let query = params
        .iter()
        .map(|(k, v)| format!("{}={}", form_encode(k), form_encode(v)))
        .collect::<Vec<_>>()
        .join("&");
    format!("{}?{query}", flow.authorize_url)
}

/// Percent-encode a query component (alphanumerics + `-_.~` pass through).
pub fn form_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for b in value.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Responses
// ---------------------------------------------------------------------------

/// What the token endpoint handed back for a PKCE exchange.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExchangeOutcome {
    /// A token pair (Anthropic, xAI-style).
    Pair {
        /// Short-lived access token.
        access: String,
        /// Long-lived refresh token.
        refresh: String,
        /// Expiry epoch millis, pi's 5-minute skew already applied.
        expires: u64,
        /// Provider extras to store beside the pair (M55: Copilot's enterprise
        /// domain). `None` for every flow that has none.
        env: Option<std::collections::BTreeMap<String, String>>,
    },
    /// A permanent API key (OpenRouter).
    Key(String),
    /// A device token that still needs minting into the spendable credential
    /// (M55: GitHub's device token → Copilot bearer). Never persisted as-is —
    /// the flow mints it before [`persist`] sees anything.
    Device(String),
}

/// Parse a PKCE token-endpoint response. `now_millis` is injected so tests
/// pin the skew math.
pub fn parse_exchange_response(
    flow: &LoginFlow,
    body: &str,
    now_millis: u64,
) -> Result<ExchangeOutcome, String> {
    let v: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| format!("token exchange returned invalid JSON: {e}"))?;
    if flow.returns_key {
        let key = v
            .get("key")
            .and_then(|x| x.as_str())
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| "token exchange response carries no \"key\"".to_string())?;
        return Ok(ExchangeOutcome::Key(key.to_string()));
    }
    let access = v
        .get("access_token")
        .and_then(|x| x.as_str())
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| "token exchange response missing access_token".to_string())?;
    let refresh = v
        .get("refresh_token")
        .and_then(|x| x.as_str())
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| {
            "token exchange response carries no refresh_token — a login without one cannot be kept alive"
                .to_string()
        })?;
    let expires_in = v
        .get("expires_in")
        .and_then(|x| x.as_u64())
        .ok_or_else(|| "token exchange response missing expires_in".to_string())?;
    Ok(ExchangeOutcome::Pair {
        access: access.to_string(),
        refresh: refresh.to_string(),
        expires: now_millis
            .saturating_add(expires_in.saturating_mul(1000))
            .saturating_sub(crate::oauth::OAUTH_EXPIRY_SKEW_MS),
        env: None,
    })
}

/// Build the device-authorization request body (pi `xai.ts` `postForm`).
pub fn device_code_request_body(flow: &LoginFlow, client_id: &str) -> String {
    let mut pairs = vec![
        ("client_id".to_string(), client_id.to_string()),
        ("scope".to_string(), flow.scopes.to_string()),
    ];
    if flow.provider == "xai" {
        // pi tags the referrer so xAI can attribute the client.
        pairs.push(("referrer".to_string(), "epsilon".to_string()));
    }
    pairs
        .iter()
        .map(|(k, v)| format!("{}={}", form_encode(k), form_encode(v)))
        .collect::<Vec<_>>()
        .join("&")
}

/// Build the device-authorization poll body (RFC 8628 §3.4).
pub fn device_poll_body(flow: &LoginFlow, client_id: &str, device_code: &str) -> String {
    let _ = flow;
    [
        (
            "grant_type".to_string(),
            "urn:ietf:params:oauth:grant-type:device_code".to_string(),
        ),
        ("client_id".to_string(), client_id.to_string()),
        ("device_code".to_string(), device_code.to_string()),
    ]
    .iter()
    .map(|(k, v)| format!("{}={}", form_encode(k), form_encode(v)))
    .collect::<Vec<_>>()
    .join("&")
}

/// Parse one device-poll response: token pair, `authorization_pending`,
/// `slow_down`, or a terminal error (pi `xai.ts` `pollForTokens`).
pub fn parse_device_poll(
    flow: &LoginFlow,
    body: &str,
    now_millis: u64,
) -> DevicePoll<ExchangeOutcome> {
    if let Some(error) = device_poll_error(flow, body) {
        return error;
    }
    match parse_exchange_response(flow, body, now_millis) {
        Ok(outcome) => DevicePoll::Complete(outcome),
        Err(message) => DevicePoll::Failed(message),
    }
}

/// The RFC 8628 progress/failure arms every device endpoint shares, or `None`
/// when the body is a success payload. Extracted so GitHub's success shape
/// (a bare `access_token`, no expiry) reuses the identical vocabulary instead
/// of a second copy of it.
pub fn device_poll_error(flow: &LoginFlow, body: &str) -> Option<DevicePoll<ExchangeOutcome>> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(body) else {
        return Some(DevicePoll::Failed(format!(
            "device token endpoint returned invalid JSON: {}",
            body.chars().take(200).collect::<String>()
        )));
    };
    let error = v.get("error").and_then(|x| x.as_str())?;
    Some(match error {
        "authorization_pending" => DevicePoll::Pending,
        "slow_down" => DevicePoll::SlowDown(v.get("interval").and_then(|x| x.as_u64())),
        "access_denied" | "authorization_denied" => {
            DevicePoll::Failed(format!("{} device authorization was denied", flow.provider))
        }
        "expired_token" => DevicePoll::Failed(format!("{} device code expired", flow.provider)),
        other => {
            let description = v
                .get("error_description")
                .and_then(|x| x.as_str())
                .unwrap_or("");
            DevicePoll::Failed(format!(
                "device flow failed: {other}{}",
                if description.is_empty() {
                    String::new()
                } else {
                    format!(": {description}")
                }
            ))
        }
    })
}

/// Parse a GitHub device-flow success body (`{access_token, token_type?,
/// scope?}`) — no expiry and no refresh token, because the GitHub token is
/// long-lived and the short-lived credential is minted from it afterwards
/// (pi `DeviceTokenSuccessResponse`).
pub fn parse_github_device_token(body: &str) -> Result<String, String> {
    let v: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| format!("device token endpoint returned invalid JSON: {e}"))?;
    v.get("access_token")
        .and_then(|x| x.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.to_string())
        .ok_or_else(|| "device token response carries no access_token".to_string())
}

/// [`parse_device_poll`] for the flow's provider: Copilot's device endpoint
/// answers the device-flow vocabulary above but its success body is a GitHub
/// token that still has to be minted, so the two shapes get their own parsers.
pub fn parse_device_poll_for(
    flow: &LoginFlow,
    body: &str,
    now_millis: u64,
) -> DevicePoll<ExchangeOutcome> {
    if flow.mint == MintKind::None {
        return parse_device_poll(flow, body, now_millis);
    }
    if let Some(error) = device_poll_error(flow, body) {
        return error;
    }
    match parse_github_device_token(body) {
        Ok(token) => DevicePoll::Complete(ExchangeOutcome::Device(token)),
        Err(message) => DevicePoll::Failed(message),
    }
}

// ---------------------------------------------------------------------------
// Interactive surface
// ---------------------------------------------------------------------------

/// A parsed device code, as handed to the user interface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevicePrompt {
    /// Short code the user types in the browser.
    pub user_code: String,
    /// Where to type it.
    pub verification_uri: String,
    /// Lifetime in seconds (for progress messages).
    pub expires_in_seconds: u64,
}

/// Everything the flow needs from the frontend (CLI stdin, TUI overlay).
///
/// Implementations are called from the thread running [`login`], never from
/// the render loop, so blocking here is fine.
pub trait LoginUi {
    /// Progress / instruction line.
    fn notify(&mut self, message: &str);
    /// Show the device code + URL (device flows).
    fn device_code(&mut self, prompt: &DevicePrompt);
    /// Block until an authorization code arrives: from the browser callback
    /// when `callback` is given and fires, or from a paste. `None` = the user
    /// gave up / the deadline passed.
    fn wait_for_code(&mut self, callback: Option<&CallbackListener>) -> Option<String>;
}

/// What a completed login stored (for the success notice).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoginSaved {
    /// A token pair was written to `auth.json`.
    Oauth {
        /// Provider id.
        provider: String,
        /// File written.
        path: PathBuf,
        /// Expiry epoch millis.
        expires: u64,
    },
    /// A permanent API key was written (OpenRouter).
    Key {
        /// Provider id.
        provider: String,
        /// File written.
        path: PathBuf,
    },
}

/// Best-effort browser open (pi shells out to the platform opener too).
/// Returns whether the spawn succeeded — a failure only changes the message,
/// the URL is always printed.
pub fn open_browser(url: &str) -> bool {
    #[cfg(target_os = "windows")]
    let (program, args): (&str, Vec<&str>) = ("cmd", vec!["/C", "start", "", url]);
    #[cfg(target_os = "macos")]
    let (program, args): (&str, Vec<&str>) = ("open", vec![url]);
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let (program, args): (&str, Vec<&str>) = ("xdg-open", vec![url]);
    std::process::Command::new(program)
        .args(args)
        // Closed stdin, like every other child this harness spawns. `cmd /C start`
        // is not interactive, and leaving stdin unset means it INHERITS the TUI's
        // console handle — the same defect M197 fixed on the tool paths, in a spot
        // nobody had looked at because nothing appears to "run a command" here.
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .is_ok()
}

// ---------------------------------------------------------------------------
// Orchestrator (`real`: needs reqwest)
// ---------------------------------------------------------------------------

/// Run a provider's login ceremony end to end and persist the credential.
///
/// `allow_callback` lets a frontend that cannot host a loopback listener
/// (or where the browser lives on another machine) go straight to paste.
/// Every failure path returns a telling error and stores nothing.
///
/// `config` is the caller's reading of `[oauth.<provider>]` (M224); this function no longer opens
/// `config.toml` itself.
#[cfg(feature = "real")]
pub fn login(
    provider: &str,
    config: &OAuthConfig,
    ui: &mut dyn LoginUi,
    allow_callback: bool,
) -> Result<LoginSaved, String> {
    let flow = login_flow(provider).ok_or_else(|| {
        format!(
            "no OAuth login for '{provider}' — supported: {}",
            oauth_providers().join(", ")
        )
    })?;
    let client_id = resolve_client_id(&flow, config)?;
    // GitHub Copilot's Business/Enterprise accounts run the whole ceremony on
    // the customer's host and mint on `api.<host>` (pi asks for it
    // interactively; a config value keeps the frontends identical).
    let enterprise = config.enterprise_domain.as_deref();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("cannot start the login runtime: {e}"))?;
    let outcome = match flow.kind {
        LoginKind::Pkce => runtime.block_on(run_pkce(&flow, &client_id, ui, allow_callback))?,
        LoginKind::Device => runtime.block_on(run_device(&flow, &client_id, enterprise, ui))?,
    };
    persist(&flow, outcome, enterprise)
}

#[cfg(test)]
mod oauth_config_resolution_tests {
    use super::*;

    /// M224: resolution is **environment first, then the caller's value, then a telling error**.
    ///
    /// This ordering had no test while it was reachable through a file: the two tests that covered
    /// the *read* could not express "the environment wins", because the read always happened and the
    /// environment check sat above it. With the value handed in the whole rule is a pure function,
    /// so it is pinned here.
    ///
    /// **The environment is the runner's, and that is the honest shape of this test**: it asserts
    /// the *rule* against whatever `client_id_env` happens to hold, and skips only the
    /// unreachable branch when a value is genuinely present. A test that cleared the variable would
    /// need `set_var`, which is unsafe in edition 2024, and the harness runs tests in threads where
    /// an environment write would leak into every other test in the process.
    #[test]
    fn the_environment_wins_then_the_configured_value_then_a_telling_error() {
        let flow = login_flow("anthropic").expect("anthropic has a flow");
        let env = flow.client_id_env;
        let inherited = std::env::var(env).ok().filter(|v| !v.trim().is_empty());

        let configured = OAuthConfig {
            client_id: Some("cid-from-config".into()),
            enterprise_domain: None,
        };
        assert_eq!(
            resolve_client_id(&flow, &configured).unwrap(),
            inherited
                .clone()
                .unwrap_or_else(|| "cid-from-config".into()),
            "the environment wins when it is set; otherwise the caller's value is used"
        );

        // A blank configured value is absent, not a blank credential.
        let blank = OAuthConfig {
            client_id: Some("   ".into()),
            enterprise_domain: None,
        };
        match &inherited {
            Some(_) => {} // an inherited value makes the "nothing to resolve from" case unreachable
            None => {
                let err = resolve_client_id(&flow, &OAuthConfig::default())
                    .expect_err("nothing to resolve from");
                assert!(err.contains(env), "the error names the variable: {err}");
                assert!(err.contains("[oauth.anthropic]"), "and the table: {err}");
                assert!(
                    err.contains("client_id"),
                    "and the key, because the user has to write it: {err}"
                );
                assert!(
                    resolve_client_id(&flow, &blank).is_err(),
                    "a blank value is absent, so the error is still the right answer"
                );
            }
        }
    }

    /// `OAuthConfig::default()` is the "no configuration" answer every caller falls back to without
    /// a wrapper, and it must not invent values.
    #[test]
    fn the_default_config_claims_nothing() {
        let cfg = OAuthConfig::default();
        assert_eq!(cfg.client_id, None);
        assert_eq!(cfg.enterprise_domain, None);
    }
}

/// PKCE half: listener (when allowed) + browser + paste fallback, then the
/// authorization-code exchange.
#[cfg(feature = "real")]
async fn run_pkce(
    flow: &LoginFlow,
    client_id: &str,
    ui: &mut dyn LoginUi,
    allow_callback: bool,
) -> Result<ExchangeOutcome, String> {
    let pkce = generate_pkce()?;
    let state = random_state()?;
    // Ephemeral-port flows (OpenRouter) include a random path suffix so a
    // stale tab cannot deliver a code into a fresh listener.
    let path = if flow.callback_port == 0 {
        format!("{}/{}", flow.callback_path, random_state()?)
    } else {
        flow.callback_path.to_string()
    };
    let listener = if allow_callback {
        match CallbackListener::bind(
            "127.0.0.1",
            flow.callback_port,
            &path,
            if flow.use_state { &state } else { "" },
            Duration::from_secs(600),
        ) {
            Ok(listener) => Some(listener),
            Err(e) => {
                ui.notify(&format!(
                    "callback listener unavailable ({e}) — paste the redirect URL instead"
                ));
                None
            }
        }
    } else {
        None
    };
    let redirect_uri = match (&listener, flow.redirect_uri()) {
        (Some(listener), _) => format!("http://localhost:{}{path}", listener.port()),
        (None, Some(fixed)) => fixed,
        (None, None) => format!("http://localhost{path}"),
    };
    let url = authorize_url(flow, client_id, &redirect_uri, &pkce, &state);
    ui.notify(&format!(
        "open this URL to authorize {} (opening your browser too):\n  {url}",
        flow.label
    ));
    let _ = open_browser(&url);
    let pasted = ui.wait_for_code(listener.as_ref());
    let raw = pasted.ok_or_else(|| "login cancelled — nothing stored".to_string())?;
    let parsed = parse_authorization_input(&raw);
    if let Some(returned) = &parsed.state
        && flow.use_state
        && returned != &state
    {
        return Err(
            "OAuth state mismatch — the pasted redirect did not come from this login".into(),
        );
    }
    let code = parsed
        .code
        .ok_or_else(|| "no authorization code in the pasted value".to_string())?;
    ui.notify("exchanging the authorization code for tokens…");
    exchange_authorization_code(
        flow,
        client_id,
        &pkce.verifier,
        &code,
        &state,
        &redirect_uri,
    )
    .await
}

/// Device half: request a code, show it, poll until the user confirms.
#[cfg(feature = "real")]
async fn run_device(
    flow: &LoginFlow,
    client_id: &str,
    enterprise: Option<&str>,
    ui: &mut dyn LoginUi,
) -> Result<ExchangeOutcome, String> {
    let info = request_device_code(flow, client_id, enterprise).await?;
    ui.device_code(&DevicePrompt {
        user_code: info.user_code.clone(),
        verification_uri: info.verification_uri.clone(),
        expires_in_seconds: info.expires_in_seconds,
    });
    if let Some(complete) = info.verification_uri_complete.clone()
        && open_browser(&complete)
    {
        ui.notify("opened the verification page in your browser");
    }
    ui.notify(&format!(
        "waiting for you to approve the code — up to {}s{}",
        info.expires_in_seconds,
        if flow.mint == MintKind::GitHubCopilot {
            " (the GitHub token is minted into a Copilot token right after)"
        } else {
            ""
        }
    ));
    let outcome = poll_device_code(info.interval_seconds, info.expires_in_seconds, || async {
        poll_device_token(flow, client_id, &info.device_code, enterprise).await
    })
    .await?;
    // M55: GitHub hands back a GitHub token; the Copilot bearer (with its own
    // expiry) is minted from it before anything is stored — pi
    // `refreshGitHubCopilotAccessToken` on the login path.
    if flow.mint == MintKind::GitHubCopilot {
        let github_token = match outcome {
            ExchangeOutcome::Device(token) => token,
            other => return Ok(other),
        };
        ui.notify("minting the Copilot token from your GitHub token…");
        let refreshed = mint_copilot_bearer(&github_token, enterprise).await?;
        let mut env = std::collections::BTreeMap::new();
        if let Some(domain) = enterprise.map(str::trim).filter(|d| !d.is_empty()) {
            env.insert(
                crate::auth::ENTERPRISE_DOMAIN_ENV_KEY.to_string(),
                domain.to_string(),
            );
        }
        return Ok(ExchangeOutcome::Pair {
            access: refreshed.access,
            refresh: refreshed.refresh,
            expires: refreshed.expires,
            env: if env.is_empty() { None } else { Some(env) },
        });
    }
    Ok(outcome)
}

/// Mint the Copilot bearer from a GitHub token (pi
/// `refreshGitHubCopilotAccessToken`, network half): GET with the editor
/// headers, `{token, expires_at}` back.
#[cfg(feature = "real")]
async fn mint_copilot_bearer(
    github_token: &str,
    enterprise: Option<&str>,
) -> Result<crate::auth::OauthRefreshed, String> {
    use crate::providers::github_copilot::{
        COPILOT_API_VERSION, COPILOT_HEADERS, copilot_mint_url,
    };
    let url = copilot_mint_url(enterprise);
    let mut req = crate::http::http_client()
        .get(&url)
        .header("Accept", "application/json");
    for (name, value) in COPILOT_HEADERS {
        req = req.header(*name, *value);
    }
    let resp = req
        .header("X-GitHub-Api-Version", COPILOT_API_VERSION)
        .header("Authorization", format!("Bearer {github_token}"))
        .timeout(Duration::from_secs(
            crate::oauth::OAUTH_REFRESH_TIMEOUT_SECS,
        ))
        .send()
        .await
        .map_err(|e| format!("copilot token mint failed: {e}"))?;
    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| format!("copilot token mint read failed: {e}"))?;
    if !status.is_success() {
        return Err(format!(
            "copilot token mint failed (HTTP {status}): {}",
            text.chars().take(300).collect::<String>()
        ));
    }
    crate::oauth::parse_copilot_mint_response(&text, github_token)
}

/// Persist a completed exchange: token pair → `save_oauth`, key →
/// `save_credential`.
#[cfg(feature = "real")]
fn persist(
    flow: &LoginFlow,
    outcome: ExchangeOutcome,
    enterprise: Option<&str>,
) -> Result<LoginSaved, String> {
    match outcome {
        ExchangeOutcome::Pair {
            access,
            refresh,
            expires,
            env,
        } => {
            // Some providers (openrouter aside) only refresh through our
            // table when the pair is stored; a provider without a refresh row
            // still stores the pair so a same-day session works. Provider
            // extras (Copilot's enterprise domain) ride along, because the
            // wire and every later refresh need them.
            let env = env.or_else(|| {
                enterprise.map(|d| {
                    std::collections::BTreeMap::from([(
                        crate::auth::ENTERPRISE_DOMAIN_ENV_KEY.to_string(),
                        d.trim().to_string(),
                    )])
                })
            });
            let path = match crate::auth::global_dir() {
                Some(dir) => crate::auth::save_oauth_full_in(
                    &dir,
                    flow.provider,
                    &refresh,
                    &access,
                    expires,
                    env,
                )?,
                None => crate::auth::save_oauth(flow.provider, &refresh, &access, expires)?,
            };
            Ok(LoginSaved::Oauth {
                provider: flow.provider.to_string(),
                path,
                expires,
            })
        }
        ExchangeOutcome::Key(key) => {
            let path = crate::auth::save_credential(flow.provider, &key)?;
            Ok(LoginSaved::Key {
                provider: flow.provider.to_string(),
                path,
            })
        }
        ExchangeOutcome::Device(_) => Err(format!(
            "login for '{}' finished with an unminted device token — refusing to store it",
            flow.provider
        )),
    }
}

/// POST the authorization code (pi per-provider body encoding + 30s timeout).
#[cfg(feature = "real")]
async fn exchange_authorization_code(
    flow: &LoginFlow,
    client_id: &str,
    verifier: &str,
    code: &str,
    state: &str,
    redirect_uri: &str,
) -> Result<ExchangeOutcome, String> {
    let (content_type, body) = match flow.style {
        ExchangeStyle::Json => {
            let mut map = serde_json::Map::new();
            map.insert("code".into(), code.into());
            map.insert("code_verifier".into(), verifier.into());
            map.insert("code_challenge_method".into(), "S256".into());
            if !flow.returns_key {
                map.insert("grant_type".into(), "authorization_code".into());
                map.insert("client_id".into(), client_id.into());
                map.insert("redirect_uri".into(), redirect_uri.into());
                if flow.use_state {
                    map.insert("state".into(), state.into());
                }
            }
            (
                "application/json",
                serde_json::Value::Object(map).to_string(),
            )
        }
        ExchangeStyle::Form => (
            "application/x-www-form-urlencoded",
            device_poll_body(flow, client_id, code),
        ),
    };
    let resp = crate::http::http_client()
        .post(flow.token_url)
        .header("Content-Type", content_type)
        .header("Accept", "application/json")
        .body(body)
        .timeout(Duration::from_secs(
            crate::oauth::OAUTH_REFRESH_TIMEOUT_SECS,
        ))
        .send()
        .await
        .map_err(|e| format!("token exchange request failed for '{}': {e}", flow.provider))?;
    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| format!("token exchange read failed for '{}': {e}", flow.provider))?;
    if !status.is_success() {
        return Err(format!(
            "token exchange failed for '{}' (HTTP {status}): {}",
            flow.provider,
            text.chars().take(300).collect::<String>()
        ));
    }
    parse_exchange_response(
        flow,
        &text,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0),
    )
    .map_err(|e| format!("token exchange for '{}': {e}", flow.provider))
}

/// Request a device code (pi `xai.ts::requestDeviceCode`, `github-copilot.ts`
/// `startDeviceFlow`). `enterprise` swaps the endpoints onto the customer's
/// host for GitHub Business/Enterprise accounts (pi `getUrls(domain)`).
#[cfg(feature = "real")]
async fn request_device_code(
    flow: &LoginFlow,
    client_id: &str,
    enterprise: Option<&str>,
) -> Result<DeviceCodeInfo, String> {
    let url = device_endpoint(flow, enterprise, DeviceEndpoint::Code);
    let mut req = crate::http::http_client()
        .post(&url)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .header("Accept", "application/json");
    if !flow.user_agent.is_empty() {
        req = req.header("User-Agent", flow.user_agent);
    }
    let resp = req
        .body(device_code_request_body(flow, client_id))
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| {
            format!(
                "device authorization request failed for '{}': {e}",
                flow.provider
            )
        })?;
    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| format!("device authorization read failed: {e}"))?;
    if !status.is_success() {
        return Err(format!(
            "device authorization failed for '{}' (HTTP {status}): {}",
            flow.provider,
            text.chars().take(300).collect::<String>()
        ));
    }
    parse_device_code(&text)
        .map_err(|e| format!("device authorization for '{}': {e}", flow.provider))
}

/// Which of a device flow's two endpoints to resolve.
#[cfg(feature = "real")]
#[derive(Debug, Clone, Copy)]
enum DeviceEndpoint {
    /// Device-authorization request (`/login/device/code`).
    Code,
    /// Access-token poll (`/login/oauth/access_token`).
    Token,
}

/// A device flow's endpoint for this account: the flow's static URL, rebased on
/// the enterprise host when one was configured (pi `getUrls(domain)` — empty
/// for a host means `github.com`).
#[cfg(feature = "real")]
fn device_endpoint(flow: &LoginFlow, enterprise: Option<&str>, which: DeviceEndpoint) -> String {
    let Some(domain) = enterprise.map(str::trim).filter(|d| !d.is_empty()) else {
        return match which {
            DeviceEndpoint::Code => flow.device_url.to_string(),
            DeviceEndpoint::Token => flow.token_url.to_string(),
        };
    };
    let (code, token) = crate::providers::github_copilot::github_login_urls(Some(domain));
    match which {
        DeviceEndpoint::Code => code,
        DeviceEndpoint::Token => token,
    }
}

/// One device poll (pi `xai.ts` poll body ↔ `pollOAuthDeviceCodeFlow`).
#[cfg(feature = "real")]
async fn poll_device_token(
    flow: &LoginFlow,
    client_id: &str,
    device_code: &str,
    enterprise: Option<&str>,
) -> DevicePoll<ExchangeOutcome> {
    let url = device_endpoint(flow, enterprise, DeviceEndpoint::Token);
    let mut req = crate::http::http_client()
        .post(&url)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .header("Accept", "application/json");
    if !flow.user_agent.is_empty() {
        req = req.header("User-Agent", flow.user_agent);
    }
    let resp = req
        .body(device_poll_body(flow, client_id, device_code))
        .timeout(Duration::from_secs(30))
        .send()
        .await;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    match resp {
        Err(e) => DevicePoll::Failed(format!("device token poll failed: {e}")),
        Ok(resp) => {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            if status.is_success() {
                parse_device_poll_for(flow, &text, now)
            } else {
                // Error bodies carry `error` for pending/slow_down even on a
                // non-200 (RFC 8628 uses 400 for both).
                match parse_device_poll_for(flow, &text, now) {
                    DevicePoll::Failed(_) if text.trim().is_empty() => {
                        DevicePoll::Failed(format!("device token poll failed with HTTP {status}"))
                    }
                    other => other,
                }
            }
        }
    }
}
