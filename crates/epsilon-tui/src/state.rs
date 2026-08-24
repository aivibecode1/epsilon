//! The transcript model: a pure reducer from [`AgentEvent`]s to renderable state.
//!
//! Frontends are reducers over the event bus (architecture law) — this module IS that
//! reducer, kept free of terminal code so it is unit-testable without a pty.

use epsilon_contracts::agent_event::AgentEvent;
use epsilon_contracts::message::{AssistantMessage, Message};

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

    /// Reduce one lifecycle event into the model.
    pub fn apply(&mut self, ev: &AgentEvent) {
        match ev {
            AgentEvent::AgentStart => {
                self.running = true;
                self.status = "run started".into();
                self.push_system("── run start ──");
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
                    if !text.is_empty() {
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
                    let head = format!("{} → {}", t.tool_name, truncate(&body, MAX_RESULT_CHARS));
                    self.push_line(TranscriptLine::expandable(
                        if t.is_error {
                            LineKind::ToolResultErr
                        } else {
                            LineKind::ToolResultOk
                        },
                        head,
                        format!("{} → {}", t.tool_name, body),
                    ));
                    self.status = "tool done".into();
                }
                _ => {}
            },
            AgentEvent::ToolExecutionStart { tool_name, .. } => {
                self.status = format!("running {tool_name}…");
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
            AgentEvent::AgentEnd { .. } => {
                // Flush any dangling stream buffer (error paths skip MessageEnd).
                self.streaming_text.clear();
                self.streaming_thinking.clear();
                self.running = false;
                self.status = "idle · type a task and press Enter".into();
                self.push_system("── run end ──");
            }
            // ToolExecutionUpdate partials and TurnEnd summaries are intentionally not
            // rendered yet: status shows activity; expandable tool cards stay on the
            // M12 polish ledger (architecture §11).
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

        let kinds: Vec<LineKind> = st.lines().iter().map(|l| l.kind).collect();
        assert_eq!(
            kinds,
            vec![
                LineKind::System,
                LineKind::Assistant,
                LineKind::ToolCall,
                LineKind::ToolResultOk,
                LineKind::System,
            ]
        );
        assert!(st.lines()[2].text.contains("bash"));
        assert!(st.lines()[3].text.contains("hi"));
        assert!(!st.running());
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
        assert_eq!(st.lines()[1].kind, LineKind::ToolResultErr);
        assert!(st.lines()[1].text.contains("boom"));
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
}
