//! Terminal frontend on ratatui + crossterm (M9b).
//!
//! Shape: a pure event reducer ([`state::UiState`]) plus a thin terminal runtime
//! ([`app::run`]). Permission asks flow through the shared [`epsilon_safety::
//! ChannelApprovalUi`], so the TUI dialog, the serve-mode REST endpoint, and tests all
//! answer gates through one mechanism.
//!
//! Offline-first: like print mode, the model is driven by the faux provider (a script via
//! `--script`, or a small default). Real providers plug in behind the existing `real`
//! feature without touching this crate's logic.

pub mod app;
pub mod editor_ops;
pub mod markdown;
pub mod picker;
pub mod scroll;
pub mod state;
pub mod theme;

pub use app::{LiveFetcher, TuiConfig, run};
