//! Plan 001 regression tests: wire-level stop-reason fidelity + Gemini part
//! accumulation. Each test drives the adapter's `on_event` directly with raw JSON
//! payloads shaped like the provider's real frames (no network).

#![cfg(feature = "real")]

use epsilon_contracts::StopReason;
use epsilon_contracts::stream::StreamEvent;
use epsilon_llm::providers::anthropic::AnthropicWire;
use epsilon_llm::providers::google::GoogleWire;
use epsilon_llm::providers::openai_compat::OpenAiCompatWire;
use epsilon_llm::providers::{HttpEventStream, ProviderWire, SseEvent};

fn ev(data: &str) -> SseEvent {
    SseEvent {
        data: data.to_string(),
    }
}

fn final_stop(events: &[StreamEvent]) -> StopReason {
    events
        .iter()
        .find_map(|e| match e {
            StreamEvent::Done { message } => Some(message.stop_reason.clone()),
            _ => None,
        })
        .expect("a Done event")
}

#[test]
fn openai_length_finish_maps_to_max_tokens_even_with_tool_calls() {
    let w = OpenAiCompatWire {
        base_url: "https://api.openai.com/v1",
        env_key: None,
    };
    let mut st = HttpEventStream::empty_for_tests();
    // A tool call was streamed and closed, then finish_reason length arrives.
    let chunk = r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"c1","function":{"name":"bash","arguments":"{}"}}]},"finish_reason":null}]}"#;
    let _ = w.on_event(&ev(chunk), &mut st);
    let fin = r#"{"choices":[{"index":0,"delta":{},"finish_reason":"length"}]}"#;
    let _ = w.on_event(&ev(fin), &mut st);
    let done = w.on_event(&ev("[DONE]"), &mut st).unwrap();
    assert_eq!(final_stop(&done), StopReason::MaxTokens);
}

#[test]
fn openai_content_filter_maps_to_error() {
    let w = OpenAiCompatWire {
        base_url: "https://api.openai.com/v1",
        env_key: None,
    };
    let mut st = HttpEventStream::empty_for_tests();
    let fin = r#"{"choices":[{"index":0,"delta":{},"finish_reason":"content_filter"}]}"#;
    let _ = w.on_event(&ev(fin), &mut st);
    let done = w.on_event(&ev("[DONE]"), &mut st).unwrap();
    assert_eq!(final_stop(&done), StopReason::Error);
}

#[test]
fn anthropic_message_delta_stop_reason_drives_message_stop() {
    let w = AnthropicWire;
    let mut st = HttpEventStream::empty_for_tests();
    let _ = w.on_event(
        &ev(r#"{"type":"message_start","message":{"usage":{"input_tokens":1,"output_tokens":1}}}"#),
        &mut st,
    );
    let _ = w.on_event(
        &ev(r#"{"type":"message_delta","delta":{"stop_reason":"max_tokens"},"usage":{"output_tokens":9}}"#),
        &mut st,
    );
    let done = w
        .on_event(&ev(r#"{"type":"message_stop"}"#), &mut st)
        .unwrap();
    assert_eq!(final_stop(&done), StopReason::MaxTokens);
}

#[test]
fn anthropic_default_stop_still_ends_turn_cleanly() {
    let w = AnthropicWire;
    let mut st = HttpEventStream::empty_for_tests();
    let _ = w.on_event(
        &ev(r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":3}}"#),
        &mut st,
    );
    let done = w
        .on_event(&ev(r#"{"type":"message_stop"}"#), &mut st)
        .unwrap();
    assert_eq!(final_stop(&done), StopReason::EndTurn);
}

#[test]
fn gemini_chunk_with_text_and_function_call_emits_both() {
    let w = GoogleWire;
    let mut st = HttpEventStream::empty_for_tests();
    let frame = r#"{"candidates":[{"content":{"parts":[{"text":"let me check that."},{"functionCall":{"name":"grep","args":{"pattern":"x"}}}]}}]}"#;
    let out = w.on_event(&ev(frame), &mut st).expect("events emitted");
    assert!(
        out.iter()
            .any(|e| matches!(e, StreamEvent::TextDelta { .. })),
        "text part must survive"
    );
    assert!(
        st.has_open_or_closed_tools(),
        "functionCall in the same chunk must register"
    );
}

#[test]
fn multi_byte_char_split_across_chunks_survives() {
    // Plan 006: the SSE line buffer reassembles raw BYTES; a 4-byte emoji split
    // across two TCP chunks must decode intact once the line completes.
    let mut dec = epsilon_llm::providers::test_support::LineReassembler::new();
    let payload = r#"{"choices":[{"index":0,"delta":{"content":"héllo 🌍 done"}}]}"#;
    let line = format!("data: {payload}\n\n");
    let bytes = line.as_bytes();
    // Split mid-emoji (🌍 is 4 bytes starting after 'héllo 🌍'[..=6]).
    let emoji_start = payload.find("🌍").unwrap();
    let split = "data: ".len() + emoji_start + 2; // two bytes into the emoji

    let mut decoded: Vec<String> = Vec::new();
    for piece in [&bytes[..split], &bytes[split..]] {
        if let Some(l) = dec.push(piece) {
            decoded.push(l);
        }
    }
    assert_eq!(decoded.len(), 1);
    assert!(decoded[0].contains("🌍 done"), "{}", decoded[0]);
}

// ---- M21: thinking-level request mapping (pi parity on the hottest path) ----

use epsilon_contracts::ThinkingLevel;
use epsilon_contracts::tool::ToolDescriptor;
use epsilon_llm::LlmContext;

fn thinking_ctx(level: ThinkingLevel) -> LlmContext {
    LlmContext {
        system_prompt: String::new(),
        messages: vec![],
        tools: vec![ToolDescriptor {
            name: "bash".into(),
            label: "bash".into(),
            description: "run shell".into(),
            parameters_schema: serde_json::json!({"type": "object"}),
            execution_mode: epsilon_contracts::ExecutionMode::Parallel,
        }],
        thinking_level: level,
    }
}

#[test]
fn anthropic_adaptive_thinking_and_effort_when_enabled() {
    let w = AnthropicWire;
    let body = w
        .request_body("claude-opus-4-8", &thinking_ctx(ThinkingLevel::High))
        .unwrap();
    assert_eq!(body["thinking"]["type"], "adaptive");
    assert_eq!(body["output_config"]["effort"], "high");
    // Off sends nothing at all.
    let off = w
        .request_body("claude-opus-4-8", &thinking_ctx(ThinkingLevel::Off))
        .unwrap();
    assert!(off.get("thinking").is_none(), "off must omit the key");
    assert!(off.get("output_config").is_none());
}

#[test]
fn openai_compat_reasoning_effort_by_family() {
    let w = OpenAiCompatWire {
        base_url: "https://api.openai.com/v1",
        env_key: None,
    };
    // OpenAI-style family.
    let gpt = w
        .request_body("gpt-5.6", &thinking_ctx(ThinkingLevel::XHigh))
        .unwrap();
    assert_eq!(gpt["reasoning_effort"], "xhigh");
    // OpenRouter nested object.
    let or = w
        .request_body(
            "openrouter/anthropic/claude-opus-5",
            &thinking_ctx(ThinkingLevel::Medium),
        )
        .unwrap();
    assert_eq!(or["reasoning"]["effort"], "medium");
    // Off omits everything.
    let off = w
        .request_body("gpt-5.6", &thinking_ctx(ThinkingLevel::Off))
        .unwrap();
    assert!(off.get("reasoning_effort").is_none());
    assert!(off.get("reasoning").is_none());

    // DeepSeek reasoning rows: switch + effort; plain deepseek-chat stays clean.
    let ds = w
        .request_body("deepseek-reasoner", &thinking_ctx(ThinkingLevel::High))
        .unwrap();
    assert_eq!(ds["thinking"]["type"], "enabled");
    assert_eq!(ds["reasoning_effort"], "high");
    let ds_chat = w
        .request_body("deepseek-chat", &thinking_ctx(ThinkingLevel::High))
        .unwrap();
    assert!(
        ds_chat.get("thinking").is_none(),
        "deepseek-chat must not take the thinking switch"
    );

    // Z.ai GLM family: enable + clear_thinking=false + effort.
    let glm = w
        .request_body("glm-5.2", &thinking_ctx(ThinkingLevel::Medium))
        .unwrap();
    assert_eq!(glm["thinking"]["type"], "enabled");
    assert_eq!(glm["thinking"]["clear_thinking"], false);
    assert_eq!(glm["reasoning_effort"], "medium");

    // kimi-thinking rows take effort; plain kimi rows ignore it harmlessly.
    let kimi_t = w
        .request_body("kimi-thinking", &thinking_ctx(ThinkingLevel::Low))
        .unwrap();
    assert_eq!(kimi_t["reasoning_effort"], "low");
    let kimi_plain = w
        .request_body("kimi-k3", &thinking_ctx(ThinkingLevel::Low))
        .unwrap();
    assert!(
        kimi_plain.get("reasoning_effort").is_none(),
        "non-thinking kimi rows must not carry the knob"
    );
}

#[test]
fn google_thinking_config_with_level() {
    let w = GoogleWire;
    let body = w
        .request_body("gemini-3.7-flash", &thinking_ctx(ThinkingLevel::Low))
        .unwrap();
    assert_eq!(
        body["generationConfig"]["thinkingConfig"]["thinkingLevel"],
        serde_json::json!("LOW")
    );
    assert_eq!(
        body["generationConfig"]["thinkingConfig"]["includeThoughts"],
        serde_json::json!(true)
    );
    let off = w
        .request_body("gemini-3.7-flash", &thinking_ctx(ThinkingLevel::Off))
        .unwrap();
    assert!(off.get("generationConfig").is_none());
}
