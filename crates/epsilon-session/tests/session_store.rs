//! Integration coverage for the session brick: append/read, tree navigation, model-view
//! shadowing, crash repair, and lease exclusivity.

use epsilon_contracts::agent_event::TurnEndReason;
use epsilon_contracts::content::{ContentBlock, UserContent};
use epsilon_contracts::message::{
    AssistantMessage, Message, StopReason, ToolResultMessage, Usage, UserMessage,
};
use epsilon_contracts::new_id;
use epsilon_contracts::session_entry::EntryKind;
use epsilon_session::repair::repair_on_open;
use epsilon_session::{ModelViewEntry, SessionLease, SessionStore};

fn user_msg(text: &str) -> Message {
    Message::User(UserMessage {
        id: new_id("msg"),
        timestamp_ms: 0,
        content: vec![UserContent::Text {
            text: text.to_string(),
        }],
    })
}

fn tool_call_msg(id: &str, name: &str) -> Message {
    Message::Assistant(AssistantMessage {
        id: new_id("msg"),
        timestamp_ms: 0,
        provider: "faux".into(),
        model_id: "faux-1".into(),
        content: vec![ContentBlock::ToolCall {
            id: id.into(),
            name: name.into(),
            arguments: serde_json::json!({}),
        }],
        stop_reason: StopReason::ToolUse,
        error_message: None,
        usage: Usage::default(),
    })
}

fn tool_result_msg(call_id: &str, name: &str) -> Message {
    Message::ToolResult(ToolResultMessage {
        id: new_id("msg"),
        timestamp_ms: 0,
        tool_call_id: call_id.into(),
        tool_name: name.into(),
        content: vec![ContentBlock::text("ok")],
        details: serde_json::json!({}),
        usage: None,
        is_error: false,
    })
}

#[test]
fn append_read_roundtrip_preserves_entries() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.jsonl");

    let mut store = SessionStore::open(&path).unwrap();
    let e0 = store
        .append(EntryKind::Message {
            message: user_msg("hello"),
            api_request_sha256: None,
        })
        .unwrap();
    let e1 = store
        .append(EntryKind::TurnEnd {
            reason: TurnEndReason::Completed,
        })
        .unwrap();

    assert_eq!(e0.seq, 0);
    assert_eq!(e0.parent_id, None);
    assert_eq!(e1.parent_id.as_deref(), Some(e0.id.as_str()));

    // Reopen from disk; everything must survive.
    drop(store);
    let reopened = SessionStore::open(&path).unwrap();
    assert_eq!(reopened.len(), 2);
    let all = reopened.read_all().unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].id, e0.id);
}

#[test]
fn navigate_and_branch_work() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = SessionStore::open(dir.path().join("s.jsonl")).unwrap();

    let a = store
        .append(EntryKind::Message {
            message: user_msg("a"),
            api_request_sha256: None,
        })
        .unwrap();
    let b = store
        .append(EntryKind::Message {
            message: user_msg("b"),
            api_request_sha256: None,
        })
        .unwrap();
    let c = store
        .append(EntryKind::Message {
            message: user_msg("c"),
            api_request_sha256: None,
        })
        .unwrap();

    // Navigate back to 'a': active branch becomes just [a].
    store.navigate_to(&a.id).unwrap();
    let branch = store.branch_entries().unwrap();
    assert_eq!(
        branch.iter().map(|e| e.id.clone()).collect::<Vec<_>>(),
        vec![a.id.clone()]
    );

    // New appends fork from 'a', not from the abandoned tail.
    let d = store
        .append(EntryKind::Message {
            message: user_msg("d"),
            api_request_sha256: None,
        })
        .unwrap();
    assert_eq!(d.parent_id.as_deref(), Some(a.id.as_str()));

    // The file still holds every entry (human log intact).
    assert_eq!(store.read_all().unwrap().len(), 4); // a,b,c,d all retained

    let _ = (b, c);
}

#[test]
fn model_view_shadows_compacted_ranges_but_file_keeps_them() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = SessionStore::open(dir.path().join("s.jsonl")).unwrap();

    store
        .append(EntryKind::Message {
            message: user_msg("old-1"),
            api_request_sha256: None,
        })
        .unwrap();
    store
        .append(EntryKind::Message {
            message: user_msg("old-2"),
            api_request_sha256: None,
        })
        .unwrap();
    store
        .append(EntryKind::Compaction {
            summary: "we discussed old things".into(),
            tokens_before: 5000,
            retained_tail: vec![user_msg("recent")],
            details: None,
            usage: None,
        })
        .unwrap();

    let view = store.model_view().unwrap();
    // Expect: summary + retained tail only. old-1/old-2 shadowed.
    assert_eq!(view.len(), 2);
    assert!(matches!(&view[0], ModelViewEntry::Summary { text, .. } if text.contains("old")));
    assert!(
        matches!(&view[1], ModelViewEntry::Message(m) if matches!(m, Message::User(u) if matches!(&u.content[0], UserContent::Text { text } if text == "recent")))
    );

    // Human log keeps everything.
    assert_eq!(store.read_all().unwrap().len(), 3);
}

#[test]
fn crash_truncated_final_line_is_detected_and_truncated() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.jsonl");
    {
        let mut store = SessionStore::open(&path).unwrap();
        store
            .append(EntryKind::Message {
                message: user_msg("good"),
                api_request_sha256: None,
            })
            .unwrap();
    }
    // Simulate crash mid-write:
    let mut raw = std::fs::read_to_string(&path).unwrap();
    raw.push_str("{\"id\":\"ent_x\",\"seq\":9,\"ki"); // cut line
    std::fs::write(&path, &raw).unwrap();

    // open() auto-recovers the truncated tail:
    let mut store = SessionStore::open(&path).unwrap();
    match store.reindex() {
        Ok(clean) => assert!(clean, "expected clean reindex after auto-truncate"),
        Err(e) => panic!("second reindex failed: {e:?}"),
    }
    assert_eq!(store.len(), 1);
    assert!(store.leaf_id().is_some());
    let all = store.read_all().unwrap();
    match &all[0].kind {
        EntryKind::Message { message, .. } => {
            assert!(
                matches!(message, Message::User(u) if matches!(&u.content[0], UserContent::Text { text } if text == "good"))
            );
        }
        other => panic!("unexpected kind: {other:?}"),
    }
}

#[test]
fn dangling_tool_calls_get_synthetic_results_and_turn_end() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = SessionStore::open(dir.path().join("s.jsonl")).unwrap();

    store
        .append(EntryKind::Message {
            message: user_msg("go"),
            api_request_sha256: None,
        })
        .unwrap();
    store
        .append(EntryKind::Message {
            message: tool_call_msg("tc_1", "bash"),
            api_request_sha256: None,
        })
        .unwrap();
    // Crash: no tool result followed.

    let report = repair_on_open(&mut store).unwrap();
    assert_eq!(report.repaired_tool_calls, 1);
    assert!(report.appended_turn_end);

    // Idempotent second pass.
    let again = repair_on_open(&mut store).unwrap();
    assert_eq!(again.repaired_tool_calls, 0);

    // Branch now provider-valid.
    let branch = store.branch_entries().unwrap();
    assert!(matches!(
        branch.last().map(|e| &e.kind),
        Some(EntryKind::TurnEnd {
            reason: TurnEndReason::Aborted
        })
    ));

    let _ = tool_result_msg("tc_1", "bash"); // shape witness for future assertions
}

#[test]
fn lease_blocks_second_writer_until_released() {
    let dir = tempfile::tempdir().unwrap();
    let session = dir.path().join("s.jsonl");
    std::fs::write(&session, "").unwrap();

    let held = SessionLease::acquire(&session).unwrap();
    assert!(matches!(held, epsilon_session::LeaseOutcome::Acquired(_)));
    assert!(matches!(
        SessionLease::acquire(&session).unwrap(),
        epsilon_session::LeaseOutcome::Held { .. }
    ));
}
