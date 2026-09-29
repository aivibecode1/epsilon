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

#[tokio::test]
async fn goal_creates_records_and_renders_the_badge() {
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
    assert!(screen.contains("● goal 1/256"), "{screen}");
}

#[tokio::test]
async fn the_badge_is_emitted_whole_or_not_at_all() {
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

#[tokio::test]
async fn the_verbs_move_the_goal_through_its_phases() {
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

#[tokio::test]
async fn an_objective_that_starts_with_a_verb_is_still_an_objective() {
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

#[tokio::test]
async fn every_refusal_is_visible_and_says_what_to_do() {
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

#[tokio::test]
async fn bare_goal_reports_the_state_or_says_there_is_none() {
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
    assert!(notice.contains("rounds 1/256"), "{notice}");
    assert!(notice.contains("continuation armed"), "{notice}");
}

#[tokio::test]
async fn a_session_with_no_store_refuses_rather_than_pretending() {
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

// ---- the round driver (M204) ------------------------------------------------------

#[tokio::test]
async fn creating_a_goal_from_the_keyboard_starts_the_first_round() {
    // `/goal fix the gates` that parked an objective and then sat there until the user typed
    // something else would be a note, not a goal. The first round starts on the command, and it
    // is counted and announced like every other round.
    let (mut app, agent, _rx, _dir) = test_app_with_store();
    app.run_command(
        &agent,
        crate::picker::Command::Goal(Some("fix every failing gate".into())),
    );
    assert_eq!(
        agent.goal().unwrap().goal.rounds_started,
        1,
        "creating a goal spends its first round immediately"
    );
    // Pausing parks it, and an edit is not work.
    app.run_command(&agent, crate::picker::Command::Goal(Some("pause".into())));
    let before = agent.goal().unwrap().goal.rounds_started;
    app.run_command(
        &agent,
        crate::picker::Command::Goal(Some("edit a sharper objective".into())),
    );
    assert_eq!(
        agent.goal().unwrap().goal.rounds_started,
        before,
        "an edit does not start work"
    );
}

#[tokio::test]
async fn a_turn_end_starts_the_next_round_and_counts_it_durably() {
    let (mut app, agent, _rx, _dir) = test_app_with_store();
    app.run_command(
        &agent,
        crate::picker::Command::Goal(Some("keep the gates green".into())),
    );

    // The create spent round 1; the turn end spends round 2, durably and before the run starts —
    // a round that crashes on the way is still spent.
    assert!(app.continue_goal(&agent), "an armed goal continues");
    assert_eq!(agent.goal().unwrap().goal.rounds_started, 2);
    // And the round is visible: the prompt is echoed into the transcript like any other turn,
    // and a notice says why a turn the user did not type has started.
    let screen = screen(&mut app, 120, 30);
    assert!(screen.contains("Continuing the session goal"), "{screen}");
    assert!(screen.contains("goal round 2/256"), "{screen}");
    // The continuation is not in the recall ring: Up-arrow is for what *you* typed.
    assert!(
        !app.history
            .iter()
            .any(|h| h.contains("Continuing the session goal")),
        "a harness-authored round must not be offered as a recalled prompt"
    );
}

#[tokio::test]
async fn a_disarmed_or_finished_goal_does_not_continue_itself() {
    let (mut app, agent, _rx, _dir) = test_app_with_store();
    app.run_command(
        &agent,
        crate::picker::Command::Goal(Some("objective".into())),
    );

    // The create spent round 1; everything below is about what does *not* spend another.
    let spent = agent.goal().unwrap().goal.rounds_started;
    assert_eq!(spent, 1, "creating a goal starts its first round");

    agent.set_goal_armed(false);
    assert!(
        !app.continue_goal(&agent),
        "a fresh process has not been granted continuation"
    );
    assert_eq!(agent.goal().unwrap().goal.rounds_started, spent);

    app.run_command(&agent, crate::picker::Command::Goal(Some("resume".into())));
    app.run_command(&agent, crate::picker::Command::Goal(Some("pause".into())));
    assert!(
        !app.continue_goal(&agent),
        "a paused goal does not continue"
    );

    app.run_command(&agent, crate::picker::Command::Goal(Some("resume".into())));
    app.run_command(
        &agent,
        crate::picker::Command::Goal(Some("complete".into())),
    );
    assert!(
        !app.continue_goal(&agent),
        "a finished goal does not continue"
    );
    assert_eq!(
        agent.goal().unwrap().goal.rounds_started,
        spent + 2,
        "the two explicit resumes each started one round; the refusals spent nothing"
    );
}

#[tokio::test]
async fn a_draft_in_the_composer_holds_the_next_round_back() {
    // The user is halfway through typing; another round would race their Enter key. The goal
    // stays armed, so their own turn continues it.
    let (mut app, agent, _rx, _dir) = test_app_with_store();
    app.run_command(
        &agent,
        crate::picker::Command::Goal(Some("objective".into())),
    );
    app.input = "actually, stop and look at this first".into();
    assert!(
        !app.continue_goal(&agent),
        "a typed draft is a person mid-sentence"
    );
    assert!(
        agent.goal().unwrap().is_continuable(),
        "still armed, just held"
    );

    app.input.clear();
    assert!(
        app.continue_goal(&agent),
        "an empty composer lets it through"
    );
}

#[tokio::test]
async fn the_cap_stops_the_loop_and_says_which_command_raises_it() {
    let (mut app, agent, _rx, _dir) = test_app_with_store();
    agent
        .goal_apply(
            None,
            epsilon_contracts::GoalOp::Create {
                objective: "two rounds".into(),
                max_rounds: Some(2),
            },
        )
        .unwrap();
    assert!(app.continue_goal(&agent));
    assert!(app.continue_goal(&agent));
    assert!(!app.continue_goal(&agent), "the third round is refused");
    let notice = last_notice(&app);
    assert!(notice.contains("cap reached (2/2)"), "{notice}");
    assert!(
        notice.contains("/goal edit rounds"),
        "a refusal has to name the way out: {notice}"
    );
    assert_eq!(agent.goal().unwrap().goal.rounds_started, 2, "exactly two");
}

#[tokio::test]
async fn aborting_a_run_disarms_the_goal_without_touching_it() {
    // Esc is a person saying stop. The durable goal survives; the permission to keep going
    // unattended does not, which is the difference between `/goal resume` and a fight.
    let (mut app, agent, mut rx, _dir) = test_app_with_store();
    agent
        .goal_apply(
            None,
            epsilon_contracts::GoalOp::Create {
                objective: "objective".into(),
                max_rounds: None,
            },
        )
        .unwrap();
    assert!(agent.goal_armed());

    // Driven through the real key handler, because the disarm lives in a branch of it: the rule
    // is only a rule if the key that reaches it does. The Esc branch needs a run in flight, and
    // `running` is the UI's view of that — set here the way the event loop sets it, rather than
    // by starting a real run.
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    app.ui.apply(&epsilon_contracts::AgentEvent::AgentStart);
    assert!(app.ui.running());
    let quit = crate::app::keys::handle_key(
        &mut app,
        &agent,
        KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
    );
    assert!(!quit, "Esc during a run stops the run, it does not quit");

    assert!(!agent.goal_armed(), "the abort disarmed it");
    let view = agent.goal().expect("the goal itself is untouched");
    assert_eq!(view.goal.objective, "objective");
    assert_eq!(view.goal.status, epsilon_contracts::GoalStatus::Active);
    assert_eq!(view.goal.rounds_started, 0, "no round was spent");
    // And the user is told, rather than left to notice that the badge changed. The key handler
    // posts through the notice channel (it must not touch the UI from a key press), so the
    // assertion is on what the loop would drain.
    let mut said = Vec::new();
    while let Ok(line) = rx.1.try_recv() {
        said.push(line);
    }
    assert!(
        said.iter()
            .any(|l| l.contains("goal disarmed by the abort")),
        "the disarm must be visible: {said:?}"
    );
    // A disarmed goal does not continue, which is the whole point of disarming it.
    assert!(!app.continue_goal(&agent));
}
