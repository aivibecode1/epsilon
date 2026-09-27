//! M70: the interrupted-sequence repair, through the real loop.
//!
//! A run can end between the assistant's tool-call block and its results — an abort, a crash, a
//! `Ctrl+C` at the wrong moment. The transcript is then structurally invalid for the next
//! request: providers validate that every tool call has a result, so the error arrives one
//! request later, far from its cause, as a 400 nobody can explain.
//!
//! `epsilon_session::repair::repair_on_open` closes that shape for a *reopened* file. These
//! scenarios cover the same shape in a live process, which is what `run()` has to survive.

use std::sync::{Arc, Mutex};

use epsilon_contracts::content::{ContentBlock, UserContent};
use epsilon_contracts::message::{
    AssistantMessage, Message, StopReason, ToolResultMessage, Usage, UserMessage,
};
use epsilon_contracts::session_entry::EntryKind;
use epsilon_core::{AutoAllow, LoopLimits, ToolRegistry};
use epsilon_llm::{FauxProvider, FauxStep};
use epsilon_session::SessionStore;

fn user(text: &str) -> Message {
    Message::User(UserMessage {
        id: epsilon_contracts::new_id("msg"),
        timestamp_ms: 0,
        content: vec![UserContent::Text {
            text: text.to_string(),
        }],
    })
}

/// An assistant turn that asked for two tools and never got their results.
fn assistant_with_dangling_calls() -> Message {
    Message::Assistant(AssistantMessage {
        id: epsilon_contracts::new_id("msg"),
        timestamp_ms: 0,
        provider: "faux".into(),
        model_id: "faux-1".into(),
        content: vec![
            ContentBlock::ToolCall {
                id: "call_a".into(),
                name: "bash".into(),
                arguments: serde_json::json!({"command": "cargo test"}),
            },
            ContentBlock::ToolCall {
                id: "call_b".into(),
                name: "read".into(),
                arguments: serde_json::json!({"path": "src/lib.rs"}),
            },
        ],
        stop_reason: StopReason::ToolUse,
        error_message: None,
        usage: Usage::default(),
    })
}

fn agent() -> Arc<epsilon_core::Agent> {
    epsilon_core::Agent::new(
        LoopLimits::default(),
        Arc::new(epsilon_llm::PanicGuard(FauxProvider::new(vec![
            FauxStep::say("carrying on"),
        ]))),
        Arc::new(ToolRegistry::new()),
        Arc::new(AutoAllow),
        std::env::temp_dir(),
    )
}

/// Every tool call in the transcript has exactly one result.
fn every_call_has_a_result(history: &[Message]) -> Result<(), String> {
    let mut open: Vec<String> = Vec::new();
    for m in history {
        match m {
            Message::Assistant(a) => {
                for (id, _, _) in a.tool_calls() {
                    open.push(id.to_string());
                }
            }
            Message::ToolResult(t) => open.retain(|id| id != &t.tool_call_id),
            _ => {}
        }
    }
    if open.is_empty() {
        Ok(())
    } else {
        Err(format!("unanswered tool calls: {open:?}"))
    }
}

#[tokio::test]
async fn a_run_that_starts_on_a_dangling_call_closes_it_before_the_next_request() {
    let agent = agent();
    // The state an interrupted run leaves behind: a user turn, an assistant turn asking for two
    // tools, and no results (the process died, or the user aborted).
    agent.restore_history(vec![
        user("make the tests pass"),
        assistant_with_dangling_calls(),
    ]);

    agent.run(user("what happened?")).await.expect("run");

    let history = agent.history();
    every_call_has_a_result(&history).expect("the transcript is valid after the repair");

    let synthesized: Vec<&ToolResultMessage> = history
        .iter()
        .filter_map(|m| match m {
            Message::ToolResult(t) if t.tool_name == "bash" || t.tool_name == "read" => Some(t),
            _ => None,
        })
        .collect();
    assert_eq!(synthesized.len(), 2, "both calls were closed");
    for t in &synthesized {
        assert!(
            t.is_error,
            "a call that never ran is reported as an error, not a success"
        );
        let text = t
            .content
            .iter()
            .filter_map(|c| match c {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            text.contains("never executed"),
            "and it says what happened: {text}"
        );
        assert_eq!(
            t.details["interrupted"], true,
            "with a machine-readable marker for frontends"
        );
    }
    // The results are paired with the calls they answer, in order.
    let ids: Vec<&str> = synthesized
        .iter()
        .map(|t| t.tool_call_id.as_str())
        .collect();
    assert_eq!(ids, vec!["call_a", "call_b"]);
}

#[tokio::test]
async fn a_healthy_transcript_is_left_alone() {
    let agent = agent();
    // The same shape, but with results — nothing to repair, and nothing may be added.
    let mut answered = assistant_with_dangling_calls();
    let Message::Assistant(a) = &mut answered else {
        unreachable!()
    };
    let results: Vec<Message> = a
        .tool_calls()
        .map(|(id, name, _)| {
            Message::ToolResult(ToolResultMessage {
                id: epsilon_contracts::new_id("msg"),
                timestamp_ms: 0,
                tool_call_id: id.to_string(),
                tool_name: name.to_string(),
                content: vec![ContentBlock::text("all good")],
                details: serde_json::json!({}),
                usage: None,
                is_error: false,
            })
        })
        .collect();
    let mut history = vec![user("make the tests pass"), answered];
    history.extend(results);
    agent.restore_history(history.clone());

    let before = history
        .iter()
        .filter(|m| matches!(m, Message::ToolResult(_)))
        .count();
    agent.run(user("thanks")).await.expect("run");

    let after = agent.history();
    let results = after
        .iter()
        .filter(|m| matches!(m, Message::ToolResult(_)))
        .count();
    assert_eq!(
        results, before,
        "no result was invented for calls that already had one: {after:#?}"
    );
    every_call_has_a_result(&after).expect("still valid");
}

#[tokio::test]
async fn a_second_run_does_not_close_the_same_calls_twice() {
    let agent = agent();
    agent.restore_history(vec![
        user("make the tests pass"),
        assistant_with_dangling_calls(),
    ]);

    agent.run(user("first")).await.expect("run");
    let after_first = agent.history().len();
    agent.run(user("second")).await.expect("run");

    // The second run must not add another pair of results for calls already answered.
    let results = agent
        .history()
        .iter()
        .filter(|m| matches!(m, Message::ToolResult(_)))
        .count();
    assert_eq!(results, 2, "the repair is idempotent");
    assert!(
        agent.history().len() > after_first,
        "the run itself did happen"
    );
}

/// Ids of the tool calls in `view` that nothing answered.
fn open_call_ids(view: &[Message]) -> Vec<String> {
    let mut open: Vec<String> = Vec::new();
    for m in view {
        match m {
            Message::Assistant(a) => {
                for (id, _, _) in a.tool_calls() {
                    open.push(id.to_string());
                }
            }
            Message::ToolResult(t) => open.retain(|id| id != &t.tool_call_id),
            _ => {}
        }
    }
    open
}

/// Every result recorded for one tool call id.
fn results_for<'a>(view: &'a [Message], call_id: &str) -> Vec<&'a ToolResultMessage> {
    view.iter()
        .filter_map(|m| match m {
            Message::ToolResult(t) if t.tool_call_id == call_id => Some(t),
            _ => None,
        })
        .collect()
}

fn text_of(t: &ToolResultMessage) -> String {
    t.content
        .iter()
        .filter_map(|c| match c {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// G1: the repair is written through the store, and only for calls that need it.
///
/// The three scenarios above seed runtime history directly and never open a session file, so
/// the half of the claim that matters most is asserted by nothing: "this call was never
/// executed" is a sentence about the *store*. It can only be true if the assistant's
/// tool-call block was appended before the tool ran — the repair does not consult the file, so
/// the only way a later run can state it is that the file already held the call.
///
/// So this scenario writes the interrupted turn through a real `SessionStore`, resumes the
/// agent from that store's own projection (what `--resume` does), and then requires the run's
/// repair to arrive in **both** views. `persist` is what keeps the transcript and the store
/// projection together, and the next request is built from the store — a repair that reached
/// only the transcript would leave the provider sending the dangling call unchanged.
///
/// The answered call rides in the same assistant message, so a repair that closed everything
/// would fail here instead of passing by over-shooting. Over-eager is as much a bug as absent.
#[tokio::test]
async fn the_repair_is_durable_and_leaves_the_answered_call_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.jsonl");
    let store = Arc::new(Mutex::new(
        SessionStore::open_in(&path, dir.path()).expect("open session store"),
    ));

    // What an interrupted run leaves on disk: one assistant turn asking for two tools, only
    // one of which was answered. This append is the property under test — it happens before
    // any run, and nothing after it can invent the call if it were missing.
    let answered_id = epsilon_contracts::new_id("msg");
    {
        let mut st = store.lock().unwrap();
        let interrupted: [Message; 3] = [
            user("make the tests pass"),
            Message::Assistant(AssistantMessage {
                id: epsilon_contracts::new_id("msg"),
                timestamp_ms: 0,
                provider: "faux".into(),
                model_id: "faux-1".into(),
                content: vec![
                    ContentBlock::ToolCall {
                        id: "call_dangling".into(),
                        name: "bash".into(),
                        arguments: serde_json::json!({"command": "cargo test"}),
                    },
                    ContentBlock::ToolCall {
                        id: "call_answered".into(),
                        name: "read".into(),
                        arguments: serde_json::json!({"path": "src/lib.rs"}),
                    },
                ],
                stop_reason: StopReason::ToolUse,
                error_message: None,
                usage: Usage::default(),
            }),
            Message::ToolResult(ToolResultMessage {
                id: answered_id.clone(),
                timestamp_ms: 0,
                tool_call_id: "call_answered".into(),
                tool_name: "read".into(),
                content: vec![ContentBlock::text("src/lib.rs contents")],
                details: serde_json::json!({}),
                usage: None,
                is_error: false,
            }),
        ];
        for m in interrupted {
            st.append(EntryKind::Message {
                message: m,
                api_request_sha256: None,
            })
            .expect("append");
        }
    }

    let agent = agent();
    agent.attach_session_store(Arc::clone(&store));
    // Resume the way `--resume` does: runtime history is seeded from the store's projection,
    // not from a literal, so the call the repair is about is the one that is on disk.
    let restored: Vec<Message> = store
        .lock()
        .unwrap()
        .model_view()
        .expect("model view")
        .into_iter()
        .map(|v| v.into_message())
        .collect();
    assert_eq!(
        open_call_ids(&restored),
        vec!["call_dangling".to_string()],
        "precondition: the interrupted call survived a write and a read-back with no result, \
         which is the only thing that makes a later 'never executed' true rather than a guess"
    );
    agent.restore_history(restored);

    agent.run(user("what happened?")).await.expect("run");

    let history = agent.history();
    let stored: Vec<Message> = store
        .lock()
        .unwrap()
        .model_view()
        .expect("model view")
        .into_iter()
        .map(|v| v.into_message())
        .collect();

    for (label, view) in [("transcript", &history), ("store", &stored)] {
        let closed = results_for(view, "call_dangling");
        assert_eq!(
            closed.len(),
            1,
            "{label}: the unanswered call got exactly one result"
        );
        let t = closed[0];
        assert!(
            t.is_error,
            "{label}: a call that never ran is an error, not a pretend success"
        );
        assert_eq!(
            t.details["interrupted"], true,
            "{label}: with the machine-readable marker a frontend renders"
        );
        assert!(
            text_of(t).contains("never executed"),
            "{label}: and it says what happened: {}",
            text_of(t)
        );

        let kept = results_for(view, "call_answered");
        assert_eq!(
            kept.len(),
            1,
            "{label}: no result was invented for a call that already had one"
        );
        assert_eq!(
            kept[0].id, answered_id,
            "{label}: the original result is the one that survives, not a replacement"
        );
        assert!(
            !kept[0].is_error,
            "{label}: and it is still the success it was"
        );
    }

    // The repair must be the *same message* in both views. Two different results would mean
    // the transcript and the projection have diverged, which is the one thing `persist`
    // exists to prevent — and the projection is the view the provider is handed.
    assert_eq!(
        results_for(&history, "call_dangling")[0],
        results_for(&stored, "call_dangling")[0],
        "the transcript and the store carry one repair, not two"
    );
    every_call_has_a_result(&stored).expect("the store's projection is valid after the repair");
}
