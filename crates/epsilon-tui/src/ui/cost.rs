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
}
