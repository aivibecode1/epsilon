//! Web fetch tool — read-group, Ask-gated, spill-aware.
//!
//! Ponytail: one reqwest GET with timeout/cap, reuses the existing
//! `spill_threshold` + `truncate_output` path used by `read`/`grep` — no
//! HTML→markdown pipeline, no new dep, no private-range blocklist at v1
//! (permission Ask gate is the control; doc notes the limitation).

use async_trait::async_trait;
use epsilon_contracts::tool::{
    ExecutionMode, Tool, ToolContext, ToolDescriptor, ToolFailure, ToolInvocation, ToolOutput,
    ToolUpdateSink,
};
use schemars::JsonSchema;
use serde::Deserialize;

pub const WEBFETCH_DESC: &str = include_str!("../descriptions/webfetch.txt");

/// Hard bounds for the one GET (the product): 15 s overall, 5 s connect, 5 MiB body.
/// Chosen to mirror opencode's `MAX_RESPONSE_BYTES` and `DEFAULT_TIMEOUT_SECONDS`.
const FETCH_TIMEOUT_SECS: u64 = 15;
const CONNECT_TIMEOUT_SECS: u64 = 5;
const MAX_FETCH_BYTES: usize = 5 * 1024 * 1024;

/// Arguments for the webfetch tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct WebFetchArgs {
    /// URL to fetch (`http://` or `https://` only).
    pub url: String,
    /// Optional max characters kept inline before spilling (defaults to the
    /// tool output byte cap).
    #[serde(default)]
    pub max_chars: Option<usize>,
}

/// Read-group HTTP fetcher (no HTML→markdown at v1 — raw text is correct).
pub struct WebFetchTool {
    descriptor: ToolDescriptor,
}

impl WebFetchTool {
    /// Build the tool.
    pub fn new() -> Self {
        Self {
            descriptor: ToolDescriptor {
                name: "webfetch".into(),
                label: "Fetch URL".into(),
                description: WEBFETCH_DESC.into(),
                parameters_schema: serde_json::to_value(schemars::schema_for!(WebFetchArgs))
                    .unwrap_or(serde_json::json!({"type":"object"})),
                execution_mode: ExecutionMode::Parallel,
            },
        }
    }
}

impl Default for WebFetchTool {
    fn default() -> Self {
        Self::new()
    }
}

/// Validate the URL string: parseable and `http`/`https` only.
pub(crate) fn validate_url(url: &str) -> Result<reqwest::Url, String> {
    let parsed = reqwest::Url::parse(url).map_err(|e| format!("invalid URL: {e}"))?;
    match parsed.scheme() {
        "http" | "https" => Ok(parsed),
        other => Err(format!("only http/https is allowed, got '{other}:'")),
    }
}

/// Map a failed HTTP status into a short error string (snippet of body).
fn status_error(status: u16, snippet: &str) -> String {
    let head: String = snippet.chars().take(2000).collect();
    format!("HTTP {status}: {head}")
}

#[async_trait]
impl Tool for WebFetchTool {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    async fn execute(
        &self,
        call: ToolInvocation,
        ctx: ToolContext,
        on_update: ToolUpdateSink,
    ) -> Result<ToolOutput, ToolFailure> {
        let args: WebFetchArgs = serde_json::from_value(call.args)
            .map_err(|e| ToolFailure::msg(format!("invalid arguments for 'webfetch': {e}")))?;

        let _url = validate_url(&args.url).map_err(ToolFailure::msg)?;

        if ctx.token.is_cancelled() {
            return Err(ToolFailure::msg("webfetch cancelled"));
        }

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(FETCH_TIMEOUT_SECS))
            .connect_timeout(std::time::Duration::from_secs(CONNECT_TIMEOUT_SECS))
            .user_agent("epsilon-webfetch/0.1")
            .build()
            .map_err(|e| ToolFailure::msg(format!("client init failed: {e}")))?;

        let url_for_fetch = args.url.clone();
        let max_chars = args.max_chars;

        // Keep the request cancellable: race the GET against the token.
        // fetch_fut uses String as its error so cap-exceeded branches do not need
        // a `From<io::Error> for reqwest::Error` impl (reqwest 0.13 has none).
        let fetch_fut = async {
            let resp = client
                .get(&url_for_fetch)
                .send()
                .await
                .map_err(|e| e.to_string())?;
            let status = resp.status();
            let content_type = resp
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_string();
            let mime = content_type
                .split(';')
                .next()
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase();

            // Unknown or clearly binary mimes become a short note, not the bytes.
            let textual = mime.is_empty()
                || mime.starts_with("text/")
                || mime == "application/json"
                || mime.ends_with("+json")
                || mime == "application/xml"
                || mime.ends_with("+xml")
                || mime == "application/javascript"
                || mime == "application/x-javascript";
            let is_image_like = mime.starts_with("image/") && mime != "image/svg+xml";

            if is_image_like {
                return Ok::<_, String>((
                    status.as_u16(),
                    content_type,
                    format!("[binary content: {mime} — not shown]"),
                    textual,
                ));
            }
            if !textual {
                let snippet = format!("[non-text content-type {content_type} — body not shown]");
                return Ok((status.as_u16(), content_type, snippet, textual));
            }

            let headers_for_cap = resp.headers().clone();
            // Honour content-length early when present.
            if let Some(len) = headers_for_cap
                .get(reqwest::header::CONTENT_LENGTH)
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse::<usize>().ok())
                && len > MAX_FETCH_BYTES
            {
                return Err(format!(
                    "response too large (declared {len} bytes > {MAX_FETCH_BYTES} cap)"
                ));
            }

            let bytes = resp.bytes().await.map_err(|e| e.to_string())?;
            if bytes.len() > MAX_FETCH_BYTES {
                return Err(format!(
                    "response too large ({} bytes > {MAX_FETCH_BYTES} cap)",
                    bytes.len()
                ));
            }
            let text = String::from_utf8_lossy(&bytes).into_owned();
            Ok((status.as_u16(), content_type, text, textual))
        };

        // Race against cancellation without capturing `ctx` across the select guard.
        let cancelled = ctx.token.clone();
        let (status, _content_type, mut body, textual_is_ok) = tokio::select! {
            _ = cancelled.cancelled() => return Err(ToolFailure::msg("webfetch cancelled")),
            res = fetch_fut => res.map_err(|e: String| ToolFailure::msg(format!("fetch failed: {e}")))?,
        };

        // Emit one throttled partial for large bodies so the transcript shows progress.
        if body.len() > 50 * 1024 {
            let trunc_preview: String = body.chars().take(4000).collect();
            on_update(epsilon_contracts::tool::ToolPartial {
                content: vec![epsilon_contracts::content::ContentBlock::text(format!(
                    "fetching {url} … {len} bytes",
                    url = args.url,
                    len = body.len()
                ))],
                details: serde_json::json!({
                    "preview": trunc_preview,
                    "bytes": body.len(),
                    "status": status,
                }),
            });
        }

        if !(200..300).contains(&status) {
            return Ok(ToolOutput {
                content: vec![epsilon_contracts::content::ContentBlock::text(
                    status_error(status, &body),
                )],
                details: serde_json::json!({"status": status}),
                usage: None,
                terminate: false,
            });
        }

        if !textual_is_ok {
            // Already handled: image / non-text produced a short note above; keep that path
            // even on 2xx so we never spill binary bytes.
        }

        // Optional caller-supplied truncation before the global spill cap.
        if let Some(limit) = max_chars
            && body.chars().count() > limit
        {
            let kept: String = body.chars().take(limit).collect();
            let truncated = body.chars().count() - limit;
            body = format!("{kept}\n... [{truncated} more chars truncated by max_chars={limit}]");
        }

        // Reuse the global spill helper: tail-capped view for the model, full body on disk.
        let stem = format!(
            "webfetch-{}",
            args.url
                .chars()
                .filter(|c| c.is_alphanumeric())
                .take(24)
                .collect::<String>()
        );
        let truncated = crate::util::truncate_output(&body, &ctx.spill_dir, &stem);
        if truncated.truncated
            && let Some(spilled) = &truncated.spilled_to
        {
            return Ok(ToolOutput {
                content: vec![epsilon_contracts::content::ContentBlock::text(
                    truncated.text,
                )],
                details: serde_json::json!({
                    "spilled": true,
                    "path": spilled.display().to_string(),
                    "bytes": body.len(),
                    "contentType": _content_type,
                }),
                usage: None,
                terminate: false,
            });
        }

        let body_len = body.len();
        Ok(ToolOutput {
            content: vec![epsilon_contracts::content::ContentBlock::text(body)],
            details: serde_json::json!({"bytes": body_len, "contentType": _content_type}),
            usage: None,
            terminate: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_http_scheme() {
        assert!(validate_url("file:///etc/passwd").is_err());
        assert!(validate_url("javascript:alert(1)").is_err());
        assert!(validate_url("ftp://example.com/a").is_err());
    }

    #[test]
    fn accepts_http_and_https() {
        assert!(validate_url("http://example.com").is_ok());
        assert!(validate_url("https://example.com/a?b=1").is_ok());
    }

    #[test]
    fn status_error_formats_snippet() {
        let s = status_error(404, "not found: the page you requested");
        assert!(s.contains("404"));
        assert!(s.contains("not found"));
    }

    #[test]
    fn max_chars_truncation_shape() {
        // Exercise the inline truncation branch via the same string math the tool uses.
        let body = "x".repeat(5000);
        let limit = 100usize;
        let kept: String = body.chars().take(limit).collect();
        let truncated = body.chars().count() - limit;
        let out = format!("{kept}\n... [{truncated} more chars truncated by max_chars={limit}]");
        assert!(out.contains(&"x".repeat(100)));
        assert!(out.contains("more chars truncated"));
    }

    #[test]
    fn spill_helper_reused() {
        // Directly exercise the global spill path to keep this test offline.
        let dir = tempfile::tempdir().unwrap();
        let big = (0..3000)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let t = crate::util::truncate_output(&big, dir.path(), "demo");
        assert!(t.truncated);
        assert!(t.text.contains("lines / "), "omission is reported");
        assert!(t.text.contains("line 0"), "the head survives");
        assert!(t.text.contains("line 2999"), "the tail survives");
        assert!(t.spilled_to.is_some());
    }
}
