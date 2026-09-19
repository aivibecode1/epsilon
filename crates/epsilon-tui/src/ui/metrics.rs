//! What the harness knows about itself, arranged for the person watching it.
//!
//! # Why this is a module and not three lines in a draw function
//!
//! Transparency is a feature, so it gets a data model. Every number here is
//! either reported by the provider or derived from reported numbers by a rule
//! stated in this file — and when a number is *not* known, the type says so
//! instead of printing a zero that reads like a real measurement.
//!
//! The three questions a developer actually asks mid-session:
//!
//! * **How much room is left?** → [`Metrics::context_used`] against
//!   [`Metrics::context_window`], shown as a share with a warning threshold.
//! * **Is the prompt cache working?** → [`Metrics::cache_hit_rate`], defined as
//!   the share of the *most recent* prompt that was served from cache. A
//!   session-wide average hides the thing being watched: whether the cache is
//!   tracking the current conversation.
//! * **What has this cost?** → [`Metrics::cost_micro_usd`], integer micro-USD,
//!   priced from the catalog for the active model. Unknown models report
//!   [`Metrics::cost`] as `None` rather than a confident `$0`.

use crate::ui::format::{format_cost, format_percent, format_tokens};

/// Context use at which the reader should start planning a compaction.
pub const CONTEXT_WARN: f64 = 0.70;
/// Context use at which the next turn may not fit at all.
pub const CONTEXT_DANGER: f64 = 0.90;

/// Everything the header reports, gathered in one place.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Metrics {
    /// Model id as the catalog spells it (display form, not the wire id).
    pub model_label: String,
    /// Whether the active model is the offline scripted provider.
    pub offline: bool,
    /// Tokens in the most recent prompt, as reported by the provider.
    ///
    /// `None` before the first reply of a session, and after a compaction until
    /// a fresh count arrives: in both cases the honest answer is "unknown", and
    /// printing the previous value would be a lie the reader cannot detect.
    pub context_used: Option<u64>,
    /// The active model's context window, when the catalog knows it.
    pub context_window: Option<u64>,
    /// Session totals.
    pub tokens_in: u64,
    /// Session totals.
    pub tokens_out: u64,
    /// Prompt tokens served from cache, session total.
    pub cache_read: u64,
    /// Prompt tokens written to cache, session total.
    pub cache_write: u64,
    /// Estimated session cost in micro-USD, or `None` when unpriced.
    pub cost_micro_usd: Option<u64>,
    /// `true` once the session has been compacted and no fresh count has arrived.
    pub compacted: bool,
}

/// How alarming the context gauge is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pressure {
    /// Plenty of room.
    Calm,
    /// Past [`CONTEXT_WARN`].
    Warn,
    /// Past [`CONTEXT_DANGER`].
    Danger,
    /// Not known yet.
    Unknown,
}

impl Metrics {
    /// The share of the context window the last prompt occupied.
    ///
    /// `None` when either half of the ratio is unknown — the caller prints a
    /// placeholder, never a fabricated zero.
    pub fn context_ratio(&self) -> Option<f64> {
        match (self.context_used, self.context_window) {
            (Some(used), Some(window)) if window > 0 => Some(used as f64 / window as f64),
            _ => None,
        }
    }

    /// How full the context is.
    pub fn pressure(&self) -> Pressure {
        match self.context_ratio() {
            None => Pressure::Unknown,
            Some(r) if r >= CONTEXT_DANGER => Pressure::Danger,
            Some(r) if r >= CONTEXT_WARN => Pressure::Warn,
            Some(_) => Pressure::Calm,
        }
    }

    /// The share of the last prompt that came from the cache.
    ///
    /// Numerator and denominator are deliberately from the same call: cache reads
    /// divided by the prompt they were part of. Dividing session cache reads by a
    /// session token total would produce a number that falls every time the model
    /// answers, which is the opposite of what the reader is watching for.
    ///
    /// `None` when there has been no prompt, or when the prompt was too small for
    /// a cache decision to mean anything (a 4-token prompt "hitting" the cache
    /// says nothing about prompt reuse).
    pub fn cache_hit_rate(&self) -> Option<f64> {
        let prompt = self.context_used?;
        if prompt < 1_000 {
            return None;
        }
        let hits = self.cache_read.min(prompt);
        Some(hits as f64 / prompt as f64)
    }

    /// The context gauge as the header prints it: `42%/200k`, or `?/200k`.
    pub fn context_gauge(&self) -> String {
        let window = match self.context_window {
            Some(w) if w > 0 => format_tokens(w),
            _ => return "ctx ?".to_string(),
        };
        match self.context_ratio() {
            Some(ratio) => format!("{}%/{window}", format_percent(ratio * 100.0)),
            // After a compaction the previous count describes a prompt that no
            // longer exists; "?" is the truth until the next reply reports one.
            None if self.compacted => format!("?/{window}"),
            None => format!("?/{window}"),
        }
    }

    /// The cache line as the header prints it: `cache 87%`, or `cache —`.
    pub fn cache_gauge(&self) -> String {
        match self.cache_hit_rate() {
            Some(rate) => format!("cache {}", format_percent(rate * 100.0)),
            None => "cache —".to_string(),
        }
    }

    /// Token totals as the header prints them: `↑12k ↓3.4k`.
    pub fn token_gauge(&self) -> String {
        format!(
            "↑{} ↓{}",
            format_tokens(self.tokens_in),
            format_tokens(self.tokens_out)
        )
    }

    /// Cost as the header prints it, or `None` when the model is unpriced.
    pub fn cost_gauge(&self) -> Option<String> {
        self.cost_micro_usd.map(format_cost)
    }
}

/// One piece of the header: the text, and how it should be drawn.
#[derive(Debug, Clone, PartialEq)]
pub struct MetricPart {
    /// Text as it appears, already formatted.
    pub text: String,
    /// Which semantic role to draw it with.
    pub role: MetricRole,
}

/// The roles a header fragment can take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetricRole {
    /// The model's name.
    Model,
    /// A healthy measurement.
    Value,
    /// A measurement worth noticing.
    Warn,
    /// A measurement at its limit.
    Danger,
    /// A label or a placeholder.
    Label,
}

impl Metrics {
    /// The right-hand side of the header, richest last.
    ///
    /// Order is the order of questions: what am I talking to, how much room is
    /// left, is the cache working, how much have I spent. Leading fragments are
    /// dropped first when the terminal narrows, so the model name — the thing
    /// that changes what every other number means — is the last to go.
    pub fn parts(&self) -> Vec<MetricPart> {
        let mut parts = Vec::new();
        let pressure_role = match self.pressure() {
            Pressure::Danger => MetricRole::Danger,
            Pressure::Warn => MetricRole::Warn,
            Pressure::Calm => MetricRole::Value,
            Pressure::Unknown => MetricRole::Label,
        };
        parts.push(MetricPart {
            text: self.context_gauge(),
            role: pressure_role,
        });
        parts.push(MetricPart {
            text: self.token_gauge(),
            role: MetricRole::Value,
        });
        parts.push(MetricPart {
            text: self.cache_gauge(),
            role: if self.cache_hit_rate().is_some() {
                MetricRole::Value
            } else {
                MetricRole::Label
            },
        });
        if let Some(cost) = self.cost_gauge() {
            parts.push(MetricPart {
                text: cost,
                role: MetricRole::Value,
            });
        }
        parts.push(MetricPart {
            text: self.model_label.clone(),
            role: if self.offline {
                MetricRole::Warn
            } else {
                MetricRole::Model
            },
        });
        if self.offline {
            parts.push(MetricPart {
                text: "offline".to_string(),
                role: MetricRole::Label,
            });
        }
        parts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metrics() -> Metrics {
        Metrics {
            model_label: "DeepSeek V3.1".into(),
            context_used: Some(24_000),
            context_window: Some(200_000),
            tokens_in: 12_400,
            tokens_out: 3_400,
            cache_read: 18_000,
            cache_write: 2_000,
            cost_micro_usd: Some(12_345),
            ..Metrics::default()
        }
    }

    #[test]
    fn an_unknown_context_is_reported_as_unknown() {
        let m = Metrics {
            context_window: Some(200_000),
            ..Metrics::default()
        };
        assert_eq!(m.context_gauge(), "?/200k");
        assert_eq!(m.pressure(), Pressure::Unknown);
        assert!(m.cache_hit_rate().is_none());
    }

    #[test]
    fn pressure_follows_the_documented_thresholds() {
        let mut m = metrics();
        m.context_used = Some(100_000);
        assert_eq!(m.pressure(), Pressure::Calm);
        m.context_used = Some(140_000);
        assert_eq!(m.pressure(), Pressure::Warn);
        m.context_used = Some(180_000);
        assert_eq!(m.pressure(), Pressure::Danger);
    }

    #[test]
    fn cache_rate_is_about_the_latest_prompt_not_the_session() {
        let mut m = metrics();
        // 18k of a 24k prompt came from cache.
        assert_eq!(m.cache_hit_rate(), Some(0.75));
        assert_eq!(m.cache_gauge(), "cache 75%");
        // A later, larger prompt with no cache hits must move the number, even
        // though the session total is unchanged.
        m.context_used = Some(48_000);
        assert_eq!(m.cache_hit_rate(), Some(0.375));
    }

    #[test]
    fn a_tiny_prompt_does_not_claim_a_cache_hit() {
        let m = Metrics {
            context_used: Some(120),
            context_window: Some(200_000),
            cache_read: 120,
            ..Metrics::default()
        };
        assert!(m.cache_hit_rate().is_none(), "120 tokens proves nothing");
        assert_eq!(m.cache_gauge(), "cache —");
    }

    #[test]
    fn cache_reads_cannot_exceed_the_prompt_they_belong_to() {
        // A provider that reports more cache reads than prompt tokens must not
        // produce a rate above 100%.
        let m = Metrics {
            context_used: Some(10_000),
            cache_read: 99_000,
            ..Metrics::default()
        };
        assert_eq!(m.cache_hit_rate(), Some(1.0));
    }

    #[test]
    fn an_unpriced_model_omits_cost_instead_of_claiming_zero() {
        let m = Metrics {
            cost_micro_usd: None,
            ..metrics()
        };
        assert!(m.cost_gauge().is_none());
        assert!(!m.parts().iter().any(|p| p.text.starts_with('$')));
    }

    #[test]
    fn the_model_name_is_last_so_it_survives_a_narrow_terminal() {
        let parts = metrics().parts();
        let last = parts.last().expect("at least one fragment");
        assert_eq!(last.text, "DeepSeek V3.1");
        assert_eq!(last.role, MetricRole::Model);
    }

    #[test]
    fn an_offline_model_is_labelled_not_merely_coloured() {
        let m = Metrics {
            offline: true,
            ..metrics()
        };
        let parts = m.parts();
        assert!(parts.iter().any(|p| p.text == "offline"));
    }
}
