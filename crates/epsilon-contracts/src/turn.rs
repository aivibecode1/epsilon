//! Turn-boundary hooks — pi's `AgentLoopConfig` callbacks.
//!
//! Where [`crate::tool::ToolHooks`] intercepts tool execution, `TurnHooks` covers the four
//! per-turn seams the loop exposes (pi `packages/agent/src/types.ts:149-293`):
//!
//! - [`TurnHooks::transform_context`] — AgentMessage-level transformation (pruning, external
//!   context injection) applied to the run's context before conversion;
//! - [`TurnHooks::convert_to_llm`] — the `AgentMessage[] → Message[]` boundary (pi's required
//!   `convertToLlm`), defaulting to the built-in harness-visibility filter;
//! - [`TurnHooks::prepare_next_turn`] — after `turn_end`, return a snapshot that replaces the
//!   run's context, model and thinking level for the remaining provider requests;
//! - [`TurnHooks::should_stop_after_turn`] — after `prepare_next_turn` and before any queue
//!   polling; `true` ends the run without draining steering or follow-ups.
//!
//! Ported semantics: pi `agent-loop.ts:224-257` (prepare then stop, both before the steering
//! poll; a stop emits `agent_end` and returns) and `:288-295` (transform, then convert, at the
//! request boundary). Epsilon's ordering and queues are unchanged: with no hook registered the
//! loop behaves exactly as it does without this module.
//!
//! Contract (pi, kept verbatim): hooks must not throw/panic — return the safe fallback instead
//! (the input messages, `None`, `false`). The loop does not rely on that promise: a panicking
//! hook is contained at the seam and the run continues on the no-op default (the same
//! mechanical guarantee the provider seam's `PanicGuard` gives a stream). Hooks are readers
//! of a run: they can neither mutate runtime history nor write the durable store (ADR-007); a
//! replacement context is run-scoped state held by the loop, not a rewrite of the transcript.
//!
//! The seams are **async** (`#[async_trait]`), matching [`crate::tool::ToolHooks`] and pi's
//! own shape: pi's hooks may return a Promise, and an async seam is how a host that needs I/O
//! (a compaction summarizer, an extension runner) satisfies that without owning a runtime of
//! its own — the loop awaits each call at the point pi does.
//!
//! Undisclosed deltas from pi, recorded here because the code cannot show them:
//!
//! - [`TurnUpdate::context`] replaces the run's *message* context only. pi's snapshot is a
//!   whole `AgentContext`, so it can also swap `systemPrompt` and `tools` (`types.ts:138-145`);
//!   Epsilon rebuilds the system prompt from session state per request (ADR-008) and the tool
//!   catalog is per-agent, so neither is a run-scoped value a hook can swap here.
//! - [`NextTurnContext::new_messages`] excludes the run's own prompt. pi's `newMessages` starts
//!   with the prompts (`agent-loop.ts:103`, `[...prompts]`), while Epsilon's slice — the same
//!   one `agent_end` carries — starts after them: `Agent::run` appends the prompt to the
//!   transcript before the loop body.
//! - Hooks cannot observe cancellation. pi hands every callback the run's `AbortSignal`
//!   (`types.ts:149-293`); no token reaches these seams, so a long hook cannot end early on a
//!   user abort — it runs to completion, and the loop's next abort checkpoint still wins.
//!
//! Object-safe + `Send + Sync` so hosts (extension runner, compaction summarizer, tests) can
//! share one `Arc<dyn TurnHooks>` with the agent.

use async_trait::async_trait;

use crate::message::{Message, ThinkingLevel};

/// Model for the remaining provider requests of a run (catalog id).
///
/// Same shape as `epsilon_llm::ModelRef` but deliberately differently named: the provider
/// layer's [`epsilon_llm::ModelRef`] cannot be reused here (contracts depends on no other
/// Epsilon crate by design), and two identically named types for one concept would invite
/// confusion. `epsilon-core` builds the provider-layer reference from `id` at request time.
/// Unifying the two into this crate — with `epsilon_llm` re-exporting — is a recorded
/// follow-up, not part of this slice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnModel {
    /// Catalog model id, e.g. `claude-sonnet-4`.
    pub id: String,
}

impl TurnModel {
    /// Convenience constructor.
    pub fn new(id: impl Into<String>) -> Self {
        Self { id: id.into() }
    }
}

/// What a turn hook sees once a turn has fully completed (pi's
/// `PrepareNextTurnContext` / `ShouldStopAfterTurnContext`).
///
/// Borrowed views, built by the loop for the hook call only: nothing here is stored, and a
/// hook that keeps nothing cannot change the run from the outside.
#[derive(Debug, Clone)]
pub struct NextTurnContext<'a> {
    /// The assistant message that produced this turn.
    pub message: &'a Message,
    /// Tool results of this turn, in assistant source order. Empty when the turn called no
    /// tools.
    pub tool_results: &'a [Message],
    /// The run's context as the next provider request would start from it — the durable
    /// store's shadowed model view when one is attached, else runtime history. For
    /// `should_stop_after_turn` this is the context *after* any replacement
    /// [`TurnUpdate::context`] was applied (pi evaluates the stop on the replaced context).
    pub context: &'a [Message],
    /// Messages produced since this run started (injected steering/follow-ups, assistant
    /// messages, tool results) — the same slice `agent_end` carries.
    pub new_messages: &'a [Message],
}

/// Replacement run state returned by [`TurnHooks::prepare_next_turn`] (pi's
/// `AgentLoopTurnUpdate`).
///
/// Every field is optional and `None` means "keep the current value", exactly like pi's
/// `snapshot.context ?? context`. The update applies to the remaining provider requests of
/// **this** run only; session state (the agent's model, thinking level, transcript) is
/// untouched.
///
/// **Delta from pi** (`types.ts:138-145`): only the message context is replaceable here — pi's
/// snapshot carries a whole `AgentContext` and can also swap `systemPrompt`/`tools` (see the
/// module docs).
#[derive(Debug, Clone, Default)]
pub struct TurnUpdate {
    /// Replacement context for subsequent provider requests. `None` keeps the current one.
    /// The loop appends everything it adds to the transcript during the run (steering
    /// messages, the turn's assistant message, tool results, nudges) to this context, exactly
    /// as pi appends to `currentContext.messages`.
    pub context: Option<Vec<Message>>,
    /// Model for subsequent provider requests. `None` keeps the current one.
    pub model: Option<TurnModel>,
    /// Thinking level for subsequent provider requests. `None` keeps the current one.
    pub thinking: Option<ThinkingLevel>,
}

/// Turn-boundary interception points (pi's `AgentLoopConfig`).
///
/// All four methods have the pi default: context passes through, conversion applies the
/// visibility filter, `prepare_next_turn` returns `None` (keep everything) and
/// `should_stop_after_turn` returns `false`. A host implements only the seams it needs.
///
/// Async like [`crate::tool::ToolHooks`] — pi's callbacks may return a Promise, so the seam
/// gives a host that needs I/O (a summarizer, an extension runner) somewhere to await it.
#[async_trait]
pub trait TurnHooks: Send + Sync {
    /// Transform the run's context before [`TurnHooks::convert_to_llm`] runs (pi's
    /// `transformContext`).
    ///
    /// This is the AgentMessage-level seam: pruning old messages, injecting context from
    /// outside the transcript. The default clones the input through, so the provider sees the
    /// run's context unchanged. Must not panic; return the input on any internal failure.
    async fn transform_context(&self, messages: &[Message]) -> Vec<Message> {
        messages.to_vec()
    }

    /// Convert the transformed context into provider-visible messages (pi's `convertToLlm`,
    /// which is required there and defaults here to Epsilon's built-in behaviour).
    ///
    /// The default drops harness-only messages ([`Message::is_llm_visible`]), which is exactly
    /// what the loop did before this seam existed. Must not panic; return a safe fallback.
    async fn convert_to_llm(&self, messages: &[Message]) -> Vec<Message> {
        messages
            .iter()
            .filter(|m| m.is_llm_visible())
            .cloned()
            .collect()
    }

    /// Called after the turn's `turn_end` was emitted and before the loop decides whether
    /// another provider request should start.
    ///
    /// Return a [`TurnUpdate`] to affect the next turn of this run, or `None` to keep the
    /// current context/model/thinking. Must not panic.
    async fn prepare_next_turn(&self, ctx: &NextTurnContext<'_>) -> Option<TurnUpdate> {
        let _ = ctx;
        None
    }

    /// Called after [`TurnHooks::prepare_next_turn`] and before steering or follow-up queues
    /// are polled.
    ///
    /// `true` ends the run: the loop emits `agent_end` and exits without another provider
    /// request and without draining queued follow-ups (pi `agent-loop.ts:248-257`). Use it to
    /// stop gracefully after a turn, e.g. before the context gets too full. Must not panic.
    async fn should_stop_after_turn(&self, ctx: &NextTurnContext<'_>) -> bool {
        let _ = ctx;
        false
    }
}

/// No-op hooks (default when no host registers any): the loop behaves as if the seam did not
/// exist.
#[derive(Debug, Default)]
pub struct NoopTurnHooks;

#[async_trait]
impl TurnHooks for NoopTurnHooks {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::ContentBlock;
    use crate::message::{CustomMessage, StopReason, UserMessage};

    fn user(text: &str) -> Message {
        Message::User(UserMessage {
            id: crate::new_id("msg"),
            timestamp_ms: 0,
            content: vec![crate::content::UserContent::Text { text: text.into() }],
        })
    }

    fn custom() -> Message {
        Message::Custom(CustomMessage {
            id: crate::new_id("msg"),
            timestamp_ms: 0,
            custom_type: "todo_snapshot".into(),
            payload: serde_json::json!({"items": []}),
        })
    }

    fn assistant() -> Message {
        Message::Assistant(crate::message::AssistantMessage {
            id: crate::new_id("msg"),
            timestamp_ms: 0,
            provider: "faux".into(),
            model_id: "faux-1".into(),
            content: vec![ContentBlock::text("hi")],
            stop_reason: StopReason::EndTurn,
            error_message: None,
            usage: crate::message::Usage::default(),
        })
    }

    #[tokio::test]
    async fn defaults_match_the_pre_hook_loop() {
        let hooks = NoopTurnHooks;
        let input = vec![user("hi"), custom(), assistant()];

        // transform passes everything through, conversion then drops the harness-only role:
        // the pair reproduces the loop's historical visibility filter byte for byte.
        let transformed = hooks.transform_context(&input).await;
        assert_eq!(transformed, input, "transform is identity by default");
        let converted = hooks.convert_to_llm(&transformed).await;
        assert_eq!(
            converted,
            vec![input[0].clone(), input[2].clone()],
            "conversion keeps user + assistant and drops the harness-only role"
        );
        assert!(
            !converted.iter().any(|m| matches!(m, Message::Custom(_))),
            "harness-only messages never reach a provider by default"
        );

        // The turn seams keep the run as it is.
        let ctx = NextTurnContext {
            message: &input[2],
            tool_results: &[],
            context: &input,
            new_messages: &input,
        };
        assert!(hooks.prepare_next_turn(&ctx).await.is_none());
        assert!(!hooks.should_stop_after_turn(&ctx).await);
    }

    #[test]
    fn turn_update_defaults_keep_everything() {
        let update = TurnUpdate::default();
        assert!(update.context.is_none());
        assert!(update.model.is_none());
        assert!(update.thinking.is_none());
        assert_eq!(TurnModel::new("m").id, "m");
    }
}
