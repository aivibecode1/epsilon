//! Permission rulesets and evaluation — Layer 1 of Epsilon's two-layer safety model.
//!
//! Semantics ported from opencode: flat rules of `{capability, pattern, action}`, evaluated
//! **last-match-wins** across stacked layers (defaults <- global <- project <- CLI). Any deny
//! fails immediately at evaluation time; `ask` routes through an [`ApprovalUi`] so frontends
//! own the interaction.

mod channel_ui;
mod engine;
pub mod env;
mod escalation;
mod gate;
pub mod sandbox;
mod wildcards;

pub use channel_ui::{ChannelApprovalUi, PendingAsk};
pub use engine::{Action, PermissionEngine, Preset, Rule, Ruleset, plan_ruleset, preset_ruleset};
pub use env::{is_secret_env_name, scrub_secret_env, secret_env_names};
pub use escalation::{
    EscalationError, EscalationRequest, SandboxMode, denial_marker, escalation_hint,
    escalation_reason, escalation_target, validate_escalation_args,
};
pub use gate::{ApprovalUi, AutoApprove, AutoDeny, RulesetGate};
#[cfg(unix)]
pub use sandbox::BwrapSandbox;
#[cfg(windows)]
pub use sandbox::JobObjectSandbox;
pub use sandbox::{
    ConfinedCommand, ConfinedOutcome, ConfinementPolicy, Enforcement, NullSandbox, OutputSink,
    Sandbox, SandboxError, SharedBuf, best_available, drain_pipe, new_shared_buf, snapshot,
};
pub use wildcards::glob_match;
