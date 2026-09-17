//! The diagnostics seam: what an edit may ask about the code it just wrote.
//!
//! A trait rather than a concrete type so `epsilon-tools` depends on a promise ("someone may know
//! what the compiler thinks") instead of on `epsilon-lsp`, and so every tool test runs against a
//! fake with no language server installed (ADR-014 amendment, M79).

use std::path::Path;

use async_trait::async_trait;

/// Something that can answer "does this file compile?" after a change.
///
/// The contract is deliberately weak, because the real answers are weak: `None` means *nothing to
/// say* — no server for this file type, no server installed, a server that is broken, or a clean
/// file — and callers must treat all four the same way (an edit that succeeded stays succeeded).
#[async_trait]
pub trait DiagnosticsProvider: Send + Sync {
    /// Report the errors `path` contains now that its content is `text`.
    ///
    /// Returns a ready-to-append text block, or `None` when there is nothing worth saying.
    async fn after_change(&self, path: &Path, text: &str) -> Option<String>;
}
