//! Text that is safe to *show*.
//!
//! **The rule moved to [`epsilon_runtime::text_safety`] (M272)** and this is a re-export of
//! it, so the ten call sites in this crate did not all churn and there is still exactly one
//! implementation.
//!
//! It moved because the desktop window needs the same rule and had none: a `U+202E` inside a
//! command reached its approval screen unstripped, while the terminal has refused to *approve*
//! an ask carrying display-steering characters since M261. A window that strips is better; a
//! window that **cannot approve what it cannot show** is the actual fix, and both halves of
//! that rule are now where both frontends can reach them.

pub use epsilon_runtime::text_safety::{Stripped, display_safe};
