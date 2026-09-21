# M111 — the cost estimate prices what it was leaving out

## What

The header's cost figure added two things up:

```rust
tokens_in * info.input_cost_micro_per_mtok / 1_000_000
    + tokens_out * info.output_cost_micro_per_mtok / 1_000_000
```

There are four things to price, not two, and the missing pair is the pair a long session
generates most of. Providers report prompt tokens in two disjoint buckets — `usage.input`
counts the tokens that were **not** cached, `cache_read` counts the ones that were (the
adapters apply that split at the wire boundary, Anthropic's `input = prompt - cached`
shape) — and cache *writes* are a third, usually a premium over a plain input token. So a
session that reads ten million cached tokens was billed for none of them: not cheap,
**free**, in the one number the user checks to see what they are spending.

This is the third of the three measurements the owner asked to be first-class (context,
cache rate, cost), and the last one to become honest. The cache rate was G16, the context
gauge was already right, and this was G18.

## Key decisions

- **The catalog has to carry the prices, so the generator was extended rather than the
  arithmetic patched.** `ModelInfo` gains `cache_read_cost_micro_per_mtok` and
  `cache_write_cost_micro_per_mtok`, `scripts/regen_catalog.py` reads models.dev's
  `cost.cache_read` / `cost.cache_write` (the registry has published them all along), and
  the bundled table was regenerated. Anything less — pricing cached tokens at the input
  rate, or at a guessed fraction of it — would have replaced an under-report with an
  over-report and called it accuracy.
- **`0` means "not published", and the header says so.** 359 of the 557 rows carry a
  cache-read price and 114 a cache-write price; the rest publish none. Charging zero for
  an unknown price is the original bug, so `Metrics::cost_is_floor` marks the case and the
  header prints `$0.42+` — "at least this" — instead of a precise-looking total that
  silently omits real tokens. An unknown is reported as an unknown, which is the rule the
  context gauge already follows with its `?`.
- **The arithmetic moved out of the draw path into [`ui/cost.rs`](../../crates/epsilon-tui/src/ui/cost.rs).**
  A claim about someone's money needs a test, and the old expression had none — which is
  how it stayed wrong through a UI redesign. The module takes counts and prices as plain
  numbers, so it tests without a terminal, an agent or a session.
- **Sum first, divide once.** The old shape divided each component by a million, which is
  four truncations; a session can be worth less than one cent, and four truncations can
  round it to zero. There is a test that would fail under the old shape.
- **The catalogue refresh is disclosed, not smuggled.** Regenerating also picks up a week
  of upstream drift, and the commit says exactly what that is: 547 rows → 557 (15 added,
  5 removed — `grok-4.7`, `glm-5.3-flashx`, several `:free` OpenRouter rows; `claude-opus-4`
  and four others gone), plus price changes. The pins in `catalog_is_current_generation`
  survived unchanged, which is the check that the snapshot is still coherent.

## Gotchas

- **The bug was invisible because the number looked plausible.** `$0.42` reads fine; it is
  merely missing most of the bill on a cached session, and nothing about it looks wrong.
  A cost estimate has no natural cross-check unless something else reports the same
  session's cost — which is why the floor marker matters more than the extra arithmetic:
  it makes "I do not know this part" visible.
- **`usage.input` excluding cached tokens is a wire-level fact, not an obvious one.** Both
  the over-report and the under-report come from misreading it: treating `input` as the
  whole prompt double-charges the cached part, and ignoring `cache_read` charges it
  nothing. The `TokenCounts` doc states the split so the next reader does not have to
  re-derive it from the adapters.
- **Python's `urllib` cannot reach models.dev on this machine** (MSYS Python has no CA
  roots: `CERTIFICATE_VERIFY_FAILED`), while PowerShell's `Invoke-WebRequest` can. The
  generator takes an `api.json` path for exactly this kind of case, so the refresh is
  `Invoke-WebRequest … -OutFile` followed by `python scripts/regen_catalog.py <path>`.

## Verification

Battery `m111a` (`target/gate-m111a.log`, copied to
[`../../audit/evidence/gate-m111a.log`](../../audit/evidence/gate-m111a.log)): **all five
gates exit 0, 1,031 passed / 0 failed / 1 ignored across 59 test binaries** — five more than
M110's 1,026, which is the four `ui/cost.rs` tests plus the `ui/metrics.rs` marker test.
`epsilon-llm` 71 + 27 tests (including `catalog_is_current_generation`, which pins the
newest model of each family and did not move); `epsilon-tui` 226.

- `every_bucket_is_charged_at_its_own_price` — $3.00 + $1.50 + $3.00 + $3.75 = $11.25.
- `cached_tokens_are_not_priced_at_the_input_rate` — ten million cached prompt tokens cost
  a tenth of ten million uncached ones. This is the assertion the old code failed.
- `the_sum_is_divided_once_so_small_components_do_not_vanish` — two half-micro-USD
  components make one micro-USD; per-component division makes zero.
- `an_unpriced_cache_makes_the_total_a_floor` — and does *not* when there was no cache
  traffic or the price is published.

Plus `a_floor_cost_is_marked_rather_than_reported_as_a_total` in `ui/metrics.rs`.

Reinstall: **TUI behaviour changed** (the header's cost figure, including the `+` marker),
so the global `epsilon.exe` is rebuilt from this round's tree.

## Limits

- **No paid call was made, so no estimate was checked against a real invoice.** The tests
  verify the arithmetic and the pricing source; that a provider's reported token buckets
  line up with what the adapters put in them is the adapters' own contract, tested there.
- **Prices are a snapshot.** models.dev is a list price, not a bill: discounts, batch
  pricing, region surcharges and free tiers are invisible, and a provider's own cache
  policies (minimum cacheable prefix, TTL) are not modelled at all. The number is an
  estimate and the milestone does not claim otherwise.
- **A published price of zero is indistinguishable from an absent one.** The registry
  encodes "no separate price" as a missing field, which arrives here as `0`; a model that
  genuinely charged nothing for cache reads would also be `0`, and the header would mark
  it as a floor. Distinguishing them needs an `Option` in the schema and a generator that
  emits `None`, which is a larger change than this slice.
- **The catalog refresh is a week of drift.** It is disclosed above rather than reviewed
  row by row: 15 rows arrived, 5 left, and prices moved on some others. A reader who wants
  the diff to be *only* cache prices should read the two new fields per row and treat the
  rest as the refresh it is.
