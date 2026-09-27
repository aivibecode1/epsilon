# 05 — Context, cache, and cost

The header shows three numbers: how full the context window is, how well the prompt
cache is working, and what the session has cost. This document asks a single question
of each: **is it true?**

Short answer: all three are computed by code that was written carefully and is
substantially right. Two of them can be wrong in ways a user cannot detect, and one
of those is silent, unbounded, and about money.

Everything here was read from the source and, where a number is quoted, measured or
checked against the vendor's own documentation. Claims are labelled the way
[`README.md`](README.md) defines them.

## Where the numbers come from

All three are assembled in one function, `App::metrics()`
(`crates/epsilon-tui/src/app/view.rs:182-246`), and rendered by `Metrics::parts()`
(`crates/epsilon-tui/src/ui/metrics.rs:316-361`) into the header
(`crates/epsilon-tui/src/ui/header.rs:100-118`). The footer shows none of them — it
carries key hints, the path and the git branch.

That single-assembly point is a genuine strength. There is one place where the three
numbers are computed, so they cannot drift apart by being read at different moments.

---

## 1. Context: `ctx 41%/164k`

**How it is computed.** `context_used` comes from `App.ui.context_now`, mirrored once
per drawn frame from `Agent::estimated_context_tokens()`
(`app/view.rs:300-302`, called at `app/event.rs:169`). That function
(`crates/epsilon-core/src/lib.rs:1934-1937`) is:

```rust
crate::compact::anchored_tokens(&self.history(), anchor)
```

The anchor is the provider's own `input + cache_read + cache_write` for the last
completed call, plus `covered`, the message count that request covered
(`lib.rs:2501-2507`). `anchored_tokens` (`compact.rs:46-53`) returns
`anchor.prompt_tokens + estimate_messages(&history[covered..])`, falling back to a
`chars/4` heuristic when there is no anchor.

**What is right.** This is the size of the *next* request, not of the last one. That
distinction is the whole point, and it was got wrong once: the header used to divide
session-total cache reads by the latest prompt, which inflated every turn. The current
code is the fixed version, and `metrics.rs:40-59` documents the old bug. Unknown prints
`?/200k` rather than a confident `0%`; below 0.5% it prints `<1%` rather than rounding
to nothing. It agrees with the compaction trigger, which reads the same function minus a
16,384-token reserve (`lib.rs:1910-1920`). That consistency — the number you see and the
number the loop acts on are the same number — is worth more than most of what is wrong
below.

**Defect C1 — the denominator is a frozen snapshot, and it is contradicted by the
vendor.** *Measured, and checked against the vendor's own pricing page.*

`context_window` is `catalog::lookup_model(&self.model_id).context_window`
(`view.rs:236`). There is no live fetch of this value: `catalog::live_models`
(`crates/epsilon-llm/src/catalog.rs:664`) returns model *ids* only. The number comes
from `crates/epsilon-llm/src/catalog/catalog_data.rs`, a generated snapshot whose header
records its provenance as models.dev on **2026-09-21**.

For the DeepSeek rows the table says `163_840`, which `format_tokens` renders as `164k`.
DeepSeek's own documentation
([api-docs.deepseek.com/quick_start/pricing](https://api-docs.deepseek.com/quick_start/pricing))
states **1M** context for `deepseek-flash` and `deepseek-v4-pro`, and 128K for the
V3.2-era line. The table is therefore wrong for those models in one direction or the
other, and the header inherits it — and so does the compaction trigger, because both read
the same field. A denominator that is 6× off does not merely misreport; it decides when
the session compacts.

**Defect C2 — the no-anchor fallback omits the largest fixed costs.**
`anchored_tokens` walks `history`, and `history` does not contain the system prompt or
the tool schemas. Those are 6,535 bytes of agent contract plus ~7,459 bytes of tool
descriptions (`crates/epsilon-core/src/prompt.rs:18,121-127`) — roughly 3.5k tokens —
plus schemars-generated JSON schemas that are never counted at all.

This is not a small error in the states where it applies. The fallback is used on turn
one, after any compaction (`clear_usage_anchor`, `lib.rs:2221`), and after `/resume`
(`lib.rs:787`). The moment that matters most is immediately after a compaction: the
anchor is cleared, the fallback takes over, and `ctx` can read 6% when the next request
really carries 20%. Because the compaction trigger reads the same figure, it
under-triggers too.

**Defect C3 — the tail heuristic is biased, and says so.** `chars/4`
(`compact.rs:12-14`) over-counts prose and under-counts code and CJK by roughly the
same factor. This is acknowledged at `lib.rs:1926-1930` and is a reasonable trade for a
heuristic. It matters less than C1 and C2 and is listed for completeness.

---

## 2. Cache: `R18k W2.0k CH 75%`

**How it is computed.** `Metrics::cache_hit_rate` is
`cache_read_last.min(prompt_tokens_last) / prompt_tokens_last`
(`metrics.rs:170-180`), where `prompt_tokens_last` is `input + cache_read +
cache_write` for the **most recent** call (`state.rs:1015`) and `cache_read_last` comes
from that same call (`state.rs:1018`).

So the rate is *not* `cache_read / (cache_read + cache_write)`. It is the share of the
entire last prompt that was served from cache, with numerator and denominator taken from
one call rather than from mismatched totals. That pairing is the correct one, and the
bug that used to get it wrong is documented at `metrics.rs:80-84` with a test at
`metrics.rs:501-528`.

`CH —` means exactly one of two things: no call has completed yet, or the last prompt
was under 1,000 tokens (`metrics.rs:172`).

**What is right.** Provider coverage here is genuinely good, and this is the part of the
header most likely to be accurate on a real model. Anthropic's
`cache_read_input_tokens` / `cache_creation_input_tokens` (`providers/mod.rs:274-275`,
delta-merged at `:352-370`), Bedrock's `cacheReadInputTokens` / `cacheWriteInputTokens`
(`:485-486`), Mistral's eight key spellings (`:572-618`), OpenAI's
`prompt_tokens_details.cached_tokens` plus the Responses `input_tokens_details`
(`:311-320`, `openai_responses.rs:326-338`), DeepSeek's `prompt_cache_hit_tokens`, and
Gemini's `cachedContentTokenCount` are all mapped, with the cached prefix correctly
subtracted from `input` in `saturating_sub` (`:326-334`).

**Defect K1 — "cannot be measured" is rendered as "measured zero".** This is the one
that will mislead a user. For any wire that reports no cache fields at all — the faux
provider, which hardcodes `input: 100, output: 50` (`faux.rs:167-171`); ollama; an
OpenAI-compatible proxy that omits `prompt_tokens_details` — `cache_read` is
structurally 0. The header then prints `R0 W0 CH 0%` in the **Value** style
(`metrics.rs:333-339`), which looks exactly like a cache that is present and failing.

Nothing in the `Usage` type records whether a provider is *able* to report cache tokens.
The fix is a capability flag, not arithmetic: "this provider does not report cache
usage" is a different sentence from "your cache hit rate is zero", and only one of them
is a fact about the user's spending.

**Defect K2 — the rate is per-call, so a legitimate miss reads as a failure.** By
design the rate describes the last request. A turn that misses for a good reason — the
first call, the first call after a compaction, the first call after a `/model` switch —
prints a confident `0%` even when the session as a whole is 90% cached. The volume
counters (`R18k W2.0k`) ride alongside so the reader has a scale, which helps, but the
percentage is the thing that gets quoted.

---

## 3. Cost: `$0.0042`

**How it is computed.** `breakdown_per_model(self.ui.usage_by_model(), lookup_model)`
(`view.rs:204-216`, `crates/epsilon-tui/src/ui/cost.rs:173-196`): prices are looked up
**per model id**, four disjoint buckets are accumulated, and the sum is divided once
(`cost.rs:86-88`). Integer micro-USD throughout. A model or a bucket the registry does
not price sets `is_floor` (`cost.rs:118-140`), rendered as `$0.42+` (`metrics.rs:273-282`).

**What is right.** All four buckets are priced — that was defect G18 and it is fixed.
Per-model pricing survives a `/model` switch and a `/resume` (M141), so a multi-model
session comes back with the bill it actually incurred. A model the registry has never
heard of prints nothing rather than a plausible-looking `$0`. Retries are not
double-counted: a pre-stream failure's `Done` is discarded before surfacing
(`retry_stream.rs:114-119`), so only the surviving attempt is billed.

**Defect $1 — delegated subagent spend is never counted. *This is the most serious
defect in this document.*** *Verified end to end.*

The `task` tool returns the child's aggregated usage on `ToolOutput.usage`
(`crates/epsilon-tools/src/task.rs:231,281`). `exec.rs:736` carries it into the
`ToolResultMessage`. And then:

```
$ grep -n "usage" crates/epsilon-tui/src/state.rs
   ->  the Message::ToolResult arm (state.rs:1022-1061) never reads t.usage
   ->  the hydrate arm (state.rs:909-934) never reads it either
   ->  the only reader of per-message usage anywhere in epsilon-tui is
      app/session.rs:205, which is the /resume path
```

So the tokens are produced, carried, stored, and dropped on the floor. A session that
fans out into subagents under-reports its cost by an unbounded multiple, with **no `+`
marker** — because `is_floor` fires only for unpriced *models* and *buckets*, never for
missing *tokens*. A user reading `$0.31` has no way to know the true figure is `$2.40`.

This is a small, well-contained fix: read `t.usage` in the one place the other usage is
already read, and route it through the existing `record_usage`.

**Defect $2 — the compaction summary's own bill is missing until you resume.**
The summarizer is a real provider call, and its usage is recorded on the store entry
(`lib.rs:2187`), but `CompactionFinished` only sets a flag in the UI
(`state.rs:1159-1164`). The figure is folded into the live total only on `/resume`
(`app/session.rs:206` → `state.rs:826-839`). The visible symptom is that **the cost
number goes up when you reopen a session you were just working in**, which reads as a
bug in the user's head rather than in the code.

**Defect $3 — a failed turn can bill nothing.** A salvage that has already streamed
visible text calls `finish(Error, …)`, which reads a `usage_tail` that the
OpenAI-compatible wires never set, because their usage rides the *last* chunk
(`openai_compat.rs:233`). A stream that dies before that chunk contributes
`Usage::default()` — and so does the error path (`stream_fn.rs:147-161`). The prompt
tokens, usually the largest line item in a turn, disappear.

**Defect $4 — it is a list-price estimate, and list price is not always the price.**
The table is the 2026-09-21 snapshot. Two consequences worth stating plainly: DeepSeek
prices by time of day (off-peak is half of peak, per the same vendor page) and the table
carries one flat rate, so the estimate can be 2× off depending on the hour; and
Anthropic's 1-hour cache writes cost 2× base rather than 1.25×, which a single
`cache_write` rate cannot express at all.

**Defect $5 — it is not persisted as money.** The figure is recomputed from per-message
usage in the session file. A restart without `/resume` loses it, and a catalog refresh
silently reprices history that was billed at the old rates.

---

## Ranked by what actually hurts a user

| # | Defect | User impact | Fix size |
|---|---|---|---|
| 1 | **$1** subagent tokens never counted | Silent, unbounded, about money, no marker | Small — one reader |
| 2 | **$2** compaction cost missing until `/resume` | The number visibly changes when you reopen a session | Small |
| 3 | **$4** context window from a 2026-09-21 snapshot, contradicted for DeepSeek | Wrong denominator *and* wrong compaction trigger | Medium — needs a live source or a correction |
| 4 | **C2** fallback omits system prompt + tool schemas | Under-reports right after a compaction, when it matters | Medium |
| 5 | **$3** failed turns report zero usage | A whole turn's prompt cost erased | Medium |
| 6 | **K1** `CH 0%` for providers that cannot report cache | "Not measurable" shown as "measured zero" | Small — a capability flag |
| 7 | **K2** rate is per-call, not per-session | A good turn reads as a failure | Small — add a session rate beside it |
| 8 | **$4/$5** list price, not a bill; recomputed not stored | 2× swings; lost on restart | Large, and arguably out of scope |

## Honest limits

- No live provider call was made in this audit. Every provider-mapping claim is a
  reading of the wire code, not an observation of a real response.
- The DeepSeek context-window contradiction is checked against the vendor's published
  pricing page, not against an API response.
- The cost table's staleness is established from the snapshot's own header date. Whether
  models.dev has since corrected those rows is not checked.
- `epsilon-tui/src/ui/metrics.rs` is 27 KB and `cost.rs` is 18 KB. Both were read in
  full; the findings above are the ones that survive that reading, not the only ones
  present.
