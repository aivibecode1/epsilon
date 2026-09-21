//! The TUI's presentation layer.
//!
//! Everything that decides *how the harness looks* lives here, and nothing in
//! here reaches for application state. A widget receives plain values — the
//! metrics, the run state, the rows to draw — which has two consequences worth
//! the discipline:
//!
//! * A screen can be rendered in a test without an agent, a terminal, or a
//!   network, so a visual regression is a failed unit test rather than a bug
//!   report from someone's console.
//! * There is exactly one file to open for any "why does it look like that"
//!   question, and it is small.
//!
//! Modules:
//!
//! | Module | Owns |
//! |---|---|
//! | [`theme`] | Colours, glyphs, `NO_COLOR` and 16-colour fallbacks |
//! | [`layout`] | The spatial model: which region sits where, and how it degrades |
//! | [`metrics`] | What the harness knows about itself, and the rules for showing it |
//! | [`format`] | Number, cost, duration and path formatting |
//! | [`header`] | Run state + live measurements |
//! | [`transcript`] | Gutter, role separation, first-run screen |
//! | [`composer`] | The draft, the caret, the rule above it |
//! | [`footer`] | The keys that work right now, and where the session lives |
//! | [`overlay`] | Modal chrome: panels, prompts, list rows |
//! | [`shell`] | Assembling one frame from application state |

pub mod composer;
pub mod cost;
pub mod footer;
pub mod format;
pub mod header;
pub mod layout;
pub mod metrics;
pub mod overlay;
pub mod shell;
pub mod theme;
pub mod transcript;
