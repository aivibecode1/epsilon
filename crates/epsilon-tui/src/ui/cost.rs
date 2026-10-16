//! What a session cost, from disjoint token counts and the registry's prices.
//!
//! This is a function rather than four lines inside `App::metrics` for one reason: the
//! estimate is a claim about the user's money, and a claim about money needs a test. The
//! previous version lived in the draw path and was wrong in a way no test could see — it
//! priced two of the four components, quietly valuing every cached prompt token at zero
//! (G18).
//!
//! # The four components are disjoint
//!
//! Providers report prompt tokens in two buckets: `input` counts the tokens that were
//! **not** cached and `cache_read` counts the ones that were (`epsilon-llm`'s adapters
//! apply that split at the wire boundary — Anthropic's shape, `input = prompt - cached`).
//! Cache *writes* are a third bucket, and output tokens a fourth. So the total is a sum of
//! four products, not two, and adding a token to two buckets would be double-charging.
//!
//! # Units
//!
//! Prices are micro-USD per **million** tokens; counts are tokens. Summing the products
//! before the single division keeps the integer rounding to one truncation instead of
//! four, which matters at the small end: a session can be worth less than one cent, and
//! four separate truncations can round it to nothing.

/// Token counts for one session, in the four buckets providers report.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TokenCounts {
    /// Prompt tokens that were not served from cache.
    pub input: u64,
    /// Completion tokens.
    pub output: u64,
    /// Prompt tokens served from the cache.
    pub cache_read: u64,
    /// Prompt tokens written to the cache.
    pub cache_write: u64,
}

/// The active model's prices, micro-USD per million tokens.
///
/// A zero cache price means the registry publishes none, which is not the same as free —
/// see [`is_floor`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TokenPrices {
    /// Uncached prompt tokens.
    pub input: u64,
    /// Completion tokens.
    pub output: u64,
    /// Prompt tokens served from cache.
    pub cache_read: u64,
    /// Prompt tokens written to cache.
    pub cache_write: u64,
}

/// What these counts cost, component by component, in micro-USD.
///
/// The breakdown rather than a single total, for two reasons. It derives
/// [`CostBreakdown::is_floor`] from the same product that produced the total instead of a
/// second expression that has to agree with it, and it gives a future `/cost` view the
/// per-rate lines for free.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CostBreakdown {
    /// Each component's cost **before** the shared division: tokens × micro-USD per
    /// million tokens. Kept unscaled because the total must divide the four products once
    /// — dividing each separately lets four truncations add up to a session that reads as
    /// free (see [`CostBreakdown::total`]). Read a component through its accessor, never
    /// through the field.
    scaled: [u64; 4],
    /// See [`CostBreakdown::is_floor`].
    floor: bool,
    /// Tokens the session moved on models the registry does not price, summed
    /// across all four buckets. Dollars for these tokens are unknown — pricing
    /// them at zero would be the silently-smaller bill this module exists to
    /// prevent — so they are carried as volume for `/session` to name
    /// (`unpriced N tokens`) while the total counts only priced tokens.
    unpriced: u64,
}

/// Index of the uncached-input component in [`CostBreakdown::scaled`].
const INPUT: usize = 0;
/// Index of the output component.
const OUTPUT: usize = 1;
/// Index of the cache-read component.
const CACHE_READ: usize = 2;
/// Index of the cache-write component.
const CACHE_WRITE: usize = 3;

impl CostBreakdown {
    /// The whole session, in micro-USD.
    ///
    /// The four unscaled products are summed and divided **once**. The old expression
    /// divided each component, and a session worth less than a micro-USD per component
    /// could then total to nothing at all.
    pub fn total(&self) -> u64 {
        self.scaled.iter().copied().sum::<u64>() / 1_000_000
    }

    /// The uncached-input component, in micro-USD.
    pub fn input(&self) -> u64 {
        self.scaled[INPUT] / 1_000_000
    }

    /// The output component, in micro-USD.
    pub fn output(&self) -> u64 {
        self.scaled[OUTPUT] / 1_000_000
    }

    /// The cache-read component, in micro-USD.
    pub fn cache_read(&self) -> u64 {
        self.scaled[CACHE_READ] / 1_000_000
    }

    /// The cache-write component, in micro-USD.
    pub fn cache_write(&self) -> u64 {
        self.scaled[CACHE_WRITE] / 1_000_000
    }

    /// Is the total a lower bound rather than the whole bill?
    ///
    /// True when the registry prices this model **and** a bucket that moved tokens has no
    /// published rate. The `prices this model` half is what keeps the marker meaningful:
    /// every rate `0` means the model is free or local (a `:free` row, an ollama model,
    /// the offline provider), where zero is the price rather than a gap, and marking those
    /// sessions `$0+` would be noise. What cannot be known is the rate of a bucket the
    /// registry does not describe at all.
    pub fn is_floor(&self) -> bool {
        self.floor
    }

    /// Tokens moved on models the registry does not price, across all four
    /// buckets — the volume behind the floor marker when the floor comes from
    /// an unknown model rather than an unpriced bucket.
    ///
    /// Zero means there is nothing unattributed: `/session` renders no line
    /// for it, because a line reading "unpriced 0 tokens" would be noise the
    /// reader has to decode. Nonzero is rendered as volume only, never priced:
    /// inventing a rate for these tokens would turn a lower bound back into a
    /// precise-looking total.
    pub fn unpriced_tokens(&self) -> u64 {
        self.unpriced
    }
}

/// Cost these counts at these prices, component by component.
pub fn breakdown(counts: TokenCounts, prices: TokenPrices) -> CostBreakdown {
    let priced_somewhere =
        prices.input > 0 || prices.output > 0 || prices.cache_read > 0 || prices.cache_write > 0;
    let moved_unpriced = (counts.input > 0 && prices.input == 0)
        || (counts.output > 0 && prices.output == 0)
        || (counts.cache_read > 0 && prices.cache_read == 0)
        || (counts.cache_write > 0 && prices.cache_write == 0);
    CostBreakdown {
        scaled: [
            counts.input * prices.input,
            counts.output * prices.output,
            counts.cache_read * prices.cache_read,
            counts.cache_write * prices.cache_write,
        ],
        floor: priced_somewhere && moved_unpriced,
        // The single-model shape has no unknown-model row: its caller names
        // one model and its prices together. Bucket-level gaps stay floor-only
        // (see `is_floor`); only `breakdown_per_model` can meet a model the
        // registry never heard of.
        unpriced: 0,
    }
}

/// What these counts cost in total, in micro-USD.
pub fn micro_usd(counts: TokenCounts, prices: TokenPrices) -> u64 {
    breakdown(counts, prices).total()
}

/// Is the total a lower bound rather than the whole bill?
///
/// True when the session moved tokens in a bucket the registry does not price. The header
/// marks such a number with a trailing `+`, because a precise-looking total that silently
/// omits real tokens is exactly the failure this module exists to end.
pub fn is_floor(counts: TokenCounts, prices: TokenPrices) -> bool {
    breakdown(counts, prices).is_floor()
}

/// Price a whole session's usage where more than one model produced it.
///
/// M141. A token's price is a property of the model that consumed it, so a session that
/// switched models has to be priced in pieces: sum the per-model counts, price each at
/// *its own* rates, and add the results. Summing first and pricing the sum once — at
/// whatever model is selected now — silently repriced every earlier turn the instant you
/// switched, with no visible sign, because the total simply became a different number.
///
/// `prices_for` looks up the rates for one model id and returns `None` when the registry
/// has no row for it. A model with no row contributes its tokens to the floor instead of
/// silently costing nothing, so an unpriced model is visible in the header's `+` marker
/// rather than quietly making the bill smaller.
///
/// Summing the four per-model `scaled` components and dividing once at the end is
/// deliberate: `breakdown` already works in a scale wide enough that per-model rounding
/// cannot lose a micro-dollar, and a single final division is the only way the total cannot
/// depend on the order the models happen to be visited.
pub fn breakdown_per_model(
    usage_by_model: &std::collections::BTreeMap<String, TokenCounts>,
    prices_for: impl Fn(&str) -> Option<TokenPrices>,
) -> CostBreakdown {
    let mut scaled = [0u64; 4];
    let mut floor = false;
    let mut unpriced = 0u64;
    for (model_id, counts) in usage_by_model {
        match prices_for(model_id) {
            Some(prices) => {
                let part = breakdown(*counts, prices);
                for (acc, add) in scaled.iter_mut().zip(part.scaled) {
                    *acc += add;
                }
                floor |= part.floor;
            }
            None => {
                // The registry does not price this model. Its tokens are real money of an
                // unknown amount, so the whole session becomes a lower bound — and the
                // tokens are carried as visible volume rather than dropped, so the
                // `+` marker never has to stand alone without saying by how much.
                // This is the same `None` branch that sets the floor: the two
                // cannot disagree about which rows are unknown.
                floor = true;
                unpriced += counts.input + counts.output + counts.cache_read + counts.cache_write;
            }
        }
    }
    CostBreakdown {
        scaled,
        floor,
        unpriced,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// $3/Mtok in, $15/Mtok out, cache read $0.30/Mtok, cache write $3.75/Mtok — the
    /// shape of a real Anthropic price list, where a cache read is a tenth of input.
    fn prices() -> TokenPrices {
        TokenPrices {
            input: 3_000_000,
            output: 15_000_000,
            cache_read: 300_000,
            cache_write: 3_750_000,
        }
    }

    // ---- M141: a session that switched models ----

    /// Ten times the input price, nothing else. Chosen to make a repricing error
    /// unmistakable in the total rather than a rounding-level nuisance.
    fn pricey() -> TokenPrices {
        TokenPrices {
            input: 30_000_000,
            output: 150_000_000,
            cache_read: 3_000_000,
            cache_write: 37_500_000,
        }
    }

    fn counts(input: u64, output: u64) -> TokenCounts {
        TokenCounts {
            input,
            output,
            cache_read: 0,
            cache_write: 0,
        }
    }

    fn by_model(pairs: &[(&str, TokenCounts)]) -> std::collections::BTreeMap<String, TokenCounts> {
        pairs.iter().map(|(k, v)| ((*k).to_string(), *v)).collect()
    }

    /// One model: identical to pricing the flat total. The fix must not perturb the
    /// overwhelmingly common single-model case, or it is not worth having.
    #[test]
    fn a_single_model_session_is_priced_exactly_as_before() {
        let usage = by_model(&[("cheap", counts(1_000_000, 100_000))]);
        let single = breakdown_per_model(&usage, |id| (id == "cheap").then(prices));
        let flat = breakdown(counts(1_000_000, 100_000), prices());

        assert_eq!(single.total(), flat.total(), "one model, one answer");
        assert_eq!(single.is_floor(), flat.is_floor());
    }

    /// The defect. A turn on the cheap model followed by a switch to the expensive one
    /// must not charge the cheap turn at the expensive rate.
    ///
    /// Under the old behaviour both turns were summed and priced at whichever model was
    /// selected at render time, so this number was whatever `/model` was set to — and a
    /// session that switched to a pricier model appeared to have cost more than it did.
    #[test]
    fn a_switch_does_not_reprice_the_turns_that_came_before_it() {
        let usage = by_model(&[
            ("cheap", counts(1_000_000, 100_000)),  // $3.00 + $1.50
            ("pricey", counts(1_000_000, 100_000)), // $30.00 + $15.00
        ]);
        let result = breakdown_per_model(&usage, |id| match id {
            "cheap" => Some(prices()),
            "pricey" => Some(pricey()),
            _ => None,
        });

        // 3.00 + 1.50 + 30.00 + 15.00 = $49.50.
        assert_eq!(
            result.total(),
            49_500_000,
            "each turn at its own model's price, not both at the current one"
        );
    }

    /// The same usage priced wrongly, stated as the number that used to be printed.
    ///
    /// `breakdown` works in micro-USD per Mtok × tokens, so the old path charged
    /// 2,000,000 input at 30 (=$60.00) plus 200,000 output at 150 (=$30.00) = **$90.00**,
    /// for work that actually cost $49.50. The wrong number is not a rounding artefact —
    /// it is 82% too high, and the direction it errs depends on which way you switched,
    /// so a session that moved to a *cheaper* model under-reports its own spending.
    #[test]
    fn the_old_pricing_would_have_overstated_this_session() {
        let usage = by_model(&[
            ("cheap", counts(1_000_000, 100_000)),
            ("pricey", counts(1_000_000, 100_000)),
        ]);
        let correct = breakdown_per_model(&usage, |id| match id {
            "cheap" => Some(prices()),
            "pricey" => Some(pricey()),
            _ => None,
        });
        // What the old code did: sum the counts, price once at the *selected* model.
        let repriced = breakdown(
            counts(2_000_000, 200_000),
            pricey(), // the model selected at render time
        );

        assert_eq!(correct.total(), 49_500_000, "3.00 + 1.50 + 30.00 + 15.00");
        assert_eq!(
            repriced.total(),
            90_000_000,
            "60.00 + 30.00, every token at the new rate"
        );
        assert!(
            repriced.total() > correct.total(),
            "switching to a pricier model inflated the bill for work already done"
        );
    }

    /// A model the registry has no row for costs an unknown amount, so the total is a
    /// floor. It must NOT be treated as free: a silently smaller bill is the failure this
    /// module exists to prevent.
    #[test]
    fn an_unpriced_model_makes_the_total_a_floor_not_a_smaller_bill() {
        let usage = by_model(&[
            ("cheap", counts(1_000_000, 100_000)),
            ("mystery", counts(5_000_000, 500_000)),
        ]);
        let result = breakdown_per_model(&usage, |id| (id == "cheap").then(prices));

        assert_eq!(
            result.total(),
            4_500_000,
            "only the priced model's tokens contribute to a known total"
        );
        assert!(
            result.is_floor(),
            "an unpriced model must show as `$…+`, never as a confidently smaller total"
        );
    }

    /// Unknown spend is visible as volume, not as dollars. A model the registry
    /// cannot price (an unpriced catalog row, or the `(model not reported)`
    /// row a subagent's unattributed tool usage lands in) contributes its
    /// tokens to `unpriced_tokens` while the dollars stay floor-honest: the
    /// total still counts only priced tokens, and no rate is invented for the
    /// rest. `/session` renders this count beside the cost; a `+` alone says
    /// the total is short without saying by how much.
    #[test]
    fn an_unpriced_model_contributes_its_volume_never_its_dollars() {
        let usage = by_model(&[
            ("cheap", counts(1_000_000, 100_000)),
            ("mystery", counts(5_000_000, 500_000)),
        ]);
        let result = breakdown_per_model(&usage, |id| (id == "cheap").then(prices));

        assert_eq!(
            result.total(),
            4_500_000,
            "only the priced model's tokens contribute to a known total"
        );
        assert!(result.is_floor());
        assert_eq!(
            result.unpriced_tokens(),
            5_500_000,
            "5M in + 500k out on the mystery model are real tokens a reader can see"
        );
    }

    /// Cache traffic on an unpriced model counts too: volume is volume, and a
    /// subagent that burned cached prefix must not vanish from the line.
    #[test]
    fn unpriced_volume_sums_all_four_buckets() {
        let mystery = TokenCounts {
            input: 1_000,
            output: 100,
            cache_read: 5_000,
            cache_write: 200,
        };
        let usage = by_model(&[("mystery", mystery)]);
        let result = breakdown_per_model(&usage, |_| None);

        assert_eq!(
            result.total(),
            0,
            "nothing priced, nothing counted in dollars"
        );
        assert!(result.is_floor());
        assert_eq!(
            result.unpriced_tokens(),
            1_000 + 100 + 5_000 + 200,
            "every bucket on an unpriced row is visible volume"
        );
    }

    /// A fully priced session has no unpriced volume, so the `/session` line
    /// stays exactly as today: the extra fragment is absent, not zero.
    #[test]
    fn a_fully_priced_session_has_no_unpriced_volume() {
        let usage = by_model(&[("cheap", counts(1_000_000, 100_000))]);
        let result = breakdown_per_model(&usage, |id| (id == "cheap").then(prices));

        assert!(!result.is_floor());
        assert_eq!(
            result.unpriced_tokens(),
            0,
            "no unknown model means no unpriced line"
        );
    }

    /// An empty session has no unpriced volume either.
    #[test]
    fn an_empty_session_has_no_unpriced_volume() {
        let result = breakdown_per_model(&std::collections::BTreeMap::new(), |_| None);
        assert_eq!(result.unpriced_tokens(), 0);
        assert!(
            !result.is_floor(),
            "no tokens means no unpriced tokens; `+` would be noise"
        );
    }

    /// Order independence: `BTreeMap` fixes the iteration order, but the total must not
    /// depend on it either way, or a future `HashMap` would silently change the number.
    #[test]
    fn the_total_does_not_depend_on_which_model_is_visited_first() {
        let usage = by_model(&[
            ("aaa-cheap", counts(1_000_000, 100_000)),
            ("zzz-pricey", counts(1_000_000, 100_000)),
        ]);
        let result = breakdown_per_model(&usage, |id| {
            if id.starts_with("aaa") {
                Some(prices())
            } else {
                Some(pricey())
            }
        });
        assert_eq!(result.total(), 49_500_000);
    }

    /// An empty session is free and is not a floor — there is nothing missing from it.
    #[test]
    fn a_session_with_no_usage_yet_is_zero_and_not_a_floor() {
        let result = breakdown_per_model(&std::collections::BTreeMap::new(), |_| Some(prices()));
        assert_eq!(result.total(), 0);
        assert!(
            !result.is_floor(),
            "no tokens means no unpriced tokens; `+` would be noise"
        );
    }

    #[test]
    fn every_bucket_is_charged_at_its_own_price() {
        let counts = TokenCounts {
            input: 1_000_000,       // $3.00
            output: 100_000,        // $1.50
            cache_read: 10_000_000, // $3.00
            cache_write: 1_000_000, // $3.75
        };
        // 3.00 + 1.50 + 3.00 + 3.75 = $11.25, and each component also reads back on its
        // own, which is what a `/cost` view would show.
        let cost = breakdown(counts, prices());
        assert_eq!(cost.total(), 11_250_000);
        assert_eq!(cost.input(), 3_000_000);
        assert_eq!(cost.output(), 1_500_000);
        assert_eq!(cost.cache_read(), 3_000_000);
        assert_eq!(cost.cache_write(), 3_750_000);
    }

    #[test]
    fn cached_tokens_are_not_priced_at_the_input_rate() {
        // The mistake this module exists to prevent: ten million cached prompt tokens cost
        // a tenth of what ten million uncached ones do. Pricing them as input is the
        // over-report; pricing them at zero (the old behaviour) is the under-report.
        let cached = TokenCounts {
            cache_read: 10_000_000,
            ..TokenCounts::default()
        };
        let uncached = TokenCounts {
            input: 10_000_000,
            ..TokenCounts::default()
        };
        assert_eq!(micro_usd(cached, prices()), 3_000_000);
        assert_eq!(micro_usd(uncached, prices()), 30_000_000);
    }

    #[test]
    fn the_sum_is_divided_once_so_small_components_do_not_vanish() {
        // One token in each of two buckets at $0.50/Mtok: half a micro-USD apiece. Divided
        // per component — the shape of the old expression — both halves truncate to zero
        // and the session reads as free. Divided once, they make the single micro-USD it
        // actually cost.
        let counts = TokenCounts {
            input: 1,
            cache_read: 1,
            ..TokenCounts::default()
        };
        let prices = TokenPrices {
            input: 500_000,
            cache_read: 500_000,
            ..TokenPrices::default()
        };
        assert_eq!(micro_usd(counts, prices), 1);
    }

    #[test]
    fn an_unpriced_cache_makes_the_total_a_floor() {
        let counts = TokenCounts {
            input: 1_000,
            cache_read: 1_000_000,
            ..TokenCounts::default()
        };
        let no_cache_price = TokenPrices {
            input: 3_000_000,
            ..TokenPrices::default()
        };
        assert!(
            is_floor(counts, no_cache_price),
            "a session that read a million cached tokens without a published cache price \
             cannot be reported as a total"
        );

        // No cache traffic: nothing is missing, so the same prices give a real total.
        let plain = TokenCounts {
            input: 1_000,
            ..TokenCounts::default()
        };
        assert!(!is_floor(plain, no_cache_price));

        // Fully priced: not a floor either.
        assert!(!is_floor(counts, prices()));
        assert!(!is_floor(TokenCounts::default(), no_cache_price));
    }

    #[test]
    fn a_model_that_costs_nothing_is_not_a_floor() {
        // Every rate zero means free or local — a `:free` row, an ollama model, the offline
        // provider. Zero is the price there, not a gap in the registry, and `$0+` for a
        // session that genuinely cost nothing would be noise the reader has to decode.
        let free = TokenPrices::default();
        let counts = TokenCounts {
            input: 5_000,
            output: 500,
            cache_read: 9_000,
            cache_write: 100,
        };
        assert_eq!(micro_usd(counts, free), 0);
        assert!(
            !is_floor(counts, free),
            "a free model is priced at zero, not unpriced"
        );
    }

    /// Tokens moved in a bucket the registry does not price make the total a
    /// floor, and the header marks it with a trailing `+` (P-052): a precise
    /// total that silently omits real tokens is the failure this marks against.
    #[test]
    fn unpriced_bucket_movement_marks_the_total_a_floor() {
        let priced = TokenPrices {
            input: 3_000_000,
            output: 15_000_000,
            cache_read: 0,
            cache_write: 0,
        };
        let moved_cache = TokenCounts {
            input: 1_000,
            output: 100,
            cache_read: 5_000,
            cache_write: 0,
        };
        assert!(
            is_floor(moved_cache, priced),
            "5k cache-read tokens at price 0 must floor the total"
        );
        assert!(
            !is_floor(counts(1_000, 100), prices()),
            "fully priced movement is exact, not a floor"
        );
    }
}
