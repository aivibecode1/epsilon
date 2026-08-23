//! Real provider adapter wrapping `genai` 0.6.x behind our [`StreamFn`] seam.
//!
//! genai types never leave this module. Mapping notes:
//! - Our `Done` message is authoritative, so we enable genai capture flags and assemble the
//!   final message from `StreamEnd.captured_*` rather than re-folding deltas.
//! - Text/reasoning deltas stream through live for UI; tool-call chunks are buffered and
//!   emitted once at end (avoids diffing partial-JSON fragments across providers).
//! - Stop reasons map: Completed->EndTurn/ToolUse, MaxTokens->MaxTokens, others->EndTurn;
//!   transport errors map to Done{Error} per the never-fail contract.

use async_trait::async_trait;

use epsilon_contracts::content::ContentBlock;
use epsilon_contracts::message::{
    AssistantMessage, Message, StopReason as OurStopReason, Usage as OurUsage,
};
use epsilon_contracts::new_id;
use epsilon_contracts::stream::StreamEvent;
use epsilon_contracts::tool::ToolDescriptor;

use crate::stream_fn::{
    BoxEventStream, EventStream, LlmContext, ModelRef, StreamFn, error_message,
};

/// Adapter over a shared genai client.
pub struct GenaiAdapter {
    client: genai::Client,
}

impl GenaiAdapter {
    /// Build with genai defaults (auth resolved by genai from standard env vars:
    /// ANTHROPIC_API_KEY / OPENAI_API_KEY / etc.; never stored by Epsilon).
    pub fn new() -> Self {
        Self {
            client: genai::Client::builder().build(),
        }
    }
}

impl Default for GenaiAdapter {
    fn default() -> Self {
        Self::new()
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn message_text(m: &Message) -> String {
    match m {
        Message::User(u) => u
            .content
            .iter()
            .filter_map(|c| match c {
                epsilon_contracts::content::UserContent::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
        Message::Assistant(a) => a.text(),
        Message::ToolResult(t) => t
            .content
            .iter()
            .filter_map(|c| match c {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
        Message::Custom(_) => String::new(),
    }
}

fn to_genai_request(ctx: &LlmContext) -> genai::chat::ChatRequest {
    let messages = ctx
        .messages
        .iter()
        .filter_map(|m| match m {
            Message::User(_) => Some(genai::chat::ChatMessage::user(message_text(m))),
            Message::Assistant(a) => {
                let tool_calls: Vec<genai::chat::ToolCall> = a
                    .content
                    .iter()
                    .filter_map(|b| match b {
                        ContentBlock::ToolCall {
                            id,
                            name,
                            arguments,
                        } => Some(genai::chat::ToolCall {
                            call_id: id.clone(),
                            fn_name: name.clone(),
                            fn_arguments: arguments.clone(),
                            thought_signatures: None,
                        }),
                        _ => None,
                    })
                    .collect();
                if tool_calls.is_empty() {
                    Some(genai::chat::ChatMessage::assistant(message_text(m)))
                } else {
                    Some(genai::chat::ChatMessage::from(tool_calls))
                }
            }
            Message::ToolResult(t) => Some(genai::chat::ChatMessage::from(
                genai::chat::ToolResponse::new(t.tool_call_id.clone(), message_text(m)),
            )),
            Message::Custom(_) => None,
        })
        .collect();

    let mut req = genai::chat::ChatRequest::new(messages);
    if !ctx.system_prompt.is_empty() {
        req = req.with_system(ctx.system_prompt.clone());
    }
    if !ctx.tools.is_empty() {
        let tools = ctx
            .tools
            .iter()
            .map(|d: &ToolDescriptor| genai::chat::Tool {
                name: genai::chat::ToolName::Custom(d.name.clone()),
                description: Some(d.description.clone()),
                schema: Some(d.parameters_schema.clone()),
                strict: None,
                config: None,
            });
        req = req.with_tools(tools);
    }
    req
}

fn to_our_usage(u: Option<&genai::chat::Usage>) -> OurUsage {
    let Some(u) = u else {
        return OurUsage::default();
    };
    let details_in = u.prompt_tokens_details.as_ref();
    let cache_read = details_in.and_then(|d| d.cached_tokens).unwrap_or(0);
    let mut cache_write = details_in
        .and_then(|d| d.cache_creation_tokens)
        .unwrap_or(0);
    if let Some(cc) = details_in.and_then(|d| d.cache_creation_details.as_ref()) {
        cache_write += cc.ephemeral_5m_tokens.unwrap_or(0) + cc.ephemeral_1h_tokens.unwrap_or(0);
    }
    OurUsage {
        input: u.prompt_tokens.unwrap_or(0).max(0) as u64,
        output: u.completion_tokens.unwrap_or(0).max(0) as u64,
        cache_read: cache_read.max(0) as u64,
        cache_write: cache_write.max(0) as u64,
        total_tokens: u.total_tokens.unwrap_or(0).max(0) as u64,
    }
}

fn map_stop(
    reason: Option<genai::chat::StopReason>,
    has_tool_calls: bool,
) -> (OurStopReason, Option<String>) {
    use genai::chat::StopReason as G;
    match reason {
        Some(G::MaxTokens(m)) => (OurStopReason::MaxTokens, Some(format!("max tokens: {m}"))),
        Some(G::ContentFilter(m)) => (OurStopReason::Error, Some(format!("content filter: {m}"))),
        _ if has_tool_calls => (OurStopReason::ToolUse, None),
        _ => (OurStopReason::EndTurn, None),
    }
}

struct GenaiEventStream {
    inner: genai::chat::ChatStream,
    text_index: usize,
    queued: std::collections::VecDeque<StreamEvent>,
    finished: bool,
}

impl GenaiEventStream {
    fn drain_end(&mut self, end: genai::chat::StreamEnd) -> StreamEvent {
        let mut content: Vec<ContentBlock> = Vec::new();
        if let Some(reasoning) = &end.captured_reasoning_content {
            if !reasoning.is_empty() {
                content.push(ContentBlock::Thinking {
                    text: reasoning.clone(),
                });
            }
        }
        let mut tool_calls = Vec::new();
        if let Some(captured) = end.captured_content.as_ref() {
            for part in captured.parts() {
                match part {
                    genai::chat::ContentPart::Text(t) => {
                        content.push(ContentBlock::text(t.clone()))
                    }
                    genai::chat::ContentPart::ToolCall(tc) => tool_calls.push(tc),
                    _ => {}
                }
            }
        }
        for tc in tool_calls {
            content.push(ContentBlock::ToolCall {
                id: tc.call_id.clone(),
                name: tc.fn_name.clone(),
                arguments: tc.fn_arguments.clone(),
            });
        }
        let has_calls = content
            .iter()
            .any(|b| matches!(b, ContentBlock::ToolCall { .. }));
        let (stop_reason, error_message) = map_stop(end.captured_stop_reason, has_calls);
        AssistantMessage {
            id: new_id("msg"),
            timestamp_ms: now_ms(),
            provider: String::new(),
            model_id: String::new(),
            content,
            stop_reason,
            error_message,
            usage: to_our_usage(end.captured_usage.as_ref()),
        }
        .into_done()
    }
}

trait IntoDone {
    fn into_done(self) -> StreamEvent;
}
impl IntoDone for AssistantMessage {
    fn into_done(self) -> StreamEvent {
        StreamEvent::Done { message: self }
    }
}

#[async_trait]
impl EventStream for GenaiEventStream {
    async fn next(&mut self) -> Option<StreamEvent> {
        if let Some(ev) = self.queued.pop_front() {
            return Some(ev);
        }
        if self.finished {
            return None;
        }

        use futures_util::StreamExt as _;
        use genai::chat::ChatStreamEvent as G;

        loop {
            match self.inner.next().await {
                Some(Ok(G::Start)) => {
                    let partial = AssistantMessage {
                        id: new_id("msg"),
                        timestamp_ms: now_ms(),
                        provider: String::new(),
                        model_id: String::new(),
                        content: vec![],
                        stop_reason: OurStopReason::EndTurn,
                        error_message: None,
                        usage: OurUsage::default(),
                    };
                    self.text_index = 0;
                    return Some(StreamEvent::Start { partial });
                }
                Some(Ok(G::Chunk(c))) => {
                    return Some(StreamEvent::TextDelta {
                        index: self.text_index,
                        delta: c.content,
                    });
                }
                Some(Ok(G::ReasoningChunk(c))) => {
                    return Some(StreamEvent::ThinkingDelta {
                        index: self.text_index,
                        delta: c.content,
                    });
                }
                Some(Ok(G::ThoughtSignatureChunk(_))) => {
                    // Provider-internal reasoning signatures; not user-visible content.
                    continue;
                }
                Some(Ok(G::ToolCallChunk(_))) => {
                    // Buffered; final calls arrive via End.captured_content (authoritative).
                    continue;
                }
                Some(Ok(G::End(end))) => {
                    self.finished = true;
                    return Some(self.drain_end(end));
                }
                Some(Err(e)) => {
                    self.finished = true;
                    let detail = format!("provider stream error: {e}");
                    return Some(error_message(detail).into_done());
                }
                None => {
                    self.finished = true;
                    // Stream ended without End event: treat as transport failure.
                    return Some(
                        error_message("provider stream ended without terminal event").into_done(),
                    );
                }
            }
        }
    }
}

#[async_trait]
impl StreamFn for GenaiAdapter {
    async fn stream(&self, model: &ModelRef, ctx: LlmContext) -> BoxEventStream {
        let req = to_genai_request(&ctx);
        let opts = genai::chat::ChatOptions {
            capture_usage: Some(true),
            capture_content: Some(true),
            capture_tool_calls: Some(true),
            capture_reasoning_content: Some(true),
            ..Default::default()
        };
        match self
            .client
            .exec_chat_stream(&model.id, req, Some(&opts))
            .await
        {
            Ok(resp) => Box::new(GenaiEventStream {
                inner: resp.stream,
                text_index: 0,
                queued: Default::default(),
                finished: false,
            }),
            Err(e) => {
                // Request setup failure (auth, unknown model, network): encoded, never thrown.
                let kind = crate::retry::classify(&e.to_string());
                let detail = format!("genai request failed ({kind:?}): {e}");
                Box::new(OnceStream {
                    ev: Some(error_message(detail).into_done()),
                })
            }
        }
    }
}

/// Degenerate one-event stream for pre-stream failures.
struct OnceStream {
    ev: Option<StreamEvent>,
}

#[async_trait]
impl EventStream for OnceStream {
    async fn next(&mut self) -> Option<StreamEvent> {
        self.ev.take()
    }
}
