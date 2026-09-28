# M69 — cache attribution: the discipline, made falsifiable

**Scope:** port-plan Tier 2 item 4 (the last Phase 2 item). ADR-008 calls the byte-stable prompt
"the economic core" of this harness, and the footer has shown a cache-hit percentage for a while
— but a rate cannot say *why* it dropped. An idle gap, a model switch, a prefix that stopped
being byte-stable and a provider that never reported cache traffic all look identical in a
percentage. This milestone names the cause and prices the damage.

## What

`crates/epsilon-core/src/cache_stats.rs` classifies every provider call:

| Outcome | Meaning |
|---|---|
| `Cold` | the first call of the process — there was no warm prefix to hit |
| `Served` | the provider answered from its cache (`cache_read > 0`) |
| `ReBilled(cause, tokens)` | the whole prompt was billed at input rate, and *why* |

The causes, in the order they are checked:

| Cause | Evidence the harness has |
|---|---|
| `ModelSwitch{from,to}` | the model id changed — a provider cache is namespaced per model |
| `PrefixChanged` | the STABLE tier's fingerprint moved (see below) — ADR-008's discipline broke |
| `IdleGap{idle_ms}` | more than the TTL (default 5 min) since the previous call |
| `Unexplained` | same model, same prefix, no gap: the provider re-read it (eviction, or no cache on this account) |

Three supporting pieces:

- **The STABLE-tier fingerprint** (`prompt::stable_fingerprint`) hashes the identity block and the
  tool lines — *only* those. The CONTEXT and VOLATILE tiers change for reasons that have nothing
  to do with cache health (today's date, the working directory, a reloaded `AGENTS.md`), and
  folding them in would report a cache break on every such turn. This is the quantity ADR-008
  promises to hold constant, now measurable from the loop.
- **Pricing** uses the catalog's list rate for the model's *input* tokens: re-billed tokens are
  billed at exactly that rate, so the number needs no assumption. Cache reads and writes are
  reported as tokens and never as dollars — their rates are provider-specific multipliers this
  crate does not know, and inventing them would be worse than omitting them. A model the catalog
  does not carry (local, custom, faux) gets no price at all, and the report says so by omission.
- **`/session` prints the report** next to the totals a user already looks at:

```
session: … · tokens 1234/567 · model claude-haiku-4-5
cache: 12 call(s) · 9 served from cache (75%) · read 45k tok · wrote 8k tok · re-billed 4,312 tok (~$0.0121)
  · 2 × idle gap over 5m — 2,110 tok
  · 1 × model switch (claude-haiku-4-5 → claude-opus-4-6) — 1,205 tok
  · 1 × prompt prefix changed — 997 tok
```

## Key decisions

- **Silence is not a miss.** A provider that reports neither reads nor writes gets "this provider
  reports no cache traffic, so nothing can be attributed" — not a hit rate of 0 % and not a pile
  of invented waste. Two tests pin this, one at the unit level and one through the loop.
- **Name the cause or say you cannot.** `Unexplained` is a first-class outcome: when the harness
  has ruled out the three things it can see, it says the provider re-read the prompt rather than
  blaming a mechanism it did not observe.
- **The fingerprint covers the stable tier on purpose.** A cache is broken by a change at the
  front of the prompt; the volatile tier is *supposed* to change. A test asserts both halves: the
  prompt bytes differ when the volatile tier differs, and the fingerprint does not.
- **Attribution lives on the agent, not the run.** A prompt cache outlives a run, and so does the
  evidence for or against it: a second run minutes later is exactly the case the idle-gap cause
  exists for.
- **The gate now fails on clippy warnings.** This milestone's first gate run reported PASS while
  clippy emitted a `manual_is_multiple_of` warning, because the script ran `cargo clippy` without
  `-D warnings` and clippy's own findings exit 0. AGENTS.md demands zero warnings; the gate now
  enforces it, and the comment in `scripts/gate.sh` records why.

## Test-caught bugs

1. **A test helper that lied about the wire.** The cache-stats test helper fabricated
   `cache_write = input` for every cold call, including calls in the "provider reports nothing"
   fixtures — so `reports_cache_traffic()` was true and the silence test failed. The helper models
   a cold *cache-writing* call; a wire that says nothing needs its own construction, and now has
   one, with the reason in a comment.
2. **`/session` crashed when no session store was attached.** The TUI command called
   `store_handle_for_tests()`, which `expect`s a store — a normal state for a plain `--tui` run or
   a child agent. Found by the new TUI test, which was the first to exercise the command at all.
   Fixed with a non-panicking `Agent::session_store()` and a branch that describes the state
   ("no durable session file attached") instead of unwinding.
3. **`deepseek-chat` is not a catalog row — again.** The "priced" scenario used it and got no
   price; the fixture now uses `claude-haiku-4-5`, and the test asserts on real catalog
   arithmetic. (M63 learned this when the credential resolver could not find the same id; the
   lesson is that a plausible-looking id is not a catalog id.)

## Verification

```
$ bash scripts/gate.sh m69b
===== fmt --check =====      clean
===== clippy =====           0 warnings        # now with -D warnings
===== test =====             617 tests, 0 failed, 1 ignored (the live smoke)
===== demo =====             completes
===== real-check =====       compiles
```

Seventeen new tests:

| Test | What it pins |
|---|---|
| `the_first_call_is_cold_and_the_warm_one_is_served` | cold ≠ miss; a served call wastes nothing |
| `a_model_switch_is_attributed_to_the_switch` | the switch, and both ids in the label |
| `a_prefix_that_stopped_being_stable_is_attributed_to_the_prefix` | the ADR-008 failure mode, named |
| `an_idle_gap_is_attributed_by_elapsed_time` | the gap, measured from the previous call |
| `same_model_same_prefix_no_gap_is_reported_as_the_providers_doing` | `Unexplained` exists and is used |
| `a_provider_that_says_nothing_about_caching_is_not_accused_of_misses` | silence is described as silence |
| `the_report_groups_the_waste_by_cause_most_expensive_first` | grouping, counts, ordering |
| `re_billed_tokens_are_priced_at_the_models_input_rate` | integer micro-USD arithmetic, no overflow |
| `an_unknown_model_is_not_priced_at_all` | no invented dollars |
| `an_empty_session_says_so` | the empty case |
| `token_and_cost_formatting_stay_readable` | `1,234` / `12k` / `2.5M` / `$0.0120` |
| `the_stable_fingerprint_moves_only_when_the_stable_tier_does` (`prompt::tests`) | stability under volatile changes, sensitivity to tool changes and order |
| `a_warm_session_reports_hits_and_no_waste` (loop) | attribution through the real loop |
| `a_cache_that_stops_being_used_is_attributed_and_priced` (loop) | the miss, its cause and its price |
| `a_model_switch_is_named_as_the_cause` (loop) | the loop hands over the live model id |
| `a_provider_that_never_mentions_caching_is_not_reported_as_wasteful` (loop) | the silence path end to end |
| `session_command_reports_cache_attribution_beside_the_totals` (TUI) | a user can see it, and the no-store case no longer panics |

Checked by mutation — `classify()` forced to always return `Unexplained`:

```
cache_stats::tests::the_first_call_is_cold_and_the_warm_one_is_served ... FAILED
cache_stats::tests::a_model_switch_is_attributed_to_the_switch ... FAILED
cache_stats::tests::a_prefix_that_stopped_being_stable_is_attributed_to_the_prefix ... FAILED
cache_stats::tests::an_idle_gap_is_attributed_by_elapsed_time ... FAILED
cache_stats::tests::the_report_groups_the_waste_by_cause_most_expensive_first ... FAILED
cache_stats::tests::an_unknown_model_is_not_priced_at_all ... FAILED
cache_attribution: a_warm_session_reports_hits_and_no_waste ... FAILED
cache_attribution: a_cache_that_stops_being_used_is_attributed_and_priced ... FAILED
cache_attribution: a_model_switch_is_named_as_the_cause ... FAILED
```

The silence tests and the arithmetic tests correctly passed under that mutation. Reverted before
the gate run above.

## Follow-ups

The port plan's ranked list is now down to provider-side message sanitization (item 18) and the
Tier 3 items (edit-matcher stages, LSP diagnostics, middleware, per-model guidance injection,
skill lint, workspace checkpoints, background process registry). Standing items: committing
M33–M69 (D-12) — the largest open risk, now ~190 files — live-provider evidence (D-13, blocked on
the expired stored key) and release artifacts (D-21). See
[`audit/07-plan.md`](../../audit/07-plan.md).
