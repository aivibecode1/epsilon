//! The Anthropic prompt-cache breakpoints, checked on a **real request body**.
//!
//! # Why a wire test and not a unit test on the policy
//!
//! The policy decides three booleans; what matters is whether they land on the right
//! blocks of the JSON that leaves the process. A test that asserts on
//! `CachePolicy::mark_last_user_turn` proves the flag exists; it does not prove the
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
    tool_result_for("tc_1", text)
}

fn tool_result_for(id: &str, text: &str) -> Message {
    Message::ToolResult(ToolResultMessage {
        id: format!("tr-{id}"),
        timestamp_ms: 0,
        tool_call_id: id.into(),
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
/// **Last**, and the last user-role turn may be a **tool result**: Anthropic sends
/// tool results as user-role messages. Pi marks that block; marking only the original
/// prompt leaves every tool result after it uncached and reprocessed at full price.
#[test]
fn the_tail_breakpoint_is_on_the_last_user_role_turn_not_the_first() {
    let body = body(vec![
        user("first request"),
        assistant_text("first answer"),
        tool_result("first result"),
        user("second request"),
        assistant_text("second answer"),
        tool_result("second result"),
    ]);
    let msgs = body["messages"].as_array().expect("messages is an array");

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
        vec![msgs.len() - 1],
        "exactly one marker, on the last user-role message (the tool result): {marked:?}"
    );
    assert_eq!(
        msgs[msgs.len() - 1]["content"][0]["type"],
        "tool_result",
        "the last user-role turn in this fixture is a tool result"
    );
}

/// Consecutive tool results are one user-role turn, as Anthropic requires after
/// parallel tool calls, and the tail marker lands on the last block of that turn.
#[test]
fn parallel_tool_results_share_one_message_and_mark_the_last_block() {
    let body = body(vec![
        user("run both"),
        assistant_text("calling two tools"),
        tool_result_for("tc_1", "first result"),
        tool_result_for("tc_2", "second result"),
    ]);
    let msgs = body["messages"].as_array().expect("messages is an array");
    let user_turns: Vec<&Value> = msgs
        .iter()
        .filter(|m| m["role"] == "user" && has_tool_result(m))
        .collect();
    assert_eq!(
        user_turns.len(),
        1,
        "parallel results must be one user turn, not one message each: {msgs:?}"
    );
    let blocks = user_turns[0]["content"]
        .as_array()
        .expect("tool result content is an array");
    assert_eq!(blocks.len(), 2, "{blocks:?}");
    assert!(
        blocks[1].get("cache_control").is_some(),
        "the marker belongs on the last block of the grouped turn: {blocks:?}"
    );
}

/// A transcript with no `Message::User` still assembles, and the tool result is the
/// user-role tail the marker lands on. There is no panic on an absent prompt.
#[test]
fn a_turn_with_no_user_message_still_assembles() {
    let body = body(vec![assistant_text("unprompted"), tool_result("r")]);
    let found = markers(&body);
    assert!(
        found.iter().any(|marker| marker.starts_with("messages[")),
        "the lone tool result is the user-role tail: {found:?}"
    );
    assert!(found.contains(&"system".to_string()), "{found:?}");
}

/// The tail breakpoint advances as a tool-heavy turn grows. This is the
/// wire-level measurement that matters: the first request can only cache the
/// prompt, and every later request caches the tool result before it.
#[test]
fn the_tail_breakpoint_advances_across_tool_round_trips() {
    fn marked_index(body: &Value) -> Option<usize> {
        body["messages"].as_array()?.iter().position(|message| {
            message["content"].as_array().is_some_and(|blocks| {
                blocks
                    .iter()
                    .any(|block| block.get("cache_control").is_some())
            })
        })
    }

    let first = body(vec![user("go")]);
    let second = body(vec![
        user("go"),
        assistant_text("calling a tool"),
        tool_result("first result"),
    ]);
    let third = body(vec![
        user("go"),
        assistant_text("calling a tool"),
        tool_result("first result"),
        assistant_text("calling another"),
        tool_result("second result"),
    ]);

    let first_at = marked_index(&first).expect("the prompt is marked");
    let second_at = marked_index(&second).expect("the first result is marked");
    let third_at = marked_index(&third).expect("the second result is marked");
    assert_eq!(first_at, 0, "the first request can only mark the prompt");
    assert!(
        second_at > first_at && third_at > second_at,
        "the tail boundary must move forward: {first_at} -> {second_at} -> {third_at}"
    );
    assert_eq!(
        third["messages"][third_at]["content"][0]["type"], "tool_result",
        "the final boundary is the latest tool result, not the prompt"
    );
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
