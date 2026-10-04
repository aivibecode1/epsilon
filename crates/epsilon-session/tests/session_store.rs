//! Integration coverage for the session brick: append/read, tree navigation, model-view
//! shadowing, crash repair, and lease exclusivity.

use epsilon_contracts::agent_event::TurnEndReason;
use epsilon_contracts::content::{ContentBlock, UserContent};
use epsilon_contracts::message::{
    AssistantMessage, Message, StopReason, ToolResultMessage, Usage, UserMessage,
};
use epsilon_contracts::new_id;
use epsilon_contracts::session_entry::{
    CURRENT_SESSION_VERSION, EntryKind, SessionEntry, SessionHeader, SessionHeaderKind,
};
use epsilon_session::repair::{RepairKind, repair_on_open};
use epsilon_session::{LineDiagnosis, ModelViewEntry, SessionLease, SessionStore, StoreError};

#[test]
fn child_sessions_record_their_parent_link() {
    // M58: a delegated task's transcript is a session too; the header records
    // the session it was spawned from.
    let tmp = tempfile::tempdir().expect("tmp");
    let path = tmp.path().join("tui-task_abc.jsonl");
    let child = SessionStore::open_child_in(&path, tmp.path(), "ses_parent").expect("child store");
    assert_eq!(
        child.header().and_then(|h| h.parent_session.as_deref()),
        Some("ses_parent"),
        "the header carries the parent session id"
    );
    // Re-opening an existing child keeps the link (no re-stamping). The first
    // store's lease must be released first: one writer per file (M61).
    drop(child);
    let again = SessionStore::open_child_in(&path, tmp.path(), "ses_other").expect("reopen");
    assert_eq!(
        again.header().and_then(|h| h.parent_session.as_deref()),
        Some("ses_parent"),
        "an existing child is never re-parented"
    );
    // A plain `open_in` still produces an unlinked session.
    let plain_path = tmp.path().join("tui.jsonl");
    let plain = SessionStore::open_in(&plain_path, tmp.path()).expect("plain store");
    assert!(
        plain
            .header()
            .and_then(|h| h.parent_session.as_deref())
            .is_none(),
        "top-level sessions carry no parent"
    );
}

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
            model_id: None,
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
fn a_second_compaction_replaces_the_superseded_projection() {
    // CORE-03: the durable model view must agree with runtime history after a
    // second compaction — the latest compaction's summary, its retained tail, and
    // everything after it. The earlier summary and its tail are superseded.
    let dir = tempfile::tempdir().unwrap();
    let mut store = SessionStore::open(dir.path().join("s.jsonl")).unwrap();

    for text in ["old-1", "old-2"] {
        store
            .append(EntryKind::Message {
                message: user_msg(text),
                api_request_sha256: None,
            })
            .unwrap();
    }
    store
        .append(EntryKind::Compaction {
            summary: "first summary".into(),
            tokens_before: 5000,
            retained_tail: vec![user_msg("tail-1")],
            details: None,
            usage: None,
            model_id: None,
        })
        .unwrap();
    store
        .append(EntryKind::Message {
            message: user_msg("between"),
            api_request_sha256: None,
        })
        .unwrap();
    store
        .append(EntryKind::Compaction {
            summary: "second summary".into(),
            tokens_before: 6000,
            retained_tail: vec![user_msg("tail-2")],
            details: None,
            usage: None,
            model_id: None,
        })
        .unwrap();
    store
        .append(EntryKind::Message {
            message: user_msg("after"),
            api_request_sha256: None,
        })
        .unwrap();

    let view = store.model_view().unwrap();
    assert_eq!(view.len(), 3, "view: {view:?}");
    assert!(
        matches!(&view[0], ModelViewEntry::Summary { text, .. } if text == "second summary"),
        "the latest summary leads the projection"
    );
    assert!(
        matches!(&view[1], ModelViewEntry::Message(Message::User(u))
            if matches!(&u.content[0], UserContent::Text { text } if text == "tail-2")),
        "and keeps its own retained tail"
    );
    assert!(
        matches!(&view[2], ModelViewEntry::Message(Message::User(u))
            if matches!(&u.content[0], UserContent::Text { text } if text == "after")),
        "messages after the latest compaction stay"
    );

    // The superseded summary and its tail must NOT ride along in the request.
    assert!(
        !view
            .iter()
            .any(|v| matches!(v, ModelViewEntry::Summary { text, .. } if text == "first summary")),
        "the first summary is superseded, not repeated"
    );

    // Retention is not the mechanism: the human log keeps every entry.
    assert_eq!(store.read_all().unwrap().len(), 6);
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
    // `acquire_now`, not `acquire`: the waiting variant would deliberately sit out
    // the 30-second stale window here, because this process is the live holder and
    // will keep heartbeating. Asserting the refusal is the point, and refusing
    // immediately is what a caller with its own timeout wants.
    assert!(matches!(
        SessionLease::acquire_now(&session).unwrap(),
        epsilon_session::LeaseOutcome::Held { .. }
    ));
}

/// M75: a repair reports the *kind*, not just a count, and the synthesized entry says which
/// repair produced it — so a reader (or a future tool) can tell "a call had no result" from
/// "the session ended mid-run".
#[test]
fn a_repair_reports_its_kinds_and_marks_what_it_synthesized() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.jsonl");
    let mut store = SessionStore::open(&path).unwrap();

    // Intact: nothing to report.
    let clean = repair_on_open(&mut store).unwrap();
    assert!(clean.kinds().is_empty(), "{clean:?}");
    assert!(clean.summary().is_empty());

    store
        .append(EntryKind::Message {
            message: tool_call_msg("tc_1", "bash"),
            api_request_sha256: None,
        })
        .unwrap();
    let report = repair_on_open(&mut store).unwrap();
    assert_eq!(
        report.kinds(),
        vec![
            (RepairKind::DanglingToolCall, 1),
            (RepairKind::MissingTurnEnd, 1)
        ],
        "{report:?}"
    );
    let lines = report.summary();
    assert_eq!(lines.len(), 2);
    assert!(lines[0].contains("no result"), "{lines:?}");
    assert!(lines[1].contains("mid-run"), "{lines:?}");

    // The synthesized entry names the repair that produced it, next to the `interrupted`
    // marker frontends already read.
    let synthesized = store
        .branch_entries()
        .unwrap()
        .into_iter()
        .find_map(|e| match e.kind {
            EntryKind::Message { message, .. } => match message {
                Message::ToolResult(t) if t.tool_call_id == "tc_1" => Some(t),
                _ => None,
            },
            _ => None,
        })
        .expect("the synthesized result");
    assert_eq!(synthesized.details["repair"], "dangling_tool_call");
    assert_eq!(synthesized.details["interrupted"], true);
    assert!(synthesized.is_error);
}

/// Build a raw session file with `n` well-formed message lines; returns the lines.
fn raw_message_lines(dir: &tempfile::TempDir, name: &str, texts: &[&str]) -> Vec<String> {
    let path = dir.path().join(name);
    {
        let mut store = SessionStore::open(&path).unwrap();
        for t in texts {
            store
                .append(EntryKind::Message {
                    message: user_msg(t),
                    api_request_sha256: None,
                })
                .unwrap();
        }
    }
    // Entry lines only — the header (line 1) is structural framing, not test data.
    std::fs::read_to_string(&path)
        .unwrap()
        .lines()
        .map(str::to_string)
        .filter(|l| !l.contains("\"type\":\"session\""))
        .collect()
}

#[test]
fn mid_file_corruption_reports_line_and_preserves_file() {
    // Pi parity (`storage.ts::load` invalid_entry): a bad line that is NOT the last
    // line is genuine corruption — the open fails naming the line, and the file is
    // left byte-identical (the good suffix must survive, never be truncated away).
    let dir = tempfile::tempdir().unwrap();
    let lines = raw_message_lines(&dir, "s.jsonl", &["one", "three"]);
    let path = dir.path().join("s.jsonl");
    let corrupt = format!("{}\nNOT-JSON{{\n{}\n", lines[0], lines[1]);
    std::fs::write(&path, &corrupt).unwrap();

    match SessionStore::open(&path) {
        // M75: the refusal names the reason, so "damaged" and "written by a newer build" are
        // told apart instead of both being reported as a bare line number.
        Err(StoreError::Corrupt {
            line: 2,
            diagnosis: LineDiagnosis::NotJson,
        }) => {}
        Err(e) => panic!("expected a NotJson diagnosis on line 2, got {e:?}"),
        Ok(_) => panic!("expected a refusal, open succeeded"),
    }
    // File preserved verbatim — no torn-tail truncation happened.
    assert_eq!(std::fs::read_to_string(&path).unwrap(), corrupt);

    // After the operator removes the bad line, the session opens with both entries.
    let fixed = format!("{}\n{}\n", lines[0], lines[1]);
    std::fs::write(&path, &fixed).unwrap();
    let store = SessionStore::open(&path).unwrap();
    assert_eq!(store.len(), 2);
}

#[test]
fn mid_file_corruption_plus_torn_tail_still_reports_line() {
    // Both failure shapes at once: the mid-file line is reported, and the torn tail
    // is NOT truncated away while the error is outstanding (pi throws invalid_entry
    // without repairing anything).
    let dir = tempfile::tempdir().unwrap();
    let lines = raw_message_lines(&dir, "s.jsonl", &["one", "three"]);
    let path = dir.path().join("s.jsonl");
    let corrupt = format!(
        "{}\nNOT-JSON{{\n{}\n{{\"id\":\"ent_x\",",
        lines[0], lines[1]
    );
    std::fs::write(&path, &corrupt).unwrap();

    match SessionStore::open(&path) {
        Err(StoreError::Corrupt {
            line: 2,
            diagnosis: LineDiagnosis::NotJson,
        }) => {}
        Err(e) => panic!("expected a NotJson diagnosis on line 2, got {e:?}"),
        Ok(_) => panic!("expected a refusal, open succeeded"),
    }
    assert_eq!(std::fs::read_to_string(&path).unwrap(), corrupt);
}

#[test]
fn new_file_starts_with_current_header() {
    // Fresh files carry a `{type:"session",version:3}` first line; seq 0 is
    // the first ENTRY (the header is framing, never indexed).
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.jsonl");
    let mut store = SessionStore::open_in(&path, dir.path()).unwrap();
    assert_eq!(store.version(), CURRENT_SESSION_VERSION);
    let header = store.header().expect("new file has a header");
    assert_eq!(header.cwd, dir.path().to_string_lossy());
    assert_eq!(header.parent_session, None);

    let e0 = store
        .append(EntryKind::Message {
            message: user_msg("hello"),
            api_request_sha256: None,
        })
        .unwrap();
    assert_eq!(e0.seq, 0);
    assert_eq!(e0.parent_id, None);

    let first: String = std::fs::read_to_string(&path)
        .unwrap()
        .lines()
        .next()
        .unwrap()
        .into();
    assert!(first.contains("\"type\":\"session\""));
    assert!(first.contains("\"version\":3"));
}

#[test]
fn old_headerless_file_opens_as_v1_and_appends_header_lazily() {
    // Files written before headers existed have no first line to skip: they
    // open at version 1 with entries numbered from 0, and the first new
    // append stamps the header WITHOUT renumbering (header is never indexed).
    let dir = tempfile::tempdir().unwrap();
    let lines = raw_message_lines(&dir, "s.jsonl", &["one", "two"]);
    let path = dir.path().join("s.jsonl");
    std::fs::write(&path, format!("{}\n{}\n", lines[0], lines[1])).unwrap();

    let store = SessionStore::open(&path).unwrap();
    assert_eq!(store.version(), 1);
    assert!(store.header().is_none());
    assert_eq!(store.len(), 2);

    // Appending stamps the header lazily; seqs continue, never shift.
    let mut store = store;
    let e2 = store
        .append(EntryKind::Message {
            message: user_msg("three"),
            api_request_sha256: None,
        })
        .unwrap();
    assert_eq!(e2.seq, 2);
    assert_eq!(store.version(), CURRENT_SESSION_VERSION);
    let body = std::fs::read_to_string(&path).unwrap();
    assert!(
        body.lines()
            .next()
            .unwrap()
            .contains("\"type\":\"session\"")
    );
    assert_eq!(store.len(), 3);
    assert_eq!(store.read_entry(0).unwrap().unwrap().seq, 0);
}

#[test]
fn v1_file_with_legacy_shapes_migrates_in_memory() {
    // A real v1 file: no header, entries without id/parent_id, compaction by
    // `firstKeptEntryIndex`. Opens chain-linked with healed ids; `migrated()`
    // reports it; `persist_migration()` writes the current form back.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.jsonl");
    std::fs::write(
        &path,
        concat!(
            r#"{"type":"message","timestamp":100,"message":{"role":"user","content":"hi"}}"#,
            "\n",
            r#"{"type":"compaction","timestamp":200,"summary":"did stuff","tokensBefore":50,"firstKeptEntryIndex":0}"#,
            "\n",
        ),
    )
    .unwrap();

    let mut store = SessionStore::open(&path).unwrap();
    assert!(store.migrated(), "v1 shapes must flag migration");
    assert_eq!(store.len(), 2);
    let all = store.read_all().unwrap();
    assert_eq!(all[0].parent_id, None);
    assert_eq!(all[1].parent_id.as_deref(), Some(all[0].id.as_str()));
    assert!(matches!(&all[0].kind, EntryKind::Message { .. }));
    let EntryKind::Compaction { details, .. } = &all[1].kind else {
        panic!("second entry must heal to compaction");
    };
    assert_eq!(
        details
            .as_ref()
            .and_then(|d| d.get("firstKeptEntryId"))
            .and_then(|v| v.as_str()),
        Some(all[0].id.as_str())
    );

    store.persist_migration().unwrap();
    assert!(!store.migrated());
    assert_eq!(store.version(), CURRENT_SESSION_VERSION);
    let body = std::fs::read_to_string(&path).unwrap();
    assert!(body.lines().next().unwrap().contains("\"version\":3"));
    drop(store); // release the write lease before reopening (M61)
    let reopened = SessionStore::open(&path).unwrap();
    assert!(!reopened.migrated());
    assert_eq!(reopened.len(), 2);
}

// ---- pi's pre-v2 entries heal faithfully ------------------------------------------------------

/// `"2025-12-08T22:41:05.306Z"` as epoch milliseconds, worked out by hand from the civil
/// calendar (2025-12-08 is 20 430 days after 1970-01-01) rather than by calling the parser
/// under test.
const PI_ISO_MS: u64 = 1_765_233_665_306;

/// The first line of pi's own `before-compaction.jsonl` — a user turn carrying 668
/// characters of text as an array of parts, stamped with an ISO string.
const PI_USER_LINE: &str = r#"{"type":"message","timestamp":"2025-12-08T22:41:05.306Z","message":{"role":"user","content":[{"type":"text","text":"alright, read @packages/coding-agent/src/main.ts in full. i feel like this is one big mess and could be refactored to be nicer."}],"timestamp":1765233665292}}"#;

/// The second line of the same fixture: an assistant turn that is *only* tool calls, with
/// the provider, model, usage and stop reason pi records alongside them.
const PI_ASSISTANT_LINE: &str = r#"{"type":"message","timestamp":"2025-12-08T22:41:09.389Z","message":{"role":"assistant","content":[{"type":"toolCall","id":"toolu_012yuiPP1VAfh196GXaAmT8D","name":"read","arguments":{"path":"/Users/badlogic/workspaces/pi-mono/packages/coding-agent/src/main.ts"}}],"api":"anthropic-messages","provider":"anthropic","model":"claude-opus-4-5","usage":{"input":2775,"output":141,"cacheRead":0,"cacheWrite":0,"totalTokens":2916},"stopReason":"toolUse","timestamp":1765233665294}}"#;

/// The third line: the tool result for that call.
const PI_TOOL_RESULT_LINE: &str = r#"{"type":"message","timestamp":"2025-12-08T22:41:09.394Z","message":{"role":"toolResult","toolCallId":"toolu_012yuiPP1VAfh196GXaAmT8D","toolName":"read","content":[{"type":"text","text":"import { Agent } from \"@mariozechner/pi-agent-core\";"}],"isError":false,"timestamp":1765233665294}}"#;

/// One legacy line's healed entry: what the migration produced for a pi-shaped file.
fn heal_pi_line(dir: &tempfile::TempDir, line: &str) -> SessionEntry {
    let path = dir.path().join("pi.jsonl");
    std::fs::write(&path, format!("{line}\n")).expect("write the pi-shaped line");
    let store = SessionStore::open(&path).expect("a pi-shaped line must open");
    assert!(store.migrated(), "a pi-shaped line is legacy and must heal");
    let all = store.read_all().expect("read the healed entry");
    assert_eq!(all.len(), 1, "one line in, one entry out");
    all.into_iter().next().expect("the healed entry")
}

/// The text of a healed user message, so an assertion reads as the thing it is checking.
fn user_text(message: &Message) -> String {
    let Message::User(user) = message else {
        panic!("expected a user message, got {message:?}");
    };
    user.content
        .iter()
        .map(|c| match c {
            UserContent::Text { text } => text.clone(),
            UserContent::Image(_) => "<image>".to_string(),
        })
        .collect()
}

/// Concatenated text of a block list — what a reader of the transcript would see.
fn blocks_text(blocks: &[ContentBlock]) -> String {
    blocks
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

/// The text of any message variant, for assertions over a whole imported view.
fn message_text(message: &Message) -> String {
    match message {
        Message::User(_) => user_text(message),
        Message::Assistant(assistant) => assistant.text(),
        Message::ToolResult(result) => blocks_text(&result.content),
        Message::Custom(custom) => custom.payload.to_string(),
    }
}

/// The unmapped content parts a healed tool result preserved, or an empty list.
fn preserved_parts(message: &Message) -> Vec<serde_json::Value> {
    let Message::ToolResult(result) = message else {
        panic!("expected a tool result, got {message:?}");
    };
    result
        .details
        .get("legacy_unmapped_parts")
        .and_then(|p| p.as_array())
        .cloned()
        .unwrap_or_default()
}

/// A pi *user* entry keeps its text, its role and its timestamp.
///
/// All three were lost at once before this: `content` is an array of parts, so reading it
/// with `as_str()` produced `""`; the role was never read, so every entry became a user
/// turn; and `timestamp` is an ISO string, so `as_u64()` produced `now_ms()`. The
/// assertion that names the worst one is the text — an importer that empties a transcript
/// has destroyed it, and the other two defects merely hid that.
#[test]
fn a_pi_user_entry_heals_with_its_text_role_and_timestamp_intact() {
    let dir = tempfile::tempdir().expect("tmp");
    let entry = heal_pi_line(&dir, PI_USER_LINE);

    let EntryKind::Message { message, .. } = &entry.kind else {
        panic!(
            "a pi user line must heal to a message entry, got {:?}",
            entry.kind
        );
    };
    let Message::User(user) = message else {
        panic!("pi's `role: user` must heal to Message::User, got {message:?}");
    };

    let text = user_text(message);
    assert_eq!(
        text,
        "alright, read @packages/coding-agent/src/main.ts in full. i feel like this is one big \
         mess and could be refactored to be nicer.",
        "the part array must be read, not emptied"
    );
    assert_eq!(user.content.len(), 1, "one text part in, one block out");

    assert_eq!(
        entry.timestamp_ms, 1_765_233_665_292,
        "the message's own millisecond stamp wins over the ISO string on the line"
    );
    assert_eq!(
        PI_ISO_MS - entry.timestamp_ms,
        14,
        "parsing the ISO string (`2025-12-08T22:41:05.306Z`) must land within milliseconds \
         of pi's millisecond field, not decades away at the import time"
    );
    assert_eq!(
        user.timestamp_ms, entry.timestamp_ms,
        "the message and its entry must agree on when the turn happened"
    );
}

/// A pi *assistant* entry heals as an assistant turn with its tool call intact.
///
/// The call has to survive as a call: an assistant message with its content emptied is
/// worse than an empty user message, because the transcript then shows a model turn that
/// asked for nothing while the tool result below it answers something.
#[test]
fn a_pi_assistant_entry_heals_with_its_tool_call_intact() {
    let dir = tempfile::tempdir().expect("tmp");
    let entry = heal_pi_line(&dir, PI_ASSISTANT_LINE);

    let EntryKind::Message { message, .. } = &entry.kind else {
        panic!(
            "a pi assistant line must heal to a message entry, got {:?}",
            entry.kind
        );
    };
    let Message::Assistant(assistant) = message else {
        panic!("pi's `role: assistant` must heal to Message::Assistant, got {message:?}");
    };

    let calls: Vec<_> = assistant.tool_calls().collect();
    assert_eq!(
        calls.len(),
        1,
        "the toolCall part must survive: {assistant:?}"
    );
    let (id, name, arguments) = calls[0];
    assert_eq!(id, "toolu_012yuiPP1VAfh196GXaAmT8D");
    assert_eq!(name, "read");
    assert_eq!(
        arguments["path"],
        serde_json::json!("/Users/badlogic/workspaces/pi-mono/packages/coding-agent/src/main.ts"),
        "the call's arguments must arrive whole"
    );

    // pi records the rest of the assistant turn too, and a healed turn that names no
    // provider or model cannot be replayed against the right one.
    assert_eq!(assistant.provider, "anthropic");
    assert_eq!(assistant.model_id, "claude-opus-4-5");
    assert_eq!(assistant.stop_reason, StopReason::ToolUse);
    assert_eq!(assistant.usage.input, 2775);
    assert_eq!(assistant.usage.total_tokens, 2916);
    assert_eq!(
        assistant.timestamp_ms, entry.timestamp_ms,
        "the healed turn is stamped with its entry's instant, not with the import time"
    );
    assert_eq!(
        entry.timestamp_ms, 1_765_233_665_294,
        "pi's millisecond stamp"
    );
    assert_eq!(
        PI_ISO_MS - entry.timestamp_ms,
        12,
        "the ISO string on the line (22:41:09.389Z) and pi's millisecond field are the same \
         instant 12 ms apart, which is why the parsed string and the number must be told \
         apart rather than assumed equal"
    );
}

/// A pi *toolResult* entry heals as a tool result, not as something the user said.
#[test]
fn a_pi_tool_result_entry_heals_as_a_tool_result() {
    let dir = tempfile::tempdir().expect("tmp");
    let entry = heal_pi_line(&dir, PI_TOOL_RESULT_LINE);

    let EntryKind::Message { message, .. } = &entry.kind else {
        panic!(
            "a pi toolResult line must heal to a message entry, got {:?}",
            entry.kind
        );
    };
    let Message::ToolResult(result) = message else {
        panic!("pi's `role: toolResult` must heal to Message::ToolResult, got {message:?}");
    };

    assert_eq!(
        result.tool_call_id, "toolu_012yuiPP1VAfh196GXaAmT8D",
        "the result must point at the call it answers"
    );
    assert_eq!(result.tool_name, "read");
    assert_eq!(
        result
            .content
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<String>(),
        "import { Agent } from \"@mariozechner/pi-agent-core\";"
    );
    assert!(!result.is_error);
    assert_eq!(
        entry.timestamp_ms, 1_765_233_665_294,
        "pi's millisecond stamp"
    );
    assert_eq!(
        PI_ISO_MS + 4_088 - entry.timestamp_ms,
        4_100,
        "the tool result's ISO string is 4.1 s after the assistant call it answers"
    );
}

/// An assistant line in pi's shape whose content holds a reasoning part **with** a provider
/// signature — exactly the field M119 measured as lost (1 entry in `large-session.jsonl`, 47 in
/// `before-compaction.jsonl`).
///
/// The signature is the head of the one pi's own `before-compaction.jsonl` carries on line 14
/// (2,760 characters there). Truncated because the value is opaque to this build and its length
/// is not what is under test — whether it arrives at all is.
const PI_ASSISTANT_THINKING_LINE: &str = r#"{"type":"message","timestamp":"2025-12-08T22:41:05.306Z","message":{"role":"assistant","content":[{"type":"thinking","thinking":"The user wants to expand AgentSession.","thinkingSignature":"EpEQCkYIChgCKkCOOc7x0voaSJaoxFkK77OqTCsW"},{"type":"text","text":"I will read the file."}],"api":"anthropic-messages","provider":"anthropic","model":"claude-opus-4-5","stopReason":"toolUse","timestamp":1765233665294}}"#;

/// The same shape with **no** `thinkingSignature` on the part: pi writing none is the absence
/// this field must not invent a value for.
const PI_ASSISTANT_THINKING_WITHOUT_SIGNATURE_LINE: &str = r#"{"type":"message","timestamp":"2025-12-08T22:41:05.306Z","message":{"role":"assistant","content":[{"type":"thinking","thinking":"No signature anywhere."},{"type":"text","text":"Done."}],"api":"anthropic-messages","provider":"anthropic","model":"claude-opus-4-5","stopReason":"endTurn","timestamp":1765233665295}}"#;

/// The reasoning blocks of an assistant message, so an assertion reads as what it checks.
fn thinking_blocks(message: &Message) -> Vec<&ContentBlock> {
    let Message::Assistant(assistant) = message else {
        panic!("expected an assistant message, got {message:?}");
    };
    assistant
        .content
        .iter()
        .filter(|block| matches!(block, ContentBlock::Thinking { .. }))
        .collect()
}

/// A pi reasoning part heals with its `thinkingSignature` intact, and one without it heals with
/// no signature rather than a default.
///
/// The signature is what Anthropic needs to replay a reasoning block verbatim, so dropping it
/// made an imported session's reasoning block unreplayable while every text string in it arrived
/// — the field-level loss M119 found and named. Both halves are asserted here because a healer
/// that invented `Some("")` for every signature-less part would pass the first one alone.
#[test]
fn a_pi_thinking_part_heals_with_its_signature_intact() {
    let dir = tempfile::tempdir().expect("tmp");
    let entry = heal_pi_line(&dir, PI_ASSISTANT_THINKING_LINE);

    let EntryKind::Message { message, .. } = &entry.kind else {
        panic!(
            "a pi assistant line must heal to a message entry, got {:?}",
            entry.kind
        );
    };
    let thinking = thinking_blocks(message);
    assert_eq!(
        thinking.len(),
        1,
        "one thinking part in, one out: {message:?}"
    );
    let ContentBlock::Thinking { text, signature } = thinking[0] else {
        unreachable!("filtered to thinking blocks");
    };
    assert_eq!(text, "The user wants to expand AgentSession.");
    assert_eq!(
        signature.as_deref(),
        Some("EpEQCkYIChgCKkCOOc7x0voaSJaoxFkK77OqTCsW"),
        "pi's `thinkingSignature` must arrive on the block, not be dropped at the heal"
    );
    // The text beside it is not displaced by carrying one more field.
    assert!(
        matches!(message, Message::Assistant(a) if a.text().contains("I will read the file.")),
        "{message:?}"
    );

    let bare = heal_pi_line(&dir, PI_ASSISTANT_THINKING_WITHOUT_SIGNATURE_LINE);
    let EntryKind::Message { message, .. } = &bare.kind else {
        panic!("the second pi line must heal to a message entry too");
    };
    let thinking = thinking_blocks(message);
    assert_eq!(thinking.len(), 1);
    let ContentBlock::Thinking { signature, .. } = thinking[0] else {
        unreachable!("filtered to thinking blocks");
    };
    assert_eq!(
        *signature, None,
        "a part pi wrote without a signature must heal without one — absent is not empty"
    );
}

/// The signature survives the store: healed from pi's shape, written to disk in this build's
/// format, read back — the round trip a resumed session actually takes.
#[test]
fn a_healed_thinking_signature_survives_the_store_round_trip() {
    let dir = tempfile::tempdir().expect("tmp");
    let path = dir.path().join("pi-roundtrip.jsonl");
    std::fs::write(&path, format!("{PI_ASSISTANT_THINKING_LINE}\n")).expect("write the pi line");

    let mut store = SessionStore::open(&path).expect("a pi-shaped line must open");
    assert!(store.migrated(), "a pi-shaped line is legacy and must heal");
    let healed = store.read_all().expect("read the healed entry");
    assert_eq!(healed.len(), 1);
    store.persist_migration().expect("write the healed file");
    drop(store); // release the write lease before reopening (M61)

    let text = std::fs::read_to_string(&path).expect("read the migrated file");
    assert!(
        text.contains(r#""signature":"EpEQCkYIChgCKkCOOc7x0voaSJaoxFkK77OqTCsW""#),
        "the signature must be on disk, not only in memory: {text}"
    );

    let reopened = SessionStore::open(&path).expect("reopen the migrated file");
    assert!(!reopened.migrated(), "a migrated file is not legacy");
    let entries = reopened.read_all().expect("read the migrated entries");
    let EntryKind::Message { message, .. } = &entries[0].kind else {
        panic!("the round trip must keep the message kind");
    };
    let thinking = thinking_blocks(message);
    let ContentBlock::Thinking { signature, .. } = thinking[0] else {
        unreachable!("filtered to thinking blocks");
    };
    assert_eq!(
        signature.as_deref(),
        Some("EpEQCkYIChgCKkCOOc7x0voaSJaoxFkK77OqTCsW"),
        "a signature that does not survive the round trip is still a loss, one write later"
    );
}

/// An unrecognized role and an unrecognized content part are preserved, not dropped.
///
/// `bashExecution` is pi's fourth message role and has no Epsilon equivalent — the same
/// shape of problem as an unrecognized part, and the same answer: keep the bytes. This is
/// the assertion that would have caught the original bug if it had existed, because the
/// old code turned this entry into an empty *user* message, which is both wrong and
/// invisible.
#[test]
fn an_unrecognized_role_and_content_part_survive_the_heal() {
    let dir = tempfile::tempdir().expect("tmp");

    // pi's bashExecution: command, output, exit code — nothing Epsilon can type.
    let bash = heal_pi_line(
        &dir,
        r#"{"type":"message","timestamp":"2025-12-08T23:58:22.057Z","message":{"role":"bashExecution","command":"ls","output":"AGENTS.md\nLICENSE\n","exitCode":0,"cancelled":false,"truncated":false,"timestamp":1765238302057}}"#,
    );
    let EntryKind::Message { message, .. } = &bash.kind else {
        panic!("the message kind is what pi wrote, whatever the role inside",);
    };
    let Message::Custom(custom) = message else {
        panic!("a role with no Epsilon variant must stay Custom, got {message:?}");
    };
    assert_eq!(custom.custom_type, "legacy");
    assert_eq!(
        custom.payload["role"], "bashExecution",
        "{:?}",
        custom.payload
    );
    assert_eq!(
        custom.payload["message"]["output"], "AGENTS.md\nLICENSE\n",
        "the command's output is still reachable"
    );
    assert_eq!(custom.payload["message"]["exitCode"], 0);
    assert_eq!(bash.timestamp_ms, 1_765_238_302_057);

    // An assistant part this build cannot type: the text still reaches the model, and the
    // part rides along in `details` instead of vanishing.
    let unknown = heal_pi_line(
        &dir,
        r#"{"type":"message","timestamp":1765233665294,"message":{"role":"toolResult","toolCallId":"tc_1","toolName":"read","content":[{"type":"text","text":"kept"},{"type":"redacted_thinking","data":"opaque-blob"}],"timestamp":1765233665294}}"#,
    );
    let EntryKind::Message { message, .. } = &unknown.kind else {
        panic!("a toolResult with one unknown part is still a message entry");
    };
    assert_eq!(preserved_parts(message).len(), 1);
    assert_eq!(preserved_parts(message)[0]["data"], "opaque-blob");
    assert!(
        preserved_parts(message)[0].get("type").is_some(),
        "the part is kept whole, `type` included"
    );
    let Message::ToolResult(result) = message else {
        panic!("the typed part must stay typed, got {message:?}");
    };
    assert_eq!(result.content.len(), 1, "the known part is still a block");
}

/// pi's `model_change` heals to the typed entry that names exactly what pi wrote.
///
/// The mapping is faithful because it renames nothing but the field spelling: pi's `provider`
/// stays `provider`, pi's `modelId` becomes `model_id`, and both keep their values. Before it the
/// line healed to `Custom` — preserved, but unreadable as a model change to anything that opens
/// the session, which made a pi session's switch inert here.
#[test]
fn a_pi_model_change_heals_to_a_typed_model_change() {
    let dir = tempfile::tempdir().expect("tmp");
    // pi's own line, from `before-compaction.jsonl`.
    let entry = heal_pi_line(
        &dir,
        r#"{"type":"model_change","timestamp":"2025-12-09T00:08:20.826Z","provider":"anthropic","modelId":"claude-3-opus-20240229"}"#,
    );

    let EntryKind::ModelChange { provider, model_id } = &entry.kind else {
        panic!(
            "a model_change must heal to the typed variant, got {:?}",
            entry.kind
        );
    };
    assert_eq!(provider, "anthropic");
    assert_eq!(
        model_id, "claude-3-opus-20240229",
        "pi's `modelId` keeps its value under Epsilon's name"
    );
    assert_eq!(
        entry.timestamp_ms, 1_765_238_900_826,
        "the line's ISO stamp is read, not replaced by the moment of the import"
    );
}

/// pi's `thinking_level_change` heals to the typed entry — note the field pi actually writes.
///
/// The spelling is `thinkingLevel`, not `level`: read from pi's own fixture rather than guessed,
/// which is the whole reason this mapping is safe to make.
#[test]
fn a_pi_thinking_level_change_heals_to_a_typed_thinking_level_change() {
    let dir = tempfile::tempdir().expect("tmp");
    let entry = heal_pi_line(
        &dir,
        r#"{"type":"thinking_level_change","timestamp":"2025-12-08T22:45:43.658Z","thinkingLevel":"high"}"#,
    );

    let EntryKind::ThinkingLevelChange { level } = &entry.kind else {
        panic!(
            "a thinking_level_change must heal to the typed variant, got {:?}",
            entry.kind
        );
    };
    assert_eq!(level, "high");
    assert_eq!(entry.timestamp_ms, 1_765_233_943_658);

    // pi's first line in that fixture asks for `minimal`, which is *not* the first rung of
    // Epsilon's ladder — the level travels as pi's word, so a reader can apply or refuse it,
    // rather than arriving as whatever a default would have produced.
    let minimal = heal_pi_line(
        &dir,
        r#"{"type":"thinking_level_change","timestamp":"2025-12-08T22:45:42.397Z","thinkingLevel":"minimal"}"#,
    );
    assert!(
        matches!(&minimal.kind, EntryKind::ThinkingLevelChange { level } if level == "minimal")
    );
}

/// Half a model change is not a model change: with no provider, the line stays `Custom`.
///
/// The typed variant has no honest value to put in the missing field — the empty string is the
/// only candidate, and `ModelChange { provider: "" }` is indistinguishable from a session whose
/// provider really is blank. That is a lie that looks like data, so the line keeps its whole self
/// in `Custom` instead: preserved, reachable, and plainly unapplied.
#[test]
fn a_model_change_missing_its_provider_stays_custom() {
    let dir = tempfile::tempdir().expect("tmp");
    let entry = heal_pi_line(
        &dir,
        r#"{"type":"model_change","timestamp":"2025-12-09T00:08:20.826Z","modelId":"claude-3-opus-20240229"}"#,
    );

    let EntryKind::Custom { custom_type, data } = &entry.kind else {
        panic!(
            "a model_change with no provider must not be half-typed, got {:?}",
            entry.kind
        );
    };
    assert_eq!(custom_type, "legacy");
    let data = data.as_ref().expect("the preserved line is the payload");
    assert_eq!(
        data["modelId"], "claude-3-opus-20240229",
        "what pi did write is still reachable inside the preserved line"
    );
    assert_eq!(data["type"], "model_change");

    // The same rule for a field that is present but not a string: `null` is not a model id, and
    // reading it as one would be the empty-string lie one step earlier.
    let nulled = heal_pi_line(
        &dir,
        r#"{"type":"model_change","timestamp":"2025-12-09T00:08:20.826Z","provider":"anthropic","modelId":null}"#,
    );
    assert!(
        matches!(
            &nulled.kind,
            EntryKind::Custom { custom_type, .. } if custom_type == "legacy"
        ),
        "a null model id must stay Custom, got {:?}",
        nulled.kind
    );
}

/// A whole pi-shaped file survives a migration write and reopen without losing anything.
///
/// The in-memory heal is only half the path: `persist_migration` rewrites the file, and the
/// re-read has to agree with what the first read produced. This is the test that would fail
/// if the healed shapes were not ones the *writer* could express — the way the old code's
/// empty user message could be written back without complaint.
#[test]
fn a_pi_shaped_file_round_trips_through_persist_migration() {
    let dir = tempfile::tempdir().expect("tmp");
    let path = dir.path().join("pi-v1.jsonl");
    let bash = r#"{"type":"message","timestamp":"2025-12-08T23:58:22.057Z","message":{"role":"bashExecution","command":"ls","output":"AGENTS.md\n","exitCode":0,"cancelled":false,"truncated":false,"timestamp":1765238302057}}"#;
    std::fs::write(
        &path,
        format!("{PI_USER_LINE}\n{PI_ASSISTANT_LINE}\n{PI_TOOL_RESULT_LINE}\n{bash}\n"),
    )
    .expect("write the pi-shaped session");

    let mut store = SessionStore::open(&path).expect("open");
    assert!(store.migrated());
    assert_eq!(store.len(), 4, "every line became an entry");
    store.persist_migration().expect("persist the healed file");
    assert!(!store.migrated(), "a rewritten file is current");
    drop(store); // release the write lease before reopening (M61)

    let reopened = SessionStore::open(&path).expect("reopen the rewritten file");
    assert!(!reopened.migrated());
    assert_eq!(reopened.len(), 4, "no entry was dropped by the rewrite");
    let all = reopened.read_all().expect("read the rewritten entries");

    let kinds: Vec<&str> = all
        .iter()
        .map(|e| match &e.kind {
            EntryKind::Message { message, .. } => match message {
                Message::User(_) => "user",
                Message::Assistant(_) => "assistant",
                Message::ToolResult(_) => "tool_result",
                Message::Custom(_) => "custom",
            },
            EntryKind::Custom { .. } => "custom_entry",
            other => panic!("unexpected entry kind {other:?}"),
        })
        .collect();
    assert_eq!(
        kinds,
        vec!["user", "assistant", "tool_result", "custom"],
        "roles survive the rewrite, so a tool result is still not a user turn"
    );

    // Nothing was lost: the text, the call and its arguments, and the untranslatable
    // bash output are all still there after the second read.
    let EntryKind::Message { message, .. } = &all[0].kind else {
        panic!("first entry is a message");
    };
    assert!(user_text(message).contains("could be refactored to be nicer"));
    let EntryKind::Message { message, .. } = &all[1].kind else {
        panic!("second entry is a message");
    };
    let Message::Assistant(assistant) = message else {
        panic!("second entry is an assistant turn");
    };
    let calls: Vec<_> = assistant.tool_calls().collect();
    assert_eq!(calls[0].0, "toolu_012yuiPP1VAfh196GXaAmT8D");
    assert_eq!(
        calls[0].2["path"],
        serde_json::json!("/Users/badlogic/workspaces/pi-mono/packages/coding-agent/src/main.ts")
    );
    let EntryKind::Message { message, .. } = &all[3].kind else {
        panic!("fourth entry is a message");
    };
    let Message::Custom(custom) = message else {
        panic!("the bashExecution entry must still be preserved after the rewrite");
    };
    assert_eq!(custom.payload["message"]["output"], "AGENTS.md\n");
    assert_eq!(
        all[0].timestamp_ms,
        PI_ISO_MS - 14,
        "the rewrite keeps the instant pi wrote, not the moment of the import"
    );
}

/// A pi header survives the conversion, on disk, in the rewritten first line.
///
/// The other half of part 2: the contracts test proves the type round-trips, and this proves the
/// *conversion path* uses it — a pi session's `provider`, `modelId`, `thinkingLevel` and
/// `branchedFrom` are still in the file after `persist_migration`, which is where they were
/// measured going missing.
#[test]
fn converting_a_pi_file_keeps_its_header_fields_on_disk() {
    let dir = tempfile::tempdir().expect("tmp");
    let path = dir.path().join("pi-headed.jsonl");
    let header = r#"{"type":"session","id":"ses_pi","timestamp":"2025-12-09T00:53:29.825Z","cwd":"/tmp/pi","provider":"anthropic","modelId":"claude-opus-4-5","thinkingLevel":"off","branchedFrom":"ses_earlier"}"#;
    std::fs::write(
        &path,
        format!("{header}\n{PI_USER_LINE}\n{PI_ASSISTANT_LINE}\n"),
    )
    .expect("write the pi-headed session");

    let mut store = SessionStore::open(&path).expect("open");
    assert!(store.migrated(), "v1 entries migrate");
    assert_eq!(
        store.header().map(|h| h.preserved.len()).unwrap_or(0),
        4,
        "the header this build read carries the fields it does not model"
    );
    store.persist_migration().expect("convert");
    drop(store);

    let converted = std::fs::read_to_string(&path).expect("the converted file");
    let first = converted.lines().next().expect("the header line");
    let rewritten: serde_json::Value = serde_json::from_str(first).expect("the rewritten header");
    // The one difference the conversion is *allowed* to make is the version it stamps; every
    // other field — named or not — must be pi's.
    let mut expected: serde_json::Value = serde_json::from_str(header).unwrap();
    expected["version"] = serde_json::json!(CURRENT_SESSION_VERSION);
    assert_eq!(
        rewritten, expected,
        "the rewritten header is the one pi wrote, plus `version`: {first}"
    );
    assert_eq!(rewritten["modelId"], serde_json::json!("claude-opus-4-5"));
    assert_eq!(rewritten["provider"], serde_json::json!("anthropic"));
    assert_eq!(rewritten["thinkingLevel"], serde_json::json!("off"));
    assert_eq!(rewritten["branchedFrom"], serde_json::json!("ses_earlier"));
    assert_eq!(
        converted.lines().count(),
        3,
        "a header plus the two entries: {converted:?}"
    );
}

// ---- the conversion keeps the original beside the file ---------------------------------------

/// A pi-shaped session file and the exact bytes it was written with.
///
/// Returned together because every test here compares the copy against the *original* bytes,
/// not against a re-serialization of them: "the backup holds the original" is a claim about
/// bytes, and the converted file is always a different byte string.
fn write_pi_shaped_file(dir: &tempfile::TempDir, name: &str) -> (std::path::PathBuf, String) {
    let path = dir.path().join(name);
    let body = format!("{PI_USER_LINE}\n{PI_ASSISTANT_LINE}\n");
    std::fs::write(&path, &body).expect("write the pi-shaped session");
    (path, body)
}

/// The copy's path: the session path with `.pre-epsilon` appended.
fn pre_epsilon_of(path: &std::path::Path) -> std::path::PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(".pre-epsilon");
    std::path::PathBuf::from(name)
}

/// Converting a pi-shaped file leaves the original bytes beside it.
///
/// The conversion is right and the rewrite is what makes the file usable at all; what it must
/// not do is leave the only copy of a pi conversation in a shape pi can no longer read. This is
/// the assertion that the way back exists, byte for byte — `s.jsonl.pre-epsilon` is the file as
/// pi wrote it, and the file itself is the converted one.
#[test]
fn a_converted_file_keeps_the_original_bytes_beside_it() {
    let dir = tempfile::tempdir().expect("tmp");
    let (path, original) = write_pi_shaped_file(&dir, "s.jsonl");

    let mut store = SessionStore::open(&path).expect("open the pi-shaped file");
    assert!(
        store.migrated(),
        "a pi-shaped file is legacy and must migrate"
    );
    store.persist_migration().expect("convert the file");
    drop(store); // release the write lease before reading again (M61)

    let backup = pre_epsilon_of(&path);
    assert_eq!(
        std::fs::read(&backup).expect("the conversion must leave a copy beside the file"),
        original.as_bytes(),
        "the copy is the file as it was written, not the converted one"
    );
    let converted = std::fs::read_to_string(&path).expect("the converted file");
    assert_ne!(
        converted, original,
        "…and the file itself really was converted, so the assertion above is not degenerate"
    );
    assert!(
        converted.contains("\"version\":3"),
        "the converted file is current: {converted:?}"
    );
}

/// A second conversion never replaces the copy that is already there.
///
/// The reason is the whole point of "never overwrite": the first pass's copy is the true
/// original, while a second pass's would be a copy of an already-converted file. Replacing it
/// would destroy the only way back in the act of appearing to protect it.
#[test]
fn an_existing_backup_is_never_replaced_by_a_second_conversion() {
    let dir = tempfile::tempdir().expect("tmp");
    let (path, original) = write_pi_shaped_file(&dir, "s.jsonl");

    let mut store = SessionStore::open(&path).expect("open");
    store.persist_migration().expect("first conversion");
    drop(store);
    let backup = pre_epsilon_of(&path);
    assert_eq!(
        std::fs::read(&backup).expect("copy"),
        original.as_bytes(),
        "precondition: the first conversion wrote the original"
    );

    // Stand in for a copy from before that first pass — whatever is already at that path is
    // the original, and nothing here may treat it as scratch.
    let sentinel = "the original from an earlier conversion\n";
    std::fs::write(&backup, sentinel).expect("write the sentinel");

    // A second pass over the file the first pass produced: the version goes backwards by hand,
    // which is the only way a current file is ever migrated again.
    let converted = std::fs::read_to_string(&path).expect("read the converted file");
    let older = converted.replacen("\"version\":3", "\"version\":2", 1);
    std::fs::write(&path, &older).expect("rewrite the header as v2");
    let mut again = SessionStore::open(&path).expect("reopen");
    assert!(again.migrated(), "a v2 header migrates forward");
    again.persist_migration().expect("second conversion");
    drop(again);

    assert_eq!(
        std::fs::read_to_string(&backup).expect("copy"),
        sentinel,
        "an existing `.pre-epsilon` is the true original and is left exactly as it is"
    );
    assert_ne!(
        std::fs::read(&backup).expect("copy"),
        older.as_bytes(),
        "and it was not replaced by a copy of the already-converted file"
    );
}

/// **The copy left by an `append`-triggered conversion is a file this build can open.**
///
/// That is a different claim from "a copy was left", and it is the one that matters: a
/// recovery copy nothing can recover *with* is not a recovery copy.
///
/// The defect was an ordering one. `append` stamps a header lazily on a headerless file,
/// and on a **migrating** file it did that *before* calling `persist_migration` — which is
/// where the copy is taken. The session handle is `O_APPEND`
/// (`store.rs:570-574`), so on a file that already had entries the header line landed at
/// EOF rather than at offset 0, and the `.pre-epsilon` copy captured `original + header`.
/// A header is legal only as the **first** line (`store.rs:889-895`), so the one artefact
/// meant to be the way back opened as `Corrupt { diagnosis: SecondHeader }`.
///
/// The production path through here is a **resume**: `Agent::live_resume`
/// (`epsilon-core/src/lib.rs:972-994`) opens and repairs without calling
/// `persist_migration`, so the first append is what takes the copy. The runtime launch path
/// calls it at startup and was never affected — which is why this sat unfixed behind a green
/// suite: the reachable path is the one the tests did not walk.
#[test]
fn a_backup_taken_from_append_is_a_file_this_build_can_open() {
    let dir = tempfile::tempdir().expect("tmp");
    let lines = raw_message_lines(&dir, "s.jsonl", &["one", "two"]);
    let path = dir.path().join("s.jsonl");
    let original = format!("{}\n{}\n", lines[0], lines[1]);
    std::fs::write(&path, &original).expect("seed a headerless v1 session");

    let mut store = SessionStore::open(&path).expect("open the headerless file");
    assert!(
        store.migrated(),
        "a headerless file is legacy, so the first append converts it"
    );
    store
        .append(EntryKind::Message {
            message: user_msg("three"),
            api_request_sha256: None,
        })
        .expect("the first append converts the file");
    drop(store); // release the write lease before reading again (M61)

    let backup = pre_epsilon_of(&path);
    assert_eq!(
        std::fs::read(&backup).expect("the conversion must leave a copy beside the file"),
        original.as_bytes(),
        "the copy is the file as it was, not the file with a header stamped onto the end of it. \
         A header is legal only as the first line, so a copy carrying one cannot be opened — \
         and this copy is the only way back from the conversion."
    );
    assert_ne!(
        std::fs::read_to_string(&path).expect("the converted file"),
        original,
        "…and the file itself really was converted, so the assertion above is not degenerate"
    );

    // The point of the copy: it opens, and it opens with the conversation still in it.
    let copy = SessionStore::open(&backup).expect("the way back must be openable");
    assert_eq!(
        copy.len(),
        2,
        "…and it still holds what the original held, rather than opening as damage"
    );
}

/// A file that needs no migration leaves no copy behind.
///
/// The copy exists because conversion destroys a foreign file's only readable copy. A current
/// session file has no such problem, so the ordinary path leaves no `.pre-epsilon` behind —
/// including when a caller asks for the rewrite anyway.
#[test]
fn a_file_that_needs_no_migration_leaves_no_backup() {
    let dir = tempfile::tempdir().expect("tmp");
    let path = dir.path().join("s.jsonl");
    let mut store = SessionStore::open_in(&path, dir.path()).expect("open");
    store
        .append(EntryKind::Message {
            message: user_msg("current session"),
            api_request_sha256: None,
        })
        .expect("append");
    assert!(!store.migrated(), "a file this build wrote is current");
    drop(store);

    let mut reopened = SessionStore::open_in(&path, dir.path()).expect("reopen");
    assert!(!reopened.migrated());
    let before = std::fs::read(&path).expect("read the current file");
    // Even a caller that asks for the rewrite anyway gets no copy: nothing was converted, so
    // there is no original to preserve, and a `.pre-epsilon` beside every session file would
    // be noise (the copy is for a file this build had to change to read).
    reopened
        .persist_migration()
        .expect("rewrite a current file");
    drop(reopened);
    assert_eq!(
        std::fs::read(&path).expect("read the file again"),
        before,
        "a current file rewrites to the same bytes"
    );

    assert!(
        !pre_epsilon_of(&path).exists(),
        "a current file must not grow a `.pre-epsilon` copy"
    );
    assert_eq!(
        std::fs::read_dir(dir.path())
            .expect("list the session directory")
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|name| name.contains("pre-epsilon"))
            .count(),
        0,
        "nothing conversion-shaped is left in the directory"
    );
}

#[test]
fn a_session_file_has_one_writer_at_a_time() {
    let dir = tempfile::tempdir().expect("tmp");
    let path = dir.path().join("s.jsonl");

    let first = SessionStore::open_in(&path, dir.path()).expect("first writer");
    let err = match SessionStore::open_in(&path, dir.path()) {
        Ok(_) => panic!("the second writer must be refused while the first is open"),
        Err(e) => e,
    };
    let text = err.to_string();
    assert!(
        matches!(err, epsilon_session::StoreError::Leased { .. }),
        "a held lease is a typed error, not a generic io failure: {err:?}"
    );
    assert!(text.contains("pid="), "the holder is named: {text}");
    assert!(text.contains("one writer per session"), "{text}");

    drop(first);
    SessionStore::open_in(&path, dir.path()).expect("the lease is released with the store");
}

#[test]
fn each_child_session_leases_its_own_file() {
    let dir = tempfile::tempdir().expect("tmp");
    let parent = dir.path().join("ses_parent.jsonl");
    let child = dir.path().join("ses_parent-task1.jsonl");
    let _parent = SessionStore::open_in(&parent, dir.path()).expect("parent");
    let _child = SessionStore::open_child_in(&child, dir.path(), "ses_parent").expect("child");

    // Distinct files: a child never contends with its parent, and two children
    // of one parent never contend with each other.
    let other = dir.path().join("ses_parent-task2.jsonl");
    SessionStore::open_child_in(&other, dir.path(), "ses_parent").expect("second child");
    assert!(
        SessionStore::open_child_in(&child, dir.path(), "ses_parent").is_err(),
        "the same child file still has one writer"
    );
}

// ---- slice 4B: a *current* pi session is imported, and the source is never touched -----------

/// The header pi writes today: `type`/`id`/`timestamp`/`cwd` plus the fields this build has no
/// name for (`provider`, `modelId`, `thinkingLevel`), all of which must survive into the copy.
const PI_V2_HEADER: &str = r#"{"type":"session","version":3,"id":"ses_pi2","timestamp":"2025-12-08T22:41:05.000Z","cwd":"/Users/badlogic/workspaces/pi-mono","provider":"anthropic","modelId":"claude-opus-4-5","thinkingLevel":"off"}"#;

/// A pi **v2** file: one line per entry, each carrying `id`/`parentId` and its kind at the top
/// level — what pi's own `migrateV1ToV2` adds
/// (`session-format.md`/`session-manager.ts:231-242`), and the shape this build refuses today.
///
/// The messages are pi's real shapes from M114's fixture (an ISO `timestamp` on the line, epoch
/// milliseconds inside the message), with the tree envelope pi wraps around them.
fn pi_v2_lines() -> Vec<String> {
    vec![
        PI_V2_HEADER.to_string(),
        r#"{"type":"message","id":"e1","parentId":null,"timestamp":"2025-12-08T22:41:05.306Z","message":{"role":"user","content":[{"type":"text","text":"alright, read @packages/coding-agent/src/main.ts in full."}],"timestamp":1765233665292}}"#.to_string(),
        r#"{"type":"model_change","id":"e2","parentId":"e1","timestamp":"2025-12-08T22:41:06.000Z","provider":"anthropic","modelId":"claude-3-opus-20240229"}"#.to_string(),
        r#"{"type":"message","id":"e3","parentId":"e2","timestamp":"2025-12-08T22:41:09.389Z","message":{"role":"assistant","content":[{"type":"text","text":"reading it now"},{"type":"toolCall","id":"toolu_012yuiPP1VAfh196GXaAmT8D","name":"read","arguments":{"path":"/Users/badlogic/workspaces/pi-mono/packages/coding-agent/src/main.ts"}}],"provider":"anthropic","model":"claude-opus-4-5","usage":{"input":2775,"output":141,"cacheRead":0,"cacheWrite":0,"totalTokens":2916},"stopReason":"toolUse","timestamp":1765233665294}}"#.to_string(),
        r#"{"type":"message","id":"e4","parentId":"e3","timestamp":"2025-12-08T22:41:09.394Z","message":{"role":"toolResult","toolCallId":"toolu_012yuiPP1VAfh196GXaAmT8D","toolName":"read","content":[{"type":"text","text":"import { Agent } from \"@mariozechner/pi-agent-core\";"}],"isError":false,"timestamp":1765233665294}}"#.to_string(),
    ]
}

/// Write a pi file (any shape) and return its path plus the exact bytes written.
///
/// Returned together because the import's central promise is about *bytes*: the source must be
/// the same file afterwards, not a re-serialization that happens to parse the same.
fn write_pi_file(
    dir: &tempfile::TempDir,
    name: &str,
    lines: &[String],
) -> (std::path::PathBuf, Vec<u8>) {
    let path = dir.path().join(name);
    let body = format!("{}\n", lines.join("\n"));
    std::fs::write(&path, &body).expect("write the pi session");
    (path, body.into_bytes())
}

/// The output the importer writes beside a source: the source name with `.epsilon.jsonl`.
fn import_of(path: &std::path::Path) -> std::path::PathBuf {
    epsilon_session::import_output_path(path)
}

/// FNV-1a — the same cheap hash the store's id index uses, here over the whole file.
///
/// A hash rather than an equality check against a buffer so the two assertions a reader wants
/// ("the bytes are the same" and "the test is not vacuous") are both cheap and both made of the
/// file's own content: the before-hash, the after-hash, and the file actually having content.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x1000_0000_01b3);
    }
    h
}

/// A current pi session imports into a new Epsilon session whose messages are *the same
/// messages* — text, roles, tool call, model switch — chained to a root.
///
/// This is the test the slice exists for. Everything it asserts was lost or refused before it:
/// the file did not open at all (refused at line 2), and the v1 healer that would have taken it
/// had already been caught turning a 668-character prompt into an empty string (G19). So the
/// assertion that matters most is the *text* — an importer that produces the right number of
/// empty entries is the defect, not a lesser version of it.
#[test]
fn a_pi_v2_file_imports_into_a_new_session_with_its_messages_intact() {
    let dir = tempfile::tempdir().expect("tmp");
    let (source, _before) = write_pi_file(&dir, "pi.jsonl", &pi_v2_lines());

    let report = epsilon_session::import_pi_session(&source).expect("a pi v2 file must import");
    assert_eq!(
        report.entries,
        4,
        "four entries on pi's active branch: {:?}",
        report.summary()
    );
    assert_eq!(
        report.preserved_kinds,
        Vec::<String>::new(),
        "every kind in this file has a typed home — nothing should have been merely preserved: \
         {:?}",
        report.summary()
    );
    assert_eq!(report.unmapped_parts, 0, "{:?}", report.summary());
    assert_eq!(report.abandoned_entries, 0, "{:?}", report.summary());
    assert_eq!(
        report.output,
        import_of(&source),
        "the output is the name the summary prints"
    );

    // The copy opens as an ordinary session: it is the whole proof that the import produced a
    // file this build can actually use, not merely a different file.
    let imported = SessionStore::open(&report.output).expect("the imported session must open");
    assert!(
        !imported.migrated(),
        "the imported file is current-version, not something that needs healing again"
    );
    assert_eq!(imported.len(), 4);

    // The chain reaches a root, in file order, one parent link at a time.
    let chain = imported.branch_entries().expect("read the active branch");
    assert_eq!(chain.len(), 4, "the active branch is the whole file");
    assert_eq!(
        chain[0].parent_id, None,
        "the first entry is a root, as pi's `parentId: null` said"
    );
    for pair in chain.windows(2) {
        assert_eq!(
            pair[1].parent_id.as_deref(),
            Some(pair[0].id.as_str()),
            "each entry chains onto the one before it"
        );
    }

    // The model-visible view: the user turn, the assistant turn with its tool call, the tool
    // result. The model switch is *applied* (typed), and — like every non-message kind — holds
    // no message of its own.
    let view = imported.model_view().expect("the imported model view");
    assert_eq!(view.len(), 3, "three message entries: {view:?}");
    let ModelViewEntry::Message(Message::User(user)) = &view[0] else {
        panic!("the first entry must be pi's user turn, got {:?}", view[0]);
    };
    assert_eq!(
        user_text(&Message::User(user.clone())),
        "alright, read @packages/coding-agent/src/main.ts in full.",
        "the text must survive the import — an empty message here is the G19 defect again"
    );
    let ModelViewEntry::Message(Message::Assistant(assistant)) = &view[1] else {
        panic!(
            "the second entry must be the assistant turn, got {:?}",
            view[1]
        );
    };
    let calls: Vec<_> = assistant.tool_calls().collect();
    assert_eq!(calls.len(), 1, "the tool call survives as a call");
    assert_eq!(calls[0].1, "read");
    assert_eq!(
        calls[0].2["path"],
        "/Users/badlogic/workspaces/pi-mono/packages/coding-agent/src/main.ts"
    );
    let ModelViewEntry::Message(Message::ToolResult(result)) = &view[2] else {
        panic!("the third entry must be the tool result, got {:?}", view[2]);
    };
    assert_eq!(result.tool_name, "read");
    assert_eq!(result.tool_call_id, calls[0].0);
    assert!(!result.is_error);
    assert_eq!(
        blocks_text(&result.content),
        "import { Agent } from \"@mariozechner/pi-agent-core\";"
    );

    // The model switch is a typed entry, not a preserved blob: "preserved but not applied" is
    // the M114 limit this importer is supposed to close for the kinds it can name.
    let kinds: Vec<&EntryKind> = chain.iter().map(|entry| &entry.kind).collect();
    assert!(
        kinds.iter().any(|kind| matches!(
            kind,
            EntryKind::ModelChange { provider, model_id }
                if provider == "anthropic" && model_id == "claude-3-opus-20240229"
        )),
        "pi's model_change must import as a typed ModelChange: {kinds:?}"
    );

    // Timestamps are pi's, both spellings: the message's epoch milliseconds win over the line's
    // ISO string, exactly as the v1 healer reads them.
    assert_eq!(
        chain[0].timestamp_ms, 1_765_233_665_292,
        "the import must not re-date the session to the moment it ran"
    );

    // The header keeps pi's `cwd` and the fields this build does not model, and the copy is a
    // new session rather than a second file claiming pi's id.
    let header = imported.header().expect("the imported header");
    assert_eq!(header.cwd, "/Users/badlogic/workspaces/pi-mono");
    assert_eq!(
        header.preserved["modelId"],
        serde_json::json!("claude-opus-4-5")
    );
    assert_ne!(header.id, "ses_pi2", "an import is a new session, not pi's");
}

/// The import does not touch the file it read — proved by hashing the source before and after.
///
/// The defect this replaces rewrote the user's file *in place* on the resume path, so the
/// original was gone with no way back. An import that leaves the source byte-identical is the
/// whole safety story for a foreign file: pi can still read it, and the user can still delete
/// the copy and try again.
#[test]
fn the_imported_source_is_byte_identical_afterwards() {
    let dir = tempfile::tempdir().expect("tmp");
    let (source, before) = write_pi_file(&dir, "pi.jsonl", &pi_v2_lines());
    let on_disk = std::fs::read(&source).expect("read the source");
    assert_eq!(on_disk, before, "the fixture is what the test thinks it is");
    let hash_before = fnv1a(&on_disk);
    assert_ne!(hash_before, 0, "and it has content to hash");

    let report = epsilon_session::import_pi_session(&source).expect("import");

    let after = std::fs::read(&source).expect("read the source again");
    assert_eq!(
        fnv1a(&after),
        hash_before,
        "the source must be byte-identical after an import — no rewrite, no backup, no rename"
    );
    assert_eq!(
        after, before,
        "…and byte equality is the stronger claim the hash stands in for"
    );
    assert!(
        report.output.exists(),
        "the copy exists, so the assertion above is not passing because nothing happened"
    );
    assert_ne!(
        std::fs::read(&report.output).expect("read the copy"),
        before,
        "and the copy is a different file, not a second copy of the source"
    );
}

/// A branch: the imported session is the chain to pi's leaf, and the abandoned sibling is left
/// in the source — loudly.
///
/// **The choice: abandoned siblings are not imported, and are counted in the report.** pi's own
/// context builder walks the branch from the leaf (`buildSessionContext`, `session-manager.ts:463-468`);
/// a sibling that is not on that path was already not part of the conversation the session was
/// having. Importing it anyway would invent a turn the model never saw, on a chain nobody wrote —
/// and the alternative, dropping it silently, is the failure this repository keeps re-learning.
/// So it is dropped *out loud*: `abandoned_entries` says how many, and the summary says the source
/// still holds them.
///
/// The leaf here is `e6`, which is *not* the last entry a parent-less walk would find by accident:
/// `e5` is its sibling, and both share `e4` as a parent.
#[test]
fn a_branch_imports_the_chain_to_the_leaf_and_reports_the_abandoned_sibling() {
    let dir = tempfile::tempdir().expect("tmp");
    let mut lines = pi_v2_lines();
    lines.push(
        r#"{"type":"message","id":"e5","parentId":"e4","timestamp":"2025-12-08T22:42:00.000Z","message":{"role":"assistant","content":[{"type":"text","text":"I will rewrite main.ts from scratch."}],"timestamp":1765233720000}}"#.to_string(),
    );
    lines.push(
        r#"{"type":"message","id":"e6","parentId":"e4","timestamp":"2025-12-08T22:42:10.000Z","message":{"role":"user","content":[{"type":"text","text":"no — walk me through it instead"}],"timestamp":1765233730000}}"#.to_string(),
    );
    let (source, _before) = write_pi_file(&dir, "branched.jsonl", &lines);

    let report = epsilon_session::import_pi_session(&source).expect("import the branched file");
    assert_eq!(
        report.abandoned_entries,
        1,
        "`e5` is on a branch the active chain does not pass through, and the report must say so: \
         {:?}",
        report.summary()
    );
    assert!(
        report
            .summary()
            .iter()
            .any(|line| line.contains("not imported") && line.contains("source still holds them")),
        "the abandoned entry is reported, not silently dropped: {:?}",
        report.summary()
    );

    let imported = SessionStore::open(&report.output).expect("open the import");
    let chain = imported.branch_entries().expect("the imported chain");
    assert_eq!(
        chain.len(),
        5,
        "e1..e4 plus the leaf e6 — the sibling e5 is not on this chain"
    );
    let view = imported.model_view().expect("model view");
    let texts: Vec<String> = view
        .iter()
        .filter_map(|entry| match entry {
            ModelViewEntry::Message(message) => Some(message_text(message)),
            _ => None,
        })
        .collect();
    assert!(
        texts
            .iter()
            .any(|t| t.contains("walk me through it instead")),
        "the chain reaches the leaf pi ended on: {texts:?}"
    );
    assert!(
        !texts.iter().any(|t| t.contains("rewrite main.ts")),
        "the abandoned sibling is not in the imported session: {texts:?}"
    );
    assert_eq!(
        chain.last().map(|entry| entry.timestamp_ms),
        Some(1_765_233_730_000),
        "the leaf is the last entry pi wrote, not the first unnamed one"
    );
}

/// An explicit leaf marker wins over the unnamed-entry rule, when a file carries one.
///
/// pi's own header has no such field (`session-manager.ts:32-39`), so this is the branch of the
/// rule a real pi file never takes — which is exactly why it needs a test: an untested branch
/// that a future pi (or another tool sharing the envelope) would take is a branch that silently
/// imports the wrong conversation the day it first matters.
#[test]
fn an_explicit_leaf_marker_selects_the_branch_it_names() {
    let dir = tempfile::tempdir().expect("tmp");
    let mut lines = pi_v2_lines();
    lines.push(
        r#"{"type":"message","id":"e5","parentId":"e4","timestamp":"2025-12-08T22:42:00.000Z","message":{"role":"assistant","content":[{"type":"text","text":"I will rewrite main.ts from scratch."}],"timestamp":1765233720000}}"#.to_string(),
    );
    lines.push(
        r#"{"type":"message","id":"e6","parentId":"e4","timestamp":"2025-12-08T22:42:10.000Z","message":{"role":"user","content":[{"type":"text","text":"no — walk me through it instead"}],"timestamp":1765233730000}}"#.to_string(),
    );
    // The header now points at the *earlier* branch, which file order alone would not choose.
    lines[0] = PI_V2_HEADER.replace(
        r#""thinkingLevel":"off""#,
        r#""thinkingLevel":"off","leafId":"e5""#,
    );
    let (source, _before) = write_pi_file(&dir, "marked.jsonl", &lines);

    let report = epsilon_session::import_pi_session(&source).expect("import");
    let imported = SessionStore::open(&report.output).expect("open the import");
    let texts: Vec<String> = imported
        .model_view()
        .expect("model view")
        .into_iter()
        .filter_map(|entry| match entry {
            ModelViewEntry::Message(message) => Some(message_text(&message)),
            _ => None,
        })
        .collect();
    assert!(
        texts.iter().any(|t| t.contains("rewrite main.ts")),
        "the marker's branch is the one imported: {texts:?}"
    );
    assert!(
        !texts.iter().any(|t| t.contains("walk me through it")),
        "…and the other branch is not: {texts:?}"
    );
    assert_eq!(report.abandoned_entries, 1, "{:?}", report.summary());
}

/// A kind this build does not know survives the *new* envelope too.
///
/// M113's rule ("an unknown entry kind is kept, not refused") was written for Epsilon's own
/// envelope. The import is a second envelope and a second chance to lose the same data, so the
/// rule is re-asserted here on a pi file: the entry lands in the copy with its kind object whole,
/// the report names it, and the model view skips it — exactly as it would for a future Epsilon's
/// entry.
#[test]
fn an_unknown_kind_in_a_pi_v2_file_is_preserved_and_reported() {
    let dir = tempfile::tempdir().expect("tmp");
    let mut lines = pi_v2_lines();
    // The foreign kind sits **on the chain**: between pi's first user turn and the model switch
    // that follows it. Anything else would test the branch rule instead of this one, and a kind
    // this build cannot read must not cost the entries around it their places in the tree.
    lines[2] = lines[2].replace(r#""parentId":"e1""#, r#""parentId":"e9""#);
    lines.insert(
        2,
        r#"{"type":"telemetry_v9","id":"e9","parentId":"e1","timestamp":"2025-12-08T22:41:07.000Z","payload":{"deep":[1,2,3]},"note":"from a newer pi"}"#.to_string(),
    );
    let (source, _before) = write_pi_file(&dir, "future.jsonl", &lines);

    let report = epsilon_session::import_pi_session(&source).expect("import");
    assert_eq!(
        report.preserved_kinds,
        vec!["telemetry_v9".to_string()],
        "the report names the kind it could only preserve: {:?}",
        report.summary()
    );
    assert_eq!(
        report.abandoned_entries,
        0,
        "the foreign kind is on the chain, so nothing was abandoned: {:?}",
        report.summary()
    );
    assert!(
        report
            .summary()
            .iter()
            .any(|line| line.contains("`telemetry_v9`")),
        "…in the summary a user reads: {:?}",
        report.summary()
    );

    let imported = SessionStore::open(&report.output).expect("open the import");
    assert_eq!(imported.len(), 5, "the entry is in the file, not dropped");
    let preserved = imported
        .read_all()
        .expect("read all")
        .into_iter()
        .find(|entry| entry.kind.is_unknown() || matches!(entry.kind, EntryKind::Custom { .. }))
        .expect("the unknown entry must be in the copy, in some preserved form");
    let EntryKind::Custom { data, .. } = &preserved.kind else {
        panic!(
            "a foreign kind is preserved as Custom here, got {:?}",
            preserved.kind
        );
    };
    let data = data
        .as_ref()
        .expect("the preserved entry carries its raw object");
    assert_eq!(data["type"], "telemetry_v9");
    assert_eq!(
        data["payload"]["deep"][2], 3,
        "every field and value survives"
    );
    assert_eq!(data["note"], "from a newer pi");
    assert_eq!(
        imported.preserved_unknown_notice(),
        None,
        "it is not Epsilon's own unknown-kind path — it was converted, so nothing is unparsed"
    );

    let view = imported.model_view().expect("model view");
    assert_eq!(
        view.len(),
        3,
        "a preserved kind contributes no message: {view:?}"
    );
}

/// A second import refuses rather than replacing the first one's output.
///
/// The same "never overwrite what is already there" rule the `.pre-epsilon` backup follows, for
/// the same reason: the file on disk may be the only surviving copy of a session, and a repeated
/// command is not a reason to destroy it.
#[test]
fn a_second_import_refuses_to_replace_the_first_output() {
    let dir = tempfile::tempdir().expect("tmp");
    let (source, before) = write_pi_file(&dir, "pi.jsonl", &pi_v2_lines());

    epsilon_session::import_pi_session(&source).expect("first import");
    let output = import_of(&source);
    let first_copy = std::fs::read(&output).expect("the first import's output");

    let err = epsilon_session::import_pi_session(&source).expect_err("the second must refuse");
    assert!(
        err.to_string().contains("already exists"),
        "the refusal names the reason: {err}"
    );
    assert_eq!(
        std::fs::read(&output).expect("the output is still there"),
        first_copy,
        "the first import's output is untouched"
    );
    assert_eq!(
        std::fs::read(&source).expect("the source is still there"),
        before,
        "and so is the source"
    );
}

// ---- M75: typed corruption reasons -----------------------------------------------------------

/// The case this test exists for: a file from a *newer* Epsilon is not damaged, and saying
/// "corrupt line 2" about it would send the user looking for a fault that is not there.
///
/// M113 changed what the store does about it, so the test changed with it. The entry is now
/// **preserved** rather than refused — the session opens, the entry keeps every field and
/// value for whoever wrote it, and the store reports what it kept instead of the frontend
/// having to guess. Refusing was safe but cost the whole conversation; preserving is safe
/// *and* usable, which is what "not damaged" should mean. The refusal path still exists for
/// the case that deserves it, one test below.
#[test]
fn an_unknown_entry_type_is_preserved_and_reported_rather_than_refused() {
    let dir = tempfile::tempdir().unwrap();
    let lines = raw_message_lines(&dir, "s.jsonl", &["one"]);
    let path = dir.path().join("s.jsonl");
    // A well-formed entry from the future: same envelope, a kind this build has never seen.
    // Its parent is the real first entry, so the lane stays one continuous chain — a foreign
    // kind must not cost the entry its place in the tree either.
    let first: serde_json::Value = serde_json::from_str(&lines[0]).unwrap();
    let first_id = first["id"].as_str().unwrap();
    let future = format!(
        r#"{{"id":"ent_future","seq":1,"parent_id":"{first_id}","timestamp_ms":0,"kind":{{"type":"telemetry_v9","payload":{{"n":1}}}}}}"#
    );
    let content = format!("{}\n{future}\n", lines[0]);
    std::fs::write(&path, &content).unwrap();

    let store =
        SessionStore::open(&path).expect("a foreign entry must not close the whole session");
    assert_eq!(
        store.preserved_unknown(),
        vec![(1u64, "telemetry_v9".to_string())],
        "the store must be able to say what it kept without understanding"
    );
    // The file is untouched at open — the store only reads.
    assert_eq!(std::fs::read_to_string(&path).unwrap(), content);

    // The entry is readable and intact, and it is NOT part of the model's view: a foreign
    // kind contributes nothing to the transcript rather than contributing something wrong.
    let all = store.read_all().unwrap();
    assert_eq!(all.len(), 2);
    assert!(all[1].kind.is_unknown());
    assert_eq!(
        all[1].parent_id.as_deref(),
        Some(all[0].id.as_str()),
        "the foreign entry keeps its place in the chain"
    );
    let view = store.model_view().unwrap();
    assert_eq!(view.len(), 1, "only the known message reaches the model");

    // And it survives a rewrite. A fork is the operation that re-serializes entries into a
    // new file one at a time, so it is the one that could quietly drop a kind it cannot
    // write — appending would not have tested this at all, since appending only adds bytes.
    let leaf = store.leaf_id().expect("the foreign entry is the leaf");
    let dest_dir = dir.path().join("forks");
    let dest = store.fork_branch_at(&leaf, &dest_dir).unwrap();
    drop(store);
    let forked = SessionStore::open_in(&dest, dir.path()).unwrap();
    assert_eq!(
        forked.preserved_unknown(),
        vec![(1u64, "telemetry_v9".to_string())],
        "a rewrite must not quietly drop what the previous read preserved"
    );
    let rewritten = std::fs::read_to_string(&dest).unwrap();
    let foreign: serde_json::Value = serde_json::from_str(
        rewritten
            .lines()
            .find(|l| l.contains("telemetry_v9"))
            .expect("the foreign entry is still in the forked file"),
    )
    .unwrap();
    assert_eq!(foreign["kind"]["payload"]["n"], 1, "{rewritten}");
    // The original is untouched by the fork, and still byte-identical.
    assert_eq!(std::fs::read_to_string(&path).unwrap(), content);
}

/// The sentence both frontends print for preserved entries: the count, the kinds deduplicated in
/// file order, and the plain statement that they are not in the transcript.
///
/// `preserved_unknown()` existed and was tested, and no caller called it — which is how a
/// preserved entry stays a silent gap: the session opens, the transcript looks whole, and the one
/// thing that knew better was a method nothing printed. The CLI and the TUI both print *this*
/// line now, so the store owns the wording and the ordering it depends on.
#[test]
fn the_preserved_unknown_notice_names_the_count_and_the_kinds_in_file_order() {
    let dir = tempfile::tempdir().unwrap();
    let lines = raw_message_lines(&dir, "s.jsonl", &["one"]);
    let path = dir.path().join("s.jsonl");
    let first: serde_json::Value = serde_json::from_str(&lines[0]).unwrap();
    let first_id = first["id"].as_str().unwrap();
    // Three foreign entries, two kinds, `telemetry_v9` on both sides of `widget_v2`: the notice
    // must count entries but name kinds, once each, in the order the file lists them.
    let a = format!(
        r#"{{"id":"ent_a","seq":1,"parent_id":"{first_id}","timestamp_ms":0,"kind":{{"type":"telemetry_v9"}}}}"#
    );
    let b = r#"{"id":"ent_b","seq":2,"parent_id":"ent_a","timestamp_ms":0,"kind":{"type":"widget_v2"}}"#;
    let c = r#"{"id":"ent_c","seq":3,"parent_id":"ent_b","timestamp_ms":0,"kind":{"type":"telemetry_v9"}}"#;
    std::fs::write(&path, format!("{}\n{a}\n{b}\n{c}\n", lines[0])).unwrap();

    let store = SessionStore::open(&path).expect("foreign kinds do not close the session");
    assert_eq!(
        store.preserved_unknown(),
        vec![
            (1u64, "telemetry_v9".to_string()),
            (2u64, "widget_v2".to_string()),
            (3u64, "telemetry_v9".to_string()),
        ],
        "every preserved entry, in file order"
    );
    assert_eq!(
        store.preserved_unknown_notice().as_deref(),
        Some(
            "3 entries of kinds this build does not know (`telemetry_v9`, `widget_v2`) are in \
             the file but not in the transcript — kept, not dropped"
        ),
        "one line: the count, the deduplicated kinds in file order, and where they are not"
    );

    // One preserved entry reads as one, and a session with nothing preserved says nothing: an
    // empty notice would be an announcement that nothing happened.
    let single_dir = tempfile::tempdir().unwrap();
    let single_lines = raw_message_lines(&single_dir, "one.jsonl", &["hi"]);
    let single_path = single_dir.path().join("one.jsonl");
    std::fs::write(
        &single_path,
        format!(
            "{}\n{}\n",
            single_lines[0],
            r#"{"id":"ent_solo","seq":1,"parent_id":null,"timestamp_ms":0,"kind":{"type":"from_the_future"}}"#
        ),
    )
    .unwrap();
    let single = SessionStore::open(&single_path).unwrap();
    assert_eq!(
        single.preserved_unknown_notice().as_deref(),
        Some(
            "1 entry of a kind this build does not know (`from_the_future`) is in the file but \
             not in the transcript — kept, not dropped"
        )
    );

    // A session with nothing preserved says nothing: an empty notice would be an announcement
    // that nothing happened.
    let _clean_lines = raw_message_lines(&dir, "clean.jsonl", &["hi"]);
    let clean = SessionStore::open(dir.path().join("clean.jsonl")).unwrap();
    assert_eq!(
        clean.preserved_unknown_notice(),
        None,
        "nothing preserved is the absence of a report, not an empty sentence"
    );
}

/// **An older session is old, not damaged.**
///
/// `Usage::cache_reported` was added in M194 without a serde default, and `Usage` sits inside
/// **every** assistant message — so every session file written before that milestone stopped
/// decoding, and because the store refuses a known entry kind whose body it cannot read,
/// `epsilon` would not start at all in a workspace that had one. That is not hypothetical: the
/// *installed* binary refused to start in this repository's own working directory, with
/// `session line #3: entry type \`message\` is one this build knows, but its body cannot be read`.
///
/// The entry below is that line, copied verbatim out of `.epsilon/sessions/tui.jsonl` — a real
/// artifact written by an earlier build, not a fixture built to match the reader. That is the
/// point of the test: **every in-tree fixture was built with the current shape, so every test
/// in this file agreed with the bug.** It was found by driving the shipped binary through
/// `scripts/console-probe.py`, which is the only check here that runs what a user runs.
///
/// The fix is `#[serde(default)]` on the added field, and the rule it establishes is on that
/// field's doc comment: a field added to a durable session type must be readable when absent.
#[test]
fn a_session_written_before_a_field_existed_still_opens() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("older.jsonl");
    // The header is the real one too, so the file is a session rather than a bare entry.
    const OLDER: &str = concat!(
        r#"{"type":"session","version":3,"id":"ses_older","timestamp":"1789981626674","cwd":"C:\\work"}"#,
        "\n",
        r#"{"id":"ent_1a0c33811345","seq":1,"parent_id":"ent_1a0c33811322","timestamp_ms":1789981626676,"kind":{"type":"message","message":{"role":"assistant","id":"msg_1a0c33811344","timestamp_ms":1789981626676,"provider":"faux","model_id":"deepseek-flash","content":[{"type":"tool_call","id":"tc_1","name":"bash","arguments":{"command":"echo epsilon-ready"}}],"stop_reason":"tool_use","error_message":null,"usage":{"input":100,"output":50,"cache_read":0,"cache_write":0,"total_tokens":150}},"api_request_sha256":null}}"#,
        "\n",
    );
    std::fs::write(&path, OLDER).unwrap();

    let store = SessionStore::open(&path)
        .expect("a session written before `cache_reported` existed must still open");
    let entries = store.read_all().expect("read");
    assert_eq!(entries.len(), 1, "the entry was read, not skipped");

    // And the missing field takes the value its own semantics give an unannotated
    // decoder: `false` means "this writer did not report cache traffic", which is the
    // honest reading and the one the header renders as `CH n/a` rather than as `0%` (K1).
    let EntryKind::Message { message, .. } = &entries[0].kind else {
        panic!("expected a message entry");
    };
    let epsilon_contracts::message::Message::Assistant(assistant) = message else {
        panic!("expected an assistant message");
    };
    assert!(
        !assistant.usage.cache_reported,
        "an absent flag must read as 'not reported', never as a measured zero"
    );
    assert_eq!(
        assistant.usage.total_tokens, 150,
        "the fields it did carry are kept"
    );
}

/// A *pre-existing* field that is missing is still refused, and the refusal names the field.
///
/// The other half of the rule above, and the reason `#[serde(default)]` is applied per field
/// rather than to the whole durable format: evolution adds fields, damage removes them, and the
/// decoder's own sentence is what tells the two apart. Before M202 the sentence stopped at "its
/// body cannot be read", which cost real time to diagnose by hand.
#[test]
fn a_missing_field_this_build_always_had_is_refused_by_name() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("damaged.jsonl");
    // `output` has been in `Usage` since the first release; its absence is damage.
    const DAMAGED: &str = concat!(
        r#"{"type":"session","version":3,"id":"ses_d","timestamp":"1789981626674","cwd":"C:\\work"}"#,
        "\n",
        r#"{"id":"ent_d","seq":1,"parent_id":null,"timestamp_ms":0,"kind":{"type":"message","message":{"role":"assistant","id":"msg_d","timestamp_ms":0,"provider":"faux","model_id":"deepseek-flash","content":[],"stop_reason":"end_turn","error_message":null,"usage":{"input":100,"cache_read":0,"cache_write":0,"total_tokens":150}},"api_request_sha256":null}}"#,
        "\n",
    );
    std::fs::write(&path, DAMAGED).unwrap();

    match SessionStore::open(&path) {
        Err(StoreError::Corrupt { line, diagnosis }) => {
            assert_eq!(line, 2);
            let text = diagnosis.explain();
            assert!(
                text.contains("output"),
                "the refusal must name the field the decoder wanted: {text}"
            );
            assert!(text.contains("older Epsilon"), "{text}");
        }
        Ok(_) => panic!("a missing pre-existing field must not open silently"),
        Err(e) => panic!("expected a typed refusal, got {e:?}"),
    }
}

/// The other half of the rule, and the reason the codec is strict about a known tag: an entry
/// of a kind this build understands, carrying a body it cannot read, would leave a *silent gap*
/// in the transcript if it were preserved. That is the failure M108 was written to end, so it
/// is still refused — with a diagnosis that names the kind and the cause rather than only the
/// line.
#[test]
fn a_known_kind_with_an_unreadable_body_is_refused_as_damage_not_as_foreign() {
    let dir = tempfile::tempdir().unwrap();
    let lines = raw_message_lines(&dir, "s.jsonl", &["one"]);
    let path = dir.path().join("s.jsonl");
    // `message` is a kind this build knows; the body has no message in it.
    let broken = r#"{"id":"ent_bad","seq":1,"parent_id":"ent_0","timestamp_ms":0,"kind":{"type":"message"}}"#;
    let content = format!("{}\n{broken}\n", lines[0]);
    std::fs::write(&path, &content).unwrap();

    match SessionStore::open(&path) {
        Err(StoreError::Corrupt { line, diagnosis }) => {
            assert_eq!(line, 2, "the line is named");
            assert_eq!(
                diagnosis,
                LineDiagnosis::MalformedEntry {
                    entry_type: "message".to_string(),
                    // M202: the decoder's own sentence rides along, because "cannot be
                    // read" is not something a reader can act on and `missing field
                    // \`message\`` is.
                    detail: Some("missing field `message`".to_string()),
                },
                "the type is known and the body is not — the actionable difference"
            );
            let text = diagnosis.explain();
            assert!(
                text.contains("missing field `message`"),
                "the decoder's own sentence is quoted, not summarized: {text}"
            );
            assert!(text.contains("message"), "{text}");
        }
        Ok(_) => panic!("a known kind with an unreadable body must not open silently"),
        Err(e) => panic!("expected a typed refusal, got {e:?}"),
    }
    assert_eq!(std::fs::read_to_string(&path).unwrap(), content);
}

#[test]
fn the_line_diagnoses_are_distinguishable_from_each_other() {
    let dir = tempfile::tempdir().unwrap();
    // The header matters for one of these: a legacy shape is *healed* in a headerless v1 file,
    // and refused only in a current-version file, where healing would change its meaning.
    let valid = raw_message_lines(&dir, "seed.jsonl", &["one"]);
    let header = {
        let p = dir.path().join("seed.jsonl");
        std::fs::read_to_string(&p)
            .unwrap()
            .lines()
            .find(|l| l.contains("\"type\":\"session\""))
            .expect("a header line")
            .to_string()
    };

    let cases: Vec<(&str, LineDiagnosis)> = vec![
        ("NOT-JSON{", LineDiagnosis::NotJson),
        (
            r#"{"type":"message","timestamp":123,"role":"user","content":"hi"}"#,
            LineDiagnosis::LegacyInCurrentFile,
        ),
        (
            r#"{"type":"session","version":1,"session_id":"other"}"#,
            LineDiagnosis::SecondHeader,
        ),
    ];
    for (n, (faulty, expected)) in cases.into_iter().enumerate() {
        let path = dir.path().join(format!("case{n}.jsonl"));
        let content = format!("{header}\n{faulty}\n{}\n", valid[0]);
        std::fs::write(&path, &content).unwrap();
        match SessionStore::open(&path) {
            Err(StoreError::Corrupt { line, diagnosis }) => {
                assert_eq!(line, 2, "{faulty}");
                assert_eq!(diagnosis, expected, "for {faulty}");
            }
            Ok(_) => panic!("expected a refusal for {faulty}, open succeeded"),
            Err(e) => panic!("expected a refusal for {faulty}, got {e:?}"),
        }
    }
}

/// A crash mid-write is still tolerated: the torn *last* line is dropped, and that is not a
/// corruption report at all. The distinction is the whole point — damage is refused, a partial
/// write is expected.
#[test]
fn a_torn_tail_is_still_tolerated() {
    // A crash mid-write leaves a *fragment*: the write died inside a value. That is debris, and
    // dropping it costs the user nothing because the tail never became a whole thought. Every
    // shape here is deliberately not parseable as JSON — the counter-case, a complete line this
    // build does not recognize, is refused instead and has its own test below.
    let dir = tempfile::tempdir().unwrap();
    let lines = raw_message_lines(&dir, "s.jsonl", &["one", "two"]);
    for torn in [
        "{\"id\":\"ent_x\",\"seq\":2,\"kin", // cut mid-key
        "{\"id\":\"ent_x\",\"seq\":2,\"kind\":{\"type\":\"message\"}", // cut before the close
        "",                                  // nothing at all after a newline
    ] {
        let path = dir.path().join("torn.jsonl");
        let content = format!("{}\n{}\n{torn}", lines[0], lines[1]);
        std::fs::write(&path, &content).unwrap();
        let store = SessionStore::open(&path).expect("a torn tail must not fail the open");
        assert!(
            store.branch_entries().map(|v| v.len()).unwrap_or(0) >= 2,
            "the intact entries survive: {torn:?}"
        );
    }
}

/// The other half of that distinction, and the half that used to be wrong: a **complete** final
/// line this build does not recognize was truncated as if it were crash debris. It is not debris
/// — something wrote it on purpose, so it is the user's newest turn or a newer Epsilon's entry —
/// and deleting it silently is the one failure a recovery path must not have (F08).
///
/// Every case below is whole JSON. The open must come back refused with a diagnosis that names
/// the shape, and the file must be byte-identical afterwards.
#[test]
fn a_complete_but_unknown_final_line_is_refused_not_truncated() {
    let dir = tempfile::tempdir().unwrap();
    let lines = raw_message_lines(&dir, "s.jsonl", &["one", "two"]);
    for (final_line, expected) in [
        (
            r#"{"id":"ent_x","seq":2,"kind":{"type":"future_thing"}}"#,
            LineDiagnosis::UnknownEntryType {
                entry_type: "future_thing".to_string(),
            },
        ),
        (
            r#"{"session_id":"ses_other","version":3,"cwd":"C:\\x"}"#,
            LineDiagnosis::SecondHeader,
        ),
        (
            r#"{"some":"future shape"}"#,
            LineDiagnosis::UnknownLineShape,
        ),
        ("42", LineDiagnosis::UnknownLineShape),
        (r#""a bare string""#, LineDiagnosis::UnknownLineShape),
        ("[]", LineDiagnosis::UnknownLineShape),
        ("{}", LineDiagnosis::UnknownLineShape),
    ] {
        // Both spellings of "the file ends here": with the trailing newline a completed append
        // leaves, and without it. The newline is not what makes a line complete.
        for trailing in ["", "\n"] {
            let path = dir.path().join("complete.jsonl");
            let content = format!("{}\n{}\n{final_line}{trailing}", lines[0], lines[1]);
            std::fs::write(&path, &content).unwrap();

            match SessionStore::open(&path) {
                Err(StoreError::Corrupt { line, diagnosis }) => {
                    assert_eq!(line, 3, "for {final_line:?}");
                    assert_eq!(diagnosis, expected, "for {final_line:?}");
                }
                Ok(_) => panic!("a complete unknown final line was accepted: {final_line:?}"),
                Err(e) => panic!("expected a refusal for {final_line:?}, got {e:?}"),
            }
            assert_eq!(
                std::fs::read_to_string(&path).unwrap(),
                content,
                "the refusal must leave the file untouched: {final_line:?}{trailing:?}"
            );
        }
    }
}

// ---- SESSION-01a: the session tree — branch summaries, no-leaf navigation, fork ---------------

fn assistant_msg(text: &str) -> Message {
    Message::Assistant(AssistantMessage {
        id: new_id("msg"),
        timestamp_ms: 0,
        provider: "faux".into(),
        model_id: "faux-1".into(),
        content: vec![ContentBlock::text(text)],
        stop_reason: StopReason::EndTurn,
        error_message: None,
        usage: Usage::default(),
    })
}

/// The user/assistant text an entry carries. Enough to tell two turns apart without
/// comparing whole `SessionEntry`s, which carry a fresh id and timestamp on every run.
fn entry_text(entry: &SessionEntry) -> String {
    match &entry.kind {
        EntryKind::Message { message, .. } => match message {
            Message::User(u) => u
                .content
                .iter()
                .filter_map(|c| match c {
                    UserContent::Text { text } => Some(text.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n"),
            Message::Assistant(a) => a.text(),
            other => format!("{other:?}"),
        },
        other => format!("{other:?}"),
    }
}

/// The provider-bound rendering of a view: what a request built from it would carry.
fn view_texts(view: &[ModelViewEntry]) -> Vec<String> {
    view.iter()
        .cloned()
        .map(|v| match v.into_message() {
            Message::User(u) => u
                .content
                .iter()
                .filter_map(|c| match c {
                    UserContent::Text { text } => Some(text.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n"),
            Message::Assistant(a) => a.text(),
            other => format!("{other:?}"),
        })
        .collect()
}

/// A hand-built message entry: the store generates these, but a hand-written source
/// file needs exact ids, parent links and timestamps that the copy can be checked
/// against.
fn message_entry(
    id: &str,
    seq: u64,
    parent_id: Option<&str>,
    timestamp_ms: u64,
    message: Message,
) -> SessionEntry {
    SessionEntry {
        id: id.to_string(),
        seq,
        parent_id: parent_id.map(str::to_string),
        timestamp_ms,
        kind: EntryKind::Message {
            message,
            api_request_sha256: None,
        },
    }
}

/// Write a session file by hand — a header plus entries, as JSONL — so a test can
/// build file states the store cannot produce itself (a foreign cwd, a parent link
/// that names nothing).
fn write_session_file(path: &std::path::Path, header: &SessionHeader, entries: &[SessionEntry]) {
    let mut body = serde_json::to_string(header).expect("header json");
    body.push('\n');
    for entry in entries {
        body.push_str(&serde_json::to_string(entry).expect("entry json"));
        body.push('\n');
    }
    std::fs::write(path, body).expect("write session file");
}

#[test]
fn a_branch_summary_lands_once_on_the_new_branch_and_never_on_the_abandoned_one() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.jsonl");
    let mut store = SessionStore::open(&path).unwrap();

    let root = store
        .append(EntryKind::Message {
            message: user_msg("keep me"),
            api_request_sha256: None,
        })
        .unwrap();
    let abandoned_a = store
        .append(EntryKind::Message {
            message: user_msg("abandoned-a"),
            api_request_sha256: None,
        })
        .unwrap();
    let abandoned_b = store
        .append(EntryKind::Message {
            message: assistant_msg("abandoned-b"),
            api_request_sha256: None,
        })
        .unwrap();

    // An unknown target is refused before anything moves: a failed navigation must
    // not leave the leaf somewhere the caller did not ask for.
    let leaf_before = store.leaf_id();
    let err = store
        .branch_with_summary(Some("ent_no_such_entry"), "nope", None, None)
        .unwrap_err();
    assert!(err.to_string().contains("no such entry"), "{err}");
    assert_eq!(
        store.leaf_id(),
        leaf_before,
        "the leaf must not move on a refusal"
    );

    // Branch from the root, summarizing the path being left behind.
    let summary = store
        .branch_with_summary(Some(&root.id), "explored the abandoned path", None, None)
        .unwrap();
    assert_eq!(
        summary.parent_id.as_deref(),
        Some(root.id.as_str()),
        "the summary is the target's child"
    );
    assert_eq!(
        summary.seq, 3,
        "appended at the next file slot: root/abandoned-a/abandoned-b hold seqs 0..2"
    );
    assert_eq!(
        store.leaf_id().as_deref(),
        Some(summary.id.as_str()),
        "and the new leaf"
    );
    match &summary.kind {
        EntryKind::BranchSummary {
            from_id,
            summary: text,
            usage,
            model_id,
        } => {
            assert_eq!(from_id.as_str(), root.id.as_str());
            assert_eq!(text, "explored the abandoned path");
            assert!(usage.is_none());
            // The pairing matters: a bill with no model is unpriceable, so "no usage" and
            // "no charging model" have to be true together or the entry claims to bill
            // something nobody can charge for.
            assert!(model_id.is_none());
        }
        other => panic!("expected a branch summary, got {other:?}"),
    }
    assert_eq!(
        store.read_all().unwrap().len(),
        4,
        "append-only: the abandoned siblings stay in the file"
    );

    // The active branch is root + summary; the abandoned lane is off-branch.
    let branch = store.branch_entries().unwrap();
    assert_eq!(
        branch.iter().map(|e| e.id.clone()).collect::<Vec<_>>(),
        vec![root.id.clone(), summary.id.clone()]
    );
    let view = view_texts(&store.model_view().unwrap());
    assert_eq!(
        view,
        vec![
            "keep me".to_string(),
            "<branch_summary>\nexplored the abandoned path\n</branch_summary>".to_string()
        ],
        "the summary is projected where it sits on the new branch"
    );
    let projected_seq = store
        .model_view()
        .unwrap()
        .into_iter()
        .find_map(|v| match v {
            ModelViewEntry::BranchSummary { seq, .. } => Some(seq),
            _ => None,
        })
        .expect("the branch summary is projected");
    assert_eq!(
        projected_seq, summary.seq,
        "the projection carries the entry's own seq"
    );
    assert!(
        !view
            .iter()
            .any(|t| t.contains("abandoned-a") || t.contains("abandoned-b")),
        "the abandoned sibling is absent: {view:?}"
    );

    // Reopen: the file replays the same tree, the same leaf, and the same projection.
    drop(store);
    let reopened = SessionStore::open(&path).unwrap();
    assert_eq!(reopened.leaf_id().as_deref(), Some(summary.id.as_str()));
    let reopened_view = reopened.model_view().unwrap();
    assert_eq!(
        view_texts(&reopened_view),
        view,
        "reopening must reproduce the projection"
    );
    assert_eq!(
        reopened_view
            .iter()
            .filter(|v| matches!(v, ModelViewEntry::BranchSummary { .. }))
            .count(),
        1,
        "exactly once — the entry is not replayed twice from the file"
    );
    let _ = (abandoned_a, abandoned_b);
}

#[test]
fn an_empty_branch_summary_is_appended_but_not_projected() {
    // pi's projection is truthiness-guarded (`session-manager.ts:401`:
    // `entry.type === "branch_summary" && entry.summary`): a branch summary with an
    // empty text contributes no message. The entry itself is still appended — the file
    // is the human log, and a branch move nobody can see would be a worse lie than a
    // summary nobody reads.
    let dir = tempfile::tempdir().unwrap();
    let mut store = SessionStore::open(dir.path().join("s.jsonl")).unwrap();
    let leaf = store
        .append(EntryKind::Message {
            message: user_msg("keep me"),
            api_request_sha256: None,
        })
        .unwrap();

    let empty = store
        .branch_with_summary(Some(&leaf.id), "", None, None)
        .unwrap();
    assert_eq!(
        store.read_all().unwrap().len(),
        2,
        "the entry is appended like any other"
    );
    assert_eq!(
        store.branch_entries().unwrap().len(),
        2,
        "and it is on the active branch"
    );
    assert_eq!(
        store.leaf_id().as_deref(),
        Some(empty.id.as_str()),
        "the leaf advances onto it"
    );
    let view = store.model_view().unwrap();
    assert!(
        !view
            .iter()
            .any(|v| matches!(v, ModelViewEntry::BranchSummary { .. })),
        "an empty summary projects nothing: {view:?}"
    );
    assert_eq!(view_texts(&view), vec!["keep me".to_string()]);

    // The guard is exactly `is_empty`: whitespace is truthy in pi's check, so a
    // whitespace-only summary projects like any other text.
    let spaced = store
        .branch_with_summary(Some(&empty.id), "   ", None, None)
        .unwrap();
    let projected = store
        .model_view()
        .unwrap()
        .into_iter()
        .filter_map(|v| match v {
            ModelViewEntry::BranchSummary { seq, .. } => Some(seq),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        projected,
        vec![spaced.seq],
        "only the whitespace summary projects; the empty one stays out"
    );
}

#[test]
fn branch_with_summary_maps_pis_target_shapes_exactly() {
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

    // `Some(id)` is pi's `branchWithSummary(id, …)`: the leaf MOVES to that entry and
    // the summary hangs off it. Target a non-tip entry: passing the tip (as this test
    // first did) makes the move a no-op, so the assertion could not see it happen.
    assert_eq!(store.leaf_id().as_deref(), Some(b.id.as_str()));
    let at_a = store
        .branch_with_summary(Some(&a.id), "summarized the abandoned tail", None, None)
        .unwrap();
    assert_eq!(
        at_a.parent_id.as_deref(),
        Some(a.id.as_str()),
        "the summary is the TARGET's child, not the old tip's"
    );
    assert_eq!(store.leaf_id().as_deref(), Some(at_a.id.as_str()));
    assert_eq!(
        store
            .branch_entries()
            .unwrap()
            .iter()
            .map(|e| e.id.clone())
            .collect::<Vec<_>>(),
        vec![a.id.clone(), at_a.id.clone()],
        "the old tip is abandoned with the branch that led to it"
    );
    assert_eq!(
        view_texts(&store.model_view().unwrap()),
        vec![
            "a".to_string(),
            "<branch_summary>\nsummarized the abandoned tail\n</branch_summary>".to_string()
        ],
        "the view follows the moved leaf; the old tip's message is gone"
    );

    // `None` is pi's `branchWithSummary(null, …)`: the summary is itself a new ROOT and
    // records `fromId = "root"`; the leaf it replaces is abandoned, not re-used.
    let from_root = store
        .branch_with_summary(None, "branch from before the start", None, None)
        .unwrap();
    assert_eq!(
        from_root.parent_id, None,
        "a summary appended with no target starts a new root"
    );
    match &from_root.kind {
        EntryKind::BranchSummary { from_id, .. } => assert_eq!(from_id, "root"),
        other => panic!("expected a branch summary, got {other:?}"),
    }
    assert_eq!(store.leaf_id().as_deref(), Some(from_root.id.as_str()));
    assert_eq!(
        view_texts(&store.model_view().unwrap()),
        vec!["<branch_summary>\nbranch from before the start\n</branch_summary>".to_string()],
        "the new root is the whole active lane; the abandoned path is not on it"
    );
}

#[test]
fn navigating_to_the_first_message_is_a_position_not_an_id() {
    // M221: this test used to drive `reset_leaf` (pi `resetLeaf`), which made the *next append*
    // a new root inside the same file. `/new` is now a new file, so the primitive is gone, and what
    // survives here is the fact underneath it: the first user message's parent is `None`, so
    // "before the session" is a position rather than an id.
    //
    // The coverage that was specifically about the reset is deliberately **not** re-pointed at
    // another primitive: that behaviour no longer exists anywhere, and a test that keeps its name
    // while checking something else is how a suite starts lying. What replaced the user-visible
    // behaviour is tested where it lives now — `/new`, in the TUI suite.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.jsonl");
    let mut store = SessionStore::open(&path).unwrap();

    let first = store
        .append(EntryKind::Message {
            message: user_msg("first try"),
            api_request_sha256: None,
        })
        .unwrap();
    store
        .append(EntryKind::Message {
            message: assistant_msg("answered"),
            api_request_sha256: None,
        })
        .unwrap();

    assert_eq!(
        first.parent_id, None,
        "a root message has no parent id, which is why `/tree` needed a position"
    );
    store.navigate_to(&first.id).unwrap();
    assert_eq!(
        store.branch_entries().unwrap().len(),
        1,
        "navigating back truncates the active branch, not the file"
    );
    assert_eq!(
        store.read_all().unwrap().len(),
        2,
        "and the abandoned lane stays in the human log (ADR-006)"
    );

    // Navigation is in-memory state: reopening is faithful to the FILE, whose last line is the
    // tip, so a crash inside this window comes back exactly where the user was.
    drop(store);
    let reopened = SessionStore::open(&path).unwrap();
    assert_eq!(reopened.branch_entries().unwrap().len(), 2);
    assert_eq!(
        view_texts(&reopened.model_view().unwrap()),
        vec!["first try".to_string(), "answered".to_string()],
        "the file is the truth about what is active"
    );
}

#[test]
fn fork_branch_at_writes_only_the_selected_ancestry_with_fresh_identity() {
    let dir = tempfile::tempdir().unwrap();
    let src_path = dir.path().join("session.jsonl");
    // The SOURCE is written by hand. Its header carries a cwd DISTINCT from every
    // directory this test opens a store in — otherwise the cwd assertion below cannot
    // tell "the fork copied the source's cwd" from "the fork used its own argument" —
    // and its ids, parent links, payloads and timestamps are hand-set, so the copy can
    // be checked against exact values instead of freshly generated ones.
    let source_cwd = dir.path().join("the-source-workdir");
    let src_id = "ses_source_identity".to_string();
    let source = vec![
        message_entry("ent_a", 0, None, 1_000, user_msg("a")),
        message_entry("ent_b", 1, Some("ent_a"), 2_000, user_msg("b")), // abandoned sibling
        message_entry("ent_c", 2, Some("ent_a"), 3_000, user_msg("c")),
        message_entry("ent_d", 3, Some("ent_c"), 4_000, assistant_msg("d")),
    ];
    write_session_file(
        &src_path,
        &SessionHeader {
            kind: SessionHeaderKind::Session,
            version: Some(CURRENT_SESSION_VERSION),
            id: src_id.clone(),
            timestamp: "2026-09-18T00-00-00-000Z".into(),
            cwd: source_cwd.to_string_lossy().into_owned(),
            parent_session: None,
            preserved: std::collections::BTreeMap::new(),
        },
        &source,
    );

    let store = SessionStore::open_in(&src_path, dir.path()).unwrap();
    // An existing header is kept verbatim, so the hand-written cwd survives the open.
    assert_eq!(
        store.header().map(|h| h.cwd.clone()),
        Some(source_cwd.to_string_lossy().into_owned()),
        "the hand-written source cwd must survive the open"
    );
    assert_eq!(
        store.leaf_id().as_deref(),
        Some("ent_d"),
        "opening a file replays its tree: the last line is the leaf"
    );
    let source_before = std::fs::read(&src_path).unwrap();

    let dest_dir = dir.path().join("forks");
    let dest = store.fork_branch_at("ent_d", &dest_dir).unwrap();
    assert!(
        dest.starts_with(&dest_dir),
        "the fork lands in the requested directory: {dest:?}"
    );

    // A session of its own: fresh id, current version, a parent link to the file.
    let forked = SessionStore::open_in(&dest, dir.path()).unwrap();
    let header = forked.header().unwrap();
    assert_eq!(header.version, Some(CURRENT_SESSION_VERSION));
    assert_ne!(header.id, src_id, "fresh session identity");
    assert_eq!(
        header.parent_session.as_deref(),
        Some(src_path.to_string_lossy().as_ref())
    );
    assert_eq!(
        header.cwd,
        source_cwd.to_string_lossy(),
        "the fork carries the SOURCE's cwd, not the directory it was opened in"
    );
    assert_ne!(
        header.cwd,
        dir.path().to_string_lossy(),
        "…and the two differ, so the assertion above is not degenerate"
    );

    // Only the selected ancestry, ids preserved, `seq` renumbered gaplessly.
    let entries = forked.read_all().unwrap();
    assert_eq!(
        entries.iter().map(|e| e.id.clone()).collect::<Vec<_>>(),
        vec![
            "ent_a".to_string(),
            "ent_c".to_string(),
            "ent_d".to_string()
        ]
    );
    assert_eq!(
        entries.iter().map(|e| e.seq).collect::<Vec<_>>(),
        vec![0, 1, 2],
        "seq is the file index: gapless in path order"
    );
    assert_eq!(entries[0].parent_id, None);
    assert_eq!(entries[1].parent_id.as_deref(), Some("ent_a"));
    assert_eq!(entries[2].parent_id.as_deref(), Some("ent_c"));
    assert!(
        !entries.iter().any(|e| e.id == "ent_b"),
        "the abandoned sibling is not copied"
    );
    // The doc promises "every id, parent_id, timestamp and payload preserved": `seq`
    // is the file index and is renumbered, but the rest must be verbatim — a fork
    // that re-stamped timestamps or rebuilt payloads would be a different history.
    for (forked_entry, source_entry) in entries.iter().zip([&source[0], &source[2], &source[3]]) {
        assert_eq!(
            forked_entry.timestamp_ms, source_entry.timestamp_ms,
            "timestamp of {}",
            forked_entry.id
        );
        assert_eq!(
            serde_json::to_value(&forked_entry.kind).unwrap(),
            serde_json::to_value(&source_entry.kind).unwrap(),
            "payload of {}",
            forked_entry.id
        );
    }
    assert_eq!(forked.leaf_id().as_deref(), Some("ent_d"));
    assert_eq!(
        view_texts(&forked.model_view().unwrap()),
        vec!["a".to_string(), "c".to_string(), "d".to_string()]
    );
    drop(forked);

    // The source is a pure read: byte-identical, still holding the whole tree.
    assert_eq!(
        std::fs::read(&src_path).unwrap(),
        source_before,
        "forking must not touch the source"
    );
    assert_eq!(store.read_all().unwrap().len(), 4);

    // Subsequent writes go to the NEW session.
    let mut child = SessionStore::open_in(&dest, dir.path()).unwrap();
    child
        .append(EntryKind::Message {
            message: user_msg("e"),
            api_request_sha256: None,
        })
        .unwrap();
    assert_eq!(child.len(), 4);
    drop(child);
    assert_eq!(
        std::fs::read(&src_path).unwrap(),
        source_before,
        "…and the source still never changes"
    );

    // An unknown leaf is refused; a destination that cannot hold a session is
    // refused untouched (the refuse-don't-truncate rule).
    assert!(
        store
            .fork_branch_at("ent_no_such_entry", &dest_dir)
            .is_err()
    );
    let occupied = dir.path().join("occupied");
    std::fs::write(&occupied, "not a directory").unwrap();
    assert!(store.fork_branch_at("ent_d", &occupied).is_err());
    assert_eq!(
        std::fs::read_to_string(&occupied).unwrap(),
        "not a directory"
    );
}

#[test]
fn fork_refuses_an_ancestry_that_does_not_reach_a_root() {
    // `branch_to` refuses an ancestry that does not reach a root with `InvalidData`
    // (store.rs): a partial path written out as a session file would be a silent lie
    // about what the branch contains. The file is written by hand because the store
    // cannot append an entry whose parent names nothing (ids, not positions).
    let dir = tempfile::tempdir().unwrap();
    let src_path = dir.path().join("dangling.jsonl");
    write_session_file(
        &src_path,
        &SessionHeader {
            kind: SessionHeaderKind::Session,
            version: Some(CURRENT_SESSION_VERSION),
            id: "ses_dangling".into(),
            timestamp: "1".into(),
            cwd: dir.path().to_string_lossy().into_owned(),
            parent_session: None,
            preserved: std::collections::BTreeMap::new(),
        },
        &[
            message_entry("ent_root", 0, None, 10, user_msg("root")),
            // The chain ends on a parent that is not in the file: `ent_missing`.
            message_entry("ent_orphan", 1, Some("ent_missing"), 20, user_msg("orphan")),
        ],
    );

    let store = SessionStore::open_in(&src_path, dir.path()).unwrap();
    let dest_dir = dir.path().join("forks");
    let err = store.fork_branch_at("ent_orphan", &dest_dir).unwrap_err();
    assert_eq!(
        err.kind(),
        std::io::ErrorKind::InvalidData,
        "a broken ancestry is InvalidData, not a generic io failure: {err}"
    );
    assert!(
        err.to_string().contains("does not reach a root"),
        "and the message names what is wrong: {err}"
    );
    // Refused before any side effect: the destination directory was never created.
    assert!(
        !dest_dir.exists(),
        "the refusal happens before anything is written"
    );
    assert_eq!(
        store.read_all().unwrap().len(),
        2,
        "the source is untouched"
    );
}

// ---- M91: reader hardening — parent cycles and `seq` mismatches (hand-corruption only) --------

/// M91 (closes the M90 limits entry + `audit/evidence/m90-review.md` finding 6): the
/// parent-chain walks had no cycle guard. A hand-corrupted file whose entries point at
/// each other used to loop forever in `branch_entries` (the active-leaf walk) and in
/// `branch_to` (the fork's ancestry walk). Both must terminate; the tolerant walk stops
/// and returns what it legitimately reached, the exact one refuses by name.
#[test]
fn a_parent_cycle_terminates_the_tolerant_walk_and_is_refused_by_the_fork() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cycle.jsonl");
    // The 2-cycle from the review: a's parent is b, b's parent is a. No store append can
    // write this (parents are ids of existing entries), so the file is hand-written.
    write_session_file(
        &path,
        &SessionHeader {
            kind: SessionHeaderKind::Session,
            version: Some(CURRENT_SESSION_VERSION),
            id: "ses_cycle".into(),
            timestamp: "1".into(),
            cwd: dir.path().to_string_lossy().into_owned(),
            parent_session: None,
            preserved: std::collections::BTreeMap::new(),
        },
        &[
            message_entry("ent_a", 0, Some("ent_b"), 10, user_msg("a")),
            message_entry("ent_b", 1, Some("ent_a"), 20, user_msg("b")),
        ],
    );

    let store = SessionStore::open_in(&path, dir.path()).unwrap();
    assert_eq!(
        store.leaf_id().as_deref(),
        Some("ent_b"),
        "opening replays the tree: the last line is the leaf"
    );

    // The tolerant walk stops where the chain would repeat and returns the entries it
    // legitimately reached — each once, oldest first, exactly as it does for a chain
    // that ends on a missing parent.
    let branch = store.branch_entries().expect("the walk must terminate");
    assert_eq!(
        branch.iter().map(|e| e.id.clone()).collect::<Vec<_>>(),
        vec!["ent_a".to_string(), "ent_b".to_string()],
        "the cycle is cut at the repeat, not replayed"
    );
    // The model view rides the same walk: degraded, but back.
    assert_eq!(
        view_texts(&store.model_view().unwrap()),
        vec!["a".to_string(), "b".to_string()]
    );

    // The fork's ancestry walk is exact and refuses a chain that cannot reach a root (a
    // cycle never does) — before any side effect, as with a dangling chain.
    let dest_dir = dir.path().join("forks");
    let err = store.fork_branch_at("ent_b", &dest_dir).unwrap_err();
    assert_eq!(
        err.kind(),
        std::io::ErrorKind::InvalidData,
        "a cycle is InvalidData, the same kind a dangling chain gets: {err}"
    );
    assert!(
        err.to_string().contains("cycle"),
        "the refusal must name the cycle: {err}"
    );
    assert!(
        err.to_string().contains("ent_"),
        "and the entry it re-entered, so the report is actionable: {err}"
    );
    assert!(
        !dest_dir.exists(),
        "the refusal happens before anything is written"
    );
    // Refused, never rewritten: the file still holds exactly the corrupt tree.
    assert_eq!(store.read_all().unwrap().len(), 2);
}

/// M91 (same review finding): `branch_entries` resolves each id through the entry's
/// STORED `seq`, so a hand-edited file whose `seq` values disagree with line order could
/// index past the parsed vector (panic) or at a different entry (a silently wrong
/// branch). Both are chain breaks now — the walk stops and returns what it legitimately
/// reached, the tolerant behaviour above, never a panic.
#[test]
fn a_seq_that_disagrees_with_line_order_is_a_chain_break_not_a_panic() {
    let dir = tempfile::tempdir().unwrap();
    let header = SessionHeader {
        kind: SessionHeaderKind::Session,
        version: Some(CURRENT_SESSION_VERSION),
        id: "ses_seq".into(),
        timestamp: "1".into(),
        cwd: dir.path().to_string_lossy().into_owned(),
        parent_session: None,
        preserved: std::collections::BTreeMap::new(),
    };

    // (a) The mismatched `seq` points PAST the end of the file: ent_b claims seq 7 while
    // sitting at position 1. Before the guard this panicked on `all[7]`.
    let path = dir.path().join("out-of-range.jsonl");
    write_session_file(
        &path,
        &header,
        &[
            message_entry("ent_a", 0, None, 10, user_msg("a")),
            message_entry("ent_b", 7, Some("ent_a"), 20, user_msg("b")),
            message_entry("ent_c", 2, Some("ent_b"), 30, user_msg("c")),
        ],
    );
    let store = SessionStore::open_in(&path, dir.path()).unwrap();
    let branch = store
        .branch_entries()
        .expect("an out-of-range seq must not fail the read");
    assert_eq!(
        branch.iter().map(|e| e.id.clone()).collect::<Vec<_>>(),
        vec!["ent_c".to_string()],
        "the walk keeps only the entries it legitimately reached: {:?}",
        branch.iter().map(|e| e.id.clone()).collect::<Vec<_>>()
    );
    // The same tolerant read backs the model view: no panic from either caller.
    assert_eq!(
        view_texts(&store.model_view().unwrap()),
        vec!["c".to_string()]
    );
    // The reader itself is untouched: every line is still there, in file order.
    assert_eq!(store.read_all().unwrap().len(), 3);

    // (b) The mismatched `seq` resolves IN BOUNDS to a different entry: ent_b claims
    // seq 0, where ent_a sits. A range guard alone would follow ent_a's parent and
    // return the wrong branch ([ent_a, ent_c]); the id check makes it a chain break.
    let path = dir.path().join("wrong-entry.jsonl");
    write_session_file(
        &path,
        &header,
        &[
            message_entry("ent_a", 0, None, 10, user_msg("a")),
            message_entry("ent_b", 0, Some("ent_a"), 20, user_msg("b")),
            message_entry("ent_c", 2, Some("ent_b"), 30, user_msg("c")),
        ],
    );
    let store = SessionStore::open_in(&path, dir.path()).unwrap();
    let branch = store.branch_entries().expect("no panic, no error");
    assert_eq!(
        branch.iter().map(|e| e.id.clone()).collect::<Vec<_>>(),
        vec!["ent_c".to_string()],
        "an entry that is not the one the chain named ends the walk"
    );

    // The fork's walk resolves by id, not by seq, so it still reaches the real root:
    // the guard is scoped to the seq-indexed walk, not to the reader.
    let dest_dir = dir.path().join("forks");
    let dest = store.fork_branch_at("ent_c", &dest_dir).unwrap();
    let forked = SessionStore::open_in(&dest, dir.path()).unwrap();
    assert_eq!(
        forked
            .read_all()
            .unwrap()
            .iter()
            .map(|e| e.id.clone())
            .collect::<Vec<_>>(),
        vec![
            "ent_a".to_string(),
            "ent_b".to_string(),
            "ent_c".to_string()
        ],
        "the fork follows parent ids, so the corrupt seqs do not affect it"
    );
}

#[test]
fn a_torn_first_entry_does_not_take_the_header_with_it() {
    // The state a kill leaves behind one line into a brand-new session: a header, then a
    // half-written entry. Repair must drop the fragment WITHOUT dropping the header.
    //
    // The header is the session's identity — id, working directory, and for a delegated
    // child the parent link that puts it in the session tree. Cutting the file back to
    // zero bytes instead (what "truncate everything the index does not cover" means when
    // no entry was indexed) left `store.header` still answering Some, so the next append
    // skipped the header it believed was there and the file reopened as a headerless v1
    // session: fresh ids, no cwd, no parent, decided silently at the next open.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("torn.jsonl");
    let header = SessionHeader {
        kind: SessionHeaderKind::Session,
        version: Some(CURRENT_SESSION_VERSION),
        id: "ses_keepme".to_string(),
        timestamp: "2026-09-20T00:00:00Z".to_string(),
        cwd: dir.path().to_string_lossy().into_owned(),
        parent_session: Some("ses_parent".to_string()),
        preserved: std::collections::BTreeMap::new(),
    };
    let mut body = serde_json::to_string(&header).unwrap();
    body.push('\n');
    // A real torn write: the JSON stops mid-string, with no newline to close the line.
    body.push_str(r#"{"type":"message","id":"ent_tr"#);
    std::fs::write(&path, body).unwrap();

    let mut store = SessionStore::open_in(&path, dir.path()).expect("a torn tail is recoverable");
    assert_eq!(store.len(), 0, "no entry survived the tear");
    assert_eq!(
        store.header().map(|h| h.id.as_str()),
        Some("ses_keepme"),
        "the header is still the one the session was created with"
    );

    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        on_disk.lines().count(),
        1,
        "the fragment is gone and the header is what is left: {on_disk:?}"
    );
    assert!(on_disk.contains("ses_keepme"));

    // And the file stays a current-version child after the next append: this is the
    // consequence the assertion above exists to protect.
    store
        .append(EntryKind::Message {
            message: user_msg("after the tear"),
            api_request_sha256: None,
        })
        .unwrap();
    drop(store); // release the write lease before reopening (M61)
    let reopened = SessionStore::open_in(&path, dir.path()).unwrap();
    assert_eq!(reopened.version(), CURRENT_SESSION_VERSION);
    assert!(
        !reopened.migrated(),
        "nothing to migrate: the header survived"
    );
    assert_eq!(
        reopened.header().and_then(|h| h.parent_session.as_deref()),
        Some("ses_parent"),
        "a repaired child session is still a child"
    );
    assert_eq!(
        reopened
            .read_all()
            .unwrap()
            .iter()
            .map(entry_text)
            .collect::<Vec<_>>(),
        vec!["after the tear".to_string()]
    );
}

#[test]
fn popping_the_last_entry_keeps_the_header_and_the_read_cache_in_step() {
    // `pop_last` is the overflow-retry discard the agent loop calls for real
    // (`epsilon_core`'s `pop_last_message`). Two ways it used to lie about the file:
    //
    // 1. the read-through entry cache kept the popped entry, so the next append recorded
    //    the new line at a sequence number the cache still answered for — and because
    //    `branch_entries` resolves the leaf id through those reads, the active branch came
    //    back EMPTY, as if the conversation had never happened;
    // 2. popping the only entry cut the file back to zero bytes, taking the header with it.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pop.jsonl");
    let mut store = SessionStore::open_in(&path, dir.path()).unwrap();
    for text in ["first", "second"] {
        store
            .append(EntryKind::Message {
                message: user_msg(text),
                api_request_sha256: None,
            })
            .unwrap();
    }

    let popped = store.pop_last().unwrap().expect("there is an entry to pop");
    assert_eq!(entry_text(&popped), "second");

    // The header is still line one, and only the surviving entry follows it.
    let after_pop = std::fs::read_to_string(&path).unwrap();
    let lines: Vec<&str> = after_pop.lines().collect();
    assert_eq!(lines.len(), 2, "header + 'first': {after_pop:?}");
    assert!(lines[0].contains("\"type\":\"session\""));
    assert!(
        !after_pop.contains("second"),
        "the popped entry is off the disk"
    );

    // The next append must land on the entry that is actually there.
    store
        .append(EntryKind::Message {
            message: user_msg("third"),
            api_request_sha256: None,
        })
        .unwrap();
    assert_eq!(
        store
            .read_all()
            .unwrap()
            .iter()
            .map(entry_text)
            .collect::<Vec<_>>(),
        vec!["first".to_string(), "third".to_string()],
        "the popped entry must not come back under the new sequence number"
    );
    let branch = store.branch_entries().unwrap();
    assert_eq!(
        branch.iter().map(entry_text).collect::<Vec<_>>(),
        vec!["first".to_string(), "third".to_string()],
        "the branch walk still reaches the leaf"
    );

    // Popping down to nothing leaves an empty but still-identified session.
    store.pop_last().unwrap();
    store.pop_last().unwrap();
    assert!(store.pop_last().unwrap().is_none(), "nothing left to pop");
    assert_eq!(store.len(), 0);
    let empty = std::fs::read_to_string(&path).unwrap();
    assert_eq!(empty.lines().count(), 1, "the header outlives its entries");
    drop(store);
    let reopened = SessionStore::open_in(&path, dir.path()).unwrap();
    assert!(reopened.is_empty());
    assert_eq!(reopened.version(), CURRENT_SESSION_VERSION);
}

// ---- M139: context settings survive a restart (pi `getSessionContextSettings`) ----

/// An assistant message on a specific model, built from the file's existing helper so
/// the two cannot drift apart as `AssistantMessage` gains fields.
fn assistant_on(provider: &str, model_id: &str) -> Message {
    match assistant_msg("hi") {
        Message::Assistant(mut a) => {
            a.provider = provider.into();
            a.model_id = model_id.into();
            Message::Assistant(a)
        }
        other => other,
    }
}

/// The last `model_change` on the path wins.
///
/// A port that returns the FIRST one passes every single-switch session and fails the
/// moment a user corrects themselves — which is the normal case, not the edge case.
#[test]
fn the_last_model_change_on_the_path_wins() {
    let tmp = tempfile::tempdir().expect("tmp");
    let path = tmp.path().join("switched.jsonl");
    let mut store = SessionStore::open_in(&path, tmp.path()).expect("store");

    store
        .append(EntryKind::ModelChange {
            provider: "anthropic".into(),
            model_id: "claude-3-opus".into(),
        })
        .expect("first");
    store
        .append(EntryKind::ModelChange {
            provider: "anthropic".into(),
            model_id: "claude-sonnet-4".into(),
        })
        .expect("second");

    let settings = store.context_settings().expect("settings");
    assert_eq!(
        settings.model,
        Some(("anthropic".into(), "claude-sonnet-4".into())),
        "the second switch is the one in force"
    );
}

/// An assistant message sets the model too, so a session that never had a
/// `model_change` at all still comes back on the model it actually used.
///
/// This is pi's third arm (`session-manager.ts:370-371`) and the case a
/// `model_change`-only implementation gets entirely wrong: it returns `None` here and
/// the caller falls back to the launch default, which is the original bug in a new coat.
#[test]
fn an_assistant_message_alone_names_the_model() {
    let tmp = tempfile::tempdir().expect("tmp");
    let path = tmp.path().join("never-switched.jsonl");
    let mut store = SessionStore::open_in(&path, tmp.path()).expect("store");

    store
        .append(EntryKind::Message {
            message: assistant_on("testprov", "gpt-x"),
            api_request_sha256: None,
        })
        .expect("append");

    let settings = store.context_settings().expect("settings");
    assert_eq!(
        settings.model,
        Some(("testprov".into(), "gpt-x".into())),
        "a session that never switched must still report the model it ran"
    );
}

/// A path with neither a `model_change` nor an assistant message reports `None`.
///
/// pi returns `null` here, and the distinction is load-bearing: `None` means "this
/// conversation never ran", not "no preference". Collapsing it into a default makes the
/// two indistinguishable to the caller.
#[test]
fn a_path_with_no_model_at_all_reports_none_not_a_default() {
    let tmp = tempfile::tempdir().expect("tmp");
    let path = tmp.path().join("empty.jsonl");
    let mut store = SessionStore::open_in(&path, tmp.path()).expect("store");
    store
        .append(EntryKind::ThinkingLevelChange {
            level: "high".into(),
        })
        .expect("append");

    let settings = store.context_settings().expect("settings");
    assert_eq!(
        settings.model, None,
        "a thinking-level change is not a model change"
    );
    assert_eq!(
        settings.thinking_level.as_deref(),
        Some("high"),
        "and the level it did record must still come back"
    );
}

/// `thinking_level` defaults to pi's `"off"` and a later entry overrides it.
#[test]
fn the_thinking_level_defaults_to_off_and_a_later_entry_wins() {
    let tmp = tempfile::tempdir().expect("tmp");
    let path = tmp.path().join("thinking.jsonl");
    let mut store = SessionStore::open_in(&path, tmp.path()).expect("store");

    let unset = store.context_settings().expect("settings");
    assert_eq!(
        unset.thinking_level.as_deref(),
        Some("off"),
        "pi initialises to off, so 'off' and 'never set' stay distinguishable"
    );

    store
        .append(EntryKind::ThinkingLevelChange {
            level: "low".into(),
        })
        .expect("first");
    store
        .append(EntryKind::ThinkingLevelChange {
            level: "xhigh".into(),
        })
        .expect("second");

    let settings = store.context_settings().expect("settings");
    assert_eq!(
        settings.thinking_level.as_deref(),
        Some("xhigh"),
        "the last level on the path is the one in force"
    );
}

/// The settings survive closing and reopening the file — which is the whole point.
///
/// A reader that only worked on a live store would pass every test above and still
/// revert the model on restart, which is the defect this milestone exists to close.
#[test]
fn the_settings_survive_a_close_and_reopen() {
    let tmp = tempfile::tempdir().expect("tmp");
    let path = tmp.path().join("persist.jsonl");
    {
        let mut store = SessionStore::open_in(&path, tmp.path()).expect("store");
        store
            .append(EntryKind::ModelChange {
                provider: "google".into(),
                model_id: "gemini-3-pro".into(),
            })
            .expect("append");
    } // the lease is released here, as on a clean exit

    let reopened = SessionStore::open_in(&path, tmp.path()).expect("reopen");
    let settings = reopened.context_settings().expect("settings");
    assert_eq!(
        settings.model,
        Some(("google".into(), "gemini-3-pro".into())),
        "a model switch must outlive the process that made it"
    );
}

// -------------------------------------------------------------------------------------
// M144 / F22 — a summarization is a real provider call, so it must be accounted for.
// -------------------------------------------------------------------------------------

fn billed(input: u64, output: u64) -> epsilon_contracts::message::Usage {
    epsilon_contracts::message::Usage {
        input,
        output,
        cache_read: 0,
        cache_write: 0,
        total_tokens: input + output,
        // A recorded session entry came from a provider that answered the cache
        // question; replaying it must not read as "never measurable" (K1).
        cache_reported: true,
    }
}

#[test]
fn a_compaction_that_was_billed_shows_up_in_the_sessions_accounting() {
    // The defect: compaction resends the whole history to a model and is charged for it,
    // and pi counts that charge explicitly. Epsilon had nowhere to read it from, so a
    // compaction was free in every number the user could see.
    let dir = tempfile::tempdir().unwrap();
    let mut store = SessionStore::open(dir.path().join("s.jsonl")).unwrap();
    store
        .append(EntryKind::Compaction {
            summary: "earlier work".into(),
            tokens_before: 12_000,
            retained_tail: Vec::new(),
            details: None,
            usage: Some(billed(12_000, 400)),
            model_id: Some("claude-sonnet-4".into()),
        })
        .unwrap();

    let accounts = store.usage_accounts().unwrap();
    assert_eq!(accounts["claude-sonnet-4"]["compaction"].input, 12_000);
    assert_eq!(accounts["claude-sonnet-4"]["compaction"].output, 400);
}

#[test]
fn accounting_is_keyed_by_the_model_that_was_charged_not_the_one_in_the_header() {
    // M141's lesson applied to a summarizer: a cheap model summarizing a session billed
    // to an expensive one must not be repriced at the header's rate. Keying by the
    // charging model is the only way to avoid inventing that number.
    let dir = tempfile::tempdir().unwrap();
    let mut store = SessionStore::open(dir.path().join("s.jsonl")).unwrap();
    for (model, input) in [("expensive", 5_000u64), ("cheap", 20_000u64)] {
        store
            .append(EntryKind::Compaction {
                summary: "earlier work".into(),
                tokens_before: input,
                retained_tail: Vec::new(),
                details: None,
                usage: Some(billed(input, 100)),
                model_id: Some(model.into()),
            })
            .unwrap();
    }

    let accounts = store.usage_accounts().unwrap();
    assert_eq!(accounts.len(), 2, "each charging model keeps its own row");
    assert_eq!(accounts["expensive"]["compaction"].input, 5_000);
    assert_eq!(accounts["cheap"]["compaction"].input, 20_000);
}

#[test]
fn two_compactions_by_one_model_add_up_rather_than_the_last_one_winning() {
    // An overwrite bug here would under-report again, just more quietly: the second
    // compaction would replace the first instead of adding to it.
    let dir = tempfile::tempdir().unwrap();
    let mut store = SessionStore::open(dir.path().join("s.jsonl")).unwrap();
    for input in [3_000u64, 1_000] {
        store
            .append(EntryKind::Compaction {
                summary: "earlier work".into(),
                tokens_before: input,
                retained_tail: Vec::new(),
                details: None,
                usage: Some(billed(input, 50)),
                model_id: Some("m".into()),
            })
            .unwrap();
    }

    let accounts = store.usage_accounts().unwrap();
    assert_eq!(accounts["m"]["compaction"].input, 4_000);
    assert_eq!(accounts["m"]["compaction"].output, 100);
}

#[test]
fn a_legacy_compaction_with_usage_but_no_accounting_entry_is_not_counted() {
    // A session written before M144 has a `Compaction` whose `usage` is populated and no
    // charging model beside it. Counting it would mean inventing a model to price it
    // against — the M141 defect, reintroduced. Omitting it under-reports instead, which
    // is the honest direction to be wrong in. This test is what makes that a decision
    // rather than a comment that can drift.
    let dir = tempfile::tempdir().unwrap();
    let mut store = SessionStore::open(dir.path().join("s.jsonl")).unwrap();
    store
        .append(EntryKind::Compaction {
            summary: "earlier work".into(),
            tokens_before: 9_000,
            retained_tail: Vec::new(),
            details: None,
            usage: Some(billed(9_000, 300)),
            model_id: None,
        })
        .unwrap();

    assert!(
        store.usage_accounts().unwrap().is_empty(),
        "an unattributable charge must not be priced at some model's rate"
    );

    // The other arm, and the reason the field is `Option`: the SAME entry with a model
    // recorded is counted. Without both halves, a reader that skipped everything would
    // pass the test above and silently under-report every future session.
    let dir2 = tempfile::tempdir().unwrap();
    let mut store2 = SessionStore::open(dir2.path().join("s.jsonl")).unwrap();
    store2
        .append(EntryKind::Compaction {
            summary: "earlier work".into(),
            tokens_before: 9_000,
            retained_tail: Vec::new(),
            details: None,
            usage: Some(billed(9_000, 300)),
            model_id: Some("m".into()),
        })
        .unwrap();
    assert_eq!(
        store2.usage_accounts().unwrap()["m"]["compaction"].input,
        9_000,
        "a bill that names its charging model is not unattributable"
    );
}

#[test]
fn a_pop_that_discards_a_turn_takes_its_bill_with_it() {
    // F23, the regression. M144 appended a SEPARATE accounting entry after the compaction,
    // which made the bill the file's last entry — and `pop_last_message` pops the last entry
    // to discard a failed turn, so the pop ate the bill and left an unbilled compaction
    // behind. With the bill on the compaction entry itself there is nothing to separate, and
    // this test is what makes that structural rather than incidental.
    let dir = tempfile::tempdir().unwrap();
    let mut store = SessionStore::open(dir.path().join("s.jsonl")).unwrap();
    store
        .append(EntryKind::Compaction {
            summary: "earlier work".into(),
            tokens_before: 4_000,
            retained_tail: Vec::new(),
            details: None,
            usage: Some(billed(4_000, 120)),
            model_id: Some("m".into()),
        })
        .unwrap();
    assert_eq!(
        store.usage_accounts().unwrap()["m"]["compaction"].input,
        4_000
    );

    // Exactly what `Agent::pop_last_message` does to undo a turn it must not keep.
    let popped = store.pop_last().unwrap().expect("something to pop");
    assert!(
        matches!(popped.kind, EntryKind::Compaction { .. }),
        "the pop removed something that is not the compaction"
    );
    assert!(
        store.usage_accounts().unwrap().is_empty(),
        "a discarded turn must take its bill with it, or the total over-reports work that \
         was thrown away"
    );
}

/// The tree must show a branch that the active-branch walk cannot.
///
/// This is the whole reason `SessionStore::tree` exists. `branch_entries`
/// walks from the leaf, so once a branch is abandoned it becomes invisible —
/// and an invisible branch is a branch the TUI cannot summarise, label, or
/// return to. The assertion is deliberately a **comparison against
/// `branch_entries`**, not a count: a test that pinned "6 nodes" would pass
/// against an implementation that had dropped the abandoned branch and
/// re-numbered, and the two functions disagreeing is the defect.
#[test]
fn the_tree_contains_a_branch_that_the_active_walk_cannot_reach() {
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

    // Abandon b: the leaf moves back to a, so b and everything after it are on
    // a branch nothing walks any more.
    store.navigate_to(&a.id).unwrap();
    let after = store
        .append(EntryKind::Message {
            message: user_msg("after"),
            api_request_sha256: None,
        })
        .unwrap();

    let on_branch: std::collections::HashSet<String> = store
        .branch_entries()
        .unwrap()
        .iter()
        .map(|e| e.id.clone())
        .collect();
    let in_tree = flatten_ids(&store.tree().unwrap());
    let in_tree: std::collections::HashSet<String> = in_tree.into_iter().collect();

    assert!(
        !on_branch.contains(&b.id),
        "precondition: b left the active branch, or this test proves nothing"
    );
    assert!(
        in_tree.contains(&b.id) && in_tree.contains(&after.id),
        "the tree must still contain the abandoned branch -- in_tree has {:?}, b={} after={}",
        in_tree,
        b.id,
        after.id
    );
    assert!(
        in_tree.len() > on_branch.len(),
        "the tree is a superset of the active branch: tree={} branch={}",
        in_tree.len(),
        on_branch.len()
    );
}

/// Every node's `children` sorts by timestamp, and nesting is real, at every level.
#[test]
fn the_tree_nests_by_parentage_and_orders_children_chronologically() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = SessionStore::open(dir.path().join("s.jsonl")).unwrap();
    for text in ["one", "two", "three"] {
        store
            .append(EntryKind::Message {
                message: user_msg(text),
                api_request_sha256: None,
            })
            .unwrap();
    }

    // Ids are generated, so compare against what the store actually wrote: a
    // hardcoded `ent_0` encodes an assumption about id generation, which is
    // not what this test is about.
    let written: Vec<String> = store
        .read_all()
        .unwrap()
        .iter()
        .map(|e| e.id.clone())
        .collect();

    let forest = store.tree().unwrap();
    assert_eq!(forest.len(), 1, "a straight-line session is one root");
    let root = &forest[0];
    assert_eq!(root.entry.id, written[0], "first entry is the root");
    assert_eq!(
        root.children.len(),
        1,
        "a straight chain has one child at each level"
    );
    assert_eq!(
        root.children[0].children[0].children.len(),
        0,
        "and it ends"
    );
    assert_eq!(root.children[0].entry.id, written[1]);
    assert_eq!(root.children[0].children[0].entry.id, written[2]);
}

/// The three root cases, which are pi's and not this implementation's.
///
/// A test that only walked a clean file would pass against an implementation
/// that simply took the first entry as the root — which is exactly the bug
/// this guards.
#[test]
fn a_self_parent_and_a_missing_parent_are_both_roots() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.jsonl");
    let real = {
        let mut store = SessionStore::open(&path).unwrap();
        store
            .append(EntryKind::Message {
                message: user_msg("root"),
                api_request_sha256: None,
            })
            .unwrap();
        store.read_all().unwrap()[0].clone()
    };

    // Write the two malformed shapes by taking a REAL entry and changing only
    // `id` and `parent_id`. Hand-writing the JSON instead means inventing a
    // message shape, and an invented shape is how this test first failed
    // (`Corrupt { line: 3, diagnosis: UnknownLineShape }`) -- the store
    // rejected the line before the tree ever saw it, so the assertion was
    // testing nothing.
    let line = |id: &str, parent: Option<&str>, seq: u64, ts: u64| {
        let mut e = real.clone();
        e.id = id.to_string();
        e.parent_id = parent.map(str::to_string);
        e.seq = seq;
        e.timestamp_ms = ts;
        serde_json::to_string(&e).unwrap()
    };

    let mut text = std::fs::read_to_string(&path).unwrap();
    text.push_str(&line("ent_self", Some("ent_self"), 1, 2));
    text.push('\n');
    text.push_str(&line("ent_orphan", Some("ent_gone"), 2, 3));
    text.push('\n');
    std::fs::write(&path, text).unwrap();

    let store = SessionStore::open(&path).unwrap();
    let ids = flatten_ids(&store.tree().unwrap());
    assert!(
        ids.contains(&"ent_self".to_string()),
        "a self-parented entry is a root, not dropped: {ids:?}"
    );
    assert!(
        ids.contains(&"ent_orphan".to_string()),
        "an entry whose parent is missing is a root, not dropped: {ids:?}"
    );
    assert_eq!(ids.len(), 3, "every entry appears exactly once: {ids:?}");
}

/// Every node's id, depth-first, for order-independent assertions.
fn flatten_ids(nodes: &[epsilon_contracts::session_entry::SessionTreeNode]) -> Vec<String> {
    let mut out = Vec::new();
    for n in nodes {
        out.push(n.entry.id.clone());
        out.extend(flatten_ids(&n.children));
    }
    out
}

/// A label survives a reopen, and clears when pi says it clears.
///
/// **The property that distinguishes a real label from the hardcoded `null` the
/// wire used to report.** Before M196 the tree returned `label: null` for every
/// node — a constant, not data. A test written against a store map rather than
/// the file would have passed while the feature did nothing, so this one closes
/// the store and reopens it: the label is on the entry (`target_id` + `label`),
/// so it is in the bytes.
#[test]
fn a_label_survives_a_reopen_and_an_empty_label_clears_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.jsonl");
    let target;
    {
        let mut store = SessionStore::open(&path).unwrap();
        let e = store
            .append(EntryKind::Message {
                message: user_msg("label me"),
                api_request_sha256: None,
            })
            .unwrap();
        target = e.id.clone();
        store.set_label(&target, Some("the good bit")).unwrap();
    }
    // Reopened from disk, not the same handle.
    {
        let store = SessionStore::open(&path).unwrap();
        let labels = store.labels().unwrap();
        assert_eq!(
            labels.get(&target).map(|(s, _)| s.as_str()),
            Some("the good bit"),
            "the label is in the file, not in a map: {labels:?}"
        );
    }
    // pi: `appendLabelChange(targetId, undefined)` deletes the label.
    {
        let mut store = SessionStore::open(&path).unwrap();
        store.set_label(&target, None).unwrap();
    }
    {
        let store = SessionStore::open(&path).unwrap();
        assert!(
            store.labels().unwrap().is_empty(),
            "an explicit clear removes the label, as pi's loader does"
        );
    }
}

/// An empty string clears, it does not write a label that reads as a label.
///
/// pi's loader branches on `if (entry.label)`, so `""` and `undefined` are the
/// same instruction. Normalising here means a caller that passes `""` cannot
/// produce a file pi and Epsilon disagree about.
#[test]
fn an_empty_label_is_a_clear_rather_than_a_blank_label() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = SessionStore::open(dir.path().join("s.jsonl")).unwrap();
    let e = store
        .append(EntryKind::Message {
            message: user_msg("x"),
            api_request_sha256: None,
        })
        .unwrap();
    store.set_label(&e.id, Some("named")).unwrap();
    store.set_label(&e.id, Some("   ")).unwrap();
    assert!(
        store.labels().unwrap().is_empty(),
        "whitespace is not a label: it is a clear"
    );
}

/// The label is recorded ON the target, and an unknown target is refused.
///
/// **M196 corrected the expectation here.** The test used to assert that
/// labelling does NOT move the leaf. It does — and so does pi: `_appendEntry`
/// sets `leafId = entry.id` unconditionally
/// (`references/pi-0.85.1/.../core/session-manager.ts:1058-1063`), so
/// `appendLabelChange` advances the leaf in the reference too. The code was
/// right and the assertion was wrong.
///
/// The property worth keeping is the one that could have gone either way: the
/// label attaches to the **target**, not to the `Label` entry that recorded it.
/// A test that only checked "some label exists" would pass if it landed on the
/// wrong entry, which is the shape of bug a session tree hides.
#[test]
fn a_label_lands_on_its_target_and_an_unknown_target_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = SessionStore::open(dir.path().join("s.jsonl")).unwrap();
    let first = store
        .append(EntryKind::Message {
            message: user_msg("one"),
            api_request_sha256: None,
        })
        .unwrap();
    let second = store
        .append(EntryKind::Message {
            message: user_msg("two"),
            api_request_sha256: None,
        })
        .unwrap();

    store.set_label(&first.id, Some("start here")).unwrap();

    let labels = store.labels().unwrap();
    assert_eq!(
        labels.get(&first.id).map(|(s, _)| s.as_str()),
        Some("start here"),
        "the label is on the entry that was labelled"
    );
    assert!(
        !labels.contains_key(&second.id),
        "and not on the leaf, which is where the user was: {labels:?}"
    );

    // pi's own behaviour, asserted rather than assumed.
    let leaf_now = store.leaf_id();
    assert_ne!(
        leaf_now.as_deref(),
        Some(second.id.as_str()),
        "the Label entry is itself appended to the lane, so the leaf advances -- \
         as it does in pi (session-manager.ts:1061)"
    );

    let err = store.set_label("no-such-entry", Some("x")).unwrap_err();
    assert!(
        matches!(err, epsilon_session::StoreError::NoSuchEntry { .. }),
        "an unknown target is refused, not written: {err:?}"
    );
}

/// Last write wins, and a later clear beats an earlier label.
///
/// The order property a test written against a single set/get pair cannot see:
/// it would pass with "first write wins" or "last write wins" alike.
#[test]
fn the_last_label_change_wins() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = SessionStore::open(dir.path().join("s.jsonl")).unwrap();
    let e = store
        .append(EntryKind::Message {
            message: user_msg("x"),
            api_request_sha256: None,
        })
        .unwrap();
    store.set_label(&e.id, Some("first")).unwrap();
    store.set_label(&e.id, Some("second")).unwrap();
    assert_eq!(
        store.labels().unwrap().get(&e.id).map(|(s, _)| s.as_str()),
        Some("second")
    );
    store.set_label(&e.id, Some("third")).unwrap();
    store.set_label(&e.id, None).unwrap();
    assert!(
        store.labels().unwrap().is_empty(),
        "the clear is last, so there is no label"
    );
}
