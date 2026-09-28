//! The session-state snapshot both wire surfaces serve (RPC-02).
//!
//! One payload, two transports: the stdio RPC `get_state` reply and serve mode's
//! `GET /state` must not be able to disagree about the shape, so it lives here rather
//! than in either frontend — a `cli -> server` edge (the alternative) is forbidden by
//! the architecture law.
//!
//! Field set: pi's `RpcSessionState` fields where this harness has a live accessor
//! (`pi/packages/coding-agent/src/modes/rpc/rpc-types.ts:95-108`), plus one field pi's
//! payload has no counterpart for at all — `estimatedContextTokens` is this harness's own
//! estimate of the next request. The shape is therefore not a subset of pi's: some pi
//! fields are absent, and one of ours is additive. Fields this harness cannot answer from
//! live state — the auto-compaction toggle and `isCompacting` — are absent rather than
//! reported as fiction, because there is no auto-compaction to report on yet.
//!
//! ## Why the keys are camelCase (D30)
//!
//! This file used to serialize as `snake_case`, with the rationale *"pi's keys are
//! camelCase (the repo's wire vocabulary is snake_case)"*. That rationale was
//! **circular** — the repo's convention cannot justify diverging from the reference, and
//! the premise had stopped being true anyway. Since M190 this same RPC surface has
//! emitted pi's exact spellings for everything else: `parentId`, `leafId`, `sessionFile`,
//! `followUp`, `costIsFloor`. So `get_state` was the one op answering in a different
//! vocabulary from its own neighbours, and a pi client polling the most-polled object in
//! the protocol got `undefined` for **every multi-word field** — `thinkingLevel`,
//! `isStreaming`, `messageCount`, `pendingMessageCount`, `sessionFile` — with `model`,
//! the only single-word field, the sole thing that worked.
//!
//! Found by probing the installed binary, not by reading this comment. The same lesson as
//! D27 and D29: a documented divergence is still a divergence, and a *reason* recorded in
//! a doc comment is not evidence that the divergence is right — it only records that
//! somebody thought about it once.
//!
//! Both surfaces change together, deliberately. The reason this payload lives in one
//! struct is so the two transports **cannot** disagree; giving the RPC surface camelCase
//! and leaving `GET /state` snake_case would reintroduce by the back door the exact
//! two-spellings defect D29 was about. The OpenAPI spec is part of the same change: a
//! specification that documents names the server does not emit is worse than no
//! specification.
//!
//! The remaining divergence is pi's `model`, a whole `Model` object where this is the
//! model id string. That one stays: an object would mean inventing a `Model` type pi's
//! client would then read fields from, and a string is honest about what it is.
//!
//! Types only: no constructor lives here, because building the payload needs an
//! `Agent` (`epsilon-core`), and this crate stays behaviour-free by rule
//! (`design/epsilon-architecture.md` §2).

use serde::{Deserialize, Serialize};

use crate::message::ThinkingLevel;
use crate::queue_mode::QueueMode;

/// Live session state as reported by RPC `get_state` and `GET /state`.
///
/// `rename_all = "camelCase"` is pi's spelling, not a style choice — see the module doc.
/// The Rust field names stay `snake_case`, which is the language's convention and costs
/// nothing, because serde is what puts a name on the wire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionState {
    /// Model identifier the next request will carry (`Agent::model_id`).
    ///
    /// pi sends a whole `Model` object here. A string is the recorded divergence.
    pub model: String,
    /// Session thinking level (`Agent::thinking_level`).
    pub thinking_level: ThinkingLevel,
    /// Whether a run is active right now (`Agent::is_running`).
    pub is_streaming: bool,
    /// How much of the steering queue a drain releases (`Agent::steering_mode`).
    pub steering_mode: QueueMode,
    /// How much of the follow-up queue a drain releases (`Agent::follow_up_mode`).
    pub follow_up_mode: QueueMode,
    /// Transcript size, all roles, including harness-only messages (`Agent::history`).
    pub message_count: usize,
    /// Steering messages queued but not yet delivered (`Agent::steer_queue_len`).
    pub pending_message_count: usize,
    /// Attached session file (`Agent::session_path`), or `null` when the agent runs
    /// without a durable mirror — a normal state, not an error.
    pub session_file: Option<String>,
    /// The session's own id from its header (`Agent::session_id`).
    ///
    /// Distinct from `session_file`: a file can be renamed, and two sessions can be
    /// forked from one file, so the id is what a client stores to identify the
    /// conversation rather than the file that happens to hold it now.
    pub session_id: Option<String>,
    /// Display name set by `/name` or RPC `set_session_name` (`Agent::session_name`).
    pub session_name: Option<String>,
    /// Best estimate of the next request's prompt size in tokens
    /// (`Agent::estimated_context_tokens`) — the number the compaction trigger
    /// decides on, so a state client shows what the loop actually uses.
    pub estimated_context_tokens: usize,
}
