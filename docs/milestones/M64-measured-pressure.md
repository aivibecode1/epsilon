# M64 — measured pressure, legal cuts, and a file ledger

**Scope:** the first three items of the port plan's Tier 1 (`audit/03-port-plan.md`) — the parts
that decide what the model sees once a session outgrows its context window. M63 named these as
the next work; this is that work.

Everything here is local to `epsilon-core` and needs no provider to test: pressure arithmetic,
the cut predicate and the ledger are pure functions with the loop's wiring around them.

## What

| # | Port plan item | Change |
|---|---|---|
| 1 | Usage-anchored token accounting (pi `harness/compaction/compaction.ts:207-250`) | the compaction trigger no longer re-estimates the whole transcript with `chars / 4`; it anchors to the prompt size the provider reported for the last request and estimates only the messages appended since (`compact::UsageAnchor`, `compact::anchored_tokens`) |
| 2 | Split-turn-safe compaction (pi `:312-422`; DSH `compaction/src/tool-pairing.ts:110`) | the retained tail is cut at a turn boundary, never inside a tool batch (`compact::legal_cut_point`) |
| 3 | File-operation ledger across compactions (pi `compaction/utils.ts:24-72`) | every summary now carries a harness-maintained `<file_ledger>` of what was read and what was changed, accumulated across compactions (`compact::FileLedger`) |

Plus `Agent::estimated_context_tokens()` — the figure the trigger actually decides on, public so a
frontend footer and the loop can never disagree. The compaction entry's `tokens_before` telemetry
now records that same number instead of the heuristic it replaced.

### 1. The trigger is a measurement

`compaction_due()` used to ask `compact::estimate_messages(&self.history())`: total characters ÷ 4.
That is a guess in a place where the provider has already sent the answer. Every response reports
`Usage.input`, and cache traffic is part of the prompt too (`input + cache_read + cache_write`), so
the loop now remembers that number together with the number of messages the request covered and
adds a heuristic estimate for whatever arrived after it.

The direction of the error matters. `chars / 4` over-counts prose (compacting early: a wasted
summary call, a wasted cache read, a context that shrinks for no reason) and under-counts code and
CJK by roughly the same factor (compacting late: the next request overflows and the loop falls back
to the overflow trigger). Anchor the prefix, estimate the tail, and both directions collapse to the
tail's own error, which is small by construction — the tail is one turn.

The anchor is *invalidated*, not trusted: it is dropped on compaction and on `restore_history`
(session switch, `/resume`, `/import`), because a number that describes a transcript which no longer
exists is worse than no number. A provider that reports nothing (`prompt_tokens == 0`) falls back to
the heuristic. A history shorter than the anchored length does the same: that is the signature of a
rewrite, and the guard is what makes the anchor safe to keep in a field.

### 2. The cut is legal

The tail budget is measured from the end of the transcript, which means it can land anywhere —
including between a tool result and the assistant turn that called it. The result stayed in the
tail, its call went into the summary, and the model received a request whose tool-result block had
no matching call. Strict wires reject that shape, so the failure lands *after* the summarization
call has been paid for.

`legal_cut_point` moves the tentative split to the nearest boundary (`!ToolResult`). Forward first:
dropping one more message costs less than an orphaned tail. Backward when the batch runs to the end
of the transcript: a tail slightly larger than the budget is a valid request, an orphaned tail is
not. A history that is somehow nothing but tool results resolves to the tentative index and is left
alone rather than looping.

### 3. The ledger is a fact, not a recollection

A summary is prose, and prose is lossy. After compaction the model reliably forgets which files it
has already opened and which it has already changed, so it opens them again — the most expensive
possible way to learn something the harness knew exactly. `FileLedger::scan` reads it out of the
tool calls (`read` → read; `write` / `edit` → modified; `grep` / `ls` / `glob` are excluded because
their `path` is a directory as often as a file, and a list whose value is precision cannot afford
noise), dedupes, and keeps first-touch order.

It rides along with the summary: inside the summarization request as a block the model is
explicitly told *not* to copy, and inside the stored summary message so the next compaction can
recover it (`from_summary`) and merge rather than restart. Coverage accumulates: each compaction
scans its own middle, and the tail it protects gets scanned by the next one.

## Key decisions

- **Measure where a measurement exists; estimate only what is new.** This is the whole of item 1,
  and it is the difference between a trigger that fires on arithmetic and one that fires on what the
  provider actually billed.
- **Invalidate the anchor at every rewrite, not just at compaction.** A stale anchor is a silent
  correctness bug in a field that otherwise looks like a cache: `restore_history` is the path a
  session switch takes, and the number it holds describes a transcript the process never sent.
- **Prefer a slightly wrong-size tail over an invalid one.** The cut walks backward when it has to.
  Token budget is a soft target; request validity is not.
- **Ledger facts live outside the template.** The summary template is the model's; the ledger is the
  harness's. Asking the model not to reproduce it keeps the two from blurring, and parsing it back
  out of the summary is reliable precisely because we control its shape.
- **Cap with evidence.** `(+N more)` rather than a quiet truncation — the block sits in the context
  of every subsequent turn, so its cost has to be bounded, and a reader who sees the block should
  know it is a window rather than the whole truth.

## Test-caught bugs

1. **`estimate_messages_before` had to move off the heuristic too.** Recording the old number in the
   compaction entry would have made the entry disagree with the trigger that produced it — telemetry
   that lies about its own cause. Found while wiring, fixed by recording
   `estimated_context_tokens()`.
2. **`UserContent` is not importable from `epsilon_contracts::message`.** The new test module
   imported it from the re-export root and failed to compile; it lives in
   `epsilon_contracts::content`, which the neighbouring module already knew. A reminder that the
   contracts crate's re-exports are the thing to check, not the module a type is *defined* in.
3. **The ledger block must not be emitted when empty.** The first `summary_with_ledger` appended
   `<file_ledger></file_ledger>` unconditionally; a session that touched no files would then carry
   an empty block in every subsequent turn. The test asserts byte-identity with the original text in
   that case.

## Verification

```
$ bash scripts/gate.sh m64
===== fmt --check =====      clean
===== clippy =====           0 warnings
===== test =====             548 tests, 0 failed, 1 ignored (the live smoke)
===== demo =====             completes
===== real-check =====       compiles
```

Sixteen new tests, all provider-free:

| Test | What it pins |
|---|---|
| `anchor_replaces_the_heuristic_for_the_prefix_it_covers` | provider truth for the prefix, heuristic for the tail; the anchor must *change* the answer |
| `anchor_is_ignored_when_it_cannot_describe_this_history` | no anchor / zero usage / shrunken history all fall back |
| `a_tail_never_starts_with_an_orphaned_tool_result` | forward walk to the next turn |
| `a_batch_running_to_the_end_grows_the_tail_instead_of_orphaning_it` | backward walk when there is no boundary ahead |
| `a_history_of_only_tool_results_is_left_alone` | degenerate input neither loops nor panics |
| `the_ledger_comes_from_the_tool_calls_themselves` | classification, dedupe, first-touch order, directory paths excluded |
| `a_ledger_rides_into_the_next_summary_and_comes_back_out` | round-trip through a summary; no block for an empty ledger |
| `successive_compactions_accumulate_instead_of_restarting` | merge keeps older entries and appends new ones once |
| `a_long_ledger_is_capped_and_says_how_much_it_hid` | `(+N more)` at the cap |
| `the_summary_request_shows_the_ledger_but_forbids_copying_it` | the summarizer sees the block and is told not to copy it |
| `no_generated_transcript_can_produce_an_illegal_tail` | 400 generated transcripts × every cut index: no tail ever starts with a tool result, and an adjusted cut always lands on a real turn |
| `an_adjusted_cut_never_loses_a_tool_result_its_call` | the same invariant stated over tail contents |
| `a_prose_like_session_is_not_compacted_early` (`tests/usage_anchor.rs`) | 8 turns of prose-like content: the provider says it fits, so nothing is compacted and no message is dropped |
| `a_dense_session_is_compacted_when_the_provider_says_so` | 1 turn of dense content already exceeds the budget by the provider's count while the char heuristic is nowhere near it |
| `the_estimate_tracks_the_provider_within_ten_percent` | the roadmap's acceptance number, over a 12-turn session |
| `a_provider_that_reports_no_usage_falls_back_to_the_heuristic` | a silent provider still compacts on the heuristic |

### The tests were checked against a mutated implementation

New tests that cannot fail are decoration, so both changes were disabled in turn and the suite
re-run:

| Mutation | Expected failures | Observed |
|---|---|---|
| `anchored_tokens` ignores its anchor (falls back to `chars / 4` everywhere) | the three anchor-dependent scenarios | 3 failed, 1 passed (the silent-provider fallback correctly does not depend on the anchor) |
| `legal_cut_point` returns the tentative index unchanged | the three hand-written cut tests + the property test | 4 failed |

Both mutations were reverted and the suite re-verified green. The `git` tree carries no trace of
them; the `if true` blocks were removed before the gate run that produced the numbers above.

## Follow-ups

The remaining Tier 1 items — cache-miss telemetry with attribution, verification enforcement and the
stale-read guard — plus the standing ones: committing M33–M64 (D-12), live-provider evidence for
every wire (D-13), release artifacts (D-21). See [`audit/07-plan.md`](../../audit/07-plan.md).
