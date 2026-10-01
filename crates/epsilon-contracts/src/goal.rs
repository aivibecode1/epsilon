//! One long-running objective per session, persisted in the session log.
//!
//! Ported from DeepSeek Harness's goal domain (`packages/goal/goal/src/types.ts`,
//! `docs/subsystems/goal.md`), narrowed to what this harness actually drives. The parts kept,
//! and why each one is load-bearing:
//!
//! * **The session log is the only durable store.** A goal is not a settings file and not a
//!   side database: every mutation is a [`crate::session_entry::EntryKind::Goal`] entry carrying
//!   the complete post-mutation snapshot, and the current goal is a fold over those entries. That
//!   is what makes a goal survive a restart, a fork and a resume with no separate migration.
//! * **Compare-and-set.** Every mutation names the exact `{id, revision}` it believes is current,
//!   and a stale ref is refused rather than applied. Without it, a tool call that read the goal
//!   two turns ago silently overwrites a change the user made in between.
//! * **Activation is process-local.** `armed` is deliberately *not* in the snapshot: it says
//!   whether *this process* may continue the goal on its own, and it is cleared on every start.
//!   A durable "keep going" flag would make a crashed session continue by itself the next time
//!   someone opened it, which is the opposite of what a person expects.
//! * **Rounds are bounded.** `max_rounds` (256 by default, DSH's own default) is the only budget
//!   there is, and it counts *attempted* rounds. DSH is explicit that it is not a token, currency
//!   or wall-clock budget, and that absence is deliberate rather than unfinished.
//!
//! What is deliberately **not** here: a round driver (a consumer's job — the TUI owns it), an
//! evaluator that certifies completion (the caller that records it is authoritative), and any
//! notion of parallel goals.

use serde::{Deserialize, Serialize};

/// The round cap a goal gets when its creator does not name one.
///
/// DSH's `defaultMaxGoalRounds`, and the same number, because it is a policy default rather
/// than a protocol constant: 256 rounds of a coding agent is already far past the point where a
/// human should look at the objective again.
pub const DEFAULT_MAX_GOAL_ROUNDS: u32 = 256;

/// The condition that stops an automatic continuation.
///
/// DSH's rule, kept verbatim because it is the difference between a useful guard and one that
/// fires on the first hard problem: **mark blocked only after the same blocking condition
/// persists for at least three consecutive rounds**, and report that concrete condition. Being
/// stuck, uncertain, or merely having more work to do is not blocked.
pub const BLOCKED_MIN_ROUNDS: u32 = 3;

/// How long a gate may run before it is treated as failed.
///
/// Five minutes is a full test suite on a slow machine, and it is the default rather than a
/// protocol constant because "how long is too long" is a policy choice the person who wrote
/// the gate can override per gate.
pub const DEFAULT_GATE_TIMEOUT_SECS: u32 = 300;

/// A deterministic command that must pass before a goal may be called done.
///
/// # Why this exists when the model already says "done"
///
/// Because the model saying it is not evidence. A goal's whole purpose is unattended
/// continuation, and an unattended loop that ends on the model's opinion ends on the one
/// judgement with nothing behind it. `cargo test` exits zero or it does not; "I believe the
/// fix is correct" is neither reproducible nor checkable.
///
/// Hermes calls this `GoalGate` (`hermes_cli/goals.py:337-368`) and it is the single most
/// valuable idea in that reference: gates run at the turn boundary **before** the model is
/// asked to judge completion, and a failing gate's bounded output becomes the continuation
/// prompt. Epsilon has no LLM judge — the caller that records completion is authoritative,
/// which this module's header already says — so a gate is exactly that caller's evidence.
///
/// # It is durable state, not a setting
///
/// A gate is part of the objective. It lives on [`GoalSnapshot`] beside `objective`, because
/// "the goal is not done until the tests pass" is a fact about the goal, and a goal whose
/// completion rule lives somewhere else can be completed by anything that forgets to look.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoalGate {
    /// The shell command to run. Shell text, exactly as `bash` takes it.
    pub command: String,
    /// Wall-clock limit in seconds; a gate that exceeds it fails.
    #[serde(default = "default_gate_timeout_secs")]
    pub timeout_seconds: u32,
}

fn default_gate_timeout_secs() -> u32 {
    DEFAULT_GATE_TIMEOUT_SECS
}

impl GoalGate {
    /// A gate with the default timeout.
    pub fn new(command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            timeout_seconds: DEFAULT_GATE_TIMEOUT_SECS,
        }
    }

    /// Whether this gate can be run at all — an empty command is a rule that can never pass,
    /// which would block completion for ever with nothing to fix.
    pub fn is_runnable(&self) -> bool {
        !self.command.trim().is_empty()
    }
}

/// A goal's durable phase.
///
/// Four, and no more: DSH folds provider limits, exhausted budgets, execution errors and
/// requests for human input into `Blocked` with a code and an explanation rather than
/// multiplying the lifecycle. Four states with a reason beat twelve states without one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalStatus {
    /// The goal is being worked on and may be continued automatically.
    Active,
    /// Stopped by a person; the state is kept.
    Paused,
    /// Stopped by a concrete condition that has to change before it can continue.
    Blocked,
    /// Achieved. Disarmed, and replaceable by a new goal.
    Complete,
}

impl GoalStatus {
    /// The word this phase is called on screen and on the wire.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Paused => "paused",
            Self::Blocked => "blocked",
            Self::Complete => "complete",
        }
    }

    /// Whether a goal in this phase may be continued automatically.
    ///
    /// Only `Active` — and the caller must also check [`GoalView::armed`], which is the
    /// process-local half of the same question.
    pub fn is_continuable(self) -> bool {
        matches!(self, Self::Active)
    }

    /// Whether this phase is finished, so a new goal may replace it without an explicit clear.
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Complete)
    }
}

/// Why a goal is blocked: a stable code, and a sentence for a person.
///
/// The code is what a policy consumer routes on (`provider_limit`, `budget_exhausted`,
/// `needs_human`, …) and the reason is what a reader acts on. DSH requires the code to be
/// lower-kebab-case so it can be matched rather than parsed; nothing here enforces that, and the
/// tool that writes one is where the constraint belongs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoalBlocker {
    /// Machine-routable code, lower-kebab-case by convention.
    pub code: String,
    /// Human-readable explanation of the concrete condition.
    pub reason: String,
}

/// One goal's complete durable state, as written by every mutation.
///
/// The snapshot is whole rather than a delta, which is what lets the fold be a `for` loop: the
/// last valid entry *is* the state, and a reader never has to replay a sequence of edits to find
/// out what an objective says now.
///
/// # Field defaults, and the M202 rule
///
/// Every field this type gains after today must carry `#[serde(default)]`, because a session
/// written by an older Epsilon will not have it and the goal fold must not be the thing that
/// makes an old session unopenable. `revision` and `id` are the exceptions by nature — a
/// snapshot without them cannot be applied to anything — but they are also the two that will
/// never be added.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoalSnapshot {
    /// Stable identity, unique within the session.
    pub id: String,
    /// Monotonic revision, incremented by one on every accepted mutation.
    ///
    /// This is the compare-and-set generation: a mutation carries the revision it read, and a
    /// mismatch is refused. It is deliberately *not* a timestamp, because two mutations inside
    /// one millisecond must still be distinguishable.
    pub revision: u64,
    /// What the user is ultimately trying to achieve.
    pub objective: String,
    /// The durable phase.
    pub status: GoalStatus,
    /// Round cap for automatic continuation.
    #[serde(default = "default_max_rounds")]
    pub max_rounds: u32,
    /// Rounds attempted so far. Counted when a round is *sent*, not when it succeeds: a round
    /// that crashed on the way is a round that was spent.
    #[serde(default)]
    pub rounds_started: u32,
    /// Set while [`GoalStatus::Blocked`], cleared by resume.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocker: Option<GoalBlocker>,
    /// Commands that must pass before this goal may be completed.
    ///
    /// Empty — the default — means "the caller decides", which is the status quo and the
    /// right default: a goal with no gate is not blocked from completing, it simply has no
    /// evidence beyond the caller's own judgement.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gates: Vec<GoalGate>,
    /// Unix epoch milliseconds when the goal was created.
    #[serde(default)]
    pub created_at_ms: u64,
    /// Unix epoch milliseconds of the last accepted mutation.
    #[serde(default)]
    pub updated_at_ms: u64,
}

fn default_max_rounds() -> u32 {
    DEFAULT_MAX_GOAL_ROUNDS
}

impl GoalSnapshot {
    /// Rounds still available under the cap.
    pub fn rounds_remaining(&self) -> u32 {
        self.max_rounds.saturating_sub(self.rounds_started)
    }

    /// Whether the cap has been reached.
    pub fn exhausted(&self) -> bool {
        self.rounds_started >= self.max_rounds
    }

    /// The `{id, revision}` a mutation has to carry to be accepted.
    pub fn as_ref(&self) -> GoalRef {
        GoalRef {
            id: self.id.clone(),
            revision: self.revision,
        }
    }
}

/// Compare-and-set identity for one exact goal revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoalRef {
    /// Which goal.
    pub id: String,
    /// Which revision of it the caller believes is current.
    pub revision: u64,
}

/// What a reader sees: the durable snapshot, plus whether *this process* may continue it.
///
/// The activation flag lives here and not in [`GoalSnapshot`] because it is the one part of a
/// goal's state that must not survive a process: see the module docs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoalView {
    /// The durable state.
    pub goal: GoalSnapshot,
    /// Whether automatic continuation is armed in this process.
    pub armed: bool,
}

impl GoalView {
    /// Whether this goal may be continued on its own right now: active, armed, and under cap.
    pub fn is_continuable(&self) -> bool {
        self.armed && self.goal.status.is_continuable() && !self.goal.exhausted()
    }

    /// The one-line summary a status row shows.
    ///
    /// Deliberately not the objective: a row has room for a state and a count, and the
    /// objective is one `/goal` away. What a reader needs at a glance is *that* a goal exists,
    /// whether it is moving, and how much of its budget is left — so the row carries the mark
    /// and the count, and the three phases that are not "active" say their name because a mark
    /// alone cannot distinguish paused from blocked from done.
    pub fn badge(&self) -> String {
        let mark = match self.goal.status {
            GoalStatus::Complete => '✓',
            GoalStatus::Blocked => '!',
            // Armed and active is the only state that will continue by itself, and the filled
            // mark is the whole difference between "this keeps going" and "this is waiting".
            _ if self.armed && self.goal.status.is_continuable() => '●',
            _ => '○',
        };
        let progress = format!("{}/{}", self.goal.rounds_started, self.goal.max_rounds);
        match self.goal.status {
            GoalStatus::Active => format!("{mark} goal {progress}"),
            other => format!("{mark} goal {} {progress}", other.as_str()),
        }
    }
}

/// One durable change to the session's goal, as written into the log.
///
/// DSH writes the complete post-mutation snapshot for every mutation and a revisioned tombstone
/// for `clear`. Both are here, and the tombstone carries the id it clears so a `clear` that
/// arrives after a *newer* goal was created cannot remove that newer goal by accident.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "change", rename_all = "snake_case")]
pub enum GoalChange {
    /// The goal is now exactly this snapshot.
    Set {
        /// The complete post-mutation state.
        goal: GoalSnapshot,
    },
    /// The goal with this id was removed. History is untouched; only the current goal goes.
    Cleared {
        /// The goal that was cleared.
        id: String,
        /// The revision the clear was issued against, plus one.
        revision: u64,
    },
}

/// The operations a caller may ask for, each one a durable change when accepted.
///
/// A closed set rather than a builder: DSH's verbs are the lifecycle, and a caller that wants
/// something else is describing a change to the *goal*, not a new operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GoalOp {
    /// Start a new goal, replacing a terminal one.
    Create {
        /// What the user is trying to achieve.
        objective: String,
        /// Round cap; `None` takes [`DEFAULT_MAX_GOAL_ROUNDS`].
        max_rounds: Option<u32>,
    },
    /// Rewrite the objective and/or the cap, leaving the phase alone.
    Edit {
        /// New objective, when it changes.
        objective: Option<String>,
        /// New cap, when it changes.
        max_rounds: Option<u32>,
        /// Replace the whole gate list, when it changes. `None` leaves it alone.
        ///
        /// **Replace, not append.** "Add a gate" is `fetch the current list, push, send it`,
        /// and a caller that has to read before it can write cannot lose a gate by racing
        /// itself. An empty list clears them, which is a different request from omitting the
        /// field and has to stay different — otherwise "clear the gates" is unreachable
        /// from a client that omits empty values.
        gates: Option<Vec<GoalGate>>,
    },
    /// Stop automatic continuation and keep the state.
    Pause,
    /// Restart continuation; also clears a former blocker.
    Resume,
    /// Mark it achieved.
    Complete,
    /// Record a concrete blocking condition.
    Blocked {
        /// The blocker, code and reason.
        blocker: GoalBlocker,
    },
    /// Remove the current goal.
    Clear,
}

impl GoalOp {
    /// The word this operation is called on the wire and in a message.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Create { .. } => "create",
            Self::Edit { .. } => "edit",
            Self::Pause => "pause",
            Self::Resume => "resume",
            Self::Complete => "complete",
            Self::Blocked { .. } => "blocked",
            Self::Clear => "clear",
        }
    }
}

/// Why a goal mutation was refused.
///
/// Every variant is a *refusal*, never a crash and never a silent no-op: the M196 rule that a
/// silently swallowed action is indistinguishable from a broken program applies to commands at
/// least as much as to keystrokes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GoalError {
    /// A mutation was asked for with no goal current.
    NoGoal,
    /// The session has no durable store, so there is nowhere to record a goal.
    ///
    /// Distinct from [`GoalError::NoGoal`] on purpose, and the distinction is the whole reason
    /// this variant exists: "no goal is set" is a fact about a session the user can act on, and
    /// "this session cannot hold a goal" is a fact about the *frontend* — a `task` child, a
    /// `--print` run, a test agent. Collapsing them tells a user their objective is unset when
    /// the truth is that nothing they type will ever set it.
    NoStore,
    /// The caller's `{id, revision}` is not the current one.
    Stale {
        /// What the caller believed.
        expected: GoalRef,
        /// What the log actually holds.
        actual: GoalRef,
    },
    /// A create was asked for while an unfinished goal holds the slot.
    AlreadyCurrent {
        /// The goal that has to be paused, completed, blocked or cleared first.
        current: GoalRef,
    },
    /// The round cap is reached, so continuation cannot be rearmed.
    RoundsExhausted {
        /// Rounds attempted.
        rounds_started: u32,
        /// The cap.
        max_rounds: u32,
    },
    /// An edit named nothing to change, or a create named an empty objective.
    Invalid(String),
}

impl std::fmt::Display for GoalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoGoal => write!(f, "no goal is set in this session"),
            Self::NoStore => write!(
                f,
                "this session has no durable store, so a goal cannot be recorded — a goal lives \
                 in the session file, not in memory"
            ),
            Self::Stale { expected, actual } => write!(
                f,
                "the goal moved: this was issued against revision {} of {}, and the session \
                 holds revision {} — re-read it and try again",
                expected.revision, expected.id, actual.revision
            ),
            Self::AlreadyCurrent { current } => write!(
                f,
                "a goal is already current (revision {}); pause, complete, block or clear it \
                 before starting another",
                current.revision
            ),
            Self::RoundsExhausted {
                rounds_started,
                max_rounds,
            } => write!(
                f,
                "the round cap is reached ({rounds_started}/{max_rounds}); edit the goal to \
                 raise it before resuming"
            ),
            Self::Invalid(why) => write!(f, "{why}"),
        }
    }
}

impl std::error::Error for GoalError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(status: GoalStatus, rounds: u32, max: u32) -> GoalSnapshot {
        GoalSnapshot {
            id: "g1".into(),
            revision: 3,
            objective: "make the doc gates pass".into(),
            status,
            max_rounds: max,
            rounds_started: rounds,
            blocker: None,
            gates: Vec::new(),
            created_at_ms: 1,
            updated_at_ms: 2,
        }
    }

    #[test]
    fn only_an_active_armed_goal_under_cap_continues_itself() {
        // The three conditions are independent, and each one alone must stop the loop: a paused
        // goal that is still armed, an active goal in a process that has not rearmed it, and an
        // active armed goal that has spent its budget are three different reasons not to type
        // the next prompt on the user's behalf.
        let active = GoalView {
            goal: snapshot(GoalStatus::Active, 1, 8),
            armed: true,
        };
        assert!(active.is_continuable());

        assert!(
            !GoalView {
                armed: false,
                ..active.clone()
            }
            .is_continuable(),
            "a disarmed goal must not continue, however active it looks"
        );
        assert!(
            !GoalView {
                goal: snapshot(GoalStatus::Paused, 1, 8),
                ..active.clone()
            }
            .is_continuable(),
            "a paused goal must not continue"
        );
        assert!(
            !GoalView {
                goal: snapshot(GoalStatus::Active, 8, 8),
                ..active.clone()
            }
            .is_continuable(),
            "an exhausted goal must not continue"
        );
    }

    #[test]
    fn the_badge_says_the_phase_and_the_progress_and_nothing_else() {
        let armed = GoalView {
            goal: snapshot(GoalStatus::Active, 2, 256),
            armed: true,
        };
        assert_eq!(armed.badge(), "● goal 2/256");
        // The same progress, disarmed: the *only* difference is the mark, which is what tells a
        // reader whether the harness will start the next round on its own.
        let disarmed = GoalView {
            armed: false,
            ..armed.clone()
        };
        assert_eq!(disarmed.badge(), "○ goal 2/256");
        // The phases a mark cannot distinguish say their name.
        let paused = GoalView {
            goal: snapshot(GoalStatus::Paused, 2, 256),
            armed: false,
        };
        assert_eq!(paused.badge(), "○ goal paused 2/256");
        let blocked = GoalView {
            goal: snapshot(GoalStatus::Blocked, 2, 256),
            armed: false,
        };
        assert_eq!(blocked.badge(), "! goal blocked 2/256");
        let complete = GoalView {
            goal: snapshot(GoalStatus::Complete, 3, 9),
            armed: true,
        };
        assert_eq!(
            complete.badge(),
            "✓ goal complete 3/9",
            "a complete goal is never armed, whatever the flag says"
        );
        // The objective is not on the row: it is a sentence, not a status.
        assert!(!armed.badge().contains("doc gates"));
    }

    #[test]
    fn a_snapshot_without_the_newer_fields_still_decodes() {
        // The M202 rule, asserted where it applies: a goal written by a build that had no
        // `blocker` or `rounds_started` must read, because the alternative is a session that
        // refuses to open over a *derived* view.
        let json = r#"{
            "id": "g1",
            "revision": 1,
            "objective": "ship it",
            "status": "active",
            "created_at_ms": 5,
            "updated_at_ms": 5
        }"#;
        let goal: GoalSnapshot = serde_json::from_str(json).expect("an older snapshot must read");
        assert_eq!(goal.max_rounds, DEFAULT_MAX_GOAL_ROUNDS);
        assert_eq!(goal.rounds_started, 0);
        assert_eq!(goal.blocker, None);
    }

    #[test]
    fn a_refusal_says_what_to_do_about_it() {
        // Each message names the state that refused it. A refusal a reader cannot act on is the
        // defect class this repository keeps paying for (M75's diagnoses, M202's field names).
        let stale = GoalError::Stale {
            expected: GoalRef {
                id: "g1".into(),
                revision: 2,
            },
            actual: GoalRef {
                id: "g1".into(),
                revision: 5,
            },
        };
        let text = stale.to_string();
        assert!(
            text.contains("revision 2") && text.contains("revision 5"),
            "{text}"
        );
        assert!(text.contains("re-read"), "{text}");
        assert!(GoalError::NoGoal.to_string().contains("no goal"));
    }
}
