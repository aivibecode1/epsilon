//! The host side: spawn, handshake, invoke-with-timeout, crash accounting.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout};
use tokio::sync::Mutex;

use crate::manifest::ExtManifest;
use crate::{CRASH_BUDGET, INVOKE_TIMEOUT_SECS};

/// Errors surfaced by the host. All are values — a crashing extension must degrade the
/// extension, never the harness.
#[derive(Debug)]
pub enum ExtError {
    /// The extension exceeded its per-invoke timeout.
    Timeout,
    /// The process died (crash budget tracks disabling).
    Crashed(String),
    /// Protocol violation (bad line, wrong id, malformed JSON).
    Protocol(String),
    /// The extension returned an error result for this invocation.
    ToolFailed {
        /// Error text the model should see.
        message: String,
    },
}

impl std::fmt::Display for ExtError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExtError::Timeout => write!(f, "extension invoke timed out"),
            ExtError::Crashed(d) => write!(f, "extension crashed: {d}"),
            ExtError::Protocol(d) => write!(f, "extension protocol violation: {d}"),
            ExtError::ToolFailed { message } => write!(f, "{message}"),
        }
    }
}

/// A live extension process.
pub struct ExtHost {
    name: String,
    io: Mutex<ChildIo>,
    disabled: AtomicBool,
    crashes: AtomicU32,
}

struct ChildIo {
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
    /// Held so kill_on_drop applies when the host is dropped.
    #[allow(dead_code)]
    child: Child,
}

impl ExtHost {
    /// Spawn + handshake. Fails fast if the binary is missing or hello never arrives.
    pub async fn spawn(dir: &Path, manifest: &ExtManifest) -> Result<Arc<Self>, ExtError> {
        let bin = manifest.resolved_bin(dir);
        let mut child = tokio::process::Command::new(&bin)
            .args(&manifest.args)
            .current_dir(dir)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| ExtError::Crashed(format!("spawn {}: {e}", bin.display())))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| ExtError::Protocol("no stdin".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| ExtError::Protocol("no stdout".into()))?;
        let mut reader = BufReader::new(stdout);

        // Handshake with timeout: first line must be a hello.
        let hello = tokio::time::timeout(
            Duration::from_secs(INVOKE_TIMEOUT_SECS),
            read_line_str(&mut reader),
        )
        .await
        .map_err(|_| ExtError::Timeout)?
        .map_err(|e| ExtError::Crashed(e.to_string()))?;

        let line = hello.ok_or_else(|| ExtError::Crashed("exited before hello".into()))?;
        let v: serde_json::Value =
            serde_json::from_str(&line).map_err(|e| ExtError::Protocol(e.to_string()))?;
        if v.get("type").and_then(|t| t.as_str()) != Some("hello") {
            return Err(ExtError::Protocol("first message must be hello".into()));
        }

        Ok(Arc::new(Self {
            name: manifest.name.clone(),
            io: Mutex::new(ChildIo {
                stdin,
                stdout: reader,
                next_id: 0,
                child,
            }),
            disabled: AtomicBool::new(false),
            crashes: AtomicU32::new(0),
        }))
    }

    /// Extension name from its manifest.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// True once the crash budget is exhausted.
    pub fn is_disabled(&self) -> bool {
        self.disabled.load(Ordering::SeqCst)
    }

    /// Invoke one tool by name. Serialized per extension (extensions own their ordering);
    /// timeout enforced; crashes counted against the budget.
    pub async fn invoke(
        &self,
        tool: &str,
        call_id: &str,
        args: serde_json::Value,
    ) -> Result<(String /*text*/, serde_json::Value /*details*/), ExtError> {
        if self.disabled.load(Ordering::SeqCst) {
            return Err(ExtError::Crashed(
                "extension disabled (crash budget)".into(),
            ));
        }

        let id = {
            let mut io = self.io.lock().await;
            io.next_id += 1;
            format!("i_{}", io.next_id)
        };
        let req = serde_json::json!({
            "type": "invoke", "id": id, "call_id": call_id,
            "tool": tool, "args": args,
        });

        let result = self.write_and_read(&id, req).await;
        match result {
            Ok(ok) => Ok(ok),
            Err(ExtError::Crashed(d)) => {
                let n = self.crashes.fetch_add(1, Ordering::SeqCst) + 1;
                if n >= CRASH_BUDGET {
                    self.disabled.store(true, Ordering::SeqCst);
                }
                Err(ExtError::Crashed(d))
            }
            Err(other) => Err(other),
        }
    }

    async fn write_and_read(
        &self,
        id: &str,
        req: serde_json::Value,
    ) -> Result<(String, serde_json::Value), ExtError> {
        let mut io = self.io.lock().await;

        let mut line =
            serde_json::to_string(&req).map_err(|e| ExtError::Protocol(e.to_string()))?;
        line.push('\n');
        io.stdin
            .write_all(line.as_bytes())
            .await
            .map_err(|e| ExtError::Crashed(format!("write: {e}")))?;
        io.stdin
            .flush()
            .await
            .map_err(|e| ExtError::Crashed(format!("flush: {e}")))?;

        loop {
            let line = tokio::time::timeout(
                Duration::from_secs(INVOKE_TIMEOUT_SECS),
                read_line_str(&mut io.stdout),
            )
            .await
            .map_err(|_| ExtError::Timeout)?
            .map_err(|e| ExtError::Crashed(format!("read: {e}")))?;
            let Some(line) = line else {
                return Err(ExtError::Crashed("stdout closed".into()));
            };
            let v: serde_json::Value = serde_json::from_str(&line)
                .map_err(|e| ExtError::Protocol(format!("{e} in: {line}")))?;
            if v.get("type").and_then(|t| t.as_str()) != Some("result") {
                continue; // ignore non-result chatter v1
            }
            if v.get("id").and_then(|i| i.as_str()) != Some(id) {
                continue; // stale/unknown id: skip
            }
            let is_error = v.get("is_error").and_then(|b| b.as_bool()).unwrap_or(false);
            let text = v
                .get("text")
                .and_then(|t| t.as_str())
                .unwrap_or_default()
                .to_string();
            let details = v.get("details").cloned().unwrap_or(serde_json::json!({}));
            return if is_error {
                Err(ExtError::ToolFailed { message: text })
            } else {
                Ok((text, details))
            };
        }
    }
}

/// tokio 1.53 dropped the `next_line` convenience; read one newline-terminated line.
async fn read_line_str<R: tokio::io::AsyncBufRead + Unpin>(
    reader: &mut R,
) -> std::io::Result<Option<String>> {
    let mut buf = String::new();
    let n = reader.read_line(&mut buf).await?;
    Ok(if n == 0 {
        None
    } else {
        Some(buf.trim_end_matches(['\r', '\n']).to_string())
    })
}
