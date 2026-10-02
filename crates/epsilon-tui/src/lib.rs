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
pub mod at_files;
pub mod bench;
pub mod clipboard;
pub mod editor_ops;
pub mod fuzzy;
pub mod git;
pub mod keybindings;
pub mod kill_ring;
pub mod latex;
pub mod markdown;
pub mod picker;
pub mod render_cache;
pub mod renderers;
pub mod scroll;
/// What a tool card says, per tool. See the module docs.
/// Finding text in the visible transcript. See the module docs for what replaced.
pub mod search;
pub mod state;
/// Making text from tools, files and models safe to *show*. See the module docs for why a
/// bidirectional override in a command line is an approval prompt that lies.
pub mod text_safety;
pub mod trust_store;
pub mod ui;
pub mod undo;
pub mod wrap;

pub use app::{LiveFetcher, TuiConfig, run};
pub use ui::theme::ThemeMode;
