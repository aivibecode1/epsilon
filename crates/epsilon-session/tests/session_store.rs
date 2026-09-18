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

/// The case this milestone exists for: a file from a *newer* Epsilon is not damaged, and saying
/// "corrupt line 2" about it sends the user looking for a fault that is not there.
#[test]
fn an_unknown_entry_type_is_refused_by_name_not_called_corruption() {
    let dir = tempfile::tempdir().unwrap();
    let lines = raw_message_lines(&dir, "s.jsonl", &["one"]);
    let path = dir.path().join("s.jsonl");
    // A well-formed entry from the future: same envelope, a kind this build has never seen.
    let future = r#"{"id":"ent_future","seq":1,"parent_id":null,"timestamp_ms":0,"kind":{"type":"telemetry_v9","payload":{"n":1}}}"#;
    let content = format!("{}\n{future}\n{}\n", lines[0], lines[0]);
    std::fs::write(&path, &content).unwrap();

    match SessionStore::open(&path) {
        Err(StoreError::Corrupt { line, diagnosis }) => {
            assert_eq!(line, 2, "the line is named");
            assert_eq!(
                diagnosis,
                LineDiagnosis::UnknownEntryType {
                    entry_type: "telemetry_v9".to_string()
                },
                "and so is the type, which is the actionable part"
            );
            let text = diagnosis.explain();
            assert!(text.contains("newer Epsilon"), "{text}");
            assert!(text.contains("telemetry_v9"), "{text}");
        }
        Ok(_) => panic!("expected a typed refusal, the open succeeded"),
        Err(e) => panic!("expected a typed refusal, got {e:?}"),
    }
    // Refused, not repaired: the file is untouched, so a newer build can still read it.
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
fn a_torn_tail_is_still_tolerated_whatever_its_shape() {
    let dir = tempfile::tempdir().unwrap();
    let lines = raw_message_lines(&dir, "s.jsonl", &["one", "two"]);
    for torn in [
        "{\"id\":\"ent_x\",\"seq\":2,\"kin", // cut mid-JSON
        r#"{"id":"ent_x","seq":2,"kind":{"type":"future_thing"}}"#, // complete JSON, unknown type
    ] {
        let path = dir.path().join("torn.jsonl");
        let content = format!("{}\n{}\n{torn}", lines[0], lines[1]);
        std::fs::write(&path, &content).unwrap();
        let store = SessionStore::open(&path).expect("a torn tail must not fail the open");
        assert!(
            store.branch_entries().map(|v| v.len()).unwrap_or(0) >= 2,
            "the intact entries survive"
        );
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
