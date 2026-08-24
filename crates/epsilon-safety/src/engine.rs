//! The ruleset engine: stacked layers, last-match-wins evaluation.

use serde::{Deserialize, Serialize};

use crate::wildcards::glob_match;

/// What a rule decides.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    /// Proceed without consulting anyone.
    Allow,
    /// Route through the approval UI.
    Ask,
    /// Refuse.
    Deny,
}

/// One permission rule. `capability` is a tool name (`read`, `write`, `bash`, …) or
/// pseudo-capability (`external_dir`, `doom_loop`); `pattern` is a glob over the call's
/// subject (path / command head / `*`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rule {
    /// Capability this rule governs; `*` matches all capabilities.
    pub capability: String,
    /// Glob matched against the call subject; `*` matches every subject.
    pub pattern: String,
    /// What happens on match.
    pub action: Action,
}

impl Rule {
    /// Convenience constructor.
    pub fn new(capability: &str, pattern: &str, action: Action) -> Self {
        Self {
            capability: capability.to_string(),
            pattern: pattern.to_string(),
            action,
        }
    }

    fn matches(&self, capability: &str, subject: &str) -> bool {
        (self.capability == "*" || self.capability == capability)
            && glob_match(&self.pattern, subject)
    }
}

/// A named set of rules forming one layer (defaults / global config / project / CLI).
#[derive(Debug, Clone)]
pub struct Ruleset {
    /// Layer name for diagnostics (`"defaults"`, `"project"`…).
    pub name: String,
    /// Rules in priority order (later rules in the layer override earlier ones).
    pub rules: Vec<Rule>,
}

/// The engine: flattened evaluation across layers, later layers winning.
///
/// This mirrors opencode's `rulesets.flat().findLast(...)` with an important addition:
/// **any deny wins immediately** regardless of position. A project cannot accidentally
/// re-allow something the global config denied — security decisions are order-independent
/// (the DSH monotonic-guard principle applied to rule stacking).
pub struct PermissionEngine {
    layers: Vec<Ruleset>,
}

impl PermissionEngine {
    /// Engine with only the built-in defaults.
    pub fn with_defaults() -> Self {
        Self {
            layers: vec![default_ruleset()],
        }
    }

    /// Engine with defaults plus extra layers, evaluated after them (later wins).
    pub fn layered(extra: Vec<Ruleset>) -> Self {
        let mut layers = vec![default_ruleset()];
        layers.extend(extra);
        Self { layers }
    }

    /// Evaluate a capability/subject pair.
    pub fn evaluate(&self, capability: &str, subject: &str) -> Action {
        let mut decision: Option<Action> = None;
        for layer in &self.layers {
            for rule in &layer.rules {
                if !rule.matches(capability, subject) {
                    continue;
                }
                if rule.action == Action::Deny {
                    // Monotonic deny: immediate, order-independent refusal.
                    return Action::Deny;
                }
                decision = Some(rule.action);
            }
        }
        decision.unwrap_or(Action::Ask)
    }
}

/// Built-in defaults. Philosophy (opencode-grade maturity, sandbox-aware posture):
/// reads are free inside the workspace shape we can see; secrets ask; writes allow because
/// workspace enforcement arrives with the M10 confinement layer (documented gap); shell asks
/// by default because Layer 2 does not exist yet — safe until proven otherwise.
pub fn default_ruleset() -> Ruleset {
    Ruleset {
        name: "defaults".into(),
        rules: vec![
            Rule::new("read", "*", Action::Allow),
            Rule::new("grep", "*", Action::Allow),
            Rule::new("glob", "*", Action::Allow),
            Rule::new("ls", "*", Action::Allow),
            // Secrets: reading environment material requires approval…
            Rule::new("read", "*.env", Action::Ask),
            Rule::new("read", "*.env.*", Action::Ask),
            // …but examples/templates are public by design.
            Rule::new("read", "*.env.example", Action::Allow),
            Rule::new("write", "*", Action::Allow),
            Rule::new("edit", "*", Action::Allow),
            // Anything outside the declared workspace asks (tools downgrade capability).
            Rule::new("external_dir", "*", Action::Ask),
            // Shell and network: ask-first until confinement exists.
            Rule::new("bash", "*", Action::Ask),
            Rule::new("webfetch", "*", Action::Ask),
        ],
    }
}

/// Plan-mode ruleset (opencode's `plan` agent pattern): read-only research.
///
/// Mutating capabilities are DENIED — not asked — because a plan run must never touch
/// the tree even if the user approves mid-run; the correct escape is to exit plan mode,
/// which keeps the decision out of per-call approval fatigue. Note the engine's
/// monotonic-deny semantics make this boundary hold no matter what later layers allow.
/// Internal session bookkeeping (compaction summaries, JSONL appends) does not travel
/// through the tool gate at all, so long planning runs still persist their log.
pub fn plan_ruleset() -> Ruleset {
    Ruleset {
        name: "plan".into(),
        rules: vec![
            Rule::new("write", "*", Action::Deny),
            Rule::new("edit", "*", Action::Deny),
            Rule::new("bash", "git commit*", Action::Deny),
            Rule::new("bash", "git push*", Action::Deny),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn last_match_wins_within_a_layer() {
        let engine = PermissionEngine::with_defaults();
        assert_eq!(engine.evaluate("read", "src/main.rs"), Action::Allow);
        assert_eq!(engine.evaluate("read", ".env"), Action::Ask);
        assert_eq!(engine.evaluate("read", ".env.example"), Action::Allow);
    }

    #[test]
    fn deny_is_monotonic_across_layers() {
        // A later layer cannot re-allow what an earlier layer denied.
        let engine = PermissionEngine::layered(vec![
            Ruleset {
                name: "g".into(),
                rules: vec![Rule::new("bash", "*", Action::Deny)],
            },
            Ruleset {
                name: "project".into(),
                rules: vec![Rule::new("bash", "cargo", Action::Allow)],
            },
        ]);
        assert_eq!(engine.evaluate("bash", "cargo"), Action::Deny);
    }

    #[test]
    fn unknown_capabilities_ask_by_default() {
        assert_eq!(
            PermissionEngine::with_defaults().evaluate("mystery_tool", "*"),
            Action::Ask
        );
    }

    #[test]
    fn later_layers_override_allows() {
        let engine = PermissionEngine::layered(vec![Ruleset {
            name: "cli".into(),
            rules: vec![Rule::new("bash", "*", Action::Allow)],
        }]);
        assert_eq!(engine.evaluate("bash", "git"), Action::Allow);
        assert_eq!(engine.evaluate("webfetch", "*"), Action::Ask); // untouched default
    }

    #[test]
    fn plan_mode_denies_mutations_even_when_project_reallows() {
        // opencode plan-agent parity: research runs cannot touch the tree, and the
        // monotonic-deny rule means no lower layer can punch through.
        let engine = PermissionEngine::layered(vec![
            Ruleset {
                name: "plan".into(),
                rules: crate::plan_ruleset().rules,
            },
            Ruleset {
                name: "project".into(),
                rules: vec![Rule::new("write", "*", Action::Allow)],
            },
        ]);
        assert_eq!(engine.evaluate("write", "src/main.rs"), Action::Deny);
        assert_eq!(engine.evaluate("edit", "src/main.rs"), Action::Deny);
        // Reads and search stay free — planning is a research activity.
        assert_eq!(engine.evaluate("read", "src/main.rs"), Action::Allow);
        assert_eq!(engine.evaluate("grep", "*"), Action::Allow);
    }

    #[test]
    fn plan_mode_blocks_commit_and_push_commands() {
        let engine = PermissionEngine::layered(vec![Ruleset {
            name: "plan".into(),
            rules: crate::plan_ruleset().rules,
        }]);
        assert_eq!(engine.evaluate("bash", "git commit -m x"), Action::Deny);
        assert_eq!(
            engine.evaluate("bash", "git push origin main"),
            Action::Deny
        );
        // Ordinary shell still follows default ask posture (not silently allowed).
        assert_eq!(engine.evaluate("bash", "cargo test"), Action::Ask);
    }
}
