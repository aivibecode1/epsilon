//! M253: a goal round consults the goal's gates, and a failing gate's output **is** the
//! round.
//!
//! This is the gap `audit/03-defects.md` A26 describes. `Agent::goal_begin_round` returned the
//! same "continue toward the objective" text every round and never looked at `goal.gates`, so
//! a goal with `cargo test` as its gate spent its round budget re-reading one sentence while
//! the suite stayed red. Hermes makes the failing gate's bounded output the continuation
//! prompt (`references/hermes-agent-2026.9.14/hermes_cli/goals.py:97-111`, `:1293-1345`).
//!
//! These drive the real `begin_goal_round` with a real runner against real commands, because
//! a test that exercised a stand-in would be a test of the stand-in — the trap
//! `agents/memory/goal-gates.md` records in its own closing paragraph.

use std::path::Path;
use std::sync::{Arc, Mutex};

use epsilon_contracts::goal::{GoalGate, GoalOp};
use epsilon_core::{Agent, AutoAllow, LoopLimits, ToolRegistry};
use epsilon_llm::{FauxProvider, FauxStep, PanicGuard};
use epsilon_runtime::{GoalRound, begin_goal_round};
use epsilon_session::SessionStore;
use epsilon_tools::goal_gate::GoalGateRunner;

/// An agent with a durable store, which is what a goal needs to exist at all.
fn agent_with_store() -> (Arc<Agent>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = SessionStore::open(dir.path().join("session.jsonl")).unwrap();
    let agent = Agent::new(
        LoopLimits::default(),
        Arc::new(PanicGuard(FauxProvider::new(Vec::<FauxStep>::new()))),
        Arc::new(ToolRegistry::new()),
        Arc::new(AutoAllow),
        dir.path().to_path_buf(),
    );
    agent.attach_session_store(Arc::new(Mutex::new(store)));
    (agent, dir)
}

/// A goal declaring `gates`, armed so it will continue.
///
/// **Returns the `TempDir` too, and every caller binds it — because dropping it deletes the
/// session directory out from under a live store (M312).** The comment this replaces said the
/// directory "must be created and used before it goes", which was not what the code did: `_dir`
/// was bound inside this function and dropped at `return`, so the agent was handed back holding
/// a store whose file *and whose `.lock`* had just been removed. Nothing noticed for the life of
/// the test, because nothing looked — and the day something looked, the five tests below failed
/// with a lease refusal that named the real cause. The fixture was wrong; the refusal was right.
fn gated_goal(gates: &[&str]) -> (Arc<Agent>, tempfile::TempDir) {
    let (agent, dir) = agent_with_store();
    agent
        .goal_apply(
            None,
            GoalOp::Create {
                objective: "make the suite green".into(),
                max_rounds: Some(4),
            },
        )
        .unwrap();
    agent
        .goal_apply(
            None,
            GoalOp::Edit {
                objective: None,
                max_rounds: None,
                gates: Some(gates.iter().map(|g| GoalGate::new(*g)).collect()),
            },
        )
        .unwrap();
    agent.set_goal_armed(true);
    (agent, dir)
}

#[tokio::test]
async fn a_goal_with_no_gates_continues_unchanged() {
    // The ordinary path must not change. A gate-aware round that altered the no-gate prompt
    // would silently change every existing goal's behaviour.
    let (agent, _dir) = gated_goal(&[]);
    let round = begin_goal_round(&agent, None, Path::new("."))
        .await
        .unwrap();
    match round {
        GoalRound::Continue {
            prompt,
            gates_passed,
        } => {
            assert_eq!(gates_passed, 0);
            assert!(prompt.contains("make the suite green"), "{prompt}");
            assert!(
                !prompt.contains("quality gate failed"),
                "a goal with no gates cannot be told a gate failed: {prompt}"
            );
        }
        other => panic!("expected an ordinary round, got {other:?}"),
    }
}

#[tokio::test]
async fn a_failing_gate_becomes_the_continuation_prompt() {
    let (agent, _dir) = gated_goal(&["cargo definitely-not-a-subcommand"]);
    let runner = GoalGateRunner::new(None, Some(std::sync::Arc::new(epsilon_safety::AutoApprove)));
    let round = begin_goal_round(&agent, Some(&runner), Path::new("."))
        .await
        .unwrap();

    let (prompt, reports) = match round {
        GoalRound::GateFailed { prompt, reports } => (prompt, reports),
        other => panic!("expected the gate to fail the round, got {other:?}"),
    };
    assert_eq!(reports.len(), 1, "one gate, one report");
    // The three things a model cannot work without: what to run, that it failed, and why.
    assert!(
        prompt.contains("cargo definitely-not-a-subcommand"),
        "the prompt must name the command: {prompt}"
    );
    assert!(
        prompt.contains("Exit code:"),
        "the prompt must carry the exit code: {prompt}"
    );
    assert!(
        prompt.contains("Exit code: 101") || prompt.contains("Exit code: 1"),
        "the code should be cargo's own: {prompt}"
    );
    // And the sentence that stops the model declaring victory anyway.
    assert!(
        prompt.contains("Do not declare the goal complete while any gate fails"),
        "a round that ends 'you are done' undoes the gate: {prompt}"
    );
    assert!(
        prompt.contains("say so clearly and stop"),
        "a goal whose gate can never pass has to be able to end: {prompt}"
    );
}

#[tokio::test]
async fn a_passing_gate_sends_the_ordinary_prompt() {
    let (agent, _dir) = gated_goal(&["cargo --version"]);
    let runner = GoalGateRunner::new(None, Some(std::sync::Arc::new(epsilon_safety::AutoApprove)));
    let round = begin_goal_round(&agent, Some(&runner), Path::new("."))
        .await
        .unwrap();
    match round {
        GoalRound::Continue {
            gates_passed,
            prompt,
        } => {
            assert_eq!(gates_passed, 1, "the gate ran and passed");
            assert!(
                !prompt.contains("quality gate failed"),
                "a passing gate must not turn the round into a failure: {prompt}"
            );
        }
        other => panic!("expected an ordinary round, got {other:?}"),
    }
}

#[tokio::test]
async fn a_gate_failure_still_costs_a_round() {
    // The accounting that stops a red suite being retried for free. Hermes says it in as
    // many words: a quality gate consumes "the same turn budget".
    let (agent, _dir) = gated_goal(&["cargo definitely-not-a-subcommand"]);
    let runner = GoalGateRunner::new(None, Some(std::sync::Arc::new(epsilon_safety::AutoApprove)));
    let before = agent.goal().unwrap().goal.rounds_started;
    begin_goal_round(&agent, Some(&runner), Path::new("."))
        .await
        .unwrap();
    let after = agent.goal().unwrap().goal.rounds_started;
    assert_eq!(after, before + 1, "a failing gate must spend a round");
}

#[tokio::test]
async fn a_goal_with_no_runner_continues_rather_than_pretending_to_have_checked() {
    // `None` means this platform resolved no sandbox. The round must still happen - refusing
    // to continue would stop a goal that never asked for a gate - and must not claim a gate
    // passed.
    let (agent, _dir) = gated_goal(&["cargo --version"]);
    let round = begin_goal_round(&agent, None, Path::new("."))
        .await
        .unwrap();
    match round {
        GoalRound::Continue { gates_passed, .. } => {
            assert_eq!(gates_passed, 0, "nothing ran, so nothing passed");
        }
        other => panic!("expected an ordinary round, got {other:?}"),
    }
}

#[tokio::test]
async fn an_exhausted_goal_stops_rather_than_spending_a_round() {
    let (agent, _dir) = agent_with_store();
    agent
        .goal_apply(
            None,
            GoalOp::Create {
                objective: "one round only".into(),
                max_rounds: Some(1),
            },
        )
        .unwrap();
    agent.set_goal_armed(true);
    assert!(matches!(
        begin_goal_round(&agent, None, Path::new("."))
            .await
            .unwrap(),
        GoalRound::Continue { .. }
    ));
    assert_eq!(
        begin_goal_round(&agent, None, Path::new("."))
            .await
            .unwrap(),
        GoalRound::Stop,
        "the cap is the only thing that stops a loop a human has stopped watching"
    );
}
