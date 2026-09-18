//! The dropped-event notice both wire surfaces report (RPC-02).
//!
//! One shape, two transports: the stdio RPC frames a dropped broadcast as
//! `{"lagged": <this notice>}` (`epsilon-cli`'s `rpc_mode`), and serve mode emits an
//! SSE event named `lagged` whose data is `<this notice>` serialized
//! (`epsilon-server`). Both surfaces build the notice from this struct so the two
//! cannot drift apart, and the wire bytes are unchanged from the pre-shared literals.
//!
//! Types only: no constructor, no behaviour — the crate stays behaviour-free by rule
//! (`design/epsilon-architecture.md` §2).

use serde::{Deserialize, Serialize};

/// How many events a stalled subscriber missed when the broadcast channel lagged.
///
/// Serializes to `{"dropped":n}`; `n` is what tokio's `RecvError::Lagged` reported.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaggedNotice {
    /// Events dropped before delivery (tokio's `RecvError::Lagged(u64)` count).
    pub dropped: u64,
}
