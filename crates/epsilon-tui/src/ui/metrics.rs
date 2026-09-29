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
//!   `context_used` is the *session context* — the same number the compaction
//!   trigger reads — not the last call's prompt size, which is what it used to
//!   be. Those two agree on turn 1 and diverge afterwards, and the divergence is
//!   always in the flattering direction (see [`Metrics::context_used`]).
//! * **Is the prompt cache working?** → [`Metrics::cache_hit_rate`], defined as
//!   the share of the *most recent* prompt that was served from cache. A
//!   session-wide average hides the thing being watched: whether the cache is
//!   tracking the current conversation.
//! * **What has this cost?** → [`Metrics::cost_micro_usd`], integer micro-USD,
//!   priced from the catalog for the active model. Unknown models report
//!   [`Metrics::cost`] as `None` rather than a confident `$0`.

use crate::ui::format::{format_cache_percent, format_cost, format_percent, format_tokens};

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
    /// How full the session's context is: the tokens the **next** request will
    /// carry, in the model view.
    ///
    /// This is `Agent::estimated_context_tokens` — provider truth for the prefix
    /// a request already covered, plus a heuristic over whatever was appended
    /// since — so the header agrees with the number the compaction trigger
    /// decides on rather than with a number of its own.
    ///
    /// It used to be `latest_prompt_tokens`: the prompt size of the *last call*.
    /// That is a true number and the wrong one. On turn 1 of a 200k model with a
    /// 14k prompt it is 14k; on turn 40 it is still only what the last request
    /// sent, so the header reported `?/200k` before the first reply and a
    /// percentage that lagged the conversation by a whole turn afterwards. A
    /// reader who trusts it plans a compaction on a context that is already
    /// larger than the number printed — the one direction a capacity figure must
    /// never be wrong in.
    ///
    /// `None` only when nothing is known at all: an empty session, or a session
    /// whose history cannot be measured. The gauge then prints `?`, never a zero,
    /// because `0%/200k` reads as a measurement rather than as an absence.
    pub context_used: Option<u64>,
    /// The active model's context window, when the catalog knows it.
    pub context_window: Option<u64>,
    /// Whether [`Metrics::context_used`] is a local estimate rather than provider truth.
    ///
    /// True on turn one, after every compaction, and after `/resume` — the anchor is
    /// what makes the figure a measurement, and those are the moments it is missing.
    /// The gauge marks it rather than silently presenting a heuristic in the same
    /// typeface as a provider's own count.
    pub context_estimated: bool,
    /// Session totals.
    pub tokens_in: u64,
    /// Session totals.
    pub tokens_out: u64,
    /// Prompt tokens served from cache, session total.
    ///
    /// A session total is the wrong numerator for the hit rate — see
    /// [`Metrics::cache_read_last`] — but it is the honest answer to "how much of this
    /// session came from cache", which a wider layout can show.
    pub cache_read_session: u64,
    /// Prompt tokens written to cache, session total.
    pub cache_write_session: u64,
    /// Prompt tokens served from cache in the **most recent call** — the numerator that
    /// belongs with [`Metrics::cache_read_last`]'s denominator, the provider-reported
    /// prompt size of that same call. It is deliberately *not* [`Metrics::context_used`]:
    /// the rate is a claim about one request, so both halves of it must come from that
    /// request, while the context gauge is a claim about the whole session.
    ///
    /// This pairing is the whole point. A hit rate is a claim about one prompt: cache
    /// reads from earlier calls divided by the latest prompt size inflates every turn and
    /// pins the gauge at 100% once the cumulative reads pass one prompt (G16, found by
    /// reading this file against its own doc comment).
    pub cache_read_last: u64,
    /// The provider-reported prompt size of the most recent call
    /// (`input + cache_read + cache_write`), or `None` before the first reply of a
    /// session.
    ///
    /// The denominator of [`Metrics::cache_hit_rate`], and the only figure here that
    /// comes straight from a provider rather than from a heuristic.
    pub prompt_tokens_last: Option<u64>,
    /// Whether the provider actually **reported** cache fields (K1).
    ///
    /// The one thing `cache_read`/`cache_write` cannot express. A wire that speaks
    /// about caching and a wire that does not both leave those counters at zero, so
    /// the counts alone render "this provider cannot report cache traffic" as
    /// `CH 0%` — a confident, styled number that reads as a cache which is present
    /// and failing. The faux provider, ollama, and any OpenAI-compatible proxy that
    /// omits `prompt_tokens_details` all land there.
    ///
    /// Carried from `Usage::cache_reported` so the answer comes from the response
    /// body rather than from a static per-provider guess: a gateway can route one
    /// model id to backends that report differently, and only the reply knows which
    /// one answered.
    pub cache_reported: bool,
    /// Estimated session cost in micro-USD, or `None` when unpriced.
    pub cost_micro_usd: Option<u64>,
    /// True when [`Metrics::cost_micro_usd`] is a **lower bound**, not the total.
    ///
    /// That happens when the session moved cache tokens and the registry publishes no
    /// price for them: the tokens are real, their price is unknown, and counting them at
    /// zero is what made the earlier estimate quietly low. The header then prints
    /// `$0.42+`, so the reader is told which kind of number they are looking at instead of
    /// being given a precise-looking one that is wrong (G18).
    pub cost_is_floor: bool,
    /// True when the context size is unknown *because a compaction has not yet
    /// been re-measured*.
    ///
    /// Distinct from "unknown because nothing has run yet", which is the state
    /// every session starts in. Mirrors the window, not the whole file: once a
    /// fresh count arrives the flag is cleared and the gauge goes back to
    /// describing an ordinary measurement. See [`Metrics::CONTEXT_UNKNOWN_COMPACTED`].
    pub compacted: bool,
}

/// A share as the gauge prints it, keeping the difference between "small" and
/// "nothing".
///
/// [`crate::ui::format::format_percent`] rounds to whole percent, which turns a
/// measured 0.08% into `0%` — the exact string the gauge avoids printing when
/// there is no measurement at all. A reader cannot tell a context that is empty
/// from one that is merely early, and on a wide window the first several turns
/// are all below one percent. `<1%` says what is true: there is something there,
/// and it rounds away.
fn percent_or_floor(ratio: f64) -> String {
    let percent = ratio * 100.0;
    if percent > 0.0 && percent < 0.5 {
        return "<1%".to_string();
    }
    format_percent(percent)
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
    /// The share of the context window the session's context occupies.
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
    /// Numerator and denominator come from the same call by construction:
    /// [`Metrics::cache_read_last`] is the cached share of the prompt
    /// [`Metrics::prompt_tokens_last`] measures. Dividing the *session* total by one
    /// prompt would produce a number that climbs every turn and sits at 100% thereafter,
    /// which is the opposite of what the reader is watching for — and dividing it by
    /// [`Metrics::context_used`] would do the same thing more quietly, because the
    /// context grows while the cached prefix does not.
    ///
    /// `None` when there has been no prompt, or when the prompt was too small for a
    /// cache decision to mean anything (a 4-token prompt "hitting" the cache says
    /// nothing about prompt reuse).
    ///
    /// Also `None` when the provider never reported cache fields at all (K1) — see
    /// [`Metrics::cache_reported`]. A percentage is an arithmetic claim about two
    /// numbers the provider did not send, and printing `0%` there would assert a
    /// fact about the user's cache that nothing observed.
    pub fn cache_hit_rate(&self) -> Option<f64> {
        if !self.cache_reported {
            return None;
        }
        let prompt = self.prompt_tokens_last?;
        if prompt < 1_000 {
            return None;
        }
        // A provider that reports more cache reads than prompt tokens is contradicting
        // itself; clamping keeps the gauge inside 0..=100% instead of reporting a lie
        // with more decimal places.
        let hits = self.cache_read_last.min(prompt);
        Some(hits as f64 / prompt as f64)
    }

    /// The `?` the context gauge shows when the size of the context is unknown.
    ///
    /// Named so the header's documentation and this string cannot drift apart:
    /// the audit that produced this file found the module promising a `?` that a
    /// later edit had stopped printing.
    pub const CONTEXT_UNKNOWN: &'static str = "ctx ?";

    /// The same unknown, with the reason attached: `ctx ? · compacted`.
    ///
    /// Two things can make the context size unreadable — no window has been
    /// measured yet, or a compaction just replaced the count and no fresh one
    /// has landed — and they are very different things to a reader. The first is
    /// the normal state of a session a moment after it starts. The second means
    /// the number the compaction trigger reads is, right now, an estimate with
    /// nothing behind it, and the next turn's threshold decision is being made on
    /// that estimate.
    ///
    /// The flag that knows the difference has been carried in `UiState` since the
    /// compaction row shipped, set on `CompactionFinished` and cleared by the
    /// first reply that reports a fresh count. It had no reader for a while,
    /// which is not the same as having no value: it is the only thing in the app
    /// that knows a compaction has happened but has not yet been re-measured.
    pub const CONTEXT_UNKNOWN_COMPACTED: &'static str = "ctx ? · compacted";

    /// The context gauge as the header prints it: `ctx 42%/200k`, or `ctx ?`.
    ///
    /// The word `ctx` is not decoration. Without it the fragment reads as
    /// `42%/200k`, and a reader has to guess what the 42% is *of*: it was the
    /// last prompt's share of the window, which is a different (and always
    /// smaller) number than the session's, so the guess a reader made was the
    /// flattering one. With it, the fragment names its own subject and the
    /// number behind it is the session estimate.
    pub fn context_gauge(&self) -> String {
        let Some(window) = self.context_window.filter(|w| *w > 0) else {
            return if self.compacted {
                Self::CONTEXT_UNKNOWN_COMPACTED.to_string()
            } else {
                Self::CONTEXT_UNKNOWN.to_string()
            };
        };
        let window = format_tokens(window);
        // The provenance mark. A `*` after the percentage, and only then: the figure
        // is a heuristic estimate whenever the usage anchor is missing, and the
        // reader's ability to trust it depends on knowing which they are looking at.
        // Hermes publishes the same distinction as a field on every status payload
        // (`context_breakdown.py:94-115`).
        let mark = if self.context_estimated { "*" } else { "" };
        match self.context_ratio() {
            Some(ratio) => format!("ctx {}{mark}/{window}", percent_or_floor(ratio)),
            // A window with no measurement behind it: `?/200k`. Reading the
            // window is still worth a reader's glance — it says which model is
            // in play — so the unknown mark replaces the number, not the pair.
            //
            // The compaction mark rides here rather than replacing the `?`. A
            // window with a number missing and a window with a *fresh* count
            // behind it are the same two situations, and the second is the one
            // worth naming: `?/200k*` says the number is estimated, and after a
            // compaction that estimate is standing in for a count the harness
            // threw away. The `· compacted` is what tells a reader the gauge is
            // describing the state *between* a compaction and the reply that
            // re-measures it.
            None if self.compacted => format!("?/{window}{mark} · compacted"),
            None => format!("?/{window}{mark}"),
        }
    }

    /// The cache line as the header prints it: `cache 87%`, or `cache —`.
    pub fn cache_gauge(&self) -> String {
        // Volume *and* rate, because a rate without a denominator is a vibe. `CH 100%` on a
        // 200-token cache and on a 2M-token cache are the same string and mean opposite
        // things; the R/W counters are what make the number mean something. This is the same
        // breakdown pi's footer prints.
        let volume = format!(
            "R{} W{}",
            format_tokens(self.cache_read_session),
            format_tokens(self.cache_write_session)
        );
        match self.cache_hit_rate() {
            Some(rate) => format!("{volume} CH {}", format_cache_percent(rate)),
            // Two different reasons for no rate, and they must not look alike.
            //
            // A provider that never reports cache traffic gets `n/a`: the number is
            // *unknowable*, and saying so is the honest reading. `CH —` is kept for
            // "a rate exists but there is not one yet" — no prompt, or a prompt too
            // small for a cache decision to mean anything — where a percentage would
            // be premature rather than impossible. Collapsing both into one dash is
            // what let a provider with caching switched off look like a cache that
            // is present and failing (K1).
            //
            // Volume stays in both: a session that already moved 400k cached tokens
            // must not read as "no cache" (the earlier bug), and `R400k W0` beside
            // `CH n/a` is the true sentence — tokens moved, the provider will not say
            // what fraction came from cache.
            None if !self.cache_reported => format!("{volume} CH n/a"),
            None => format!("{volume} CH —"),
        }
    }

    /// The row a session shows before anything has happened: no usage, no cache, no cost.
    ///
    /// Used by the welcome panel and by tests that need a well-formed `Metrics` without
    /// inventing usage numbers. Constructing the struct literally in every test meant a dozen
    /// call sites that all had to change when a field was added.
    pub fn idle() -> Self {
        Self {
            model_label: "no model".to_string(),
            context_used: None,
            context_window: None,
            // Nothing has been measured, so nothing is claimed to be a measurement.
            context_estimated: true,
            tokens_in: 0,
            tokens_out: 0,
            cache_read_session: 0,
            cache_write_session: 0,
            cache_read_last: 0,
            prompt_tokens_last: None,
            // Nothing has been reported, so nothing is claimed to be measurable (K1).
            cache_reported: false,
            cost_micro_usd: None,
            cost_is_floor: false,
            offline: false,
            // Nothing has been compacted, so the `?` has its ordinary meaning.
            compacted: false,
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
    ///
    /// A trailing `+` marks a lower bound — the session moved cache tokens the registry
    /// could not price — so `$0.42+` reads as "at least 42 cents" rather than as a total.
    pub fn cost_gauge(&self) -> Option<String> {
        self.cost_micro_usd.map(|micro| {
            let text = format_cost(micro);
            if self.cost_is_floor {
                format!("{text}+")
            } else {
                text
            }
        })
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
            cache_read_session: 18_000,
            cache_write_session: 2_000,
            cache_read_last: 18_000,
            prompt_tokens_last: Some(24_000),
            // This fixture is a provider that reports cache traffic; the K1 test
            // below is the one that deliberately does not.
            cache_reported: true,
            cost_micro_usd: Some(12_345),
            ..Metrics::default()
        }
    }

    /// A context size that is unknown is one of three different facts, and the
    /// gauge has to be able to tell them apart.
    ///
    /// The case added here is the middle one. `UiState` has carried a `compacted`
    /// flag since the compaction row shipped — set when a compaction replaces the
    /// count, cleared by the first reply that reports a fresh one — and for a
    /// while nothing read it. It is the only thing in the app that knows the
    /// difference between "the context is unknown because this session has not
    /// run yet" and "the context is unknown because it was just thrown away", and
    /// the second is the one a reader watching the compaction trigger needs: it
    /// means the number deciding the next threshold is an estimate with nothing
    /// behind it.
    #[test]
    fn a_compaction_makes_an_unknown_context_say_why() {
        let window_only = Metrics {
            context_window: Some(200_000),
            context_used: None,
            ..Metrics::default()
        };
        assert_eq!(window_only.context_gauge(), "?/200k");
        assert!(
            !window_only.context_gauge().contains("compacted"),
            "an ordinary unknown must not claim a compaction happened"
        );

        let compacted = Metrics {
            compacted: true,
            ..window_only.clone()
        };
        assert_eq!(compacted.context_gauge(), "?/200k · compacted");

        // And a fresh count retires the mark, because the flag is cleared by the
        // same event that restores the number.
        let measured = Metrics {
            context_used: Some(50_000),
            compacted: true,
            ..window_only
        };
        // A measured count speaks for itself, so the compaction note is not
        // added — it explains an *unknown*, and this is a known number. The `*`
        // is the provenance mark, and it rides on the percentage, not on the
        // compaction note.
        let measured_estimate = Metrics {
            context_estimated: true,
            ..measured.clone()
        };
        assert_eq!(measured_estimate.context_gauge(), "ctx 25%*/200k");
    }

    /// The other two cases, for the same reason: a window with no measurement and
    /// a session with no window at all must not read alike.
    #[test]
    fn the_three_kinds_of_unknown_context_stay_distinguishable() {
        let no_window = Metrics::default();
        assert_eq!(no_window.context_gauge(), "ctx ?");
        let no_window_compacted = Metrics {
            compacted: true,
            ..Metrics::default()
        };
        assert_eq!(no_window_compacted.context_gauge(), "ctx ? · compacted");
        assert_ne!(
            no_window.context_gauge(),
            no_window_compacted.context_gauge(),
            "a compaction must be visible even when there is no window to print"
        );
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
    fn the_context_fragment_says_what_the_number_is() {
        // The defect: `42%/200k` reads as window occupancy and invites the
        // reader to assume it measures the session, while the number behind it
        // measured one prompt. A bare percentage may not be printed at all.
        let m = metrics();
        let gauge = m.context_gauge();
        assert_eq!(gauge, "ctx 12%/200k");
        assert!(
            gauge.starts_with("ctx "),
            "a percentage with no subject reads as a different claim: {gauge:?}"
        );
        // And the fragment the header actually draws is that same one.
        let parts = m.parts();
        assert_eq!(parts[0].text, gauge);
    }

    #[test]
    fn a_measured_context_too_small_to_round_is_not_printed_as_empty() {
        // A wide window makes the first several turns sub-percent: 150 tokens of
        // 200k is 0.075%, which rounds to `0%` — the exact string `?` exists to
        // avoid, and one that reads as "the context is empty" when the context
        // has something in it.
        let tiny = Metrics {
            context_used: Some(150),
            context_window: Some(200_000),
            ..Metrics::default()
        };
        assert_eq!(tiny.context_gauge(), "ctx <1%/200k");
        assert!(
            !tiny.context_gauge().contains("0%"),
            "a measured context must never print as empty"
        );
        // Unknown is still unknown, and a real percentage is unchanged.
        assert_eq!(
            Metrics {
                context_window: Some(200_000),
                ..Metrics::default()
            }
            .context_gauge(),
            "?/200k"
        );
        assert_eq!(
            Metrics {
                context_used: Some(1_000),
                context_window: Some(200_000),
                ..Metrics::default()
            }
            .context_gauge(),
            "ctx 1%/200k",
            "exactly half a percent rounds up, as it always did"
        );
        assert_eq!(
            Metrics {
                context_used: Some(9_000),
                context_window: Some(200_000),
                ..Metrics::default()
            }
            .context_gauge(),
            "ctx 5%/200k"
        );
    }

    #[test]
    fn the_context_gauge_and_the_cache_rate_measure_different_calls() {
        // The two numbers share a denominator-shaped hole and must not share a
        // numerator. A session context of 40k with a last prompt of 24k that hit
        // the cache 18k times is a 75% hit rate and 20% of the window — reading
        // the hit rate against the context (18/40) or the window share against
        // the prompt (18/24) are both one edit away and both wrong.
        let m = Metrics {
            context_used: Some(40_000),
            prompt_tokens_last: Some(24_000),
            cache_read_last: 18_000,
            context_window: Some(200_000),
            ..metrics()
        };
        assert_eq!(m.context_gauge(), "ctx 20%/200k");
        assert_eq!(m.cache_hit_rate(), Some(0.75));

        // Before the first reply there is a context estimate and no prompt to
        // compute a hit rate from: the rate must be unknown rather than 100%.
        let fresh = Metrics {
            context_used: Some(40_000),
            prompt_tokens_last: None,
            cache_read_last: 0,
            ..metrics()
        };
        assert!(fresh.cache_hit_rate().is_none());
        // Session volume is real and is shown; the *rate* is withheld, because a rate
        // computed from a 120-token prompt is a number with no meaning.
        assert_eq!(fresh.cache_gauge(), "R18k W2.0k CH —");
        assert_eq!(fresh.context_gauge(), "ctx 20%/200k");
    }

    #[test]
    fn a_provider_that_cannot_report_cache_never_prints_a_percentage() {
        // K1. This is the defect in one test: a wire that says nothing about caching
        // and a wire that says "zero of your prompt was cached" both leave
        // `cache_read == 0`, and the header rendered the first as the second — a
        // confident, styled `CH 0%` that reads as a cache which is present and
        // failing. The user is being told their cache is broken.
        //
        // The three states are now three different sentences:
        //   not measurable      -> `CH n/a`  (the provider will not say)
        //   measurable, no rate -> `CH —`    (no prompt yet / too small)
        //   measured            -> `CH 75%`  (a real number about a real call)
        // Built from `Metrics::default()` rather than `..metrics()`, because the
        // spread would overwrite exactly the fields this test is about.
        let silent = Metrics {
            cache_read_session: 18_000,
            cache_write_session: 2_000,
            prompt_tokens_last: Some(24_000),
            cache_read_last: 0,
            cache_reported: false,
            ..Metrics::default()
        };
        assert!(
            silent.cache_hit_rate().is_none(),
            "a provider that reports no cache fields has no hit rate, and a rate is a \
             claim about two numbers that were never sent"
        );
        assert_eq!(
            silent.cache_gauge(),
            "R18k W2.0k CH n/a",
            "a wire that cannot report cache traffic must not be rendered as a failing cache"
        );
        // The volume still shows: a session that moved 400k tokens read as "no cache"
        // once already, and hiding the counters is the older version of this same lie.
        assert!(
            silent.cache_gauge().contains("R18k"),
            "volume is a real measurement and survives an unmeasurable rate"
        );

        // A measured zero is a real answer and must still be a percentage. If this
        // arm were dropped, "not measurable" and "measured 0%" would have merged
        // back together, which is the defect again from the other side.
        let measured_zero = Metrics {
            prompt_tokens_last: Some(24_000),
            cache_read_last: 0,
            cache_reported: true,
            ..Metrics::default()
        };
        assert_eq!(
            measured_zero.cache_hit_rate(),
            Some(0.0),
            "a provider that reports zero cached tokens reported a zero, not a silence"
        );
        assert_eq!(measured_zero.cache_gauge(), "R0 W0 CH 0%");

        // The two no-rate states stay distinct from each other.
        let no_prompt_yet = Metrics {
            cache_read_session: 18_000,
            cache_write_session: 2_000,
            prompt_tokens_last: None,
            cache_reported: true,
            ..Metrics::default()
        };
        assert_eq!(
            no_prompt_yet.cache_gauge(),
            "R18k W2.0k CH —",
            "a provider that can report, with no prompt yet, is 'not yet', not 'never'"
        );
    }

    #[test]
    fn the_cache_gauge_never_confuses_a_silence_with_a_zero() {
        // The property above, stated as the thing a reader of the header needs: no
        // input can produce the same string for "not measurable" and "measured 0%".
        // Asserted on the two gauges rather than on the flag, because the flag is
        // an implementation detail and the string is what the user actually reads.
        let silent = Metrics {
            prompt_tokens_last: Some(24_000),
            cache_reported: false,
            ..metrics()
        };
        let zero = Metrics {
            prompt_tokens_last: Some(24_000),
            cache_reported: true,
            ..metrics()
        };
        assert_ne!(
            silent.cache_gauge(),
            zero.cache_gauge(),
            "the header must be able to tell 'cannot measure' from 'measured zero'"
        );
    }

    /// A figure and the mark that says how it was made.
    ///
    /// The number is the same either way; what differs is whether a reader can tell a
    /// provider's count from a 4-chars-per-token guess. They cannot otherwise, and the
    /// moments where it *is* a guess — turn one, after a compaction, after `/resume` —
    /// are exactly the moments a user is watching the figure. Hermes ships the same
    /// distinction on every status payload for this reason.
    #[test]
    fn an_estimated_context_is_marked_as_one() {
        let measured = Metrics {
            context_used: Some(40_000),
            context_window: Some(200_000),
            context_estimated: false,
            ..metrics()
        };
        assert_eq!(measured.context_gauge(), "ctx 20%/200k");

        let estimated = Metrics {
            context_estimated: true,
            ..measured
        };
        assert_eq!(
            estimated.context_gauge(),
            "ctx 20%*/200k",
            "a local estimate must not be presented in the same typeface as a \
             provider's own count"
        );
        // The mark rides on the percentage, never on the window: the window is a
        // capacity from the catalog and is a measurement whenever it is known at all.
        assert!(estimated.context_gauge().ends_with("/200k"));
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
        assert_eq!(m.cache_gauge(), "R18k W2.0k CH 75%");

        // A later prompt that hit nothing must read as a *miss*, even though the session
        // still holds 18k of cache reads. This is the assertion the pre-G16 gauge failed:
        // it divided the session total by this prompt and reported
        // min(18_000, 48_000) / 48_000 = 37.5%, a hit rate for a prompt that had none.
        m.prompt_tokens_last = Some(48_000);
        m.cache_read_last = 0;
        assert_eq!(
            m.cache_hit_rate(),
            Some(0.0),
            "a prompt with no cache reads is a 0% hit rate, whatever the session total says"
        );

        // And a later prompt that hit half of itself reads as 50%, not as the session
        // average.
        m.cache_read_last = 24_000;
        assert_eq!(m.cache_hit_rate(), Some(0.5));
    }

    #[test]
    fn the_cache_numerator_never_borrows_from_earlier_calls() {
        // The shape that produced G16: cumulative reads above one prompt. The session
        // total is deliberately larger than the prompt, and the rate must ignore it.
        let m = Metrics {
            context_used: Some(60_000),
            prompt_tokens_last: Some(10_000),
            context_window: Some(200_000),
            cache_read_session: 90_000,
            cache_read_last: 1_000,
            // A rate is asserted below, so the provider is one that reports cache
            // traffic. Without this the rate is "not measurable" (K1) and the
            // assertion would be testing a different thing than it names.
            cache_reported: true,
            ..Metrics::default()
        };
        assert_eq!(m.cache_hit_rate(), Some(0.1));
        // The session total did not leak into the window reading either.
        assert_eq!(m.context_gauge(), "ctx 30%/200k");
    }

    #[test]
    fn a_tiny_prompt_does_not_claim_a_cache_hit() {
        let m = Metrics {
            context_used: Some(120),
            prompt_tokens_last: Some(120),
            context_window: Some(200_000),
            cache_read_last: 120,
            // Reports cache traffic; the prompt is simply too small for a rate to
            // mean anything. `CH —` is "not yet", which is a different sentence from
            // the `CH n/a` a silent provider gets — and this test is the one that
            // keeps the two apart.
            cache_reported: true,
            ..Metrics::default()
        };
        assert!(m.cache_hit_rate().is_none(), "120 tokens proves nothing");
        assert_eq!(m.cache_gauge(), "R0 W0 CH —");
    }

    #[test]
    fn cache_reads_cannot_exceed_the_prompt_they_belong_to() {
        // A provider that reports more cache reads than prompt tokens must not
        // produce a rate above 100%.
        let m = Metrics {
            context_used: Some(10_000),
            prompt_tokens_last: Some(10_000),
            cache_read_last: 99_000,
            cache_reported: true,
            ..Metrics::default()
        };
        assert_eq!(m.cache_hit_rate(), Some(1.0));
    }

    #[test]
    fn a_window_with_no_measurement_behind_it_prints_the_unknown_mark() {
        // The dead branch this replaces: `None if self.compacted` and `None`
        // printed the same string, under a comment claiming the compaction flag
        // changed it. The window is still worth printing — it says which model
        // is in play — so the mark replaces the number, not the pair.
        let unknown = Metrics {
            context_window: Some(200_000),
            ..Metrics::default()
        };
        assert_eq!(unknown.context_gauge(), "?/200k");
        assert_eq!(unknown.pressure(), Pressure::Unknown);
        // Both unknown shapes print it: no window at all, and a window with no
        // measurement. That is what [`Metrics::CONTEXT_UNKNOWN`] names.
        let no_window = Metrics::default();
        assert_eq!(no_window.context_gauge(), Metrics::CONTEXT_UNKNOWN);
        assert!(
            Metrics::CONTEXT_UNKNOWN.contains('?'),
            "the constant must be the unknown mark, not a zero"
        );
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
    fn a_floor_cost_is_marked_rather_than_reported_as_a_total() {
        // The registry had no cache price for a session that moved cache tokens, so the
        // total is a lower bound: `$0.012+` says that, `$0.012` would not.
        let floor = Metrics {
            cost_micro_usd: Some(12_345),
            cost_is_floor: true,
            ..metrics()
        };
        assert_eq!(floor.cost_gauge().as_deref(), Some("$0.012+"));

        // A fully priced session carries no marker.
        let exact = Metrics {
            cost_micro_usd: Some(12_345),
            cost_is_floor: false,
            ..metrics()
        };
        assert_eq!(exact.cost_gauge().as_deref(), Some("$0.012"));

        // A zero cost is still a dollar figure — and a zero-cost floor is still a floor,
        // because the unpriced tokens may have cost something.
        let free = Metrics {
            cost_micro_usd: Some(0),
            cost_is_floor: true,
            ..metrics()
        };
        assert_eq!(free.cost_gauge().as_deref(), Some("$0+"));
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
