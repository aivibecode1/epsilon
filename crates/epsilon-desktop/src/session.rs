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
use epsilon_contracts::message::{COMPACTION_MARKER, Message, StopReason, Usage};
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

/// How many lines of a tool's output the pane shows unexpanded.
///
/// The ROW keeps everything (full-port round 54): `/expand` shows it all,
/// like the terminal's cards. The display caps at this number with a
/// marker, so a long `cargo build` does not pin an unscrollable wall —
/// the terminal keeps 40 display lines for the same reason
/// (`CARD_STREAM_TAIL_LINES`); one number, two frontends. Retaining full
/// output per row is the same trade the terminal makes (rows are capped
/// at [`MAX_ROWS`]; bytes are not).
pub(crate) const TOOL_OUTPUT_TAIL_LINES: usize = 40;

/// Append `chunk` to `existing`, keeping everything.
///
/// One function for the live path and the replay path, because the two
/// answering this differently is what made a replayed session read
/// differently from a live one. The bound moved to the draw: rows retain,
/// the pane displays a tail until `/expand`.
fn append_output(existing: &str, chunk: &str) -> String {
    let mut lines: Vec<&str> = existing.lines().collect();
    lines.extend(chunk.lines().filter(|l| !l.trim().is_empty()));
    lines.join("\n")
}

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
        /// The agent's id for this invocation.
        ///
        /// It is what streamed output is matched on, and the transcript matched *positionally*
        /// before: "the last tool row that has not finished". That is wrong the moment two
        /// tools run at once, which the FIFO approval channel permits — a `bash` waiting on a
        /// permission ask is exactly when a second call arrives. The id was already in hand
        /// (`ToolCall.id`, `ToolResultMessage.tool_call_id`); it simply was not kept.
        id: String,
        /// The tool's name, as the agent declared it.
        name: String,
        /// `None` while running.
        failed: Option<bool>,
        /// The tail of what the tool printed while it ran.
        ///
        /// The window used to show `▸ bash {"command": …}` and then `✓`, and never the output
        /// between them — while the *resume* path showed results, so the same session showed
        /// two different transcripts before and after a session switch. An IDE that cannot
        /// show you what `cargo test` printed is not yet an IDE.
        ///
        /// A **bounded tail**, because `partial` is a delta and a runaway command would
        /// otherwise pin its whole output in a row nobody can scroll (the terminal keeps 40
        /// lines for the same reason, `CARD_STREAM_TAIL_LINES`).
        output: String,
        /// The call's arguments, kept so the row title is tool-owned: the
        /// same `call_head` the terminal draws, so both front ends name a
        /// call the same way (C-120, two views into one agent).
        args: serde_json::Value,
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
    /// A failure the harness is reporting — the `SystemErr` row's window half.
    ///
    /// **The severity dimension the window's notices did not have.** Every notice
    /// drew `theme.dim` with a blank marker, so `error: 500 from provider` and
    /// `favorited deepseek-flash` were the same quietness — the terminal had the
    /// same defect (its `System` rows shared a bullet and `theme.faint()`) and
    /// fixed it at round 97 with a dedicated row kind. The window now has its
    /// own: a failure carries the cross the failed tool row already owns — same
    /// glyph, same meaning — and the danger colour.
    ///
    /// A variant rather than a flag on `Notice` for the same reason the terminal
    /// gives: "how a row looks" is `RowKind` already (the marker table and the
    /// accessible name both switch on it), and a second column beside it would be
    /// a second thing those functions consult.
    NoticeFail,
    /// A compaction digest: summarizer-model prose about compacted-away context.
    ///
    /// A variant rather than text inside the cost `Notice` for the reason the draw
    /// path states: only model prose is interpreted as markdown, and the cost line
    /// is harness text. One row would force the summary through the notice's
    /// plain-text path or the cost line through the markdown parser — both
    /// misrender one half. The terminal's equivalent is the expandable event row
    /// (cost head, digest body); the window has no collapsed rows, so both halves
    /// draw.
    Digest,
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
    /// Requests that have been answered, so `open_approvals` stops offering them.
    ///
    /// An id set rather than a row removal, and the reason is the pair of questions
    /// above: the row answers "what did the reader see?", the set answers "what can the
    /// user still act on?". Deleting the row to answer the second would be answering it
    /// by destroying the first.
    answered: std::collections::BTreeSet<String>,
    /// Bumped once per recorded change, and the workspace paths they touched.
    ///
    /// A frontend shows a *snapshot*: a file tree walked at launch and an editor holding
    /// whatever the file said when it was opened. An agent that writes a file therefore
    /// changes the world without changing the window, and the reader is left looking at a
    /// repository that no longer exists — including a file the agent has just edited,
    /// still showing the text it replaced. This is the signal that says "your view is
    /// stale" without the frontend having to diff the workspace itself.
    changes: Vec<String>,
    /// The changes themselves, newest last, so a reader can be shown what the agent did.
    ///
    /// The rows a change produces are enough to *read about* and not enough to *review*:
    /// they are capped, interleaved with everything else, and gone the moment the
    /// transcript trims. A diff that only exists as prose in a scrollback is a report, not
    /// a review.
    ///
    /// Bounded because this is a review surface, not an archive — the session log is the
    /// archive, and it already holds every one of these.
    diffs: Vec<crate::diff::Change>,
}

impl Transcript {
    /// The retained changes, oldest first.
    pub fn diffs(&self) -> &[crate::diff::Change] {
        &self.diffs
    }

    /// How many changes are kept for review.
    ///
    /// Chosen against the transcript's own `MAX_ROWS`: a review list longer than the
    /// conversation that produced it is not a list anyone scrolls.
    const MAX_DIFFS: usize = 200;
    /// Forget which paths have been reported, so the next caller sees only new ones.
    ///
    /// Taking rather than reading, so a frontend that does not act on a change cannot leave
    /// a growing list behind it.
    pub fn take_changed_paths(&mut self) -> Vec<String> {
        std::mem::take(&mut self.changes)
    }
}

impl Transcript {
    /// Build a transcript from messages already in history, with no events.
    ///
    /// Rebuild from an agent's live history, reattaching its compaction digests.
    ///
    /// One call for every path that resumes — session switches, timeline jumps — so
    /// the surfaces cannot drift apart. No store, a locked file, or a failed read
    /// leaves the markers plain (today's behavior) rather than failing a resume that
    /// already hydrated: a digest no action can follow is not worth noise (the P-055
    /// rule, one function over).
    pub fn from_history(agent: &Agent, history: &[Message]) -> Self {
        let mut digests = Vec::new();
        if let Some(store) = agent.session_store()
            && let Ok(st) = store.lock()
            && let Ok(read) = st.compaction_digests()
        {
            digests = read;
        }
        Self::from_messages_with_digests(history, &digests)
    }

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
        Self::from_messages_with_digests(messages, &[])
    }

    /// Rebuild from replayed messages, reattaching compaction digests (P-054/P-183,
    /// window replay half).
    ///
    /// Each `[context compacted]` marker row takes the next digest, oldest first, and
    /// becomes the same cost-plus-summary notice the live arm draws — one builder,
    /// both paths. A marker with no digest stays the user row hydration made it:
    /// inventing a summary is worse than a strange-looking user line. Digests come
    /// from the store's `compaction_digests` (branch order, so digest `i` meets
    /// marker `i`); [`Transcript::from_history`] reads them beside the messages.
    pub fn from_messages_with_digests(
        messages: &[Message],
        digests: &[epsilon_session::CompactionDigest],
    ) -> Self {
        let mut t = Transcript::new(std::env::temp_dir());
        let mut digests = digests.iter();
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
                        if text == COMPACTION_MARKER {
                            match digests.next() {
                                Some(d) => {
                                    t.push_compaction_digest(d.tokens_before, d.usage, &d.summary)
                                }
                                None => t.push(RowKind::User, text),
                            }
                        } else {
                            t.push(RowKind::User, text);
                        }
                    }
                }
                Message::Assistant(a) => {
                    let mut thinking = String::new();
                    let mut answer = String::new();
                    for block in &a.content {
                        match block {
                            ContentBlock::Thinking { text, .. } => thinking.push_str(text),
                            ContentBlock::Text { text } => answer.push_str(text),
                            ContentBlock::ToolCall {
                                id,
                                name,
                                arguments,
                            } => {
                                // The result row carries the outcome; this one only has to
                                // say the call happened, or the transcript shows a tool
                                // answering a question nobody recorded asking.
                                // Tool-owned title via the shared `call_head`
                                // (C-120): both front ends name the call alike.
                                // Capped like the old JSON formatter was: a
                                // one-line row must say when it was cut.
                                let head = epsilon_tui::renderers::call_head(name, arguments, "…");
                                t.push(
                                    RowKind::Tool {
                                        id: id.to_string(),
                                        name: name.clone(),
                                        failed: None,
                                        output: String::new(),
                                        args: arguments.clone(),
                                    },
                                    cap_title(head),
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
                    if a.stop_reason == StopReason::MaxTokens {
                        // P-127, window half, history path: a replayed reply that
                        // hit the token limit says so — same words, same
                        // condition as the terminal.
                        t.push(
                            RowKind::Notice,
                            "Response was truncated before completion.".to_string(),
                        );
                    }
                    if a.stop_reason == StopReason::Aborted {
                        // C-110, window half, history path: a replayed reply the
                        // reader stopped says so too, or a resumed half answer
                        // reads whole.
                        t.push(
                            RowKind::Notice,
                            "Response was stopped before completion.".to_string(),
                        );
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
                    // Mark the tool row this result belongs to rather than adding a row
                    // per result: the pair is what a reader reads, and two rows for one
                    // call makes the transcript longer without adding anything.
                    //
                    // Matched by **id**, with the old "last unresolved row" as the
                    // fallback: a session file written before rows carried ids still
                    // replays, and a file with two calls in flight cannot be matched
                    // positionally at all.
                    let failed = r.is_error;
                    let wanted = r.tool_call_id.to_string();
                    let found = t
                        .rows
                        .iter()
                        .rposition(
                            |row| matches!(&row.kind, RowKind::Tool { id, .. } if *id == wanted),
                        )
                        .or_else(|| {
                            t.rows.iter().rposition(|row| {
                                matches!(row.kind, RowKind::Tool { failed: None, .. })
                            })
                        });
                    if let Some(i) = found
                        && let RowKind::Tool {
                            failed: f, output, ..
                        } = &mut t.rows[i].kind
                    {
                        *f = Some(failed);
                        // The result goes in `output`, beside the live path's streamed
                        // tail — it used to overwrite `text`, which held the arguments,
                        // so **a replayed session showed the result where a live one
                        // showed the call**. Same conversation, two transcripts.
                        *output = append_output("", &text);
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
        self.changes.push(change.path.clone());
        self.diffs.push(change.clone());
        if self.diffs.len() > Self::MAX_DIFFS {
            self.diffs.remove(0);
        }
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

    /// A compaction digest row: the cost line, then the summary as its own markdown row.
    ///
    /// The window has no collapsed rows (it virtualises and shows everything), so both
    /// halves draw — the cost as a `Notice`, the digest as a [`RowKind::Digest`]
    /// through the same markdown path as assistant prose. The cost line shares one
    /// builder with the terminal (`epsilon_tui::state::compaction_cost_line`): three
    /// surfaces, one wording.
    fn push_compaction_digest(&mut self, tokens_before: u64, usage: Option<Usage>, summary: &str) {
        let cost = epsilon_tui::state::compaction_cost_line(tokens_before, usage);
        self.push(RowKind::Notice, cost);
        self.push(RowKind::Digest, summary.to_string());
    }

    /// Add a harness notice the reader should see.
    pub fn push_notice(&mut self, text: &str) {
        self.push(RowKind::Notice, text.to_string());
    }

    /// A failure, as a row of its own kind — the terminal's `SystemErr` half.
    ///
    /// Every caller reporting something that went wrong routes through here, so
    /// the failure's emphasis is decided once (the cross the failed tool owns,
    /// the danger colour, the accessible name) rather than per call site. The
    /// wording stays the caller's: this decides how a failure *looks*, never
    /// what it says.
    pub fn push_failure(&mut self, text: impl Into<String>) {
        self.push(RowKind::NoticeFail, text.into());
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

            // P-054/P-183, window live half: the reducer folded no compaction event,
            // so a compaction the agent ran left no row here — only the `/compact`
            // channel's bare notice. The digest rides the row as text: the window
            // has no collapsed rows, so there is no chord to hide it behind.
            AgentEvent::CompactionFinished {
                usage,
                summary,
                tokens_before,
                ..
            } => self.push_compaction_digest(*tokens_before, *usage, summary),

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
                // P-127, window half, live path: the stop reason is authoritative
                // only at the end, so the notice lands here, not on the rows the
                // stream opened.
                if let Message::Assistant(a) = &**message
                    && a.stop_reason == StopReason::MaxTokens
                {
                    self.push(
                        RowKind::Notice,
                        "Response was truncated before completion.".to_string(),
                    );
                }
                // C-110, window half, live path: a reply the reader stopped
                // says so for the same reason.
                if let Message::Assistant(a) = &**message
                    && a.stop_reason == StopReason::Aborted
                {
                    self.push(
                        RowKind::Notice,
                        "Response was stopped before completion.".to_string(),
                    );
                }
                // M62, window half: the reducer read the stop reasons and never
                // the error field, so a provider failure produced a silent,
                // complete-looking answer. The failure is a row of its own kind
                // (the cross the failed tool owns, the danger colour), and it
                // says the reply above is partial when text did arrive — a
                // cut-off answer must never read as a whole one.
                if let Message::Assistant(a) = &**message
                    && let Some(err) = a.error_message.as_deref()
                {
                    let label = if message_text(message).is_none_or(|t| t.trim().is_empty()) {
                        format!("error: {err}")
                    } else {
                        format!("error: {err} — the reply above is partial")
                    };
                    self.push_failure(label);
                    // The auth sibling, on the terminal's predicate so the two
                    // front ends cannot disagree about which failures earn it.
                    if epsilon_core::looks_like_auth_failure(err) {
                        self.push_failure(
                            "the configured model was never reached — no reply was \
                             generated. Run `/login <provider>` or set the provider's \
                             API key, then send this again",
                        );
                    }
                }
            }

            AgentEvent::ToolExecutionStart {
                tool_call_id,
                tool_name,
                args,
            } => {
                // **Snapshot before the tool runs.** `ToolExecutionStart` is emitted
                // before execution, so reading now is genuinely "before" — reading after
                // would produce an empty diff for every edit, which is the failure mode
                // of a review pane that silently shows nothing.
                self.watcher.begin(args, &self.workspace);
                let row = Row {
                    kind: RowKind::Tool {
                        id: tool_call_id.to_string(),
                        name: tool_name.clone(),
                        failed: None,
                        output: String::new(),
                        args: args.clone(),
                    },
                    // The arguments, not the result: what the agent is *about to do* is
                    // the thing a reader needs while it is doing it — titled per
                    // tool via the shared `call_head` (C-120), not raw JSON.
                    text: cap_title(epsilon_tui::renderers::call_head(tool_name, args, "…")),
                };
                self.push_row(row);
            }

            AgentEvent::ToolExecutionUpdate {
                tool_call_id,
                partial,
                ..
            } => {
                // **What the tool printed, while it is printing it.**
                //
                // This event reached the `_ => {}` arm and was dropped, which is why the
                // window showed `▸ bash {…}` and then `✓` with nothing in between — and why
                // a three-minute `cargo test` looked like a three-minute freeze, the exact
                // complaint that sent this project to the references in the first place.
                //
                // A long command that produces no intermediate events would still look
                // frozen here, so this is the fix for the *shape* of that symptom rather
                // than a claim that every tool streams. It is also what makes the window
                // agree with the terminal, which has shown this since M63.
                let text: String = partial
                    .content
                    .iter()
                    .filter_map(|b| match b {
                        ContentBlock::Text { text } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                if text.trim().is_empty() {
                    return;
                }
                let wanted = tool_call_id.to_string();
                if let Some(i) = self
                    .rows
                    .iter()
                    .position(|row| matches!(&row.kind, RowKind::Tool { id, .. } if *id == wanted))
                    && let RowKind::Tool { output, .. } = &mut self.rows[i].kind
                {
                    *output = append_output(output, &text);
                }
            }

            AgentEvent::ToolExecutionEnd {
                tool_call_id,
                is_error,
                ..
            } => {
                // By id, like the update above. It used to be `live_tool`, a **single**
                // slot: two tools in flight — a `bash` parked on a permission ask is the
                // ordinary case, not an exotic one — finished in the wrong order and the
                // first result was marked onto the second row. The field is deleted along
                // with its last reader, rather than left as a definition with no caller.
                let wanted = tool_call_id.to_string();
                if let Some(i) = self
                    .rows
                    .iter()
                    .position(|row| matches!(&row.kind, RowKind::Tool { id, .. } if *id == wanted))
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

    /// Empty the view, keeping every durable thing (M312).
    ///
    /// **A view, not the conversation.** Nothing here is lost: the rows are a projection of the
    /// session file, and the figures the window shows come from [`IdeSession`](crate::session::IdeSession)
    /// rather than from these rows, so `/clear` leaves the token totals on screen exactly as the
    /// terminal's does. The terminal's sentence says the same — *"transcript cleared — session
    /// totals kept"* — and both front ends say it because it is the truth about both.
    ///
    /// **The live indices are cleared too, and that is the whole subtlety.** `live_assistant`
    /// and `live_thinking` are *absolute* positions into `rows`; leaving them set would make
    /// the next stream update write into a row that no longer exists, or into whatever took its
    /// place. `trim` already had to learn that for the same reason (`rows.drain`), which is why
    /// this is stated here as well as written there.
    pub fn clear(&mut self) {
        self.rows.clear();
        self.live_assistant = None;
        self.live_thinking = None;
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
/// A tool title on one row, cut with a marker when it does not fit.
///
/// The shared `call_head` does not cap (the terminal wraps); a window row
/// is one line, so an unmarked cut here would read as the whole call. Same
/// 160-cell budget and marker the old JSON formatter used, so the
/// truncation pin keeps holding without knowing the format changed.
fn cap_title(title: String) -> String {
    if title.chars().count() <= 160 {
        title
    } else {
        // Cut on a character boundary and say that it was cut.
        let head: String = title.chars().take(157).collect();
        format!("{head}… (truncated)")
    }
}

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

// ==========================================================================================
// Session
// ==========================================================================================

/// The agent this window is driving.
pub struct IdeSession {
    /// The one agent. Shared with the runtime, never rebuilt.
    agent: Arc<Agent>,
    /// The goal-gate runner, sharing this agent's sandbox.
    ///
    /// Held so a goal round in the window is gated by the **same** confinement the agent's
    /// own shell is. `None` where the platform resolved no backend, which makes a gated
    /// goal uncompletable — the honest direction, and already what completion does with it.
    gate_runner: Option<epsilon_runtime::GoalGateRunner>,
    /// The approvals channel. The terminal uses the same type.
    approvals: Arc<ChannelApprovalUi>,
    /// What the launch resolved to — the source of every number the status line shows.
    launch: Launch,
    /// What composition said while it was building this agent, in the order it said it.
    ///
    /// **A window has no stderr.** These lines were `eprintln!` inside
    /// `build_runtime_and_gate_runner`, so a skill that failed to load, an extension that
    /// was skipped, a session that migrated or repaired itself and a migration that failed
    /// all reached a terminal and nobody else — the silent failure `AGENTS.md` §2.7 rules
    /// out. They ride with the session because the session is what composition produced,
    /// and [`IdeApp::new`](crate::app::IdeApp::new) is the only place with a surface to
    /// put them on.
    notices: Vec<String>,
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
    /// Events the broadcast lost because this window could not keep up (M312).
    ///
    /// **A counter beside the session, not a field on the transcript — and that placement is the
    /// fix.** `Lagged(n)` is raised on the *bridge task*, not on the transcript: the transcript is
    /// fed only by `AgentEvent`, so a count kept there could not learn of a lag without a new
    /// wire variant to carry it. The status line's `N event(s) dropped` segment was therefore
    /// **unreachable in production** — `Transcript::note_lag`, its only writer, had no caller, and
    /// the live lag path forwarded a notice instead (a read-only sweep found this; see
    /// `audit/evidence/plan024-item7-class-b-rederived.md`).
    ///
    /// Sharing an `AtomicU64` with the bridge is the existing idiom here, not a new one:
    /// `failure_slot` is shared with the spawned run for the same reason — a fact born on a task
    /// has to survive to the frame that will show it.
    dropped_events: Arc<std::sync::atomic::AtomicU64>,
}

/// A composed replacement session for a window workspace switch (D-046/S-105).
///
/// Built off the paint thread by [`IdeSession::start_for_workspace`] and handed to the
/// frame through the workspace channel: everything the swap touches travels in one
/// value, so the paint thread never composes. The old session (and its lease, released
/// on drop) is replaced whole — one workspace per window, which is why L-120's
/// multi-root tree stays out of scope.
pub struct WorkspaceSwitch {
    /// The new session, composed for the new workspace.
    pub session: IdeSession,
    /// The workspace it was composed for.
    pub workspace: std::path::PathBuf,
    /// Launch diagnostics to show (a window has no stderr).
    pub notices: Vec<String>,
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
    ///
    /// `root` here is the **sessions root**, not the workspace: since M217 a launch
    /// writes into `root/--<cwd>--/` and older sessions sit in `root` itself, so
    /// this reads [`epsilon_runtime::sessions_root`], the one string every reader
    /// takes rather than composing `.epsilon/sessions` itself. Passing the
    /// workspace root reads `<workspace>/--<cwd>--/`, a directory that does not
    /// exist — and the Sessions pane lists nothing, forever, with no error.
    /// Found because a `/new` test counted the list before and after: 0 -> 0.
    pub fn sessions(&self, cwd: &std::path::Path) -> Vec<epsilon_session::SessionSummary> {
        epsilon_session::list_sessions_for_cwd(
            &epsilon_runtime::sessions_root(self.agent.workspace_root()),
            cwd,
        )
    }

    /// Every session of `cwd` this window archived, newest first.
    ///
    /// The same root the live list reads, through the same union — so an
    /// archived row is found wherever its live self was, and the pane can
    /// list what it hid without a second notion of where sessions live.
    pub fn archived(&self, cwd: &std::path::Path) -> Vec<epsilon_session::SessionSummary> {
        epsilon_session::list_archived_for_cwd(
            &epsilon_runtime::sessions_root(self.agent.workspace_root()),
            cwd,
        )
    }

    /// The session file this agent currently holds.
    pub fn session_path(&self) -> Option<PathBuf> {
        self.agent.session_path()
    }

    /// Whether the project layer is trusted (round 56).
    ///
    /// Carried on the launch, like everything composition decided: prompt
    /// templates gate project roots on it, the same rule the terminal uses.
    pub fn project_trusted(&self) -> bool {
        self.launch.project_trusted
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
        // Where composition's startup diagnostics go. Collected into a local rather than
        // printed: see the field's doc — a window has no stderr, and the only reader of
        // these lines is this session's own notice strip.
        let mut notices = Vec::new();
        // `build_runtime_and_gate_runner`, not `build_runtime`: a goal round has to be able to
        // evaluate the goal's gates, and the runner shares the agent's sandbox. Resolving a
        // second one here would be the thing `epsilon-tools`' module header rules out — two
        // confinement answers in one session.
        let (agent, gate_runner) =
            epsilon_runtime::build_runtime_and_gate_runner(epsilon_runtime::RunParams {
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
                preset: launch.preset,
                notices: &mut notices,
            })
            .await?;

        let (forwarder_tx, inbox) = std::sync::mpsc::channel();
        Ok(Self {
            agent,
            gate_runner,
            approvals,
            launch,
            notices,
            runtime,
            forwarder_tx,
            inbox: std::sync::Mutex::new(inbox),
            failure_slot: Arc::new(Mutex::new(None)),
            running: AtomicBool::new(false),
            dropped_events: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        })
    }

    /// Compose a session for a *different* workspace, for a window switch (D-046/S-105).
    ///
    /// The same composition startup runs (`resolve_launch` + [`IdeSession::start`]), so
    /// opening a workspace behaves exactly like launching the binary in it — except for
    /// the two things that belong to the reader, not the directory: an offline session
    /// stays offline (a test or demo session that phoned home on switch would be a
    /// data-loss-shaped surprise), and an explicitly chosen model travels along (a
    /// folder change must not change the model). Trust is never carried: the new
    /// workspace resolves its own project state fresh. Blocking I/O throughout — call
    /// off the paint thread (the `/open` spawn runs this in `spawn_blocking`, where
    /// `block_on` is legal).
    pub fn start_for_workspace(
        runtime: std::sync::Arc<tokio::runtime::Runtime>,
        workspace: std::path::PathBuf,
        current: epsilon_runtime::Launch,
    ) -> Result<WorkspaceSwitch, String> {
        let mut request = epsilon_runtime::LaunchRequest::for_workspace(&workspace);
        if current.offline {
            request.provider = epsilon_runtime::ProviderChoice::Faux;
        }
        if current.model_explicit {
            request.model_reference = Some(current.model_id.clone());
        }
        let launch = epsilon_runtime::resolve_launch(&request)?;
        let notices = launch.notices.clone();
        let session = runtime.block_on(Self::start(runtime.clone(), launch))?;
        Ok(WorkspaceSwitch {
            session,
            workspace,
            notices,
        })
    }

    /// The current session's thread pool, for handing to its replacement (D-046/S-105).
    ///
    /// Cloned, not referenced: the old session is dropped by the swap, and the pool
    /// must outlive it — which it does, through the new session's own `Arc`.
    pub(crate) fn runtime_arc(&self) -> std::sync::Arc<tokio::runtime::Runtime> {
        self.runtime.clone()
    }

    /// How many agent events this window has lost to lag (M312).
    ///
    /// Cumulative for the life of the session, which is what the status line says: a count that
    /// reset would under-report a hole the transcript still has.
    pub fn dropped_events(&self) -> u64 {
        self.dropped_events
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Record `n` dropped events the way the bridge would, for a test (M312).
    ///
    /// **The window field cannot be set instead, and that is the point.** `IdeApp::pump`
    /// refreshes its copy from this counter every frame, so a test that wrote to the app's field
    /// would be testing a copy it also owns — and would go green against a window whose status
    /// bar read nothing at all. Driving the seam the bridge drives is what makes the assertion
    /// about the product's path rather than about its plumbing.
    ///
    /// A real lag needs a broadcast outrun, which no test should arrange by racing one; this is
    /// the same shape as `SessionStore::set_write_fault_for_tests` and `seed_transcript`.
    #[doc(hidden)]
    pub fn note_dropped_events_for_tests(&self, n: u64) {
        self.dropped_events
            .fetch_add(n, std::sync::atomic::Ordering::Relaxed);
    }

    /// The agent, for a caller that wants to run a turn.
    pub fn agent(&self) -> &Arc<Agent> {
        &self.agent
    }

    /// The goal-gate runner that shares this agent's sandbox, if the platform has one.
    ///
    /// Public because the window runs goal rounds; the alternative is for the window to
    /// resolve its own sandbox, which is a second confinement answer in one session.
    pub fn gate_runner(&self) -> Option<&epsilon_runtime::GoalGateRunner> {
        self.gate_runner.as_ref()
    }

    /// What the launch resolved to.
    pub fn launch(&self) -> &Launch {
        &self.launch
    }

    /// The startup diagnostics composition emitted while building this agent, in order.
    ///
    /// Public because the window is the only thing that can show them: they used to be
    /// `eprintln!`, which a terminal has and this window does not. Returned in emission
    /// order because that order is the launch the reader is being told about.
    pub fn notices(&self) -> &[String] {
        &self.notices
    }

    /// The approvals channel, so a pane can resolve a parked ask by id.
    pub fn approvals(&self) -> &Arc<ChannelApprovalUi> {
        &self.approvals
    }

    /// A handle to this session's runtime, so a caller can `enter` it.
    ///
    /// **Exists because of a quiet failure (M312).** `ask_line`, the goal round driver and the
    /// compaction path all use `tokio::runtime::Handle::try_current`, which returns `None` and
    /// makes them **degrade quietly** — the round driver resets its pending flag and returns, so
    /// the feature simply does not happen. That is the shape of the freeze this harness exists to
    /// remove, and a test that has to park an ask has no way to get onto the runtime that owns
    /// the agent otherwise.
    pub fn runtime_handle(&self) -> tokio::runtime::Handle {
        self.runtime.handle().clone()
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
        let dropped = self.dropped_events.clone();
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
                        if forward_lag(&dropped, &tx, n).is_err() {
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

/// Record a lag the bridge saw and forward a marker for it (M312).
///
/// **Both effects are here, in one function, because the status bar's `N event(s) dropped`
/// segment used to be unreachable (M312).** `Transcript::note_lag` counted drops and had no
/// caller; the bridge counted nothing and forwarded a notice. One of the two was therefore dead
/// and the other was the only thing a reader ever saw — so the count the window shows is taken
/// from the same place the notice is sent.
///
/// The notice stays: it says *when*, and the counter says *how much*, and a transcript with a
/// silent hole needs the marker in it. Dropping the notice to save one line would be the
/// "reuse over add" instinct applied backwards — it is not duplication, it is two audiences.
///
/// Returns `Err(())` when the window is gone, which is the bridge's cue to stop.
fn forward_lag(
    dropped: &std::sync::atomic::AtomicU64,
    tx: &std::sync::mpsc::Sender<Arc<AgentEvent>>,
    n: u64,
) -> Result<(), ()> {
    use std::sync::atomic::Ordering;
    dropped.fetch_add(n, Ordering::Relaxed);
    tx.send(Arc::new(AgentEvent::Notice {
        text: format!("{n} event(s) dropped — the window could not keep up"),
    }))
    .map_err(|_| ())
}

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

    /// **The lag path counts and reports, or the status bar's segment is decoration (M312).**
    ///
    /// Found by a read-only sweep rather than by a failing run: `Transcript::note_lag` counted
    /// drops, `app.rs` drew `N event(s) dropped` from it, and **nothing in production ever
    /// called it** — the bridge's lag arm forwarded a notice and moved on. The segment could not
    /// appear on any real window, which is `AGENTS.md` §2.7: a control that does not do what it
    /// names.
    ///
    /// **This test keeps the name of the one it replaces, and not only for the guard.** The old
    /// `a_lagging_broadcast_is_reported_and_counted_not_swallowed` called `note_lag` directly — a
    /// method no production code reached, so it passed for years over a path nothing used. This
    /// one drives `forward_lag`, which is what the bridge actually calls, so a regression in the
    /// real path could not leave it green. The repository's own
    /// `no_source_file_lost_a_test_since_head` guard caught the deletion and forced the question,
    /// which is the guard working: a renamed test has to be *explained*, not waved through.
    ///
    /// Both halves are asserted because the defect was one half being live and the other dead:
    /// a counter that is not read is as much a lie as a drawn number that is always zero, and a
    /// notice with no count leaves the reader unable to tell how bad the hole was.
    #[test]
    fn a_lagging_broadcast_is_reported_and_counted_not_swallowed() {
        use std::sync::atomic::AtomicU64;
        let dropped = AtomicU64::new(0);
        let (tx, rx) = std::sync::mpsc::channel();

        forward_lag(&dropped, &tx, 7).expect("the window is still there");
        assert_eq!(
            dropped.load(std::sync::atomic::Ordering::Relaxed),
            7,
            "the status bar reads this number, so the lag path must write it"
        );
        let marker = match &*rx.recv().expect("the marker is forwarded") {
            AgentEvent::Notice { text } => text.clone(),
            other => panic!("a lag is announced as a notice, not as {other:?}"),
        };
        assert!(
            marker.contains("7 event(s) dropped"),
            "and the transcript still gets the hole marked in it: {marker}"
        );

        // Cumulative, because a transcript still has the hole and a count that reset would
        // under-report it.
        forward_lag(&dropped, &tx, 3).expect("still there");
        assert_eq!(
            dropped.load(std::sync::atomic::Ordering::Relaxed),
            10,
            "drops accumulate across a session rather than reporting only the last one"
        );
        assert!(
            rx.try_recv().is_ok(),
            "a second lag says so too — a reader must not have to guess whether the count moved"
        );
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
                id: "c1".into(),
                name: "bash".into(),
                failed: None,
                output: String::new(),
                args: serde_json::json!({"command": "cargo test"}),
            }
        );
        assert_eq!(
            running.text, "$ cargo test",
            "the title is tool-owned (shared call_head), not raw JSON: {:?}",
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
                id: "c1".into(),
                name: "bash".into(),
                failed: Some(true),
                output: String::new(),
                args: serde_json::json!({"command": "cargo test"}),
            }
        );
    }

    /// **A tool's output reaches the row, and two tools in flight do not cross.**
    ///
    /// `ToolExecutionUpdate` reached the reducer's `_ => {}` arm and was dropped, so the
    /// window showed `▸ bash {…}` and then `✓` and never a line of what the command
    /// printed. For `cargo test` that is the difference between "it ran" and "it passed".
    ///
    /// Two calls, deliberately **interleaved**, because that is the case the old code could
    /// not express: it tracked one running tool in a single slot, so the first result to
    /// arrive was marked onto whichever row happened to be live. A `bash` parked on a
    /// permission ask while a second call starts is ordinary, not exotic.
    #[test]
    fn streamed_tool_output_lands_on_the_row_that_produced_it() {
        let mut t = transcript();
        for (id, cmd) in [("c1", "cargo test"), ("c2", "ls -la")] {
            t.apply(&AgentEvent::ToolExecutionStart {
                tool_call_id: id.into(),
                tool_name: "bash".into(),
                args: serde_json::json!({ "command": cmd }),
            });
        }
        // Interleaved: c1 speaks, c2 speaks, c1 speaks again.
        for (id, chunk) in [
            ("c1", "running 12 tests\ntest a ... ok"),
            ("c2", "total 8\ndrwxr-xr-x  ."),
            ("c1", "test result: ok. 12 passed"),
        ] {
            t.apply(&AgentEvent::ToolExecutionUpdate {
                tool_call_id: id.into(),
                tool_name: "bash".into(),
                partial: epsilon_contracts::tool::ToolPartial {
                    content: vec![epsilon_contracts::content::ContentBlock::text(chunk)],
                    details: serde_json::Value::Null,
                },
            });
        }
        t.apply(&AgentEvent::ToolExecutionEnd {
            tool_call_id: "c1".into(),
            tool_name: "bash".into(),
            is_error: false,
        });
        t.apply(&AgentEvent::ToolExecutionEnd {
            tool_call_id: "c2".into(),
            tool_name: "bash".into(),
            is_error: true,
        });

        let rows = t.rows();
        let first = match &rows[0].kind {
            RowKind::Tool {
                id,
                name,
                failed,
                output,
                ..
            } => (id, name, *failed, output),
            other => panic!("a tool row, got {other:?}"),
        };
        let second = match &rows[1].kind {
            RowKind::Tool {
                id,
                name,
                failed,
                output,
                ..
            } => (id, name, *failed, output),
            other => panic!("a tool row, got {other:?}"),
        };

        assert_eq!(first.0, "c1", "row order follows call order");
        assert!(
            first.3.contains("12 passed") && first.3.contains("test a ... ok"),
            "c1 keeps its own output in order: {:?}",
            first.3
        );
        assert!(
            !first.3.contains("total 8"),
            "c2's output must not land on c1's row: {:?}",
            first.3
        );
        assert_eq!(second.0, "c2");
        assert!(
            second.3.contains("total 8") && !second.3.contains("12 passed"),
            "c2 keeps its own output: {:?}",
            second.3
        );
        // And the outcome still lands on the right row, which is what the single slot broke.
        assert_eq!(first.2, Some(false), "c1 succeeded");
        assert_eq!(second.2, Some(true), "c2 failed");
    }

    /// The row keeps **everything**, because `/expand` shows it all: the
    /// bound moved to the draw (a tail plus a marker until expanded), so a
    /// runaway command's bytes live in the row and the pane stays
    /// scrollable. Same trade the terminal makes.
    #[test]
    fn streamed_tool_output_keeps_everything_for_expand() {
        let mut t = transcript();
        t.apply(&AgentEvent::ToolExecutionStart {
            tool_call_id: "c1".into(),
            tool_name: "bash".into(),
            args: serde_json::json!({ "command": "build" }),
        });
        for n in 0..(TOOL_OUTPUT_TAIL_LINES + 25) {
            t.apply(&AgentEvent::ToolExecutionUpdate {
                tool_call_id: "c1".into(),
                tool_name: "bash".into(),
                partial: epsilon_contracts::tool::ToolPartial {
                    content: vec![epsilon_contracts::content::ContentBlock::text(format!(
                        "line {n}"
                    ))],
                    details: serde_json::Value::Null,
                },
            });
        }
        let RowKind::Tool { output, .. } = &t.rows()[0].kind else {
            panic!("a tool row");
        };
        assert_eq!(
            output.lines().count(),
            TOOL_OUTPUT_TAIL_LINES + 25,
            "kept {} lines for /expand; the draw caps at {TOOL_OUTPUT_TAIL_LINES}",
            output.lines().count()
        );
        assert!(
            output.contains(&format!("line {}", TOOL_OUTPUT_TAIL_LINES + 24)),
            "the newest line must survive: {output}"
        );
        assert!(
            output.contains("line 0\n"),
            "and the oldest must survive too — dropping it is the draw's job: {output}"
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

    // The test that used to stand here kept its name and was rewritten: it called
    // `Transcript::note_lag`, which had **no production caller**, so it passed for years over a
    // method nothing used. The one above it, of the same name, drives `forward_lag` — the path
    // the bridge actually takes. `Transcript::note_lag` and `Transcript::dropped()` are deleted.

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

    /// A reply cut off by the token limit adds a notice row (P-127, window half).
    ///
    /// The terminal says the same words over the same condition; a window that
    /// stayed silent would let a half answer read whole. The EndTurn message is
    /// the control: two replies, one notice. RED first: no row carries it.
    #[test]
    fn a_truncated_reply_adds_a_notice_row() {
        fn stopped(text: &str, stop: StopReason) -> Message {
            let mut m = assistant(text);
            if let Message::Assistant(a) = &mut m {
                a.stop_reason = stop;
            }
            m
        }
        let mut t = transcript();
        for (text, stop) in [
            ("the answer so far", StopReason::MaxTokens),
            ("the whole answer", StopReason::EndTurn),
        ] {
            t.apply(&AgentEvent::MessageEnd {
                message: Box::new(stopped(text, stop)),
            });
        }
        let notices: Vec<&Row> = t
            .rows()
            .iter()
            .filter(|r| r.kind == RowKind::Notice && r.text.contains("Response was truncated"))
            .collect();
        assert_eq!(
            notices.len(),
            1,
            "exactly one truncation notice for one truncated reply: {:?}",
            t.rows()
        );
    }

    /// A reply that ended in an error says so, as a failure (M62, window half).
    ///
    /// The terminal's M62 marker stops a cut-off answer from reading as a
    /// complete one, and says the failure out loud. This window dropped the
    /// assistant's `error_message` entirely: the reducer's `MessageEnd` arm
    /// read the stop reasons and never the error field, so a provider failure
    /// produced a silent, complete-looking answer — the owner's 2026-10-04
    /// report ("the output of the model is not shown") was this shape one layer
    /// down.
    ///
    /// The EndTurn message is the control: one reply, one failure row.
    #[test]
    fn a_failed_reply_marks_the_transcript_as_a_failure() {
        let mut t = transcript();
        let mut m = assistant("the answer that never arrived");
        if let Message::Assistant(a) = &mut m {
            a.error_message = Some("HTTP 401 (Auth): api key invalid".into());
        }
        t.apply(&AgentEvent::MessageEnd {
            message: Box::new(m),
        });
        t.apply(&AgentEvent::MessageEnd {
            message: Box::new(assistant("a whole answer")),
        });
        let failures: Vec<&Row> = t
            .rows()
            .iter()
            .filter(|r| r.text.contains("HTTP 401"))
            .collect();
        assert_eq!(
            failures.len(),
            1,
            "the provider's failure must reach the transcript: {:?}",
            t.rows()
        );
    }

    /// An auth rejection says the configured model was never reached.
    ///
    /// The terminal's sibling note (M62): the provider's body says "invalid api
    /// key" and a reader has no way to tell the model named in their status bar
    /// was never reached at all — the confusion behind the owner's report. One
    /// shared predicate (`epsilon_core::looks_like_auth_failure`) so the two
    /// front ends cannot disagree about which failures get it.
    #[test]
    fn an_auth_failure_says_the_model_was_never_reached() {
        let mut t = transcript();
        let mut m = assistant("");
        if let Message::Assistant(a) = &mut m {
            a.error_message = Some("HTTP 401 Authentication Fails, Your api key is invalid".into());
        }
        t.apply(&AgentEvent::MessageEnd {
            message: Box::new(m),
        });
        assert!(
            t.rows().iter().any(|r| r.text.contains("never reached")),
            "an auth failure must say the model was never reached: {:?}",
            t.rows()
        );
    }

    /// Replayed history carries the notice too (P-127, window half).
    ///
    /// `from_messages` renders without events; a session reopened onto a
    /// cut-off answer must not read whole either.
    #[test]
    fn replayed_history_carries_the_truncation_notice() {
        let mut m = assistant("the resumed half");
        if let Message::Assistant(a) = &mut m {
            a.stop_reason = StopReason::MaxTokens;
        }
        let t = Transcript::from_messages(&[m]);
        assert!(
            t.rows()
                .iter()
                .any(|r| r.kind == RowKind::Notice && r.text.contains("Response was truncated")),
            "a replayed cut-off answer must carry its notice: {:?}",
            t.rows()
        );
    }

    /// A stopped reply adds a notice row (C-110, window half).
    ///
    /// Same hole as truncation, other cause: without the line, an answer cut
    /// short by the reader reads as a whole one. The EndTurn message is the
    /// control: two replies, one notice. RED first: no row carries it.
    #[test]
    fn a_stopped_reply_adds_a_notice_row() {
        fn stopped(text: &str, stop: StopReason) -> Message {
            let mut m = assistant(text);
            if let Message::Assistant(a) = &mut m {
                a.stop_reason = stop;
            }
            m
        }
        let mut t = transcript();
        for (text, stop) in [
            ("the answer so far", StopReason::Aborted),
            ("the whole answer", StopReason::EndTurn),
        ] {
            t.apply(&AgentEvent::MessageEnd {
                message: Box::new(stopped(text, stop)),
            });
        }
        let notices: Vec<&Row> = t
            .rows()
            .iter()
            .filter(|r| r.kind == RowKind::Notice && r.text.contains("Response was stopped"))
            .collect();
        assert_eq!(
            notices.len(),
            1,
            "exactly one stopped notice for one stopped reply: {:?}",
            t.rows()
        );
    }

    /// Replayed history carries the stopped notice too (C-110, window half).
    #[test]
    fn replayed_history_carries_the_stopped_notice() {
        let mut m = assistant("the stopped half");
        if let Message::Assistant(a) = &mut m {
            a.stop_reason = StopReason::Aborted;
        }
        let t = Transcript::from_messages(&[m]);
        assert!(
            t.rows()
                .iter()
                .any(|r| r.kind == RowKind::Notice && r.text.contains("Response was stopped")),
            "a replayed stopped answer must carry its notice: {:?}",
            t.rows()
        );
    }

    /// A resumed compaction marker regains its digest (P-054/P-183, window replay half).
    ///
    /// `from_messages` renders without events and a compaction entry is not a message,
    /// so today the marker replays as a user turn the reader never typed. RED first:
    /// no row carries the cost line.
    #[test]
    fn replayed_history_reattaches_the_compaction_digest() {
        let marker = Message::User(epsilon_contracts::message::UserMessage {
            id: "m".into(),
            timestamp_ms: 0,
            content: vec![epsilon_contracts::content::UserContent::Text {
                text: epsilon_contracts::message::COMPACTION_MARKER.into(),
            }],
        });
        let t = Transcript::from_messages_with_digests(
            &[marker],
            &[epsilon_session::CompactionDigest {
                summary: "earlier work".into(),
                tokens_before: 12_000,
                usage: None,
                model_id: None,
            }],
        );
        // Two rows, not one: the cost notice, then the digest as its own markdown
        // row. One row would force the summary through the notice's plain-text
        // path or the cost line through the markdown parser — both misrender one
        // half. RED first: the cost row carries the summary as text.
        let cost = t
            .rows()
            .iter()
            .find(|r| r.text.contains("context compacted · from"))
            .expect("the resumed compaction must draw its cost row");
        assert_eq!(
            cost.kind,
            RowKind::Notice,
            "a compaction row is a notice, not a user turn: {:?}",
            t.rows()
        );
        assert!(
            !cost.text.contains("earlier work"),
            "the cost row names the price, not the prose: {}",
            cost.text
        );
        let digest = t
            .rows()
            .iter()
            .find(|r| r.kind == RowKind::Digest)
            .expect("the digest must ride its own row");
        assert_eq!(
            digest.text, "earlier work",
            "the digest row carries the summary"
        );
    }

    /// A marker with no digest stays a user row — the reader typed nothing, but neither
    /// did the compactor record anything, and inventing a summary is worse than a
    /// strange-looking user line. RED first: the row must survive untouched.
    #[test]
    fn a_marker_without_a_digest_stays_a_user_row() {
        let marker = Message::User(epsilon_contracts::message::UserMessage {
            id: "m".into(),
            timestamp_ms: 0,
            content: vec![epsilon_contracts::content::UserContent::Text {
                text: epsilon_contracts::message::COMPACTION_MARKER.into(),
            }],
        });
        let t = Transcript::from_messages_with_digests(&[marker], &[]);
        assert_eq!(t.rows().len(), 1);
        assert_eq!(
            t.rows()[0].kind,
            RowKind::User,
            "no digest, no rewrite: {:?}",
            t.rows()
        );
    }

    /// A live compaction draws its cost row with the digest (P-054/P-183, window live half).
    ///
    /// The reducer folded no compaction event, so a compaction the agent ran left no
    /// row at all in this front end — only the `/compact` channel's bare "context
    /// compacted" notice. RED first: no row carries the cost line.
    #[test]
    fn a_live_compaction_draws_its_cost_row_with_the_digest() {
        let mut t = transcript();
        t.apply(&AgentEvent::CompactionFinished {
            usage: None,
            model_id: None,
            summary: "earlier work".into(),
            tokens_before: 8_000,
        });
        let cost = t
            .rows()
            .iter()
            .find(|r| r.text.contains("context compacted · from"))
            .expect("the live compaction must draw its cost row");
        assert_eq!(cost.kind, RowKind::Notice, "{:?}", t.rows());
        let digest = t
            .rows()
            .iter()
            .find(|r| r.kind == RowKind::Digest)
            .expect("the digest must ride its own markdown row");
        assert_eq!(digest.text, "earlier work", "{:?}", t.rows());
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

    /// **A notice emitted while the agent was composed is held by the session.**
    ///
    /// This is the minimum claim the fix has to make: composition writes its startup
    /// diagnostics somewhere a window can read. Before it, every one of those lines was an
    /// `eprintln!` and the window had no stderr, so a skill that failed to load or a
    /// session that repaired itself reached nobody who could act on it.
    ///
    /// **The trigger is a resumed session** because it announces itself unconditionally —
    /// no project trust, no skill on disk, nothing about the developer's home directory.
    ///
    /// **Negative control.** With the emissions left as `eprintln!`, `notices()` is empty
    /// and this fails; it cannot pass for the wrong reason because `resolve_launch` runs
    /// *before* composition and is asserted not to be the source of this line.
    #[test]
    fn a_composition_notice_reaches_the_stored_notices() {
        use epsilon_runtime::{LaunchRequest, ProviderChoice, Resume};

        let ws = tempfile::tempdir().expect("a workspace");
        let prior = ws.path().join("prior.jsonl");
        let mut store =
            epsilon_session::SessionStore::open_in(&prior, ws.path()).expect("the prior session");
        store
            .append(epsilon_contracts::session_entry::EntryKind::Message {
                message: assistant("something said last time"),
                api_request_sha256: None,
            })
            .expect("the message lands in the log");
        drop(store);

        let runtime = Arc::new(
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .expect("a test runtime"),
        );
        let launch = epsilon_runtime::resolve_launch(&LaunchRequest {
            workspace: ws.path().to_path_buf(),
            provider: ProviderChoice::Faux,
            resume: Resume::Path(prior.clone()),
            ..LaunchRequest::for_workspace(ws.path())
        })
        .expect("a faux launch always resolves");
        assert!(
            !launch
                .notices
                .iter()
                .any(|n| n.contains("resumed 1 entries")),
            "the resume happens inside composition, not inside `resolve_launch`; if this fails \
             the assertion below is reading the wrong vector"
        );

        let session = runtime
            .block_on(IdeSession::start(Arc::clone(&runtime), launch))
            .expect("composing an agent over a stored session");

        let notices = session.notices();
        assert!(
            notices.iter().any(|n| n.contains("resumed 1 entries")),
            "composition announced the restore and the session has to be holding it, because \
             the window is the only surface that can show it. Got: {notices:?}"
        );
        // And the order the reader gets is the order composition emitted in, which is the
        // order the vector was appended to — not sorted, not grouped by subsystem.
        let first_resume = notices
            .iter()
            .position(|n| n.contains("resumed 1 entries"))
            .expect("the restore line");
        assert!(
            notices[..first_resume]
                .iter()
                .all(|n| !n.contains("resumed 1 entries")),
            "the restore is announced exactly once, in one place: {notices:?}"
        );
    }
}
