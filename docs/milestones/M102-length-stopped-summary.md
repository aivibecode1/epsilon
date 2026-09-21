# M102 — a summary cut off by the output limit is not committed

## What

`summary_text` refused an empty answer and accepted anything else, including a summary the
provider stopped with `MaxTokens`. The gate above it did not catch the case either: `is_failure()`
covers `Error` and `Aborted`, and `MaxTokens` is not a failure — it is how a provider says "I ran
out of room and stopped mid-sentence". So a truncated summary passed every check and was
committed over the transcript it was supposed to compress.

The empty case is obviously broken, and that is exactly why it is the less dangerous of the two.
**A truncated summary has a Goal section and a Progress section.** It reads as a complete record
of a session that simply had less in it — and committing it replaces the real transcript with
that plausible-looking shorter one. A blank summary announces itself; this one does not.

`compact::summary_text` now returns `None` for a `MaxTokens` answer, and the caller's failure
message says which of the two it was, because they need different responses from whoever reads
the log:

```
summarization hit the output limit and was cut off; a truncated summary is worse than
none because it reads as complete
```

pi refuses a `length`-stopped summary for the same reason (`compaction.ts:549-551`), so this
restores a fidelity behaviour the port had dropped.

## Key decisions

- **The refusal lives in `summary_text`, not in `is_failure()`.** `is_failure()` describes the
  transport: the turn did not complete. A truncated turn *did* complete and produced text. The
  two are different statements, and folding `MaxTokens` into the failure predicate would have
  changed what every other caller of `is_failure()` means — including the places that decide
  whether a turn's output is worth keeping at all.
- **Three tests, and the second two matter as much as the first.** A length-stopped summary is
  refused, an ordinary `EndTurn` summary is still accepted with its exact text, and an empty one
  is still refused. A fix that refused more than the bug would be a different bug, and the
  easiest over-refusal here is to treat "short" as "truncated" — the test that pins the accepted
  case is what makes that impossible.
- **The two refusals get different words.** An empty answer and a cut-off answer both mean "no
  summary was committed", but the empty one means the provider returned nothing and this one
  means the answer was plausible and incomplete. A single message would have hidden the
  distinction the fix exists to make.
- **The empty-answer rule stays labelled as Epsilon's own.** The doc comment says plainly that
  the nonempty check is this harness's reliability rule, not a claimed parity behaviour — pi's
  inspected helper returns the extracted text without one. The `MaxTokens` refusal *is* parity,
  and the comment says that too. Keeping the two claims apart is what stops a later reader from
  citing parity for something pi never did.

## Gotchas

- **`is_failure()` was the wrong gate for this, and it looks like the right one.** The check
  immediately above the fixed line reads `if answer.stop_reason.is_failure()`, and the audit's
  own "smallest fix" was to extend it to `|| answer.stop_reason == StopReason::MaxTokens`. That
  would have worked here and been wrong as a rule: `MaxTokens` is a stop reason a run can
  legitimately finish with, and `is_failure()` is consulted in several places that have nothing
  to do with summarization.
- **A truncated summary is not short.** It can be a full page of text with several complete
  sections. Nothing about its length distinguishes it from a good summary of a shorter session,
  which is why the *stop reason* is the only reliable signal and why no heuristic on the text
  would have been acceptable.
- **The failure is silent at the point of commit.** `summary_text` returns `Option<String>`, so
  the caller has to choose the words; the reason the message names the output limit is that the
  stop reason is no longer visible by the time the user reads it.
- **What the caller does with the refusal is state, not a fallback.** `CompactionOutcome::Failed`
  is emitted as a `Notice`; on the pressure path the attempt is suppressed for the rest of the
  run (`compact_suppressed`) because every retry is a paid provider call, and on the overflow
  recovery path the run ends with `TurnEndReason::Error(why)`. So the real outcome is: the
  summary is never committed, the transcript is left as it was, and the harness says why. The
  commit message's phrase "falls back to context-free pruning" describes the degradation loosely
  — the pruning is what the *next* attempt would do; what this code path does is not commit and
  not pretend (see Limits).

## Verification

No battery log for this slice is preserved, and no gate log exists for M102 (`audit/evidence/`
holds `gate-m93b`, `gate-m94c`, `gate-m95b` and `m101-gates.log`). The evidence is the commit
message's own measurements, reproduced here as the commit states them:

- `cargo test -p epsilon-core` — green; the commit does not state a count.
- `cargo clippy -- -D warnings` — exit 0.
- `cargo fmt --all -- --check` — exit 0.

This slice's edits to `compact.rs`/`lib.rs` are the ones `audit/evidence/m101-gates.log` records
as an in-flight workstream during M101's gate re-run — its demo compiled a tree carrying them and
noticed `unused import: AssistantMessage`, a warning this commit's own clippy run is the one that
cleared. The M101 log is evidence about *that* tree, not about these final bytes, and is named
here only to explain the overlap.

Three tests were added: `a_summary_cut_off_by_the_output_limit_is_refused`,
`a_complete_summary_is_still_accepted`, `an_empty_summary_is_still_refused` (with an
`answer_with(stop_reason)` helper). None is claimed as a pre-fix RED — the pre-fix failure is
decidable by reading, and no red run was preserved. `audit/11` records the corresponding gap as
**G9**, closed in this commit against the fidelity audit's **D-5**.

## Limits

- **The commit message's "falls back to context-free pruning" overstates what the code does.**
  Read against `lib.rs`, a `Failed` outcome emits a `Notice` and either suppresses further
  compaction for the run or ends it with an `Error` naming the cause; the pruning is
  `compact_now`'s own first step on its *next* invocation, not a fallback this branch takes. The
  consequence that matters is intact and is what the fix is for — a truncated summary never
  replaces the transcript — but the summary sentence should not be quoted as a description of
  the control flow.
- **Nothing tests the consequence end to end.** The three tests drive `summary_text` directly.
  No test compacts a session through a provider that returns `MaxTokens` and asserts that the
  transcript survived and the notice appeared; that needs the faux provider wired into a
  loop-level compaction test.
- **`MaxTokens` is the only truncation-shaped stop reason handled.** A provider that reports a
  cut-off answer under some other reason still passes. The set is `StopReason`'s, and this fix
  reads one variant of it.
- **The empty-answer refusal still has no parity claim and no upstream precedent** — it is this
  harness's own rule, as the doc comment says. Nothing about this slice changes that; it only
  adds a second, upstream-backed refusal beside it.
- **The caller's wording is asserted by nothing.** The `why` string is chosen in `lib.rs` and no
  test pins it, so the two refusals could be quietly collapsed into one message by a later edit
  without a single test failing.
- **Deliberately not claimed:** D-5 is closed, but the audit's D-12 (compaction triggers before a
  turn where pi triggers after a run) and D-13 (a length-truncated *assistant response*, not a
  summary, gets no recovery) are separate findings and remain open.
- **Windows-only verification.** Nothing here is platform-specific; the Linux CI job has not run
  on these bytes.
