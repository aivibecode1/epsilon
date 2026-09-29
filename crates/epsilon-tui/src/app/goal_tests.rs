//! M203: `/goal` from the user's side — what the command does, and what the frame shows.
//!
//! Two things are asserted here that nothing else can: that the command reaches the durable
//! state (rather than printing something plausible), and that the footer *renders* it. The
//! second is the repository's own rule — a feature can be correct at every level you test and
//! still not exist on screen — and a badge is exactly the kind of thing that passes a state
//! assertion while never being drawn.

use super::*;
use crate::app::tests_support::{render_rows, test_app_with_store};

fn screen(app: &mut App, width: u16, height: u16) -> String {
    render_rows(app, width, height).join("\n")
}

/// The last notice pushed, which is what a slash command answers with.
fn last_notice(app: &App) -> String {
    app.ui
        .lines()
        .iter()
        .rev()
        .find(|l| matches!(l.kind, crate::state::LineKind::System))
        .map(|l| l.text.clone())
        .unwrap_or_default()
}

#[test]
fn goal_creates_records_and_renders_the_badge() {
    let (mut app, agent, _rx, _dir) = test_app_with_store();
    app.run_command(
        &agent,
        crate::picker::Command::Goal(Some("ship the migration".into())),
    );
    // Durable, not merely displayed: the state is read back from the store.
    let view = agent.goal().expect("the command created a goal");
    assert_eq!(view.goal.objective, "ship the migration");
    assert!(view.armed, "creating one from the keyboard arms it");

    // And the footer shows it, on a real frame.
    app.refresh_goal(&agent);
    let screen = screen(&mut app, 100, 20);
    assert!(screen.contains("● goal 0/256"), "{screen}");
}

#[test]
fn the_badge_is_emitted_whole_or_not_at_all() {
    // The M201 rule, and the reason it matters more here than anywhere: `● goal 2/256` cut to
    // `● goal 2/2` is a goal one round from its cap rather than one round into a long one.
    let (mut app, agent, _rx, _dir) = test_app_with_store();
    agent
        .goal_apply(
            None,
            epsilon_contracts::GoalOp::Create {
                objective: "keep the doc gates green".into(),
                max_rounds: Some(256),
            },
        )
        .unwrap();
    app.refresh_goal(&agent);
    for width in 40u16..=120 {
        let screen = screen(&mut app, width, 12);
        for row in screen.lines() {
            // Every rendering of the badge on this row must be either the whole thing or
            // nothing: `● goal 2/2` (a prefix) is the failure this refuses.
            if let Some(at) = row.find("goal") {
                let tail = &row[at..];
                let digits: String = tail
                    .chars()
                    .skip_while(|c| !c.is_ascii_digit())
                    .take_while(|c| c.is_ascii_digit() || *c == '/')
                    .collect();
                if !digits.is_empty() {
                    assert_eq!(digits, "0/256", "width {width}: the badge was cut: {row:?}");
                }
            }
        }
    }
}

#[test]
fn the_verbs_move_the_goal_through_its_phases() {
    use crate::picker::Command;
    let (mut app, agent, _rx, _dir) = test_app_with_store();
    let run = |app: &mut App, agent: &Arc<Agent>, arg: &str| {
        app.run_command(agent, Command::Goal(Some(arg.to_string())));
    };

    run(&mut app, &agent, "one objective");
    run(&mut app, &agent, "pause");
    assert_eq!(
        agent.goal().unwrap().goal.status,
        epsilon_contracts::GoalStatus::Paused
    );
    run(&mut app, &agent, "resume");
    assert!(agent.goal().unwrap().armed, "resume arms it again");
    run(&mut app, &agent, "edit a sharper objective");
    assert_eq!(agent.goal().unwrap().goal.objective, "a sharper objective");
    run(&mut app, &agent, "edit rounds 8");
    assert_eq!(agent.goal().unwrap().goal.max_rounds, 8);
    run(&mut app, &agent, "complete");
    assert_eq!(
        agent.goal().unwrap().goal.status,
        epsilon_contracts::GoalStatus::Complete
    );
    // A completed goal can be replaced without an explicit clear, which is what stops `/goal`
    // being a command a user has to undo before it will work again.
    run(&mut app, &agent, "the next objective");
    assert_eq!(agent.goal().unwrap().goal.objective, "the next objective");
    run(&mut app, &agent, "clear");
    assert!(agent.goal().is_none());
    assert!(!last_notice(&app).is_empty(), "a clear says so");
}

#[test]
fn an_objective_that_starts_with_a_verb_is_still_an_objective() {
    // The parser rule, asserted because it is the one a naive `match` gets wrong: only a verb
    // that is the *whole* argument is a verb, so this creates a goal rather than pausing one.
    let (mut app, agent, _rx, _dir) = test_app_with_store();
    app.run_command(
        &agent,
        crate::picker::Command::Goal(Some("clear the failing gates and pause nothing".into())),
    );
    let view = agent.goal().expect("a goal was created");
    assert_eq!(
        view.goal.objective,
        "clear the failing gates and pause nothing"
    );
    assert_eq!(view.goal.status, epsilon_contracts::GoalStatus::Active);
}

#[test]
fn every_refusal_is_visible_and_says_what_to_do() {
    let (mut app, agent, _rx, _dir) = test_app_with_store();
    // Nothing to resume.
    app.run_command(&agent, crate::picker::Command::Goal(Some("resume".into())));
    let notice = last_notice(&app);
    assert!(notice.contains("refused"), "{notice}");
    assert!(notice.contains("no goal"), "{notice}");

    // A create while one is unfinished.
    app.run_command(&agent, crate::picker::Command::Goal(Some("first".into())));
    app.run_command(&agent, crate::picker::Command::Goal(Some("second".into())));
    let notice = last_notice(&app);
    assert!(notice.contains("refused"), "{notice}");
    assert!(notice.contains("already current"), "{notice}");

    // And a bad argument shape, which must not be silently treated as an objective.
    app.run_command(
        &agent,
        crate::picker::Command::Goal(Some("edit rounds zero".into())),
    );
    let notice = last_notice(&app);
    assert!(notice.contains("positive number"), "{notice}");
}

#[test]
fn bare_goal_reports_the_state_or_says_there_is_none() {
    let (mut app, agent, _rx, _dir) = test_app_with_store();
    app.run_command(&agent, crate::picker::Command::Goal(None));
    assert!(
        last_notice(&app).contains("no goal is set"),
        "{}",
        last_notice(&app)
    );

    app.run_command(
        &agent,
        crate::picker::Command::Goal(Some("do the thing".into())),
    );
    app.run_command(&agent, crate::picker::Command::Goal(None));
    let notice = last_notice(&app);
    assert!(notice.contains("goal active"), "{notice}");
    assert!(notice.contains("rounds 0/256"), "{notice}");
    assert!(notice.contains("continuation armed"), "{notice}");
}

#[test]
fn a_session_with_no_store_refuses_rather_than_pretending() {
    // `test_app` deliberately has no durable mirror. The wrong answer here is a friendly "no
    // goal is set", which would tell the user the session has no objective when the truth is
    // that this session cannot record one.
    let (mut app, agent, _rx) = crate::app::tests_support::test_app();
    app.run_command(
        &agent,
        crate::picker::Command::Goal(Some("something".into())),
    );
    let notice = last_notice(&app);
    assert!(notice.contains("refused"), "{notice}");
    assert!(!notice.contains("no goal is set"), "{notice}");
}
