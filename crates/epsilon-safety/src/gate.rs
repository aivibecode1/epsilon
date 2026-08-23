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

/// `(capability, pattern)` pairs promoted to allow-for-this-session by an `AlwaysSession`.
type SessionMemory = Arc<Mutex<Vec<(String, String)>>>;

/// Gate implementing the opencode-style flow:
/// session memory → engine evaluation → (`ask`) approval UI → optional memory promotion.
pub struct RulesetGate {
    engine: PermissionEngine,
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
            engine,
            ui,
            memory: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Read-only view of promoted always-allow pairs (diagnostics/tests).
    pub fn session_allows(&self) -> Vec<(String, String)> {
        self.memory.lock().map(|m| m.clone()).unwrap_or_default()
    }
}

#[async_trait]
impl PermissionGate for RulesetGate {
    async fn authorize(&self, req: PermissionRequest) -> Authorization {
        // 1. Session memory short-circuits before any UI round-trip.
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

        // 2. Engine evaluation.
        let action = self.engine.evaluate(&req.capability, &req.pattern);
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
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
