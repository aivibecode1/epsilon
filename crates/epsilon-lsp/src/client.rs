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
