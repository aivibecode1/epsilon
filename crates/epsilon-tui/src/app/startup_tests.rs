//! M205: what the harness knows at startup and does not say.
//!
//! Ledger A17. A plain launch opens the session file, appends to it, and starts with an empty
//! in-memory history — so a returning user sees a first-run screen while the header reports a
//! few percent of the context already used. Both facts are true and together they read as a bug.
//!
//! The condition under test is "the store has entries **and** this process is not carrying
//! them", because that is exactly the state that looks wrong. A launch that did restore history
//! says nothing, and an empty session says nothing.

use super::*;
use crate::app::tests_support::test_app_with_store;

fn notices(app: &App) -> Vec<String> {
    app.ui.lines().iter().map(|l| l.text.clone()).collect()
}

#[test]
fn a_plain_launch_over_an_existing_session_says_so() {
    let (mut app, agent, _rx, _dir) = test_app_with_store();
    // One prior turn in the file that this process is not carrying.
    {
        let handle = agent.session_store().expect("attached");
        let mut guard = handle.lock().unwrap();
        guard
            .append(epsilon_contracts::EntryKind::SessionInfo {
                name: Some("an earlier conversation".into()),
            })
            .unwrap();
    }
    assert!(
        agent.history().is_empty(),
        "a plain launch restores nothing"
    );

    app.note_prior_history(&agent);
    let said = notices(&app);
    assert!(
        said.iter()
            .any(|l| l.contains("1 entry") && l.contains("epsilon -c")),
        "the notice names the count and the way back: {said:?}"
    );
}

#[test]
fn the_notice_is_plural_and_absent_when_there_is_nothing_to_say() {
    let (mut app, agent, _rx, _dir) = test_app_with_store();

    // Nothing in the file: silence. "0 entries" would be an announcement that nothing happened.
    app.note_prior_history(&agent);
    assert!(notices(&app).is_empty(), "{:?}", notices(&app));

    // Several entries: the plural, and still one notice.
    {
        let handle = agent.session_store().expect("attached");
        let mut guard = handle.lock().unwrap();
        for i in 0..3 {
            guard
                .append(epsilon_contracts::EntryKind::SessionInfo {
                    name: Some(format!("turn {i}")),
                })
                .unwrap();
        }
    }
    app.note_prior_history(&agent);
    let said = notices(&app);
    assert_eq!(said.len(), 1, "one notice, not one per entry: {said:?}");
    assert!(said[0].contains("3 entries"), "{said:?}");
}

#[test]
fn a_launch_that_did_restore_the_history_says_nothing() {
    // The condition has two halves and this is the second: if the run *is* carrying the history,
    // the screen matches the file and there is nothing to explain.
    let (mut app, agent, _rx, _dir) = test_app_with_store();
    {
        let handle = agent.session_store().expect("attached");
        let mut guard = handle.lock().unwrap();
        guard
            .append(epsilon_contracts::EntryKind::SessionInfo {
                name: Some("a prior turn".into()),
            })
            .unwrap();
    }
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
