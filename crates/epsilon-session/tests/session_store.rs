//! Integration coverage for the session brick: append/read, tree navigation, model-view
//! shadowing, crash repair, and lease exclusivity.

use epsilon_contracts::agent_event::TurnEndReason;
use epsilon_contracts::content::{ContentBlock, UserContent};
use epsilon_contracts::message::{
    AssistantMessage, Message, StopReason, ToolResultMessage, Usage, UserMessage,
};
use epsilon_contracts::new_id;
use epsilon_contracts::session_entry::{CURRENT_SESSION_VERSION, EntryKind};
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
