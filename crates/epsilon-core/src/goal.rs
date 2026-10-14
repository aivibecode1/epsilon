//! The session goal — one long-running objective per session (ADR-006).

use std::sync::Arc;
use std::sync::atomic::Ordering;

use epsilon_contracts::goal::{
    DEFAULT_MAX_GOAL_ROUNDS, GoalChange, GoalError, GoalOp, GoalRef, GoalSnapshot, GoalStatus,
    GoalView,
};

use crate::{Agent, now_ms};

impl Agent {
    // ---- the session goal (ADR-006 / DSH's goal domain) -------------------------------
    //
    // One long-running objective per session, persisted in the session log and nowhere else.
    // Everything a reader needs is `goal()`; everything a writer needs is `goal_apply`, and
    // the two differences from DSH's service are deliberate and named:
    //
    //   * there is no async surface, because the store is a `std::sync` mutex over file I/O
    //     and awaiting while holding it is how a run deadlocks;
    //   * activation is one flag on the Agent rather than a per-session cache, because
    //     Epsilon has one live conversation per Agent.

    /// The current goal, with whether this process may continue it on its own.
    ///
    /// `None` covers three situations that a caller must not conflate, which is why the
    /// *status* is in the view and not in this return: no goal was ever set, the goal was
    /// cleared, and there is no durable mirror to read one from (a `task` child before its
    /// store exists). A frontend that needs to tell them apart asks
    /// [`Agent::session_store`].
    pub fn goal(&self) -> Option<GoalView> {
        let store = self.store()?;
        let goal = store.lock().ok()?.goal().ok()??;
        Some(GoalView {
            goal,
            armed: self.goal_armed(),
        })
    }

    /// Whether automatic continuation is armed in this process.
    ///
    /// Never persisted: a goal that survives a restart does not get to keep going by itself,
    /// because the person who started it is not necessarily the person looking at it now.
    pub fn goal_armed(&self) -> bool {
        self.goal_armed.load(Ordering::SeqCst)
    }

    /// Arm or disarm automatic continuation without touching the durable state.
    ///
    /// This is the `disarm()` half of DSH's activation model: it writes no revision and emits
    /// no mutation, because "may this process continue" is not part of what a goal *is*.
    pub fn set_goal_armed(&self, armed: bool) {
        self.goal_armed.store(armed, Ordering::SeqCst);
    }

    /// Apply one goal operation, with compare-and-set.
    ///
    /// `expect` is the revision the caller read: `None` means "whatever is current" and is for a
    /// frontend acting on what the user is looking at; `Some` is for a caller holding an older
    /// view, which is every model tool call.
    ///
    /// The rules, each one DSH's and each one load-bearing:
    ///
    /// * **One current goal.** A create is refused while an unfinished goal holds the slot
    ///   (`GoalError::AlreadyCurrent`); a completed one may be replaced, because the alternative
    ///   is making the user clear a goal they just finished.
    /// * **Resume clears the blocker** and refuses when the round cap is spent — rearming an
    ///   exhausted goal would only fire the guard again.
    /// * **Create and resume arm; pause, complete, blocked and clear disarm.** Completion is the
    ///   only phase that is terminal, and blocking is the only one that carries a reason.
    pub fn goal_apply(
        &self,
        expect: Option<GoalRef>,
        op: GoalOp,
    ) -> Result<Option<GoalView>, GoalError> {
        let store = self.store().ok_or(GoalError::NoStore)?;
        let mut guard = store.lock().map_err(|_| GoalError::NoStore)?;
        let current = guard
            .goal()
            .map_err(|e| GoalError::Invalid(e.to_string()))?;
        let previous_revision = current.as_ref().map(|g| g.revision);
        let is_create = matches!(op, GoalOp::Create { .. });

        // Compare-and-set first, so a stale caller is told the goal moved rather than being
        // handed a refusal about the operation it asked for.
        if let Some(expected) = &expect {
            match current.as_ref() {
                Some(actual)
                    if expected.id == actual.id && expected.revision == actual.revision => {}
                Some(actual) => {
                    return Err(GoalError::Stale {
                        expected: expected.clone(),
                        actual: actual.as_ref(),
                    });
                }
                None => return Err(GoalError::NoGoal),
            }
        }

        let now = now_ms();
        let next: GoalSnapshot = match (&op, current) {
            (
                GoalOp::Create {
                    objective,
                    max_rounds,
                },
                None,
            ) => new_goal(objective, *max_rounds, now)?,
            (GoalOp::Create { .. }, Some(existing)) if !existing.status.is_terminal() => {
                return Err(GoalError::AlreadyCurrent {
                    current: existing.as_ref(),
                });
            }
            // Replacing a completed goal takes a new identity, so the finished one stays
            // addressable in the log and a stale ref to it can never match the new goal.
            (
                GoalOp::Create {
                    objective,
                    max_rounds,
                },
                Some(_),
            ) => new_goal(objective, *max_rounds, now)?,
            (_, None) => return Err(GoalError::NoGoal),
            (
                GoalOp::Edit {
                    objective,
                    max_rounds,
                    gates,
                },
                Some(mut g),
            ) => {
                if objective.is_none() && max_rounds.is_none() && gates.is_none() {
                    return Err(GoalError::Invalid(
                        "edit needs an objective, a round cap, a gate list, or some combination"
                            .into(),
                    ));
                }
                if let Some(objective) = objective {
                    let objective = objective.trim();
                    if objective.is_empty() {
                        return Err(GoalError::Invalid(
                            "an edit cannot blank the objective; clear the goal instead".into(),
                        ));
                    }
                    g.objective = objective.to_string();
                }
                if let Some(max_rounds) = max_rounds {
                    if *max_rounds == 0 {
                        return Err(GoalError::Invalid("max_rounds must be at least 1".into()));
                    }
                    g.max_rounds = *max_rounds;
                }
                if let Some(gates) = gates {
                    // An empty gate list is a **request**, not a no-op: it is how a user
                    // takes the rule back off a goal, and a rule that cannot be removed
                    // is a rule the user is stuck with.
                    if let Some(bad) = gates.iter().find(|g| !g.is_runnable()) {
                        let _ = bad;
                        return Err(GoalError::Invalid(
                            "a gate needs a command; one with an empty command could never pass \
                             and would block completion for ever"
                                .into(),
                        ));
                    }
                    g.gates.clone_from(gates);
                }
                g
            }
            (GoalOp::Pause, Some(mut g)) => {
                g.status = GoalStatus::Paused;
                g
            }
            (GoalOp::Resume, Some(mut g)) => {
                if g.exhausted() {
                    return Err(GoalError::RoundsExhausted {
                        rounds_started: g.rounds_started,
                        max_rounds: g.max_rounds,
                    });
                }
                g.status = GoalStatus::Active;
                // Resume clears a former blocker: the phase and the reason behind it are one
                // fact, and a goal that is active again while still carrying a blocker would
                // show a reason nobody can act on.
                g.blocker = None;
                g
            }
            (GoalOp::Complete, Some(mut g)) => {
                g.status = GoalStatus::Complete;
                g.blocker = None;
                g
            }
            (GoalOp::Blocked { blocker }, Some(mut g)) => {
                if blocker.code.trim().is_empty() || blocker.reason.trim().is_empty() {
                    return Err(GoalError::Invalid(
                        "a blocker needs a code and a reason; a blocked goal nobody can explain \
                         is indistinguishable from one that stalled"
                            .into(),
                    ));
                }
                g.status = GoalStatus::Blocked;
                g.blocker = Some(blocker.clone());
                g
            }
            (GoalOp::Clear, Some(g)) => {
                let change = GoalChange::Cleared {
                    id: g.id.clone(),
                    revision: g.revision + 1,
                };
                guard
                    .append_goal(change)
                    .map_err(|e| GoalError::Invalid(e.to_string()))?;
                self.set_goal_armed(false);
                // `None` is the outcome, not a failure: the caller asked for the goal to be
                // gone, and it is gone.
                return Ok(None);
            }
        };

        let mut next = next;
        if !is_create {
            // Monotonic by one, which is what makes the compare-and-set ref meaningful: a
            // revision that jumped would let a stale caller's ref match a later state.
            next.revision = previous_revision.unwrap_or(0) + 1;
        }
        next.updated_at_ms = now;
        guard
            .append_goal(GoalChange::Set { goal: next.clone() })
            .map_err(|e| GoalError::Invalid(e.to_string()))?;
        // Arming is the caller-visible half of the same decision, and it happens only after the
        // durable write succeeded: a process that armed itself on a failed append would continue
        // a goal the next reader cannot see.
        match op {
            GoalOp::Create { .. } | GoalOp::Resume => self.set_goal_armed(true),
            GoalOp::Pause | GoalOp::Complete | GoalOp::Blocked { .. } => self.set_goal_armed(false),
            GoalOp::Edit { .. } | GoalOp::Clear => {}
        }
        Ok(Some(GoalView {
            goal: next,
            armed: self.goal_armed(),
        }))
    }

    /// Spend one round against the current goal, durably, and return the prompt to send.
    ///
    /// The round is counted when it is *sent*, not when it succeeds: a round that crashed on the
    /// way is a round that was spent, and a counter that only advanced on success would let a
    /// loop that fails instantly run for ever.
    ///
    /// `None` means this process must not continue — no goal, not active, not armed, or the cap
    /// is reached — which is the same condition [`GoalView::is_continuable`] states, evaluated
    /// against the log at the moment the decision is made rather than against a view read
    /// earlier.
    pub fn goal_begin_round(&self) -> Result<Option<String>, GoalError> {
        let Some(view) = self.goal() else {
            return Ok(None);
        };
        if !view.is_continuable() {
            return Ok(None);
        }
        let mut goal = view.goal;
        goal.rounds_started += 1;
        goal.revision += 1;
        goal.updated_at_ms = now_ms();
        let store = self.store().ok_or(GoalError::NoGoal)?;
        store
            .lock()
            .map_err(|_| GoalError::NoGoal)?
            .append_goal(GoalChange::Set { goal: goal.clone() })
            .map_err(|e| GoalError::Invalid(e.to_string()))?;
        Ok(Some(goal_round_prompt(&goal)))
    }

    /// The ``GoalHost`` view of this agent, for a `ToolContext`.
    ///
    /// **Public because composition needs it, not because a test wanted it.** The execution path
    /// builds this itself (`exec.rs`), and a frontend that wants to invoke a goal tool outside a
    /// turn — or a test that wants to drive the tool against a *real* goal rather than a snapshot
    /// of one — needs the same adapter. A second implementation would be a second answer to
    /// "where does a goal live", which is the thing that goes wrong when there are two.
    pub fn clone_goal_host(self: &Arc<Self>) -> Arc<dyn epsilon_contracts::tool::GoalHost> {
        Arc::clone(self) as Arc<dyn epsilon_contracts::tool::GoalHost>
    }
}

/// The goal record a `create` writes, validated.
///
/// A free function because two match arms build it — the empty slot and the slot held by a
/// *completed* goal — and two copies of "a goal needs an objective" is how one of them ends up
/// accepting an empty one.
fn new_goal(objective: &str, max_rounds: Option<u32>, now: u64) -> Result<GoalSnapshot, GoalError> {
    let objective = objective.trim();
    if objective.is_empty() {
        return Err(GoalError::Invalid("a goal needs an objective".into()));
    }
    let max_rounds = max_rounds.unwrap_or(DEFAULT_MAX_GOAL_ROUNDS);
    if max_rounds == 0 {
        return Err(GoalError::Invalid(
            "max_rounds must be at least 1; a goal that may never continue is a note".into(),
        ));
    }
    Ok(GoalSnapshot {
        id: epsilon_contracts::new_id("goal"),
        revision: 1,
        objective: objective.to_string(),
        status: GoalStatus::Active,
        max_rounds,
        rounds_started: 0,
        blocker: None,
        // A new goal has no completion rule beyond the caller's own judgement. Gates are
        // added by an explicit edit, never inferred from the objective's text: a harness
        // that guesses `cargo test` from the word "fix" would run a command the user never
        // asked for.
        gates: Vec::new(),
        created_at_ms: now,
        updated_at_ms: now,
    })
}

/// The message a continuation round sends.
///
/// It names the objective rather than assuming the model remembers it, states the round and the
/// cap so the model can judge how much room is left, and says what ends the loop — because a
/// continuation the model cannot stop is a loop only a human can interrupt, and the human is not
/// watching.
///
/// The wording is deliberately plain and imperative, and it is the *only* thing a continuation
/// adds to the conversation: DSH's rule that a goal mutation injects no model context is kept,
/// so a round costs one message and nothing else.
fn goal_round_prompt(goal: &GoalSnapshot) -> String {
    format!(
        "Continuing the session goal (round {}/{}). Objective: {}\n\n\
         Work on it, then stop and report what changed and what is left. If the objective is \
         achieved, mark the goal complete with the `goal` tool; if you are blocked by something \
         you cannot change, mark it blocked and say what it is. The harness starts the next \
         round only if the goal is still active.",
        goal.rounds_started, goal.max_rounds, goal.objective
    )
}

/// The `goal` tool's way back into session state, over the same seam `task` uses.
///
/// No `#[async_trait]`: both methods are synchronous by design (see the trait's own docs), and
/// making them `async` to match a neighbouring trait would be a shape borrowed from a different
/// problem.
impl epsilon_contracts::tool::GoalHost for Agent {
    fn current_goal(&self) -> Option<GoalView> {
        Agent::goal(self)
    }

    fn apply_goal(
        &self,
        expect: Option<GoalRef>,
        op: GoalOp,
    ) -> Result<Option<GoalView>, GoalError> {
        Agent::goal_apply(self, expect, op)
    }
}
