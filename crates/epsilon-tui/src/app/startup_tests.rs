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

/// A session from **before** the per-working-copy directory: written straight into the sessions
/// root, the way every session written up to M217 was.
///
/// Its header still names this workspace, which is what keeps it this workspace's session — and
/// what makes `epsilon -c` able to open it. Nothing here is unusual: it is a file that already
/// exists on any machine that ran Epsilon before M217.
fn plant_legacy_session(workspace: &std::path::Path, name: &str) {
    let root = workspace.join(".epsilon").join("sessions");
    std::fs::create_dir_all(&root).expect("the sessions root");
    let mut other = epsilon_session::SessionStore::open_in(root.join(name), workspace)
        .expect("a legacy session opens");
    other
        .append(epsilon_contracts::EntryKind::SessionInfo {
            name: Some("a conversation from before the directory".into()),
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

/// M222, and the whole point of this test: a session from **before** the per-working-copy
/// directory is still this workspace's, `epsilon -c` still opens it — so the notice must count it.
///
/// The notice listed the directory its own file sits in, so since M217 it saw the workspace's own
/// sessions and none of the flat root: a user upgrading their machine got **silence** while the
/// command it names had something to open. That is the reader/writer asymmetry M217 removed on the
/// other side of the same week, and it was invisible here for a second reason — the fixture planted
/// its session outside any sessions tree, so no layout rule in this file had ever run.
#[test]
fn a_legacy_session_in_the_flat_root_is_still_an_earlier_session() {
    let (mut app, agent, _rx, dir) = test_app_with_store();
    let workspace = dir.path().to_path_buf();
    // Nothing in the per-working-copy directory: only the pre-M217 file exists.
    plant_legacy_session(&workspace, "2026-08-01T00-00-00-000Z_ses_legacy.jsonl");

    app.note_prior_history(&agent);
    let said = notices(&app);
    assert!(
        said.iter().any(|l| l.contains("1 earlier session")),
        "the notice names a session `epsilon -c` can open: {said:?}"
    );

    // And the claim it makes is true: `-c`'s own resolver finds that file. Asserted here rather
    // than in the CLI because the notice's job is to agree with it.
    let root = workspace.join(".epsilon").join("sessions");
    assert_eq!(
        epsilon_session::list_sessions_for_cwd(&root, &workspace).len(),
        2,
        "the launch's own file and the legacy one: `-c` sees both, so the notice must too"
    );
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
