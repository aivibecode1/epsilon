//! Cache-miss attribution (port plan item 4; pi `coding-agent/src/core/cache-stats.ts:56-163`).
//!
//! ADR-008 calls the byte-stable prompt the economic core of this harness: a stable prefix means
//! the provider's prompt cache keeps serving the system prompt and tool list at a fraction of the
//! input rate. The footer has shown a hit *rate* for a while, and a rate cannot tell you why it
//! dropped — an idle gap, a model switch, a prompt that stopped being byte-stable, or a provider
//! that simply does not report cache traffic.
//!
//! This module attributes each call that was *not* served from cache, prices the tokens the
//! provider billed at full rate as a result, and prints the breakdown. The point is to make the
//! discipline falsifiable: if the prefix is stable and the session is warm, the number is small,
//! and when it is not, the reason is named.
//!
//! Pricing note: only re-billed tokens are priced, at the model's **input** rate — that is exactly
//! what those tokens cost, so it needs no assumption. Cache reads and writes are reported as
//! tokens, not dollars: their rates are provider-specific multipliers this crate does not know.

use std::collections::BTreeMap;

use epsilon_contracts::message::Usage;
use epsilon_llm::catalog;

/// Default idle gap after which a provider cache is assumed cold. Five minutes is the
/// conservative end (the shortest common TTL); a longer TTL means we under-report misses, which
/// is the safer direction for a diagnostic.
pub const DEFAULT_CACHE_TTL_MS: u64 = 5 * 60 * 1_000;

/// Why a call was re-billed at full input rate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MissCause {
    /// A different model: a cache is namespaced per model, so the prefix does not carry over.
    ModelSwitch {
        /// Model used before.
        from: String,
        /// Model used now.
        to: String,
    },
    /// The prompt prefix stopped being byte-identical (the ADR-008 discipline broke), so the
    /// provider had to re-read it.
    PrefixChanged,
    /// Enough wall-clock time passed that the provider's cache entry expired.
    IdleGap {
        /// Milliseconds since the previous call.
        idle_ms: u64,
    },
    /// Same model, same prefix, no gap — the provider re-read it anyway (eviction, or the cache
    /// is not enabled for this account/key).
    Unexplained,
}

impl MissCause {
    /// A short label for the report.
    fn label(&self) -> String {
        match self {
            MissCause::ModelSwitch { from, to } => format!("model switch ({from} → {to})"),
            MissCause::PrefixChanged => "prompt prefix changed".to_string(),
            MissCause::IdleGap { idle_ms } => {
                format!("idle gap over {}m", idle_ms.div_ceil(60_000))
            }
            MissCause::Unexplained => "re-read by the provider".to_string(),
        }
    }
}

/// What happened to one call's prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CacheOutcome {
    /// The first call of this process: there was no warm prefix to hit.
    Cold,
    /// The provider served part or all of the prompt from its cache.
    Served {
        /// Tokens served from cache.
        read: u64,
    },
    /// The whole prompt was billed at input rate.
    ReBilled {
        /// Why.
        cause: MissCause,
        /// Prompt tokens billed at the input rate.
        tokens: u64,
    },
}

/// One provider call, as far as the cache is concerned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheRequest {
    /// Model id the call went to.
    pub model: String,
    /// Hash of the byte-stable prefix (system prompt + tools) sent with it.
    pub prefix_hash: u64,
    /// Usage as reported by the provider.
    pub usage: Usage,
    /// Wall-clock milliseconds when the call was made.
    pub at_ms: u64,
}

#[derive(Debug, Clone)]
struct Observed {
    model: String,
    prefix_hash: u64,
    at_ms: u64,
    usage: Usage,
    outcome: CacheOutcome,
}

/// Running attribution over the calls of one agent.
#[derive(Debug)]
pub struct CacheStats {
    ttl_ms: u64,
    calls: Vec<Observed>,
}

impl Default for CacheStats {
    fn default() -> Self {
        Self::new(DEFAULT_CACHE_TTL_MS)
    }
}

impl CacheStats {
    /// A tracker with an explicit idle-TTL.
    pub fn new(ttl_ms: u64) -> Self {
        Self {
            ttl_ms,
            calls: Vec::new(),
        }
    }

    /// Fold in one call and say what its prompt cost.
    pub fn observe(&mut self, req: CacheRequest) -> CacheOutcome {
        let outcome = self.classify(&req);
        self.calls.push(Observed {
            model: req.model,
            prefix_hash: req.prefix_hash,
            at_ms: req.at_ms,
            usage: req.usage,
            outcome: outcome.clone(),
        });
        outcome
    }

    fn classify(&self, req: &CacheRequest) -> CacheOutcome {
        if req.usage.cache_read > 0 {
            return CacheOutcome::Served {
                read: req.usage.cache_read,
            };
        }
        let Some(prev) = self.calls.last() else {
            return CacheOutcome::Cold;
        };
        // A provider that reports no cache traffic at all is not a miss — it is silence. Saying
        // "miss" there would invent a problem the harness cannot see or fix.
        let cause = if req.model != prev.model {
            MissCause::ModelSwitch {
                from: prev.model.clone(),
                to: req.model.clone(),
            }
        } else if req.prefix_hash != prev.prefix_hash {
            MissCause::PrefixChanged
        } else if req.at_ms.saturating_sub(prev.at_ms) > self.ttl_ms {
            MissCause::IdleGap {
                idle_ms: req.at_ms.saturating_sub(prev.at_ms),
            }
        } else {
            MissCause::Unexplained
        };
        CacheOutcome::ReBilled {
            cause,
            tokens: req.usage.input,
        }
    }

    /// How many calls were served (at least partly) from cache.
    pub fn served(&self) -> usize {
        self.calls
            .iter()
            .filter(|c| matches!(c.outcome, CacheOutcome::Served { .. }))
            .count()
    }

    /// Whether any call at all moved cache traffic.
    ///
    /// **This is not the same question as "does the provider report caching?"**,
    /// and the narrowness of the name is the point. A provider that speaks about
    /// caching and legitimately hit 0% on every call answers `false` here, and
    /// treating that as silence about caching is how a working cache gets
    /// reported as absent. `Usage::cache_reported` carries the authoritative
    /// answer; this carries the volume.
    ///
    /// It was previously named and used as if the two were the same question:
    /// `report()` printed "this provider reports no cache traffic" whenever this
    /// was false, which is a false statement about the provider on the most
    /// common path -- a healthy cache on a session too short to have hit.
    pub fn moved_cache_traffic(&self) -> bool {
        self.calls
            .iter()
            .any(|c| c.usage.cache_read > 0 || c.usage.cache_write > 0)
    }

    /// Whether the provider **reports** cache fields at all (K1).
    ///
    /// The authoritative answer, set by the decoder that saw the keys and OR-ed
    /// across calls. `None` while no call has been recorded, because a session
    /// with no calls has not been told anything yet.
    pub fn provider_reports_cache(&self) -> Option<bool> {
        let mut seen = false;
        let mut reported = false;
        for c in &self.calls {
            seen = true;
            reported |= c.usage.cache_reported;
        }
        seen.then_some(reported)
    }

    /// Tokens the provider billed at full input rate on calls that were not served from cache.
    pub fn rebilled_tokens(&self) -> u64 {
        self.calls
            .iter()
            .filter_map(|c| match &c.outcome {
                CacheOutcome::ReBilled { tokens, .. } => Some(*tokens),
                _ => None,
            })
            .sum()
    }

    /// Sum of cache reads over the session (tokens served cheap).
    pub fn cache_read_tokens(&self) -> u64 {
        self.calls.iter().map(|c| c.usage.cache_read).sum()
    }

    /// Sum of cache writes over the session (tokens paid for once, to be re-read).
    pub fn cache_write_tokens(&self) -> u64 {
        self.calls.iter().map(|c| c.usage.cache_write).sum()
    }

    /// Re-billed tokens grouped by cause, most expensive first.
    pub fn by_cause(&self) -> Vec<(String, u64, usize)> {
        let mut grouped: BTreeMap<String, (u64, usize)> = BTreeMap::new();
        for call in &self.calls {
            if let CacheOutcome::ReBilled { cause, tokens } = &call.outcome {
                let entry = grouped.entry(cause.label()).or_insert((0, 0));
                entry.0 += tokens;
                entry.1 += 1;
            }
        }
        let mut out: Vec<(String, u64, usize)> = grouped
            .into_iter()
            .map(|(label, (tokens, count))| (label, tokens, count))
            .collect();
        out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        out
    }

    /// Cost of the re-billed tokens in micro-USD, at the model's list input rate.
    ///
    /// `None` when the model is not in the catalog (a custom provider, a local model, a faux) —
    /// the report then says how many tokens were re-billed without pretending to price them.
    pub fn rebilled_cost_micro(&self) -> Option<u64> {
        let model = self.calls.last()?.model.clone();
        let info = catalog::lookup_model(&model)?;
        Some(micro_cost(
            self.rebilled_tokens(),
            info.input_cost_micro_per_mtok,
        ))
    }

    /// The human-readable breakdown, one line per fact.
    pub fn report(&self) -> Vec<String> {
        if self.calls.is_empty() {
            return vec!["cache: no provider call yet".to_string()];
        }
        let mut lines = Vec::new();
        if !self.moved_cache_traffic() {
            // Say WHICH of the two facts is true. "no traffic" and "the provider
            // does not report caching" are different facts with different
            // consequences, and collapsing them is what made a healthy short
            // session look like a provider without caching.
            let why = match self.provider_reports_cache() {
                Some(false) => {
                    "the provider does not report cache fields, so nothing can be attributed"
                }
                // Traffic is zero but the provider answers: the cache is there and
                // has not been hit yet, which is a fact about the session, not a
                // defect in the harness or the provider.
                _ => "no call has hit the cache yet, so nothing can be attributed",
            };
            lines.push(format!("cache: {} call(s) · {why}", self.calls.len()));
            return lines;
        }
        let served = self.served();
        let share = served as f64 / self.calls.len() as f64 * 100.0;
        let mut head = format!(
            "cache: {} call(s) · {served} served from cache ({share:.0}%) · read {} tok · wrote {} tok",
            self.calls.len(),
            fmt_tokens(self.cache_read_tokens()),
            fmt_tokens(self.cache_write_tokens()),
        );
        let rebilled = self.rebilled_tokens();
        if rebilled > 0 {
            head.push_str(&format!(
                " · re-billed {} tok{}",
                fmt_tokens(rebilled),
                match self.rebilled_cost_micro() {
                    Some(micro) => format!(" (~{})", fmt_cost(micro)),
                    None => String::new(),
                }
            ));
        }
        lines.push(head);
        for (label, tokens, count) in self.by_cause() {
            lines.push(format!(
                "  · {count} × {label} — {} tok",
                fmt_tokens(tokens)
            ));
        }
        lines
    }
}

/// Micro-USD for `tokens` at `micro_per_mtok` (integer math; `u128` in the middle so a
/// pathological call cannot wrap the total).
fn micro_cost(tokens: u64, micro_per_mtok: u64) -> u64 {
    let product = (tokens as u128) * (micro_per_mtok as u128);
    (product / 1_000_000).min(u64::MAX as u128) as u64
}

/// `1234` → `1,234`; large counts in `k`/`M` so a footer line stays a line.
///
/// **Public since M196** because the compaction row in the transcript formats
/// token counts too, and two formatters for the same number is how a header
/// starts reading `12k` beside a transcript row reading `12000` -- a reader has
/// to guess whether those are the same figure.
pub fn fmt_tokens(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 10_000 {
        format!("{:.0}k", n as f64 / 1_000.0)
    } else {
        let s = n.to_string();
        let mut out = String::new();
        for (i, c) in s.chars().enumerate() {
            if i > 0 && (s.len() - i).is_multiple_of(3) {
                out.push(',');
            }
            out.push(c);
        }
        out
    }
}

/// Micro-USD as a compact dollar string (`$0.0120`).
fn fmt_cost(micro: u64) -> String {
    format!("${:.4}", micro as f64 / 1_000_000.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage(input: u64, cache_read: u64, cache_write: u64) -> Usage {
        Usage {
            input,
            output: 0,
            cache_read,
            cache_write,
            total_tokens: input + cache_read + cache_write,
            // This helper exists to exercise cache attribution, so every usage it
            // builds is one where the provider reported cache fields.
            cache_reported: true,
        }
    }

    fn req(model: &str, prefix: u64, input: u64, read: u64, at_ms: u64) -> CacheRequest {
        CacheRequest {
            model: model.into(),
            prefix_hash: prefix,
            usage: usage(input, read, if read > 0 { 0 } else { input }),
            at_ms,
        }
    }

    #[test]
    fn the_first_call_is_cold_and_the_warm_one_is_served() {
        let mut stats = CacheStats::new(DEFAULT_CACHE_TTL_MS);
        assert_eq!(
            stats.observe(req("m", 1, 1_000, 0, 0)),
            CacheOutcome::Cold,
            "nothing to compare against yet — not a miss"
        );
        assert_eq!(
            stats.observe(req("m", 1, 100, 900, 1_000)),
            CacheOutcome::Served { read: 900 }
        );
        assert_eq!(stats.served(), 1);
        assert_eq!(
            stats.rebilled_tokens(),
            0,
            "the warm call was not re-billed"
        );
    }

    #[test]
    fn a_model_switch_is_attributed_to_the_switch() {
        let mut stats = CacheStats::new(DEFAULT_CACHE_TTL_MS);
        stats.observe(req("deepseek-flash", 1, 1_000, 0, 0));
        stats.observe(req("deepseek-flash", 1, 100, 900, 1_000));
        let outcome = stats.observe(req("deepseek-v4", 1, 4_000, 0, 2_000));
        assert_eq!(
            outcome,
            CacheOutcome::ReBilled {
                cause: MissCause::ModelSwitch {
                    from: "deepseek-flash".into(),
                    to: "deepseek-v4".into()
                },
                tokens: 4_000
            }
        );
    }

    #[test]
    fn a_prefix_that_stopped_being_stable_is_attributed_to_the_prefix() {
        let mut stats = CacheStats::new(DEFAULT_CACHE_TTL_MS);
        stats.observe(req("m", 7, 1_000, 0, 0));
        stats.observe(req("m", 7, 100, 900, 1_000));
        // Same model, no idle gap, different bytes: the ADR-008 discipline broke.
        let outcome = stats.observe(req("m", 8, 2_000, 0, 1_500));
        assert_eq!(
            outcome,
            CacheOutcome::ReBilled {
                cause: MissCause::PrefixChanged,
                tokens: 2_000
            }
        );
    }

    #[test]
    fn an_idle_gap_is_attributed_by_elapsed_time() {
        let mut stats = CacheStats::new(60_000); // one-minute TTL for the test
        stats.observe(req("m", 1, 1_000, 0, 0));
        stats.observe(req("m", 1, 100, 900, 30_000));
        let outcome = stats.observe(req("m", 1, 1_500, 0, 120_000));
        match outcome {
            CacheOutcome::ReBilled {
                cause: MissCause::IdleGap { idle_ms },
                tokens,
            } => {
                assert_eq!(idle_ms, 90_000, "measured from the previous call");
                assert_eq!(tokens, 1_500);
            }
            other => panic!("expected an idle gap, got {other:?}"),
        }
    }

    #[test]
    fn same_model_same_prefix_no_gap_is_reported_as_the_providers_doing() {
        let mut stats = CacheStats::new(DEFAULT_CACHE_TTL_MS);
        stats.observe(req("m", 1, 1_000, 0, 0));
        stats.observe(req("m", 1, 100, 900, 1_000));
        let outcome = stats.observe(req("m", 1, 1_200, 0, 2_000));
        assert_eq!(
            outcome,
            CacheOutcome::ReBilled {
                cause: MissCause::Unexplained,
                tokens: 1_200
            },
            "the harness reports what it can see and does not invent a cause"
        );
    }

    /// A provider that DOES report caching, and has simply not hit it yet, must
    /// not be told it does not report caching.
    ///
    /// This is the case the old code got wrong, and the reason the function was
    /// renamed. `moved_cache_traffic()` is false here -- no call has moved a
    /// cached token -- and the old `report()` read that as "this provider
    /// reports no cache traffic", which is a different claim, about a different
    /// thing, and false. It is the most common path there is: a healthy cache on
    /// a session too short to have hit it.
    #[test]
    fn a_reporting_provider_that_has_not_hit_yet_is_not_called_silent() {
        let mut stats = CacheStats::new(DEFAULT_CACHE_TTL_MS);
        for i in 0..3 {
            stats.observe(CacheRequest {
                model: "m".into(),
                prefix_hash: 1,
                // `cache_reported: true`, zero reads and writes: the provider is
                // speaking about caching and the answer so far is "no hits".
                usage: usage(1_000, 0, 0),
                at_ms: i * 1_000,
            });
        }
        assert!(
            !stats.moved_cache_traffic(),
            "precondition: nothing has hit the cache"
        );
        assert_eq!(
            stats.provider_reports_cache(),
            Some(true),
            "the provider IS reporting cache fields; that is a different fact from              whether anything has hit"
        );
        let report = stats.report().join("\n");
        assert!(
            !report.contains("does not report cache fields"),
            "a reporting provider must not be described as silent:\n{report}"
        );
        assert!(
            report.contains("no call has hit the cache yet"),
            "the report should name the true reason:\n{report}"
        );
    }

    /// And the genuinely silent provider is still described as silent. Without
    /// this, the fix above could pass by never accusing anyone of anything.
    #[test]
    fn a_silent_provider_is_still_called_silent() {
        let mut stats = CacheStats::new(DEFAULT_CACHE_TTL_MS);
        for i in 0..3 {
            let mut u = usage(1_000, 0, 0);
            // What an unannotated decoder leaves behind.
            u.cache_reported = false;
            stats.observe(CacheRequest {
                model: "m".into(),
                prefix_hash: 1,
                usage: u,
                at_ms: i * 1_000,
            });
        }
        assert!(!stats.moved_cache_traffic());
        assert_eq!(stats.provider_reports_cache(), Some(false));
        let report = stats.report().join("\n");
        assert!(
            report.contains("does not report cache fields"),
            "a provider that says nothing about caching must be named as such:\n{report}"
        );
    }

    #[test]
    fn a_provider_that_says_nothing_about_caching_is_not_accused_of_misses() {
        let mut stats = CacheStats::new(DEFAULT_CACHE_TTL_MS);
        for i in 0..5 {
            // A wire that reports neither reads nor writes: `cache_write` has to be zero too,
            // which the `req` helper does not do (it models a cold call that *did* write).
            //
            // M196: the `usage` helper sets `cache_reported: true`, so this test
            // was NOT exercising a silent provider -- it was exercising a
            // reporting one with no hits, which is the case the next test now
            // owns. It passed for the wrong reason. `cache_reported` is set
            // explicitly so the name of the test is true of the thing it builds.
            let mut u = usage(1_000, 0, 0);
            u.cache_reported = false;
            stats.observe(CacheRequest {
                model: "m".into(),
                prefix_hash: 1,
                usage: u,
                at_ms: i * 1_000,
            });
        }
        assert!(!stats.moved_cache_traffic());
        let report = stats.report().join("\n");
        assert!(
            // M196: the wording changed from "reports no cache traffic" to "does
            // not report cache fields" because the old phrasing claimed the
            // provider had said nothing, which is a different claim from
            // "nothing has hit yet" -- and the test could not tell the two
            // apart, which is the defect. The intent is unchanged: a provider
            // that says nothing must be described as saying nothing.
            report.contains("does not report cache fields"),
            "silence is described as silence: {report}"
        );
        assert!(
            !report.contains("re-billed"),
            "and no waste is claimed: {report}"
        );
    }

    #[test]
    fn the_report_groups_the_waste_by_cause_most_expensive_first() {
        let mut stats = CacheStats::new(DEFAULT_CACHE_TTL_MS);
        stats.observe(req("m", 1, 1_000, 0, 0)); // cold
        stats.observe(req("m", 2, 500, 0, 1_000)); // prefix change, 500
        stats.observe(req("m", 2, 100, 900, 2_000)); // served
        stats.observe(req("m", 3, 9_000, 0, 3_000)); // prefix change again, 9_000
        let grouped = stats.by_cause();
        assert_eq!(grouped[0].0, "prompt prefix changed");
        assert_eq!(grouped[0].1, 9_500, "both prefix misses, one line");
        assert_eq!(grouped[0].2, 2, "and how many calls");
        let report = stats.report().join("\n");
        assert!(report.contains("1 served from cache"), "{report}");
        assert!(report.contains("re-billed 9,500 tok"), "{report}");
    }

    #[test]
    fn re_billed_tokens_are_priced_at_the_models_input_rate() {
        // Priced only when the model is in the catalog; the arithmetic is integer micro-USD.
        assert_eq!(
            micro_cost(1_000_000, 250_000),
            250_000,
            "1 Mtok at $0.25/Mtok"
        );
        assert_eq!(micro_cost(1_000, 250_000), 250, "1k tokens → $0.00025");
        assert_eq!(micro_cost(0, 250_000), 0);
        // No overflow on a pathological call.
        assert_eq!(micro_cost(u64::MAX, u64::MAX), u64::MAX);
    }

    #[test]
    fn an_unknown_model_is_not_priced_at_all() {
        let mut stats = CacheStats::new(DEFAULT_CACHE_TTL_MS);
        stats.observe(req("not-a-real-model", 1, 1_000, 0, 0));
        stats.observe(req("not-a-real-model", 2, 500, 0, 1_000));
        assert_eq!(
            stats.rebilled_cost_micro(),
            None,
            "a model the catalog does not know gets no price"
        );
        let report = stats.report().join("\n");
        assert!(report.contains("re-billed 500 tok"), "{report}");
        assert!(!report.contains('$'), "and no invented dollars: {report}");
    }

    #[test]
    fn an_empty_session_says_so() {
        let stats = CacheStats::default();
        assert_eq!(stats.report(), vec!["cache: no provider call yet"]);
    }

    #[test]
    fn token_and_cost_formatting_stay_readable() {
        assert_eq!(fmt_tokens(999), "999");
        assert_eq!(fmt_tokens(1_234), "1,234");
        assert_eq!(fmt_tokens(9_999), "9,999");
        assert_eq!(fmt_tokens(12_345), "12k");
        assert_eq!(fmt_tokens(2_500_000), "2.5M");
        assert_eq!(fmt_cost(12_000), "$0.0120");
    }
}
