//! The host side: spawn, handshake, invoke-with-timeout, crash accounting.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout};
use tokio::sync::{Mutex, oneshot};

use crate::manifest::ExtManifest;
use crate::{CRASH_BUDGET, INTERPOSE_TIMEOUT_SECS, INVOKE_TIMEOUT_SECS};

/// Largest single protocol frame accepted from a child (2 MiB, aligned with
/// `epsilon_tools::checkpoint::MAX_FILE_BYTES`).
///
/// A line is the only unit the protocol has, so a child that never sends a newline would
/// otherwise make the host buffer whatever it likes: `read_line` is happy to accumulate
/// megabytes, and the child chooses the number. The cap is far above any frame the host
/// itself sends and far above a large tool result; it exists to bound the host's memory,
/// not to police result size.
const MAX_FRAME_BYTES: usize = 2 * 1024 * 1024;

/// Most bytes of child output one invoke may consume.
///
/// `INVOKE_TIMEOUT_SECS` bounds an invoke in time; this bounds it in bytes. Without it a
/// child could keep the reader busy for the whole window — and, because chatter is skipped
/// rather than failing, busy is all it would ever be — on a stream the host can never
/// finish reading. Hitting this is a protocol violation, not a slow extension.
const MAX_INVOKE_BYTES: usize = 16 * 1024 * 1024;

/// An environment scrubber for a spawned extension: the harness's rule, injected so the
/// subprocess policy lives in one place.
///
/// `epsilon-safety` owns the rule and this crate is a leaf that must not depend on it — the
/// same arrangement `epsilon-lsp` uses (`client::ScrubEnv`). Passing it as an `Option` is
/// deliberate rather than lazy: it keeps the leaf honest about who decides, and `None` is a
/// legitimate answer for a test. **The guard that keeps it from rotting is a test, not this
/// comment**: `runtime` walks every `Command::new` in the workspace and fails on one that
/// spawns without a scrubber, because an extension is a *third-party binary* and this was one
/// of only two spawn paths that handed it the developer's whole environment (ledger A34).
pub type ScrubEnv = std::sync::Arc<dyn Fn(&mut tokio::process::Command) + Send + Sync>;

/// Errors surfaced by the host. All are values — a crashing extension must degrade the
/// extension, never the harness.
///
/// `Clone` because the reader reports one structural failure to every request waiting on
/// the pipe, and each needs its own copy. Every variant is owned data (a `String` and two
/// scalars), so this is a memcpy rather than a deep clone, and it removes a footgun: a
/// hand-written `Clone` that quietly dropped a variant's payload would be invisible.
#[derive(Debug, Clone)]
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
    /// **Write side only** — see [`ChildIo`]. Holding this across an await that waits
    /// for the child to speak would deadlock, because the child cannot speak until
    /// something is reading its stdout, and reading is the reader task's job (ADR-019).
    io: Mutex<ChildIo>,
    disabled: AtomicBool,
    crashes: AtomicU32,
    /// Plan 007: the manifest's declared event interests (`["*"]` = all). Empty list
    /// means the extension receives nothing. Evaluated by `wants_event`.
    events: Vec<String>,
    /// In-flight requests keyed by the `id` sent in the request frame.
    ///
    /// The reader task owns the child's stdout and is the only thing that reads it: it
    /// parses each frame, takes the `id`, and completes the matching oneshot. Two
    /// requests can therefore be outstanding at once, and a reply for `i_7` can never be
    /// handed to whoever is waiting on `i_8`. One mutex over one pipe could not do that —
    /// a reader looping until it sees *its* id would block every other request behind the
    /// first, and the same mutex is what `send_event` needs to write a line.
    ///
    /// `i_*` is an invoke, `x_*` an interpose. They share the map because they share the
    /// pipe, and a single reader cannot tell them apart without looking.
    pending: Arc<Mutex<PendingMap>>,
    /// Set once the reader task has stopped for any reason other than our own shutdown.
    ///
    /// **Why the host needs to know this at all.** The reader calls `fail_all` when it
    /// stops, which wakes every request that was *already registered* at that moment. A
    /// request registered *after* the reader is gone has nobody to wake it: its frame
    /// goes into a pipe no one reads, and it waits out the full invoke timeout —
    /// ten seconds of silence for a child that died a moment ago. That is the "the
    /// harness has frozen" symptom, reproduced in miniature, and it is why an extension
    /// that crashed on the first call made every subsequent call look like a hang.
    ///
    /// So the reader marks itself gone, and a request that arrives afterwards is
    /// refused immediately with the reason the reader stopped. A `Timeout` here would
    /// also be a lie: nothing waited ten seconds to fail.
    ///
    /// The flag is set before `fail_all` so a request registered between the reader's
    /// last frame and its `fail_all` sees a stopped reader rather than racing it.
    stopped: Arc<AtomicBool>,
    /// The child's `{"type":"log"}` lines. Split out of [`ChildIo`] because the reader
    /// task is the only thing that sees stdout now, and it must not take the write mutex
    /// to append a diagnostic — doing so would reintroduce exactly the coupling ADR-019
    /// removes.
    logs: Arc<Mutex<Vec<String>>>,
    /// The stdout reader. Kept so the host owns its lifetime: dropping the host stops it.
    _reader: ReaderTask,
}

/// Outstanding requests, keyed by frame id.
///
/// The channel carries a `Result`, not a bare value, because **the reason a request failed
/// is part of the answer**. The pre-ADR-019 code read the pipe inside `invoke`, so it knew
/// whether it had seen a malformed line, an oversized frame, or a spent byte budget. A
/// demultiplexer sees those same conditions but cannot tell which request they belong to,
/// so it reports them to the requests that are in flight and lets `validate_result` handle
/// a well-formed frame. Collapsing all of that to a single `Null` marker would have
/// silently turned three protocol violations into "the child died" — which is both a lie
/// and, per the crash budget, a behaviour change.
#[derive(Default)]
struct PendingMap {
    map: HashMap<String, oneshot::Sender<Result<serde_json::Value, ExtError>>>,
}

struct ChildIo {
    stdin: ChildStdin,
    next_id: u64,
    /// Held so kill_on_drop applies when the host is dropped.
    #[allow(dead_code)]
    child: Child,
}

/// Cap on retained child log lines. Oldest are dropped, not the newest: a runaway
/// extension is best explained by its first complaints.
const MAX_LOG_LINES: usize = 1000;

/// The reader task's handle. Held so the host owns the reader's lifetime.
///
/// The shutdown signal is carried by **dropping the sender**, not by sending on it: a
/// `oneshot::Sender` that is dropped makes the receiver resolve to `Err`, which ends the
/// reader's `select!` without needing a running task to deliver anything. That is why the
/// field looks unread — it is load-bearing by existing. `Drop` says so out loud, and drops
/// the `JoinHandle` without awaiting it: the task observes channel closure and exits on
/// its next frame, and blocking a `Drop` on a task that might be mid-read would risk
/// hanging whichever runtime is tearing the host down.
struct ReaderTask {
    /// Dropped to signal shutdown. See the type comment.
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    /// Detached on drop; see the type comment.
    handle: tokio::task::JoinHandle<()>,
}

impl Drop for ReaderTask {
    fn drop(&mut self) {
        // Dropping the sender resolves the reader's `select!` with `Err`, which ends the
        // loop. Taken explicitly so the field is genuinely read, and so the intent
        // survives a later edit that would otherwise drop it as unused.
        drop(self.shutdown.take());
        self.handle.abort();
    }
}

impl ExtHost {
    /// Spawn + handshake. Fails fast if the binary is missing or hello never arrives.
    ///
    /// `scrub_env` is the harness's credential filter, **injected** rather than reached for:
    /// `epsilon-ext` is a leaf crate and must not depend on `epsilon-safety`, which is the
    /// same arrangement `epsilon-lsp` uses (`client::ScrubEnv`). See [`ScrubEnv`] for why
    /// this parameter exists at all — an extension is a **third-party binary**, and this was
    /// one of the two spawn paths that handed it the developer's whole environment
    /// (ledger A34).
    pub async fn spawn(
        dir: &Path,
        manifest: &ExtManifest,
        scrub_env: Option<ScrubEnv>,
    ) -> Result<Arc<Self>, ExtError> {
        let bin = manifest.resolved_bin(dir);
        let mut command = tokio::process::Command::new(&bin);
        command
            .args(&manifest.args)
            .current_dir(dir)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);
        // Before the spawn, and before anything this process will read back: an
        // extension is code from outside the harness, and `API_KEY` in its environment is
        // a credential handed to it by name.
        if let Some(scrub) = &scrub_env {
            scrub(&mut command);
        }
        let mut child = command
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

        // Handshake with timeout: first line must be a hello. Read DIRECTLY rather than
        // through the reader task, because the task is not started yet and no id has been
        // allocated — the hello is not a reply to anything.
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

        // From here on the reader task owns stdout. `self` must exist before it starts,
        // because the task holds a clone of the pending map the host also holds.
        let pending: Arc<Mutex<PendingMap>> = Arc::new(Mutex::new(PendingMap::default()));
        let logs: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let stopped = Arc::new(AtomicBool::new(false));
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(read_frames(
            reader,
            pending.clone(),
            logs.clone(),
            stopped.clone(),
            shutdown_rx,
        ));

        Ok(Arc::new(Self {
            name: manifest.name.clone(),
            io: Mutex::new(ChildIo {
                stdin,
                next_id: 0,
                child,
            }),
            disabled: AtomicBool::new(false),
            crashes: AtomicU32::new(0),
            events: manifest.events.clone(),
            pending,
            stopped,
            logs,
            _reader: ReaderTask {
                shutdown: Some(shutdown_tx),
                handle: task,
            },
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

    /// Forward one agent event to the child (fire-and-forget, subject to the
    /// extension's declared manifest filter). Best-effort: event delivery must never
    /// break a run.
    ///
    /// Plan 007: the `filtered` parameter is GONE — the host now evaluates its own
    /// manifest's `events` list (captured at spawn) so a declared `["*"]` genuinely
    /// receives events and an empty list receives nothing. The old signature with a
    /// hardcoded `true` made forwarding dead code.
    pub async fn send_event(&self, event: &serde_json::Value) {
        // Event type tag lives at the top level of the payload (`event.type`).
        let tag = event
            .get("type")
            .and_then(|t| t.as_str())
            .unwrap_or("unknown");
        let wants = self.events.iter().any(|e| e == "*" || e == tag);
        if !wants {
            return;
        }
        let mut io = self.io.lock().await;
        let mut line = serde_json::to_string(&serde_json::json!({
            "type": "event",
            "event": event,
        }))
        .expect("event serializes");
        line.push('\n');
        let _ = io.stdin.write_all(line.as_bytes()).await;
        let _ = io.stdin.flush().await;
    }

    /// How long a forwarded event was blocked by concurrent activity on the write lock.
    ///
    /// Named for what it measures, and public, because the number is the claim: under the
    /// pre-ADR-019 design an `invoke` held the pipe's lock for its whole duration, so this
    /// returned the remaining length of that invoke (measured at ~2.0 s against a 10 s
    /// timeout). It now returns the time to write one line.
    pub async fn probe_event_forward_latency(
        &self,
        event: &serde_json::Value,
    ) -> std::time::Duration {
        let t0 = std::time::Instant::now();
        self.send_event(event).await;
        t0.elapsed()
    }

    /// Log receipts the child sent via {"type":"log"} lines.
    pub async fn logs(&self) -> Vec<String> {
        self.logs.lock().await.clone()
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

        match self
            .request(id, req, Duration::from_secs(INVOKE_TIMEOUT_SECS))
            .await
        {
            Ok(frame) => self.validate_result(&frame),
            // A dead child spends the budget; a malformed frame does not. That distinction
            // is the reason `request` returns a typed error rather than a bare value, and
            // losing it here would let one stray line disable an extension for the session.
            Err(ExtError::Crashed(d)) => {
                self.count_crash();
                Err(ExtError::Crashed(d))
            }
            Err(other) => Err(other),
        }
    }

    /// Charge one crash against the budget, disabling the extension once it is spent.
    ///
    /// Two crashes is a deliberate number rather than one: a single death is often a race
    /// between the host closing stdin and the child exiting, and disabling on that would
    /// make ordinary shutdown look like a fault.
    fn count_crash(&self) {
        let n = self.crashes.fetch_add(1, Ordering::SeqCst) + 1;
        if n >= CRASH_BUDGET {
            self.disabled.store(true, Ordering::SeqCst);
        }
    }

    /// Turn a `result` frame into a tool result, or into the protocol error it is.
    ///
    /// A frame is validated before anything in it is believed. Accepting a frame with no
    /// `text`, or a `details` that is not an object, would turn a broken child into a
    /// successful empty tool result — indistinguishable, in the transcript, from a tool
    /// that legitimately returned nothing. So the host names the problem instead of
    /// quoting the payload, and `ExtError::Protocol` becomes `ToolFailure` text that
    /// lands in the transcript where the user can see it.
    fn validate_result(
        &self,
        v: &serde_json::Value,
    ) -> Result<(String, serde_json::Value), ExtError> {
        let text = match v.get("text") {
            Some(serde_json::Value::String(t)) => t.clone(),
            Some(other) => {
                return Err(ExtError::Protocol(format!(
                    "result.text must be a string, got {}",
                    json_kind(other)
                )));
            }
            None => return Err(ExtError::Protocol("result has no text".into())),
        };
        let details = match v.get("details") {
            Some(d @ serde_json::Value::Object(_)) => d.clone(),
            Some(other) => {
                return Err(ExtError::Protocol(format!(
                    "result.details must be an object, got {}",
                    json_kind(other)
                )));
            }
            None => return Err(ExtError::Protocol("result has no details".into())),
        };
        let is_error = match v.get("is_error") {
            Some(serde_json::Value::Bool(b)) => *b,
            Some(other) => {
                return Err(ExtError::Protocol(format!(
                    "result.is_error must be a bool, got {}",
                    json_kind(other)
                )));
            }
            None => return Err(ExtError::Protocol("result has no is_error".into())),
        };
        if is_error {
            Err(ExtError::ToolFailed { message: text })
        } else {
            Ok((text, details))
        }
    }

    /// Write one request frame and wait for the frame whose `id` matches.
    ///
    /// The wait is a `oneshot` completed by the reader task, not a read loop. That is the
    /// whole point of ADR-019: because the reply arrives through a shared demultiplexer
    /// rather than through a stream this call owns, **the write mutex is released before
    /// anything is awaited**. Two invocations can be in flight, an `event` frame can be
    /// written while a tool call is outstanding, and a reply for one request can never be
    /// consumed by another.
    ///
    /// The deadline covers the whole request, and a timeout is reported as `ExtError::Timeout`
    /// so the caller applies the usual crash accounting.
    async fn request(
        &self,
        id: String,
        req: serde_json::Value,
        budget: Duration,
    ) -> Result<serde_json::Value, ExtError> {
        // Refuse a request the reader can no longer answer, before writing to a pipe
        // nobody reads. Without this, a caller that arrives after the child died waits
        // out the whole budget for a reply that was never going to come: the frame is
        // written, the `oneshot` is registered, and the only thing that can complete it
        // is a task that has already exited. The caller sees a `Timeout` — which is
        // also a lie, because nothing was slow — after ten seconds of silence.
        //
        // This is the "nothing happened" state this project refuses to let the user see,
        // one layer below the UI: a dead extension looks exactly like a frozen harness.
        // Charged as a crash, because the child is what stopped answering, and the crash
        // budget is what eventually disables a host that keeps dying.
        if self.stopped.load(Ordering::SeqCst) {
            return Err(ExtError::Crashed(
                "extension reader stopped before this request".into(),
            ));
        }

        let (tx, rx) = oneshot::channel();
        {
            let mut io = self.io.lock().await;
            // Register BEFORE writing. The reader can reply the instant the bytes land, so
            // a write-then-register order would race and drop a perfectly good reply.
            self.pending.lock().await.map.insert(id.clone(), tx);
            let mut line =
                serde_json::to_string(&req).map_err(|e| ExtError::Protocol(e.to_string()))?;
            line.push('\n');
            if let Err(e) = io.stdin.write_all(line.as_bytes()).await {
                self.pending.lock().await.map.remove(&id);
                return Err(ExtError::Crashed(format!("write: {e}")));
            }
            if let Err(e) = io.stdin.flush().await {
                self.pending.lock().await.map.remove(&id);
                return Err(ExtError::Crashed(format!("flush: {e}")));
            }
        }

        let deadline = tokio::time::Instant::now() + budget;
        match tokio::time::timeout_at(deadline, rx).await {
            Ok(Ok(Ok(v))) => Ok(v),
            // The reader reports a structural failure (malformed line, oversized frame,
            // spent budget) to every request in flight, and reports the stream ending as a
            // crash. Both are the reader's account of what it saw.
            Ok(Ok(Err(e))) => Err(e),
            // The reader task is gone without answering: the channel closed. Treat as a
            // crash, because the child is what stopped answering.
            Ok(Err(_)) => Err(ExtError::Crashed("extension reader stopped".into())),
            Err(_) => {
                // Stop a late reply from completing a request nobody is waiting for.
                self.pending.lock().await.map.remove(&id);
                Err(ExtError::Timeout)
            }
        }
    }

    /// Ask every extension that declared interest in `event` for a decision, and collect
    /// the answers. Extensions are asked concurrently; one that hangs is bounded by the
    /// shared invoke timeout, not by the others.
    ///
    /// Returns a list of decisions **in the order hosts were attached**, so a caller can
    /// attribute a veto to the extension that cast it. An empty list means no extension had
    /// an opinion, which is the common case and the only one that leaves the call alone.
    pub async fn interpose(&self, event: &serde_json::Value) -> Vec<serde_json::Value> {
        let tag = event
            .get("type")
            .and_then(|t| t.as_str())
            .unwrap_or("unknown");
        if !self.events.iter().any(|e| e == "*" || e == tag) {
            return Vec::new();
        }
        if self.is_disabled() {
            return Vec::new();
        }
        let id = {
            let mut io = self.io.lock().await;
            io.next_id += 1;
            format!("x_{}", io.next_id)
        };
        let req = serde_json::json!({
            "type": "interpose",
            "id": id,
            "event": event,
        });
        match self
            .request(id, req, Duration::from_secs(INTERPOSE_TIMEOUT_SECS))
            .await
        {
            Ok(v) if v.is_null() => Vec::new(),
            Ok(v) => vec![v],
            // Timeout, crash, or protocol violation: no opinion. The run continues, which
            // is the whole failure policy (ADR-019). An interpose is allowed to add
            // friction deliberately; it is never allowed to remove the harness's ability
            // to finish, so an extension that cannot answer does not get a veto.
            Err(_) => Vec::new(),
        }
    }
}

/// Own the child's stdout for the life of the host, routing each frame to whoever is
/// waiting for its id.
///
/// Four outcomes per frame, and the distinctions matter more than they look:
///
/// 1. **Not JSON** — a protocol violation. Every request in flight is told so. Skipping it
///    instead would make a child that answers everything with prose indistinguishable from
///    one that has gone silent.
/// 2. **`{"type":"log"}`** — the child's diagnostics channel. Collected, never a reply.
/// 3. **Valid JSON with no `id`** — chatter. Skipped, and the wait continues.
/// 4. **Valid JSON with an `id`** — handed to that request's oneshot. An id nobody is
///    waiting on is dropped, which is the normal outcome for a reply that arrives after
///    its own timeout fired.
///
/// A demultiplexer cannot attribute an id-less failure to one request, so a structural
/// failure is reported to all of them. That is the conservative direction: a caller that
/// gets an error still lets the run continue, whereas a caller left waiting does not.
///
/// `MAX_FRAME_BYTES` and `MAX_INVOKE_BYTES` bound the read *here*, because this is now the
/// only place stdout is read: a bound enforced by a caller that no longer reads is a bound
/// that no longer bounds anything.
async fn read_frames(
    mut reader: BufReader<ChildStdout>,
    pending: Arc<Mutex<PendingMap>>,
    logs: Arc<Mutex<Vec<String>>>,
    stopped: Arc<AtomicBool>,
    mut shutdown: tokio::sync::oneshot::Receiver<()>,
) {
    let mut total_bytes: u64 = 0;
    // Why the loop stopped, if it did. `None` means the host shut us down, which is not a
    // failure and must not be reported as one.
    let mut abort: Option<String> = None;
    // Whether the `select!` below ended on the shutdown signal rather than on the stream.
    // The distinction decides two things: whether a crash is charged to whatever was in
    // flight, and whether the host is told its reader is gone. Shutdown is neither.
    let mut stopped_on_shutdown = false;

    loop {
        let line = tokio::select! {
            biased;
            _ = &mut shutdown => {
                stopped_on_shutdown = true;
                break;
            },
            read = read_line_bytes(&mut reader) => match read {
                Ok(Frame::Line(l)) => l,
                Ok(Frame::Eof) | Err(_) => {
                    // The child stopped talking. That is a crash, and it is charged to the
                    // crash budget — but only by the awaiting caller, which knows whether
                    // anything was actually in flight.
                    break;
                }
                Ok(Frame::TooLong) => {
                    // The rest of the line is still in the pipe, so the stream is now
                    // desynchronized: reading on would splice that remainder into the next
                    // frame. Stop, and say why.
                    abort = Some(format!("frame exceeds {MAX_FRAME_BYTES} bytes"));
                    break;
                }
            },
        };

        total_bytes = total_bytes.saturating_add(line.len() as u64);
        if total_bytes > MAX_INVOKE_BYTES as u64 {
            abort = Some(format!(
                "child produced more than {MAX_INVOKE_BYTES} bytes without answering"
            ));
            break;
        }

        // A line that is not JSON is a protocol violation, not chatter: the old loop
        // reported it as one, and quietly skipping it would let a child that answers
        // everything with prose look like a child that has gone quiet. It is charged to
        // the requests in flight, because there is no id to attribute it to and refusing
        // to answer is the honest outcome for a caller waiting on a broken peer.
        //
        // The child's own text is deliberately not quoted: this text would otherwise reach
        // the model transcript behind a `[harness]` prefix, carrying the extension's words
        // as though the harness had written them.
        let v: serde_json::Value = match serde_json::from_slice(&line) {
            Ok(v) => v,
            Err(_) => {
                fail_all(
                    &pending,
                    ExtError::Protocol("a response line was not valid JSON".to_string()),
                )
                .await;
                continue;
            }
        };

        if v.get("type").and_then(|t| t.as_str()) == Some("log") {
            if let Some(text) = v.get("text").and_then(|t| t.as_str()) {
                let mut l = logs.lock().await;
                if l.len() < MAX_LOG_LINES {
                    l.push(text.to_string());
                }
            }
            continue;
        }

        // Well-formed JSON that is not a reply is chatter: keep serving.
        //
        // A bare scalar (`42`) has no fields to be wrong about, so it is plainly noise. A
        // JSON *object* that claims to be a reply but cannot be addressed — `id` missing, or
        // `id` a number — is a different case: it is a frame the child meant as an answer
        // and got wrong. Either way it is not the answer to the request in flight, and the
        // caller must not be handed it, so the wait continues. What matters is that
        // neither is mistaken for a result, which is what the pre-ADR-019 loop guaranteed
        // and what `--shape-noise` exists to check.
        // Only a *reply* frame is an answer, and there are now two of them: `result` for an
        // invoke, `interpose_result` for a mutating event (ADR-019).
        //
        // This check is load-bearing and is *not* redundant with the id match: a child can
        // stamp a real request id on a frame of any other type, and demultiplexing on the
        // id alone would hand that through as the reply. `--shape-noise` sends exactly
        // such a frame, and the pre-ADR-019 loop filtered on type for the same reason.
        //
        // Kept as a match on both names rather than "anything with an id", because the
        // permissive version is precisely the bug this guards.
        match v.get("type").and_then(|t| t.as_str()) {
            Some("result") | Some("interpose_result") => {}
            _ => continue,
        }
        if !v.is_object() {
            continue;
        }
        let Some(id) = v.get("id").and_then(|i| i.as_str()).map(str::to_owned) else {
            continue;
        };
        let tx = pending.lock().await.map.remove(&id);
        if let Some(tx) = tx {
            let _ = tx.send(Ok(v));
        }
    }

    // Announce the stop BEFORE waking anyone, so a request registered in the gap between
    // this line and `fail_all` sees a stopped reader rather than racing the wake-up.
    if !stopped_on_shutdown {
        stopped.store(true, Ordering::SeqCst);
    }

    if let Some(why) = abort {
        fail_all(&pending, ExtError::Protocol(why)).await;
    } else if !stopped_on_shutdown {
        // The stream ended without a verdict. Whoever was waiting learns the child is gone,
        // which the caller charges to the crash budget.
        fail_all(
            &pending,
            ExtError::Crashed("extension closed the connection".to_string()),
        )
        .await;
    }
}

/// Tell every request in flight why it is not going to be answered, then forget them.
///
/// `ExtError` is not `Clone` (it carries owned text), so each waiter gets its own
/// construction of the same reason rather than a shared value.
async fn fail_all(pending: &Arc<Mutex<PendingMap>>, why: ExtError) {
    let drained: Vec<_> = {
        let mut m = pending.lock().await;
        m.map.drain().map(|(_, tx)| tx).collect()
    };
    for tx in drained {
        let _ = tx.send(Err(why.clone()));
    }
}

/// What one read off the child's stdout produced.
enum Frame {
    /// A complete line, with its terminator stripped and its JSON escapes intact.
    Line(Vec<u8>),
    /// Clean end of stream.
    Eof,
    /// A line longer than [`MAX_FRAME_BYTES`]; the rest of it is left unread on purpose.
    TooLong,
}

/// Read one protocol frame as bytes, refusing to buffer more than [`MAX_FRAME_BYTES`].
///
/// Bytes rather than `read_line`'s `String`: a line is only known to be UTF-8 once it is
/// complete, and deciding that per 8 KiB pipe chunk would reject a multi-byte character
/// that happens to straddle a buffer boundary. The cap is what keeps a child that never
/// sends a newline from choosing how much memory the harness allocates; when it trips,
/// the host stops draining the line, so callers must treat the stream as desynchronized.
async fn read_line_bytes<R: tokio::io::AsyncBufRead + Unpin>(
    reader: &mut R,
) -> std::io::Result<Frame> {
    let mut buf: Vec<u8> = Vec::new();
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            return Ok(if buf.is_empty() {
                Frame::Eof
            } else {
                // Final line with no terminator, then EOF: still a frame.
                Frame::Line(buf)
            });
        }
        if let Some(pos) = available.iter().position(|b| *b == b'\n') {
            if buf.len() + pos > MAX_FRAME_BYTES {
                // The terminator is already in view, so the overrun is knowable without
                // buffering it: the check below the loop exists for the cases this one
                // cannot see, and waiting for it here would store the payload it refuses.
                return Ok(Frame::TooLong);
            }
            buf.extend_from_slice(&available[..=pos]);
            reader.consume(pos + 1);
            break;
        }
        let n = available.len();
        buf.extend_from_slice(available);
        reader.consume(n);
        if buf.len() > MAX_FRAME_BYTES {
            // A line that never ends is refused as soon as it is longer than the cap.
            return Ok(Frame::TooLong);
        }
    }
    while matches!(buf.last(), Some(b'\r' | b'\n')) {
        buf.pop();
    }
    // Terminator and payload can arrive in one read, so the finished line is checked too.
    if buf.len() > MAX_FRAME_BYTES {
        return Ok(Frame::TooLong);
    }
    // A malformed UTF-8 line is a protocol problem the caller reports, not an I/O error.
    if std::str::from_utf8(&buf).is_err() {
        buf = String::from_utf8_lossy(&buf).into_owned().into_bytes();
    }
    Ok(Frame::Line(buf))
}

/// Name a JSON value's kind, for diagnostics that must not quote the child's payload.
fn json_kind(v: &serde_json::Value) -> &'static str {
    match v {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "a bool",
        serde_json::Value::Number(_) => "a number",
        serde_json::Value::String(_) => "a string",
        serde_json::Value::Array(_) => "an array",
        serde_json::Value::Object(_) => "an object",
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The handshake still reads a `String`; the frame reader hands back bytes. Both have
    /// to agree on where a line ends, or a child's first line and its tenth would be
    /// framed differently.
    #[tokio::test]
    async fn the_frame_reader_and_the_handshake_reader_agree_on_a_line() {
        let raw = b"{\"type\":\"hello\"}\r\n{\"type\":\"result\"}\n";
        let mut a = BufReader::new(std::io::Cursor::new(raw.to_vec()));
        let mut b = BufReader::new(std::io::Cursor::new(raw.to_vec()));

        for expected in ["{\"type\":\"hello\"}", "{\"type\":\"result\"}"] {
            let via_handshake = read_line_str(&mut a).await.unwrap().unwrap();
            let via_frames = match read_line_bytes(&mut b).await.unwrap() {
                Frame::Line(l) => String::from_utf8(l).unwrap(),
                other => panic!(
                    "expected a line, got {}",
                    match other {
                        Frame::Eof => "eof",
                        _ => "too long",
                    }
                ),
            };
            assert_eq!(via_handshake, expected);
            assert_eq!(via_frames, expected);
        }
        assert!(read_line_str(&mut a).await.unwrap().is_none());
        assert!(matches!(read_line_bytes(&mut b).await.unwrap(), Frame::Eof));
    }

    /// A line with no terminator at the end of the stream is still a frame: closing stdin
    /// is a legitimate way for a child to end.
    #[tokio::test]
    async fn a_final_line_without_a_terminator_is_a_frame() {
        let mut r = BufReader::new(std::io::Cursor::new(b"{\"a\":1}".to_vec()));
        match read_line_bytes(&mut r).await.unwrap() {
            Frame::Line(l) => assert_eq!(l, b"{\"a\":1}"),
            _ => panic!("expected the unterminated line"),
        }
    }

    /// The cap has to trip on total bytes, including bytes that arrive after some complete
    /// characters — so this overruns with a multi-byte character rather than with `x`.
    #[tokio::test]
    async fn a_line_past_the_cap_is_refused_rather_than_buffered() {
        let mut raw = vec![b'a'; MAX_FRAME_BYTES + 1];
        raw.push(b'\n');
        let mut r = BufReader::new(std::io::Cursor::new(raw));
        assert!(matches!(
            read_line_bytes(&mut r).await.unwrap(),
            Frame::TooLong
        ));
    }

    /// A frame exactly at the cap is accepted: the boundary must not depend on which side
    /// of it the newline falls.
    #[tokio::test]
    async fn a_line_at_the_cap_is_still_a_frame() {
        let mut raw = vec![b'a'; MAX_FRAME_BYTES];
        raw.push(b'\n');
        let mut r = BufReader::new(std::io::Cursor::new(raw));
        match read_line_bytes(&mut r).await.unwrap() {
            Frame::Line(l) => assert_eq!(l.len(), MAX_FRAME_BYTES),
            _ => panic!("a frame at the cap must be read"),
        }
    }

    /// A reader that hands over a few bytes per poll, so the frame reader has to
    /// accumulate instead of seeing the whole line at once. Real pipes behave this way,
    /// and staying inside `AsyncRead` keeps the test free of `unsafe`.
    struct Drip {
        data: Vec<u8>,
        at: usize,
        size: usize,
    }

    impl tokio::io::AsyncRead for Drip {
        fn poll_read(
            mut self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
            buf: &mut tokio::io::ReadBuf<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            let n = (self.data.len() - self.at)
                .min(buf.remaining())
                .min(self.size);
            let at = self.at;
            buf.put_slice(&self.data[at..at + n]);
            self.at += n;
            std::task::Poll::Ready(Ok(()))
        }
    }

    /// The cap must fire while the line is still arriving. A check that only runs on the
    /// finished line would have stored the whole payload first — which is the entire cost
    /// the cap exists to avoid, and unbounded when the child chooses to keep writing.
    #[tokio::test]
    async fn a_line_past_the_cap_is_refused_while_it_is_still_arriving() {
        let endless = Drip {
            data: vec![b'a'; MAX_FRAME_BYTES * 2],
            at: 0,
            size: 4096,
        };
        let mut r = BufReader::new(endless);
        assert!(
            matches!(read_line_bytes(&mut r).await.unwrap(), Frame::TooLong),
            "an endless line must be refused before it is buffered whole"
        );
    }

    /// Invalid UTF-8 is reported by the JSON parse (a protocol violation), not as an I/O
    /// error that would read as a crash.
    #[tokio::test]
    async fn invalid_utf8_becomes_a_line_for_the_json_parser_to_refuse() {
        let mut r = BufReader::new(std::io::Cursor::new(vec![0xff, 0xfe, b'\n']));
        match read_line_bytes(&mut r).await.unwrap() {
            Frame::Line(l) => {
                assert!(serde_json::from_slice::<serde_json::Value>(&l).is_err());
            }
            _ => panic!("invalid UTF-8 must still produce a line"),
        }
    }
}
