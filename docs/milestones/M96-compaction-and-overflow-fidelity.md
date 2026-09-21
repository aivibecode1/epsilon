# M96 — three port gaps the fidelity audit proved (overflow, summary template, dead allow)

## What

`audit/evidence/port-fidelity-audit.md` read pi 0.85.1's loop, prompt, skills and compaction
against the Rust counterparts and returned twenty-two findings. Three of the cheap ones land
here (D-2, D-3, and the dead-code warning the same pass turned up). Each is a few lines. None
had a test, which is exactly why each survived this long: the code was plausible, it ran, and
nothing ever asked it a question it could fail.

**Context overflow was never detected for Anthropic** (D-2). `looks_like_context_overflow`
matched the *spaced* spelling `"request too large"`. The message it classifies is the provider
body verbatim, and Anthropic's is `request_too_large` — which contains no `request too large`.
So the compaction-and-retry branch that exists for exactly this situation never fired for that
provider, and a real overflow ended the run with a raw HTTP error where pi compacts once and
retries. `prompt is too long` was missing as well. The needle list is now one `const NEEDLES:
[&str; 12]` covering the shapes the providers actually send — the spaced and underscored forms
of each, `context_length`/`context_window`, `maximum context`, `too many tokens`, `input is too
long`, `exceeds the maximum`, `reduce the length`.

**The compaction summary template was never sent** (D-3). `SUMMARY_SYSTEM` said "Output ONLY
the template" and the request carried no template, so every compaction produced a summary of
whatever shape the model invented — and a summary whose shape changes is one a reader has to
re-learn each time. `SUMMARY_SECTIONS` (`[(&str, &str); 6]`: Goal, Constraints & Preferences,
Progress, Key Decisions, Next Steps, Critical Context) is now both the constant and the
document: `summary_system_prompt()` renders precisely those six headings in that order, and
`summary_context` sends the rendered string. Two tests pin it, one from each side — the prompt
names every section in order, and each section renders from the constant in the pinned format.

**A dead-code warning was silenced on purpose.** `impl App::theme()` tripped `dead_code`
because the draw path reads the field directly. It is documented and `#[allow(dead_code,
reason = …)]`-ed rather than deleted, because the screenshots harness and any future
integration test need the binary's own palette; deleting the accessor would have made that a
re-add. The same commit removed two unused `Line` imports and collapsed `selected_muted`'s two
identical branches into one expression with the reason written down.

## Key decisions

- **A false positive is as expensive as a miss, so the classifier is tested in both
  directions.** A wrong `true` costs a pointless compaction that throws away context; a wrong
  `false` fails a run that could have recovered. The test therefore carries eleven real bodies
  that must be recognised *and* five unrelated failures (401, rate limit, connection reset,
  truncated tool arguments, empty response) that must not be. The audit's "smallest fix" also
  suggested adding pi's three-pattern *non*-overflow exclusion list (`overflow.ts:74-78`); that
  is **not** in this commit, and is not claimed.
- **The template is generated from the section constant, not written beside it.** Two things
  that must agree and are written twice drift; the audit's own "smallest fix" was to inline the
  template, and inlining is what the first version of this code did — with the instruction to
  follow it and no it to follow. `the_section_list_is_the_one_the_prompt_renders` asserts the
  rendered line for every entry, so the constant *is* the document.
- **`Critical Context` stays last.** The order is the order a reader reloads context in, and
  the catch-all goes last on purpose: put it earlier and the model dumps everything there and
  leaves the specific sections thin.
- **The prompt gained rules the un-templated one had no place for**: quote the user's
  instructions rather than paraphrasing, prefer specific facts to narrative, say what is
  UNVERIFIED as clearly as what is done, never invent progress the transcript does not show.
  These describe the same standard the rest of the harness already holds itself to.
- **The dead accessor is kept and justified, not deleted to make a warning go away.** A warning
  silenced without a reason is indistinguishable from one hidden; the `reason` string is the
  difference.

## Gotchas

- **`request_too_large` contains no `request too large`.** That is the whole bug, and it is the
  kind that survives review: the needle list *looks* complete, and the same body spelled with
  spaces would have matched. The fix is not a cleverer matcher — it is a list written from the
  bodies the providers send, with a test per real body.
- **Matching is case-insensitive and has to be**: `classification_ignores_case` pins
  `REQUEST_TOO_LARGE` and `Prompt Is Too Long`. A provider that shouts is the same provider.
- **The old list carried spellings the new one reaches through a broader needle.** The pre-fix
  classifier had six lowercase needles; `context_window_exceeded` and `context_length_exceeded`
  are now covered by `context_window` / `context_length`, and both survive in the test as inputs
  so the coverage is pinned rather than assumed.
- **The TUI edits in this commit are not a refactor.** `selected_muted` had `if mode ==
  ColorMode::None { reversed } else { reversed }` — two branches, byte-identical bodies, and
  the doc comment above it already explained that both halves of one highlighted row must read
  as a single band. Removing the branch is the change; the comment now sits where the reader
  needs it.
- **The dead-code allow is a claim about the future, and it is written as one.** The reason
  names the screenshot harness and integration tests rather than asserting the method is used
  today.

## Verification

No battery log or preserved gate log for this slice exists: `audit/evidence/` holds
`gate-m93b`, `gate-m94c`, `gate-m95b` and `m101-gates.log`, and nothing for M96. The evidence
is the commit message's own measurements, reproduced here as the commit states them:

- `cargo test -p epsilon-core` — green (all targets); the commit does not state a count.
- `cargo test -p epsilon-tui` — **213 passed, 0 failed**.
- `cargo clippy -p epsilon-core -p epsilon-tui --all-targets -- -D warnings` — exit 0.

The commit also records that the workspace run which unblocked M95 confirmed the suite green
end to end on a fresh profile, and carries no count of its own. The nearest preserved numbers
are later and belong to another slice: `agents/memory/port-revalidation.md` records
`audit/evidence/g7g8-workspace-tests.log` at **975 passed / 0 failed / 1 ignored across 57
binaries**, and `audit/evidence/m101-gates.log` — a log this milestone does not own, run hours
later — reports the same figure. Neither is evidence for these bytes; both are named only so
the reader can see which number belongs to which slice.

Five new tests: `every_spelling_the_providers_use_is_recognised`,
`unrelated_failures_are_not_mistaken_for_overflow` and `classification_ignores_case` in
`lib.rs`; `the_summarizer_is_sent_the_template_it_is_told_to_follow` and
`the_section_list_is_the_one_the_prompt_renders` in `compact.rs`.

## Limits

- **No frozen-candidate battery, and no pre-fix RED, for this slice.** M95 was in its own battery
  rounds at the time and this commit names a workspace test run that unblocked it, so the numbers
  above are focused runs on a working tree several workstreams were touching, not a battery
  against frozen bytes; the new tests were not shown failing against the pre-fix code either. For
  both defects the pre-fix failure is decidable by reading — `request too large` cannot match
  `request_too_large`, and the pre-fix prompt contained none of the six section names — but
  decidable by reading is not the same evidence as a red run, and it is recorded here as the
  weaker thing it is.
- **The three-pattern non-overflow exclusion list from pi is not implemented.** The audit's D-2
  suggested it beside the missing needles; it is a separate behaviour and this commit did not
  take it. A body that contains an overflow phrasing *and* one of pi's exclusions could still be
  classified as an overflow here.
- **The needles are still a substring list.** It is broader and provider-derived now, but it is
  not pi's 27-pattern set, and a provider that invents a new spelling will be missed until
  someone adds it. The test's value is that adding one is a line, not a redesign.
- **The six sections are the harness's, not pi's document.** pi's `## Progress` carries
  Done/In Progress/Blocked sub-structure and there is an `UPDATE_SUMMARIZATION_PROMPT` variant
  for iterative updates that selects when amending (`compaction.ts:500-539,678`); neither is
  here. The sections match pi's names, not its full template, and the audit's D-3 stays
  partially open on that point.
- **Nothing pins the summary's *shape* end to end.** The tests pin that the prompt carries the
  template; no test drives a compaction through a provider and asserts the summary that comes
  back has the six headings. That needs the faux provider in a loop-level test.
- **Deliberately not claimed here, and named in the commit as still open from the same audit:**
  the contract tells the model to load a skill's body through a `skill` tool that does not exist
  in the registry (D-1, closed in M97); `disable-model-invocation` is parsed but never filtered
  (D-4, closed in M97's second commit); several tools claim `.gitignore` respect they do not
  implement (D-9, closed in M99).
- **Windows-only.** The TUI edits touch colour modes, not platform rendering, and nothing here
  has run on Linux.
