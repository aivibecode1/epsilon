#![allow(dead_code)]
//! Append-only JSONL tree session store.
//!
//! One `.jsonl` file per session; each line is a [`SessionEntry`] assigned a monotonic `seq`,
//! a parent link (branch tree), and an append timestamp. The human transcript is the file;
//! the model-visible history is a derived view where compaction entries shadow the ranges they
//! summarize (DSH's surface-shadowing principle over pi's entry format).

// repair is consumed from M5's loop runner; until then integration tests exercise it.
mod lease;
#[allow(dead_code)]
pub mod repair;
mod store;

pub use lease::{LeaseOutcome, SessionLease};
pub use store::{ModelViewEntry, SessionStore, StoreError};
