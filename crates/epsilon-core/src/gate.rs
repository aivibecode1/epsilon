//! The permission gate seam: the loop asks, a policy decides.
//!
//! V1 ships trivial policies; the real ruleset engine (opencode-style wildcards) lands in M6.
//! The seam exists NOW so the loop's ask-points are fixed and testable.

use async_trait::async_trait;

use epsilon_contracts::agent_event::{PermissionDecision, PermissionRequest};

/// Decides whether an action may proceed. Implementations must be cheap and non-blocking
/// beyond their own UI latency.
#[async_trait]
pub trait PermissionGate: Send + Sync {
    /// Decide one request.
    async fn decide(&self, req: PermissionRequest) -> PermissionDecision;
}

/// Allows everything — for tests and trusted automation only.
pub struct AutoAllow;

#[async_trait]
impl PermissionGate for AutoAllow {
    async fn decide(&self, _req: PermissionRequest) -> PermissionDecision {
        PermissionDecision::AllowOnce
    }
}

/// Denies everything with fixed guidance text fed back to the model.
pub struct AutoDeny {
    /// Guidance returned in the denial result so the model can adapt.
    pub guidance: String,
}

#[async_trait]
impl PermissionGate for AutoDeny {
    async fn decide(&self, _req: PermissionRequest) -> PermissionDecision {
        PermissionDecision::Deny
    }
}
