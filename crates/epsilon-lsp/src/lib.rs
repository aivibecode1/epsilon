//! `epsilon-lsp` — language-server diagnostics as edit feedback (roadmap Phase 5 item 1).
//!
//! The problem this solves is the one every coding agent has: the model writes code, and the
//! *compiler* disagrees. Without a server in the loop the model only learns about a type error
//! when the user runs the build — a whole turn later, and often not at all. opencode's answer,
//! and the one ported here, is to ask the language server that already knows the answer and ride
//! the result on the edit's own tool output.
//!
//! Shape of the port (opencode `packages/opencode/src/lsp/*`, `tool/edit.ts:196-211`):
//!
//! - [`framing`] — JSON-RPC over stdio with `Content-Length` headers. Thirty lines, no dependency.
//! - [`server`] — which server answers for which file, and where it lives. **User-installed only**:
//!   the auto-install paths opencode carries are a supply chain this harness refuses.
//! - [`client`] — one child process per (project root, server), `initialize` handshake, document
//!   notifications, and the diagnostics that come back.
//! - [`service`] — the lifecycle that keeps that from becoming a process leak: reuse, in-flight
//!   spawn dedupe, and a `broken` set so a server that dies is not retried forever.
//! - [`report`] — the bounded block an edit carries: ERROR only, at most twenty per file.
//!
//! Degradation is silent by design. A machine without `rust-analyzer` gets exactly today's
//! behaviour: the edit succeeds, and nothing is said. A harness that fails an edit for want of a
//! tool is worse than one that cannot lint.

pub mod client;
pub mod framing;
pub mod report;
pub mod server;
pub mod service;

pub use client::{Client, ClientConfig};
pub use report::{Diagnostic, MAX_REPORTED, Severity, render_report};
pub use server::{SERVERS, ServerDef, select_server, which};
pub use service::{DiagnosticsService, ServiceConfig};
