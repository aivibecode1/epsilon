# M140 — the header's three numbers were being computed from double-counted tokens

**Status:** fixed, battery `m140b`, negative control below.

## The defect

The TUI header shows three numbers: context used, cache hit rate, and estimated cost. All
three are derived from `epsilon_contracts::Usage`, and all three read its four token
buckets as **disjoint** — `input` is what you were billed at the input rate *after*
removing the prefix that came from cache.

On every non-Anthropic, non-Mistral provider, `input` did not mean that.
`HttpEventStream::seed_usage_compat` set `input = prompt_tokens` with no subtraction, then
read the cached count out of the same payload. The wire format counts the cached prefix in
**both** places.

pi states the rule in two places, with citations, and the Epsilon port omitted it:

> OpenAI includes cached and cache-write tokens in input_tokens, so subtract both.
> — `openai-responses-shared.ts:567`, repeated at `openai-completions.ts:1534`

Mistral was the only adapter that got it right (`mod.rs:572`, `prompt.saturating_sub(cached)`),
which is itself the evidence that the arithmetic was understood somewhere and simply not
applied everywhere.

## Why it matters more than a number being off

Three consequences, and **all three are silent** — the header shows a plausible value in
each case:

1. **The cache hit rate reads low.** Its denominator is the prompt size, which the cached
   prefix inflated, while its numerator was correct. A turn that was 75% cached printed as
   ~43%.
2. **The context gauge reads high.** The compaction anchor is set from the measured prompt
   (`epsilon-core/src/lib.rs:2469`), so every `ctx N%` was computed against a prompt that
   included tokens the model never re-read.
3. **The cost reads high — twice.** `cost::breakdown` (`ui/cost.rs:133-136`) multiplies each
   bucket by its own rate independently, so the cached prefix was charged at the full input
   rate *and* again at the (much lower) cache-read rate.

A fourth, on the Responses wire only: `responses_usage` dropped `input_tokens_details`
entirely, so `cache_read` was always 0. That is worse than absent, because the header
printed a confident `cache 0%` instead of `—`, and the floor marker never fired, so there
was no hint that a number was missing.

## The fix

Two lines of behaviour, at the boundary where the mistake is made:

- `seed_usage_compat` now subtracts cache reads and writes from `input`, using
  `saturating_sub` per term. It also accepts the key shapes pi lists
  (`prompt_cache_hit_tokens` for DeepSeek, top-level `cached_tokens` for Kimi) and
  `prompt_tokens_details.cache_write_tokens` for providers that publish it.
- `responses_usage` forwards `input_tokens_details` instead of discarding it.

`saturating_sub` per term rather than one subtraction of the sum, because a proxy that
aggregates several requests into one usage block reports a cached count larger than the
prompt, and plain `u64` arithmetic wraps to about 1.8e19 there — a garbage percentage
rather than a failure.

No change was needed in `ui/cost.rs` or `ui/metrics.rs`. Both consume the buckets correctly
*given* the invariant; the invariant was what was missing, and fixing it at the source
repairs every downstream number at once.

## Tests

`crates/epsilon-llm/tests/usage_arithmetic.rs` — six tests, all through the real decoders:

1. `the_cached_prefix_is_not_counted_twice` — the 8,000/6,000 case; also asserts the buckets
   add back up to the billed prompt.
2. `cache_writes_are_excluded_from_input_too` — a write is not an input-rate token.
3. `reads_and_writes_on_one_turn_both_leave_input` — both on one turn.
4. `a_cached_count_larger_than_the_prompt_saturates_instead_of_wrapping` — robustness, not
   hypothetical.
5. `the_responses_wire_forwards_its_cache_details` — the second defect.
6. `an_uncached_turn_is_untouched` — the fix must not invent a bucket or move uncached
   tokens.

They needed a seam: the usage arithmetic happens mid-stream on a JSON object, long before an
`AssistantMessage` exists, so no end-to-end test could reach it. Two `test_support` entry
points (`compat_usage_from_json`, `responses_usage_from_json`) now expose it — the same
pattern `LineReassembler` established in M137 for the same reason.

## Negative control — captured, not composed

Two reverts inside the shipped bytes, then the same test binary re-run:

1. `seed_usage_compat`: `input` back to `prompt` (the subtraction removed).
2. `responses_usage`: `input_tokens_details` dropped again, as it was before the fix.

```
test an_uncached_turn_is_untouched ... ok
test the_cached_prefix_is_not_counted_twice ... FAILED
test reads_and_writes_on_one_turn_both_leave_input ... FAILED
test a_cached_count_larger_than_the_prompt_saturates_instead_of_wrapping ... FAILED
test the_responses_wire_forwards_its_cache_details ... FAILED
test cache_writes_are_excluded_from_input_too ... FAILED

thread 'the_cached_prefix_is_not_counted_twice' panicked at
  crates\epsilon-llm\tests\usage_arithmetic.rs:51:5:
assertion `left == right` failed: input must exclude the cached prefix, or cost is charged twice for it
  left: 8000
 right: 2000

thread 'cache_writes_are_excluded_from_input_too' panicked at
  crates\epsilon-llm\tests\usage_arithmetic.rs:69:5:
assertion `left == right` failed: a written prefix is not an input-rate token
  left: 8000
 right: 5000

thread 'reads_and_writes_on_one_turn_both_leave_input' panicked at
  crates\epsilon-llm\tests\usage_arithmetic.rs:81:5:
assertion `left == right` failed
  left: 10000
 right: 4000

thread 'a_cached_count_larger_than_the_prompt_saturates_instead_of_wrapping' panicked at
  crates\epsilon-llm\tests\usage_arithmetic.rs:101:5:
assertion `left == right` failed: input clamps at zero rather than wrapping around
  left: 100
 right: 0

thread 'the_responses_wire_forwards_its_cache_details' panicked at
  crates\epsilon-llm\tests\usage_arithmetic.rs:128:5:
assertion `left == right` failed: a Responses reply that cached 6,000 tokens must not report zero
  left: 0
 right: 6000

test result: FAILED. 1 passed; 5 failed; 0 ignored; 0 measured; 0 filtered out
```

**5 failed, 1 passed.** The survivor is the point, not an oversight: `an_uncached_turn_is_untouched`
has nothing to subtract, so it passes under the control. A control where every test fails
would prove the test file was wired up, not that these tests detect *this* defect.

Two failures are worth reading closely because they name the user-visible harm:

- `left: 8000, right: 2000` — under the control the model appears to have been billed 8,000
  tokens at the input rate **and** to have read 6,000 from cache. Fourteen thousand tokens
  of story from an eight-thousand-token prompt, and the header printed a plausible number
  for each of the three gauges.
- `left: 0, right: 6000` — the Responses wire reported **no cache activity at all** for a
  reply that cached 6,000 tokens. The header showed a confident `cache 0%` rather than `—`,
  and the floor marker never fired, so nothing on screen hinted a number was missing.

With the fix restored, all six pass.
