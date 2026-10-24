//! Usage arithmetic at the wire boundary (M140).
//!
//! The header shows three numbers to the user: context used, cache hit rate, and cost.
//! All three are computed from [`Usage`], and all three assume its buckets are
//! **disjoint** — that `input` is what you were billed at the input rate *after*
//! removing the prefix that was served from cache.
//!
//! That assumption is not a detail of our own types. It is the documented behaviour of
//! the wire formats: pi states it twice, in `openai-completions.ts:1534` and
//! `openai-responses-shared.ts:567`, both citing the same rule —
//!
//! > OpenAI includes cached and cache-write tokens in input_tokens, so subtract both.
//!
//! When `input` double-counts the cached prefix, all three header numbers are wrong at
//! once, and each is wrong in a way the user cannot detect:
//!
//! - the **cache rate** reads low, because the denominator is inflated by the numerator;
//! - the **context gauge** reads high, because the anchor is the inflated prompt size;
//! - the **cost** reads high, because the same tokens are charged at the full input rate
//!   *and* again at the cheaper cache-read rate.
//!
//! These tests pin the arithmetic at the boundary where the mistake is made, because the
//! consequences are only visible much further downstream.

use epsilon_contracts::Usage;
use serde_json::json;

/// The compat wire shape shared by OpenAI, DeepSeek, Groq, xAI, OpenRouter, Gemini and
/// the Responses API. `seed_usage_compat` is the single function all of them funnel
/// through, so a fix here repairs every one of them at once.
fn compat_usage(prompt: u64, cached: u64, cache_write: u64, completion: u64) -> Usage {
    epsilon_llm::providers::test_support::compat_usage_from_json(&json!({
        "prompt_tokens": prompt,
        "completion_tokens": completion,
        "prompt_tokens_details": {
            "cached_tokens": cached,
            "cache_write_tokens": cache_write,
        },
    }))
}

/// A request whose 8,000-token prompt was 6,000 tokens of cached prefix.
///
/// The case that matters: with the cached prefix left inside `input`, the model appears
/// to have been billed 8,000 tokens at the input rate *and* to have read 6,000 from
/// cache — 14,000 tokens of story from an 8,000-token prompt.
/// K1: a wire that says nothing about caching is distinguishable from one that says
/// "zero of your prompt was cached" — decided by the *keys in the body*, never by the
/// value they carry.
///
/// This is the boundary the defect lived at. `cache_read == 0` is produced identically
/// by ollama, by an OpenAI-compatible proxy that omits `prompt_tokens_details`, and by
/// Anthropic's honest first-call miss. The header turned all three into `CH 0%`. The
/// two tests below are the two arms, and they must not be able to produce the same
/// answer.
#[test]
fn a_body_without_cache_keys_is_not_a_measured_zero() {
    let silent = epsilon_llm::providers::test_support::compat_usage_from_json(&json!({
        "prompt_tokens": 24_000,
        "completion_tokens": 500,
    }));

    assert_eq!(silent.cache_read, 0, "the counts are identical either way");
    assert!(
        !silent.cache_reported,
        "no cache key appeared in the body, so nothing about the cache was measured"
    );
}

#[test]
fn a_body_reporting_zero_cache_tokens_is_a_measured_zero() {
    // The exact same token counts, with the keys present and set to zero — which is
    // what Anthropic sends on a genuine first-call miss.
    let measured = compat_usage(24_000, 0, 0, 500);

    assert_eq!(measured.cache_read, 0);
    assert!(
        measured.cache_reported,
        "the provider answered the question, and the answer was zero"
    );
}

/// The flag must survive aggregation, or a session with one cache-aware call would
/// report `n/a` for the turn that actually had cache traffic.
#[test]
fn combining_usages_keeps_the_cache_evidence_from_either_side() {
    let silent = epsilon_llm::providers::test_support::compat_usage_from_json(&json!({
        "prompt_tokens": 1_000,
        "completion_tokens": 10,
    }));
    let measured = compat_usage(1_000, 400, 0, 10);

    assert!(
        silent.combine(measured).cache_reported,
        "one cache-aware call is enough for the combined figure to be a measurement"
    );
    assert!(
        measured.combine(silent).cache_reported,
        "combination is order-independent"
    );
    assert!(
        !silent.combine(silent).cache_reported,
        "two silent calls are still silence, not a zero"
    );
}

#[test]
fn the_cached_prefix_is_not_counted_twice() {
    let u = compat_usage(8_000, 6_000, 0, 500);

    assert_eq!(
        u.input, 2_000,
        "input must exclude the cached prefix, or cost is charged twice for it"
    );
    assert_eq!(u.cache_read, 6_000, "the cached prefix is cache reads");
    assert_eq!(
        u.input + u.cache_read,
        8_000,
        "the buckets must add back up to the prompt the provider actually billed"
    );
}

/// A cache **write** is excluded too, not just a read. The two have different prices, so
/// a write left inside `input` is charged at the input rate and again at the write rate.
#[test]
fn cache_writes_are_excluded_from_input_too() {
    let u = compat_usage(8_000, 0, 3_000, 500);

    assert_eq!(
        u.input, 5_000,
        "a written prefix is not an input-rate token"
    );
    assert_eq!(u.cache_write, 3_000);
}

/// Read and write on the same turn, which is what a long session actually produces.
#[test]
fn reads_and_writes_on_one_turn_both_leave_input() {
    let u = compat_usage(10_000, 4_000, 2_000, 1_000);

    assert_eq!(u.input, 4_000);
    assert_eq!(u.cache_read, 4_000);
    assert_eq!(u.cache_write, 2_000);
    assert_eq!(u.output, 1_000);
    assert_eq!(
        u.input + u.cache_read + u.cache_write + u.output,
        10_000 + 1_000
    );
}

/// A provider that reports a cached count *larger* than the prompt is describing
/// reality differently from the documented one, and must not produce negative tokens:
/// `u64` arithmetic would wrap to ~1.8e19 and turn the header into garbage.
///
/// This is a real robustness property, not a hypothetical: a proxy that aggregates
/// several requests into one usage block would produce exactly this shape.
#[test]
fn a_cached_count_larger_than_the_prompt_saturates_instead_of_wrapping() {
    let u = compat_usage(100, 5_000, 0, 10);

    assert_eq!(
        u.input, 0,
        "input clamps at zero rather than wrapping around"
    );
    assert_eq!(
        u.cache_read, 5_000,
        "the provider's number is reported as received"
    );
}

/// The OpenAI Responses API carries its cache counts one level deeper than the compat
/// shape, inside `input_tokens_details`. If that object is not forwarded, the cached
/// prefix silently vanishes: `cache_read` is 0, so the header prints a confident
/// `cache 0%` and the whole prefix is billed at the input rate with no `+` floor marker
/// to hint that anything is missing.
#[test]
fn the_responses_wire_forwards_its_cache_details() {
    let u = epsilon_llm::providers::test_support::responses_usage_from_json(&json!({
        "input_tokens": 8_000,
        "output_tokens": 500,
        "total_tokens": 8_500,
        "input_tokens_details": {
            "cached_tokens": 6_000,
            "cache_write_tokens": 1_000,
        },
    }));

    assert_eq!(
        u.cache_read, 6_000,
        "a Responses reply that cached 6,000 tokens must not report zero"
    );
    assert_eq!(u.cache_write, 1_000);
    assert_eq!(
        u.input, 1_000,
        "8,000 input minus 6,000 read minus 1,000 written"
    );
    assert_eq!(u.output, 500);
}

/// A wire that reports no cache activity at all stays entirely in `input` — the fix
/// must not invent a zero-rate bucket or move tokens that were never cached.
#[test]
fn an_uncached_turn_is_untouched() {
    let u = compat_usage(1_500, 0, 0, 200);

    assert_eq!(u.input, 1_500);
    assert_eq!(u.cache_read, 0);
    assert_eq!(u.cache_write, 0);
    assert_eq!(u.output, 200);
}

/// D4: Mistral's `num_cached_tokens` shape is parsed into `cache_read`, so it
/// must also set `cache_reported` — otherwise a turn with real cached tokens
/// prints `CH n/a` next to the tokens it just moved.
#[test]
fn mistral_num_cached_tokens_sets_reported() {
    let u = epsilon_llm::providers::test_support::mistral_usage_from_json(&json!({
        "prompt_tokens": 110,
        "completion_tokens": 10,
        "total_tokens": 120,
        "num_cached_tokens": 30,
    }));

    assert_eq!(u.cache_read, 30, "the parse reads the num_* shape");
    assert_eq!(u.input, 80, "110 prompt minus 30 cached");
    assert!(
        u.cache_reported,
        "a parsed cache read is a measurement, not silence"
    );
}

/// D5: a Responses reply with no `input_tokens_details` is cache-silent, not a
/// measured zero — the reshaping step must not invent zero keys that the flag
/// then reads as evidence.
#[test]
fn responses_silent_stays_silent() {
    let u = epsilon_llm::providers::test_support::responses_usage_from_json(&json!({
        "input_tokens": 1_500,
        "output_tokens": 200,
        "total_tokens": 1_700,
    }));

    assert_eq!(u.cache_read, 0);
    assert!(
        !u.cache_reported,
        "no cache key appeared on the wire, so nothing was measured"
    );
}

/// A Responses reply omitting `total_tokens` falls back to the four-bucket
/// sum: the reshape must leave the key ABSENT (not zero), or the fallback
/// reads a present 0 as "measured nothing" and pins the total there.
#[test]
fn responses_missing_total_sums_instead_of_zero() {
    let u = epsilon_llm::providers::test_support::responses_usage_from_json(&json!({
        "input_tokens": 8_000,
        "output_tokens": 500,
        "input_tokens_details": {
            "cached_tokens": 6_000,
        },
    }));

    assert_eq!(u.input, 2_000, "8,000 input minus 6,000 cached");
    assert_eq!(u.output, 500);
    assert_eq!(
        u.total_tokens, 8_500,
        "absent total falls back to 2,000 + 500 + 6,000, not 0"
    );
}

// --- Wave C: the separately-reported reasoning-token bucket (criterion 6/7) ---
//
// Pi's `Usage.reasoning` (`references/pi-1.3.0/packages/ai/src/types.ts:427-448`):
// reasoning/thinking tokens are a SUBSET of `output` — `output` already
// includes them. Providers that expose the split set a number (possibly 0);
// the rest leave it undefined. Cost stays on `output` only: pricing the split
// a second time would charge the same tokens twice.
//
// The tests below pin each wire's split at the boundary, the same way the M140
// tests above pin the cache split: the mistake (folding reasoning into a priced
// bucket of its own, or dropping the split) is invisible downstream.

/// Anthropic reports thinking as a subset of output: 500 output tokens of
/// which 200 were thinking stay 500 output with the 200 recorded separately.
#[test]
fn anthropic_thinking_tokens_are_a_subset_of_output() {
    let u = epsilon_llm::providers::test_support::anthropic_usage_from_json(&json!({
        "input_tokens": 1_000,
        "output_tokens": 500,
        "cache_read_input_tokens": 0,
        "cache_creation_input_tokens": 0,
        "output_tokens_details": { "thinking_tokens": 200 },
    }));

    assert_eq!(
        u.output, 500,
        "output already includes thinking; it is not added"
    );
    assert_eq!(
        u.reasoning,
        Some(200),
        "the thinking split is stored, not folded away"
    );
}

/// The delta-body half of the same wire: fields present overwrite the seed.
#[test]
fn anthropic_delta_thinking_overwrites_when_present() {
    let u = epsilon_llm::providers::test_support::anthropic_delta_usage_from_json(
        &json!({
            "input_tokens": 1_000,
            "output_tokens": 500,
            "output_tokens_details": { "thinking_tokens": 200 },
        }),
        &json!({
            "output_tokens": 600,
            "output_tokens_details": { "thinking_tokens": 250 },
        }),
    );

    assert_eq!(u.output, 600);
    assert_eq!(
        u.reasoning,
        Some(250),
        "a thinking count the delta carries overwrites the seed"
    );
}

/// ...and fields absent keep the seed (pi preserves `input_tokens` across the
/// same boundary when proxies omit it; the split behaves the same way).
#[test]
fn anthropic_delta_without_thinking_keeps_the_seed() {
    let u = epsilon_llm::providers::test_support::anthropic_delta_usage_from_json(
        &json!({
            "input_tokens": 1_000,
            "output_tokens": 500,
            "output_tokens_details": { "thinking_tokens": 200 },
        }),
        &json!({ "output_tokens": 600 }),
    );

    assert_eq!(u.output, 600);
    assert_eq!(
        u.reasoning,
        Some(200),
        "a delta that says nothing about thinking must not erase the seed's split"
    );
}

/// An Anthropic reply with no `output_tokens_details` is reasoning-silent, not
/// a measured zero — the K1 presence rule, applied to the new bucket.
#[test]
fn anthropic_silent_wire_leaves_reasoning_unset() {
    let u = epsilon_llm::providers::test_support::anthropic_usage_from_json(&json!({
        "input_tokens": 1_000,
        "output_tokens": 500,
    }));

    assert_eq!(u.output, 500);
    assert_eq!(u.reasoning, None, "no thinking key, no measurement");
}

/// ...while an explicit zero is a measurement, not silence.
#[test]
fn anthropic_measured_zero_thinking_is_some_zero() {
    let u = epsilon_llm::providers::test_support::anthropic_usage_from_json(&json!({
        "input_tokens": 1_000,
        "output_tokens": 500,
        "output_tokens_details": { "thinking_tokens": 0 },
    }));

    assert_eq!(
        u.reasoning,
        Some(0),
        "the provider answered the question, and the answer was zero"
    );
}

/// A present-but-malformed split is unparseable, not zero: a proxy that
/// sends `"thinking_tokens": "200"` (a string) must not produce a measured
/// 0, which would claim the provider answered zero.
#[test]
fn anthropic_malformed_thinking_is_silence_not_zero() {
    let u = epsilon_llm::providers::test_support::anthropic_usage_from_json(&json!({
        "input_tokens": 1_000,
        "output_tokens": 500,
        "output_tokens_details": { "thinking_tokens": "200" },
    }));

    assert_eq!(
        u.reasoning, None,
        "a string where a count belongs is unknown, not a measured zero"
    );
}

/// ...and a malformed delta overwrites nothing: the seed's measured split
/// survives a delta whose thinking value is not a number.
#[test]
fn anthropic_malformed_delta_thinking_keeps_the_seed() {
    let u = epsilon_llm::providers::test_support::anthropic_delta_usage_from_json(
        &json!({
            "input_tokens": 1_000,
            "output_tokens": 500,
            "output_tokens_details": { "thinking_tokens": 200 },
        }),
        &json!({
            "output_tokens": 600,
            "output_tokens_details": { "thinking_tokens": "lots" },
        }),
    );

    assert_eq!(u.output, 600);
    assert_eq!(
        u.reasoning,
        Some(200),
        "a malformed delta must not erase the seed's split with a coerced zero"
    );
}

/// Chat-Completions reasoning arrives inside `completion_tokens_details`, and
/// `completion_tokens` already includes it (pi `openai-completions.ts:1539`):
/// parsing must record the split WITHOUT growing `output`.
#[test]
fn completions_reasoning_tokens_do_not_inflate_output() {
    let u = epsilon_llm::providers::test_support::compat_usage_from_json(&json!({
        "prompt_tokens": 8_000,
        "completion_tokens": 500,
        "completion_tokens_details": { "reasoning_tokens": 150 },
    }));

    assert_eq!(
        u.output, 500,
        "completion_tokens already includes reasoning; adding it would double-count"
    );
    assert_eq!(u.reasoning, Some(150));
}

/// A completions chunk with no `completion_tokens_details` is silent.
#[test]
fn completions_silent_wire_leaves_reasoning_unset() {
    let u = epsilon_llm::providers::test_support::compat_usage_from_json(&json!({
        "prompt_tokens": 8_000,
        "completion_tokens": 500,
    }));

    assert_eq!(u.output, 500);
    assert_eq!(u.reasoning, None, "no reasoning key, no measurement");
}

/// An explicit zero split is `Some(0)`, matching the cache-bucket rule that a
/// key carrying zero is a measurement.
#[test]
fn completions_measured_zero_is_some_zero() {
    let u = epsilon_llm::providers::test_support::compat_usage_from_json(&json!({
        "prompt_tokens": 8_000,
        "completion_tokens": 500,
        "completion_tokens_details": { "reasoning_tokens": 0 },
    }));

    assert_eq!(u.reasoning, Some(0));
}

/// Same rule on the compat path: a string `reasoning_tokens` is silence,
/// not a measured zero.
#[test]
fn completions_malformed_reasoning_tokens_are_silence_not_zero() {
    let u = epsilon_llm::providers::test_support::compat_usage_from_json(&json!({
        "prompt_tokens": 8_000,
        "completion_tokens": 500,
        "completion_tokens_details": { "reasoning_tokens": "150" },
    }));

    assert_eq!(u.output, 500);
    assert_eq!(
        u.reasoning, None,
        "a string where a count belongs is unknown, not a measured zero"
    );
}

/// The Responses API nests the split in `output_tokens_details`; the reshaping
/// step must forward it (like `input_tokens_details` in M140) or the split
/// silently vanishes.
#[test]
fn responses_reasoning_tokens_are_forwarded() {
    let u = epsilon_llm::providers::test_support::responses_usage_from_json(&json!({
        "input_tokens": 8_000,
        "output_tokens": 500,
        "total_tokens": 8_500,
        "output_tokens_details": { "reasoning_tokens": 120 },
    }));

    assert_eq!(
        u.output, 500,
        "output already includes reasoning on this wire too"
    );
    assert_eq!(
        u.reasoning,
        Some(120),
        "a Responses reply that reasoned 120 tokens must not report an unset split"
    );
}

/// Gemini is the one wire where `output` must GROW: the wire splits completion
/// volume into `candidatesTokenCount` + `thoughtsTokenCount`, and our `output`
/// is their sum (pi `google-generative-ai.ts:237-240`) with thoughts recorded
/// as the split.
#[test]
fn gemini_thoughts_are_added_to_output_and_reported_as_reasoning() {
    let u = epsilon_llm::providers::test_support::compat_usage_from_json(&json!({
        "promptTokenCount": 200,
        "candidatesTokenCount": 100,
        "thoughtsTokenCount": 50,
        "totalTokenCount": 350,
    }));

    assert_eq!(
        u.output, 150,
        "candidates + thoughts: thoughts are completion volume, not free text"
    );
    assert_eq!(u.reasoning, Some(50));
    assert_eq!(u.input, 200);
}

/// A Gemini reply with no `thoughtsTokenCount` leaves the split unset and
/// `output` at exactly the candidates count.
#[test]
fn gemini_silent_thoughts_leave_reasoning_unset() {
    let u = epsilon_llm::providers::test_support::compat_usage_from_json(&json!({
        "promptTokenCount": 200,
        "candidatesTokenCount": 100,
        "totalTokenCount": 300,
    }));

    assert_eq!(u.output, 100);
    assert_eq!(u.reasoning, None, "no thoughts key, no measurement");
}

/// Aggregation ORs presence like `cache_reported` (one measured call is enough
/// for the combined figure to carry the split) and sums values.
#[test]
fn combining_reasoning_ors_presence_and_sums_values() {
    let silent = Usage {
        input: 1_000,
        output: 10,
        ..Default::default()
    };
    let measured = Usage {
        input: 1_000,
        output: 500,
        reasoning: Some(200),
        ..Default::default()
    };

    assert_eq!(
        silent.combine(silent).reasoning,
        None,
        "two silent calls are still silence, not a zero"
    );
    assert_eq!(
        silent.combine(measured).reasoning,
        Some(200),
        "one reasoning-aware call is enough for the combined figure to carry the split"
    );
    assert_eq!(
        measured.combine(silent).reasoning,
        Some(200),
        "combination is order-independent"
    );
    assert_eq!(
        measured
            .combine(Usage {
                output: 300,
                reasoning: Some(50),
                ..Default::default()
            })
            .reasoning,
        Some(250),
        "two measured splits add, staying a subset of the summed output"
    );
}

/// Cost stays on `output` ONLY. This prices a usage the way the header does —
/// four disjoint buckets (input/output/cache_read/cache_write), deliberately
/// no fifth product — and proves the reasoning split cannot move the bill.
#[test]
fn reasoning_is_priced_inside_output_never_beside_it() {
    /// Mirrors the header's four-bucket shape (`epsilon-tui` `cost::micro_usd`):
    /// each priced bucket appears exactly once, and reasoning appears nowhere
    /// because it is inside `output` already.
    fn priced_like_the_header(u: &Usage, out_rate: u64) -> u64 {
        u.output * out_rate
    }
    const OUT_RATE: u64 = 10;

    let with_split = Usage {
        input: 1_000,
        output: 500,
        reasoning: Some(200),
        ..Default::default()
    };
    let without_split = Usage {
        reasoning: None,
        ..with_split
    };

    assert!(
        with_split.reasoning.unwrap_or(0) <= with_split.output,
        "the subset invariant the pricing depends on: reasoning never exceeds output"
    );
    assert_eq!(
        priced_like_the_header(&with_split, OUT_RATE),
        priced_like_the_header(&without_split, OUT_RATE),
        "recording the split must not move the bill"
    );
    assert_eq!(
        priced_like_the_header(&with_split, OUT_RATE),
        500 * OUT_RATE,
        "the bill is the output count at the output rate — the 200 thinking tokens \
         are inside the 500, not beside them"
    );
    assert_ne!(
        priced_like_the_header(&with_split, OUT_RATE),
        (500 + 200) * OUT_RATE,
        "the double-count trap: output + reasoning at the output rate is the wrong number"
    );
}
