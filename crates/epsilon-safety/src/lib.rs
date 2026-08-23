//! Permission rulesets and evaluation — Layer 1 of Epsilon's two-layer safety model.
//!
//! Semantics ported from opencode: flat rules of `{capability, pattern, action}`, evaluated
//! **last-match-wins** across stacked layers (defaults <- global <- project <- CLI). Any deny
//! fails immediately at evaluation time; `ask` routes through an [`ApprovalUi`] so frontends
//! own the interaction.

mod engine;
mod gate;
mod wildcards;

pub use engine::{Action, PermissionEngine, Rule, Ruleset};
pub use gate::{ApprovalUi, AutoApprove, RulesetGate};
pub use wildcards::glob_match;
