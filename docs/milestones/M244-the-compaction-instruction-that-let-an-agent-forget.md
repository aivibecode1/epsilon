# M244 — the compaction instruction that let an agent forget what it was told

One sentence of prompt, and it was the most valuable missing rule in the harness.

## What was wrong

When a session compacts for the *second* time, Epsilon asked the summarizer to:

> *"Update this summary with anything important from the NEW conversation below. Keep every
> section."*

That reads as a neutral instruction and is not one. It frames the prior summary as something
to be **updated from the new conversation** — which means anything the new conversation does
not mention is fair game to drop.

And the newest window of a long session is a **local** conversation. The last twenty messages
are about the bug in front of the model, not about the constraint the user set an hour ago:
*don't change the public API*, *keep this under 200 lines*, *the fixtures must not touch the
network*. None of those appear in the local window. A summarizer following the instruction
faithfully drops them — and the next agent confidently violates an instruction the user is
still relying on, with nothing in the transcript to correct it.

This is the failure the Hermes reference calls the common one, and it is silent: no error, no
gap in the transcript, just an agent that has forgotten.

## What replaced it

`AMEND_INSTRUCTIONS`, two rules:

1. **On a conflict the conversation wins** — state the corrected fact and drop the old claim,
   rather than keeping both and leaving the model to choose.
2. **Carry forward what the new window does not mention** — "EVEN WHEN the conversation does
   not mention them… Drop something only when it is finished and no longer needed, never
   because this window happens not to mention it."

The capitals are not decoration. A rule stated flatly in a prompt that also asks the model to
be concise competes with the concision instruction, and the one in capitals does not. The
test asserts the capitals, which is a slightly odd thing to assert and is here precisely
because the alternative — the rule quietly softened into a suggestion — is what happened
here.

It is also a `const`, so it can be read on its own. The rule's *reason* is in the doc comment
and partly in the prompt, because this prompt gets summarised like everything else.

## And the first compaction must not carry anything

A first summary has no prior. Telling it to carry something forward is a request it cannot
fill, and a model handed an unsatisfiable instruction will improvise — usually by inventing
a prior. `a_first_compaction_carries_nothing_and_says_so` pins both halves: no
`<prior_summary>` block, and no amend instructions at all.

## Why this is a harness change, not an IDE one

It sits in `epsilon-core`, and both frontends get it. A session that compacts in the terminal
and the same session continued in the desktop window keeps the same rule, which is the point of
the composition work in M234.

## Verification

**Battery `m244` — GATE PASS on attempt 1, 1,785 tests across 74 binaries, 0 failed**
(`audit/evidence/gate-m244.log`). Up from 1,783: two compaction tests.

## What nothing exercises

- **That it works.** Nothing here measures whether a summarizer *keeps* the constraint; the
  test checks that the instruction is present, which is the whole of what a unit test can
  honestly claim about a prompt. The evidence that matters is a long session that compacts
  twice and a reviewer reading the second summary for something only the first window knew.
- **The non-amend template.** `summary_system_prompt`'s rules were read and left alone; the
  gap was only in the amend path, and widening it without a reason would be change for
  change's sake.
- **Other compaction inputs.** The file ledger and the user-instruction block have their own
  tests and were not touched.