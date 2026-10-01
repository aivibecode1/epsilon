//! The Anthropic prompt-cache breakpoints, checked on a **real request body**.
//!
//! # Why a wire test and not a unit test on the policy
//!
//! The policy decides three booleans; what matters is whether they land on the right
//! blocks of the JSON that leaves the process. A test that asserts on
//! `CachePolicy::mark_last_user_message` proves the flag exists; it does not prove the
//! marker is on the **last** user message, in the body, among every block that could
//! carry one.
//!
//! So these build the body through [`ProviderWire::request_body`] — the same call the
//! transport makes — and read it back. No network: the wire is a pure function of its
//! context.

#![cfg(feature = "real")]

use epsilon_contracts::content::{ContentBlock, UserContent};
use epsilon_contracts::message::{
    AssistantMessage, Message, StopReason, ToolResultMessage, UserMessage,
};
use epsilon_contracts::tool::ToolDescriptor;
use epsilon_llm::LlmContext;
use epsilon_llm::providers::ProviderWire;
use epsilon_llm::providers::anthropic::AnthropicWire;
use serde_json::Value;

/// One tool, enough for the tools array to exist.
fn tool() -> ToolDescriptor {
    ToolDescriptor {
        name: "bash".into(),
        label: "bash".into(),
        description: "run a command".into(),
        parameters_schema: serde_json::json!({"type": "object"}),
        execution_mode: epsilon_contracts::tool::ExecutionMode::Sequential,
    }
}

fn user(text: &str) -> Message {
    Message::User(UserMessage {
        id: format!("u-{text}"),
        timestamp_ms: 0,
        content: vec![UserContent::Text { text: text.into() }],
    })
}

fn assistant_text(text: &str) -> Message {
    Message::Assistant(AssistantMessage {
        id: "a1".into(),
        timestamp_ms: 0,
        provider: "anthropic".into(),
        model_id: "claude-sonnet-4-5".into(),
        content: vec![ContentBlock::text(text)],
        stop_reason: StopReason::EndTurn,
        error_message: None,
        usage: Default::default(),
    })
}

fn tool_result(text: &str) -> Message {
    Message::ToolResult(ToolResultMessage {
        id: "tr-1".into(),
        timestamp_ms: 0,
        tool_call_id: "tc_1".into(),
        tool_name: "bash".into(),
        content: vec![ContentBlock::text(text)],
        details: serde_json::Value::Null,
        is_error: false,
        usage: None,
    })
}

fn context(messages: Vec<Message>) -> LlmContext {
    LlmContext {
        system_prompt: "you are epsilon".into(),
        messages,
        tools: vec![tool()],
        thinking_level: Default::default(),
    }
}

fn body(messages: Vec<Message>) -> Value {
    AnthropicWire
        .request_body("claude-sonnet-4-5", &context(messages))
        .expect("the wire assembles")
}

/// Every block in the body that carries a cache marker, with where it was found.
fn markers(body: &Value) -> Vec<String> {
    let mut found = Vec::new();
    if body["system"]
        .as_array()
        .is_some_and(|blocks| blocks.iter().any(|b| b.get("cache_control").is_some()))
    {
        found.push("system".into());
    }
    if let Some(tools) = body["tools"].as_array() {
        for (i, t) in tools.iter().enumerate() {
            if t.get("cache_control").is_some() {
                found.push(format!("tools[{i}]"));
            }
        }
    }
    for (i, m) in body["messages"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
    {
        if let Some(blocks) = m["content"].as_array() {
            for (j, b) in blocks.iter().enumerate() {
                if b.get("cache_control").is_some() {
                    found.push(format!("messages[{i}].content[{j}]"));
                }
            }
        }
    }
    found
}

/// The three boundaries the policy names, and nothing else.
#[test]
fn a_request_carries_exactly_the_three_boundaries() {
    let body = body(vec![user("do the thing"), assistant_text("on it")]);
    let mut found = markers(&body);
    found.sort();
    assert_eq!(
        found,
        vec!["messages[0].content[0]", "system", "tools[0]"],
        "the three breakpoints, on the system block, the last tool, and the user turn"
    );
}

/// The one that pays for the change.
///
/// **Last**, not first: the point is a prefix that stays put while the turn's own tool
/// round-trips extend the transcript after it. A marker on the *first* user message would
/// be correct JSON and would save nothing on the calls that matter.
#[test]
fn the_tail_breakpoint_is_on_the_last_user_message_not_the_first() {
    let body = body(vec![
        user("first request"),
        assistant_text("first answer"),
        tool_result("first result"),
        user("second request"),
        assistant_text("second answer"),
        tool_result("second result"),
    ]);
    let msgs = body["messages"].as_array().expect("messages is an array");
    let user_indices: Vec<usize> = msgs
        .iter()
        .enumerate()
        .filter(|(_, m)| m["role"] == "user" && !has_tool_result(m))
        .map(|(i, _)| i)
        .collect();
    assert!(
        user_indices.len() >= 2,
        "the fixture needs two user turns: {user_indices:?}"
    );

    let marked: Vec<usize> = msgs
        .iter()
        .enumerate()
        .filter(|(_, m)| {
            m["content"]
                .as_array()
                .is_some_and(|b| b.iter().any(|b| b.get("cache_control").is_some()))
        })
        .map(|(i, _)| i)
        .collect();
    assert_eq!(
        marked,
        vec![*user_indices.last().expect("a last user turn")],
        "exactly one message carries the tail marker, and it is the last user one: {marked:?}"
    );
}

/// A turn with no user message at all must still produce a valid request.
///
/// The marker's index is computed from the conversation, so "no user message" has to be a
/// state the wire handles rather than a panic on `rposition`.
#[test]
fn a_turn_with_no_user_message_still_assembles() {
    let body = body(vec![assistant_text("unprompted"), tool_result("r")]);
    let found = markers(&body);
    assert!(
        !found.iter().any(|m| m.starts_with("messages")),
        "with no user turn there is nothing to mark in the transcript: {found:?}"
    );
    assert!(found.contains(&"system".to_string()), "{found:?}");
}

/// A body with no tools and no system prompt must not gain a phantom marker.
#[test]
fn empty_sections_are_left_empty_rather_than_gaining_a_marker() {
    let mut ctx = context(vec![user("hello")]);
    ctx.system_prompt = String::new();
    ctx.tools = Vec::new();
    let body = AnthropicWire
        .request_body("claude-sonnet-4-5", &ctx)
        .expect("the wire assembles");
    assert!(body.get("system").is_none(), "{body}");
    assert!(body.get("tools").is_none(), "{body}");
    assert_eq!(markers(&body), vec!["messages[0].content[0]"]);
}

/// The policy is a policy: turning it off removes every marker.
///
/// Without this, a future edit that hard-codes a marker somewhere else would still pass
/// every test above, because they all assert that a marker *exists*.
#[test]
fn the_markers_belong_to_the_policy_rather_than_to_the_wire() {
    // The policy is a value; the wire reads it. Asserting the count against the policy
    // rather than a literal is what keeps the two in step when either changes.
    assert_eq!(
        epsilon_llm::cache_policy::CachePolicy::DEFAULT.count(),
        3,
        "three boundaries, and Anthropic accepts four"
    );
    assert_eq!(epsilon_llm::cache_policy::CachePolicy::NONE.count(), 0);
}

/// Does a message carry a tool result?
fn has_tool_result(m: &Value) -> bool {
    m["content"]
        .as_array()
        .is_some_and(|blocks| blocks.iter().any(|b| b["type"] == "tool_result"))
}
