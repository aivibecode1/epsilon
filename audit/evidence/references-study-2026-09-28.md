# References study — 2026-09-28

Read-only study of the four harnesses in `references/`, focused on the three numbers the
user actually watches: **context-window usage, cache hit rate, estimated cost**.

Read-only: nothing was built, no `cargo` was run, no repository file was modified except
this one. Nothing was executed, so every claim about behaviour is a claim about *source*, and
is labelled accordingly. Where a reference appears to do something I infer rather than read,
it says **"inferred"**.

Claim labels, per `audit/README.md`:
**[V]** verified by reading cited source · **[I]** inference from cited source ·
**[E]** not checked.

---

## 0. Corrections to `audit/05-metrics.md`

The prior audit was checked line by line before being used as a baseline. **Three of its
findings no longer describe the code**, and one of them was its headline defect.

| Audit claim | Status | Evidence |
|---|---|---|
| **$1** "subagent tokens never counted — *the most serious defect in this document*" | **FIXED — audit is stale** | `record_tool_usage` exists at `crates/epsilon-tui/src/state.rs:615-628` and is called from **both** arms: live `state.rs:1074-1077` and `/resume` hydration `state.rs:958-961`. Commit `12ae518 fix(tui): subagent tokens are counted in the cost, on both paths`. |
| **C1** DeepSeek rows say `163_840`, contradicted by vendor's 1M | **WRONG as stated — the table is correct for current models** | `catalog_data.rs:719-727` (`deepseek-flash`) and `:761-769` (`deepseek-v4-pro`) both carry `context_window: 1_000_000`. The `163_840` rows at `catalog_data.rs:1985-2032` are OpenRouter-routed legacy ids (`deepseek/deepseek-chat`, `deepseek-r1` at 64k) — a different provider route with genuinely different windows. The audit measured a routed id and generalised it to the family. |
| **K1** "Nothing in the `Usage` type records whether a provider is *able* to report cache tokens" | **WRONG — the capability flag exists** | `CacheStats::reports_cache_traffic()` at `crates/epsilon-core/src/cache_stats.rs:182-188`, and `report()` at `:246-252` already prints "this provider reports no cache traffic, so nothing can be attributed". It is a *core* capability, not a *TUI* one. The audit was right that the **header** never consults it (see §2) and wrong that the capability was absent. |
| **C2** no-anchor fallback omits system prompt + tool schemas | **CONFIRMED** | `anchored_tokens` walks `history` only (`crates/epsilon-core/src/compact.rs:46-53`); the system prompt is built separately at `crates/epsilon-core/src/lib.rs:2629-2682` and is not in `history`. Still open. |
| **$2** compaction summary cost missing until `/resume` | **CONFIRMED** | `CompactionFinished` at `crates/epsilon-tui/src/state.rs:1215-1220` sets only `status` and `compacted`. The summariser is a real provider call (`crates/epsilon-core/src/lib.rs:2324-2340`). Still open. |
| **$3** failed turns report zero usage | plausible, not independently re-probed here | — [E] |

**Consequence for this study:** the audit's own advice ("read `t.usage` in the one place the
other usage is already read, and route it through `record_usage`") has been executed. Work
below assumes the fixed state.

---

## 1. The three metrics, per reference

### 1a. Context-window usage

**pi** — anchored to the last *applicable* provider usage, with a staleness rule Epsilon
lacks. `calculateContextTokens` is `usage.totalTokens || input+output+cacheRead+cacheWrite`
(`packages/ai/src/utils/estimate.ts:17-19`); `getLastAssistantUsageInfo`
(`:63-84`) walks forward keeping the **newest** assistant whose timestamp is `>=` every
message seen so far, skipping `aborted`/`error` stops and zero-token usage. `estimateContextTokens`
(`:114-146`) adds the trailing messages after that index. Critically, when **no** anchor
exists it falls back to `estimateMessages` **plus** `systemPrompt` and `JSON.stringify(tools)`
(`:134-136`) — pi prices the system prompt and tool schemas. `getContextUsage`
(`packages/coding-agent/src/core/agent-session.ts:3383-3427`) adds a post-compaction guard: if
no clean assistant usage exists after the latest compaction entry, it returns
`{ tokens: null, percent: null }` (`:3415`) rather than a stale number. [V]

**opencode** — the weakest of the four and the clearest anti-pattern.
`packages/app/src/components/session/session-context-metrics.ts:27-30` defines
`tokenTotal = input + output + reasoning + cache.read + cache.write`, and `:58` computes
`usage = round(total/limit*100)`. It **includes output tokens in the context window**, which
is a different quantity: output is not in the *next* request's prompt. It also re-derives
everything from the last assistant message (`lastAssistantWithTokens`, `:33-44`) with no
trailing-delta walk, so the figure lags by a whole turn. [V]

**DSH** — the most rigorous. The type is explicit that the fields are deliberately *not* one
atomic observation (`packages/llm/token-meter/src/projection.ts:23-49`) and splits
occupancy into three:
- `pressureTokens` — provider-reported prompt side of the last request, `input + cacheRead + cacheWrite`, **output excluded** (`usage-projection.ts:78-80`). Holds still while a turn streams.
- `projectedTokens` — `max(0, pressureTokens + surfaceTokens - sampledSurfaceTokens)` (`usage-projection.ts:215`), i.e. the pressure plus the **signed** movement of the surface since the sample. This is the number that answers "what will the next request cost", and it survives a compaction, which `pressureTokens` alone cannot see.
- `contextWindow` — newest recorded capacity, independent slot.

The heuristic is a shared module both the meter and the breakdown projection call, so identical
content prices to identical numbers (`estimate.ts:13-19` — `CHARS_PER_TOKEN = 4`,
`BLOCK_OVERHEAD = 4`, `ROLE_OVERHEAD = 4`), and it prices the system prompt
(`estimateSystemMessage`, `:74-83`) and the tool schemas (`estimateToolsTokens`, `:102-104`)
as first-class surface nodes. `TokenMeasurementBaseline` is a three-way sum type:
`{kind:'none'} | {kind:'estimated'} | {kind:'usage'}` (`types.ts:15-19`). [V]

**Hermes** — anchored, but with the best **invalidation** discipline. `usage_anchor.py`
stores `prompt_tokens`, `completion_tokens`, `base_count`, `base_last_role` and
`base_last_fp` — a SHA-256 fingerprint over the provider-visible persisted fields
(`usage_anchor.py:31-42`). `_anchor_matches` (`:65-77`) re-hashes the message at
`base_count - 1` on every use; compaction, splices and rewinds all change it, so the anchor
**fails closed** to full estimation. The module docstring is explicit that the fingerprint
(not object identity) is used because the gateway re-reads the transcript from the DB every
turn and a resumed session is a fresh process. `anchored_context_tokens` (`:79-95`) returns
`Optional[int]` and skips the anchored response's own reply, which is already in
`completion_tokens`. Provenance is published, not implicit: `context_display_source`
(`context_breakdown.py:94-102`) returns `"local_estimate"` or `"provider_usage"`, and
`context_usage_fields` (`:105-115`) ships `context_source` + `context_estimated` alongside
the number. [V]

**Epsilon today.** `estimated_context_tokens` (`crates/epsilon-core/src/lib.rs:2189-2202`)
→ `anchored_tokens` (`crates/epsilon-core/src/compact.rs:46-53`): `anchor.prompt_tokens +
estimate_messages(&history[covered..])`, else `estimate_messages(history)`. Mirror is taken
once per frame (`app/view.rs:300-302`, called from `app/event.rs:169`). Denominator is the
catalog snapshot (`app/view.rs:236`).

**Epsilon is already better than pi/opencode on one axis:** it anchors the *prefix* to
provider truth and estimates only the tail, and — the thing the audit called a strength and
is genuinely one — **the compaction trigger reads the same function** (`lib.rs:2167-2178`),
so the number on screen and the number the loop acts on cannot disagree. Keep that.

**Epsilon is worse than all four on two axes:** the fallback omits system prompt + tool
schemas (audit C2, confirmed), and the anchor has no staleness fingerprint — only
`covered <= history.len()` (`compact.rs:47`), so a *rewrite that keeps the length* (a
compaction that lands a similar number of messages, a `/resume`) leaves a stale anchor live.

### 1b. Cache hit rate

**Nobody in these four solves "not measurable vs zero" at the wire level the way it should.**

- **pi** computes the rate inline in the footer: `(cacheRead / promptTokens) * 100`
  (`packages/coding-agent/src/modes/interactive/interactive-mode.ts:6220-6221`). No guard
  against a zero denominator, no capability concept. [V]
- **opencode** has no hit rate at all in the reviewed surface — `session-context-metrics.ts`
  is context-only. [E for the rest of the tree; I did not find a hit-rate display.]
- **DSH** reports `cacheReadTokens`/`cacheWriteTokens` as disjoint durable buckets
  (`usage-projection.ts:20-25`, `projection.ts:19-32`) and has **no** published hit-rate
  display. The buckets are the trustworthy part; the rate is left to the surface. [V]
- **Hermes** is the only one that made the rate a first-class, tested quantity, and its two
  ideas are the valuable ones:
  1. **It hides the rate rather than showing zero.** `cli_status_bar_mixin.py:370-371`:
     `if delta_prompt > 0 and delta_read > 0:` — a zero-read regime leaves `pct = None` and
     the renderer prints nothing. The comment says it outright: *"A zero-read regime hides the
     segment (no data ≠ an alarming 0%)"*. The wire contract makes it explicit:
     `cache_hit_pct: int | None` (`tui_gateway/contracts/common.py:34`), and
     `tui_gateway/server.py:1957-1959` only sets the key when both totals are positive, so
     absence is representable end-to-end. [V]
  2. **It rebases the numerator and denominator on the events that invalidate the cache.**
     A `/model` switch or a compaction destroys the prompt cache, so both counters are
     re-baselined at that instant (`cli_status_bar_mixin.py:343-365`, `_rebase(tokens=True)`),
     and the displayed rate describes *the current cache regime* rather than the session's
     average. The docstring at `:54-58` says the delta pct "reflects the *current* cache
     regime" and is preferred over the lifetime ratio. [V]

  Hermes also normalises every wire shape into one `CanonicalUsage`
  (`agent/usage_pricing.py:484-490` lists the OpenAI `prompt_tokens_details.cached_tokens`,
  Anthropic `cache_read_input_tokens`, DeepSeek `prompt_cache_hit_tokens`, and bare
  `cached_tokens` fallbacks, in that order), and explicitly subtracts cached tokens out of the
  prompt total for the non-Anthropic shapes (`:517-520`) so the buckets stay disjoint —
  the same discipline Epsilon applies at the wire boundary. [V]

**Epsilon today.** `Metrics::cache_hit_rate` = `cache_read_last.min(prompt_tokens_last) /
prompt_tokens_last`, `None` below 1,000 prompt tokens (`ui/metrics.rs:170-180`); numerator
and denominator from the same call (`ui/state.rs:1067-1071` live, `:951-956` on resume).
Printed as `R{} W{} CH {}` with `CH —` for no rate (`ui/metrics.rs:220-236`).

**Epsilon's numerator/denominator pairing is correct and matches Hermes's formula.** The
gaps are exactly the two Hermes ideas Epsilon lacks: (i) the header never consults
`reports_cache_traffic()`, so a provider that cannot report cache prints `CH 0%` in the
**Value** role (`metrics.rs:334-338`) — the core already knows better; (ii) no rebase on
model switch or compaction, so the per-call rate has audit defect K2's property that a
legitimate miss reads as a failure.

### 1c. Cost

**pi — the best cost model of the four, and the one thing here Epsilon should copy
wholesale.** `calculateCost` (`packages/ai/src/models.ts:891-910`) runs at the **wire
adapter**, not in the UI: each provider calls it with the model and the usage it just parsed
(`api/anthropic-messages.ts:616,778`, `bedrock-converse-stream.ts:696`,
`google-generative-ai.ts:242`, `openai-completions.ts:1546`,
`openai-responses-shared.ts:576`, `mistral-conversations.ts:607`). Two capabilities Epsilon's
four-bucket model **cannot express at all**:
- **Tiered rates** (`:893-901`): a `tiers` array keyed on total prompt tokens, selecting the
  matching rate row for the whole request. This is how Gemini's >200k pricing is real.
- **1-hour cache writes at 2× input** (`:903-908`): `cacheWrite1h` is split out of
  `cacheWrite`, and `cost.cacheWrite = (rates.cacheWrite * shortWrite + rates.input * 2 *
  longWrite) / 1e6`. Anthropic charges 2× base for a 1h write versus 1.25× for 5m —
  audit defect $4 said a single `cache_write` rate "cannot express this at all". It cannot;
  **pi expresses it** with one extra counter (`cacheWrite1h`, populated at
  `anthropic-messages.ts:612`).

Cost is **carried on the usage object** (`types.ts:310`, `:397`) and accumulated, never
recomputed: `addUsage` sums the four components and the total
(`packages/agent/src/harness/utils/usage.ts:14-34`), and `addUsageToTotals` sums
`usage.cost.total` (`packages/coding-agent/src/core/usage-totals.ts:23-29`).
`getUsageCostBreakdown` (`:37-77`) groups by `provider/model` for assistant messages and
routes tool-result usage, branch summaries and compaction usage into an explicit
**`"Tools/summaries"`** bucket (`:47-50`) — so audit $2 (compaction cost invisible) does not
exist in pi. Cost override per message is a tested regression
(`test/suite/regressions/3982-message-end-cost-override.test.ts`). [V]

**opencode** persists cost as a durable column and updates it by delta
(`packages/core/src/session/sql.ts:43` `cost: real().notNull().default(0)`;
`session/projector.ts:98` `cost: ${SessionTable.cost} + ${value.cost * sign}`). The `* sign`
is the interesting part: corrections subtract. [V]

**DSH** has token metering but **no cost table** — `token-meter` prices *tokens against a
route* (`route-pricing.ts:32-49`, which is about request-representation pricing for images
and files, not money). DSH defers cost. [V]

**Hermes — the best provenance model, by a wide margin.** `CostResult`
(`agent/usage_pricing.py:112-124`) carries `amount_usd: Optional[Decimal]`, a
**`status`** from `"actual" | "estimated" | "included" | "unknown"` (`:48`) and a **`source`**
from `"provider_cost_api" | "provider_generation_api" | "provider_models_api" |
"official_docs_snapshot" | "user_override" | "custom_contract" | "none"` (`:49-51`), plus
`pricing_version`, `source_url`, `fetched_at` and `notes`. `PricingEntry` (`:88-110`) carries
per-bucket `Optional[Decimal]` rates **and** whole-request tier rates
(`tier_threshold_tokens`, `*_above`), where the comment at `:99-101` notes the above-threshold
rates replace the base "for the WHOLE request (Google's semantics, not marginal brackets)".
`estimate_usage_cost` (`:552-603`) is a subscription-route check (`:557-561` → `status:
"included"`), a missing-entry bail (`:565` → `_unknown_cost`), and then a per-bucket loop
(`:570-584`) where **a missing rate for a bucket that moved tokens aborts the whole total** to
`unknown` — it does not silently count them at zero. The `_SNAPSHOTS` table (`:152-230`) is
hard-coded *from vendor pages* with a `pricing_version` and a `source_url` per group, and
encodes DeepSeek's off-peak rates with the peak window in a comment (`:198-201`).

Display honesty is handled by `format_cost_label` (`:29-45`): below $0.01 it renders 4 dp
(`~$0.0046`) and, if that still rounds to zero, `~$<0.0001` — the label never reads as zero
while the amount is non-zero. [V]

**Epsilon today.** `breakdown_per_model` (`ui/cost.rs:173-196`) prices each model id at its
own rates and ORs the `floor` flag; `is_floor` (`:118-140`) fires when a bucket that moved
tokens has no published rate; rendered as `$0.42+` (`ui/metrics.rs:273-282`). Integer
micro-USD, four disjoint buckets, sum divided once (`:86-88`).

**Epsilon already matches or beats all four on:** per-model attribution (M141,
`cost.rs:156-172`), the floor marker, order-independence, and — importantly — pricing
*correctness as a tested property* (`cost.rs:198-464` is a real test suite, not a comment).
Epsilon's four-bucket `breakdown` is more carefully argued than anything in the references.

**Epsilon lacks, versus the references:**
- tiered pricing (pi `models.ts:893-901`, Hermes `usage_pricing.py:99-101`) — DeepSeek's
  off-peak halving (audit $4) and Gemini's >200k are both invisible to it;
- 1h cache-write pricing at 2× (pi `models.ts:903-908`) — audit $4 called this unexpressible;
- any notion of **where the number came from** (Hermes `CostSource`): Epsilon has one
  provenance bit (`cost_is_floor`) where Hermes has seven named sources.

---

## 2. Ranked recommendations

Ordered by (user harm × confidence) ÷ size. **R1–R4 are the ones that make the three numbers
trustworthy; R5+ are the rest.**

### R1 — Give `Metrics` a provenance bit for context, and use the capability flag that already exists
**Size: small. Highest value. Closes audit C2 + K1 together.**

Epsilon already computes `reports_cache_traffic()` (`core/cache_stats.rs:182-188`) and the
core report already refuses to attribute when it is false (`:246-252`). The header just
never asks. Concretely:

1. In `crates/epsilon-core/src/lib.rs`, add one accessor next to
   `estimated_context_tokens` (`:2189`), e.g.
   `pub fn context_is_estimated(&self) -> bool` — true when the last
   `set_usage_anchor` was cleared and no fresh anchor has landed, i.e. whenever
   `anchored_tokens` took its `_ =>` arm (`compact.rs:50`).
2. Mirror it per frame in `App::mirror_context_size` (`app/view.rs:300-302`) into
   `UiState`, and add `context_estimated: bool` to `Metrics` (`ui/metrics.rs:34-103`).
3. In `Metrics::cache_gauge` (`ui/metrics.rs:220-236`), when the agent reports
   `!reports_cache_traffic()` **and** `cache_read_session + cache_write_session == 0`, print
   `CH n/a` in the `Label` role instead of `CH 0%` in `Value`. This is the sentence
   "your cache hit rate is zero" vs "this provider cannot tell me your hit rate" that the
   audit said needed a capability flag — the flag is already there, it is just not plumbed.
4. Mirror Hermes's `context_source` string (`context_breakdown.py:94-102`): publish
   `"provider_usage"` / `"local_estimate"` alongside the number so the header can show
   `ctx <1%/200k*` where `*` means estimated. Epsilon's header already has a `Label` role
   (`ui/metrics.rs:296-307`) and a documented convention for an honest unknown
   (`metrics.rs:182-187` `CONTEXT_UNKNOWN`); this extends an established pattern rather than
   inventing one.

**Why this is right and not cosmetic:** DSH's projection doc
(`token-meter/src/projection.ts:23-27`) says the occupancy figure is "a user-facing reference,
not a billing or gating input" — and the only way a user can act on that is if the figure
says how it was made. Hermes ships `context_estimated` on every status payload for exactly
this reason.

### R2 — Price the system prompt and tool schemas in the no-anchor fallback
**Size: small. Closes audit C2, the defect that bites hardest right after a compaction.**

`anchored_tokens`'s fallback arm (`compact.rs:50`) is `estimate_messages(history)`, and
`history` does not contain the system prompt or tool schemas. Epsilon *has* both at request
build time: `prompt::build(...)` at `lib.rs:2629` and `self.tools.descriptors()` at
`lib.rs:2682`. pi does exactly this in its own fallback
(`packages/ai/src/utils/estimate.ts:134-136`: `estimateTextTokens(context.systemPrompt) +
estimateToolsTokens(context.tools)`), and DSH prices both as surface nodes
(`token-meter/src/estimate.ts:74-83`, `:102-104`).

Implementation: store the rendered system prompt and the serialised tool descriptors on the
`Agent` when the request is built (`lib.rs:2629-2682`, both already computed there and
discarded), and have the fallback arm add
`estimate_tokens(system_prompt) + estimate_tokens(serde_json::to_string(&descriptors))`.
Add ~4 tokens of per-message role overhead per DSH (`estimate.ts:19`) — free, and it makes
the heuristic consistent with what the provider actually bills.

**Measurable gate:** a test that asserts the no-anchor fallback is **greater** than
`estimate_messages(history)`, and that a one-shot faux session with a system prompt reports a
non-zero context on turn 1. The faux provider already reports `input: 100`
(`epsilon-llm/src/faux.rs:167-171`), so the number is reachable in a test today.

### R3 — Adopt DSH's `pressureTokens` / `projectedTokens` split
**Size: medium. This is the single best idea in the four references for Epsilon.**

DSH publishes two numbers, not one (`token-meter/src/projection.ts:34-49`):
- `pressureTokens` = provider prompt-side of the last request, output excluded
  (`usage-projection.ts:78-80`) — *what the provider says the prompt was*;
- `projectedTokens` = `max(0, pressureTokens + surfaceTokens - sampledSurfaceTokens)`
  (`usage-projection.ts:215`) — *what the next request will be*, because it adds the signed
  movement of the surface since that sample.

The doc comment at `usage-projection.ts:145-152` names the exact failure this fixes:
*"it also cannot see a compaction: the fold therefore carries a running surface total
alongside it and publishes `projectedTokens` … so occupancy answers for the next request
rather than the last one."*

**Epsilon already implements DSH's algorithm** — `anchored_tokens` is
`prompt_tokens + estimate_messages(&history[covered..])`, which is the same arithmetic, and
it already agrees with the compaction trigger. So this is *not* a rewrite. What DSH adds is
the **pair**, and the pair changes what the UI can say. Concretely, in `UiState` add a
`context_pressure: Option<u64>` mirrored from `latest_prompt_tokens` (already tracked at
`state.rs:1067-1071`), and give the `/session` or `/context` panel — not the header — both:
`ctx 41% (last request) · 44% (next request)`. During a long turn, and immediately after a
compaction, these two diverge, and that divergence is the most useful thing the harness
could tell a user who is deciding whether to compact manually.

Keep the header on `projectedTokens` only. Two numbers in a header is a regression, not a
feature.

### R4 — Copy pi's `cacheWrite1h` and Hermes's `CostStatus`/`CostSource`
**Size: small for the first, medium for the second. Closes audit $4.**

**(a) `cacheWrite1h`** — pi tracks 1-hour cache writes as a separate counter
(`packages/ai/src/types.ts:389`, populated from
`cache_creation.ephemeral_1h_input_tokens` at `api/anthropic-messages.ts:612`) and prices
them at 2× input while 5-minute writes take the normal rate
(`api/../models.ts:903-908`). Epsilon's `Usage` (`crates/epsilon-contracts/src/message.rs:120-131`)
has four counters and no fifth. Adding `cache_write_1h: u64` and changing
`TokenCounts`/`TokenPrices` (`ui/cost.rs:26-51`) to a fifth disjoint bucket turns audit $4
from "unexpressible" into "expressed", at the cost of one field and one product term. The
`breakdown` array is already a fixed `[u64; 4]` with named indices (`cost.rs:71-78`) — a
fifth index is mechanical. **[I]** that this is cheap; I did not run the build.

**(b) provenance** — Hermes's `CostResult` (`agent/usage_pricing.py:112-124`) is the right
shape and Epsilon's `cost_is_floor: bool` (`ui/metrics.rs:95-102`) is a one-bit reduction of
it. Replace/extend the bool with:

```rust
pub enum CostProvenance {
    Priced,                        // every bucket had a published rate
    LowerBound(CostGap),           // priced total, but something real is unpriced
    Unpriced,                      // no registry row -> omit, as today
    Included,                      // subscription route: $0 and no provider invoice
}
pub enum CostGap { UnpricedModel, MissingCacheRead, MissingCacheWrite, MissingTier }
```

`Included` is the one Epsilon has no concept of and cannot distinguish: a subscription route
is `$0` because the user is not billed, which is a **different fact** from a free model, and
Hermes attaches `_INCLUDED_NOTE` to every such result (`usage_pricing.py:24-27`) precisely so
consumers can tell them apart. `CostGap` is what makes the `$+` marker actionable instead of
decorative — today `+` says "under-report, cause unknown".

**Do not** port Hermes's whole `PricingEntry` (`:88-110`) and `CostSource` (`:49-51`) union.
Epsilon generates its table from models.dev via `scripts/regen_catalog.py`
(`catalog_data.rs:1-6`), which is a better provenance story than a hand-maintained table, and
Hermes's `source_url` / `fetched_at` fields have no consumer in Epsilon today. That is R7, not
R4.

### R5 — Rebase the cache-hit rate on model switch and compaction
**Size: small. Closes audit K2.**

Take Hermes's `cli_status_bar_mixin.py:343-371` literally. In `UiState`, keep
`cache_baseline_prompt` / `cache_baseline_read` / `cache_baseline_model` /
`cache_baseline_compactions`. On a `/model` change or a `CompactionFinished`
(`state.rs:1215`), re-baseline the counters to the current totals. The rate then describes
the current cache regime, and the first turn after a rebase shows `—` rather than a
confident `0%` — which is Hermes's `delta_read > 0` guard at `:370`. Hermes has tests for
exactly this (`tests/hermes_cli/test_cli_status_bar.py:680-710`; the assertions name the
cases: post-model-switch `None`, post-compression 400/4000). Epsilon's `metrics.rs` test
module (`:364-647`) is structured to receive these as four more cases.

### R6 — Fingerprint the usage anchor instead of trusting its length
**Size: small. Prevents a wrong gauge; no visible feature.**

Hermes's `_FINGERPRINT_KEYS` + `message_fingerprint` (`usage_anchor.py:31-42`) and
`_anchor_matches` (`:65-77`) exist because a length check is not an identity check. Epsilon's
guard is `a.covered <= history.len()` (`compact.rs:47`), which passes after a compaction that
happens to produce a similar message count — and the anchor then describes a transcript that
no longer exists. Epsilon already clears the anchor on commit
(`lib.rs:2484`), so this is defence in depth for `/resume` and any other rewrite
(`restore_history`). Add a `covered_fingerprint: u64` to `UsageAnchor` (a cheap hash of the
last covered message's id + content length), and treat a mismatch as a fallback. The field
is 8 bytes and the check is O(1) per frame.

### R7 — Record cost at the wire adapter, not at render
**Size: medium. The structural fix; do it after R1–R6.**

Every pi adapter calls `calculateCost(model, usage)` with the model and the usage it just
parsed (`api/anthropic-messages.ts:616,778`; `bedrock-converse-stream.ts:696`;
`google-generative-ai.ts:242`; `openai-completions.ts:1546`;
`openai-responses-shared.ts:576`; `mistral-conversations.ts:607`). Epsilon prices in the draw
path (`app/view.rs:204-216`) by looking up the catalog from per-message model ids, which is
why audit $5 exists: **a catalog refresh silently reprices history.** If cost were computed
at the adapter and stored on the `Usage` (or on the message), the number would be a fact
about a past event rather than a function of today's price list — which is what opencode
does with its durable `cost` column and signed deltas
(`packages/core/src/session/sql.ts:43`, `session/projector.ts:98`).

`Usage` already has a `total_tokens` field that is populated but unused for pricing
(`message.rs:129-130`), so the type has room. This also subsumes audit $3: a salvage that
streams text and then fails has already had its adapter compute a cost from whatever usage
arrived, instead of falling through to `Usage::default()`.

**This is the largest item and the one most likely to be done wrong**, because it touches
every wire. Sequence it last and gate it on R4 landing first (so the stored figure has
provenance attached to it).

### R8 — Windows: the one opencode behaviour Epsilon should copy
**Size: small. `packages/tui/src/terminal-win32.ts` (130 lines, FFI to kernel32).**

opencode does three things Epsilon does not:
- `win32DisableProcessedInput()` (`:30-45`) clears `ENABLE_PROCESSED_INPUT` on stdin so the
  console stops line-buffering keystrokes;
- `win32FlushInputBuffer()` (`:47-…`) discards queued input (mouse events, buffered keys) at
  a hand-off point — the same problem Epsilon solves for `/editor`
  (`app/event.rs:114-119`);
- `win32InstallCtrlCGuard()` (`:69-130`) **re-asserts** the mode on an interval, with an
  explicit comment: *"Some runtimes can re-apply console modes on the next tick; enforce
  twice."* It wraps `stdin.setRawMode` and also runs `setImmediate(enforce)`.

The generalisable idea: **console modes are not sticky, so a one-shot set is not a set.** If
Epsilon ever sets console modes directly, it must re-assert. Epsilon's existing loop is
already good on responsiveness grounds — `dirty` + `frame_due` + `FRAME_MIN = 33ms`
(`app.rs:76`, `app/event.rs:110,167-181`) with a biased `select!` that serves input first
(`event.rs:190-193`), and a measured Windows justification in the comment at `event.rs:100-108`.
pi's `requestImmediateRender` (`tui/src/tui.ts:961-978`, called at `:1076-1080` after
`handleInput`) is the *same* idea from the other side, and its comment is worth quoting into
Epsilon's: *"Keyboard input is latency-sensitive. Avoid the throttled timer path, where even
setTimeout(0) can take a full 16 ms tick on Windows."* **[I]** Epsilon's `FRAME_MIN` is a
ceiling on a dirty frame, not a delay imposed on a keystroke, so it is not the same bug — but
the invariant "a keystroke must not wait for a frame timer" is worth writing down as a
property of the loop.

### R9 — Sub-cent cost display
**Size: very small. Hermes `agent/usage_pricing.py:29-45`.**

Epsilon's `format_cost` is used for a whole session and is fine, but a per-turn or per-model
breakdown (the thing `getUsageCostBreakdown` gives pi,
`coding-agent/src/core/usage-totals.ts:37-77`) will hit sub-cent values. Hermes renders 4 dp
below $0.01 and, when that still rounds to zero, `~$<0.0001` — the label never reads as zero
while the amount is not zero. Adopt verbatim if a per-turn cost is ever shown.

---

## 3. Traps — things that look like strengths and are not, at Epsilon's stage

1. **DSH's `route-pricing.ts` (image/file request pricing, `:32-49`) is a trap.**
   It is elegant — and it prices the *request representation*, replacing a structural
   heuristic with the actual projected image token count. But it exists because DSH's
   surface is a durable, replayable log with an epoch header carrying the tool envelope
   (`estimate.ts:102`). Epsilon has `descriptors()` at request build time; the moment is
   already there. **[I]** Do not port the machinery — but do take the *shape*: the request
   envelope is real data Epsilon already has and currently throws away (same data R2 needs).

2. **opencode's context metric is a trap in the most dangerous way: it looks like the
   obvious implementation.** `tokenTotal = input + output + reasoning + cache.read +
   cache.write` (`session-context-metrics.ts:27-30`) is a one-liner that reads
   self-evidently correct. It includes output, which is not context. If Epsilon ever
   "simplifies" `anchored_tokens` into a sum of the four `Usage` counters, it will have
   copied this bug. Epsilon's anchored-plus-tail design is better; protect it with the test
   `metrics.rs:458-488` already provides and add a comment naming this trap at
   `compact.rs:46`.

3. **Hermes's `usage_anchor.py` is a trap if copied literally.** Its `base_last_fp`
   fingerprint exists because *"the gateway re-reads the transcript from the DB every turn
   and a resumed session runs in a fresh process, so object identity is never stable"*
   (`usage_anchor.py:8-11`). Epsilon's history lives in one process behind a `Mutex`
   (`lib.rs:608`, `:670`), so the failure mode it defends against **does not exist here**.
   A full port buys nothing; the cheap identity check in R6 buys most of it. Take the
   *principle* (an anchor must prove the transcript it priced is still the transcript),
   not the hashing machinery.

4. **pi's per-adapter `calculateCost` is a trap to copy *prematurely*.** It is the right
   design (R7) but it is also a change to seven wire adapters plus a new counter. Doing it
   before R4 means storing an un-provenanced number. R4 → R7, not the reverse.

5. **Hermes's `context_breakdown.py` category breakdown (`:19-28`, 8 categories: system
   prompt, tools, rules, skills, MCP, subagents, memory, conversation) is a trap now.**
   It is genuinely the best breakdown in the four, and it is a *display* feature that
   requires Epsilon's prompt to already be decomposed into those exact tiers. Epsilon's
   prompt.rs is 47 KB (`crates/epsilon-core/src/prompt.rs`) and its ADR-008 three-tier
   structure is not the same partition. Port R2 first — it gets the system prompt and tools
   into the number — and revisit the breakdown only if the tier structure already lines up.

6. **A durable, replayable session projection (DSH's whole design) is a trap at Epsilon's
   stage.** `contextPressureProjectionDefinition` is beautiful and has 5 state versions
   (`usage-projection.ts:171`). But its correctness comes from being a pure fold over an
   append-only log, which is a storage-architecture commitment, not a UI feature. Epsilon
   has a session store and a repair path (`crates/epsilon-session/src/repair.rs`) and should
   not be rewired to chase it. **Steal the arithmetic (R3) and the type discipline
   (`TokenMeasurementBaseline`, `types.ts:15-19`); do not steal the fold.**

7. **Hermes's `tools/approval_smart.py` LLM-as-approver is a trap.** Its own docstring
   admits the command text is untrusted and may be prompt-injected (`:1-9`), and it defends
   by stripping shell comments, wrapping in XML delimiters, and telling the model to ignore
   embedded directives (`:23-34`). That is mitigation, not a boundary — it costs an extra
   model call per approval. Not a metrics question, and not a safe default for a harness
   that already has an explicit `PermissionDecision` enum
   (`epsilon-contracts/src/agent_event.rs`, `DenyWithGuidance` at `state.rs:1205`).
   Epsilon's deterministic path is the better default; the LLM reviewer would be an
   opt-in escalation tier, not a replacement.

---

## 4. What Epsilon already does better than all four references

Worth recording so these are not regressed while the above is ported.

- **The compaction trigger and the displayed gauge read the same function**
  (`lib.rs:2167-2178` and `lib.rs:2189` both go through `anchored_tokens`). pi's trigger
  (`agent-session.ts:548-549`) and display (`getContextUsage`, `:3383-3427`) are separate
  code paths over separate estimates. DSH's is a projection and its compaction trigger is
  elsewhere. This single property is worth more than several of the items above.
- **Cost arithmetic is defended by a real test suite**, not a comment
  (`ui/cost.rs:198-464`: switch-does-not-reprice, unpriced-model-is-a-floor-not-a-smaller-bill,
  order-independence, divide-once, cached-tokens-not-at-input-rate, free-model-is-not-a-floor).
  pi's equivalent correctness argument is a doc comment on `calculateCost`.
- **The floor marker (`$0.42+`, `metrics.rs:273-282`) is the honest-move idea**, implemented
  without the seven-way provenance union. R4 extends it; it does not replace it.
- **`CH —` and `?/200k` and `<1%`** (`metrics.rs:114-120`, `:187`, `:232-235`) — the
  module's own doc comment (`:1-24`) states the rule: *"when a number is not known, the type
  says so instead of printing a zero that reads like a real measurement."* R1 completes
  this discipline where it is currently incomplete, rather than introducing it.
- **Cache-miss attribution** (`crates/epsilon-core/src/cache_stats.rs`) — naming *why* a
  call was not served (`ModelSwitch`, `PrefixChanged`, `MissCause` at `:36-50`) and pricing
  only the re-billed tokens at the input rate (`:15-16`). None of the four references has
  this. It is a port of pi's `cache-stats.ts` but it is a real strength.

---

## 5. Honest limits

- **Nothing was executed.** No `cargo build`, no `cargo test`, no provider call, no TUI run.
  Every claim is a reading of source at the cited line. Statements about what a harness
  *would* do at runtime are marked **[I]**.
- **Line numbers are from the working tree as of 2026-09-28** and the references are
  read-only, un-committed checkouts. If a reference is re-synced, citations must be re-pinned.
- **§0 is a partial re-audit.** I verified $1, C1, K1, C2 and $2 directly. I did **not**
  re-probe $3, $4 or $5, and did not re-verify any provider wire mapping. `$4`'s
  time-of-day DeepSeek claim is neither confirmed nor refuted here; the *expressibility*
  point stands regardless, because pi and Hermes both have the mechanism and Epsilon does not.
- **Not studied in depth** (out of scope for the three metrics, mentioned only where they
  intersect): pi's diff renderer, opencode's plugin system, DSH's `native`/terminal layer,
  Hermes's approval gateway. The approval material in §3.7 is read from one file's docstring
  and header.
- **R7's blast radius is unmeasured.** Seven wire adapters plus a persisted figure and a
  migration path for sessions written under the old repricing model. It is the one item here
  I would want a written plan for before starting, not a patch.
