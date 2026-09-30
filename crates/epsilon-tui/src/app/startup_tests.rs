//! M214: what the harness knows at startup and does not say (M205's intent, M210's shape).
//!
//! The condition under test is **"this workspace holds earlier sessions this launch is not
//! carrying"**. It used to be "the file this launch opened holds entries", which was the same
//! question while every frontend opened one fixed file — and became unreachable the moment M210
//! gave each launch a file of its own (A20). A launch that restored history says nothing, and a
//! workspace with no earlier sessions says nothing.

use super::*;
use crate::app::tests_support::test_app_with_store;

fn notices(app: &App) -> Vec<String> {
    app.ui.lines().iter().map(|l| l.text.clone()).collect()
}

/// A second session file in the same directory, written in the real format.
///
/// Hand-rolled JSON would rot the moment the header changes; this opens a store the way the product
/// does, which is also what makes the test's premise — "the directory holds a session that is not
/// this one" — true by construction rather than by a fixture that agrees with itself.
fn plant_earlier_session(own: &std::path::Path, name: &str) {
    let dir = own.parent().expect("a session file has a directory");
    let mut other = epsilon_session::SessionStore::open_in(dir.join(name), dir)
        .expect("a second session opens");
    other
        .append(epsilon_contracts::EntryKind::SessionInfo {
            name: Some("an earlier conversation".into()),
        })
        .expect("and records something");
}

#[test]
fn a_launch_in_a_workspace_with_earlier_sessions_says_so() {
    let (mut app, agent, _rx, _dir) = test_app_with_store();
    let own = agent.session_path().expect("attached");
    plant_earlier_session(&own, "2026-09-01T00-00-00-000Z_ses_earlier.jsonl");

    app.note_prior_history(&agent);
    let said = notices(&app);
    assert!(
        said.iter()
            .any(|l| l.contains("1 earlier session") && l.contains("epsilon -c")),
        "the notice names the count and the way back: {said:?}"
    );
}

#[test]
fn the_notice_is_plural_and_absent_when_there_is_nothing_earlier() {
    let (mut app, agent, _rx, _dir) = test_app_with_store();
    let own = agent.session_path().expect("attached");

    // Nothing else in the directory: silence. The launch's own file is not "earlier".
    app.note_prior_history(&agent);
    assert!(notices(&app).is_empty(), "{:?}", notices(&app));

    // Three earlier sessions: the plural, and still exactly one notice.
    for i in 0..3 {
        plant_earlier_session(
            &own,
            &format!("2026-09-0{i}T00-00-00-000Z_ses_old{i}.jsonl"),
        );
    }
    app.note_prior_history(&agent);
    let said = notices(&app);
    assert_eq!(said.len(), 1, "one notice, not one per session: {said:?}");
    assert!(said[0].contains("3 earlier sessions"), "{said:?}");
}

#[test]
fn a_launch_that_did_restore_the_history_says_nothing() {
    let (mut app, agent, _rx, _dir) = test_app_with_store();
    let own = agent.session_path().expect("attached");
    plant_earlier_session(&own, "2026-09-01T00-00-00-000Z_ses_earlier.jsonl");

    agent.restore_history(vec![epsilon_contracts::Message::User(
        epsilon_contracts::message::UserMessage {
            id: "msg_1".into(),
            timestamp_ms: 0,
            content: vec![epsilon_contracts::UserContent::Text {
                text: "hello".into(),
            }],
        },
    )]);
    assert!(!agent.history().is_empty());

    app.note_prior_history(&agent);
    assert!(
        notices(&app).is_empty(),
        "a resumed session has nothing to explain: {:?}",
        notices(&app)
    );
}
