//! Epsilon CLI library: the frontends' helpers plus a re-export of the composition crate.
//!
//! `runtime` is a **re-export, not a module of this crate**. Composition lives in
//! `epsilon-runtime` (ADR-021) because a frontend in another crate — the desktop IDE —
//! has to reach it, and it was a private function in this binary until M233. The re-export
//! keeps every existing `epsilon_cli::runtime::…` path working unchanged, so the move is
//! one commit that adds a crate rather than one that rewrites thirty call sites.

pub mod doctor;
pub mod rpc_mode;

pub use epsilon_runtime as runtime;
