//! Channel-backed approval UI shared by interactive frontends (M9b).
//!
//! The TUI dialog and the serve-mode REST endpoint both need the same shape: gate asks
//! park somewhere addressable by `request_id`, a UI loop (or an HTTP handler) resolves
//! them later, possibly out of order. [`ChannelApprovalUi`] is that parking lot — one
//! implementation so both frontends behave identically and tests can drive either.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use epsilon_contracts::agent_event::{PermissionDecision, PermissionRequest};

use crate::gate::ApprovalUi;

/// One parked permission ask: the request plus the private reply channel.
pub struct PendingAsk {
    /// The request being asked (capability/pattern/detail for rendering).
    pub request: PermissionRequest,
    tx: tokio::sync::oneshot::Sender<PermissionDecision>,
}

impl PendingAsk {
    /// Answer this ask; false when nobody is waiting anymore.
    pub fn reply(self, decision: PermissionDecision) -> bool {
        self.tx.send(decision).is_ok()
    }
}

struct State {
    /// FIFO of request ids awaiting pickup by a UI loop.
    order: VecDeque<String>,
    /// Every live ask until resolved — enables out-of-order resolution and listing.
    asks: HashMap<String, PendingAsk>,
}

/// Approval UI that parks asks in an id-keyed queue for later resolution.
///
/// Clone-friendly: every clone feeds the same queue. If a consumer drops a
/// [`PendingAsk`] without replying, the blocked `ask()` resolves to
/// [`PermissionDecision::Deny`] — fail closed, per the house safety rule.
#[derive(Clone)]
pub struct ChannelApprovalUi {
    state: Arc<Mutex<State>>,
    notify: Arc<tokio::sync::Notify>,
}

impl Default for ChannelApprovalUi {
    fn default() -> Self {
        Self::new()
    }
}

impl ChannelApprovalUi {
    /// Create an empty ask queue.
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(State {
                order: VecDeque::new(),
                asks: HashMap::new(),
            })),
            notify: Arc::new(tokio::sync::Notify::new()),
        }
    }

    /// Take the oldest parked ask (FIFO), removing it from the pending set.
    ///
    /// Resolution stays possible afterwards via [`Self::resolve`] only while the taken
    /// ask's reply channel is alive — callers that hold the [`PendingAsk`] should reply
    /// on it directly.
    pub async fn next_ask(&self) -> Option<PendingAsk> {
        loop {
            if let Ok(mut st) = self.state.lock() {
                if let Some(id) = st.order.pop_front()
                    && let Some(ask) = st.asks.remove(&id)
                {
                    return Some(ask);
                }
            } else {
                return None; // poisoned: no UI service possible
            }
            self.notify.notified().await;
        }
    }

    /// Snapshot of every unresolved ask, oldest first.
    pub fn pending(&self) -> Vec<PermissionRequest> {
        let mut out = Vec::new();
        if let Ok(st) = self.state.lock() {
            for id in &st.order {
                if let Some(ask) = st.asks.get(id) {
                    out.push(ask.request.clone());
                }
            }
        }
        out
    }

    /// Resolve an ask by its `request_id` without consuming FIFO order first.
    ///
    /// Returns false when the id is unknown or already resolved.
    pub fn resolve(&self, request_id: &str, decision: PermissionDecision) -> bool {
        if let Ok(mut st) = self.state.lock()
            && let Some(ask) = st.asks.remove(request_id)
        {
            st.order.retain(|id| id != request_id);
            return ask.reply(decision);
        }
        false
    }
}

#[async_trait]
impl ApprovalUi for ChannelApprovalUi {
    async fn ask(&self, req: PermissionRequest) -> PermissionDecision {
        let (tx, rx) = tokio::sync::oneshot::channel();
        {
            let mut st = match self.state.lock() {
                Ok(s) => s,
                Err(_) => return PermissionDecision::Deny,
            };
            st.order.push_back(req.request_id.clone());
            st.asks
                .insert(req.request_id.clone(), PendingAsk { request: req, tx });
        }
        self.notify.notify_one();
        // Dropped/unanswered responder => nobody can approve; deny beats hanging forever.
        rx.await.unwrap_or(PermissionDecision::Deny)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(id: &str, cap: &str) -> PermissionRequest {
        PermissionRequest {
            request_id: id.into(),
            capability: cap.into(),
            pattern: "*".into(),
            detail: serde_json::json!({}),
        }
    }

    #[tokio::test]
    async fn ask_parks_and_next_ask_replies_in_fifo_order() {
        let ui = ChannelApprovalUi::new();
        let ui_a = ui.clone();
        let ui_b = ui.clone();
        let f1 = tokio::spawn(async move { ui_a.ask(req("p1", "bash")).await });
        let f2 = tokio::spawn(async move { ui_b.ask(req("p2", "write")).await });
        while ui.pending().len() < 2 {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }

        let first = ui.next_ask().await.expect("first ask");
        assert_eq!(first.request.request_id, "p1");
        assert!(first.reply(PermissionDecision::AllowOnce));

        // Out-of-order resolution by id must work even while p2 sits unpicked.
        assert!(ui.resolve("p2", PermissionDecision::AlwaysSession));
        assert!(ui.pending().is_empty());
        assert_eq!(f1.await.unwrap(), PermissionDecision::AllowOnce);
        assert_eq!(f2.await.unwrap(), PermissionDecision::AlwaysSession);
    }

    #[tokio::test]
    async fn resolve_unknown_or_double_ids_report_false() {
        let ui = ChannelApprovalUi::new();
        assert!(!ui.resolve("nope", PermissionDecision::Deny));

        let ui2 = ui.clone();
        let f = tokio::spawn(async move { ui2.ask(req("p9", "bash")).await });
        while (ui.pending()).is_empty() {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        assert!(ui.resolve("p9", PermissionDecision::AllowOnce));
        assert!(
            !ui.resolve("p9", PermissionDecision::AllowOnce),
            "double resolve"
        );
        assert_eq!(f.await.unwrap(), PermissionDecision::AllowOnce);
    }

    #[tokio::test]
    async fn dropped_ask_denies_fail_closed() {
        let ui = ChannelApprovalUi::new();
        let ui2 = ui.clone();
        let f = tokio::spawn(async move { ui2.ask(req("px", "external_dir")).await });
        while (ui.pending()).is_empty() {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        let ask = ui.next_ask().await.expect("ask");
        drop(ask); // UI vanished without answering
        assert_eq!(f.await.unwrap(), PermissionDecision::Deny);
    }
}
