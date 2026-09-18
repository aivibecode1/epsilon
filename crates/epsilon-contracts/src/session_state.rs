//! The session-state snapshot both wire surfaces serve (RPC-02).
//!
//! One payload, two transports: the stdio RPC `get_state` reply and serve mode's
//! `GET /state` must not be able to disagree about the shape, so it lives here rather
//! than in either frontend — a `cli -> server` edge (the alternative) is forbidden by
//! the architecture law.
//!
//! Field set: pi's `RpcSessionState` fields where this harness has a live accessor
//! (`pi/packages/coding-agent/src/modes/rpc/rpc-types.ts:95-108`), plus one field pi's
//! payload has no counterpart for at all — `estimated_context_tokens` is this harness's
//! own estimate of the next request. The shape is therefore not a subset of pi's: some
//! pi fields are absent, and one of ours is additive. Deliberate divergences beyond
//! that: pi's `model` is a whole `Model` object where ours is the model id string, and
//! pi's keys are camelCase (the repo's wire vocabulary is snake_case). Fields this
//! harness cannot answer from live state — session name, queue modes, the
//! auto-compaction toggle, `isCompacting` — are absent rather than reported as fiction.
//!
//! Types only: no constructor lives here, because building the payload needs an
//! `Agent` (`epsilon-core`), and this crate stays behaviour-free by rule
//! (`design/epsilon-architecture.md` §2).

use serde::{Deserialize, Serialize};

use crate::message::ThinkingLevel;

/// Live session state as reported by RPC `get_state` and `GET /state`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SessionState {
    /// Model identifier the next request will carry (`Agent::model_id`).
    pub model: String,
    /// Session thinking level (`Agent::thinking_level`).
    pub thinking_level: ThinkingLevel,
    /// Whether a run is active right now (`Agent::is_running`).
    pub is_streaming: bool,
    /// Transcript size, all roles, including harness-only messages (`Agent::history`).
    pub message_count: usize,
    /// Steering messages queued but not yet delivered (`Agent::steer_queue_len`).
    pub pending_message_count: usize,
    /// Attached session file (`Agent::session_path`), or `null` when the agent runs
    /// without a durable mirror — a normal state, not an error.
    pub session_file: Option<String>,
    /// Best estimate of the next request's prompt size in tokens
    /// (`Agent::estimated_context_tokens`) — the number the compaction trigger
    /// decides on, so a state client shows what the loop actually uses.
    pub estimated_context_tokens: usize,
}
