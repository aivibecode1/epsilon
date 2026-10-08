//! The permission gate seam: the loop asks, a policy decides.
//!
//! V1 policies here are trivial; the real ruleset engine lives in `epsilon-safety`
//! (`RulesetGate`) and implements this same trait. The seam shape is fixed NOW so loop
//! call-sites never change as policies mature.

use async_trait::async_trait;

use epsilon_contracts::agent_event::{PermissionDecision, PermissionRequest};

/// Outcome of one authorization check.
///
/// `asked` tells the loop whether a human was consulted, so permission dialogs/events fire
/// exactly once per real ask instead of flooding on silent allows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Authorization {
    /// What was decided.
    pub decision: PermissionDecision,
    /// True when an interactive approval (or its automation stand-in) was consulted.
    pub asked: bool,
}

impl Authorization {
    /// Silent allow helper.
    pub fn allow_once() -> Self {
        Self {
            decision: PermissionDecision::AllowOnce,
            asked: false,
        }
    }

    /// Silent deny helper.
    pub fn denied(guidance: impl Into<String>) -> Self {
        Self {
            decision: PermissionDecision::DenyWithGuidance {
                guidance: guidance.into(),
            },
            asked: false,
        }
    }

    /// True when the call may proceed.
    pub fn allowed(&self) -> bool {
        !self.decision.is_denial()
    }
}

/// Decides whether an action may proceed. Implementations must be cheap and non-blocking
/// beyond their own UI latency.
#[async_trait]
pub trait PermissionGate: Send + Sync {
    /// Authorize one request; report whether asking happened.
    async fn authorize(&self, req: PermissionRequest) -> Authorization;

    /// The gate a **subagent** should use, or `None` to share `self`.
    ///
    /// # Why this exists
    ///
    /// A subagent is built with a gate, and the obvious thing — handing it the parent's — is
    /// wrong in a way nothing could see. `RulesetGate` keeps the session's "always allow"
    /// decisions *inside* the gate, so sharing it shares them: one `always allow bash` in the
    /// main loop silently authorised `bash` for every subagent afterwards, with no prompt and no
    /// inheritance decision anywhere. Measured on 2026-10-04 as a live path
    /// (`build_task_child` → `delegate_task` → `exec.rs:452`).
    ///
    /// The distinction being drawn is **policy vs. conversation**. A *ruleset* is what the owner
    /// wrote and is the same for every agent; a *session memory* is a decision someone made while
    /// talking to one of them. Subagents inherit the first and not the second — which is
    /// exactly opencode's rule, verified at
    /// `references/opencode-1.18.34/packages/opencode/src/agent/subagent-permissions.ts:21-25`.
    ///
    /// Defaulted to `None` (share self) so every existing implementation — `AutoAllow`,
    /// `AutoDeny`, and any test's own gate — compiles unchanged and keeps its current behaviour.
    /// A gate with no notion of session state has nothing to withhold.
    fn child_gate(&self) -> Option<std::sync::Arc<dyn PermissionGate>> {
        None
    }

    /// Switch the named permission posture mid-session (ADR-032), or `false`
    /// when this gate has no notion of presets.
    ///
    /// Defaulted to `false` so fixed-posture gates (`AutoAllow`, `AutoDeny`,
    /// test doubles) compile unchanged — and correctly, since a gate that
    /// allows everything has nothing to switch. `RulesetGate` overrides it:
    /// it parses the name, swaps its engine, and clears the session memory
    /// (a promotion under the old policy must not survive the switch,
    /// whichever direction it goes).
    ///
    /// Takes the preset *name*, not the enum, because the enum lives in
    /// `epsilon-safety` and this crate cannot name it without a dependency
    /// cycle (`safety` implements this trait). Front ends validate with
    /// `Preset::parse` before calling.
    fn set_preset(&self, _preset: &str, _workspace: &std::path::Path) -> bool {
        false
    }

    /// The preset name last applied, or `"unknown"` for gates with no notion
    /// of presets. What bare `/preset` reports.
    fn current_preset(&self) -> String {
        "unknown".to_string()
    }
}

/// Allows everything without asking — for tests and trusted automation only.
pub struct AutoAllow;

#[async_trait]
impl PermissionGate for AutoAllow {
    async fn authorize(&self, _req: PermissionRequest) -> Authorization {
        Authorization::allow_once()
    }
}

/// Denies everything with fixed guidance text fed back to the model.
pub struct AutoDeny {
    /// Guidance returned in the denial result so the model can adapt.
    pub guidance: String,
}

#[async_trait]
impl PermissionGate for AutoDeny {
    async fn authorize(&self, _req: PermissionRequest) -> Authorization {
        Authorization {
            decision: PermissionDecision::Deny,
            asked: false,
        }
    }
}
