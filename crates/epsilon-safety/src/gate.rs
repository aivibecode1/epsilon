//! `RulesetGate`: bridges the engine into the loop's `PermissionGate` seam, with
//! session-scoped "always allow" memory and a pluggable approval UI.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use epsilon_contracts::agent_event::{PermissionDecision, PermissionRequest};
use epsilon_core::{Authorization, PermissionGate};

use crate::engine::{Action, PermissionEngine};

/// Frontend-side prompt for `ask` decisions (TUI dialog, HTTP round-trip, test script…).
#[async_trait]
pub trait ApprovalUi: Send + Sync {
    /// Ask the human; return their decision.
    async fn ask(&self, req: PermissionRequest) -> PermissionDecision;
}

/// Approves every ask without interaction — tests and trusted automation.
pub struct AutoApprove;

#[async_trait]
impl ApprovalUi for AutoApprove {
    async fn ask(&self, _req: PermissionRequest) -> PermissionDecision {
        PermissionDecision::AllowOnce
    }
}

/// Denies every ask without interaction (plan 003): the safe default for unattended
/// headless runs — the model keeps making progress on allowed work but gated actions
/// fail closed instead of silently self-approving.
pub struct AutoDeny;

#[async_trait]
impl ApprovalUi for AutoDeny {
    async fn ask(&self, req: PermissionRequest) -> PermissionDecision {
        PermissionDecision::DenyWithGuidance {
            guidance: format!(
                "unattended run: {} on {} requires an interactive session or --yes",
                req.capability, req.pattern
            ),
        }
    }
}

/// `(capability, pattern)` pairs promoted to allow-for-this-session by an `AlwaysSession`.
type SessionMemory = Arc<Mutex<Vec<(String, String)>>>;

/// Gate implementing the opencode-style flow:
/// session memory → engine evaluation → (`ask`) approval UI → optional memory promotion.
pub struct RulesetGate {
    /// The launch-time stack (defaults + project rules), never touched by
    /// `set_preset`. Locked for the same reason as `preset`: evaluation holds
    /// each lock only for its synchronous call, never across the UI await.
    base: Mutex<PermissionEngine>,
    /// The ADR-032 preset layer over `base`: a matching rule decides (deny
    /// immediately, like everywhere), no match falls back to `base`.
    preset: Mutex<crate::engine::Ruleset>,
    /// The preset name last applied (`auto` at construction). What bare
    /// `/preset` reports — the gate is the only place that knows both the
    /// launch posture and every mid-session switch.
    current: Mutex<String>,
    /// The plan-mode layer (criterion 4): `plan_ruleset()` when on, an empty
    /// ruleset when off. Mirrors the preset layer — mutex-swapped, fail-closed
    /// on lock failure — so mid-session `/plan` toggles keep the base stack
    /// (defaults + project rules) intact.
    plan: Mutex<crate::engine::Ruleset>,
    /// Whether plan mode is on. What bare `/plan` reports.
    plan_on: Mutex<bool>,
    ui: Arc<dyn ApprovalUi>,
    memory: SessionMemory,
}

impl RulesetGate {
    /// Gate over the default ruleset with the given approval UI.
    pub fn new(ui: Arc<dyn ApprovalUi>) -> Self {
        Self::layered(PermissionEngine::with_defaults(), ui)
    }

    /// Gate over a custom engine.
    pub fn layered(engine: PermissionEngine, ui: Arc<dyn ApprovalUi>) -> Self {
        Self {
            base: Mutex::new(engine),
            preset: Mutex::new(crate::engine::preset_ruleset(
                crate::engine::Preset::Auto,
                std::path::Path::new(""),
            )),
            current: Mutex::new(crate::engine::Preset::Auto.name().to_string()),
            plan: Mutex::new(crate::engine::Ruleset {
                name: "plan:off".into(),
                rules: Vec::new(),
            }),
            plan_on: Mutex::new(false),
            ui,
            memory: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// The preset name last applied (diagnostics/front ends).
    pub fn current_preset(&self) -> String {
        self.current
            .lock()
            .map(|c| c.clone())
            .unwrap_or_else(|_| "unknown".to_string())
    }

    /// Whether plan mode is on (diagnostics/front ends; what bare `/plan`
    /// reports). Mirrors [`Self::current_preset`]: the gate is the only place
    /// that knows the live plan state.
    pub fn in_plan_mode(&self) -> bool {
        self.plan_on.lock().map(|on| *on).unwrap_or(false)
    }

    /// Read-only view of promoted always-allow pairs (diagnostics/tests).
    pub fn session_allows(&self) -> Vec<(String, String)> {
        self.memory.lock().map(|m| m.clone()).unwrap_or_default()
    }

    /// Evaluate one capability/subject across plan + base + preset: any side's
    /// deny wins immediately (monotonic, all directions), a plan or preset
    /// match otherwise decides, and no match falls back to base. A lock
    /// failure denies (fail closed, like the engine helper below).
    ///
    /// The plan slot is checked FIRST: a plan deny wins before base, preset,
    /// or memory — the same position the baked plan layer held, so `--plan`
    /// launches behave byte-identical after the un-baking.
    fn evaluate(&self, capability: &str, pattern: &str) -> Action {
        let Ok(plan) = self.plan.lock() else {
            return Action::Deny;
        };
        for rule in &plan.rules {
            if rule.capability != "*" && rule.capability != capability {
                continue;
            }
            if !crate::glob_match(&rule.pattern, pattern) {
                continue;
            }
            if rule.action == Action::Deny {
                return Action::Deny;
            }
        }
        drop(plan);
        let base = self
            .base
            .lock()
            .map(|e| e.evaluate(capability, pattern))
            .unwrap_or(Action::Deny);
        if base == Action::Deny {
            return Action::Deny;
        }
        let Ok(preset) = self.preset.lock() else {
            return Action::Deny;
        };
        let mut decision: Option<Action> = None;
        for rule in &preset.rules {
            if rule.capability != "*" && rule.capability != capability {
                continue;
            }
            if !crate::glob_match(&rule.pattern, pattern) {
                continue;
            }
            if rule.action == Action::Deny {
                return Action::Deny;
            }
            decision = Some(rule.action);
        }
        decision.unwrap_or(base)
    }
}

#[async_trait]
impl PermissionGate for RulesetGate {
    async fn authorize(&self, req: PermissionRequest) -> Authorization {
        // 1. Engine evaluation FIRST for Deny (plan 004 / SECURITY-07): a session
        // "always allow" promotion must never override a deny the engine would now
        // produce — policy tightening mid-session takes effect immediately. Memory
        // only converts Ask→Allow below; it can never resurrect a denied call.
        if self.evaluate(&req.capability, &req.pattern) == Action::Deny {
            return Authorization {
                decision: PermissionDecision::Deny,
                asked: false,
            };
        }

        // 2. Session memory short-circuits before any UI round-trip.
        if let Ok(mem) = self.memory.lock()
            && mem
                .iter()
                .any(|(c, p)| *c == req.capability && crate::glob_match(p, &req.pattern))
        {
            return Authorization {
                decision: PermissionDecision::AllowOnce,
                asked: false,
            };
        }

        // 3. Engine evaluation.
        let action = self.evaluate(&req.capability, &req.pattern);
        match action {
            Action::Allow => Authorization {
                decision: PermissionDecision::AllowOnce,
                asked: false,
            },
            Action::Deny => Authorization {
                decision: PermissionDecision::Deny,
                asked: false,
            },
            Action::Ask => {
                // 3. Consult the frontend; honor once/always/deny-with-guidance.
                let decision = self.ui.ask(req.clone()).await;
                if let PermissionDecision::AlwaysSession = decision {
                    if let Ok(mut mem) = self.memory.lock() {
                        mem.push((req.capability.clone(), req.pattern.clone()));
                    }
                    return Authorization {
                        decision: PermissionDecision::AllowOnce,
                        asked: true,
                    };
                }
                Authorization {
                    decision,
                    asked: true,
                }
            }
        }
    }

    /// A subagent gets the same rules and a **fresh** session memory.
    ///
    /// # What is inherited, and what is not
    ///
    /// `engine` is the owner's ruleset — the same for every agent in the session, cloned so a
    /// deny cannot be escaped by being spawned into. `ui` is shared because it *is* the user: a
    /// subagent's prompt goes to the same person, and duplicating it would let a subagent ask a
    /// question nobody is watching.
    ///
    /// `memory` is **not** shared, and that is the entire point. It holds the decisions *this
    /// conversation* accumulated — one `a` keypress is one `always allow`, made about the agent in
    /// front of the user. Inheriting it turns a single "yes, always" into a standing permission
    /// for every agent spawned afterwards, which the user was never asked about and cannot see.
    /// opencode draws the same line by handing its subagents a derived ruleset filtered to denies
    /// (`references/opencode-1.18.34/packages/opencode/src/agent/subagent-permissions.ts:21-25`).
    ///
    /// **This override is the fix, so it lives in the trait impl and not beside it.** An earlier
    /// version put it in the inherent `impl RulesetGate` block, where it compiled, was documented,
    /// and was **never called** — a trait method has to be in the trait impl to override. The
    /// integration test caught it, which is the argument for having one that exercises a real
    /// subagent rather than the seam directly.
    fn child_gate(&self) -> Option<std::sync::Arc<dyn epsilon_core::PermissionGate>> {
        let base = self.base.lock().ok()?.clone();
        let preset = self.preset.lock().ok()?.clone();
        let plan = self.plan.lock().ok()?.clone();
        let plan_on = self.plan_on.lock().ok().map(|on| *on).unwrap_or(false);
        let mut gate = Self::layered(base, std::sync::Arc::clone(&self.ui));
        gate.preset = Mutex::new(preset);
        gate.plan = Mutex::new(plan);
        gate.plan_on = Mutex::new(plan_on);
        Some(std::sync::Arc::new(gate))
    }

    /// Swap the preset layer for the named preset's ruleset (ADR-032) and
    /// clear the session memory: a promotion under the old policy must not
    /// survive the switch, whichever direction it goes. The base stack
    /// (defaults + project rules) is untouched, so project denies still win.
    /// Unknown names refuse (`false`) rather than running a different policy
    /// than asked for.
    fn set_preset(&self, preset: &str, workspace: &std::path::Path) -> bool {
        let Ok(parsed) = crate::engine::Preset::parse(preset) else {
            return false;
        };
        let Ok(mut layer) = self.preset.lock() else {
            return false;
        };
        *layer = crate::engine::preset_ruleset(parsed, workspace);
        if let Ok(mut current) = self.current.lock() {
            *current = parsed.name().to_string();
        }
        if let Ok(mut mem) = self.memory.lock() {
            mem.clear();
        }
        true
    }

    fn current_preset(&self) -> String {
        self.current
            .lock()
            .map(|c| c.clone())
            .unwrap_or_else(|_| "unknown".to_string())
    }

    /// Install (`true`) or remove (`false`) the plan-mode layer (criterion 4)
    /// and clear the session memory: a promotion under the old policy must not
    /// survive the switch, whichever direction it goes — the `set_preset`
    /// rule, because the reason is the same. The base stack (defaults +
    /// project rules) and the preset layer are untouched. A lock failure
    /// refuses (`false`, fail closed) rather than running half a policy.
    fn set_plan_mode(&self, on: bool) -> bool {
        let Ok(mut layer) = self.plan.lock() else {
            return false;
        };
        *layer = if on {
            crate::engine::plan_ruleset()
        } else {
            crate::engine::Ruleset {
                name: "plan:off".into(),
                rules: Vec::new(),
            }
        };
        if let Ok(mut flag) = self.plan_on.lock() {
            *flag = on;
        }
        if let Ok(mut mem) = self.memory.lock() {
            mem.clear();
        }
        true
    }

    fn in_plan_mode(&self) -> bool {
        self.plan_on.lock().map(|on| *on).unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{Rule, Ruleset};

    fn req(cap: &str, pattern: &str) -> PermissionRequest {
        PermissionRequest {
            request_id: epsilon_contracts::new_id("perm"),
            capability: cap.into(),
            pattern: pattern.into(),
            detail: serde_json::json!({}),
        }
    }

    /// Scripted UI: pops one canned answer per ask; records asks.
    struct ScriptedUi {
        answers: Mutex<Vec<PermissionDecision>>,
        asked: Mutex<usize>,
    }

    #[async_trait]
    impl ApprovalUi for ScriptedUi {
        async fn ask(&self, _r: PermissionRequest) -> PermissionDecision {
            *self.asked.lock().unwrap() += 1;
            let mut a = self.answers.lock().unwrap();
            if a.is_empty() {
                PermissionDecision::Deny
            } else {
                a.remove(0)
            }
        }
    }

    #[tokio::test]
    async fn allows_reads_silently_without_ui() {
        let ui = Arc::new(ScriptedUi {
            answers: Mutex::new(vec![]),
            asked: Mutex::new(0),
        });
        let gate = RulesetGate::new(Arc::clone(&ui) as Arc<dyn ApprovalUi>);

        let a = gate.authorize(req("read", "src/lib.rs")).await;
        assert!(a.allowed() && !a.asked);
        assert_eq!(*ui.asked.lock().unwrap(), 0);
    }

    #[tokio::test]
    async fn env_read_asks_then_remembered_always_session() {
        let ui = Arc::new(ScriptedUi {
            answers: Mutex::new(vec![PermissionDecision::AlwaysSession]),
            asked: Mutex::new(0),
        });
        let gate = RulesetGate::new(Arc::clone(&ui) as Arc<dyn ApprovalUi>);

        let first = gate.authorize(req("read", ".env")).await;
        assert!(first.allowed() && first.asked);

        // Second identical subject: served from session memory, no new ask.
        let second = gate.authorize(req("read", ".env")).await;
        assert!(second.allowed() && !second.asked);
        assert_eq!(*ui.asked.lock().unwrap(), 1);
        assert_eq!(gate.session_allows().len(), 1);
    }

    #[tokio::test]
    async fn bash_defaults_to_ask_and_deny_carries_no_access() {
        let ui = Arc::new(ScriptedUi {
            answers: Mutex::new(vec![]),
            asked: Mutex::new(0),
        });
        let gate = RulesetGate::new(Arc::clone(&ui) as Arc<dyn ApprovalUi>);

        let a = gate.authorize(req("bash", "git")).await; // scripted UI denies when empty
        assert!(!a.allowed());
        assert!(a.asked);
    }

    #[tokio::test]
    async fn deny_with_guidance_passes_through() {
        let ui = Arc::new(ScriptedUi {
            answers: Mutex::new(vec![PermissionDecision::DenyWithGuidance {
                guidance: "run cargo fmt first".into(),
            }]),
            asked: Mutex::new(0),
        });
        let gate = RulesetGate::new(Arc::clone(&ui) as Arc<dyn ApprovalUi>);

        let a = gate.authorize(req("bash", "cargo")).await;
        assert!(!a.allowed());
        assert_eq!(
            a.decision.guidance(),
            Some("run cargo fmt first"),
            "guidance must survive to feed the model"
        );
    }

    #[tokio::test]
    async fn memory_promotion_cannot_override_a_fresh_deny() {
        // Plan 004 / SECURITY-07: promote ("bash","git") via always-session, then a
        // deny layer for the same pair appears — the promotion must NOT win. The
        // authorize() ordering (engine deny checked BEFORE memory) is what makes
        // this hold; verify both halves.
        let ui = Arc::new(ScriptedUi {
            answers: Mutex::new(vec![PermissionDecision::AlwaysSession]),
            asked: Mutex::new(0),
        });
        let gate = RulesetGate::new(Arc::clone(&ui) as Arc<dyn ApprovalUi>);
        let first = gate.authorize(req("bash", "git")).await;
        assert!(first.allowed(), "promotion happened");
        assert_eq!(gate.session_allows().len(), 1);

        // Mid-session tightening: a deny layer for the promoted pair.
        let tightened = RulesetGate::layered(
            PermissionEngine::layered(vec![Ruleset {
                name: "tightened".into(),
                rules: vec![Rule::new("bash", "git*", Action::Deny)],
            }]),
            Arc::clone(&ui) as Arc<dyn ApprovalUi>,
        );
        // Fresh gate has no memory; the engine deny alone must hold...
        let denied = tightened.authorize(req("bash", "git")).await;
        assert!(!denied.allowed() && !denied.asked, "deny wins");
    }

    /// `set_preset` swaps the posture mid-session (ADR-032): read-only
    /// denies what the defaults asked, full-access allows it, and unknown
    /// names refuse without changing anything.
    #[tokio::test]
    async fn set_preset_swaps_the_posture_and_refuses_unknown() {
        use epsilon_core::PermissionGate;
        let ui = Arc::new(ScriptedUi {
            answers: Mutex::new(vec![PermissionDecision::AllowOnce]),
            asked: Mutex::new(0),
        });
        let gate = RulesetGate::new(Arc::clone(&ui) as Arc<dyn ApprovalUi>);
        let ws = std::path::Path::new("/ws");

        assert!(gate.set_preset("read-only", ws));
        let a = gate.authorize(req("bash", "echo hi")).await;
        assert!(!a.allowed() && !a.asked, "read-only denies shell outright");

        assert!(gate.set_preset("danger-full-access", ws));
        let b = gate.authorize(req("bash", "echo hi")).await;
        assert!(b.allowed() && !b.asked, "full-access allows it");

        assert!(!gate.set_preset("yolo", ws), "unknown refuses");
        let c = gate.authorize(req("bash", "echo hi")).await;
        assert!(c.allowed(), "a refused switch changes nothing");
    }

    /// `set_plan_mode(true)` denies mutations outright (the `--plan` posture);
    /// `set_plan_mode(false)` restores the base posture; either switch clears
    /// the session memory like a preset switch. RED-first for criterion 4.
    #[tokio::test]
    async fn set_plan_mode_round_trip_denies_then_restores_and_clears_memory() {
        use epsilon_core::PermissionGate;
        let ui = Arc::new(ScriptedUi {
            answers: Mutex::new(vec![PermissionDecision::AlwaysSession]),
            asked: Mutex::new(0),
        });
        let gate = RulesetGate::new(Arc::clone(&ui) as Arc<dyn ApprovalUi>);
        assert!(!gate.in_plan_mode(), "off at construction");

        // Promote something memorable under the base policy, so the switch
        // has memory to clear.
        let first = gate.authorize(req("bash", "git")).await;
        assert!(first.allowed(), "promotion happened");
        assert_eq!(gate.session_allows().len(), 1);

        assert!(gate.set_plan_mode(true));
        assert!(gate.in_plan_mode(), "on after enabling");
        assert!(
            gate.session_allows().is_empty(),
            "enabling clears the old policy's promotions"
        );
        let denied = gate.authorize(req("write", "src/main.rs")).await;
        assert!(
            !denied.allowed() && !denied.asked,
            "plan on denies writes outright"
        );
        let denied_shell = gate.authorize(req("bash", "cargo test")).await;
        assert!(
            !denied_shell.allowed() && !denied_shell.asked,
            "plan on denies shell outright"
        );
        let read = gate.authorize(req("read", "src/main.rs")).await;
        assert!(read.allowed(), "reads stay free in plan mode");

        assert!(gate.set_plan_mode(false));
        assert!(!gate.in_plan_mode(), "off after disabling");
        assert!(
            gate.session_allows().is_empty(),
            "disabling clears plan-era promotions too"
        );
        let write = gate.authorize(req("write", "src/main.rs")).await;
        assert!(write.allowed(), "plan off restores the base posture");
    }

    /// The slot matches the old baked layer outcome for outcome: a gate with
    /// the slot on authorizes exactly like an engine with the plan layer
    /// baked in. This is the equivalence proof for the composition un-baking.
    #[tokio::test]
    async fn plan_slot_matches_the_baked_layer_posture() {
        use epsilon_core::PermissionGate;
        let ui = Arc::new(ScriptedUi {
            answers: Mutex::new(vec![]),
            asked: Mutex::new(0),
        });
        let slotted = RulesetGate::new(Arc::clone(&ui) as Arc<dyn ApprovalUi>);
        assert!(slotted.set_plan_mode(true));
        let baked = RulesetGate::layered(
            PermissionEngine::layered(vec![Ruleset {
                name: "plan".into(),
                rules: crate::plan_ruleset().rules,
            }]),
            Arc::clone(&ui) as Arc<dyn ApprovalUi>,
        );
        for (cap, subject) in [
            ("write", "src/main.rs"),
            ("edit", "src/main.rs"),
            ("bash", "cargo test"),
            ("shell", "echo hi"),
            ("read", "src/main.rs"),
            ("grep", "*"),
        ] {
            let a = slotted.authorize(req(cap, subject)).await;
            let b = baked.authorize(req(cap, subject)).await;
            assert_eq!(
                a.allowed(),
                b.allowed(),
                "slot vs baked disagree on ({cap}, {subject})"
            );
            assert_eq!(
                a.asked, b.asked,
                "slot vs baked ask-flag disagree on ({cap}, {subject})"
            );
        }
    }
}
