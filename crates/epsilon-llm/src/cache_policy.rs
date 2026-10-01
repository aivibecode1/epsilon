//! Where the provider prompt cache is asked to break.
//!
//! # Why this is a policy and not a constant
//!
//! Anthropic caches a prefix of the request and charges full price above the last
//! `cache_control` marker. **One marker is a choice**, and the obvious place to put it --
//! the system prompt, which is what this crate did -- is the *least* useful place for a
//! coding agent: the system prompt barely changes within a session, while the transcript
//! grows on every tool call, and nothing after the system block was ever cached.
//!
//! # The three breakpoints, and what each is for
//!
//! | Marker | Why |
//! |---|---|
//! | the last **tool definition** | Tool schemas sit between the system prompt and the transcript, so a stable marker there keeps the expensive part of the prefix reusable when a later conversation extends it. |
//! | the last **system block** | The stable prefix. Kept from pi's behaviour. |
//! | the last **user message** | **This is the one that pays.** A single turn is many API calls: each tool result sends the whole history back. The latest user message sits at the start of the turn and does not move, so every one of those calls reuses everything up to it. |
//!
//! opencode reaches the same three (`packages/llm/src/cache-policy.ts:18-22`) and gives the
//! same reason: *"the latest user message stays put while a single turn explodes into many
//! assistant/tool round-trips, so caching at that boundary lets every intra-turn API call
//! hit the prefix."*
//!
//! # Why this lives outside the wire
//!
//! The decision is about a conversation, not about JSON: the same policy applies to a body
//! assembled for a different provider with different cache semantics. Naming it separately
//! keeps the wire file about shapes and makes the policy testable without a request.
//!
//! **And why only Anthropic.** OpenAI and Gemini cache implicitly by prefix and ignore
//! inline hints; emitting them there would be harmless but pointless. Capability-gated on the
//! wire, not provider-name-gated, so a provider that gains the feature gets it without a
//! second edit here.

/// How many cache breakpoints a request may carry.
///
/// Anthropic accepts at most four. Three are spent on the boundaries above; the fourth is
/// left deliberately free rather than spent on the newest assistant turn, because a marker
/// at the very end caches a prefix that the *next* request invalidates — it would be paid
/// for on this call and gone on the next.
pub const MAX_BREAKPOINTS: usize = 3;

/// Which blocks in a request should carry `cache_control`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CachePolicy {
    /// Mark the final tool definition.
    pub mark_last_tool: bool,
    /// Mark the final system block.
    pub mark_last_system: bool,
    /// Mark the final user message's last text-bearing block.
    pub mark_last_user_message: bool,
}

impl CachePolicy {
    /// The policy every Anthropic request uses.
    ///
    /// Marking is **on by default**, and the economics are the reason: a cache write costs
    /// 1.25x base and a cache read 0.1x, so a single reuse within five minutes already
    /// pays for the write. A harness that only caches on request would be saving nothing.
    pub const DEFAULT: Self = Self {
        mark_last_tool: true,
        mark_last_system: true,
        mark_last_user_message: true,
    };

    /// No markers at all — for a wire whose provider caches implicitly, or a test that needs
    /// the raw shape.
    pub const NONE: Self = Self {
        mark_last_tool: false,
        mark_last_system: false,
        mark_last_user_message: false,
    };

    /// How many breakpoints this policy will place.
    pub fn count(&self) -> usize {
        usize::from(self.mark_last_tool)
            + usize::from(self.mark_last_system)
            + usize::from(self.mark_last_user_message)
    }
}

/// The `cache_control` value Anthropic expects.
pub fn ephemeral() -> serde_json::Value {
    serde_json::json!({ "type": "ephemeral" })
}

#[cfg(test)]
mod tests {
    use super::*;

    // These tests are about the **arithmetic** — how many markers a policy spends and
    // whether it fits inside what the provider accepts. Whether a marker lands on the
    // right block of the right JSON is not testable here at all, and asserting a `const`
    // boolean would be a test that cannot fail. `tests/cache_breakpoints.rs` builds a real
    // request body for that.

    #[test]
    fn the_default_policy_uses_at_most_what_the_provider_accepts() {
        assert!(
            CachePolicy::DEFAULT.count() <= MAX_BREAKPOINTS,
            "Anthropic accepts four; the policy spends three and leaves the fourth free"
        );
    }

    #[test]
    fn marking_is_on_by_default_because_a_single_reuse_pays_for_the_write() {
        // A cache write costs 1.25x base and a read 0.1x, so one reuse inside five
        // minutes already repays it. A harness that only cached on request would be
        // saving nothing.
        assert_ne!(CachePolicy::DEFAULT, CachePolicy::NONE);
        assert_eq!(CachePolicy::NONE.count(), 0);
    }
}
