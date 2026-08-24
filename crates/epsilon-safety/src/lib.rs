//! Permission rulesets and evaluation — Layer 1 of Epsilon's two-layer safety model.
//!
//! Semantics ported from opencode: flat rules of `{capability, pattern, action}`, evaluated
//! **last-match-wins** across stacked layers (defaults <- global <- project <- CLI). Any deny
//! fails immediately at evaluation time; `ask` routes through an [`ApprovalUi`] so frontends
//! own the interaction.

mod channel_ui;
mod engine;
mod gate;
pub mod sandbox;
mod wildcards;

pub use channel_ui::{ChannelApprovalUi, PendingAsk};
pub use engine::{Action, PermissionEngine, Rule, Ruleset, plan_ruleset};
pub use gate::{ApprovalUi, AutoApprove, AutoDeny, RulesetGate};
#[cfg(unix)]
pub use sandbox::BwrapSandbox;
#[cfg(windows)]
pub use sandbox::JobObjectSandbox;
pub use sandbox::{
    ConfinedCommand, ConfinedOutcome, ConfinementPolicy, NullSandbox, Sandbox, SandboxError,
    best_available,
};
pub use wildcards::glob_match;
