//! The query seam: what the model may ask a language server about code it did not just write.
//!
//! A trait rather than a concrete type so `epsilon-tools` depends on a promise ("someone can
//! answer go-to-definition") instead of on `epsilon-lsp`, and so every tool test runs against a
//! fake with no language server installed (ADR-014 amendment, M79; the same M215 seam as
//! [`crate::diagnostics::DiagnosticsProvider`]).

use std::path::{Path, PathBuf};

use async_trait::async_trait;

/// One navigation hit, rebased to the 1-based positions the model uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LspLocation {
    /// Absolute file path.
    pub path: PathBuf,
    /// 1-based line.
    pub line: u32,
    /// 1-based column.
    pub character: u32,
}

/// One named symbol a file defines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LspSymbol {
    /// The symbol's name, as the server reported it.
    pub name: String,
    /// The server's kind label (for example `"Function"`), verbatim.
    pub kind: String,
    /// Where the symbol is defined (1-based).
    pub location: LspLocation,
}

/// Why a query has nothing to show.
///
/// An empty answer is NOT this: a query that ran and found nothing is `Ok(vec![])`, so the
/// caller can say "no definitions here" instead of reporting a failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LspQueryFailure {
    /// No server claims this file's extension. Carries every known extension, so the caller
    /// can list what would have worked.
    UnknownExtension {
        /// Lower-cased extension without the dot; empty when the file has none.
        extension: String,
        /// Every extension the server table knows, sorted.
        known: Vec<String>,
    },
    /// The server for this file is not installed on this machine.
    MissingBinary {
        /// Human name of the server, for the message.
        name: String,
        /// Executable that was looked up on `PATH`.
        binary: String,
    },
    /// The server failed, died, or never answered. The service will not retry it this session.
    Broken {
        /// Server id plus the reason, for the message.
        detail: String,
    },
}

/// Something that can answer navigation questions about workspace files.
///
/// Positions IN are 0-based LSP positions — the tool converts the model's 1-based input, so
/// every implementor and fake speaks wire numbers. Positions OUT ([`LspLocation`]) are
/// 1-based, ready to render as `path:line:col`.
#[async_trait]
pub trait LspQueryProvider: Send + Sync {
    /// Where the symbol at 0-based (`line`, `character`) of `path` is defined.
    async fn go_to_definition(
        &self,
        path: &Path,
        line: u32,
        character: u32,
    ) -> Result<Vec<LspLocation>, LspQueryFailure>;

    /// Where the symbol at 0-based (`line`, `character`) of `path` is used.
    async fn find_references(
        &self,
        path: &Path,
        line: u32,
        character: u32,
        include_declaration: bool,
    ) -> Result<Vec<LspLocation>, LspQueryFailure>;

    /// Where the type at 0-based (`line`, `character`) of `path` is implemented.
    async fn go_to_implementation(
        &self,
        path: &Path,
        line: u32,
        character: u32,
    ) -> Result<Vec<LspLocation>, LspQueryFailure>;

    /// Every named symbol `path` defines.
    async fn document_symbol(&self, path: &Path) -> Result<Vec<LspSymbol>, LspQueryFailure>;
}
