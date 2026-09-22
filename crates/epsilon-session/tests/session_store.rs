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
    assert!(matches!(
        SessionLease::acquire(&session).unwrap(),
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

/// The other half of the rule, and the reason the codec is strict about a known tag: an entry
/// of a kind this build understands, carrying a body it cannot read, would leave a *silent gap*
/// in the transcript if it were preserved. That is the failure M108 was written to end, so it
/// is still refused — with a diagnosis that says "damaged", not "from a newer build".
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
                    entry_type: "message".to_string()
                },
                "the type is known and the body is not — the actionable difference"
            );
            let text = diagnosis.explain();
            assert!(text.contains("damaged rather than foreign"), "{text}");
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
        .branch_with_summary(Some("ent_no_such_entry"), "nope", None)
        .unwrap_err();
    assert!(err.to_string().contains("no such entry"), "{err}");
    assert_eq!(
        store.leaf_id(),
        leaf_before,
        "the leaf must not move on a refusal"
    );

    // Branch from the root, summarizing the path being left behind.
    let summary = store
        .branch_with_summary(Some(&root.id), "explored the abandoned path", None)
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
        } => {
            assert_eq!(from_id.as_str(), root.id.as_str());
            assert_eq!(text, "explored the abandoned path");
            assert!(usage.is_none());
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

    let empty = store.branch_with_summary(Some(&leaf.id), "", None).unwrap();
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
        .branch_with_summary(Some(&empty.id), "   ", None)
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
        .branch_with_summary(Some(&a.id), "summarized the abandoned tail", None)
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
        .branch_with_summary(None, "branch from before the start", None)
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
fn resetting_the_leaf_makes_the_next_append_a_new_root_and_reopens_faithfully() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.jsonl");
    let mut store = SessionStore::open(&path).unwrap();

    let first = store
        .append(EntryKind::Message {
            message: user_msg("first try"),
            api_request_sha256: None,
        })
        .unwrap();
    let reply = store
        .append(EntryKind::Message {
            message: assistant_msg("answered"),
            api_request_sha256: None,
        })
        .unwrap();

    // `/tree` re-edit of the FIRST user message: the target is its parent, which for
    // a root message is "before the session" — a position, not an id (pi `resetLeaf`).
    assert_eq!(first.parent_id, None);
    store.navigate_to(&first.id).unwrap();
    assert_eq!(store.branch_entries().unwrap().len(), 1);
    store.reset_leaf();
    assert_eq!(
        store.leaf_id(),
        None,
        "no leaf is a state, not a failed lookup"
    );
    assert!(
        store.branch_entries().unwrap().is_empty(),
        "no active branch until something is appended"
    );
    assert_eq!(
        store.read_all().unwrap().len(),
        2,
        "the reset itself writes nothing"
    );

    // The reset is in-memory state, not a file write — pi does not persist `leafId`
    // either, and the file is never rewritten (ADR-006). Reopening WITHOUT appending
    // restores the leaf from the file's last line: the old tip, exactly where a crash
    // inside this window would reopen.
    drop(store);
    let reopened_before_append = SessionStore::open(&path).unwrap();
    assert_eq!(
        reopened_before_append.leaf_id().as_deref(),
        Some(reply.id.as_str()),
        "the reset never reached the file"
    );
    assert_eq!(
        reopened_before_append.branch_entries().unwrap().len(),
        2,
        "so the old branch is the active one again"
    );
    drop(reopened_before_append);

    // Continue from the same reset position: the next append starts a new root.
    let mut store = SessionStore::open(&path).unwrap();
    store.reset_leaf();

    // The next append is a new root at the next file slot — not a rewrite of history.
    let retry = store
        .append(EntryKind::Message {
            message: user_msg("re-edited first try"),
            api_request_sha256: None,
        })
        .unwrap();
    assert_eq!(retry.parent_id, None, "the reset leaf starts a new root");
    assert_eq!(
        retry.seq, 2,
        "seq continues: nothing is renumbered or dropped"
    );
    assert_eq!(
        store.read_all().unwrap().len(),
        3,
        "the abandoned lane stays in the human log"
    );
    let branch = store.branch_entries().unwrap();
    assert_eq!(
        branch.iter().map(|e| e.id.clone()).collect::<Vec<_>>(),
        vec![retry.id.clone()]
    );
    assert_eq!(
        view_texts(&store.model_view().unwrap()),
        vec!["re-edited first try".to_string()]
    );

    // Reopening is faithful to the FILE: its last line is the new root, so that is
    // the leaf — the same rule pi's `_buildIndex` applies when it loads a session.
    drop(store);
    let reopened = SessionStore::open(&path).unwrap();
    assert_eq!(reopened.len(), 3);
    assert_eq!(reopened.leaf_id().as_deref(), Some(retry.id.as_str()));
    assert_eq!(
        view_texts(&reopened.model_view().unwrap()),
        vec!["re-edited first try".to_string()]
    );
    let _ = (first, reply);
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
