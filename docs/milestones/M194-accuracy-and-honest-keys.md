# M194 — eleven defects, and the two questions nobody had asked

## What

Eleven confirmed defects and one missing capability, across the RPC surface, the TUI and
the three numbers in the header. Every one was fixed **and** pinned by a test written to
fail without the fix.

Two of them were found by asking a question the previous audits had not asked:

- **What does the user mean by "shrinks"?** The reported TUI complaint — *"it shrinks and
  freezes as soon as I start typing and shows no input or output"* — had been chased twice
  and cleared twice, because both audits tested **freeze** and **no input/output** and
  both passed. Nobody tested the **third** word.
- **Is the number the code reads the number I reasoned about?** A context-accuracy fix
  moved three fixtures. All three had budgets calibrated against an estimate that
  understated the request by 1,633 tokens.

## The two that mattered most

### D32 — a pi client could not send pi's own field name

`set_model` carried `model_id` with no alias, because the `Op` enum is
`#[serde(tag = "op", rename_all = "snake_case")]`. pi documents
`{"op":"set_model","modelId":"…"}` (`rpc-types.ts:33`). So **every pi client sending the
documented frame was refused**, with an error reading *"set_model needs a model"* — which
blames the model rather than the key, so the user goes looking in the wrong place.

`fork`, three variants below it, already had `rename = "entryId", alias = "entry_id"`.
`set_model` was the lone miss.

**How it was found:** I was writing the D31 test and sent the frame pi documents. It came
back refused. That is the fourth time this project has been bitten by *a test that built
its input from the Rust identifier* — and the third time (D26, D29, D32) the test was
structurally incapable of seeing the wire.

### G1 — `/hotkeys` printed a binding that did nothing

`app.clear` was in the defaults table, `/hotkeys` printed the effective table verbatim,
and **nothing in the crate read `app.clear`**. Meanwhile `app.exit` matched the same chord
first, so Ctrl+C quit the process **with the draft still in the buffer**.

`grep -rn 'app.clear' crates/` returns exactly one hit: the line that declares it.

This is the "code claims done, code does not" class in its purest form, and the fix is
small: a real `clear_editor()`, matched *before* `app.exit`, with `app.exit` moved to
ctrl+q alone so no chord names two actions. A test asserts that no chord in the effective
table is bound to two ids.

## The metrics: three numbers, three different lies

| # | What it was | What it is now |
|---|---|---|
| **K1** | a provider that reports no cache fields printed `CH 0%` — a cache that is present and failing | `CH n/a`. "Cannot measure" and "measured zero" are different sentences |
| **C2** | the no-anchor context estimate ignored the system prompt and tool schemas — 1,633 tokens of request the gauge never saw | counted, on the fallback arm only |
| **R1** | an estimate and a measurement were printed in the same typeface | `ctx 20%*/200k` marks the estimate |

K1 is a flag on the wire type (`Usage::cache_reported`), set only by a decoder that saw a
cache **key** in the response body. It is per-**call**, not per-provider, because a gateway
can route one model id to two backends that report differently and only the reply knows
which one answered.

C2 is the fix with consequences, and the consequences are the interesting part.

## The consequence: making a number truthful moves calibrated thresholds

`compaction_due` reads `estimated_context_tokens`. When C2 started counting the system
prompt, the pressure trigger **correctly fired earlier** — and three fixtures broke.

The tempting fix in each case is to move the number to match. That would have been
**asserting the regression**: a *less* accurate estimate. What each fixture actually
needed was the **relationship** its comment claimed, restored:

- `a_compaction_records_its_bill_on_its_own_entry` asserted `input == 200` — a
  fixture-dependent constant for a figure that is `100 × (number of summariser calls)`.
  Now asserts the property F23 is about: a billed compaction has `input > 0`.
- `a_compaction_keeps_the_users_own_words` and
  `mid_run_compaction_invalidates_the_replacement_context` both stated *"the bare prompt
  does not trip the budget"*. M194 made a bare prompt measure 1,674, past a 1,200 budget.
  Both now sit above the prompt and below the prompt-plus-result, which is the property.

**I proved the causation rather than assuming it.** A `git worktree` at HEAD with its own
`CARGO_TARGET_DIR` ran the first test and passed at 200, so the change was mine. Setting
`let fixed = 0` for one run reproduced the second's pass, in eight seconds. Both took
minutes; guessing at thresholds took an hour, and all six attempts were wrong in the same
direction — each assumed the estimate had *shrank* when it had grown.

## G2 — `/fork` was a file copy

The TUI's `/fork` and `/clone` were `std::fs::copy`: the whole file, every abandoned
branch included, under the **same session id** in the header, so the two files claimed one
identity.

`SessionStore::fork_branch_at` already existed, writes a fresh header, copies only the
active branch, and is what pi's `forkFrom` does — and it was wired **only** from the RPC
surface. So the transport a human uses had the worse implementation, next to the correct
one. Both TUI commands now call it, and a test asserts the fork carries its own id.

This is `branch_with_summary`'s defect class for the fourth time: **a faithful port with
passing tests and no production caller on the path anyone actually takes.**

## G3 — jumping to a branch scrolled you away from it

After `/tree` navigation, the handler pushed one notice per branch entry — "so the user can
see what they jumped into without re-opening the picker". Jumping to entry 3 of a
400-entry session therefore pushed 400 rows into the transcript. They are *notices*, which
sit at the bottom, so the reader was pushed off the message they had just jumped to. The
transcript above is now the branch; re-printing it told the user nothing.

## G9 and G10 — two menu/keys that lied

- **G9** — a palette entry carried `"\u{1}expand"`, which `parse_command` cannot read (it
  requires a leading `/`). Choosing it closed the palette, cleared the composer, and ran
  nothing, with no notice. Its key hint was also wrong: a bare `x`, the very binding that
  was *removed* so the letter stays typable. Now a real `/expand` command, with the key and
  the menu calling one shared method so they cannot drift.
- **G10** — `handle_key` ended in `_ => false`. A silently swallowed keystroke is
  indistinguishable from a frozen program, which is the exact shape of the report this
  project received. Now it says `<key> is not bound here — /hotkeys lists what is`.

  This is the invariant the whole investigation produced: **"nothing happened" must never
  be a reachable UI state.**

## The remaining gaps (measured, not fixed — `audit/evidence/port-fidelity-study-2026-09-28.md`)

A fresh study against `pi-0.85.1` surveyed **84 TUI/loop capabilities: 38 DONE, 22
PARTIAL, 24 MISSING**. The missing ones concentrate in one place: everything pi renders
through a `Component` Epsilon has no equivalent of — a foldable session tree, a settings
selector, an editor with jump-to-character. Those are days of work each and are the next
milestones, in the order the study ranks them.

Also still open, and honestly so: `branch_with_summary` has no production caller (fourth
confirmation); there is no thinking-block visibility toggle; `PageUp` scrolls a fixed ten
rows rather than one viewport; markdown silently drops tables, task lists and links because
the match ends in `_ => {}`; and there is no syntax highlighting.

## Verification

| | |
|---|---|
| Gate | **`m194` — GATE PASS**, all five gates `exit=0` on a frozen tree |
| Suite | **1,491 passed, 63 targets, 0 failed** |
| Log | `audit/evidence/gate-m194.log` (124 KB, copied byte-identical out of `target/`) |
| Terminal (ConPTY) | 8/8, `--test-threads=1` |
| Installed binary | refreshed; the TUI-visible changes (`CH n/a`, `ctx …*`, Ctrl+C) need it |

**One honest note on the count:** `the_tui_works_when_started_from_powershell` failed once
in a parallel `cargo test --workspace` and passed in all eight when run serialised. Each
test opens its own console, and parallel consoles contend — which `AGENTS.md` already
records. The battery runs them serially; the failure was a harness artefact, not a defect,
and it is named here rather than quietly dropped.

## What nothing exercises

- **No test drives `/fork` or `/clone` end-to-end through the TUI and re-opens the result.**
  The new test checks the fork's header id, not that the forked session resumes.
- **No test asserts the header renders `CH n/a` end-to-end.** The unit test builds
  `Metrics` directly; nothing proves a real faux session (which reports no cache fields)
  shows `n/a` rather than `0%`.
- **The real-console probe has not been re-run since these changes.** `CH n/a` and the
  `*` mark are asserted in unit tests against a `TestBackend`, which cannot see whether the
  bytes reached a console. Run `scripts/console-probe.py --late --enter` before trusting
  the header on a real terminal.
- **`context_estimated` has no test that a live session's mark flips** after a compaction
  clears the anchor. The accessor is tested directly; the mirror is not.
