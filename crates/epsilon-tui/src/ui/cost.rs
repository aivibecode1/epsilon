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

/// What these counts cost, in micro-USD.
pub fn micro_usd(counts: TokenCounts, prices: TokenPrices) -> u64 {
    (counts.input * prices.input
        + counts.output * prices.output
        + counts.cache_read * prices.cache_read
        + counts.cache_write * prices.cache_write)
        / 1_000_000
}

/// Is the total a lower bound rather than the whole bill?
///
/// True when the session moved cache tokens and the registry publishes no price for them.
/// The header marks such a number with a trailing `+`, because a precise-looking total
/// that silently omits real tokens is exactly the failure this module exists to end.
pub fn is_floor(counts: TokenCounts, prices: TokenPrices) -> bool {
    (counts.cache_read > 0 && prices.cache_read == 0)
        || (counts.cache_write > 0 && prices.cache_write == 0)
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
        // 3.00 + 1.50 + 3.00 + 3.75 = $11.25
        assert_eq!(micro_usd(counts, prices()), 11_250_000);
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
}
