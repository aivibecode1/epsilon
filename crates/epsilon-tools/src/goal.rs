//! The goal tools: one long-running objective, readable and mutable by the model.
//!
//! Ported from DSH's `@deepseek-ai/dsh-tool-goal` (`create_goal`, `get_goal`, `update_goal`;
//! `docs/tool-catalog.md`). Three tools rather than one `goal` tool with an `action` field, and
//! the reason is the same one DSH gives: the *authority* differs per verb. Reading a goal is
//! safe at any time; creating, editing, pausing and resuming one rewrite what the session is
//! for and are things the **human** asked for; completing or blocking it is a report about work
//! the model just did. One tool cannot state three contracts, and a model that learns "call
//! `goal`" learns nothing about when it may.
//!
//! Two rules that are enforced here rather than merely described, because a description is not
//! enforcement:
//!
//! * **Every update carries `{goal_id, revision}`** and is refused when stale — DSH's
//!   compare-and-set. A model holding a goal it read ten tool calls ago cannot overwrite a
//!   change the user made in between.
//! * **`blocked` is refused before [`BLOCKED_MIN_ROUNDS`] rounds.** "Blocked" that a model can
//!   declare on its first obstacle is a tool for giving up; DSH's floor means the same condition
//!   has to survive several attempts before it counts as a blocker.

use async_trait::async_trait;
use epsilon_contracts::goal::{
    BLOCKED_MIN_ROUNDS, DEFAULT_MAX_GOAL_ROUNDS, GoalError, GoalGate, GoalOp, GoalRef, GoalView,
};
use epsilon_contracts::tool::{
    ExecutionMode, GoalHost, Tool, ToolContext, ToolDescriptor, ToolFailure, ToolInvocation,
    ToolOutput, ToolUpdateSink,
};
use schemars::JsonSchema;
use serde::Deserialize;
use std::sync::Arc;

pub use descriptions::{CREATE_GOAL_DESC, GET_GOAL_DESC, UPDATE_GOAL_DESC};

mod descriptions {
    //! Descriptions live in `.txt` files (ADR-004) so the prompt text is reviewable as prose
    //! rather than as a Rust string literal with escapes in it.
    /// Description for `create_goal`.
    pub const CREATE_GOAL_DESC: &str = include_str!("../descriptions/create_goal.txt");
    /// Description for `get_goal`.
    pub const GET_GOAL_DESC: &str = include_str!("../descriptions/get_goal.txt");
    /// Description for `update_goal`.
    pub const UPDATE_GOAL_DESC: &str = include_str!("../descriptions/update_goal.txt");
}

/// How one goal looks to the model.
///
/// The tool result is a *rendering*, not a JSON dump: the model has to act on this, and a
/// sentence it can read beats an object it has to interpret. The `details` carry the structured
/// form for the transcript's card.
fn render(view: &GoalView) -> String {
    let g = &view.goal;
    let mut out = format!(
        "{} ({}) — id {} revision {}, rounds {}/{}{}",
        g.status.as_str(),
        if view.armed {
            "continuation armed"
        } else {
            "continuation disarmed"
        },
        g.id,
        g.revision,
        g.rounds_started,
        g.max_rounds,
        if g.exhausted() { " (cap reached)" } else { "" },
    );
    out.push_str(&format!("\nobjective: {}", g.objective));
    if let Some(blocker) = &g.blocker {
        out.push_str(&format!(
            "\nblocked by {}: {}",
            blocker.code, blocker.reason
        ));
    }
    out
}

/// What "no goal" prints, and why it is a sentence rather than an error.
fn none_text() -> &'static str {
    "no goal is set in this session"
}

fn details(view: Option<&GoalView>) -> serde_json::Value {
    match view {
        Some(v) => serde_json::json!({
            "goal": {
                "id": v.goal.id,
                "revision": v.goal.revision,
                "objective": v.goal.objective,
                "status": v.goal.status.as_str(),
                "max_rounds": v.goal.max_rounds,
                "rounds_started": v.goal.rounds_started,
                "armed": v.armed,
            }
        }),
        None => serde_json::json!({ "goal": null }),
    }
}

fn text(body: String, view: Option<&GoalView>) -> ToolOutput {
    ToolOutput {
        content: vec![epsilon_contracts::content::ContentBlock::text(body)],
        details: details(view),
        usage: None,
        terminate: false,
    }
}

/// The host, or a refusal that names why there is none.
///
/// "No durable mirror" is the case that matters: a `task` child before its store exists, or a
/// test agent. Reporting it as a goal failure would be a lie about the session, so it is its own
/// sentence.
fn host(ctx: &ToolContext) -> Result<Arc<dyn GoalHost>, ToolFailure> {
    ctx.goal_host.clone().ok_or_else(|| {
        ToolFailure::msg(
            "this session has no durable store, so a goal cannot be recorded — start a normal \
             session (the goal lives in the session file, not in memory)",
        )
    })
}

/// Turn a typed refusal into the sentence the model reads.
///
/// Every arm says what to do next, because a refusal the caller cannot act on is a dead end: the
/// model's next move is supposed to be different from the one that just failed.
fn refused(e: GoalError) -> ToolFailure {
    let hint = match &e {
        GoalError::Stale { .. } => " Call get_goal for the current revision.",
        GoalError::NoGoal => " There is nothing to update; create one if the user asked for it.",
        GoalError::NoStore => {
            " This session cannot record one at all; tell the user rather than retrying."
        }
        GoalError::AlreadyCurrent { .. } => {
            " Edit, pause, complete, block or clear it first, or ask the user."
        }
        GoalError::RoundsExhausted { .. } => " Raise max_goal_rounds with an edit first.",
        GoalError::Invalid(_) => "",
    };
    ToolFailure::msg(format!("{e}.{hint}"))
}

// ---- create_goal -------------------------------------------------------------------

/// Arguments for `create_goal`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateGoalArgs {
    /// What the user is ultimately trying to achieve, in their own words.
    pub objective: String,
    /// Optional cap on automatic continuation rounds; the harness default applies when omitted.
    #[serde(default)]
    pub max_goal_rounds: Option<u32>,
}

/// Create the session's goal. Only when the user asked for long-running work.
pub struct CreateGoalTool {
    descriptor: ToolDescriptor,
}

impl CreateGoalTool {
    /// Build the tool.
    pub fn new() -> Self {
        Self {
            descriptor: ToolDescriptor {
                name: "create_goal".into(),
                label: "Create goal".into(),
                description: CREATE_GOAL_DESC.into(),
                parameters_schema: serde_json::to_value(schemars::schema_for!(CreateGoalArgs))
                    .unwrap_or(serde_json::json!({"type":"object"})),
                execution_mode: ExecutionMode::Parallel,
            },
        }
    }
}

impl Default for CreateGoalTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for CreateGoalTool {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    async fn execute(
        &self,
        call: ToolInvocation,
        ctx: ToolContext,
        _on_update: ToolUpdateSink,
    ) -> Result<ToolOutput, ToolFailure> {
        let args: CreateGoalArgs = serde_json::from_value(call.args)
            .map_err(|e| ToolFailure::msg(format!("invalid arguments for 'create_goal': {e}")))?;
        let host = host(&ctx)?;
        let op = GoalOp::Create {
            objective: args.objective,
            max_rounds: args.max_goal_rounds,
        };
        match host.apply_goal(None, op) {
            Ok(Some(view)) => Ok(text(
                format!("goal created\n{}", render(&view)),
                Some(&view),
            )),
            // `clear` is the only op that returns `None`, and this is not it.
            Ok(None) => Err(ToolFailure::msg(
                "the goal was created and immediately gone",
            )),
            Err(e) => Err(refused(e)),
        }
    }
}

// ---- get_goal ----------------------------------------------------------------------

/// Arguments for `get_goal`: none. The tool reads the session it is running in.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetGoalArgs {}

/// Read the current goal, including the id and revision `update_goal` requires.
pub struct GetGoalTool {
    descriptor: ToolDescriptor,
}

impl GetGoalTool {
    /// Build the tool.
    pub fn new() -> Self {
        Self {
            descriptor: ToolDescriptor {
                name: "get_goal".into(),
                label: "Get goal".into(),
                description: GET_GOAL_DESC.into(),
                parameters_schema: serde_json::to_value(schemars::schema_for!(GetGoalArgs))
                    .unwrap_or(serde_json::json!({"type":"object"})),
                execution_mode: ExecutionMode::Parallel,
            },
        }
    }
}

impl Default for GetGoalTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for GetGoalTool {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    async fn execute(
        &self,
        _call: ToolInvocation,
        ctx: ToolContext,
        _on_update: ToolUpdateSink,
    ) -> Result<ToolOutput, ToolFailure> {
        let host = host(&ctx)?;
        match host.current_goal() {
            Some(view) => Ok(text(render(&view), Some(&view))),
            None => Ok(text(none_text().to_string(), None)),
        }
    }
}

// ---- update_goal -------------------------------------------------------------------

/// What `update_goal` is asked to do, in DSH's vocabulary.
#[derive(Debug, Clone, Copy, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GoalAction {
    /// Rewrite the objective and/or the round cap. The phase is unchanged.
    Edit,
    /// Stop automatic continuation, keeping the state.
    Pause,
    /// Restart continuation; also clears a former blocker.
    Resume,
    /// Mark the objective achieved. Stops continuation.
    Complete,
    /// Record a concrete blocking condition that has to change before work can continue.
    Blocked,
    /// Remove the current goal. Its history stays in the session log.
    Clear,
}

/// Arguments for `update_goal`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateGoalArgs {
    /// The exact id from `get_goal`.
    pub goal_id: String,
    /// The exact revision from `get_goal`. A stale revision is refused rather than applied.
    pub revision: u64,
    /// Which lifecycle change to make.
    pub action: GoalAction,
    /// New objective, for `edit`.
    #[serde(default)]
    pub objective: Option<String>,
    /// New round cap, for `edit`.
    #[serde(default)]
    pub max_goal_rounds: Option<u32>,
    /// Short machine-routable code, for `blocked` (lower-kebab-case, e.g. `needs_human`).
    #[serde(default)]
    pub blocker_code: Option<String>,
    /// What the concrete blocking condition is, for `blocked`.
    #[serde(default)]
    pub blocker_reason: Option<String>,
    /// The goal's **whole** gate list, for `edit`: shell commands that must exit zero
    /// before this goal may be completed. Replaces what is there.
    ///
    /// **Commands, not objects.** `epsilon-contracts` has no `schemars` dependency and
    /// adding one for a single field would be a real change to a crate that is deliberately
    /// dependency-light; the timeout is the only other knob and its default is right.
    ///
    /// Replace rather than append, and an empty list is a request rather than an omission:
    /// a caller omits the field when the list is unchanged, and sends `[]` to take the
    /// rule off. A rule a user cannot remove is a rule the user is stuck with.
    #[serde(default)]
    pub gates: Option<Vec<String>>,
}

/// Change the current goal, or report that it is finished or stuck.
pub struct UpdateGoalTool {
    descriptor: ToolDescriptor,
    /// Runs the goal's gates when a completion is claimed.
    ///
    /// `None` means no evidence is available, and then **completion is still refused** when
    /// the goal declares gates. A tool that quietly stops enforcing a rule when its
    /// collaborator is missing is worse than one that never had it: the user reads "done"
    /// and believes a test passed.
    gate_runner: Option<Arc<crate::goal_gate::GoalGateRunner>>,
}

impl UpdateGoalTool {
    /// Build the tool.
    pub fn new() -> Self {
        Self {
            descriptor: ToolDescriptor {
                name: "update_goal".into(),
                label: "Update goal".into(),
                description: UPDATE_GOAL_DESC.into(),
                parameters_schema: serde_json::to_value(schemars::schema_for!(UpdateGoalArgs))
                    .unwrap_or(serde_json::json!({"type":"object"})),
                execution_mode: ExecutionMode::Parallel,
            },
            gate_runner: None,
        }
    }

    /// Give the tool the means to check a goal's completion rule.
    pub fn with_gate_runner(mut self, runner: Arc<crate::goal_gate::GoalGateRunner>) -> Self {
        self.gate_runner = Some(runner);
        self
    }
}

impl Default for UpdateGoalTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for UpdateGoalTool {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    async fn execute(
        &self,
        call: ToolInvocation,
        ctx: ToolContext,
        _on_update: ToolUpdateSink,
    ) -> Result<ToolOutput, ToolFailure> {
        let args: UpdateGoalArgs = serde_json::from_value(call.args)
            .map_err(|e| ToolFailure::msg(format!("invalid arguments for 'update_goal': {e}")))?;
        let host = host(&ctx)?;
        let expect = GoalRef {
            id: args.goal_id.clone(),
            revision: args.revision,
        };

        let op = match args.action {
            GoalAction::Edit => GoalOp::Edit {
                objective: args.objective,
                max_rounds: args.max_goal_rounds,
                gates: args
                    .gates
                    .map(|cmds| cmds.iter().map(GoalGate::new).collect()),
            },
            GoalAction::Pause => GoalOp::Pause,
            GoalAction::Resume => GoalOp::Resume,
            GoalAction::Complete => {
                // **A gate is the evidence that "done" means done.**
                //
                // The model's opinion is not evidence, and an unattended goal loop that
                // ends on one ends on the only judgement with nothing behind it. Gates run
                // before the completion is recorded, not after: recording it and then
                // checking would leave a durable `complete` to walk back.
                let gates = host
                    .current_goal()
                    .map(|v| v.goal.gates.clone())
                    .unwrap_or_default();
                if !gates.is_empty() {
                    let Some(runner) = self.gate_runner.as_ref() else {
                        return Err(ToolFailure::msg(format!(
                            "this goal declares {} completion gate(s) and this process cannot \
                             run them, so completion is refused rather than recorded unevidenced. \
                             Re-run with a confinement backend, or clear the gates first.",
                            gates.len()
                        )));
                    };
                    let mut failures = Vec::new();
                    for gate in &gates {
                        let report = runner.run(gate, &ctx.cwd).await;
                        if !report.passed {
                            failures.push(report);
                        }
                    }
                    if !failures.is_empty() {
                        let detail = failures
                            .iter()
                            .map(|r| {
                                let tail = if r.output_tail.trim().is_empty() {
                                    String::new()
                                } else {
                                    format!("\n{}", r.output_tail)
                                };
                                format!("{}{tail}", r.sentence())
                            })
                            .collect::<Vec<_>>()
                            .join("\n\n");
                        return Err(ToolFailure::msg(format!(
                            "completion refused: the goal's gate(s) have not passed.\n\n{detail}\n\n\
                             Fix what they report, or tell the user the rule no longer applies.",
                        )));
                    }
                }
                GoalOp::Complete
            }
            GoalAction::Clear => GoalOp::Clear,
            GoalAction::Blocked => {
                // DSH's floor, enforced: a blocker declared on the first obstacle is a model
                // giving up, not a goal that is stuck. Read from the live view rather than from
                // the caller's arguments so a stale ref cannot talk its way past the guard.
                let rounds = host.current_goal().map(|v| v.goal.rounds_started);
                if let Some(rounds) = rounds
                    && rounds < BLOCKED_MIN_ROUNDS
                {
                    return Err(ToolFailure::msg(format!(
                        "blocked is refused before {BLOCKED_MIN_ROUNDS} rounds of work \
                         (this goal has run {rounds}). Keep going, or tell the user what is in \
                         the way — difficulty is not a blocker."
                    )));
                }
                let code = args.blocker_code.unwrap_or_default();
                let reason = args.blocker_reason.unwrap_or_default();
                GoalOp::Blocked {
                    blocker: epsilon_contracts::goal::GoalBlocker { code, reason },
                }
            }
        };

        match host.apply_goal(Some(expect), op) {
            Ok(Some(view)) => Ok(text(
                format!("goal updated\n{}", render(&view)),
                Some(&view),
            )),
            Ok(None) => Ok(text(
                "goal cleared. Its history stays in the session log.".to_string(),
                None,
            )),
            Err(e) => Err(refused(e)),
        }
    }
}

/// The default round cap, re-exported so a caller can name it without reaching into contracts.
pub const GOAL_DEFAULT_ROUNDS: u32 = DEFAULT_MAX_GOAL_ROUNDS;

#[cfg(test)]
mod tests {
    use super::*;
    use epsilon_contracts::goal::{GoalSnapshot, GoalStatus};
    use std::sync::Mutex;

    /// A goal host backed by an in-memory snapshot: the smallest thing that can answer the two
    /// calls these tools make. Real persistence is `epsilon-session`'s, and is tested there.
    #[derive(Default)]
    struct FakeHost {
        goal: Mutex<Option<GoalSnapshot>>,
        armed: Mutex<bool>,
    }

    impl FakeHost {
        fn with_rounds(rounds: u32) -> Self {
            Self::build(rounds, Vec::new())
        }

        /// A host whose goal carries `gates` — the fixture the gate tests need, and the
        /// reason the gates live on the snapshot rather than beside it.
        fn with_gates(gates: Vec<GoalGate>) -> Self {
            Self::build(0, gates)
        }

        fn build(rounds: u32, gates: Vec<GoalGate>) -> Self {
            let host = Self::default();
            *host.goal.lock().unwrap() = Some(GoalSnapshot {
                id: "g1".into(),
                revision: 2,
                objective: "make the doc gates pass".into(),
                status: GoalStatus::Active,
                max_rounds: 8,
                rounds_started: rounds,
                blocker: None,
                gates,
                created_at_ms: 0,
                updated_at_ms: 0,
            });
            *host.armed.lock().unwrap() = true;
            host
        }
    }

    impl GoalHost for FakeHost {
        fn current_goal(&self) -> Option<GoalView> {
            self.goal.lock().unwrap().clone().map(|goal| GoalView {
                goal,
                armed: *self.armed.lock().unwrap(),
            })
        }

        fn apply_goal(
            &self,
            _expect: Option<GoalRef>,
            op: GoalOp,
        ) -> Result<Option<GoalView>, GoalError> {
            let mut slot = self.goal.lock().unwrap();
            let mut goal = slot.clone().ok_or(GoalError::NoGoal)?;
            match op {
                GoalOp::Complete => goal.status = GoalStatus::Complete,
                GoalOp::Blocked { blocker } => {
                    goal.status = GoalStatus::Blocked;
                    goal.blocker = Some(blocker);
                }
                GoalOp::Clear => {
                    *slot = None;
                    return Ok(None);
                }
                _ => {}
            }
            goal.revision += 1;
            *slot = Some(goal.clone());
            Ok(Some(GoalView {
                goal,
                armed: *self.armed.lock().unwrap(),
            }))
        }
    }

    fn ctx_with(host: Option<Arc<dyn GoalHost>>) -> ToolContext {
        ToolContext {
            session_id: "t".into(),
            observation_scope: Arc::new(()),
            cwd: std::env::temp_dir(),
            spill_dir: std::env::temp_dir(),
            token: Default::default(),
            task_host: None,
            goal_host: host,
        }
    }

    async fn call(
        tool: &dyn Tool,
        args: serde_json::Value,
        host: Arc<dyn GoalHost>,
    ) -> Result<ToolOutput, ToolFailure> {
        tool.execute(
            ToolInvocation {
                tool_call_id: "tc".into(),
                args,
            },
            ctx_with(Some(host)),
            Arc::new(|_| {}),
        )
        .await
    }

    fn body(out: &ToolOutput) -> String {
        match &out.content[0] {
            epsilon_contracts::content::ContentBlock::Text { text } => text.clone(),
            other => panic!("text expected, got {other:?}"),
        }
    }

    /// **The test M240 was missing**: a goal can declare a gate, and the gate is enforced
    /// when the model tries to complete it.
    ///
    /// The gate tests in `tests/goal_gates.rs` prove the runner works and that the field
    /// decodes. This one proves the two are **connected**, which is the whole feature: a
    /// gate that can be declared but never blocks completion is a field, not a rule.
    #[tokio::test]
    async fn a_declared_gate_refuses_a_completion_it_fails() {
        let host: Arc<dyn GoalHost> = Arc::new(FakeHost::with_gates(vec![GoalGate::new(
            "cargo definitely-not-a-subcommand",
        )]));
        let tool = UpdateGoalTool::new().with_gate_runner(Arc::new(
            crate::goal_gate::GoalGateRunner::new(
                None,
                Some(std::sync::Arc::new(epsilon_safety::AutoApprove)),
            ),
        ));
        let err = call(
            &tool,
            serde_json::json!({"action": "complete", "goal_id": "g1", "revision": 2}),
            host,
        )
        .await
        .expect_err("a failing gate must refuse the completion");
        assert!(
            err.message.contains("refused"),
            "the refusal must say it refused: {}",
            err.message
        );
        assert!(
            err.message.contains("cargo definitely-not-a-subcommand"),
            "and name the command, which is the only part a reader can act on: {}",
            err.message
        );
        assert!(
            !err.message.contains("goal updated"),
            "and must not have recorded the completion anyway: {}",
            err.message
        );
    }

    /// The other half, so the refusal above is the gate working rather than the tool
    /// refusing everything.
    #[tokio::test]
    async fn a_declared_gate_that_passes_lets_the_completion_through() {
        let host: Arc<dyn GoalHost> =
            Arc::new(FakeHost::with_gates(vec![GoalGate::new("cargo --version")]));
        let tool = UpdateGoalTool::new().with_gate_runner(Arc::new(
            crate::goal_gate::GoalGateRunner::new(
                None,
                Some(std::sync::Arc::new(epsilon_safety::AutoApprove)),
            ),
        ));
        let out = call(
            &tool,
            serde_json::json!({"action": "complete", "goal_id": "g1", "revision": 2}),
            host,
        )
        .await
        .expect("a passing gate must not refuse");
        assert!(body(&out).contains("goal updated"), "{}", body(&out));
    }

    /// A goal with **no** gates is exactly as completable as it was. A feature that made
    /// completion harder for everyone until they declared a rule would be adopted by nobody.
    #[tokio::test]
    async fn a_goal_with_no_gates_completes_as_it_always_did() {
        let host: Arc<dyn GoalHost> = Arc::new(FakeHost::with_gates(Vec::new()));
        let tool = UpdateGoalTool::new();
        call(
            &tool,
            serde_json::json!({"action": "complete", "goal_id": "g1", "revision": 2}),
            host,
        )
        .await
        .expect("a goal with no gates is unaffected by this feature");
    }

    #[tokio::test]
    async fn get_goal_reports_the_state_and_the_ref_a_writer_needs() {
        let host: Arc<dyn GoalHost> = Arc::new(FakeHost::with_rounds(3));
        let out = call(&GetGoalTool::new(), serde_json::json!({}), host)
            .await
            .unwrap();
        let text = body(&out);
        assert!(text.contains("active"), "{text}");
        assert!(text.contains("id g1 revision 2"), "{text}");
        assert!(text.contains("rounds 3/8"), "{text}");
        assert!(text.contains("continuation armed"), "{text}");
        assert_eq!(out.details["goal"]["revision"], 2);
    }

    #[tokio::test]
    async fn get_goal_says_there_is_none_rather_than_failing() {
        let host: Arc<dyn GoalHost> = Arc::new(FakeHost::default());
        let out = call(&GetGoalTool::new(), serde_json::json!({}), host)
            .await
            .unwrap();
        assert!(body(&out).contains("no goal"));
        assert!(out.details["goal"].is_null());
    }

    #[tokio::test]
    async fn blocked_is_refused_before_the_floor_and_allowed_after_it() {
        // The rule DSH ships, and the one that decides whether this tool is a way to report a
        // wall or a way to stop working: one obstacle is not a blocker.
        let early: Arc<dyn GoalHost> = Arc::new(FakeHost::with_rounds(1));
        let err = call(
            &UpdateGoalTool::new(),
            serde_json::json!({
                "goal_id": "g1", "revision": 2, "action": "blocked",
                "blocker_code": "needs_human", "blocker_reason": "the API key is missing"
            }),
            early,
        )
        .await
        .unwrap_err();
        assert!(
            err.message.contains("refused before 3 rounds"),
            "{}",
            err.message
        );
        assert!(
            err.message.contains("difficulty is not a blocker"),
            "{}",
            err.message
        );

        let late: Arc<dyn GoalHost> = Arc::new(FakeHost::with_rounds(BLOCKED_MIN_ROUNDS));
        let out = call(
            &UpdateGoalTool::new(),
            serde_json::json!({
                "goal_id": "g1", "revision": 2, "action": "blocked",
                "blocker_code": "needs_human", "blocker_reason": "the API key is missing"
            }),
            late,
        )
        .await
        .unwrap();
        let text = body(&out);
        assert!(text.contains("blocked"), "{text}");
        assert!(
            text.contains("needs_human: the API key is missing"),
            "{text}"
        );
    }

    #[tokio::test]
    async fn a_stale_revision_is_refused_with_the_way_forward() {
        // The host refuses it (CAS is the host's rule, not the tool's), and what this asserts is
        // the sentence: a refusal that does not say "call get_goal" leaves the model retrying the
        // same call.
        struct StaleHost;
        impl GoalHost for StaleHost {
            fn current_goal(&self) -> Option<GoalView> {
                None
            }
            fn apply_goal(
                &self,
                expect: Option<GoalRef>,
                _op: GoalOp,
            ) -> Result<Option<GoalView>, GoalError> {
                Err(GoalError::Stale {
                    expected: expect.unwrap_or(GoalRef {
                        id: "g1".into(),
                        revision: 1,
                    }),
                    actual: GoalRef {
                        id: "g1".into(),
                        revision: 9,
                    },
                })
            }
        }
        let host: Arc<dyn GoalHost> = Arc::new(StaleHost);
        let err = call(
            &UpdateGoalTool::new(),
            serde_json::json!({"goal_id": "g1", "revision": 1, "action": "pause"}),
            host,
        )
        .await
        .unwrap_err();
        assert!(err.message.contains("revision 9"), "{}", err.message);
        assert!(err.message.contains("get_goal"), "{}", err.message);
    }

    #[tokio::test]
    async fn clear_reports_the_outcome_rather_than_an_error() {
        let host: Arc<dyn GoalHost> = Arc::new(FakeHost::with_rounds(1));
        let out = call(
            &UpdateGoalTool::new(),
            serde_json::json!({"goal_id": "g1", "revision": 2, "action": "clear"}),
            host,
        )
        .await
        .unwrap();
        assert!(body(&out).contains("cleared"), "{}", body(&out));
        assert!(out.details["goal"].is_null());
    }

    #[tokio::test]
    async fn a_session_with_no_store_says_so_instead_of_pretending() {
        // The wrong answer here is the interesting one: a tool that reported "no goal is set"
        // would tell the model the session has no objective, when the truth is that this session
        // cannot record one at all.
        let out = GetGoalTool::new()
            .execute(
                ToolInvocation {
                    tool_call_id: "tc".into(),
                    args: serde_json::json!({}),
                },
                ctx_with(None),
                Arc::new(|_| {}),
            )
            .await;
        let err = out.unwrap_err();
        assert!(err.message.contains("no durable store"), "{}", err.message);
    }
}
