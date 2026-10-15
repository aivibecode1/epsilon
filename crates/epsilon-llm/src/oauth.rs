//! OAuth refresh-token exchanges, one per subscription provider (pi port).
//!
//! Ported from pi-0.84.2 `packages/ai/src/auth/oauth/*.ts` (`refresh*`
//! functions only — the login ceremonies stay out; see M46). Every provider
//! speaks the same shape: `POST {token_url}` with
//! `{grant_type: "refresh_token", client_id, refresh_token}` →
//! `{access_token, refresh_token, expires_in}` with pi's 5-minute skew
//! (`expires = now + expires_in*1000 - 300_000`). pi hardcodes each client id
//! per file; Epsilon reads it from the environment (`*_OAUTH_CLIENT_ID`) and
//! NEVER embeds another project's credential — a missing id is a telling
//! error, not a silent fallback.
//!
//! Transport split: [`parse_refresh_response`] is pure and always compiled
//! (unit-tested offline); [`exchange_refresh_token`] needs `reqwest` and
//! lives behind the `real` feature like every other network call.

use crate::auth::OauthRefreshed;

/// pi's refresh window: tokens expiring within 5 minutes refresh before use
/// (`DEFAULT_OAUTH_MINIMUM_VALIDITY_MS`).
pub const OAUTH_MIN_VALIDITY_MS: u64 = 5 * 60 * 1000;

/// pi's expiry skew applied at issue (`expires_in*1000 - 5*60*1000`).
pub const OAUTH_EXPIRY_SKEW_MS: u64 = 5 * 60 * 1000;

/// pi's refresh HTTP timeout (30s `postJson` abort in `anthropic.ts`).
pub const OAUTH_REFRESH_TIMEOUT_SECS: u64 = 30;

/// pi's three refresh shapes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefreshStyle {
    /// POST a JSON body (`anthropic.ts::postJson`).
    JsonBody,
    /// POST a form-encoded body (`openai-codex.ts`, `xai.ts`).
    FormBody,
    /// GET with bearer auth and Copilot's editor-identity headers; the response
    /// carries `token` + `expires_at` in seconds (pi
    /// `refreshGitHubCopilotAccessToken`). The "refresh token" is the long-lived
    /// GitHub token, which is *re-spent*, never rotated — so the stored pair's
    /// `refresh` survives every refresh.
    CopilotMint,
}

/// One provider's refresh endpoint + client-id env var (pi's per-file
/// `TOKEN_URL` + `CLIENT_ID`, minus the embedded secret).
#[derive(Debug, Clone, Copy)]
pub struct RefreshEndpoint {
    /// Provider id as stored in `auth.json` (`anthropic`, `xai`, ...).
    pub provider: &'static str,
    /// Token endpoint URL (pi's `TOKEN_URL` verbatim).
    pub token_url: &'static str,
    /// Env var carrying the OAuth client id (pi hardcodes it; we don't).
    pub client_id_env: &'static str,
    /// Request/response shape (see [`RefreshStyle`]).
    pub style: RefreshStyle,
}

/// pi's refresh-capable subscription providers (openrouter's `refresh` is the
/// identity — an API key, not a pair — so it gets no row here).
pub const REFRESH_ENDPOINTS: &[RefreshEndpoint] = &[
    RefreshEndpoint {
        provider: "anthropic",
        token_url: "https://platform.claude.com/v1/oauth/token",
        client_id_env: "ANTHROPIC_OAUTH_CLIENT_ID",
        style: RefreshStyle::JsonBody,
    },
    RefreshEndpoint {
        provider: "openai-codex",
        token_url: "https://auth.openai.com/oauth/token",
        client_id_env: "OPENAI_CODEX_OAUTH_CLIENT_ID",
        style: RefreshStyle::FormBody,
    },
    RefreshEndpoint {
        provider: "xai",
        token_url: "https://auth.x.ai/oauth2/token",
        client_id_env: "XAI_OAUTH_CLIENT_ID",
        style: RefreshStyle::FormBody,
    },
    // M55: Copilot's "refresh" re-mints the short-lived proxy bearer from the
    // long-lived GitHub token (no client id involved, no token rotation).
    // `token_url` is the individual default; `exchange_refresh_token` swaps in
    // `api.<enterprise-domain>` when the credential carries one.
    RefreshEndpoint {
        provider: "github-copilot",
        token_url: "https://api.github.com/copilot_internal/v2/token",
        client_id_env: "",
        style: RefreshStyle::CopilotMint,
    },
];

/// Look up the refresh endpoint for a provider id.
pub fn refresh_endpoint(provider: &str) -> Option<RefreshEndpoint> {
    REFRESH_ENDPOINTS
        .iter()
        .find(|e| e.provider == provider)
        .copied()
}

/// The OAuth client id for `endpoint`: its env var's value, or `""` when the
/// endpoint names none.
///
/// **The empty case is why this is a function rather than a bare
/// `std::env::var`.** Copilot's mint re-spends the long-lived GitHub token and
/// needs no client id, so its row carries `client_id_env: ""` — and
/// `std::env::var("")` is *always* an error. Reading it anyway is what made
/// `refresh_oauth_before_use` return early every time for the one provider
/// whose refresh is a plain re-mint (`oauth.rs:83-88` is the row; the old call
/// site read the var before branching on style).
///
/// A named var that is not set stays a telling error — never an empty id.
pub fn client_id_for(endpoint: &RefreshEndpoint) -> Result<String, String> {
    if endpoint.client_id_env.is_empty() {
        return Ok(String::new());
    }
    std::env::var(endpoint.client_id_env).map_err(|_| {
        format!(
            "OAuth refresh for '{}' needs {} (pi embeds its own client id; Epsilon never will)",
            endpoint.provider, endpoint.client_id_env
        )
    })
}

/// Parse a token endpoint's JSON body into a rotated pair (pi's
/// `readTokenResponse` / `refreshAnthropicToken` response half).
/// `now_millis` is injected so tests pin time. Applies pi's 5-min skew.
pub fn parse_refresh_response(body: &str, now_millis: u64) -> Result<OauthRefreshed, String> {
    let v: serde_json::Value =
        serde_json::from_str(body).map_err(|e| format!("refresh returned invalid JSON: {e}"))?;
    let access = v
        .get("access_token")
        .and_then(|x| x.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "refresh response missing access_token".to_string())?;
    // pi rotates the refresh token when the server returns one (anthropic,
    // codex); xai keeps the old one when omitted (`body.refresh_token ??
    // previousRefreshToken` in `xai.ts:130-134`) — the caller passes the
    // stored one back via `fallback_refresh`.
    let refresh = v
        .get("refresh_token")
        .and_then(|x| x.as_str())
        .filter(|s| !s.is_empty());
    let expires_in = v
        .get("expires_in")
        .and_then(|x| x.as_u64())
        .ok_or_else(|| "refresh response missing expires_in".to_string())?;
    Ok(OauthRefreshed {
        access: access.to_string(),
        // Empty means "server didn't rotate" (xai parity) — the exchange
        // caller backfills the stored token before persisting.
        refresh: refresh.unwrap_or("").to_string(),
        expires: now_millis
            .saturating_add(expires_in.saturating_mul(1000))
            .saturating_sub(OAUTH_EXPIRY_SKEW_MS),
    })
}

/// Parse a Copilot mint response into the rotated pair (pi
/// `refreshGitHubCopilotAccessToken` response half): `{token, expires_at}` in
/// seconds, with pi's 5-minute skew applied. The refresh token is the GitHub
/// token that minted it — passed through unchanged, because GitHub's token is
/// not rotated by this exchange.
pub fn parse_copilot_mint_response(
    body: &str,
    refresh_token: &str,
) -> Result<OauthRefreshed, String> {
    let v: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| format!("copilot token response was not JSON: {e}"))?;
    let token = v
        .get("token")
        .and_then(|x| x.as_str())
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| "copilot token response carries no \"token\"".to_string())?;
    let expires_at = v
        .get("expires_at")
        .and_then(|x| x.as_u64())
        .ok_or_else(|| "copilot token response carries no \"expires_at\"".to_string())?;
    Ok(OauthRefreshed {
        access: token.to_string(),
        refresh: refresh_token.to_string(),
        expires: expires_at
            .saturating_mul(1000)
            .saturating_sub(OAUTH_EXPIRY_SKEW_MS),
    })
}

/// Build the refresh request body (pi's per-provider encoding): JSON object
/// for Anthropic (`postJson`), `application/x-www-form-urlencoded` otherwise.
/// `None` for [`RefreshStyle::CopilotMint`], which sends no body at all.
pub fn refresh_request_body(
    endpoint: &RefreshEndpoint,
    client_id: &str,
    refresh_token: &str,
) -> Option<(String, String)> {
    if endpoint.style == RefreshStyle::CopilotMint {
        return None;
    }
    if endpoint.style == RefreshStyle::JsonBody {
        Some((
            "application/json".to_string(),
            serde_json::json!({
                "grant_type": "refresh_token",
                "client_id": client_id,
                "refresh_token": refresh_token,
            })
            .to_string(),
        ))
    } else {
        Some((
            "application/x-www-form-urlencoded".to_string(),
            format!(
                "grant_type=refresh_token&client_id={}&refresh_token={}",
                url_encode(client_id),
                url_encode(refresh_token)
            ),
        ))
    }
}

/// Minimal percent-encoding for form bodies (no new deps: alphanumerics +
/// `-_.~` pass through, everything else becomes `%XX`).
fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Exchange a refresh token for a rotated pair (pi `refreshAnthropicToken`
/// and siblings, network half). Runs OFF the auth lock — the caller persists
/// via [`crate::auth::store_refreshed_oauth_in`]. `client_id` comes from the
/// caller (env); a missing/unknown provider is a telling error, never a
/// silent skip. xai parity: when the server omits `refresh_token`, the stored
/// one is kept.
#[cfg(feature = "real")]
pub async fn exchange_refresh_token(
    provider: &str,
    refresh_token: &str,
    client_id: &str,
) -> Result<OauthRefreshed, String> {
    let endpoint = refresh_endpoint(provider)
        .ok_or_else(|| format!("no OAuth refresh endpoint for '{provider}'"))?;
    // M55: the Copilot mint is a different animal — a GET with the editor
    // headers, no client id and no body, whose response is `{token,
    // expires_at}` rather than a rotated OAuth pair.
    if endpoint.style == RefreshStyle::CopilotMint {
        let url = crate::providers::github_copilot::copilot_mint_url(
            crate::auth::enterprise_domain_of(provider).as_deref(),
        );
        let mut req = crate::http::http_client()
            .get(&url)
            .header("Accept", "application/json");
        for (name, value) in crate::providers::github_copilot::COPILOT_HEADERS {
            req = req.header(*name, *value);
        }
        req = req
            .header(
                "X-GitHub-Api-Version",
                crate::providers::github_copilot::COPILOT_API_VERSION,
            )
            .header("Authorization", format!("Bearer {refresh_token}"))
            .timeout(std::time::Duration::from_secs(OAUTH_REFRESH_TIMEOUT_SECS));
        let resp = req
            .send()
            .await
            .map_err(|e| format!("copilot token mint failed for '{provider}': {e}"))?;
        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| format!("copilot token mint read failed for '{provider}': {e}"))?;
        if !status.is_success() {
            let head: String = text.chars().take(200).collect();
            return Err(format!(
                "copilot token mint failed for '{provider}' (HTTP {status}): {head}"
            ));
        }
        return parse_copilot_mint_response(&text, refresh_token)
            .map_err(|e| format!("copilot token mint for '{provider}': {e}"));
    }
    if client_id.trim().is_empty() {
        return Err(format!(
            "OAuth refresh for '{provider}' needs {} (pi embeds its own client id; Epsilon never will)",
            endpoint.client_id_env
        ));
    }
    let (content_type, body) = refresh_request_body(&endpoint, client_id, refresh_token)
        .expect("pair styles always build a body");
    let resp = crate::http::http_client()
        .post(endpoint.token_url)
        .header("Content-Type", content_type)
        .header("Accept", "application/json")
        .body(body)
        .timeout(std::time::Duration::from_secs(OAUTH_REFRESH_TIMEOUT_SECS))
        .send()
        .await
        .map_err(|e| format!("OAuth refresh request failed for '{provider}': {e}"))?;
    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| format!("OAuth refresh read failed for '{provider}': {e}"))?;
    if !status.is_success() {
        let head: String = text.chars().take(200).collect();
        return Err(format!(
            "OAuth refresh failed for '{provider}' (HTTP {status}): {head}"
        ));
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let mut refreshed = parse_refresh_response(&text, now)
        .map_err(|e| format!("OAuth refresh for '{provider}': {e}"))?;
    if refreshed.refresh.is_empty() {
        refreshed.refresh = refresh_token.to_string();
    }
    if refreshed.access.trim().is_empty() {
        return Err(format!(
            "OAuth refresh for '{provider}' returned an empty access token"
        ));
    }
    Ok(refreshed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_applies_skew_and_requires_fields() {
        // pi: expires = now + expires_in*1000 - 300_000.
        let r = parse_refresh_response(
            r#"{"access_token":"at","refresh_token":"rt","expires_in":3600}"#,
            1_000_000,
        )
        .unwrap();
        assert_eq!(r.access, "at");
        assert_eq!(r.refresh, "rt");
        assert_eq!(r.expires, 1_000_000 + 3_600_000 - 300_000);
    }

    #[test]
    fn parse_missing_fields_fails_tellingly() {
        assert!(parse_refresh_response(r#"{"access_token":"at"}"#, 0).is_err());
        assert!(parse_refresh_response("not json", 0).is_err());
        assert!(
            parse_refresh_response(
                r#"{"access_token":"","refresh_token":"r","expires_in":1}"#,
                0
            )
            .is_err()
        );
    }

    #[test]
    fn parse_missing_rotation_leaves_marker_for_caller() {
        // xai omits refresh_token when not rotated — caller backfills.
        let r =
            parse_refresh_response(r#"{"access_token":"at","expires_in":60}"#, 1_000_000).unwrap();
        assert!(r.refresh.is_empty());
    }

    #[test]
    fn body_encoding_matches_provider_convention() {
        let anthropic = refresh_endpoint("anthropic").unwrap();
        let (ct, body) = refresh_request_body(&anthropic, "cid", "rt").unwrap();
        assert_eq!(ct, "application/json");
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["grant_type"], "refresh_token");

        let xai = refresh_endpoint("xai").unwrap();
        let (ct, body) = refresh_request_body(&xai, "cid", "rt").unwrap();
        assert_eq!(ct, "application/x-www-form-urlencoded");
        assert!(body.contains("grant_type=refresh_token"));

        assert!(refresh_endpoint("openrouter").is_none());
    }

    #[test]
    fn copilot_refresh_is_a_bodiless_mint_style() {
        // M55: the Copilot row exists, takes no client id and sends no body.
        let copilot = refresh_endpoint("github-copilot").unwrap();
        assert_eq!(copilot.style, RefreshStyle::CopilotMint);
        assert!(copilot.client_id_env.is_empty());
        assert!(refresh_request_body(&copilot, "", "gh-token").is_none());
        assert_eq!(
            copilot.token_url,
            "https://api.github.com/copilot_internal/v2/token"
        );
    }

    /// **Criterion 7: the provider whose refresh needs no client id can reach
    /// its exchange.** Copilot's row carries `client_id_env: ""`, and
    /// `std::env::var("")` is always an error — so reading the var before
    /// branching on style returned early for every copilot refresh, and the one
    /// provider whose refresh is a plain re-mint was unreachable. `Ok("")` can
    /// only come from the empty-env branch, so this test fails against the old
    /// code.
    #[test]
    fn an_endpoint_with_no_client_id_env_is_not_an_error() {
        let copilot = refresh_endpoint("github-copilot").expect("the copilot row");
        assert_eq!(
            client_id_for(&copilot).as_deref(),
            Ok(""),
            "a mint needs no client id, so the read must not be attempted at all"
        );

        // The other arm: a NAMED var that nothing sets is still a telling
        // error. A literal row rather than a real one, so the assertion cannot
        // depend on this machine's environment.
        let named = RefreshEndpoint {
            provider: "test-provider",
            token_url: "https://example.invalid/token",
            client_id_env: "EPSILON_UNSET_CLIENT_ID_FOR_TEST",
            style: RefreshStyle::FormBody,
        };
        let err = client_id_for(&named).expect_err("an unset var must refuse");
        assert!(
            err.contains("EPSILON_UNSET_CLIENT_ID_FOR_TEST"),
            "and it must name the var so the reader can set it: {err}"
        );
    }

    #[test]
    fn copilot_mint_parses_token_and_expiry_skew() {
        // pi: expires = expires_at*1000 - 5 min; the GitHub token passes through.
        let r = parse_copilot_mint_response(
            r#"{"token":"tid=1;proxy-ep=proxy.individual.githubcopilot.com","expires_at":1790000000}"#,
            "gh-token",
        )
        .unwrap();
        assert!(r.access.starts_with("tid=1;"));
        assert_eq!(r.refresh, "gh-token");
        assert_eq!(r.expires, 1_790_000_000_000 - 300_000);

        assert!(parse_copilot_mint_response(r#"{"expires_at":1}"#, "gh").is_err());
        assert!(parse_copilot_mint_response(r#"{"token":"t"}"#, "gh").is_err());
        assert!(parse_copilot_mint_response(r#"{"token":"  ","expires_at":1}"#, "gh").is_err());
        assert!(parse_copilot_mint_response("nope", "gh").is_err());
    }
}
