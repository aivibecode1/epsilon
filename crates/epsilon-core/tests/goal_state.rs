//! M203: the session goal's lifecycle, asserted through the real `Agent`.
//!
//! The point of these tests is the *rules*, not the plumbing: one current goal, a
//! compare-and-set that refuses a stale writer, arming that follows the phase, and a clear that
//! leaves the history behind. Each one is a rule DSH states and a naive implementation gets
//! wrong in a way that only shows up after a restart.

use std::sync::{Arc, Mutex};

use epsilon_contracts::goal::{
    DEFAULT_MAX_GOAL_ROUNDS, GoalBlocker, GoalError, GoalOp, GoalRef, GoalStatus,
};
use epsilon_core::{AutoAllow, LoopLimits, ToolRegistry};
use epsilon_llm::{FauxProvider, FauxStep};
use epsilon_session::SessionStore;

/// An agent with a durable store in a temporary directory.
///
/// The store is the whole point of the fixture: a goal lives in the session log and nowhere
/// else, so an agent without one has no goal to test.
fn agent_with_store() -> (Arc<epsilon_core::Agent>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = SessionStore::open(dir.path().join("session.jsonl")).unwrap();
    let agent = epsilon_core::Agent::new(
        LoopLimits::default(),
        Arc::new(epsilon_llm::PanicGuard(FauxProvider::new(
            Vec::<FauxStep>::new(),
        ))),
        Arc::new(ToolRegistry::new()),
        Arc::new(AutoAllow),
        dir.path().to_path_buf(),
    );
    agent.attach_session_store(Arc::new(Mutex::new(store)));
    (agent, dir)
}

fn create(agent: &Arc<epsilon_core::Agent>, objective: &str) -> epsilon_contracts::GoalView {
    agent
        .goal_apply(
            None,
            GoalOp::Create {
                objective: objective.into(),
                max_rounds: None,
            },
        )
        .expect("a create on an empty session is accepted")
        .expect("a create leaves a goal")
}

#[test]
fn a_goal_survives_a_reopen_and_keeps_its_revision() {
    // Durable, and durable *from the log*: the second agent opens the same file with no state
    // handed over, which is what a restart actually does.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.jsonl");
    let objective = "make every documentation gate pass";
    let (id, revision) = {
        let store = SessionStore::open(&path).unwrap();
        let agent = epsilon_core::Agent::new(
            LoopLimits::default(),
            Arc::new(epsilon_llm::PanicGuard(FauxProvider::new(
                Vec::<FauxStep>::new(),
            ))),
            Arc::new(ToolRegistry::new()),
            Arc::new(AutoAllow),
            dir.path().to_path_buf(),
        );
        agent.attach_session_store(Arc::new(Mutex::new(store)));
        let view = create(&agent, objective);
        (view.goal.id, view.goal.revision)
    };

    let reopened = SessionStore::open(&path).unwrap();
    let goal = reopened.goal().unwrap().expect("the goal is in the file");
    assert_eq!(goal.id, id);
    assert_eq!(goal.revision, revision);
    assert_eq!(goal.objective, objective);
    assert_eq!(goal.status, GoalStatus::Active);
    assert_eq!(goal.max_rounds, DEFAULT_MAX_GOAL_ROUNDS);
}

#[test]
fn a_fresh_process_does_not_inherit_the_permission_to_continue() {
    // The rule that makes an unattended loop safe to leave running: the goal is durable, the
    // permission to keep working on it is not. A restart has to be told again.
    let (agent, _dir) = agent_with_store();
    create(&agent, "keep the port green");
    assert!(
        agent.goal_armed(),
        "creating a goal arms it in this process"
    );

    let (fresh, _dir2) = agent_with_store();
    assert!(
        fresh.goal().is_none(),
        "the second agent has its own (empty) store"
    );
    // A goal that is active in the *log* of a process that has not asked for it: the state is
    // readable and continuable in principle, and this process may not act on it.
    //
    // Written through the agent's own store handle rather than by reopening the file: a session
    // file has one writer at a time and the lease refuses a second open, which is the product
    // being right and the first version of this test being wrong.
    fresh
        .session_store()
        .expect("attached")
        .lock()
        .unwrap()
        .append_goal(epsilon_contracts::GoalChange::Set {
            goal: epsilon_contracts::GoalSnapshot {
                id: "g".into(),
                revision: 1,
                objective: "resumed objective".into(),
                status: GoalStatus::Active,
                max_rounds: 4,
                rounds_started: 0,
                blocker: None,
                created_at_ms: 0,
                updated_at_ms: 0,
            },
        })
        .unwrap();
    let view = fresh.goal().expect("the durable goal is readable");
    assert!(view.goal.status.is_continuable());
    assert!(!view.armed, "nothing in this process asked to continue it");
    assert!(
        !view.is_continuable(),
        "active and durable is not the same as armed and continuable"
    );
}

#[test]
fn the_lifecycle_arms_and_disarms_with_the_phase() {
    let (agent, _dir) = agent_with_store();
    let view = create(&agent, "ship the migration");
    assert!(view.armed && view.goal.status == GoalStatus::Active);

    let paused = agent
        .goal_apply(None, GoalOp::Pause)
        .unwrap()
        .expect("pause keeps the goal");
    assert_eq!(paused.goal.status, GoalStatus::Paused);
    assert!(!paused.armed, "pausing stops continuation");

    let resumed = agent
        .goal_apply(None, GoalOp::Resume)
        .unwrap()
        .expect("resume keeps the goal");
    assert_eq!(resumed.goal.status, GoalStatus::Active);
    assert!(resumed.armed, "resuming starts it again");
    assert!(
        resumed.goal.revision > paused.goal.revision,
        "every accepted mutation moves the revision, so a stale ref cannot match"
    );

    let done = agent
        .goal_apply(None, GoalOp::Complete)
        .unwrap()
        .expect("complete keeps the record");
    assert_eq!(done.goal.status, GoalStatus::Complete);
    assert!(!done.armed, "a finished goal is never armed");
}

#[test]
fn a_second_goal_needs_the_first_one_finished() {
    let (agent, _dir) = agent_with_store();
    let first = create(&agent, "first objective");

    let refused = agent
        .goal_apply(
            None,
            GoalOp::Create {
                objective: "second objective".into(),
                max_rounds: None,
            },
        )
        .unwrap_err();
    assert!(
        matches!(refused, GoalError::AlreadyCurrent { .. }),
        "an unfinished goal holds the slot: {refused:?}"
    );

    // Complete it and the slot opens — with a NEW identity, so a stale ref to the finished goal
    // can never be mistaken for the new one.
    agent.goal_apply(None, GoalOp::Complete).unwrap();
    let second = agent
        .goal_apply(
            None,
            GoalOp::Create {
                objective: "second objective".into(),
                max_rounds: Some(3),
            },
        )
        .unwrap()
        .expect("a completed goal can be replaced");
    assert_ne!(second.goal.id, first.goal.id, "a replacement is a new goal");
    assert_eq!(second.goal.revision, 1, "the new goal starts its own count");
    assert_eq!(second.goal.max_rounds, 3);
}

#[test]
fn a_stale_writer_is_refused_rather_than_obeyed() {
    // The whole reason the revision exists: a tool call that read the goal two turns ago must
    // not silently overwrite what the user changed in between.
    let (agent, _dir) = agent_with_store();
    let created = create(&agent, "objective");
    let stale = GoalRef {
        id: created.goal.id.clone(),
        revision: created.goal.revision,
    };
    agent.goal_apply(None, GoalOp::Pause).unwrap();

    let refused = agent.goal_apply(Some(stale), GoalOp::Resume).unwrap_err();
    match refused {
        GoalError::Stale { expected, actual } => {
            assert_eq!(expected.revision, 1);
            assert_eq!(actual.revision, 2);
        }
        other => panic!("expected a stale refusal, got {other:?}"),
    }
    // And the current ref still works, which is what makes the refusal recoverable.
    let current = agent.goal().unwrap().goal.as_ref();
    assert!(agent.goal_apply(Some(current), GoalOp::Resume).is_ok());
}

#[test]
fn clearing_leaves_the_history_and_a_late_tombstone_cannot_remove_a_newer_goal() {
    let (path_dir, path) = {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("session.jsonl");
        (dir, p)
    };
    let store = SessionStore::open(&path).unwrap();
    let agent = epsilon_core::Agent::new(
        LoopLimits::default(),
        Arc::new(epsilon_llm::PanicGuard(FauxProvider::new(
            Vec::<FauxStep>::new(),
        ))),
        Arc::new(ToolRegistry::new()),
        Arc::new(AutoAllow),
        path_dir.path().to_path_buf(),
    );
    agent.attach_session_store(Arc::new(Mutex::new(store)));
    let first = create(&agent, "the first objective");
    assert!(
        agent.goal_apply(None, GoalOp::Clear).unwrap().is_none(),
        "clear reports that there is no goal now"
    );
    assert!(agent.goal().is_none());
    assert!(!agent.goal_armed(), "clearing disarms");

    let second = create(&agent, "the second objective");
    assert_ne!(second.goal.id, first.goal.id);

    // A tombstone aimed at the *first* goal, written after the second exists. It must not
    // remove the second: the log's own story is the only thing that decides this. Written
    // through the attached store, because the lease refuses a second open of one session file.
    {
        let handle = agent.session_store().expect("attached");
        let mut guard = handle.lock().unwrap();
        guard
            .append_goal(epsilon_contracts::GoalChange::Cleared {
                id: first.goal.id.clone(),
                revision: 99,
            })
            .unwrap();
        let now = guard.goal().unwrap().expect("the newer goal survives");
        assert_eq!(now.id, second.goal.id);
    }
}

#[test]
fn the_wire_payload_carries_the_goal_and_its_activation() {
    // M206. A client cannot see the session's screen, so the goal reaches it through the one
    // payload both transports already share. Two facts have to survive the trip, and the second is
    // the one a client would otherwise get wrong: an `active` goal in a process that has not asked
    // to continue it is **disarmed**, so `status` alone would show a goal that has stopped.
    let (agent, _dir) = agent_with_store();
    assert!(
        agent.session_state().goal.is_none(),
        "no goal, and `null` is how a client tells that from an empty one"
    );

    let view = create(&agent, "make the wire carry this");
    let goal = agent.session_state().goal.expect("the goal is on the wire");
    assert_eq!(goal.objective, "make the wire carry this");
    assert_eq!(goal.id, view.goal.id);
    assert_eq!(goal.revision, view.goal.revision);
    assert_eq!(goal.rounds_started, view.goal.rounds_started);
    assert_eq!(goal.max_rounds, view.goal.max_rounds);
    assert_eq!(goal.status, view.goal.status);
    assert!(goal.armed, "this process created it, so it is armed");
    assert_eq!(goal.armed, view.armed, "the payload and the view agree");

    // A fresh process over the same file: the phase is the same, the activation is not.
    agent.set_goal_armed(false);
    let disarmed = agent.session_state().goal.expect("still on the wire");
    assert_eq!(disarmed.status, epsilon_contracts::GoalStatus::Active);
    assert!(
        !disarmed.armed,
        "the difference between `active` and `will continue`, which is this field's whole job"
    );
}

#[test]
fn a_blocked_goal_carries_its_reason_and_resume_clears_it() {
    let (agent, _dir) = agent_with_store();
    create(&agent, "wait for the credential");
    let blocked = agent
        .goal_apply(
            None,
            GoalOp::Blocked {
                blocker: GoalBlocker {
                    code: "needs_human".into(),
                    reason: "the API key is not configured".into(),
                },
            },
        )
        .unwrap()
        .expect("blocking keeps the record");
    assert_eq!(blocked.goal.status, GoalStatus::Blocked);
    assert!(!blocked.armed);
    assert_eq!(
        blocked.goal.blocker.as_ref().map(|b| b.code.as_str()),
        Some("needs_human")
    );

    let resumed = agent
        .goal_apply(None, GoalOp::Resume)
        .unwrap()
        .expect("resume keeps the goal");
    assert!(
        resumed.goal.blocker.is_none(),
        "an active goal carrying a blocker is a reason nobody can act on"
    );
}

#[test]
fn a_round_is_counted_and_the_cap_stops_it() {
    // Rounds are the only budget there is, so the counting has to be exact: one per attempt,
    // durably, and the cap refuses a resume rather than firing the guard again.
    let (agent, _dir) = agent_with_store();
    agent
        .goal_apply(
            None,
            GoalOp::Create {
                objective: "two rounds only".into(),
                max_rounds: Some(2),
            },
        )
        .unwrap();

    let first = agent
        .goal_begin_round()
        .unwrap()
        .expect("round 1 is allowed");
    assert!(first.contains("round 1/2"), "{first}");
    assert!(first.contains("two rounds only"), "{first}");
    let second = agent
        .goal_begin_round()
        .unwrap()
        .expect("round 2 is allowed");
    assert!(second.contains("round 2/2"), "{second}");
    assert!(
        agent.goal_begin_round().unwrap().is_none(),
        "the cap stops the third round"
    );

    let refused = agent.goal_apply(None, GoalOp::Resume).unwrap_err();
    assert!(
        matches!(refused, GoalError::RoundsExhausted { .. }),
        "rearming an exhausted goal only fires the guard again: {refused:?}"
    );
    // Raising the cap is the way out, which is what the refusal's own message says.
    agent
        .goal_apply(
            None,
            GoalOp::Edit {
                objective: None,
                max_rounds: Some(4),
            },
        )
        .unwrap();
    assert!(agent.goal_apply(None, GoalOp::Resume).is_ok());
}

#[test]
fn an_edit_can_raise_the_objective_or_the_cap_and_refuses_to_blank_either() {
    let (agent, _dir) = agent_with_store();
    create(&agent, "original objective");

    let edited = agent
        .goal_apply(
            None,
            GoalOp::Edit {
                objective: Some("sharpened objective".into()),
                max_rounds: None,
            },
        )
        .unwrap()
        .unwrap();
    assert_eq!(edited.goal.objective, "sharpened objective");

    assert!(
        agent
            .goal_apply(
                None,
                GoalOp::Edit {
                    objective: Some("   ".into()),
                    max_rounds: None
                }
            )
            .is_err(),
        "an edit cannot blank the objective"
    );
    assert!(
        agent
            .goal_apply(
                None,
                GoalOp::Edit {
                    objective: None,
                    max_rounds: None
                }
            )
            .is_err(),
        "an edit that changes nothing is a mistake, not a no-op"
    );
}

#[test]
fn a_session_with_no_goal_says_so_instead_of_inventing_one() {
    let (agent, _dir) = agent_with_store();
    assert!(agent.goal().is_none());
    assert!(matches!(
        agent.goal_apply(None, GoalOp::Pause).unwrap_err(),
        GoalError::NoGoal
    ));
    assert!(agent.goal_begin_round().unwrap().is_none());
}
