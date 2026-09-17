//! GitHub Copilot subscription wire (pi `githubCopilotProvider`,
//! `githubCopilotBaseUrl`).
//!
//! Copilot is a proxy, not a provider: the same model families pi serves from
//! their native endpoints are re-served behind one host, and the credential is
//! a Copilot bearer (minted from a GitHub token — see
//! `crate::oauth_login::github_copilot`) rather than an `sk-…` API key. Two
//! things therefore cannot ride the stock wires unchanged:
//!
//! 1. **Token-derived endpoint.** `https://api.individual.githubcopilot.com`
//!    is only the default. A credential minted for a business/enterprise
//!    account carries `proxy-ep=<host>` inside the token string, and the API
//!    host is that host with the leading `proxy.` label rewritten to `api.`
//!    (pi `getBaseUrlFromToken`); an explicit enterprise domain falls back to
//!    `https://copilot-api.<domain>`. `endpoint()` cannot see the credential,
//!    so the wire implements [`ProviderWire::endpoint_for`], which the
//!    transport prefers for every real request.
//! 2. **Bearer auth on both dialects.** Claude rows are served over Anthropic
//!    Messages — with `Authorization: Bearer`, never `x-api-key` — and
//!    everything else over the OpenAI-compatible surface. The inner wire is
//!    therefore chosen per row by the catalog (`github-copilot/claude-*` →
//!    Anthropic Messages, the rest → completions), and this wrapper only
//!    replaces URL + auth headers.
//!
//! Path shapes follow the SDKs pi hands the request to: the Anthropic SDK
//! appends `/v1/messages`, the OpenAI SDK `/chat/completions`, both relative
//! to the Copilot base URL (pi passes `baseURL: model.baseUrl`).
//!
//! pi sends its editor-identity headers (`COPILOT_HEADERS`) on the *token
//! mint* and the model-listing requests, not on chat calls — the chat path
//! carries only the bearer, so this wire does the same. If a live account ever
//! rejects that, the header set is already defined here to add.

use super::{HttpEventStream, ProviderWire};
use epsilon_contracts::message::StopReason;
use epsilon_contracts::stream::StreamEvent;

/// Copilot's default API host (pi `getGitHubCopilotBaseUrl` fallback).
pub const COPILOT_DEFAULT_BASE_URL: &str = "https://api.individual.githubcopilot.com";

/// Editor-identity headers pi (and the Copilot proxy) expect on the token mint
/// and `/models` calls, not on chat requests.
pub const COPILOT_HEADERS: &[(&str, &str)] = &[
    ("User-Agent", "GitHubCopilotChat/0.35.0"),
    ("Editor-Version", "vscode/1.107.0"),
    ("Editor-Plugin-Version", "copilot-chat/0.35.0"),
    ("Copilot-Integration-Id", "vscode-chat"),
];

/// `X-GitHub-Api-Version` for Copilot endpoints (pi `COPILOT_API_VERSION`).
pub const COPILOT_API_VERSION: &str = "2026-06-01";

/// Extract the API base URL from a Copilot token.
///
/// The token is a semicolon-delimited payload (`tid=…;exp=…;proxy-ep=…;…`), not
/// a JWT. `proxy-ep` names the proxy host the credential was minted for; the
/// API host is the same host with `proxy.` → `api.` (pi `getBaseUrlFromToken`).
/// `None` when the token carries no `proxy-ep` (plain individual accounts
/// sometimes omit it) or the value is empty.
pub fn base_url_from_token(token: &str) -> Option<String> {
    let rest = token.split("proxy-ep=").nth(1)?;
    let host = rest.split(';').next()?.trim();
    if host.is_empty() {
        return None;
    }
    let host = host
        .strip_prefix("proxy.")
        .map_or_else(|| host.to_string(), |tail| format!("api.{tail}"));
    Some(format!("https://{host}"))
}

/// The base URL a request from this credential should go to: the token's
/// `proxy-ep` when it carries one, else the enterprise host, else the
/// individual default (pi `getGitHubCopilotBaseUrl` precedence).
pub fn copilot_base_url(token: &str, enterprise_domain: Option<&str>) -> String {
    if let Some(from_token) = base_url_from_token(token) {
        return from_token;
    }
    match enterprise_domain.map(str::trim).filter(|d| !d.is_empty()) {
        Some(domain) => format!("https://copilot-api.{domain}"),
        None => COPILOT_DEFAULT_BASE_URL.to_string(),
    }
}

/// Copilot token-mint URL for an account (pi `getUrls`: `api.${domain}` with
/// `domain = enterpriseDomain || "github.com"`). The login ceremony mints
/// from here, and so does every refresh — the minted bearer is what expires.
pub fn copilot_mint_url(enterprise_domain: Option<&str>) -> String {
    let domain = enterprise_domain
        .map(str::trim)
        .filter(|d| !d.is_empty())
        .unwrap_or("github.com");
    format!("https://api.{domain}/copilot_internal/v2/token")
}

/// GitHub device-flow endpoints for an account (pi `getUrls`): device code and
/// access token live on the login domain, which is the enterprise host when one
/// was configured and `github.com` otherwise.
pub fn github_login_urls(enterprise_domain: Option<&str>) -> (String, String) {
    let domain = enterprise_domain
        .map(str::trim)
        .filter(|d| !d.is_empty())
        .unwrap_or("github.com");
    (
        format!("https://{domain}/login/device/code"),
        format!("https://{domain}/login/oauth/access_token"),
    )
}

/// GitHub Copilot wire: an inner family wire (Anthropic Messages or
/// OpenAI-compatible) with Copilot's URL resolution and bearer auth layered on.
pub struct CopilotWire {
    /// Base URL from the catalog row (the individual default; the credential
    /// may upgrade it per request — see [`copilot_base_url`]).
    base_url: String,
    /// Auth env var (`COPILOT_GITHUB_TOKEN`), owned like the compat wire's.
    env_key: Option<String>,
    /// Enterprise host a Business/Enterprise login stored (pi
    /// `credential.enterpriseUrl`); `None` for individual accounts.
    enterprise_domain: Option<String>,
    /// Inner wire carrying the family's request shape + SSE mapping.
    inner: Box<dyn ProviderWire>,
}

impl CopilotWire {
    /// Wrap `inner` for the Copilot host. `anthropic_family` selects the
    /// `/v1/messages` (Anthropic Messages) rather than `/chat/completions`
    /// (OpenAI-compatible) path.
    pub fn new(
        base_url: &str,
        env_key: Option<&str>,
        enterprise_domain: Option<&str>,
        anthropic_family: bool,
        inner: Box<dyn ProviderWire>,
    ) -> Self {
        let _ = anthropic_family;
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            env_key: env_key.map(str::to_string),
            enterprise_domain: enterprise_domain.map(str::to_string),
            inner,
        }
    }

    /// Request path on the Copilot host for the wrapped family.
    fn path(&self, model: &str) -> String {
        // The inner wire owns the family decision (it is the one that can
        // build the body/parse the stream); ask it for its own endpoint and
        // keep only the path, then re-base it on the Copilot host.
        let inner_url = self.inner.endpoint(model);
        let path = inner_url
            .split_once("://")
            .and_then(|(_, rest)| rest.find('/').map(|i| &rest[i..]))
            .unwrap_or("/v1/messages");
        path.to_string()
    }
}

impl ProviderWire for CopilotWire {
    fn endpoint(&self, model: &str) -> String {
        format!("{}{}", self.base_url, self.path(model))
    }

    /// Copilot's host depends on the credential, so the transport asks for the
    /// endpoint *with* the resolved key.
    fn endpoint_for(&self, model: &str, key: &str) -> String {
        let base = if key.trim().is_empty() {
            self.base_url.clone()
        } else {
            copilot_base_url(key, self.enterprise_domain.as_deref())
        };
        format!("{}{}", base, self.path(model))
    }

    fn env_key(&self) -> &str {
        self.env_key.as_deref().unwrap_or("")
    }

    /// Bearer auth for both dialects: the Copilot proxy authenticates the
    /// bearer and routes to the family backend itself, so no `x-api-key` and
    /// no Claude Code identity headers (pi's copilot rows never use the
    /// `sk-ant-oat` branch).
    fn headers(&self, key: &str) -> Vec<(String, String)> {
        let mut out = Vec::new();
        if self.env_key.is_some() || !key.is_empty() {
            out.push(("Authorization".to_string(), format!("Bearer {key}")));
        }
        // Anthropic rows still speak the Messages protocol, which requires the
        // version header even behind the proxy.
        if self.path("<model>").starts_with("/v1/messages") {
            out.push(("anthropic-version".to_string(), "2023-06-01".to_string()));
        }
        out
    }

    fn request_body(
        &self,
        model: &str,
        ctx: &crate::stream_fn::LlmContext,
    ) -> Result<serde_json::Value, String> {
        self.inner.request_body(model, ctx)
    }

    fn on_event(
        &self,
        ev: &crate::http::SseEvent,
        st: &mut HttpEventStream,
    ) -> Option<Vec<StreamEvent>> {
        self.inner.on_event(ev, st)
    }

    fn map_stop(&self, wire: Option<&str>, has_tool_calls: bool) -> (StopReason, Option<String>) {
        self.inner.map_stop(wire, has_tool_calls)
    }

    fn finalize_usage_from_tail(&self) -> bool {
        self.inner.finalize_usage_from_tail()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::{AnthropicWire, OpenAiCompatWire};

    fn compat() -> CopilotWire {
        let inner = Box::new(OpenAiCompatWire {
            base_url: COPILOT_DEFAULT_BASE_URL.to_string(),
            env_key: Some("COPILOT_GITHUB_TOKEN".to_string()),
        });
        CopilotWire::new(
            COPILOT_DEFAULT_BASE_URL,
            Some("COPILOT_GITHUB_TOKEN"),
            None,
            false,
            inner,
        )
    }

    fn anthropic() -> CopilotWire {
        let inner = Box::new(AnthropicWire);
        CopilotWire::new(
            COPILOT_DEFAULT_BASE_URL,
            Some("COPILOT_GITHUB_TOKEN"),
            None,
            true,
            inner,
        )
    }

    #[test]
    fn token_proxy_ep_becomes_the_api_host() {
        let token =
            "tid=abc;exp=1790000000;proxy-ep=proxy.individual.githubcopilot.com;sku=copilot";
        assert_eq!(
            base_url_from_token(token).as_deref(),
            Some("https://api.individual.githubcopilot.com")
        );
        // A host without the `proxy.` label is used as-is (pi's replace is a no-op).
        assert_eq!(
            base_url_from_token("proxy-ep=api.example.com;sku=x").as_deref(),
            Some("https://api.example.com")
        );
        // No `proxy-ep` at all: nothing to derive.
        assert_eq!(base_url_from_token("tid=abc;exp=1"), None);
        assert_eq!(base_url_from_token("proxy-ep=;sku=x"), None);
    }

    #[test]
    fn base_url_precedence_is_token_then_enterprise_then_default() {
        let token = "sku=individual;proxy-ep=proxy.business.githubcopilot.com;exp=1";
        assert_eq!(
            copilot_base_url(token, Some("ghe.example.com")),
            "https://api.business.githubcopilot.com"
        );
        // Enterprise domain is the fallback when the token carries no proxy-ep.
        assert_eq!(
            copilot_base_url("tid=x;exp=1", Some(" ghe.example.com ")),
            "https://copilot-api.ghe.example.com"
        );
        assert_eq!(
            copilot_base_url("tid=x;exp=1", None),
            COPILOT_DEFAULT_BASE_URL
        );
        assert_eq!(
            copilot_base_url("tid=x;exp=1", Some("   ")),
            COPILOT_DEFAULT_BASE_URL
        );
    }

    #[test]
    fn login_and_mint_urls_follow_the_account_domain() {
        assert_eq!(
            copilot_mint_url(None),
            "https://api.github.com/copilot_internal/v2/token"
        );
        assert_eq!(
            copilot_mint_url(Some("ghe.example.com")),
            "https://api.ghe.example.com/copilot_internal/v2/token"
        );
        assert_eq!(
            copilot_mint_url(Some("  ")),
            "https://api.github.com/copilot_internal/v2/token"
        );
        assert_eq!(
            github_login_urls(None),
            (
                "https://github.com/login/device/code".to_string(),
                "https://github.com/login/oauth/access_token".to_string()
            )
        );
        assert_eq!(
            github_login_urls(Some("ghe.example.com")),
            (
                "https://ghe.example.com/login/device/code".to_string(),
                "https://ghe.example.com/login/oauth/access_token".to_string()
            )
        );
    }

    #[test]
    fn paths_follow_the_family() {
        assert_eq!(
            compat().endpoint("gpt-5.4"),
            "https://api.individual.githubcopilot.com/chat/completions"
        );
        assert_eq!(
            anthropic().endpoint("claude-opus-5"),
            "https://api.individual.githubcopilot.com/v1/messages"
        );
    }

    #[test]
    fn endpoint_for_prefers_the_credential_host() {
        let token = "proxy-ep=proxy.business.githubcopilot.com;exp=1";
        assert_eq!(
            anthropic().endpoint_for("claude-opus-5", token),
            "https://api.business.githubcopilot.com/v1/messages"
        );
        // A plain (or empty) key keeps the catalog default.
        assert_eq!(
            compat().endpoint_for("gpt-5.4", "plain-token"),
            "https://api.individual.githubcopilot.com/chat/completions"
        );
        assert_eq!(
            compat().endpoint_for("gpt-5.4", "   "),
            "https://api.individual.githubcopilot.com/chat/completions"
        );
    }

    #[test]
    fn headers_are_bearer_only_plus_anthropic_version_for_claude() {
        let compat_headers = compat().headers("cop-token");
        assert_eq!(
            compat_headers,
            vec![("Authorization".to_string(), "Bearer cop-token".to_string())]
        );
        let anthropic_headers = anthropic().headers("cop-token");
        assert!(
            anthropic_headers
                .contains(&("anthropic-version".to_string(), "2023-06-01".to_string()))
        );
        assert!(
            anthropic_headers
                .contains(&("Authorization".to_string(), "Bearer cop-token".to_string()))
        );
        // No shared provider key leaks into the copilot wire as `x-api-key`.
        assert!(!anthropic_headers.iter().any(|(n, _)| n == "x-api-key"));
    }

    #[test]
    fn request_shape_and_stop_mapping_are_delegated() {
        let ctx = crate::stream_fn::LlmContext {
            system_prompt: "sys".to_string(),
            messages: vec![epsilon_contracts::message::Message::User(
                epsilon_contracts::message::UserMessage {
                    id: epsilon_contracts::new_id("msg"),
                    timestamp_ms: 0,
                    content: vec![epsilon_contracts::content::UserContent::Text {
                        text: "hi".to_string(),
                    }],
                },
            )],
            tools: Vec::new(),
            thinking_level: epsilon_contracts::ThinkingLevel::Off,
        };
        let body = anthropic().request_body("claude-opus-5", &ctx).unwrap();
        // Anthropic shape: `messages` + a required `max_tokens` (the wire
        // derives it from the context/system prompt).
        assert!(body.get("messages").is_some(), "{body}");
        assert!(
            body.get("max_tokens")
                .and_then(|v| v.as_u64())
                .is_some_and(|n| n > 0),
            "the anthropic wire must send max_tokens: {body}"
        );
        let compat_body = compat().request_body("gpt-5.4", &ctx).unwrap();
        assert!(compat_body.get("messages").is_some(), "{compat_body}");
        assert_eq!(
            compat().map_stop(Some("stop"), false).0,
            StopReason::EndTurn
        );
        assert_eq!(
            anthropic().map_stop(Some("end_turn"), false).0,
            StopReason::EndTurn
        );
    }
}
