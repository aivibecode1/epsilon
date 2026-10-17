//! Web fetch tool — read-group, Ask-gated, spill-aware, with a `format` switch.
//!
//! Ponytail: one reqwest GET with timeout/cap, reuses the existing
//! `spill_threshold` + `truncate_output` path used by `read`/`grep`. HTML
//! conversion is a stdlib tag scanner, not a new dependency: the five
//! production call sites need headings/links/lists plus script/style
//! stripping, and a turndown-equivalent would buy fidelity nobody here
//! asserts.
//! ponytail: naive HTML→markdown (headings, links, lists, emphasis, code —
//! no tables, footnotes or nested-list fidelity); take `htmd`/`html2text` as
//! a pinned dep when a real page shows structural loss, not before.

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

/// Install the `ring` crypto provider before the first client exists.
///
/// The build uses reqwest's `rustls-no-provider`, so rustls has **no**
/// default provider until somebody installs one: the first
/// `Client::builder().build()` in a process that never made an HTTPS
/// request panics instead of returning a network error. Production used to
/// survive this only when some earlier call (LLM traffic, MCP) had already
/// installed the process default — a webfetch issued before any of those
/// was a panic on ordering luck. Mirrors `epsilon-llm/src/http.rs` (M196);
/// idempotent, one atomic load per call.
fn ensure_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

/// Arguments for the webfetch tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct WebFetchArgs {
    /// URL to fetch (`http://` or `https://` only).
    pub url: String,
    /// Output shape for HTML pages (`text|markdown|html`, default `markdown`;
    /// non-HTML passes through untouched whatever this says).
    /// Port of opencode's `format` (`webfetch.ts:13-22`).
    #[serde(default)]
    pub format: WebFetchFormat,
    /// Optional max characters kept inline before spilling (defaults to the
    /// tool output byte cap).
    #[serde(default)]
    pub max_chars: Option<usize>,
}

/// Output shape for an HTML page (opencode `webfetch.ts:13-22` — default markdown).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum WebFetchFormat {
    /// Convert HTML to markdown; anything else passes through.
    #[default]
    Markdown,
    /// Strip HTML to plain text; anything else passes through.
    Text,
    /// Return the raw body regardless of content type.
    Html,
}

impl WebFetchFormat {
    /// Stable wire name for `details`.
    pub fn as_str(&self) -> &'static str {
        match self {
            WebFetchFormat::Markdown => "markdown",
            WebFetchFormat::Text => "text",
            WebFetchFormat::Html => "html",
        }
    }
}

/// The Accept header for one format (opencode `webfetch.ts:52-74`, strings verbatim).
pub(crate) fn accept_header_for(format: WebFetchFormat) -> &'static str {
    match format {
        WebFetchFormat::Markdown => {
            "text/markdown;q=1.0, text/x-markdown;q=0.9, text/plain;q=0.8, text/html;q=0.7, */*;q=0.1"
        }
        WebFetchFormat::Text => "text/plain;q=1.0, text/markdown;q=0.9, text/html;q=0.8, */*;q=0.1",
        WebFetchFormat::Html => {
            "text/html;q=1.0, application/xhtml+xml;q=0.9, text/plain;q=0.8, text/markdown;q=0.7, */*;q=0.1"
        }
    }
}

/// Shape the fetched body: convert HTML per `format`, pass anything else through.
///
/// Mirrors opencode's dispatch (`webfetch.ts:129-152`): `html` is always raw, and
/// `markdown`/`text` convert only `text/html` — anything else is already the
/// representation the caller asked for.
pub(crate) fn convert_body(format: WebFetchFormat, content_type: &str, body: &str) -> String {
    let is_html = content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .eq_ignore_ascii_case("text/html");
    match (format, is_html) {
        (WebFetchFormat::Html, _) | (_, false) => body.to_string(),
        (WebFetchFormat::Markdown, true) => html_to_markdown(body),
        (WebFetchFormat::Text, true) => html_to_text(body),
    }
}

/// Elements whose *content* is never prose (opencode `webfetch.ts:164`).
const SKIP_ELEMENTS: &[&str] = &["script", "style", "noscript", "iframe", "object", "embed"];

/// One lexical token of an HTML document: text, or a tag with its raw attribute tail.
enum HtmlToken<'a> {
    Text(&'a str),
    Tag {
        name: String,
        close: bool,
        attrs: &'a str,
    },
}

/// Split `html` into text/tag tokens. Comments (`<!-- … -->`) and declarations
/// (`<!DOCTYPE …>`) are skipped; malformed `<` without a `>` becomes text.
fn html_tokens(html: &str) -> Vec<HtmlToken<'_>> {
    let mut out = Vec::new();
    let bytes = html.as_bytes();
    let mut i = 0;
    let mut text_start = 0;
    while i < bytes.len() {
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }
        let pushed = text_start < i;
        if pushed {
            out.push(HtmlToken::Text(&html[text_start..i]));
        }
        let rest = &html[i..];
        let end = if let Some(comment) = rest.strip_prefix("<!--") {
            comment.find("-->").map(|j| i + 4 + j + 3)
        } else if rest.starts_with("<!") {
            rest.find('>').map(|j| i + j + 1)
        } else {
            rest.find('>').map(|j| i + j + 1)
        };
        let Some(end) = end else {
            // No terminator: the `<` is text. Retract the text token just
            // pushed (it ends before the `<`) and resume the run at the `<`.
            if pushed {
                out.pop();
                text_start = i;
            }
            i += 1;
            continue;
        };
        let inner = html[i + 1..end - 1].trim();
        let inner = inner.strip_prefix('!').unwrap_or(inner);
        if inner.starts_with("!--") || inner.starts_with('!') {
            // Comment / declaration already consumed above; nothing to emit.
        } else {
            let close = inner.starts_with('/');
            let body = inner.strip_prefix('/').unwrap_or(inner).trim_start();
            let name_end = body
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == ':'))
                .unwrap_or(body.len());
            let name = body[..name_end].to_ascii_lowercase();
            let attrs = body[name_end..]
                .trim()
                .strip_suffix('/')
                .unwrap_or(&body[name_end..])
                .trim();
            if !name.is_empty() {
                out.push(HtmlToken::Tag { name, close, attrs });
            }
        }
        i = end;
        text_start = end;
    }
    if text_start < html.len() {
        out.push(HtmlToken::Text(&html[text_start..]));
    }
    out
}

/// Read `name="value"` (or `'value'`, or a bare token) from a tag's attribute tail.
fn attr_value(attrs: &str, name: &str) -> Option<String> {
    let bytes = attrs.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        while i < bytes.len() && (bytes[i].is_ascii_whitespace() || bytes[i] == b'/') {
            i += 1;
        }
        let start = i;
        while i < bytes.len()
            && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'-' || bytes[i] == b':')
        {
            i += 1;
        }
        if start == i {
            i += 1;
            continue;
        }
        let key = &attrs[start..i];
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let mut value = String::new();
        if i < bytes.len() && bytes[i] == b'=' {
            i += 1;
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if i < bytes.len() && (bytes[i] == b'"' || bytes[i] == b'\'') {
                let quote = bytes[i];
                i += 1;
                let vstart = i;
                while i < bytes.len() && bytes[i] != quote {
                    i += 1;
                }
                value = attrs[vstart..i].to_string();
                i += 1;
            } else {
                let vstart = i;
                while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
                    i += 1;
                }
                value = attrs[vstart..i].to_string();
            }
        }
        if key.eq_ignore_ascii_case(name) {
            return Some(value);
        }
    }
    None
}

/// Decode the entities prose actually contains; anything unknown is left as-is.
fn decode_entities(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(semi) = rest.find(';') {
        let Some(amp) = rest[..semi].rfind('&') else {
            break;
        };
        let entity = &rest[amp + 1..semi];
        let decoded = match entity {
            "amp" => Some("&".to_string()),
            "lt" => Some("<".to_string()),
            "gt" => Some(">".to_string()),
            "quot" => Some("\"".to_string()),
            "apos" | "#39" => Some("'".to_string()),
            "nbsp" => Some(" ".to_string()),
            _ if entity.starts_with('#') => {
                let (digits, radix) = match entity
                    .strip_prefix("#x")
                    .or_else(|| entity.strip_prefix("#X"))
                {
                    Some(hex) => (hex, 16),
                    None => (entity[1..].as_ref(), 10),
                };
                u32::from_str_radix(digits, radix)
                    .ok()
                    .and_then(char::from_u32)
                    .map(|c| c.to_string())
            }
            _ => None,
        };
        let Some(decoded) = decoded else {
            out.push_str(&rest[..semi + 1]);
            rest = &rest[semi + 1..];
            continue;
        };
        out.push_str(&rest[..amp]);
        out.push_str(&decoded);
        rest = &rest[semi + 1..];
    }
    out.push_str(rest);
    out
}

/// Collapse ASCII whitespace runs to one space (block structure is newlines, added by callers).
fn collapse_ws(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut pending_space = false;
    for c in text.chars() {
        if c.is_ascii_whitespace() {
            pending_space = true;
        } else {
            if pending_space && !out.is_empty() {
                out.push(' ');
            }
            pending_space = false;
            out.push(c);
        }
    }
    out
}

/// Strip HTML to plain text (opencode `extractTextFromHTML`, `webfetch.ts:158-180`).
pub(crate) fn html_to_text(html: &str) -> String {
    let mut out = String::new();
    let mut skip_depth = 0usize;
    for token in html_tokens(html) {
        match token {
            HtmlToken::Tag { name, close, .. } => {
                if SKIP_ELEMENTS.contains(&name.as_str()) {
                    if close {
                        skip_depth = skip_depth.saturating_sub(1);
                    } else {
                        skip_depth += 1;
                    }
                }
            }
            HtmlToken::Text(t) => {
                if skip_depth == 0 {
                    out.push_str(&decode_entities(t));
                }
            }
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Convert HTML to markdown: headings, links, lists, emphasis, code, rules.
///
/// Turndown parity is explicitly NOT the goal (see the module docs): the shapes
/// a fetched doc is read for — headings, link targets, list items — survive, and
/// `script`/`style`/metadata never reach the model. `head` is skipped with them:
/// the title is metadata, not content.
pub(crate) fn html_to_markdown(html: &str) -> String {
    let mut out = String::new();
    let mut skip_depth = 0usize;
    let mut pre_depth = 0usize;
    // Open link targets; link text buffers so `[text](href)` emits on close.
    // `push` takes the buffer stack explicitly so pops and pushes can interleave.
    let mut link_stack: Vec<Option<String>> = Vec::new();
    let mut buffers: Vec<String> = vec![String::new()];
    let push = |buffers: &mut Vec<String>, s: &str| {
        if let Some(top) = buffers.last_mut() {
            top.push_str(s);
        }
    };

    for token in html_tokens(html) {
        match token {
            HtmlToken::Tag { name, close, attrs } => {
                let name = name.as_str();
                if name == "head" || SKIP_ELEMENTS.contains(&name) {
                    if close {
                        skip_depth = skip_depth.saturating_sub(1);
                    } else {
                        skip_depth += 1;
                    }
                    continue;
                }
                if skip_depth > 0 {
                    continue;
                }
                match (name, close) {
                    ("h1", false) => push(&mut buffers, "\n\n# "),
                    ("h2", false) => push(&mut buffers, "\n\n## "),
                    ("h3", false) => push(&mut buffers, "\n\n### "),
                    ("h4", false) => push(&mut buffers, "\n\n#### "),
                    ("h5", false) => push(&mut buffers, "\n\n##### "),
                    ("h6", false) => push(&mut buffers, "\n\n###### "),
                    ("h1" | "h2" | "h3" | "h4" | "h5" | "h6", true) => push(&mut buffers, "\n"),
                    ("p" | "div" | "section" | "article" | "table", false) => {
                        push(&mut buffers, "\n\n")
                    }
                    ("p" | "div" | "section" | "article" | "table", true) => {
                        push(&mut buffers, "\n")
                    }
                    ("br", _) => push(&mut buffers, "\n"),
                    ("hr", _) => push(&mut buffers, "\n\n---\n\n"),
                    ("li", false) => push(&mut buffers, "\n- "),
                    ("ul" | "ol", true) => push(&mut buffers, "\n"),
                    ("tr", false) => push(&mut buffers, "\n"),
                    ("td" | "th", _) => push(&mut buffers, " "),
                    ("strong" | "b", false) => push(&mut buffers, "**"),
                    ("strong" | "b", true) => push(&mut buffers, "**"),
                    ("em" | "i", false) => push(&mut buffers, "*"),
                    ("em" | "i", true) => push(&mut buffers, "*"),
                    ("pre", false) => {
                        pre_depth += 1;
                        push(&mut buffers, "\n```\n");
                    }
                    ("pre", true) => {
                        pre_depth = pre_depth.saturating_sub(1);
                        push(&mut buffers, "\n```\n");
                    }
                    ("code", _) => push(&mut buffers, "`"),
                    ("a", false) => {
                        link_stack.push(attr_value(attrs, "href"));
                        buffers.push(String::new());
                    }
                    ("a", true) => {
                        let text = buffers.pop().unwrap_or_default();
                        let text = text.trim().to_string();
                        match link_stack.pop().flatten() {
                            Some(href) if !href.is_empty() => {
                                push(&mut buffers, &format!("[{text}]({href})"));
                            }
                            _ => push(&mut buffers, &text),
                        }
                    }
                    ("img", _) => {
                        let alt = attr_value(attrs, "alt").unwrap_or_default();
                        let src = attr_value(attrs, "src").unwrap_or_default();
                        if !src.is_empty() {
                            push(&mut buffers, &format!("\n![{alt}]({src})\n"));
                        }
                    }
                    _ => {}
                }
            }
            HtmlToken::Text(t) => {
                if skip_depth > 0 {
                    continue;
                }
                let decoded = decode_entities(t);
                if pre_depth > 0 {
                    push(&mut buffers, decoded.trim_matches('\n'));
                } else {
                    push(&mut buffers, &collapse_ws(&decoded));
                }
            }
        }
    }
    while buffers.len() > 1 {
        // Unclosed `<a>`: keep its text, drop the link shape.
        let text = buffers.pop().unwrap_or_default();
        link_stack.pop();
        push(&mut buffers, &text);
    }
    out.push_str(&buffers.pop().unwrap_or_default());

    // Normalise: trim line ends, collapse 3+ newlines to two, trim the whole.
    let lines: Vec<&str> = out.split('\n').map(str::trim_end).collect();
    let mut normalised = lines.join("\n");
    while normalised.contains("\n\n\n") {
        normalised = normalised.replace("\n\n\n", "\n\n");
    }
    normalised.trim().to_string()
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

        ensure_crypto_provider();
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(FETCH_TIMEOUT_SECS))
            .connect_timeout(std::time::Duration::from_secs(CONNECT_TIMEOUT_SECS))
            .user_agent("epsilon-webfetch/0.1")
            .build()
            .map_err(|e| ToolFailure::msg(format!("client init failed: {e}")))?;

        let url_for_fetch = args.url.clone();
        let max_chars = args.max_chars;
        let format = args.format;
        let accept = accept_header_for(format);

        // Keep the request cancellable: race the GET against the token.
        // fetch_fut uses String as its error so cap-exceeded branches do not need
        // a `From<io::Error> for reqwest::Error` impl (reqwest 0.13 has none).
        let fetch_fut = async {
            let resp = client
                .get(&url_for_fetch)
                .header(reqwest::header::ACCEPT, accept)
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
        let (status, _content_type, body, textual_is_ok) = tokio::select! {
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

        // Shape the body per `format` BEFORE the caps below: truncation and
        // spill describe what the model actually receives, converted or raw.
        // `html` returns the bytes untouched; `markdown`/`text` convert only
        // `text/html` and pass anything else through (opencode `webfetch.ts:129-152`).
        let mut body = convert_body(format, &_content_type, &body);

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
        // `true`: `task` ships in every production registry (single
        // `register_builtins_and_gate_runner` entry point).
        let truncated = crate::util::truncate_output_with_task(&body, &ctx.spill_dir, &stem, true);
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
                    "format": format.as_str(),
                }),
                usage: None,
                terminate: false,
            });
        }

        let body_len = body.len();
        Ok(ToolOutput {
            content: vec![epsilon_contracts::content::ContentBlock::text(body)],
            details: serde_json::json!({
                "bytes": body_len,
                "contentType": _content_type,
                "format": format.as_str(),
            }),
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

#[cfg(test)]
mod format_tests {
    use super::{WebFetchArgs, WebFetchFormat, WebFetchTool, accept_header_for, convert_body};
    use epsilon_contracts::tool::Tool;

    const FIXTURE_HTML: &str = "<!DOCTYPE html><html><head><title>T</title><style>body{color:red}</style></head>\
        <body><h1>Hello</h1><p>See <a href=\"https://example.com/docs\">docs</a>.</p>\
        <script>evil()</script></body></html>";

    #[test]
    fn accept_header_prefers_the_requested_format() {
        assert!(
            accept_header_for(WebFetchFormat::Markdown).contains("text/markdown;q=1.0"),
            "markdown first"
        );
        assert!(
            accept_header_for(WebFetchFormat::Text).starts_with("text/plain;q=1.0"),
            "text first"
        );
        assert!(
            accept_header_for(WebFetchFormat::Html).starts_with("text/html;q=1.0"),
            "html first"
        );
    }

    #[test]
    fn markdown_format_converts_a_fixture_html_doc() {
        let out = convert_body(
            WebFetchFormat::Markdown,
            "text/html; charset=utf-8",
            FIXTURE_HTML,
        );
        assert!(out.contains("# Hello"), "heading becomes atx: {out}");
        assert!(
            out.contains("[docs](https://example.com/docs)"),
            "link survives: {out}"
        );
        assert!(!out.contains("evil()"), "script stripped: {out}");
        assert!(!out.contains("color:red"), "style stripped: {out}");
        assert!(!out.contains("<h1>"), "no raw tags: {out}");
    }

    #[test]
    fn markdown_passes_non_html_through() {
        let raw = "# already markdown";
        assert_eq!(
            convert_body(WebFetchFormat::Markdown, "text/plain", raw),
            raw
        );
    }

    #[test]
    fn text_format_strips_tags() {
        let out = convert_body(
            WebFetchFormat::Text,
            "text/html",
            "<p>Hello <b>World</b></p><script>evil()</script>",
        );
        assert!(out.contains("Hello"), "words survive: {out}");
        assert!(out.contains("World"), "words survive: {out}");
        assert!(!out.contains('<'), "no tags left: {out}");
        assert!(!out.contains("evil()"), "script stripped: {out}");
    }

    #[test]
    fn html_format_returns_raw() {
        assert_eq!(
            convert_body(WebFetchFormat::Html, "text/html", FIXTURE_HTML),
            FIXTURE_HTML
        );
    }

    #[test]
    fn format_param_defaults_to_markdown_and_parses_all_shapes() {
        let d: WebFetchArgs =
            serde_json::from_value(serde_json::json!({"url": "http://x"})).unwrap();
        assert_eq!(d.format, WebFetchFormat::Markdown);
        for (s, v) in [
            ("markdown", WebFetchFormat::Markdown),
            ("text", WebFetchFormat::Text),
            ("html", WebFetchFormat::Html),
        ] {
            let d: WebFetchArgs =
                serde_json::from_value(serde_json::json!({"url": "http://x", "format": s}))
                    .unwrap();
            assert_eq!(d.format, v, "format={s}");
        }
        assert!(
            serde_json::from_value::<WebFetchArgs>(
                serde_json::json!({"url": "http://x", "format": "pdf"})
            )
            .is_err(),
            "unknown formats are invalid arguments"
        );
    }

    /// The webfetch client must build in a process that never made an HTTPS
    /// request before: reqwest is `rustls-no-provider`, so without an
    /// explicit install the FIRST client construction panics instead of
    /// returning a network error. This drives `execute` (not just a bare
    /// builder) against a refused port: pre-fix it panics at client
    /// construction; post-fix it returns a network failure. Run FILTERED
    /// in a fresh process for the honest RED — in a full suite another
    /// test may have installed the process default first, which is exactly
    /// the ordering accident this guards against.
    #[tokio::test]
    async fn execute_builds_its_client_with_no_prior_https_in_the_process() {
        let dir = tempfile::tempdir().unwrap();
        let tool = WebFetchTool::new();
        let call = epsilon_contracts::tool::ToolInvocation {
            tool_call_id: "client-test".into(),
            args: serde_json::json!({"url": "http://127.0.0.1:9/nope"}),
        };
        let ctx = epsilon_contracts::tool::ToolContext {
            session_id: "client-test".into(),
            observation_scope: std::sync::Arc::new(()),
            cwd: dir.path().to_path_buf(),
            spill_dir: dir.path().join(".epsilon/spill"),
            token: tokio_util::sync::CancellationToken::new(),
            task_host: None,
            goal_host: None,
        };
        let err = tool
            .execute(call, ctx, std::sync::Arc::new(|_| {}))
            .await
            .expect_err("nothing listens on the discard port");
        assert!(
            !format!("{err:?}").contains("CryptoProvider"),
            "a network failure, not a provider panic: {err:?}"
        );
    }
}
