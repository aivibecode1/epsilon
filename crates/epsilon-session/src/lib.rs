//! Append-only JSONL tree session store.
//!
//! One `.jsonl` file per session; each line is a [`SessionEntry`] assigned a monotonic `seq`,
//! a parent link (branch tree), and an append timestamp. The human transcript is the file;
//! the model-visible history is a derived view where compaction entries shadow the ranges they
//! summarize (DSH's surface-shadowing principle over pi's entry format).

pub mod discover;
mod lease;
// Wired into the CLI resume path (--resume/--continue) since M19's ponytail-audit
// follow-up; integration tests cover the crash scenarios.
pub mod repair;
pub mod search;
mod store;

pub use discover::{SessionSummary, latest_session, list_sessions, summarize};
pub use lease::{LeaseOutcome, SessionLease};
pub use search::{SearchOptions, SessionSearchHit, search_session};
pub use store::{LineDiagnosis, ModelViewEntry, SessionStore, StoreError};
