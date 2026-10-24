//! Plan 001 regression tests: wire-level stop-reason fidelity + Gemini part
//! accumulation. Each test drives the adapter's `on_event` directly with raw JSON
//! payloads shaped like the provider's real frames (no network).

#![cfg(feature = "real")]

use epsilon_contracts::StopReason;
use epsilon_contracts::stream::StreamEvent;
use epsilon_llm::custom::CustomProvider;
use epsilon_llm::providers::anthropic::AnthropicWire;
use epsilon_llm::providers::google::GoogleWire;
use epsilon_llm::providers::openai_compat::OpenAiCompatWire;
use epsilon_llm::providers::{HttpEventStream, ProviderWire, SseEvent};

/// M28: a custom id routes to the declared endpoint on the OpenAI-compat wire,
/// and the endpoint join never doubles a trailing slash.
#[test]
fn custom_provider_id_routes_to_declared_endpoint() {
    let customs =
        vec![CustomProvider::from_config("lmstudio", "http://localhost:1234/v1/", "").unwrap()];
    let w = OpenAiCompatWire {
        base_url: "http://localhost:1234/v1".to_string(),
        env_key: None,
    };
    // The wire's endpoint shape is what custom routing produces; assert it
    // against the same join logic production uses (no trailing-slash doubles).
    assert_eq!(
        w.endpoint("lmstudio/qwen2.5-7b-instruct"),
        "http://localhost:1234/v1/chat/completions"
    );
    assert_eq!(w.env_key(), "", "no env_key = no auth header");
    assert!(w.headers("ignored").is_empty());
    // The lookup that picks this wire for an arbitrary id:
    assert!(epsilon_llm::custom::lookup_custom(&customs, "lmstudio/anything").is_some());
    assert!(epsilon_llm::custom::lookup_custom(&customs, "other/thing").is_none());
    // The wire body carries the remote model name, not the local routing prefix.
    let body = w
        .request_body(
            epsilon_llm::custom::wire_model_id("lmstudio/qwen2.5-7b-instruct", &customs),
            &thinking_ctx(ThinkingLevel::Off),
        )
        .expect("a valid request");
    assert_eq!(
        body["model"], "qwen2.5-7b-instruct",
        "the endpoint names the provider; the body names the model: {body}"
    );
}

fn ev(data: &str) -> SseEvent {
    SseEvent {
        name: None,
        data: data.to_string(),
    }
}

/// The assembled message a completed stream hands over — the Done event carries it.
fn done_message(
    events: &[StreamEvent],
    st: &HttpEventStream,
) -> epsilon_contracts::message::AssistantMessage {
    for e in events {
        if let StreamEvent::Done { message } = e {
            return message.clone();
        }
    }
    panic!(
        "no Done event for {:?}; the accumulator still holds a tool in flight: {}",
        events,
        st.has_open_or_closed_tools()
    );
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
        base_url: "https://api.openai.com/v1".to_string(),
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

/// **A usage-only frame is billed.** Providers that honour
/// `stream_options.include_usage` send a final chunk with `choices: []` and a
/// usage object. The old handler returned early on the missing choices and threw
/// the count away, so a real request showed `0` tokens and `$0`.
#[test]
fn openai_usage_only_frame_with_no_choices_is_still_counted() {
    let w = OpenAiCompatWire {
        base_url: "https://api.openai.com/v1".to_string(),
        env_key: None,
    };
    let mut st = HttpEventStream::empty_for_tests();
    let usage = r#"{"choices":[],"usage":{"prompt_tokens":120,"completion_tokens":30,"prompt_tokens_details":{"cached_tokens":100}}}"#;
    assert!(
        w.on_event(&ev(usage), &mut st).is_none(),
        "a usage frame emits no text event"
    );
    let done = w.on_event(&ev("[DONE]"), &mut st).unwrap();
    let message = done_message(&done, &st);
    assert_eq!(message.usage.input, 20, "prompt minus the cached prefix");
    assert_eq!(message.usage.cache_read, 100);
    assert_eq!(message.usage.output, 30);
}

/// Some compatible providers put usage on the choice instead of the envelope
/// (the reference names Moonshot). Both shapes must bill.
#[test]
fn openai_choice_level_usage_is_read() {
    let w = OpenAiCompatWire {
        base_url: "https://api.moonshot.cn/v1".to_string(),
        env_key: None,
    };
    let mut st = HttpEventStream::empty_for_tests();
    let frame = r#"{"choices":[{"index":0,"delta":{},"usage":{"prompt_tokens":12,"completion_tokens":3}}]}"#;
    let _ = w.on_event(&ev(frame), &mut st);
    let done = w.on_event(&ev("[DONE]"), &mut st).unwrap();
    let usage = done_message(&done, &st).usage;
    assert_eq!((usage.input, usage.output), (12, 3));
}

#[test]
fn openai_content_filter_maps_to_error() {
    let w = OpenAiCompatWire {
        base_url: "https://api.openai.com/v1".to_string(),
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

/// A blocked prompt can carry `usageMetadata` with no candidates; the billed
/// input must survive, and no text event is invented.
#[test]
fn gemini_usage_metadata_without_a_candidate_is_still_counted() {
    let w = GoogleWire;
    let mut st = HttpEventStream::empty_for_tests();
    let frame = r#"{"usageMetadata":{"promptTokenCount":77,"candidatesTokenCount":0,"totalTokenCount":77},"promptFeedback":{"blockReason":"SAFETY"}}"#;
    assert!(w.on_event(&ev(frame), &mut st).is_none());
    let done = w
        .on_event(&ev(r#"{"candidates":[{"finishReason":"STOP"}]}"#), &mut st)
        .unwrap();
    let usage = done_message(&done, &st).usage;
    assert_eq!(usage.input, 77, "the blocked prompt was billed");
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
        base_url: "https://api.openai.com/v1".to_string(),
        env_key: None,
    };
    // OpenAI-style family (catalog row, standard knob).
    let gpt = w
        .request_body("gpt-5.6", &thinking_ctx(ThinkingLevel::XHigh))
        .unwrap();
    assert_eq!(gpt["reasoning_effort"], "xhigh");
    // OpenRouter rows nest the effort — keyed on the row's provider, not an
    // `openrouter/` id prefix (Wave D: no catalog row carries that prefix,
    // so the old substring branch never fired for real traffic).
    let or = w
        .request_body("openai/gpt-5-mini", &thinking_ctx(ThinkingLevel::Medium))
        .unwrap();
    assert_eq!(or["reasoning"]["effort"], "medium");
    assert!(or.get("reasoning_effort").is_none(), "{or}");
    // Off omits everything.
    let off = w
        .request_body("gpt-5.6", &thinking_ctx(ThinkingLevel::Off))
        .unwrap();
    assert!(off.get("reasoning_effort").is_none());
    assert!(off.get("reasoning").is_none());

    // DeepSeek reasoning rows: switch + effort. Uncatalogued ids (the old
    // bare `deepseek-chat` included — the registry retired that row) take
    // the default branch: the flat knob, no switch.
    let ds = w
        .request_body("deepseek-v4-pro", &thinking_ctx(ThinkingLevel::High))
        .unwrap();
    assert_eq!(ds["thinking"]["type"], "enabled");
    assert_eq!(ds["reasoning_effort"], "high");
    let ds_chat = w
        .request_body("deepseek-chat", &thinking_ctx(ThinkingLevel::High))
        .unwrap();
    assert_eq!(ds_chat["reasoning_effort"], "high");
    assert!(
        ds_chat.get("thinking").is_none(),
        "uncatalogued ids must not take the thinking switch"
    );

    // Z.ai GLM family: enable + clear_thinking=false + effort.
    let glm = w
        .request_body("glm-5.2", &thinking_ctx(ThinkingLevel::Medium))
        .unwrap();
    assert_eq!(glm["thinking"]["type"], "enabled");
    assert_eq!(glm["thinking"]["clear_thinking"], false);
    assert_eq!(glm["reasoning_effort"], "medium");

    // Kimi thinking rows arm the knob (nested, via their OpenRouter row);
    // plain native kimi rows stay clean.
    let kimi_t = w
        .request_body(
            "moonshotai/kimi-k2-thinking",
            &thinking_ctx(ThinkingLevel::Low),
        )
        .unwrap();
    assert_eq!(kimi_t["reasoning"]["effort"], "low");
    let kimi_plain = w
        .request_body("kimi-k3", &thinking_ctx(ThinkingLevel::Low))
        .unwrap();
    assert!(
        kimi_plain.get("reasoning_effort").is_none(),
        "non-thinking kimi rows must not carry the knob"
    );
    assert!(kimi_plain.get("reasoning").is_none(), "{kimi_plain}");
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

/// M60: the stall watchdog's salvage rules — what a silent stream leaves behind.
/// Drives the accumulator exactly as `WireStream` does when its idle timer trips.
#[test]
fn stall_after_text_keeps_the_prose_and_flags_the_turn_failed() {
    let w = OpenAiCompatWire {
        base_url: "https://api.openai.com/v1".to_string(),
        env_key: None,
    };
    let mut st = HttpEventStream::new("gpt-x", "openai", false);
    let payload = r#"{"choices":[{"delta":{"content":"half an answ"}}]}"#;
    assert!(w.on_event(&ev(payload), &mut st).is_some());

    let message = st.stall_message(120);
    assert_eq!(message.text(), "half an answ", "the model's work is kept");
    assert_eq!(message.stop_reason, StopReason::Error);
    let detail = message.error_message.expect("stall explains itself");
    assert!(
        detail.contains("timed out") && detail.contains("120s"),
        "{detail}"
    );
    // ...and the retry policy can act on it (the bug the first test round caught:
    // "timed out" must classify as a network failure, not as `Other`).
    assert_eq!(
        epsilon_llm::classify(&detail),
        epsilon_llm::FailureKind::Network
    );
}

#[test]
fn stall_with_a_tool_call_in_flight_never_returns_the_call() {
    let w = OpenAiCompatWire {
        base_url: "https://api.openai.com/v1".to_string(),
        env_key: None,
    };
    let mut st = HttpEventStream::new("gpt-x", "openai", false);
    // Open a tool call and leave its arguments unfinished — the classic stall shape.
    let payload = r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"bash","arguments":"{\"command\":"}}]}}]}"#;
    // `on_event` only returns the events *newly* produced for a batch — the wire
    // emits Start lazily, so the contract here is the accumulator state, not the
    // return value (same convention as the fixtures above).
    let _ = w.on_event(&ev(payload), &mut st);
    assert!(
        st.has_open_or_closed_tools(),
        "precondition: a tool call is in flight"
    );

    let message = st.stall_message(120);
    assert_eq!(message.stop_reason, StopReason::Error);
    assert!(
        message.tool_calls().count() == 0,
        "half-written arguments must never become an executed call"
    );
}

// ---- M70: provider-facing sanitation at the wire boundary ---------------------------------

/// Three calls in one message, all with the same id. Providers reject duplicates, and the result
/// pairing becomes ambiguous — the second and third are renamed deterministically.
#[test]
fn duplicate_tool_call_ids_in_one_message_are_uniquified() {
    let w = OpenAiCompatWire {
        base_url: "https://api.openai.com/v1".to_string(),
        env_key: None,
    };
    let mut st = HttpEventStream::new("gpt-x", "openai", false);
    let payload = r#"{"choices":[{"index":0,"delta":{"tool_calls":[
        {"index":0,"id":"call_same","function":{"name":"bash","arguments":"{}"}},
        {"index":1,"id":"call_same","function":{"name":"read","arguments":"{}"}},
        {"index":2,"id":"call_same","function":{"name":"ls","arguments":"{}"}}
    ]}}]}"#;
    let _ = w.on_event(&ev(payload), &mut st);
    let fin = r#"{"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}"#;
    let _ = w.on_event(&ev(fin), &mut st);
    let events = w.on_event(&ev("[DONE]"), &mut st).unwrap();
    let message = done_message(&events, &st);

    let ids: Vec<String> = message
        .tool_calls()
        .map(|(id, _, _)| id.to_string())
        .collect();
    let unique: std::collections::HashSet<&String> = ids.iter().collect();
    assert_eq!(ids.len(), 3, "all three calls survive: {ids:?}");
    assert_eq!(unique.len(), 3, "and they are distinguishable: {ids:?}");
    assert_eq!(ids[0], "call_same", "the first keeps its id");
    assert_eq!(ids[1], "call_same#2", "the second is suffixed");
    assert_eq!(ids[2], "call_same#3");
}

/// A model that emits a trailing comma: the arguments used to become `{}`, so the tool ran with
/// nothing. They are now recovered, with the meaning the model wrote.
#[test]
fn malformed_arguments_are_repaired_rather_than_emptied() {
    let w = OpenAiCompatWire {
        base_url: "https://api.openai.com/v1".to_string(),
        env_key: None,
    };
    let mut st = HttpEventStream::new("gpt-x", "openai", false);
    let payload = r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"c1","function":{"name":"read","arguments":"{\"path\": \"src/lib.rs\",}"}}]}}]}"#;
    let _ = w.on_event(&ev(payload), &mut st);
    let fin = r#"{"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}"#;
    let _ = w.on_event(&ev(fin), &mut st);
    let events = w.on_event(&ev("[DONE]"), &mut st).unwrap();
    let message = done_message(&events, &st);

    let args: Vec<serde_json::Value> = message.tool_calls().map(|(_, _, a)| a.clone()).collect();
    assert_eq!(args.len(), 1);
    assert_eq!(
        args[0]["path"], "src/lib.rs",
        "the path the model wrote survives: {args:?}"
    );
}

/// Arguments that cannot be read at all are handed on as written, so the tool's failure can quote
/// them. Before M70 this became `{}` and the model was told it forgot a field.
#[test]
fn unreadable_arguments_are_preserved_for_the_failure_to_quote() {
    let w = OpenAiCompatWire {
        base_url: "https://api.openai.com/v1".to_string(),
        env_key: None,
    };
    let mut st = HttpEventStream::new("gpt-x", "openai", false);
    let payload = r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"c1","function":{"name":"bash","arguments":"{unquoted: keys}"}}]}}]}"#;
    let _ = w.on_event(&ev(payload), &mut st);
    let fin = r#"{"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}"#;
    let _ = w.on_event(&ev(fin), &mut st);
    let events = w.on_event(&ev("[DONE]"), &mut st).unwrap();
    let message = done_message(&events, &st);

    let args: Vec<serde_json::Value> = message.tool_calls().map(|(_, _, a)| a.clone()).collect();
    assert_eq!(
        args[0].as_str(),
        Some("{unquoted: keys}"),
        "the payload is kept verbatim for the error message: {args:?}"
    );
}

#[test]
fn stall_with_no_output_is_a_plain_network_failure() {
    let mut st = HttpEventStream::new("gpt-x", "openai", false);
    let message = st.stall_message(90);
    assert_eq!(message.stop_reason, StopReason::Error);
    assert!(message.text().is_empty());
    assert!(
        message
            .error_message
            .as_deref()
            .is_some_and(|m| m.contains("90s"))
    );
}

/// M62: what the user already watched arrive survives the failure.
#[test]
fn an_error_frame_after_deltas_keeps_the_deltas() {
    let w = OpenAiCompatWire {
        base_url: "https://api.openai.com/v1".to_string(),
        env_key: None,
    };
    let mut st = HttpEventStream::new("gpt-x", "openai", false);
    let _ = w.on_event(
        &ev(r#"{"choices":[{"index":0,"delta":{"content":"the answer so f"}}]}"#),
        &mut st,
    );
    let batch = w
        .on_event(&ev(r#"{"error":{"message":"upstream exploded"}}"#), &mut st)
        .expect("the error frame terminates the stream");
    let StreamEvent::Done { message } = batch.last().expect("a terminal event") else {
        panic!("expected Done, got {batch:?}");
    };
    assert_eq!(
        message.text(),
        "the answer so f",
        "visible output is never discarded by a later failure"
    );
    assert_eq!(message.stop_reason, StopReason::Error);
    assert!(
        message
            .error_message
            .as_deref()
            .is_some_and(|m| m.contains("upstream exploded")),
        "the failure still explains itself: {message:?}"
    );
}

/// The same rule for the anthropic wire (its error frame is a typed event).
#[test]
fn an_anthropic_error_event_after_thinking_keeps_the_thinking() {
    let w = AnthropicWire;
    let mut st = HttpEventStream::new("claude-x", "anthropic", false);
    let _ = w.on_event(
        &ev(r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"weighing options"}}"#),
        &mut st,
    );
    let batch = w
        .on_event(
            &ev(r#"{"type":"error","error":{"message":"overloaded"}}"#),
            &mut st,
        )
        .expect("terminates");
    let StreamEvent::Done { message } = batch.last().expect("terminal") else {
        panic!("expected Done, got {batch:?}");
    };
    let thinking: String = message
        .content
        .iter()
        .filter_map(|b| match b {
            epsilon_contracts::content::ContentBlock::Thinking { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        thinking.contains("weighing options"),
        "salvaged thinking: {message:?}"
    );
    assert_eq!(message.stop_reason, StopReason::Error);
}
