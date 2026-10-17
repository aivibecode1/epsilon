//! One language-server conversation: spawn, handshake, document sync, diagnostics.
//!
//! The client speaks the minimum of LSP that makes an edit's feedback true: `initialize` +
//! `initialized`, `textDocument/didOpen` and `didChange` (full sync — deltas buy nothing when the
//! caller has already written the file), and the `textDocument/publishDiagnostics` notifications
//! the server sends back. Everything else the server says is ignored, which is also why nothing
//! here can hang on a message it does not understand.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tokio::io::BufReader;
use tokio::process::{Child, Command};

use crate::framing;
use crate::report::{Diagnostic, Severity};
use crate::server::{ServerDef, file_uri};
use epsilon_contracts::lsp_queries::{LspLocation, LspSymbol};

/// An environment scrubber for a spawned server: the harness's rule, injected so the subprocess
/// policy lives in one place (`epsilon-safety`) instead of being re-derived here.
pub type ScrubEnv = std::sync::Arc<dyn Fn(&mut Command) + Send + Sync>;

/// How much of the server's life this client controls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientState {
    /// Spawned and handshaken; document notifications will be answered.
    Ready,
    /// A read or write failed, or the process died. The service marks it broken and stops asking.
    Broken,
}

/// Where a client came from and how it was started (its own record of the conversation).
#[derive(Debug, Clone)]
pub struct ClientConfig {
    /// The server definition this client runs.
    pub def: &'static ServerDef,
    /// The project root the server was started for.
    pub root: PathBuf,
    /// Absolute path to the executable (found on `PATH`, or supplied by the caller).
    pub command_path: PathBuf,
    /// How long to wait for the handshake.
    pub handshake_timeout: Duration,
    /// How long to wait for a diagnostic publish after a document change.
    pub publish_timeout: Duration,
    /// Environment for the child, applied after scrubbing. Per-child rather than process-global:
    /// a test (or a user) can configure one server without racing every other spawn.
    pub env: Vec<(String, String)>,
}

impl ClientConfig {
    /// Defaults for a server found on `PATH`: 10 s to handshake, 4 s to publish.
    pub fn new(def: &'static ServerDef, root: PathBuf, command_path: PathBuf) -> Self {
        Self {
            def,
            root,
            command_path,
            handshake_timeout: Duration::from_secs(10),
            publish_timeout: Duration::from_secs(4),
            env: Vec::new(),
        }
    }
}

/// A live language-server process.
pub struct Client {
    cfg: ClientConfig,
    child: Child,
    stdin: tokio::process::ChildStdin,
    stdout: BufReader<tokio::process::ChildStdout>,
    next_id: i64,
    state: ClientState,
    /// Latest published diagnostics per URI.
    published: HashMap<String, Vec<Diagnostic>>,
    /// Documents already opened, so a second report sends `didChange` instead of `didOpen`.
    opened: Vec<String>,
}

impl Client {
    /// Spawn the server with the built-in environment floor and handshake.
    pub async fn start(cfg: ClientConfig) -> Result<Self, String> {
        Self::start_with(cfg, None).await
    }

    /// Spawn the server, let `scrub_env` (when given) strip credentials, then handshake.
    ///
    /// The caller-supplied scrubber is the harness's real rule from `epsilon-safety`; the built-in
    /// floor below applies either way, because running a third-party binary with the harness's API
    /// keys in its environment is never acceptable.
    pub async fn start_with(
        cfg: ClientConfig,
        scrub_env: Option<ScrubEnv>,
    ) -> Result<Self, String> {
        let mut cmd = Command::new(&cfg.command_path);
        cmd.args(cfg.def.args)
            .current_dir(&cfg.root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        // A language server has no business reading the harness's API keys (DSH subprocess rule):
        // the caller's scrubber first (that is the house rule), then the local floor.
        if let Some(scrub) = &scrub_env {
            scrub(&mut cmd);
        }
        scrub_env_floor(&mut cmd);
        // Applied after the scrub: the caller's explicit environment is a decision, not a leak.
        for (key, value) in &cfg.env {
            cmd.env(key, value);
        }

        let mut child = cmd
            .spawn()
            .map_err(|e| format!("spawn {}: {e}", cfg.command_path.display()))?;
        let stdin = child.stdin.take().ok_or("server stdin unavailable")?;
        let stdout = child.stdout.take().ok_or("server stdout unavailable")?;
        let mut client = Self {
            cfg,
            child,
            stdin,
            stdout: BufReader::new(stdout),
            next_id: 1,
            state: ClientState::Ready,
            published: HashMap::new(),
            opened: Vec::new(),
        };
        client.handshake().await?;
        Ok(client)
    }

    /// Current state — the service uses this to decide whether to keep the client.
    pub fn state(&self) -> ClientState {
        self.state
    }

    /// The project root this client serves.
    pub fn root(&self) -> &Path {
        &self.cfg.root
    }

    /// The server definition this client runs.
    pub fn def(&self) -> &'static ServerDef {
        self.cfg.def
    }

    async fn send(&mut self, message: serde_json::Value) -> Result<(), String> {
        framing::write_message(&mut self.stdin, &message)
            .await
            .map_err(|e| {
                self.state = ClientState::Broken;
                format!("write to language server: {e}")
            })
    }

    async fn next_message(&mut self) -> Result<Option<serde_json::Value>, String> {
        framing::read_message(&mut self.stdout).await.map_err(|e| {
            self.state = ClientState::Broken;
            format!("read from language server: {e}")
        })
    }

    async fn handshake(&mut self) -> Result<(), String> {
        let root_uri = file_uri(&self.cfg.root);
        let id = self.next_id;
        self.next_id += 1;
        let params = serde_json::json!({
            "processId": std::process::id(),
            "clientInfo": {"name": "epsilon", "version": env!("CARGO_PKG_VERSION")},
            "rootUri": root_uri,
            "capabilities": {
                // Full sync: the caller has already written the file, so deltas would be busywork.
                "textDocument": {
                    "synchronization": {"dynamicRegistration": false},
                    "publishDiagnostics": {"relatedInformation": false}
                }
            },
            "workspaceFolders": [{"uri": root_uri, "name": "workspace"}],
        });
        self.send(serde_json::json!({
            "jsonrpc": "2.0", "id": id, "method": "initialize", "params": params
        }))
        .await?;

        // Wait for the response to *this* id; servers may send notices first.
        let wait = self.cfg.handshake_timeout;
        let saw_response = tokio::time::timeout(wait, async {
            loop {
                match self.next_message().await {
                    Ok(Some(msg)) => {
                        if msg.get("id").and_then(|v| v.as_i64()) == Some(id)
                            && msg.get("result").is_some()
                        {
                            return Ok::<bool, String>(true);
                        }
                        if msg.get("id").and_then(|v| v.as_i64()) == Some(id)
                            && msg.get("error").is_some()
                        {
                            self.state = ClientState::Broken;
                            return Err(format!("initialize refused: {}", msg["error"]));
                        }
                        // Not ours (a `window/logMessage`, a publish for a document we never sent):
                        // ignore and keep waiting.
                    }
                    Ok(None) => {
                        self.state = ClientState::Broken;
                        return Err("language server exited during handshake".into());
                    }
                    Err(e) => return Err(e),
                }
            }
        })
        .await
        .map_err(|_| {
            self.state = ClientState::Broken;
            format!(
                "{} did not answer initialize within {:?}",
                self.cfg.def.name, wait
            )
        })??;
        debug_assert!(saw_response);

        self.send(serde_json::json!({"jsonrpc": "2.0", "method": "initialized", "params": {}}))
            .await?;
        Ok(())
    }

    /// Tell the server the file's current text and wait for its verdict.
    pub async fn diagnose(&mut self, path: &Path, text: &str) -> Result<Vec<Diagnostic>, String> {
        if self.state == ClientState::Broken {
            return Err("client is broken".into());
        }
        let uri = self.ensure_open(path, text).await?;

        // Collect until a publish for this document arrives, or the wait elapses. A server that
        // never publishes is not an error: it means "nothing to say yet".
        let uri_wanted = uri.clone();
        let wait = self.cfg.publish_timeout;
        let collected = tokio::time::timeout(wait, async {
            loop {
                match self.next_message().await {
                    Ok(Some(msg)) => {
                        if msg.get("method").and_then(|m| m.as_str())
                            == Some("textDocument/publishDiagnostics")
                        {
                            let params = &msg["params"];
                            let published_uri =
                                params["uri"].as_str().unwrap_or_default().to_string();
                            let diags = parse_diagnostics(params);
                            self.published.insert(published_uri.clone(), diags);
                            if published_uri == uri_wanted {
                                return Ok::<Vec<Diagnostic>, String>(
                                    self.published[&uri_wanted].clone(),
                                );
                            }
                        }
                    }
                    Ok(None) => {
                        self.state = ClientState::Broken;
                        return Err("language server exited".into());
                    }
                    Err(e) => return Err(e),
                }
            }
        })
        .await;

        match collected {
            Ok(inner) => inner,
            // Timed out: the last known publish for this document (usually empty) is the answer.
            Err(_) => Ok(self.published.remove(&uri).unwrap_or_default()),
        }
    }

    /// Make sure the server has `path`'s current text, so a query answers about these bytes.
    ///
    /// The `didOpen`/`didChange` choice is the same one [`Client::diagnose`] makes (full sync —
    /// deltas buy nothing when the caller has already read the file); returning the URI saves
    /// every caller from recomputing it.
    async fn ensure_open(&mut self, path: &Path, text: &str) -> Result<String, String> {
        let uri = file_uri(path);
        let method = if self.opened.contains(&uri) {
            "textDocument/didChange"
        } else {
            self.opened.push(uri.clone());
            "textDocument/didOpen"
        };
        let language_id = self
            .cfg
            .def
            .extensions
            .first()
            .copied()
            .unwrap_or("plaintext")
            .to_string();

        let params = if method == "textDocument/didOpen" {
            serde_json::json!({
                "textDocument": {"uri": uri, "languageId": language_id, "version": 1, "text": text}
            })
        } else {
            serde_json::json!({
                "textDocument": {"uri": uri, "version": 2},
                "contentChanges": [{"text": text}]
            })
        };
        self.send(serde_json::json!({
            "jsonrpc": "2.0", "method": method, "params": params
        }))
        .await?;
        Ok(uri)
    }

    /// Send one request and wait for the response carrying its id.
    ///
    /// Notifications arriving meanwhile (a `publishDiagnostics` for a document synced earlier)
    /// are filed, not dropped: ignoring them would lose diagnostics, and hanging on them would
    /// deadlock every query against a chatty server. Anything else with another id is not ours,
    /// so the wait continues.
    async fn request(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        if self.state == ClientState::Broken {
            return Err("client is broken".into());
        }
        let id = self.next_id;
        self.next_id += 1;
        self.send(serde_json::json!({
            "jsonrpc": "2.0", "id": id, "method": method, "params": params
        }))
        .await?;
        loop {
            match self.next_message().await? {
                Some(msg) => {
                    if msg.get("id").and_then(|v| v.as_i64()) == Some(id) {
                        if msg.get("error").is_some() {
                            self.state = ClientState::Broken;
                            return Err(format!("{method} failed: {}", msg["error"]));
                        }
                        return Ok(msg
                            .get("result")
                            .cloned()
                            .unwrap_or(serde_json::Value::Null));
                    }
                    if msg.get("method").and_then(|m| m.as_str())
                        == Some("textDocument/publishDiagnostics")
                    {
                        let uri = msg["params"]["uri"]
                            .as_str()
                            .unwrap_or_default()
                            .to_string();
                        let diags = parse_diagnostics(&msg["params"]);
                        self.published.insert(uri, diags);
                    }
                }
                None => {
                    self.state = ClientState::Broken;
                    return Err("language server exited".into());
                }
            }
        }
    }

    /// Where the symbol at 0-based (`line`, `character`) of `path` is defined (1-based out).
    pub async fn go_to_definition(
        &mut self,
        path: &Path,
        text: &str,
        line: u32,
        character: u32,
    ) -> Result<Vec<LspLocation>, String> {
        let uri = self.ensure_open(path, text).await?;
        let result = self
            .request(
                "textDocument/definition",
                serde_json::json!({
                    "textDocument": {"uri": uri},
                    "position": {"line": line, "character": character}
                }),
            )
            .await?;
        parse_locations(&result)
    }

    /// Where the symbol at 0-based (`line`, `character`) of `path` is used (1-based out).
    pub async fn find_references(
        &mut self,
        path: &Path,
        text: &str,
        line: u32,
        character: u32,
        include_declaration: bool,
    ) -> Result<Vec<LspLocation>, String> {
        let uri = self.ensure_open(path, text).await?;
        let result = self
            .request(
                "textDocument/references",
                serde_json::json!({
                    "textDocument": {"uri": uri},
                    "position": {"line": line, "character": character},
                    "context": {"includeDeclaration": include_declaration}
                }),
            )
            .await?;
        parse_locations(&result)
    }

    /// Where the type at 0-based (`line`, `character`) of `path` is implemented (1-based out).
    pub async fn go_to_implementation(
        &mut self,
        path: &Path,
        text: &str,
        line: u32,
        character: u32,
    ) -> Result<Vec<LspLocation>, String> {
        let uri = self.ensure_open(path, text).await?;
        let result = self
            .request(
                "textDocument/implementation",
                serde_json::json!({
                    "textDocument": {"uri": uri},
                    "position": {"line": line, "character": character}
                }),
            )
            .await?;
        parse_locations(&result)
    }

    /// Every named symbol `path` defines (1-based out).
    pub async fn document_symbol(
        &mut self,
        path: &Path,
        text: &str,
    ) -> Result<Vec<LspSymbol>, String> {
        let uri = self.ensure_open(path, text).await?;
        let result = self
            .request(
                "textDocument/documentSymbol",
                serde_json::json!({"textDocument": {"uri": uri}}),
            )
            .await?;
        parse_symbols(&result, path)
    }

    /// Ask the process to exit, then stop waiting for it (`kill_on_drop` collects the rest).
    pub async fn shutdown(mut self) {
        let _ = self
            .send(serde_json::json!({"jsonrpc": "2.0", "id": self.next_id, "method": "shutdown"}))
            .await;
        let _ = self
            .send(serde_json::json!({"jsonrpc": "2.0", "method": "exit"}))
            .await;
        let _ = tokio::time::timeout(Duration::from_millis(500), self.child.wait()).await;
    }
}

/// Parse a `publishDiagnostics` params object into our reduced form.
fn parse_diagnostics(params: &serde_json::Value) -> Vec<Diagnostic> {
    let Some(list) = params["diagnostics"].as_array() else {
        return Vec::new();
    };
    list.iter()
        .map(|d| {
            let severity = d["severity"].as_i64().unwrap_or(3);
            let start = &d["range"]["start"];
            Diagnostic {
                line: start["line"].as_u64().unwrap_or(0) as u32 + 1, // LSP is 0-based
                column: start["character"].as_u64().unwrap_or(0) as u32 + 1,
                severity: Severity::from_lsp(severity),
                message: d["message"].as_str().unwrap_or_default().to_string(),
            }
        })
        .collect()
}

/// Parse a definition/references/implementation result: one `Location`, a list of them, or
/// `null` (a server answering "nothing" — zero hits, not an error).
///
/// A single malformed entry is skipped rather than failing the query: one bad item among
/// good ones is the server's bug, and the good ones are still an answer.
fn parse_locations(result: &serde_json::Value) -> Result<Vec<LspLocation>, String> {
    match result {
        serde_json::Value::Null => Ok(Vec::new()),
        serde_json::Value::Array(items) => {
            Ok(items.iter().filter_map(parse_one_location).collect())
        }
        serde_json::Value::Object(_) => Ok(parse_one_location(result).into_iter().collect()),
        _ => Err("language server sent an unexpected location result".into()),
    }
}

/// One `Location` object into 1-based form; `None` when it is missing its URI or range.
fn parse_one_location(item: &serde_json::Value) -> Option<LspLocation> {
    let path = uri_to_path(item.get("uri")?.as_str()?)?;
    let start = item.get("range")?.get("start")?;
    Some(LspLocation {
        path,
        line: start.get("line")?.as_u64()? as u32 + 1, // LSP is 0-based
        character: start.get("character")?.as_u64()? as u32 + 1,
    })
}

/// Parse a `documentSymbol` result into a flat symbol list.
///
/// Both wire shapes are accepted: `SymbolInformation` (flat, carrying its own `location`)
/// and `DocumentSymbol` (nested, carrying `selectionRange` plus `children` — flattened here,
/// parent first). `doc_path` is the file that was asked about, which is what a nested entry
/// belongs to, since it carries no URI of its own.
fn parse_symbols(result: &serde_json::Value, doc_path: &Path) -> Result<Vec<LspSymbol>, String> {
    match result {
        serde_json::Value::Null => Ok(Vec::new()),
        serde_json::Value::Array(items) => {
            let mut out = Vec::new();
            for item in items {
                flatten_symbol(item, doc_path, &mut out);
            }
            Ok(out)
        }
        _ => Err("language server sent an unexpected documentSymbol result".into()),
    }
}

/// One symbol node into the flat list, recursing into `children`.
fn flatten_symbol(item: &serde_json::Value, doc_path: &Path, out: &mut Vec<LspSymbol>) {
    let name = item
        .get("name")
        .and_then(|n| n.as_str())
        .unwrap_or_default()
        .to_string();
    if name.is_empty() {
        return;
    }
    let kind = symbol_kind_name(item.get("kind"));
    if let Some(location) = item.get("location") {
        // `SymbolInformation`: the location carries the URI.
        if let Some(hit) = parse_one_location(location) {
            out.push(LspSymbol {
                name,
                kind,
                location: hit,
            });
        }
    } else if let Some(selection) = item.get("selectionRange") {
        // `DocumentSymbol`: the entry belongs to the asked-about file.
        let start = &selection["start"];
        let (Some(line), Some(character)) = (
            start.get("line").and_then(|l| l.as_u64()),
            start.get("character").and_then(|c| c.as_u64()),
        ) else {
            return;
        };
        out.push(LspSymbol {
            name,
            kind,
            location: LspLocation {
                path: doc_path.to_path_buf(),
                line: line as u32 + 1, // LSP is 0-based
                character: character as u32 + 1,
            },
        });
        if let Some(children) = item.get("children").and_then(|c| c.as_array()) {
            for child in children {
                flatten_symbol(child, doc_path, out);
            }
        }
    }
    // Anything else is not a symbol: skipped, like a malformed location.
}

/// A symbol kind in words: real servers send the LSP number, the fixture sends the name, and
/// both are accepted so tests assert on what the model will read.
fn symbol_kind_name(kind: Option<&serde_json::Value>) -> String {
    match kind {
        Some(serde_json::Value::String(name)) => name.clone(),
        Some(serde_json::Value::Number(number)) => {
            symbol_kind_label(number.as_u64().unwrap_or(0)).to_string()
        }
        _ => "Unknown".to_string(),
    }
}

/// The LSP `SymbolKind` number table, in words.
fn symbol_kind_label(kind: u64) -> &'static str {
    match kind {
        1 => "File",
        2 => "Module",
        3 => "Namespace",
        4 => "Package",
        5 => "Class",
        6 => "Method",
        7 => "Property",
        8 => "Field",
        9 => "Constructor",
        10 => "Enum",
        11 => "Interface",
        12 => "Function",
        13 => "Variable",
        14 => "Constant",
        15 => "String",
        16 => "Number",
        17 => "Boolean",
        18 => "Array",
        19 => "Object",
        20 => "Key",
        21 => "Null",
        22 => "EnumMember",
        23 => "Struct",
        24 => "Event",
        25 => "Operator",
        26 => "TypeParameter",
        _ => "Unknown",
    }
}

/// The reverse of [`file_uri`](crate::server::file_uri): a `file://` URI back to a path.
///
/// Percent-decoding mirrors the encoder (`%25` last, so an encoded `%25` never decodes
/// twice), and a Windows `file:///C:/…` URI loses the leading slash before the drive letter.
/// Anything that is not a `file://` URI is `None` — an untitled document has no path to give.
fn uri_to_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let decoded = rest
        .replace("%20", " ")
        .replace("%23", "#")
        .replace("%3F", "?")
        .replace("%25", "%");
    let bytes = decoded.as_bytes();
    let text = if decoded.len() >= 3
        && bytes[0] == b'/'
        && bytes[1].is_ascii_alphabetic()
        && bytes[2] == b':'
    {
        decoded[1..].to_string()
    } else {
        decoded
    };
    Some(PathBuf::from(text))
}

/// Scrub credential-shaped environment variables from the server's environment.
///
/// `epsilon-safety` owns the real scrubber; this crate must not depend on it (the leaf tier is
/// `contracts <- lsp <- tools`, and safety is a sibling, not a dependency), so the same rule is
/// applied here in miniature: anything that looks like a key, token or secret is removed before a
/// third-party binary inherits it. The list is deliberately name-based and conservative — a
/// language server needs `PATH`, `HOME` and its own toolchain variables, nothing else.
fn scrub_env_floor(cmd: &mut Command) {
    const SECRETISH: &[&str] = &[
        "API_KEY",
        "APIKEY",
        "SECRET",
        "TOKEN",
        "PASSWORD",
        "PASSWD",
        "CREDENTIAL",
        "AUTH",
        "SESSION_KEY",
        "PRIVATE_KEY",
    ];
    let inherited: Vec<(String, String)> = std::env::vars()
        .filter(|(k, _)| {
            let upper = k.to_ascii_uppercase();
            if upper.starts_with("EPSILON_") {
                return true;
            }
            SECRETISH.iter().any(|needle| upper.contains(needle))
        })
        .collect();
    for (k, _) in inherited {
        cmd.env_remove(k);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostics_are_rebased_from_zero_to_one() {
        let params = serde_json::json!({
            "uri": "file:///x.rs",
            "diagnostics": [
                {"range": {"start": {"line": 0, "character": 0}}, "severity": 1, "message": "first"},
                {"range": {"start": {"line": 4, "character": 7}}, "severity": 2, "message": "second"}
            ]
        });
        let parsed = parse_diagnostics(&params);
        assert_eq!(parsed.len(), 2);
        assert_eq!((parsed[0].line, parsed[0].column), (1, 1));
        assert_eq!((parsed[1].line, parsed[1].column), (5, 8));
        assert_eq!(parsed[0].severity, Severity::Error);
        assert_eq!(parsed[1].severity, Severity::Warning);
    }

    #[test]
    fn a_publish_without_diagnostics_is_empty_not_an_error() {
        assert!(parse_diagnostics(&serde_json::json!({"uri": "file:///x.rs"})).is_empty());
        assert!(parse_diagnostics(&serde_json::json!({"diagnostics": []})).is_empty());
    }
}

#[cfg(test)]
mod query_tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::Duration;

    use crate::server::ServerDef;

    fn fixture_def() -> &'static ServerDef {
        Box::leak(Box::new(ServerDef {
            id: "fixture",
            name: "fixture",
            extensions: &["rs"],
            command: "lsp_fixture_server",
            args: &[],
            root_markers: &[],
        }))
    }

    /// The fixture executable beside this test binary (`target/<profile>/lsp_fixture_server`).
    ///
    /// `env!("CARGO_BIN_EXE_…")` is unavailable here — Cargo sets it when compiling
    /// integration tests, not the lib target — so the path is resolved at run time. Running
    /// the crate's tests (not `--lib` alone) builds the binary first.
    fn fixture_exe() -> PathBuf {
        if let Ok(path) = std::env::var("CARGO_BIN_EXE_lsp_fixture_server") {
            return PathBuf::from(path);
        }
        let mut dir = std::env::current_exe().expect("test executable path");
        dir.pop();
        if dir.file_name().is_some_and(|n| n == "deps") {
            dir.pop();
        }
        dir.join(format!(
            "lsp_fixture_server{}",
            std::env::consts::EXE_SUFFIX
        ))
    }

    /// Two sequential requests through one client: the correlator keys each response by id,
    /// so the second answer cannot be the first one repeated — and the echo proves the
    /// positions survived the round trip in the units they were sent in.
    #[tokio::test]
    async fn queries_echo_through_the_id_correlator_rebased_to_one_based() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.rs");
        std::fs::write(&file, "fn a() {}\n").unwrap();
        let cfg = ClientConfig {
            def: fixture_def(),
            root: dir.path().to_path_buf(),
            command_path: fixture_exe(),
            handshake_timeout: Duration::from_secs(10),
            publish_timeout: Duration::from_secs(10),
            env: Vec::new(),
        };
        let mut client = Client::start(cfg).await.expect("fixture starts");
        let defs = client
            .go_to_definition(&file, "fn a() {}\n", 4, 2)
            .await
            .expect("definition answers");
        assert_eq!(defs.len(), 1, "the canned echo answers once");
        assert_eq!(
            (defs[0].line, defs[0].character),
            (5, 3),
            "0-based (4,2) in, 1-based (5,3) out"
        );
        let refs = client
            .find_references(&file, "fn a() {}\n", 4, 2, true)
            .await
            .expect("references answer");
        assert_eq!(
            refs.len(),
            25,
            "the canned reference list is 25 with the declaration"
        );
        let refs_no_decl = client
            .find_references(&file, "fn a() {}\n", 4, 2, false)
            .await
            .expect("references answer");
        assert_eq!(
            refs_no_decl.len(),
            24,
            "includeDeclaration=false drops the declaration"
        );
        client.shutdown().await;
    }

    #[test]
    fn a_single_location_and_a_list_parse_the_same() {
        let single = serde_json::json!({
            "uri": "file:///ws/src/a.rs",
            "range": {"start": {"line": 4, "character": 2}, "end": {"line": 4, "character": 9}}
        });
        let parsed = parse_locations(&single).expect("a bare Location parses");
        assert_eq!(parsed.len(), 1);
        assert_eq!((parsed[0].line, parsed[0].character), (5, 3));
        let list = serde_json::json!([single]);
        let parsed = parse_locations(&list).expect("a list parses");
        assert_eq!(parsed.len(), 1);
    }

    #[test]
    fn a_null_result_is_zero_hits_not_an_error() {
        let parsed = parse_locations(&serde_json::Value::Null).expect("null is an answer");
        assert!(parsed.is_empty());
        let parsed = parse_symbols(&serde_json::Value::Null, std::path::Path::new("/ws/a.rs"))
            .expect("null symbols parse");
        assert!(parsed.is_empty());
    }

    #[test]
    fn symbols_parse_both_wire_shapes() {
        let doc = std::path::Path::new("/ws/a.rs");
        // SymbolInformation: flat, with `location`, string kind as the fixture sends.
        let flat = serde_json::json!([
            {"name": "alpha", "kind": "Function",
             "location": {"uri": "file:///ws/a.rs",
                          "range": {"start": {"line": 0, "character": 0},
                                    "end": {"line": 0, "character": 5}}}}
        ]);
        let parsed = parse_symbols(&flat, doc).expect("flat symbols parse");
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].name, "alpha");
        assert_eq!(parsed[0].kind, "Function");
        assert_eq!(
            (parsed[0].location.line, parsed[0].location.character),
            (1, 1)
        );
        // A numeric kind maps to its LSP name (12 is Function).
        let numeric = serde_json::json!([
            {"name": "beta", "kind": 12,
             "location": {"uri": "file:///ws/a.rs",
                          "range": {"start": {"line": 1, "character": 0},
                                    "end": {"line": 1, "character": 4}}}}
        ]);
        let parsed = parse_symbols(&numeric, doc).expect("numeric kinds parse");
        assert_eq!(parsed[0].kind, "Function");
        // DocumentSymbol: nested, with `selectionRange` and `children`.
        let nested = serde_json::json!([
            {"name": "outer", "kind": 5,
             "range": {"start": {"line": 0, "character": 0}, "end": {"line": 9, "character": 1}},
             "selectionRange": {"start": {"line": 0, "character": 6},
                                "end": {"line": 0, "character": 11}},
             "children": [
                 {"name": "inner", "kind": 6,
                  "range": {"start": {"line": 2, "character": 0},
                            "end": {"line": 3, "character": 1}},
                  "selectionRange": {"start": {"line": 2, "character": 8},
                                     "end": {"line": 2, "character": 13}}}
             ]}
        ]);
        let parsed = parse_symbols(&nested, doc).expect("nested symbols flatten");
        assert_eq!(parsed.len(), 2, "parent plus child");
        assert_eq!(parsed[0].name, "outer");
        assert_eq!(
            (parsed[0].location.line, parsed[0].location.character),
            (1, 7)
        );
        assert_eq!(parsed[1].name, "inner");
    }

    #[test]
    fn uri_decoding_round_trips_file_uri() {
        // A real absolute path: the round trip only holds for one, since `file_uri`
        // absolutizes against the current drive.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a dir").join("file#1.rs");
        let uri = crate::server::file_uri(&path);
        assert_eq!(uri_to_path(&uri), Some(path));
        assert_eq!(uri_to_path("untitled:Untitled-1"), None);
    }
}
