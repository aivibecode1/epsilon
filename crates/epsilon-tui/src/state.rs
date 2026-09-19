//! The transcript model: a pure reducer from [`AgentEvent`]s to renderable state.
//!
//! Frontends are reducers over the event bus (architecture law) — this module IS that
//! reducer, kept free of terminal code so it is unit-testable without a pty.

use epsilon_contracts::agent_event::AgentEvent;
use epsilon_contracts::message::{AssistantMessage, Message, StopReason};

/// Visual class of one transcript line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    /// User input (local echo or steering).
    User,
    /// Assistant prose.
    Assistant,
    /// Assistant thinking block.
    Thinking,
    /// One requested tool call.
    ToolCall,
    /// Successful tool output.
    ToolResultOk,
    /// Failed tool output / blocked call.
    ToolResultErr,
    /// Lifecycle/system notice.
    System,
}

use std::sync::atomic::{AtomicU64, Ordering};

/// Process-wide id source. Uniqueness is what the render cache needs (content-stable,
/// never reused), whether lines come from a live session or are built ad hoc in tests.
static NEXT_LINE_ID: AtomicU64 = AtomicU64::new(1);

fn take_line_id() -> u64 {
    NEXT_LINE_ID.fetch_add(1, Ordering::Relaxed)
}

/// Human-readable placeholder for an image that cannot be rendered as graphics.
///
/// Mirrors pi's `imageFallback()` text path — the Kitty/iTerm2 escape is honest-parity
/// deferred (needs `detectCapabilities()` + `Kitty` negotiation). Until then the transcript
/// must not swallow images silently. `ponytail: no terminal probing, no base64 decode at v1`.
pub fn image_fallback_line(mime: &str, bytes: Option<usize>, filename: Option<&str>) -> String {
    let name = filename.unwrap_or("image");
    match bytes {
        Some(n) => format!("[image: {mime} {name} ({} bytes) — graphics deferred]", n),
        None => format!("[image: {mime} {name} — graphics deferred]"),
    }
}

/// Try to summarize a tool output that is likely an inline image blob.
///
/// Returns a single-line fallback when the text looks like `data:image/...;base64,` or a
/// very long base64 payload that would otherwise flood the transcript. `None` means "not an image".
pub fn summarize_image_blob(text: &str) -> Option<String> {
    let t = text.trim();
    if t.starts_with("data:image/") {
        // `data:image/png;base64,iVBOR…`
        let mime_end = t.find(';').unwrap_or(12);
        let mime = &t[5..mime_end];
        // rough byte estimate: base64 len * 3/4
        let b64_start = t.find(',').map(|i| i + 1).unwrap_or(t.len());
        let b64_len = t[b64_start..].trim().len();
        let est = if b64_len > 0 {
            Some((b64_len * 3) / 4)
        } else {
            None
        };
        return Some(image_fallback_line(mime, est, None));
    }
    // Heuristic: single line > 4k that is > 95% base64 chars → treat as image blob
    if t.len() > 4096 && !t.contains('\n') {
        let b64_like = t
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '+' || *c == '/' || *c == '=')
            .count();
        if b64_like * 20 >= t.len() * 19 {
            return Some(image_fallback_line(
                "image/*",
                Some((t.len() * 3) / 4),
                None,
            ));
        }
    }
    None
}

/// One rendered transcript row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptLine {
    /// Monotonic unique id (M16a): the render cache keys wrapped rows on this, so
    /// committed lines never re-render while content is unchanged.
    pub id: u64,
    /// Visual class driving styling.
    pub kind: LineKind,
    /// Display text (already summarized/truncated where appropriate).
    pub text: String,
    /// Untruncated original for expandable rows (tool outputs; M13 card polish).
    pub full: Option<String>,
    /// Render as a fenced-code block row (dim + gutter).
    pub code: bool,
    /// Raw markdown source for assistant rows (expanded by the renderer).
    pub md: Option<String>,
}

impl TranscriptLine {
    fn new(
        kind: LineKind,
        text: impl Into<String>,
        full: Option<String>,
        code: bool,
        md: Option<String>,
    ) -> Self {
        Self {
            id: take_line_id(),
            kind,
            text: text.into(),
            full,
            code,
            md,
        }
    }

    /// Plain line with no expansion payload.
    pub fn plain(kind: LineKind, text: impl Into<String>) -> Self {
        Self::new(kind, text, None, false, None)
    }

    /// Expandable line: `text` is the truncated head, `full` the complete body.
    pub fn expandable(kind: LineKind, text: String, full: String) -> Self {
        Self::new(kind, text, Some(full), false, None)
    }

    /// Fenced-code line (markdown-lite rendering).
    pub fn code_line(text: impl Into<String>) -> Self {
        Self::new(LineKind::Assistant, text, None, true, None)
    }

    /// Assistant message carrying raw markdown; expanded at render time.
    pub fn assistant_md(text: impl Into<String>) -> Self {
        let text = text.into();
        Self::new(LineKind::Assistant, text.clone(), None, false, Some(text))
    }
}

/// Long tool outputs collapse to a head + ellipsis so one bash call cannot flood the pane.
const MAX_RESULT_CHARS: usize = 400;

/// Reducible UI state for one TUI session.
#[derive(Debug, Default, Clone)]
pub struct UiState {
    lines: Vec<TranscriptLine>,
    streaming_text: String,
    streaming_thinking: String,
    status: String,
    running: bool,
    tokens_in: u64,
    tokens_out: u64,
    cache_read: u64,
    cache_write: u64,
    latest_prompt_tokens: u64,
    compacted: bool,
}

impl UiState {
    /// Empty state with an idle status line.
    pub fn new() -> Self {
        Self {
            status: "idle · type a task and press Enter".into(),
            ..Default::default()
        }
    }

    /// Transcript rows in order.
    pub fn lines(&self) -> &[TranscriptLine] {
        &self.lines
    }

    /// In-flight assistant text not yet committed to the transcript.
    pub fn streaming_text(&self) -> &str {
        &self.streaming_text
    }

    /// In-flight thinking not yet committed (pi: rendered as DIM thinking tail).
    pub fn streaming_thinking(&self) -> &str {
        &self.streaming_thinking
    }

    /// Footer status text.
    pub fn status(&self) -> &str {
        &self.status
    }

    /// True while a run is active (typed input becomes steering).
    pub fn running(&self) -> bool {
        self.running
    }

    /// Accumulated provider usage across the session: (input, output) billed tokens.
    pub fn usage(&self) -> (u64, u64) {
        (self.tokens_in, self.tokens_out)
    }

    /// Accumulated prompt-cache traffic across the session: (cache read, cache write).
    pub fn cache_usage(&self) -> (u64, u64) {
        (self.cache_read, self.cache_write)
    }

    /// Latest single-call prompt size = input + cache_read + cache_write
    /// (pi footer convention: this is what "context usage" measures). Zero until the
    /// first provider response of the session.
    pub fn latest_prompt_tokens(&self) -> u64 {
        self.latest_prompt_tokens
    }

    /// True right after a compaction pass, before the next provider response reports
    /// fresh token counts (pi shows `?` for context % in exactly this window).
    pub fn is_compacted(&self) -> bool {
        self.compacted
    }

    /// Local echo of submitted input; distinct from replayed transcript messages.
    pub fn push_local_input(&mut self, text: &str) {
        self.push_line(TranscriptLine::plain(LineKind::User, format!("> {text}")));
    }

    /// Runtime notice (run failures, lag warnings) rendered as a system row.
    pub fn push_notice(&mut self, text: impl Into<String>) {
        self.push_line(TranscriptLine::plain(LineKind::System, text));
    }

    /// Drop all transcript rows (`/clear`); usage counters survive.
    pub fn clear_lines(&mut self) {
        self.lines.clear();
        self.streaming_text.clear();
        self.streaming_thinking.clear();
    }

    /// Hydrate the visible transcript from a restored history (live `/resume` + `/import`).
    ///
    /// Clears existing rows and rebuilds them from `messages` (the store's shadowed
    /// model view). Ponytail: reuse existing line constructors and token accounting.
    pub fn hydrate_from_history(&mut self, messages: &[Message]) {
        self.lines.clear();
        self.streaming_text.clear();
        self.streaming_thinking.clear();
        self.tokens_in = 0;
        self.tokens_out = 0;
        self.cache_read = 0;
        self.cache_write = 0;
        self.latest_prompt_tokens = 0;
        self.compacted = false;
        for msg in messages {
            match msg {
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
                        .join(" ");
                    if text.trim().is_empty() {
                        continue;
                    }
                    self.push_line(TranscriptLine::plain(LineKind::User, format!("> {}", text)));
                }
                Message::Assistant(a) => {
                    let thinking = thinking_of(a);
                    if !thinking.is_empty() {
                        self.push_line(TranscriptLine::plain(
                            LineKind::Thinking,
                            truncate(&thinking, MAX_RESULT_CHARS),
                        ));
                    }
                    let text = a.text();
                    if !text.is_empty() {
                        self.push_line(TranscriptLine::assistant_md(text));
                    }
                    for (_, name, args) in a.tool_calls() {
                        self.push_line(TranscriptLine::plain(
                            LineKind::ToolCall,
                            format!("tool-call {} {}", name, summarize_args(args)),
                        ));
                    }
                    self.tokens_in += a.usage.input;
                    self.tokens_out += a.usage.output;
                    self.cache_read += a.usage.cache_read;
                    self.cache_write += a.usage.cache_write;
                    let prompt = a.usage.input + a.usage.cache_read + a.usage.cache_write;
                    if prompt > 0 {
                        self.latest_prompt_tokens = prompt;
                        self.compacted = false;
                    }
                }
                Message::ToolResult(t) => {
                    let body = t
                        .content
                        .iter()
                        .filter_map(|c| match c {
                            epsilon_contracts::content::ContentBlock::Text { text } => {
                                Some(text.as_str())
                            }
                            _ => None,
                        })
                        .collect::<String>();
                    if t.tool_name == "todo"
                        && let Some(head) = todo_head(&t.details)
                    {
                        self.push_line(TranscriptLine::expandable(
                            LineKind::ToolResultOk,
                            head,
                            format!("todo\n{body}"),
                        ));
                        continue;
                    }
                    if let Some(img) = summarize_image_blob(&body) {
                        self.push_line(TranscriptLine::expandable(
                            if t.is_error {
                                LineKind::ToolResultErr
                            } else {
                                LineKind::ToolResultOk
                            },
                            img.clone(),
                            img,
                        ));
                        continue;
                    }
                    let head = format!("{} → {}", t.tool_name, truncate(&body, MAX_RESULT_CHARS));
                    let full = if t.tool_name == "edit"
                        && let Some(diff) = t.details.get("diff").and_then(|d| d.as_str())
                        && !diff.is_empty()
                    {
                        format!("{} → diff:\n{diff}", t.tool_name)
                    } else {
                        format!("{} → {body}", t.tool_name)
                    };
                    self.push_line(TranscriptLine::expandable(
                        if t.is_error {
                            LineKind::ToolResultErr
                        } else {
                            LineKind::ToolResultOk
                        },
                        head,
                        full,
                    ));
                }
                Message::Custom(c) => {
                    self.push_line(TranscriptLine::plain(
                        LineKind::System,
                        format!("[custom {}] {}", c.custom_type, c.payload),
                    ));
                }
            }
        }
        if !messages.is_empty() {
            self.status = format!("resumed {} messages", messages.len());
        }
    }

    /// Reduce one lifecycle event into the model.
    pub fn apply(&mut self, ev: &AgentEvent) {
        match ev {
            AgentEvent::AgentStart => {
                self.running = true;
                self.status = "run started".into();
                // No "run started" row. The header already says a run is in
                // flight, and a marker row between every user turn and every
                // answer turns a conversation into a log file. The transcript is
                // for what was *said*; lifecycle state belongs in the chrome.
            }
            AgentEvent::TurnStart { turn_index } => {
                self.status = format!("turn {turn_index}: thinking…");
            }
            AgentEvent::MessageUpdate { message } => {
                if let Message::Assistant(a) = &**message {
                    self.streaming_thinking = thinking_of(a);
                    self.streaming_text = a.text();
                    self.status = "streaming…".into();
                }
            }
            AgentEvent::MessageEnd { message } => match &**message {
                Message::Assistant(a) => {
                    let thinking = std::mem::take(&mut self.streaming_thinking);
                    if !thinking.is_empty() {
                        self.push_line(TranscriptLine::plain(
                            LineKind::Thinking,
                            truncate(&thinking, MAX_RESULT_CHARS),
                        ));
                    }
                    self.streaming_text.clear();
                    let text = a.text();
                    let text_empty = text.is_empty();
                    if !text_empty {
                        // M15: raw markdown stored; the renderer expands it into
                        // styled rows (via the M16a render cache — once per message).
                        self.push_line(TranscriptLine::assistant_md(text));
                    }
                    for (_, name, args) in a.tool_calls() {
                        self.push_line(TranscriptLine::plain(
                            LineKind::ToolCall,
                            format!("tool-call {} {}", name, summarize_args(args)),
                        ));
                    }
                    if a.stop_reason == StopReason::Error
                        && let Some(err) = a.error_message.as_deref()
                        && !err.is_empty()
                    {
                        // Empty-Error Dones used to vanish (no text + no tool_call
                        // → zero lines), so the provider detail is always rendered.
                        // M62: when text DID arrive (salvage after a mid-stream
                        // failure) the marker is what stops a cut-off answer from
                        // reading as a complete one.
                        let label = if text_empty {
                            format!("error: {err}")
                        } else {
                            format!("error: {err} — the reply above is partial")
                        };
                        self.push_line(TranscriptLine::plain(LineKind::System, label));
                    }
                    self.tokens_in += a.usage.input;
                    self.tokens_out += a.usage.output;
                    self.cache_read += a.usage.cache_read;
                    self.cache_write += a.usage.cache_write;
                    // pi footer convention: context size = latest call's full prompt.
                    let prompt = a.usage.input + a.usage.cache_read + a.usage.cache_write;
                    if prompt > 0 {
                        self.latest_prompt_tokens = prompt;
                        self.compacted = false; // fresh counts retire the `?` marker
                    }
                }
                Message::ToolResult(t) => {
                    let body = t
                        .content
                        .iter()
                        .filter_map(|c| match c {
                            epsilon_contracts::content::ContentBlock::Text { text } => {
                                Some(text.as_str())
                            }
                            _ => None,
                        })
                        .collect::<String>();
                    // M30: todo results render their checklist (opencode's todo
                    // block) — short lists fully visible, long ones summarized.
                    if t.tool_name == "todo"
                        && let Some(head) = todo_head(&t.details)
                    {
                        self.push_line(TranscriptLine::expandable(
                            LineKind::ToolResultOk,
                            head,
                            format!("todo\n{body}"),
                        ));
                        self.status = "tool done".into();
                        return;
                    }
                    // Image blobs are summarized to a one-line placeholder so a pasted
                    // screenshot / data: URI does not flood the transcript (pi imageFallback parity).
                    if let Some(img) = summarize_image_blob(&body) {
                        self.push_line(TranscriptLine::expandable(
                            if t.is_error {
                                LineKind::ToolResultErr
                            } else {
                                LineKind::ToolResultOk
                            },
                            img.clone(),
                            img,
                        ));
                        self.status = "tool done".into();
                        return;
                    }
                    let head = format!("{} → {}", t.tool_name, truncate(&body, MAX_RESULT_CHARS));
                    // M30: edit results with a diff show the unified diff when
                    // expanded (Ctrl+X), giving the user a clear before/after view.
                    let full = if t.tool_name == "edit"
                        && let Some(diff) = t.details.get("diff").and_then(|d| d.as_str())
                        && !diff.is_empty()
                    {
                        format!("{} → diff:\n{diff}", t.tool_name)
                    } else {
                        format!("{} → {}", t.tool_name, body)
                    };
                    self.push_line(TranscriptLine::expandable(
                        if t.is_error {
                            LineKind::ToolResultErr
                        } else {
                            LineKind::ToolResultOk
                        },
                        head,
                        full,
                    ));
                    self.status = "tool done".into();
                }
                _ => {}
            },
            AgentEvent::ToolExecutionStart { tool_name, .. } => {
                self.status = format!("running {tool_name}…");
            }
            AgentEvent::ToolExecutionUpdate {
                tool_name, partial, ..
            } => {
                // M63: a long command reports what it is doing while it runs. The
                // status line carries the newest output line (bounded to one row);
                // the complete output still arrives with the tool result, so this
                // is a view, never the record.
                let text = partial
                    .content
                    .iter()
                    .filter_map(|b| match b {
                        epsilon_contracts::content::ContentBlock::Text { text } => {
                            Some(text.as_str())
                        }
                        _ => None,
                    })
                    .collect::<String>();
                let last = text
                    .lines()
                    .rev()
                    .find(|l| !l.trim().is_empty())
                    .unwrap_or("")
                    .trim();
                if !last.is_empty() {
                    self.status = format!("{tool_name}: {}", truncate(last, 120));
                }
            }
            AgentEvent::PermissionDecided { decision, .. } => {
                let label = match decision {
                    epsilon_contracts::agent_event::PermissionDecision::AllowOnce => "allowed",
                    epsilon_contracts::agent_event::PermissionDecision::AlwaysSession => {
                        "allowed (session)"
                    }
                    epsilon_contracts::agent_event::PermissionDecision::Deny => "denied",
                    epsilon_contracts::agent_event::PermissionDecision::DenyWithGuidance {
                        ..
                    } => "denied with guidance",
                };
                self.push_system(format!("permission {label}"));
            }
            AgentEvent::CompactionStarted => {
                self.status = "compacting context…".into();
                self.push_system("── compaction started ──");
            }
            AgentEvent::CompactionFinished => {
                self.status = "context compacted".into();
                self.push_system("── compaction finished ──");
                // Context size is now unknown until the next provider response
                // (pi footer: render `?` for the context percentage in this window).
                self.compacted = true;
            }
            AgentEvent::Notice { text } => {
                // M60: transport status (provider retries, stall watchdogs). The
                // status line carries it live; the transcript keeps a dim system
                // line so a pause after the fact is still explainable.
                self.status = text.clone();
                self.push_system(format!("⟳ {text}"));
            }
            AgentEvent::AgentEnd { .. } => {
                // Flush any dangling stream buffer (error paths skip MessageEnd).
                self.streaming_text.clear();
                self.streaming_thinking.clear();
                self.running = false;
                self.status = "idle · type a task and press Enter".into();
                // The matching marker for `AgentStart`: the header returns to
                // `idle`, which is the visible signal that the run finished.
                // A row saying so would be the third place the same fact lives.
            }
            // TurnEnd summaries are intentionally not rendered yet: status shows
            // activity; expandable tool cards stay on the M12 polish ledger
            // (architecture §11).
            _ => {}
        }
    }

    fn push_system(&mut self, text: impl Into<String>) {
        self.push_line(TranscriptLine::plain(LineKind::System, text));
    }

    /// Single mutation point for transcript appends. Ids come from the line itself
    /// (unique by construction, M16a); kept as a choke point for future hooks.
    fn push_line(&mut self, line: TranscriptLine) {
        self.lines.push(line);
    }
}

fn thinking_of(a: &AssistantMessage) -> String {
    a.content
        .iter()
        .filter_map(|c| match c {
            epsilon_contracts::content::ContentBlock::Thinking { text } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

/// Glanceable head for todo results (opencode's todo block): up to five items
/// render fully with their markers; longer lists summarize to counts.
/// Returns None when the details payload isn't the expected shape (then the
/// generic tool-result rendering takes over).
fn todo_head(details: &serde_json::Value) -> Option<String> {
    let items = details.get("todo")?.as_array()?;
    let marker = |status: &str| match status {
        "completed" => "[x]",
        "in_progress" => "[~]",
        _ => "[ ]",
    };
    let rendered: Vec<String> = items
        .iter()
        .filter_map(|i| {
            Some(format!(
                "{} {}",
                marker(i.get("status")?.as_str()?),
                i.get("content")?.as_str()?.trim()
            ))
        })
        .collect();
    if rendered.len() != items.len() {
        return None;
    }
    if rendered.len() <= 5 {
        Some(format!("todo {}", rendered.join(" · ")))
    } else {
        let done = rendered.iter().filter(|r| r.starts_with("[x]")).count();
        Some(format!(
            "todo ({done}/{} done) {}…",
            rendered.len(),
            rendered.first().unwrap_or(&String::new())
        ))
    }
}

fn summarize_args(args: &serde_json::Value) -> String {
    match args.get("command").and_then(|v| v.as_str()) {
        Some(cmd) => cmd.to_string(),
        None => {
            let s = args.to_string();
            truncate(&s, 80)
        }
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let head: String = s.chars().take(max).collect();
    format!("{head}…")
}

/// Whether expandable rows currently render their full payload.
#[derive(Debug, Default, Clone, Copy)]
pub struct RenderPrefs {
    /// `x` toggle: tool-result rows show full output instead of the truncated head.
    pub expand_outputs: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use epsilon_contracts::agent_event::TurnEndReason;
    use epsilon_contracts::content::ContentBlock;
    use epsilon_contracts::ids::new_id;
    use epsilon_contracts::message::{StopReason, ToolResultMessage, Usage};

    fn assistant(text: &str, calls: Vec<(&str, &str)>, usage: Usage) -> AssistantMessage {
        let mut content = vec![ContentBlock::text(text)];
        for (id, name) in calls {
            content.push(ContentBlock::ToolCall {
                id: id.into(),
                name: name.into(),
                arguments: serde_json::json!({"command": "echo hi"}),
            });
        }
        AssistantMessage {
            id: new_id("msg"),
            timestamp_ms: 0,
            provider: "faux".into(),
            model_id: "faux-1".into(),
            content,
            stop_reason: StopReason::EndTurn,
            error_message: None,
            usage,
        }
    }

    #[test]
    fn a_salvaged_partial_reply_is_marked_partial_not_complete() {
        // M62: text + a failure must read as a cut-off answer, never as a
        // finished one — the whole point of salvaging instead of discarding.
        let mut st = UiState::new();
        let mut msg = assistant("the answer so f", vec![], Usage::default());
        msg.stop_reason = StopReason::Error;
        msg.error_message = Some("stream read failed: connection reset".into());
        st.apply(&AgentEvent::MessageEnd {
            message: Box::new(Message::Assistant(msg)),
        });
        let joined = st
            .lines()
            .iter()
            .map(|l| l.text.clone())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(joined.contains("the answer so f"), "{joined}");
        assert!(
            joined.contains("partial"),
            "the failure is stated, not implied: {joined}"
        );
        assert!(joined.contains("connection reset"), "{joined}");
    }

    #[test]
    fn retry_notices_land_in_the_transcript_and_the_status_line() {
        let mut st = UiState::new();
        st.apply(&AgentEvent::Notice {
            text: "HTTP 503: overloaded — retrying gpt-x in 2.0s (attempt 2) (server error)".into(),
        });
        assert!(
            st.status.contains("retrying"),
            "status shows the wait: {}",
            st.status
        );
        let joined = st
            .lines()
            .iter()
            .map(|l| l.text.clone())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            joined.contains("retrying gpt-x"),
            "the transcript keeps the notice so a pause is explainable afterwards: {joined}"
        );
    }

    #[test]
    fn run_lifecycle_renders_in_order() {
        let mut st = UiState::new();
        st.apply(&AgentEvent::AgentStart);
        st.apply(&AgentEvent::TurnStart { turn_index: 0 });
        st.apply(&AgentEvent::MessageEnd {
            message: Box::new(Message::Assistant(assistant(
                "working on it",
                vec![("tc_1", "bash")],
                Usage {
                    input: 10,
                    output: 5,
                    ..Default::default()
                },
            ))),
        });
        st.apply(&AgentEvent::MessageEnd {
            message: Box::new(Message::ToolResult(ToolResultMessage {
                id: new_id("msg"),
                timestamp_ms: 0,
                tool_call_id: "tc_1".into(),
                tool_name: "bash".into(),
                content: vec![ContentBlock::text("hi")],
                details: serde_json::json!({}),
                usage: None,
                is_error: false,
            })),
        });
        st.apply(&AgentEvent::AgentEnd { messages: vec![] });

        // The transcript holds what was said, and only that: assistant prose,
        // the tool call, its result. Lifecycle state lives in the header, so a
        // run produces no marker rows to scroll past.
        let kinds: Vec<LineKind> = st.lines().iter().map(|l| l.kind).collect();
        assert_eq!(
            kinds,
            vec![
                LineKind::Assistant,
                LineKind::ToolCall,
                LineKind::ToolResultOk
            ]
        );
        assert!(st.lines()[1].text.contains("bash"));
        assert!(st.lines()[2].text.contains("hi"));
        // The header is where "a run is happening" is visible, so the state must
        // still know it even though no row says so.
        assert!(!st.running(), "the run ended");
        assert_eq!(st.status(), "idle · type a task and press Enter");
        assert_eq!(st.usage(), (10, 5));
    }

    #[test]
    fn streaming_updates_replace_then_commit_on_end() {
        let mut st = UiState::new();
        let mut a = assistant("", vec![], Usage::default());
        a.content[0] = ContentBlock::text("hel");
        st.apply(&AgentEvent::MessageUpdate {
            message: Box::new(Message::Assistant(a.clone())),
        });
        assert_eq!(st.streaming_text(), "hel");
        assert!(st.lines().is_empty(), "nothing committed mid-stream");

        a.content[0] = ContentBlock::text("hello world");
        st.apply(&AgentEvent::MessageUpdate {
            message: Box::new(Message::Assistant(a.clone())),
        });
        assert_eq!(st.streaming_text(), "hello world", "replace, not append");

        st.apply(&AgentEvent::MessageEnd {
            message: Box::new(Message::Assistant(a)),
        });
        assert_eq!(st.streaming_text(), "");
        assert_eq!(st.lines().len(), 1);
        assert_eq!(st.lines()[0].kind, LineKind::Assistant);
        assert_eq!(st.lines()[0].text, "hello world");
    }

    #[test]
    fn errors_render_as_error_lines_and_reason_survives() {
        let mut st = UiState::new();
        st.apply(&AgentEvent::AgentStart);
        st.apply(&AgentEvent::MessageEnd {
            message: Box::new(Message::ToolResult(ToolResultMessage {
                id: new_id("msg"),
                timestamp_ms: 0,
                tool_call_id: "tc_1".into(),
                tool_name: "bash".into(),
                content: vec![ContentBlock::text("boom")],
                details: serde_json::json!({}),
                usage: None,
                is_error: true,
            })),
        });
        st.apply(&AgentEvent::TurnEnd {
            turn_index: 0,
            message: Box::new(Message::Assistant(assistant("", vec![], Usage::default()))),
            tool_results: vec![],
        });
        let _ = TurnEndReason::Completed; // reason taxonomy exercised elsewhere
        // No leading lifecycle marker row any more, so the error is first.
        assert_eq!(st.lines()[0].kind, LineKind::ToolResultErr);
        assert!(st.lines()[0].text.contains("boom"));
    }

    #[test]
    fn oversized_tool_output_is_truncated() {
        let mut st = UiState::new();
        let big = "x".repeat(MAX_RESULT_CHARS + 50);
        st.apply(&AgentEvent::MessageEnd {
            message: Box::new(Message::ToolResult(ToolResultMessage {
                id: new_id("msg"),
                timestamp_ms: 0,
                tool_call_id: "tc_1".into(),
                tool_name: "read".into(),
                content: vec![ContentBlock::text(big)],
                details: serde_json::json!({}),
                usage: None,
                is_error: false,
            })),
        });
        let line = &st.lines()[0];
        assert!(line.text.chars().count() < MAX_RESULT_CHARS + 10);
        assert!(line.text.ends_with('…'));
    }

    #[test]
    fn local_input_echoes_with_prompt_prefix() {
        let mut st = UiState::new();
        st.push_local_input("fix the tests");
        assert_eq!(st.lines()[0].text, "> fix the tests");
        assert_eq!(st.lines()[0].kind, LineKind::User);
    }

    // ---- M30: todo results render their checklist --------------------------------------

    #[test]
    fn todo_results_render_as_a_checklist_head() {
        let mut st = UiState::new();
        st.apply(&AgentEvent::MessageEnd {
            message: Box::new(Message::ToolResult(ToolResultMessage {
                id: new_id("msg"),
                timestamp_ms: 0,
                tool_call_id: "tc_1".into(),
                tool_name: "todo".into(),
                content: vec![ContentBlock::text("[x] one\n[ ] two\n(1/2 completed)")],
                details: serde_json::json!({"todo": [
                    {"content": "one", "status": "completed"},
                    {"content": "two", "status": "pending"}
                ]}),
                usage: None,
                is_error: false,
            })),
        });
        let line = &st.lines()[0];
        assert_eq!(
            line.text, "todo [x] one · [ ] two",
            "short lists render fully: {:?}",
            line.text
        );
        assert!(line.full.as_deref().is_some_and(|f| f.contains("[x] one")));
    }

    #[test]
    fn long_todo_lists_summarize_to_counts() {
        let mut st = UiState::new();
        let items: Vec<serde_json::Value> = (0..7)
            .map(|i| {
                serde_json::json!({"content": format!("task {i}"), "status": if i < 3 {
                    "completed"
                } else {
                    "pending"
                }})
            })
            .collect();
        st.apply(&AgentEvent::MessageEnd {
            message: Box::new(Message::ToolResult(ToolResultMessage {
                id: new_id("msg"),
                timestamp_ms: 0,
                tool_call_id: "tc_1".into(),
                tool_name: "todo".into(),
                content: vec![ContentBlock::text("...")],
                details: serde_json::json!({ "todo": items }),
                usage: None,
                is_error: false,
            })),
        });
        let line = &st.lines()[0];
        assert!(line.text.starts_with("todo (3/7 done)"), "{:?}", line.text);
    }

    #[test]
    fn malformed_todo_details_fall_back_to_generic_rendering() {
        let mut st = UiState::new();
        st.apply(&AgentEvent::MessageEnd {
            message: Box::new(Message::ToolResult(ToolResultMessage {
                id: new_id("msg"),
                timestamp_ms: 0,
                tool_call_id: "tc_1".into(),
                tool_name: "todo".into(),
                content: vec![ContentBlock::text("boom")],
                details: serde_json::json!({"blocked": true}),
                usage: None,
                is_error: true,
            })),
        });
        let line = &st.lines()[0];
        assert_eq!(line.kind, LineKind::ToolResultErr, "generic error path");
        assert!(line.text.starts_with("todo → "));
    }

    // ---- M30: edit diff preview rows -----------------------------------------------

    #[test]
    fn edit_result_with_diff_shows_diff_when_expanded() {
        let mut st = UiState::new();
        st.apply(&AgentEvent::MessageEnd {
            message: Box::new(Message::ToolResult(ToolResultMessage {
                id: new_id("msg"),
                timestamp_ms: 0,
                tool_call_id: "tc_1".into(),
                tool_name: "edit".into(),
                content: vec![ContentBlock::text("edited a.rs (1 replacement(s))")],
                details: serde_json::json!({"diff": "@@ -1,2 +1,2 @@\n old\n-new\n+new\n"}),
                usage: None,
                is_error: false,
            })),
        });
        let line = &st.lines()[0];
        // Head stays short — the diff is hidden until Ctrl+X.
        assert!(line.text.starts_with("edit → "), "head: {:?}", line.text);
        let full = line.full.as_deref().expect("full must exist");
        assert!(full.contains("@@"), "full should contain the diff: {full}");
        assert!(full.contains("-new"));
        assert!(full.contains("+new"));
    }

    #[test]
    fn edit_result_without_diff_uses_body_for_full() {
        let mut st = UiState::new();
        st.apply(&AgentEvent::MessageEnd {
            message: Box::new(Message::ToolResult(ToolResultMessage {
                id: new_id("msg"),
                timestamp_ms: 0,
                tool_call_id: "tc_1".into(),
                tool_name: "edit".into(),
                content: vec![ContentBlock::text("edited b.rs (1 replacement(s))")],
                details: serde_json::json!({}),
                usage: None,
                is_error: false,
            })),
        });
        let line = &st.lines()[0];
        let full = line.full.as_deref().expect("full must exist");
        assert!(full.contains("edited b.rs"));
        assert!(
            !full.contains("@@"),
            "no diff in full when details.diff is absent"
        );
    }

    #[test]
    fn data_uri_image_becomes_placeholder_not_flood() {
        let mut st = UiState::new();
        let blob = format!("data:image/png;base64,{}", "A".repeat(8000));
        st.apply(&AgentEvent::MessageEnd {
            message: Box::new(Message::ToolResult(ToolResultMessage {
                id: new_id("msg"),
                timestamp_ms: 0,
                tool_call_id: "tc_1".into(),
                tool_name: "read".into(),
                content: vec![ContentBlock::text(blob.clone())],
                details: serde_json::json!({}),
                usage: None,
                is_error: false,
            })),
        });
        let line = &st.lines()[0];
        assert!(
            line.text.starts_with("[image:"),
            "should be placeholder: {:?}",
            line.text
        );
        assert!(!line.text.contains('A'), "must not contain raw base64");
        assert!(line.text.contains("graphics deferred"));
    }

    #[test]
    fn long_base64_blob_becomes_placeholder() {
        let mut st = UiState::new();
        let blob = "A".repeat(9000);
        st.apply(&AgentEvent::MessageEnd {
            message: Box::new(Message::ToolResult(ToolResultMessage {
                id: new_id("msg"),
                timestamp_ms: 0,
                tool_call_id: "tc_1".into(),
                tool_name: "bash".into(),
                content: vec![ContentBlock::text(blob)],
                details: serde_json::json!({}),
                usage: None,
                is_error: false,
            })),
        });
        assert!(
            st.lines()[0].text.starts_with("[image:"),
            "{:?}",
            st.lines()[0].text
        );
    }

    #[test]
    fn normal_text_not_mistaken_for_image() {
        assert!(summarize_image_blob("hello world").is_none());
        assert!(summarize_image_blob("data:text/plain;base64,SGVsbG8=").is_none());
    }
}
