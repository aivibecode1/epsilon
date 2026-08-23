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
