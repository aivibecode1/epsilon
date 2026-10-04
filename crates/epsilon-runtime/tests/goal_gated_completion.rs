//! The end-to-end gated-goal test that has never existed: the **tool**, a **real** `Agent` as
//! the goal host, a **real** gate runner, and a **real** command.
//!
//! Every gate test until now has driven a `FakeHost` or called the runner directly.
//! `FakeHost` answers `current_goal` from a snapshot it was handed, so the two questions
//! that matter were never asked of the thing that actually holds the goal:
//!
//! * **does the gate the user declared reach the tool?** and
//! * **does a refused completion leave durable state unrecorded?**
//!
//! The second is the one a snapshot structurally cannot check. A refusal that still wrote
//! `complete` would be invisible to every other test in the crate and would leave a goal
//! marked done for ever.
//!
//! This lives in `epsilon-runtime` rather than beside the tool because it needs `epsilon-llm`,
//! `epsilon-session` and `epsilon-core` at once, and this is the crate whose whole job is
//! being where a workspace, a configuration and a runtime become a running agent.

use std::sync::{Arc, Mutex};

use epsilon_contracts::goal::{GoalGate, GoalOp, GoalStatus};
use epsilon_contracts::tool::{Tool, ToolContext, ToolFailure, ToolInvocation};
use epsilon_core::{Agent, AutoAllow, LoopLimits, ToolRegistry};
use epsilon_llm::{FauxProvider, FauxStep, PanicGuard};
use epsilon_session::SessionStore;
use epsilon_tools::goal::UpdateGoalTool;
use epsilon_tools::goal_gate::GoalGateRunner;

/// A real `Agent` over a real session store — the host the product hands the tool.
fn real_agent() -> (Arc<Agent>, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("a workspace");
    let store = SessionStore::open(dir.path().join("session.jsonl")).expect("a store opens");
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

/// A goal declaring `gate`, created through the same ops the model would use.
///
/// Returns its **real** id and revision. `"g1"` — which every `FakeHost` test hardcodes — is
/// a stand-in artefact: a real `Agent` generates its own id, so a test that keeps `"g1"` past
/// the point where the goal is real is testing the fake. That mistake cost this file one
/// confusing failure before it was found.
fn goal_with_gate(agent: &Arc<Agent>, gate: &str) -> (String, u64) {
    agent
        .goal_apply(
            None,
            GoalOp::Create {
                objective: "make the suite green".into(),
                max_rounds: Some(4),
            },
        )
        .expect("a goal is created")
        .expect("a fresh goal");
    agent
        .goal_apply(
            None,
            GoalOp::Edit {
                objective: None,
                max_rounds: None,
                gates: Some(vec![GoalGate::new(gate)]),
            },
        )
        .expect("the gate is recorded")
        .expect("the goal is still active");
    let view = agent.goal().expect("the goal is durable").goal;
    (view.id.clone(), view.revision)
}

/// Call the tool the way the agent loop does, against a real host.
///
/// `ToolContext` is built here rather than through a helper because **the host is the whole
/// point**: every other gate test hands the tool a snapshot of a goal, and a snapshot cannot
/// say whether a refusal was recorded.
async fn complete(
    agent: &Arc<Agent>,
    goal_id: &str,
    revision: u64,
) -> Result<epsilon_contracts::tool::ToolOutput, ToolFailure> {
    let tool = UpdateGoalTool::new().with_gate_runner(Arc::new(GoalGateRunner::new(
        None,
        Some(std::sync::Arc::new(epsilon_safety::AutoApprove)),
    )));
    let ctx = ToolContext {
        session_id: "t".into(),
        observation_scope: Arc::new(()),
        cwd: std::env::temp_dir(),
        spill_dir: std::env::temp_dir(),
        token: Default::default(),
        task_host: None,
        // The production adapter, not a stand-in: `Agent::clone_goal_host()` is exactly
        // what `epsilon-core`'s execution path puts in this slot, so a test that built the
        // host any other way would be testing a wiring nobody ships.
        goal_host: Some(agent.clone_goal_host()),
    };
    tool.execute(
        ToolInvocation {
            tool_call_id: "tc".into(),
            args: serde_json::json!({"action": "complete", "goal_id": goal_id, "revision": revision}),
        },
        ctx,
        Arc::new(|_| {}),
    )
    .await
}

#[tokio::test]
async fn a_real_agent_refuses_a_completion_its_own_gate_fails() {
    let (agent, _dir) = real_agent();
    let (goal_id, revision) = goal_with_gate(&agent, "cargo definitely-not-a-subcommand");
    assert_eq!(
        agent.goal().expect("a goal").goal.gates.len(),
        1,
        "precondition: the gate reached durable state through the real host"
    );

    let err = complete(&agent, &goal_id, revision)
        .await
        .expect_err("a failing gate must refuse the completion");
    assert!(
        err.message.contains("cargo definitely-not-a-subcommand"),
        "the refusal must name the gate the user actually declared, not one the test handed \
         the host: {}",
        err.message
    );

    // The part no `FakeHost` test can reach.
    let after = agent.goal().expect("the goal is still there").goal;
    assert_ne!(
        after.status,
        GoalStatus::Complete,
        "a refused completion must not have been recorded: {after:?}"
    );
    assert_eq!(
        after.gates.len(),
        1,
        "and the gate must survive the refusal, or the next completion runs unguarded"
    );
}

#[tokio::test]
async fn a_real_agent_records_the_completion_when_its_own_gate_passes() {
    // Without this half, the refusal above is satisfied by a tool that refuses everything.
    let (agent, _dir) = real_agent();
    let (goal_id, revision) = goal_with_gate(&agent, "cargo --version");

    complete(&agent, &goal_id, revision)
        .await
        .expect("a passing gate must let the completion through");
    assert_eq!(
        agent.goal().expect("still there").goal.status,
        GoalStatus::Complete,
        "a completion the gate allowed has to be recorded, or the gate is only a delay"
    );
    assert_eq!(
        agent.goal().expect("still there").goal.gates.len(),
        1,
        "and the gate must still be on the goal afterwards: completion is not a licence to \
         forget the rule"
    );
}
