# M140 — the header's three numbers were counted twice

**Status:** fixed. Battery `m140`, negative control at
[`../../audit/evidence/m140-negative-control.md`](../../audit/evidence/m140-negative-control.md).

The owner asked for accurate real-time context usage, cache hit rate, and estimated cost.
This milestone is the difference between *present* and *accurate*: all three numbers were
already on screen, and on every provider except Anthropic and Mistral they were wrong —
in three different directions, none of them visible to the user.

## What was wrong

`epsilon_contracts::Usage` has four buckets, and everything downstream reads them as
**disjoint**: `input` is what you were billed at the input rate, *after* removing the prefix
that came from cache.

The wire formats do not agree. `input_tokens` / `prompt_tokens` counts the cached prefix
**and** reports it separately. pi says so in two places, with citations:

> OpenAI includes cached and cache-write tokens in input_tokens, so subtract both.
> — `openai-responses-shared.ts:567`, repeated at `openai-completions.ts:1534`

`HttpEventStream::seed_usage_compat` set `input = prompt_tokens` with no subtraction and
then read the cached count out of the same payload. So the prefix was in two buckets at
once.

The tell that this was an oversight rather than a decision: **Mistral already had it
right** (`mod.rs:572`, `prompt.saturating_sub(cached)`). The arithmetic was understood
somewhere in the codebase and applied to exactly one of four adapters.

## Why three numbers, not one

Each consumer is correct *given* the invariant, and each fails differently:

| number | consumer | how it read |
|---|---|---|
| cache hit rate | `UiState::latest_prompt_tokens` is the denominator | **low** — denominator inflated by its own numerator |
| context gauge | compaction anchor set from the measured prompt | **high** — anchored to a prompt that includes tokens never re-read |
| cost | `cost::breakdown` multiplies each bucket by its own rate | **high, twice** — the prefix is charged at the input rate *and* the cache-read rate |

A turn that was 75% cached displayed as roughly 43%. This is the failure mode the repo's
own rule warns about: a wrong number is worse than a missing one, because a missing one is
noticed.

A fourth defect on the Responses wire only: `responses_usage` dropped `input_tokens_details`
entirely, so `cache_read` was always `0`. That is the worst of the four — the header printed
a confident `cache 0%` instead of the `—` it uses for unknown, and the floor marker never
fired, so there was no on-screen hint that a number was missing.

## The fix

At the boundary, because that is where the mistake was made:

- `seed_usage_compat` subtracts cache reads and writes from `input`, per term with
  `saturating_sub`. It also accepts the key shapes pi enumerates —
  `prompt_cache_hit_tokens` (DeepSeek), top-level `cached_tokens` (Kimi) — and
  `prompt_tokens_details.cache_write_tokens` for providers that publish it.
- `responses_usage` forwards `input_tokens_details`.

Nothing in `ui/cost.rs` or `ui/metrics.rs` changed. Both were right about what disjoint
buckets mean; the invariant was what was missing, and repairing it at the source fixes every
consumer at once.

`per term` rather than one subtraction of the sum, because a proxy that aggregates several
requests into one usage block reports a cached count larger than the prompt, and plain
`u64` subtraction wraps to about 1.8e19 — a garbage percentage rather than a failure. There
is a test for exactly that.

## Tests — six, through the real decoders

`crates/epsilon-llm/tests/usage_arithmetic.rs`. They needed a seam first: the usage
arithmetic happens mid-stream on a JSON object, long before any `AssistantMessage` exists,
so no end-to-end test could reach it — and `wire_fixtures.rs` has no usage fixtures, which
is why this survived. Two `test_support` entry points now expose the decoders, the same
pattern `LineReassembler` established in M137 for the same underlying reason.

**Battery `m140`: `GATE PASS`, 62 test binaries, 1,210 passed, 0 failed, 1 ignored** — up
from 1,204 at M139, and the delta is exactly these 6 tests in a new binary.

1. `the_cached_prefix_is_not_counted_twice` — the 8,000/6,000 case, plus that the buckets
   add back up to the billed prompt.
2. `cache_writes_are_excluded_from_input_too`
3. `reads_and_writes_on_one_turn_both_leave_input`
4. `a_cached_count_larger_than_the_prompt_saturates_instead_of_wrapping`
5. `the_responses_wire_forwards_its_cache_details`
6. `an_uncached_turn_is_untouched` — the fix must not invent a bucket.

**Negative control:** both fixes reverted, same binary re-run — **5 failed, 1 passed**. The
survivor is deliberate: an uncached turn has nothing to subtract, so it passes under the
control. A control where everything fails would prove the test file was wired up, not that
these tests catch this defect.

## Limits — what this slice does not do

- **Session cost is still re-priced at the current model.** `App::metrics` prices the whole
  accumulated total with `lookup_model(&self.model_id)`, so a `/model` switch reprices
  every earlier token, and `/resume` re-accumulates from history while ignoring each
  message's own `model_id`. That is a real defect and it is **not** fixed here — it is a
  different layer (accumulation, not decoding) and it deserves its own milestone rather
  than being folded in.
- **`total_tokens` is still dead.** Every adapter writes it, nothing in the TUI or core reads
  it, and `Metrics` recomputes from the four buckets — which is the right call now, but the
  field is a trap for the next reader. Bedrock's fallback for it also omits cache traffic
  while Anthropic's includes it.
- **No live-provider measurement.** The arithmetic is verified against wire fixtures through
  the real decoders, not against a real API response. A captured response from each provider
  family would be stronger evidence, and is the natural next fixture to add.
- **The cache rate is per-turn, not per-session**, and is suppressed below a 1,000-token
  prompt. That is a deliberate existing choice, unchanged here, but it means a short cached
  turn shows no rate at all.
- **Nothing asserts the header's rendered string for a cached turn.** The three fragments
  are tested in `ui/metrics.rs` against hand-built counts; that the corrected counts reach
  that renderer is composition, not execution.

## What nothing exercises

- **A real HTTP response on every provider family.** The key-shape tolerance in
  `seed_usage_compat` — four different spellings of "tokens I read from cache" — is
  exercised only for `prompt_tokens_details.cached_tokens`. The DeepSeek and Kimi keys that
  were just added are **untested**.
- **Bedrock and Anthropic are unchanged** and still correct on this property, but nothing
  now *pins* that. A test that would have caught the compat regression is one assertion per
  adapter; the compat one exists and two others do not.
- **`UiState`'s prompt/cache pair** (`state.rs:819-822`) is written in one place and
  assumed disjoint by the rate calculation. No test would fail if a future adapter wrote
  the two halves from different turns.
