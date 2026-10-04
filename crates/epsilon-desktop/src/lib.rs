//! Epsilon Desktop — a **native** IDE window onto the same agent loop the CLI drives.
//!
//! # Why this crate exists
//!
//! Epsilon has two frontends today: a terminal UI and a headless/served mode. This
//! crate is the third — a real desktop application, in the operating system's own
//! window, rendering natively rather than through an embedded web view.
//!
//! # The one rule
//!
//! **This crate owns pixels. It does not own agent behaviour.**
//!
//! Every turn, tool call, permission decision and compaction in this window is the
//! same `epsilon_core::Agent` the terminal UI drives, through the same
//! [`ApprovalUi`] seam, over the same session files. There is no second agent loop,
//! no second permission engine, and no desktop-only shortcut around a gate. If the
//! desktop can do something, the terminal can do the same thing, because it is the
//! same machinery behind a different draw call.
//!
//! That rule is why this crate may not depend on `epsilon-cli`: composition lives in
//! `epsilon-runtime` (ADR-021) and both frontends call it, so neither can fork it.
//!
//! # Module map
//!
//! | Module | Owns |
//! |---|---|
//! | [`theme`] | The palette, adapted from the terminal's semantic roles to egui colors |
//! | [`layout`] | Which region sits where, and how the layout degrades as the window shrinks |
//! | [`app`] | Application state and the frame |
//!
//! [`ApprovalUi`]: epsilon_core::gate::ApprovalUi

#![deny(missing_docs)]

/// Compile-time marker that this crate declares the `real` feature (ADR-020).
///
/// `Agent::try_hot_swap_to_real` compiles to a `false` stub when `epsilon-core` was
/// built without `real`, and the failure is silent: the window opens, names a paid model
/// in its header, and never calls it. `epsilon-cli` binds the same marker
/// (`crates/epsilon-cli/src/main.rs`).
///
/// **What this does and does not catch**, stated plainly rather than implied: it fails
/// only if `epsilon-core` stops exporting the symbol, which it does not. The real
/// protection is `real = ["epsilon-core/real", "epsilon-runtime/real"]` in this crate's
/// `Cargo.toml` plus `default = ["real"]`, and the thing that will catch a future
/// regression is the `--list-models` / `doctor` output agreeing with what the window says.
#[cfg(feature = "real")]
const _REAL_PATH_FORWARDED: () = epsilon_core::REAL_PATH_ENABLED;

pub mod app;
pub mod commands;
pub mod diff;
/// Rasterise a real frame offscreen. **Test-only**: it needs `wgpu`, `egui-wgpu` and
/// `png`, which are dev-dependencies precisely so this cannot reach the binary. See the
/// module's own doc for why an offscreen path is the one that works on this machine.
#[cfg(test)]
mod frame_raster;
pub mod layout;
pub mod markup;
pub mod session;
pub mod theme;
pub mod workspace;

pub use app::IdeApp;
pub use diff::{Change, ChangeWatcher, DiffLine};
pub use layout::IdeLayout;
pub use session::{IdeSession, Row, RowKind, Transcript};
pub use theme::IdeTheme;
pub use workspace::{FileTree, OpenFile, Row as TreeRow};

/// How the desktop window is opened.
#[derive(Debug, Clone)]
pub struct Options {
    /// Workspace root the agent is bound to.
    pub workspace: std::path::PathBuf,
    /// Which palette to use. `None` follows `EPSILON_THEME`, the same way the
    /// terminal does, so a user who chose `light` for `epsilon` gets it here too.
    pub theme: Option<epsilon_tui::ThemeMode>,
    /// Session file to open, or `None` to start a new one the way a bare `epsilon`
    /// launch does.
    pub session: Option<std::path::PathBuf>,
    /// Model to run on. `None` resolves through the same precedence the CLI uses.
    pub model_id: Option<String>,
}

impl Options {
    /// Options for a plain launch in `workspace`, with everything else defaulted.
    pub fn for_workspace(workspace: impl Into<std::path::PathBuf>) -> Self {
        Self {
            workspace: workspace.into(),
            theme: None,
            session: None,
            model_id: None,
        }
    }
}

/// Open the IDE window and run until the user closes it.
///
/// # Order matters here, and it is not obvious
///
/// The agent is composed **before** the window opens. `build_runtime` scans extension
/// manifests, opens a session lease and may migrate a session file; doing that after
/// `run_native` would mean a window that shows an empty agent panel for as long as the
/// scan takes, which is precisely the "nothing happened" state the terminal frontend's
/// invariant exists to forbid. It is also the same reason a launch that *fails* — an
/// unreadable session, a model with no credentials — produces a sentence and no window,
/// rather than a window that cannot do anything.
///
/// The runtime is created here and owned by the [`IdeSession`], because the window loop
/// owns the thread: egui repaints on the thread that owns the winit event loop, and every
/// agent call that has to touch the loop runs on this runtime.
///
/// The error is a [`String`] rather than `eframe::Error` on purpose. `eframe`'s own
/// errors are about the window and the adapter, which the caller cannot act on; what
/// a person running this needs is a sentence, and mapping an opaque library error
/// through `Display` into one message is what the CLI's `die()` does everywhere else.
pub fn run(opts: Options) -> Result<(), String> {
    let runtime = std::sync::Arc::new(
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|e| format!("could not start the agent runtime: {e}"))?,
    );

    // The same resolution the CLI runs, through the same function (ADR-021). A window
    // that decided its own model would be a window that could name a provider it is not
    // calling — the M233 defect, in a new place.
    let request = epsilon_runtime::LaunchRequest {
        workspace: opts.workspace.clone(),
        model_reference: opts.model_id.clone(),
        resume: match opts.session.clone() {
            Some(path) => epsilon_runtime::Resume::Path(path),
            None => epsilon_runtime::Resume::No,
        },
        ..epsilon_runtime::LaunchRequest::for_workspace(opts.workspace.clone())
    };
    let launch = epsilon_runtime::resolve_launch(&request)?;
    let notices = launch.notices.clone();

    let session = runtime.block_on(IdeSession::start(runtime.clone(), launch))?;

    let title = opts
        .workspace
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("Epsilon")
        .to_string();

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 820.0])
            // **Above `MEDIUM` (860), deliberately.**
            //
            // The sidebar can now be resized, folded with `Ctrl+B`, and remembered across
            // launches — and at the old 720×480 minimum it was none of those things at the only
            // size a window cannot be dragged away from: below 860 the layout folds the sidebar
            // for narrowness, so a reader who sized it, collapsed it, or restored it would be
            // shown a window that silently disagreed with their own choice, and could not drag it
            // back. A minimum that contradicts a control is not a minimum, it is a bug waiting to
            // be resized into.
            //
            // The layout's own degradation thresholds are untouched: a user can still shrink the
            // window below this with a different window manager or a display change, and the
            // tiers still fall away in order. This only says where the guarantee *starts*.
            .with_min_inner_size([900.0, 560.0])
            .with_title(title),
        ..Default::default()
    };

    // **Not prefixed here.** `main` adds "could not open the Epsilon window" to whatever
    // comes back, so phrasing it here as well produced, on the one machine where this has
    // been run end to end, "could not open the Epsilon window: could not open the Epsilon
    // window: WGPU error: Parent device is lost" — the message a reader meets at exactly
    // the moment they need it most, doubled. This function is public, so its error is still
    // self-describing; it just is not dressed twice.
    eframe::run_native(
        "Epsilon",
        native_options,
        Box::new(move |cc| Ok(Box::new(IdeApp::new(opts, session, notices, &cc.egui_ctx)))),
    )
    .map_err(|e| e.to_string())
}
