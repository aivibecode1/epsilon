//! The agent, in this window.
//!
//! # Three things live here, and the boundaries between them are the point
//!
//! 1. **[`IdeSession`]** — the harness: one `Arc<Agent>`, the approvals channel, and the
//!    launch that produced them. It is composed through `epsilon_runtime::build_runtime`,
//!    the same call the terminal makes, so a permission behaves identically in both
//!    windows because it *is* the same machinery.
//! 2. **[`Transcript`]** — a pure reducer from `AgentEvent` to rows. No agent, no window,
//!    no I/O: feed it events and it produces something a pane can draw. That is what makes
//!    it testable, and it is the same shape as the terminal's `render_rows`.
//! 3. **[`Bridge`]** — the crossing between an async broadcast and a synchronous repaint.
//!
//! # Why the crossing is a task and not a poll
//!
//! `AgentEvent` is a `tokio::broadcast::Receiver` and an egui frame is not `async`. Two
//! wrong answers are available: block the render thread on `recv().await`, which freezes the
//! window exactly as often as the model speaks; or poll a channel every frame, which wakes
//! the UI sixty times a second to discover there is nothing to draw.
//!
//! The right one is a task that owns the broadcast receiver, forwards over a standard
//! channel the frame drains without blocking, and asks for a repaint **coalesced** — a
//! streamed answer arrives as one event per delta, and one repaint per delta is the stutter
//! the terminal frontend spent M198 removing.
//!
//! # The rule this crate keeps
//!
//! Nothing here decides anything. It draws what the agent says, and it shows refusals as
//! refusals — see [`crate::IdeApp::paint`].

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use epsilon_contracts::agent_event::{AgentEvent, PermissionDecision, PermissionRequest};
use epsilon_contracts::content::ContentBlock;
use epsilon_contracts::message::Message;
use epsilon_core::Agent;
use epsilon_safety::ChannelApprovalUi;

use epsilon_runtime::Launch;

/// How long to wait before repainting after an event arrives.
///
/// One display frame. Coalescing here rather than repainting per event is the whole
/// difference between a transcript that streams and one that flickers; the terminal
/// frontend measured the same thing and bounded its drain the same way.
const REPAINT_COALESCE: Duration = Duration::from_millis(16);

/// How many changed lines one change contributes to the transcript.
const MAX_DIFF_LINES: usize = 12;

/// Rows are retained up to this count, oldest dropped.
///
/// A window is not a log file. The session file holds everything; this is a view, and a
/// view that grows without bound is a leak with a scrollbar.
const MAX_ROWS: usize = 2_000;

// ==========================================================================================
// Transcript
// ==========================================================================================

/// What a transcript row is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowKind {
    /// Something the user typed.
    User,
    /// The assistant's answer.
    Assistant,
    /// Reasoning, which the terminal draws quieter and so does this.
    Thinking,
    /// A tool call, running or finished.
    Tool {
        /// The tool's name, as the agent declared it.
        name: String,
        /// `None` while running.
        failed: Option<bool>,
    },
    /// A permission ask. Present until it is answered.
    Approval {
        /// The id to answer with.
        request_id: String,
        /// The capability being requested (`bash`, `write`, …).
        capability: String,
        /// The subject the rule matched against.
        pattern: String,
    },
    /// A harness notice, including a lost-event warning.
    Notice,
    /// A file changed, as one summary line: path, additions, removals.
    Changed,
    /// One changed line inside a `RowKind::Changed` block.
    Diff,
}

/// One drawable row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// What it is.
    pub kind: RowKind,
    /// The text to draw. Empty for a tool row that has not produced output yet.
    pub text: String,
}

/// A pure reducer from `AgentEvent` to [`Row`]s.
///
/// Nothing here can block, fail, or reach an agent — which is the property that makes the
/// interesting cases (a tool that fails, an approval that arrives mid-stream, a broadcast
/// that lagged) testable without a provider or a window.
#[derive(Debug, Default, Clone)]
pub struct Transcript {
    /// The workspace watched files are resolved against.
    ///
    /// Held here rather than passed per event because the watcher *reads files*, and a
    /// reducer that could not say where the workspace is would be one argument short of
    /// being able to review a change.
    workspace: PathBuf,
    /// Files the running tool call might change, and what they held when it started.
    watcher: crate::diff::ChangeWatcher,
    rows: Vec<Row>,
    /// Index of the assistant row currently streaming, so `MessageUpdate` replaces rather
    /// than appends. One per run: a second `MessageStart` starts a new one.
    live_assistant: Option<usize>,
    /// Index of the reasoning row currently streaming, for the same reason.
    live_thinking: Option<usize>,
    /// Index of the tool row for the call currently running.
    live_tool: Option<usize>,
    /// Requests that have been answered, so `open_approvals` stops offering them.
    ///
    /// An id set rather than a row removal, and the reason is the pair of questions
    /// above: the row answers "what did the reader see?", the set answers "what can the
    /// user still act on?". Deleting the row to answer the second would be answering it
    /// by destroying the first.
    answered: std::collections::BTreeSet<String>,
    /// How many events were dropped by a lagging broadcast. Surfaced, never swallowed —
    /// the terminal reports lag rather than skipping it, and a window that silently loses
    /// tool output is worse than one that says it did.
    dropped: u64,
}

impl Transcript {
    /// Build a transcript from messages already in history, with no events.
    ///
    /// Needed for **opening a session**: switching to a stored conversation restores its
    /// history on the agent, and nothing emits an event for it, so a window that only
    /// reduced events would open the file and show an empty pane. That is the "nothing
    /// happened" state again, arrived at by a different road — and it is the *first* thing
    /// a reader sees after choosing "resume", so it would be the first thing they judge
    /// the window on.
    ///
    /// Tool calls become their own rows and tool results their own rows, because a
    /// transcript that showed only the prose would hide half of what the agent did.
    pub fn from_messages(messages: &[Message]) -> Self {
        let mut t = Transcript::new(std::env::temp_dir());
        for message in messages {
            match message {
                Message::User(u) => {
                    let text: String = u
                        .content
                        .iter()
                        .filter_map(|c| match c {
                            epsilon_contracts::content::UserContent::Text { text } => {
                                Some(text.as_str())
                            }
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    if !text.trim().is_empty() {
                        t.push(RowKind::User, text);
                    }
                }
                Message::Assistant(a) => {
                    let mut thinking = String::new();
                    let mut answer = String::new();
                    for block in &a.content {
                        match block {
                            ContentBlock::Thinking { text, .. } => thinking.push_str(text),
                            ContentBlock::Text { text } => answer.push_str(text),
                            ContentBlock::ToolCall { name, .. } => {
                                // The result row carries the outcome; this one only has to
                                // say the call happened, or the transcript shows a tool
                                // answering a question nobody recorded asking.
                                t.push(
                                    RowKind::Tool {
                                        name: name.clone(),
                                        failed: None,
                                    },
                                    String::new(),
                                );
                            }
                            _ => {}
                        }
                    }
                    if !thinking.trim().is_empty() {
                        t.push(RowKind::Thinking, thinking);
                    }
                    if !answer.trim().is_empty() {
                        t.push(RowKind::Assistant, answer);
                    }
                }
                Message::ToolResult(r) => {
                    let text: String = r
                        .content
                        .iter()
                        .filter_map(|b| match b {
                            ContentBlock::Text { text } => Some(text.as_str()),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    // Mark the **last** tool row with this outcome rather than adding a row
                    // per result: the pair is what a reader reads, and two rows for one
                    // call makes the transcript longer without adding anything.
                    let failed = r.is_error;
                    if let Some(i) = t
                        .rows
                        .iter()
                        .rposition(|row| matches!(row.kind, RowKind::Tool { failed: None, .. }))
                    {
                        if let RowKind::Tool { failed: f, .. } = &mut t.rows[i].kind {
                            *f = Some(failed);
                        }
                        if !text.trim().is_empty() {
                            t.rows[i].text = text;
                        }
                    }
                }
                Message::Custom(_) => {}
            }
        }
        t
    }

    /// An empty transcript for `workspace`.
    pub fn new(workspace: impl Into<PathBuf>) -> Self {
        Self {
            workspace: workspace.into(),
            ..Self::default()
        }
    }

    /// The workspace this transcript watches.
    pub fn workspace(&self) -> &std::path::Path {
        &self.workspace
    }

    /// Record what a tool call changed: one summary row, then the changed lines.
    ///
    /// The lines are capped. A rewrite of a thousand-line file is not something a
    /// transcript row should carry in full — the summary already says how big it was, and
    /// the editor pane has the file itself.
    pub fn push_change(&mut self, change: &crate::diff::Change) {
        self.push(RowKind::Changed, change.summary());
        let changed: Vec<&crate::diff::DiffLine> = change
            .lines
            .iter()
            .filter(|l| !matches!(l, crate::diff::DiffLine::Context(_)))
            .collect();
        for line in changed.iter().take(MAX_DIFF_LINES) {
            self.push(RowKind::Diff, format!("{} {}", line.marker(), line.text()));
        }
        if changed.len() > MAX_DIFF_LINES {
            self.push(
                RowKind::Diff,
                format!(
                    "   … {} more changed lines — open the file",
                    changed.len() - MAX_DIFF_LINES
                ),
            );
        }
    }

    /// The rows, oldest first.
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    /// How many events a lagging broadcast dropped.
    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    /// The permission asks that have not been answered, oldest first.
    ///
    /// Answered asks are skipped even though their rows are still on screen: this answers
    /// *"can the user act on this?"*, and offering a resolved ask a second time is how a
    /// permission gets applied twice.
    pub fn open_approvals(&self) -> Vec<(&str, &str, &str)> {
        self.rows
            .iter()
            .filter_map(|r| match &r.kind {
                RowKind::Approval {
                    request_id,
                    capability,
                    pattern,
                } if !self.answered.contains(request_id) => {
                    Some((request_id.as_str(), capability.as_str(), pattern.as_str()))
                }
                _ => None,
            })
            .collect()
    }

    /// Add a harness notice the reader should see.
    pub fn push_notice(&mut self, text: &str) {
        self.push(RowKind::Notice, text.to_string());
    }

    /// Add a row the user produced themselves.
    ///
    /// The prompt is an **input**, so no `AgentEvent` carries it — and a frontend that
    /// waits for an event shows an empty transcript until the model answers, which reads
    /// as "nothing happened" the moment Enter is pressed.
    pub fn push_user(&mut self, text: &str) {
        self.push(RowKind::User, text.to_string());
    }

    /// Fold one event in.
    pub fn apply(&mut self, ev: &AgentEvent) {
        match ev {
            AgentEvent::Notice { text } => self.push(RowKind::Notice, text.clone()),

            AgentEvent::MessageStart { message } => self.open_live_rows(message),

            AgentEvent::MessageUpdate { message } => {
                // **`MessageUpdate` opens a row as well as updating one.** The loop does
                // not always emit `MessageStart` first — the offline provider goes
                // straight to `MessageUpdate`, and the very first end-to-end run of this
                // window produced a transcript with *zero* rows for exactly that reason:
                // nothing opened a row, so every update had nowhere to land.
                //
                // Depending on a "start" event that the producer may omit is the same
                // class of assumption as depending on a key that nothing binds: the
                // reducer now opens on whichever of the two arrives first, which is also
                // what a reader would expect of a stream.
                if self.live_assistant.is_none() && self.live_thinking.is_none() {
                    self.open_live_rows(message);
                }
                if let Some(i) = self.live_assistant
                    && let Some(text) = message_text(message)
                {
                    self.rows[i].text = text;
                }
                if let Some(i) = self.live_thinking
                    && let Some(text) = message_thinking(message)
                {
                    self.rows[i].text = text;
                }
            }

            AgentEvent::MessageEnd { message } => {
                // **`MessageEnd` opens a row as well as updating one.** It updates only
                // a row that `MessageStart` or `MessageUpdate` already opened, so a
                // provider that emits the completed message and nothing before it
                // produced **no transcript row at all** — the agent answered and the
                // window stayed empty. Found by the markdown frame test, which seeded
                // exactly that event shape.
                if self.live_assistant.is_none() && self.live_thinking.is_none() {
                    self.open_live_rows(message);
                }
                if let Some(i) = self.live_assistant
                    && let Some(text) = message_text(message)
                {
                    self.rows[i].text = text;
                }
                if let Some(i) = self.live_thinking
                    && let Some(text) = message_thinking(message)
                {
                    self.rows[i].text = text;
                }
                self.live_assistant = None;
                self.live_thinking = None;
            }

            AgentEvent::ToolExecutionStart {
                tool_name, args, ..
            } => {
                // **Snapshot before the tool runs.** `ToolExecutionStart` is emitted
                // before execution, so reading now is genuinely "before" — reading after
                // would produce an empty diff for every edit, which is the failure mode
                // of a review pane that silently shows nothing.
                self.watcher.begin(args, &self.workspace);
                let row = Row {
                    kind: RowKind::Tool {
                        name: tool_name.clone(),
                        failed: None,
                    },
                    // The arguments, not the result: what the agent is *about to do* is
                    // the thing a reader needs while it is doing it.
                    text: one_line(args),
                };
                self.live_tool = Some(self.push_row(row));
            }

            AgentEvent::ToolExecutionEnd { is_error, .. } => {
                if let Some(i) = self.live_tool.take()
                    && let RowKind::Tool { failed, .. } = &mut self.rows[i].kind
                {
                    *failed = Some(*is_error);
                }
                // What actually changed on disk, which is the only version worth
                // reviewing. A call that changed nothing produces **no row**: a review
                // pane full of "no changes" is one nobody reads.
                let changes = self.watcher.finish(&self.workspace);
                for change in &changes {
                    self.push_change(change);
                }
            }

            AgentEvent::PermissionRequested { request } => {
                self.push_row(Row {
                    kind: RowKind::Approval {
                        request_id: request.request_id.clone(),
                        capability: request.capability.clone(),
                        pattern: request.pattern.clone(),
                    },
                    text: format!("{} {}", request.capability, request.pattern),
                });
            }

            AgentEvent::PermissionDecided {
                request_id,
                decision,
            } => {
                let outcome = match decision {
                    PermissionDecision::AllowOnce => "allowed",
                    PermissionDecision::AlwaysSession => "allowed for this session",
                    PermissionDecision::Deny => "refused",
                    PermissionDecision::DenyWithGuidance { .. } => "refused with guidance",
                };
                // The ask keeps its row and is marked by what follows, rather than being
                // removed: a reader who scrolled past a prompt should be able to see that
                // it was answered, and by whom. **Keeping it is not the same as leaving it
                // open** — `open_approvals` skips answered ids, because the two questions
                // are different: "is it on screen?" and "can the user still act on it?".
                // Answering the second by deleting the row would destroy the first.
                self.answered.insert(request_id.clone());
                if let Some(i) = self
                    .rows
                    .iter()
                    .position(|r| matches!(&r.kind, RowKind::Approval { request_id: id, .. } if id == request_id))
                {
                    self.rows.insert(i + 1, Row {
                        kind: RowKind::Notice,
                        text: format!("   {outcome}"),
                    });
                }
            }

            _ => {}
        }
        self.trim();
    }

    /// Record that a lagging broadcast dropped `n` events, and say so on screen.
    pub fn note_lag(&mut self, n: u64) {
        self.dropped += n;
        self.push(
            RowKind::Notice,
            format!(
                "{n} event(s) dropped while the window was busy — the transcript may be incomplete"
            ),
        );
    }

    /// Add a row and return its index.
    fn push_row(&mut self, row: Row) -> usize {
        self.rows.push(row);
        self.rows.len() - 1
    }

    fn push(&mut self, kind: RowKind, text: String) {
        self.push_row(Row { kind, text });
    }

    /// Open the live rows for `message`, if it carries anything worth a row.
    fn open_live_rows(&mut self, message: &Message) {
        if let Some(i) = self.push_assistant(message) {
            self.live_assistant = Some(i);
        }
        if let Some(i) = self.push_thinking(message) {
            self.live_thinking = Some(i);
        }
    }

    /// Start an assistant row for `message`, if it carries answer text.
    ///
    /// Returns `None` for a message with no text — a tool-only assistant message, or one
    /// that is still empty. Pushing an empty row for those would draw a blank line the
    /// reader has to interpret.
    fn push_assistant(&mut self, message: &Message) -> Option<usize> {
        let text = message_text(message).filter(|t| !t.trim().is_empty())?;
        Some(self.push_row(Row {
            kind: RowKind::Assistant,
            text,
        }))
    }

    /// Start a reasoning row for `message`, if it carries reasoning text.
    fn push_thinking(&mut self, message: &Message) -> Option<usize> {
        let text = message_thinking(message).filter(|t| !t.trim().is_empty())?;
        Some(self.push_row(Row {
            kind: RowKind::Thinking,
            text,
        }))
    }

    /// Drop the oldest rows past [`MAX_ROWS`].
    ///
    /// In one `drain`, not per row: a flood of events would otherwise be O(n) per event.
    fn trim(&mut self) {
        if self.rows.len() > MAX_ROWS {
            self.rows.drain(..self.rows.len() - MAX_ROWS);
            // The live indices are absolute, so a drain invalidates them. Losing the
            // handle means the next update appends instead of replacing, which is wrong
            // but visible; keeping a stale index would write into the wrong row, which is
            // wrong and invisible.
            self.live_assistant = None;
            self.live_thinking = None;
            self.live_tool = None;
        }
    }
}

/// The answer text of an assistant message, if it has any.
fn message_text(message: &Message) -> Option<String> {
    let Message::Assistant(a) = message else {
        return None;
    };
    let joined: String = a
        .content
        .iter()
        .filter_map(|b| match b {
            epsilon_contracts::content::ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .concat();
    (!joined.is_empty()).then_some(joined)
}

/// The reasoning text of an assistant message, if it has any.
fn message_thinking(message: &Message) -> Option<String> {
    let Message::Assistant(a) = message else {
        return None;
    };
    let joined: String = a
        .content
        .iter()
        .filter_map(|b| match b {
            epsilon_contracts::content::ContentBlock::Thinking { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .concat();
    (!joined.is_empty()).then_some(joined)
}

/// A tool's arguments on one line, so a row is one row.
fn one_line(v: &serde_json::Value) -> String {
    let joined = v.to_string().replace(['\n', '\r'], " ");
    if joined.chars().count() <= 160 {
        joined
    } else {
        // Cut on a character boundary and say that it was cut. A truncated JSON fragment
        // with no marker reads as the whole call, which is how a reader ends up believing
        // something the agent never asked for.
        let head: String = joined.chars().take(157).collect();
        format!("{head}… (truncated)")
    }
}

// ==========================================================================================
// Session
// ==========================================================================================

/// The agent this window is driving.
pub struct IdeSession {
    /// The one agent. Shared with the runtime, never rebuilt.
    agent: Arc<Agent>,
    /// The approvals channel. The terminal uses the same type.
    approvals: Arc<ChannelApprovalUi>,
    /// What the launch resolved to — the source of every number the status line shows.
    launch: Launch,
    /// The tokio runtime the agent runs on. Owned, because `run` needs it alive for as
    /// long as the window is, and because a second runtime would mean a second thread
    /// pool the agent and the UI do not share.
    runtime: Arc<tokio::runtime::Runtime>,
    /// Where the bridge forwards events. Kept because [`IdeSession::attach_bridge`] needs
    /// it after construction, and the receiver below is the only other half.
    forwarder_tx: std::sync::mpsc::Sender<Arc<AgentEvent>>,
    /// Events forwarded from the broadcast.
    ///
    /// Behind a mutex rather than owned outright, so [IdeSession::drain] can take
    /// &self — see its doc for why that matters.
    inbox: Mutex<Receiver<Arc<AgentEvent>>>,
    /// A failure a background run reported, waiting for the next frame to say it.
    ///
    /// A mutex rather than a channel because there is exactly one value, it is written
    /// once and taken once, and a second queue would be a second thing to drain.
    /// The failure slot, shared with the spawned run so it survives the call that made it.
    failure_slot: Arc<Mutex<Option<String>>>,
    /// Whether a run is in flight.
    running: AtomicBool,
}

impl IdeSession {
    /// Compose an agent for `launch` on `runtime` and start listening to it.
    /// Switch this session to a stored conversation.
    ///
    /// Returns the message count that was restored, or the reason it could not be. The
    /// refusal is **returned, not swallowed**: a click that appears to do nothing is the
    /// state this crate keeps failing to rule out, and a session file held by another
    /// process is an ordinary thing to hit.
    pub fn resume(&self, path: &std::path::Path) -> Result<usize, String> {
        self.agent.live_resume(path)
    }

    /// Every session stored under the workspace for `cwd`, newest first.
    pub fn sessions(&self, cwd: &std::path::Path) -> Vec<epsilon_session::SessionSummary> {
        epsilon_session::list_sessions_for_cwd(self.agent.workspace_root(), cwd)
    }

    /// The session file this agent currently holds.
    pub fn session_path(&self) -> Option<PathBuf> {
        self.agent.session_path()
    }

    /// Compose an agent for `launch` on `runtime` and start listening to it.
    ///
    /// Called before the window exists: nothing here needs the render thread, and building
    /// an agent *after* the window opens would mean a window with nothing in it for as
    /// long as the extension scan takes.
    pub async fn start(
        runtime: Arc<tokio::runtime::Runtime>,
        launch: Launch,
    ) -> Result<Self, String> {
        let approvals = Arc::new(ChannelApprovalUi::new());
        let agent = epsilon_runtime::build_runtime(epsilon_runtime::RunParams {
            workspace: &launch.workspace,
            stream_fn: launch.stream_fn.clone(),
            approvals: approvals.clone(),
            mcp_servers: &launch.mcp_servers,
            session_file: epsilon_runtime::fresh_session_file(
                epsilon_runtime::now_millis(),
                &epsilon_contracts::new_id("ses"),
                &launch.workspace,
            ),
            confinement: &launch.confinement,
            lsp: launch.lsp,
            model_id: &launch.model_id,
            model_explicit: launch.model_explicit,
            resume: &launch.resume,
            project: &launch.project,
            project_trusted: launch.project_trusted,
            plan_mode: launch.plan_mode,
        })
        .await?;

        let (forwarder_tx, inbox) = std::sync::mpsc::channel();
        Ok(Self {
            agent,
            approvals,
            launch,
            runtime,
            forwarder_tx,
            inbox: std::sync::Mutex::new(inbox),
            failure_slot: Arc::new(Mutex::new(None)),
            running: AtomicBool::new(false),
        })
    }

    /// The agent, for a caller that wants to run a turn.
    pub fn agent(&self) -> &Arc<Agent> {
        &self.agent
    }

    /// What the launch resolved to.
    pub fn launch(&self) -> &Launch {
        &self.launch
    }

    /// The approvals channel, so a pane can resolve a parked ask by id.
    pub fn approvals(&self) -> &Arc<ChannelApprovalUi> {
        &self.approvals
    }

    /// Whether a run is in flight.
    pub fn running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }

    /// Take everything the bridge has forwarded since the last call, folding it into
    /// `transcript`.
    ///
    /// Never blocks and never fails: a frame that could not be drawn is a frozen window,
    /// and a queue this returns is the only thing that can stop it. The bound is the point
    /// — a frame that processes an unbounded backlog is a frame that never ends, which is
    /// the freeze this crate is most exposed to.
    ///
    /// **Takes `&self`, and that is not an oversight.** The first version took `&mut self`
    /// and the window reached it through `Arc::get_mut`, which returns `None` the moment
    /// anything else holds a handle — and a frame that quietly stops draining is a
    /// transcript that silently stops updating while every test still passes. The queue and
    /// the flag are behind a mutex and an atomic instead, so a shortcut, a test, or a
    /// second window can hold the session and the drawing still works.
    pub fn drain(&self, transcript: &mut Transcript) -> usize {
        // A run that was refused says so before anything else: it is the newest fact, and
        // a frame that draws the transcript without it would leave a user who pressed
        // Enter staring at an unchanged screen.
        if let Some(failure) = self.take_failure() {
            transcript.apply(&AgentEvent::Notice { text: failure });
        }
        // Take the receiver's lock for the length of the drain and no longer. A poisoned
        // lock here would mean a frame panicked while holding it; there is nothing to
        // recover and nothing to say, so it yields the rest of the queue rather than
        // spinning on a poisoned mutex.
        let Ok(inbox) = self.inbox.lock() else {
            return 0;
        };
        let mut taken = 0;
        loop {
            match inbox.try_recv() {
                Ok(ev) => {
                    match &*ev {
                        AgentEvent::AgentStart => self.running.store(true, Ordering::Relaxed),
                        AgentEvent::AgentEnd { .. } => self.running.store(false, Ordering::Relaxed),
                        _ => {}
                    }
                    transcript.apply(&ev);
                    taken += 1;
                    if taken >= MAX_EVENTS_PER_FRAME {
                        return taken;
                    }
                }
                Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => return taken,
            }
        }
    }

    /// Ask the agent to run `message`.
    ///
    /// `text` goes through `epsilon_core::references::user_message`, the same path every
    /// other frontend uses, so `@file` means the same thing here as it does in `epsilon`.
    pub fn submit(&self, text: &str) -> Result<(), String> {
        if self.running() {
            // The loop is single-flight (`Agent::run` refuses a second run), and the
            // refusal would arrive as an error the user never asked for. Saying so at the
            // point of the click is the honest version.
            return Err("a run is already in flight — wait for it, or press Esc to stop it".into());
        }
        let message = epsilon_core::references::user_message(&self.launch.workspace, text);
        let agent = self.agent.clone();
        // Cloned, not borrowed: the spawned task outlives this call, and a `spawn` that
        // could not outlive the frame is a task that never runs.
        let failure = self.last_error_shared();
        self.runtime.spawn(async move {
            // A refused run produces no `AgentEnd`, so `running` would stay true and the
            // composer would stay locked. The reason is handed back through a slot the
            // next frame drains, rather than being dropped on a task nobody reads.
            if let Err(e) = agent.run(message).await
                && let Ok(mut slot) = failure.lock()
            {
                *slot = Some(format!("run refused: {e}"));
            }
        });
        Ok(())
    }

    /// The failure slot as something a spawned task can hold.
    fn last_error_shared(&self) -> Arc<Mutex<Option<String>>> {
        self.failure_slot.clone()
    }

    /// Take any failure a background run reported, so the frame can say it out loud.
    fn take_failure(&self) -> Option<String> {
        self.failure_slot
            .lock()
            .ok()
            .and_then(|mut slot| slot.take())
    }

    /// Stop the run in flight, if any.
    pub fn abort(&self) -> bool {
        self.agent.abort_current_run()
    }

    /// Start listening, forwarding events to this session's inbox.
    ///
    /// Called once the egui context exists, because asking for a repaint is the one thing
    /// in this path that has to reach the render thread.
    pub fn attach_bridge(&self, ctx: &egui::Context) {
        let mut events = self.agent.subscribe();
        let tx = self.forwarder_tx.clone();
        let ctx = ctx.clone();
        self.runtime.spawn(async move {
            loop {
                match events.recv().await {
                    Ok(ev) => {
                        if tx.send(ev).is_err() {
                            // The window is gone; nothing left to forward to.
                            break;
                        }
                        // Coalesced: a streamed answer is one event per delta, and one
                        // repaint per delta is the stutter M198 spent a milestone on.
                        ctx.request_repaint_after(REPAINT_COALESCE);
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        // Forwarded as a marker rather than dropped: the transcript is a
                        // view of the run, and a view with a silent hole in it is worse
                        // than one that says it has one.
                        if tx
                            .send(Arc::new(AgentEvent::Notice {
                                text: format!(
                                    "{n} event(s) dropped — the window could not keep up"
                                ),
                            }))
                            .is_err()
                        {
                            break;
                        }
                        ctx.request_repaint_after(REPAINT_COALESCE);
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }

    /// Answer a parked permission ask.
    ///
    /// Returns `false` when the id is unknown or already resolved — the same refusal the
    /// HTTP surface gives, so a double-click cannot answer twice.
    pub fn resolve(&self, request_id: &str, decision: PermissionDecision) -> bool {
        self.approvals.resolve(request_id, decision)
    }

    /// The permission asks waiting for an answer, oldest first.
    pub fn pending_approvals(&self) -> Vec<PermissionRequest> {
        self.approvals.pending()
    }
}

/// How many events one frame will fold before returning to the render loop.
///
/// Sized so a long backlog drains in a few frames rather than one, and so no single frame
/// can be spent entirely catching up — the failure mode is a window that stops responding
/// while it is technically busy.
const MAX_EVENTS_PER_FRAME: usize = 512;

#[cfg(test)]
mod tests {
    use super::*;
    use epsilon_contracts::content::ContentBlock;
    use epsilon_contracts::message::{AssistantMessage, StopReason};

    /// A transcript over a scratch workspace.
    ///
    /// The reducer tests feed synthetic events, so nothing is read; the path is here
    /// because the change watcher resolves files against one and the constructor asks.
    fn transcript() -> Transcript {
        Transcript::new(std::env::temp_dir())
    }

    fn assistant(text: &str) -> Message {
        Message::Assistant(AssistantMessage {
            id: "m1".into(),
            timestamp_ms: 0,
            provider: "faux".into(),
            model_id: "faux-1".into(),
            content: vec![ContentBlock::text(text)],
            stop_reason: StopReason::EndTurn,
            error_message: None,
            usage: Default::default(),
        })
    }

    /// A streamed answer must **replace** its row, not append one per delta.
    ///
    /// This is the property that decides whether the transcript is readable: an
    /// implementation that pushes on `MessageUpdate` produces one row per token, which is
    /// the shape of the defect the terminal frontend calls a stutter.
    ///
    /// The row opens on whichever of `MessageStart` / `MessageUpdate` arrives first — see
    /// the reducer — so this drives the update-only shape, which is what the offline
    /// provider actually produces.
    #[test]
    fn a_streamed_answer_grows_one_row_rather_than_one_row_per_delta() {
        for opening in [
            |t: &mut Transcript| {
                t.apply(&AgentEvent::MessageStart {
                    message: Box::new(assistant("")),
                })
            },
            |_t: &mut Transcript| {},
        ] {
            let mut t = transcript();
            opening(&mut t);
            for text in ["H", "He", "Hel", "Hell", "Hello"] {
                t.apply(&AgentEvent::MessageUpdate {
                    message: Box::new(assistant(text)),
                });
            }
            let assistant_rows: Vec<_> = t
                .rows()
                .iter()
                .filter(|r| r.kind == RowKind::Assistant)
                .collect();
            assert_eq!(
                assistant_rows.len(),
                1,
                "one row, not one per delta: {:?}",
                t.rows()
            );
            assert_eq!(assistant_rows[0].text, "Hello");
        }
    }

    /// A `MessageStart` carrying no text creates no row.
    ///
    /// The converse of the test above, and the reason the reducer filters on non-empty
    /// text: a tool-only assistant message has no words, and a blank row is a mark the
    /// reader has to decode.
    #[test]
    fn a_message_with_no_text_creates_no_row() {
        let mut t = transcript();
        t.apply(&AgentEvent::MessageStart {
            message: Box::new(assistant("   ")),
        });
        assert!(
            t.rows().is_empty(),
            "whitespace is not an answer: {:?}",
            t.rows()
        );
    }

    #[test]
    fn a_tool_call_shows_what_it_is_doing_and_how_it_ended() {
        let mut t = transcript();
        t.apply(&AgentEvent::ToolExecutionStart {
            tool_call_id: "c1".into(),
            tool_name: "bash".into(),
            args: serde_json::json!({"command": "cargo test"}),
        });
        let running = t.rows().first().expect("a tool row");
        assert_eq!(
            running.kind,
            RowKind::Tool {
                name: "bash".into(),
                failed: None
            }
        );
        assert!(
            running.text.contains("cargo test"),
            "the arguments are what a reader needs while it runs: {:?}",
            running.text
        );

        t.apply(&AgentEvent::ToolExecutionEnd {
            tool_call_id: "c1".into(),
            tool_name: "bash".into(),
            is_error: true,
        });
        let ended = t.rows().first().expect("the tool row");
        assert_eq!(
            ended.kind,
            RowKind::Tool {
                name: "bash".into(),
                failed: Some(true)
            }
        );
    }

    #[test]
    fn a_long_tool_call_is_marked_truncated_rather_than_silently_cut() {
        let long = "x".repeat(400);
        let mut t = transcript();
        t.apply(&AgentEvent::ToolExecutionStart {
            tool_call_id: "c1".into(),
            tool_name: "read".into(),
            args: serde_json::json!({ "path": long }),
        });
        let text = &t.rows()[0].text;
        assert!(
            text.ends_with("(truncated)"),
            "an unmarked cut reads as the whole call: {text}"
        );
        assert!(text.chars().count() < 200, "and it is actually shorter");
    }

    /// The ask keeps its row and gains an outcome line. Removing it would mean a reader
    /// who scrolled past a permission prompt could not tell whether it was ever answered.
    #[test]
    fn an_answered_approval_keeps_the_ask_and_records_the_outcome() {
        let mut t = transcript();
        let request = PermissionRequest {
            request_id: "perm-1".into(),
            capability: "bash".into(),
            pattern: "git push".into(),
            detail: serde_json::json!({}),
        };
        t.apply(&AgentEvent::PermissionRequested {
            request: request.clone(),
        });
        assert_eq!(t.open_approvals(), vec![("perm-1", "bash", "git push")]);

        t.apply(&AgentEvent::PermissionDecided {
            request_id: "perm-1".into(),
            decision: PermissionDecision::AllowOnce,
        });
        assert!(
            t.open_approvals().is_empty(),
            "an answered ask is no longer open"
        );
        assert!(
            t.rows().iter().any(|r| r.text.contains("allowed")),
            "and the outcome is on screen: {:?}",
            t.rows()
        );
        assert!(
            t.rows().iter().any(|r| r.text.contains("git push")),
            "the ask itself is still there"
        );
    }

    /// A refusal is a first-class outcome, not an absence. `DenyWithGuidance` in
    /// particular carries text the user wrote, and it must be visible that they wrote it.
    #[test]
    fn a_refusal_is_shown_as_a_refusal() {
        let mut t = transcript();
        t.apply(&AgentEvent::PermissionDecided {
            request_id: "perm-x".into(),
            decision: PermissionDecision::DenyWithGuidance {
                guidance: "use the test helper".into(),
            },
        });
        // No matching ask, so nothing is inserted -- and that is the honest outcome:
        // inventing an ask the user never saw would be worse.
        assert!(t.rows().is_empty(), "{:?}", t.rows());
    }

    #[test]
    fn a_lagging_broadcast_is_reported_and_counted_not_swallowed() {
        let mut t = transcript();
        t.note_lag(12);
        assert_eq!(t.dropped(), 12);
        assert!(
            t.rows().iter().any(|r| r.text.contains("12")),
            "the reader is told the transcript has a hole: {:?}",
            t.rows()
        );
    }

    #[test]
    fn the_transcript_is_bounded_and_the_trim_drops_the_oldest() {
        let mut t = transcript();
        for i in 0..(MAX_ROWS + 50) {
            t.apply(&AgentEvent::Notice {
                text: format!("row {i}"),
            });
        }
        assert_eq!(t.rows().len(), MAX_ROWS);
        assert_eq!(
            t.rows().last().map(|r| r.text.as_str()),
            Some(format!("row {}", MAX_ROWS + 49).as_str()),
            "the newest row survives"
        );
        assert!(
            t.rows()[0].text != "row 0",
            "the oldest is what goes: {:?}",
            t.rows()[0]
        );
    }

    #[test]
    fn a_completed_message_with_nothing_before_it_still_reaches_the_transcript() {
        // `MessageEnd` alone. A provider that reports only the finished message must
        // still produce a row -- otherwise the agent answers and the window is empty,
        // which is the state this transcript exists to rule out. Found by the markdown
        // frame test, which seeded exactly that event shape.
        let mut t = transcript();
        t.apply(&AgentEvent::MessageEnd {
            message: Box::new(assistant("the whole answer, at once")),
        });
        let rows: Vec<&Row> = t
            .rows()
            .iter()
            .filter(|r| r.kind == RowKind::Assistant)
            .collect();
        assert_eq!(rows.len(), 1, "{:?}", t.rows());
        assert_eq!(rows[0].text, "the whole answer, at once");
    }

    #[test]
    fn reasoning_and_the_answer_are_separate_rows() {
        let mut t = transcript();
        let mut m = assistant("the answer");
        if let Message::Assistant(a) = &mut m {
            a.content.insert(
                0,
                ContentBlock::Thinking {
                    text: "let me check".into(),
                    signature: None,
                },
            );
        }
        t.apply(&AgentEvent::MessageStart {
            message: Box::new(m),
        });
        let kinds: Vec<&RowKind> = t.rows().iter().map(|r| &r.kind).collect();
        assert!(
            kinds.contains(&&RowKind::Thinking),
            "reasoning is background material and draws as its own row: {kinds:?}"
        );
        assert!(
            kinds.contains(&&RowKind::Assistant),
            "and the answer is separate: {kinds:?}"
        );
    }

    #[test]
    fn a_newline_in_tool_arguments_does_not_make_a_row_break_the_pane() {
        let mut t = transcript();
        t.apply(&AgentEvent::ToolExecutionStart {
            tool_call_id: "c1".into(),
            tool_name: "bash".into(),
            args: serde_json::json!({ "command": "a\nb\rc" }),
        });
        let text = &t.rows()[0].text;
        assert!(!text.contains('\n') && !text.contains('\r'), "{text:?}");
    }
}
