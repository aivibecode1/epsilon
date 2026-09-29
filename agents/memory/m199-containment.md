# M199: containment follows links — what landed and what it left

**Status:** closed and committed. GATE PASS `m199c`, 1,568 tests / 63 binaries.
Dev log: `docs/milestones/M199-symlink-containment.md`.
Negative controls: `audit/evidence/m199-negative-controls.md`.

## The rule now

`epsilon_core::paths` has two functions and the difference is the point:

- `lexically_normalize` — for a **planner key** (`path_key`, `search_key`). An
  identity must not change when a symlink is retargeted, or two calls onto one real
  file get scheduled as independent.
- `resolve_for_access` — for **enforcement**. Canonicalizes the deepest existing
  prefix, re-appends the rest, so `write` to a new file is still judgeable.

`Resolution { path, inside, real }` — `real` is the canonical path, and
`escaped_to()` is derived from it. `display_canonical()` strips the Windows `\\?\`
verbatim prefix so a refusal reads like a sentence.

Callers: `read`/`write`/`edit` via `resolve_or_refuse`, `find`, `grep`, `glob`,
`ls`, `references::resolve_one`, `exec::gate_subject`.

## The lesson worth more than the fix

The slice would have shipped a **bypass**: containment was answering on the real path
while `is_sensitive` still read the spelling, so `@vcs/` → `.git` passed one check
and should have failed the other. The control printed the body `config` — not
refused.

**Two answers to "where does this path point" is where the bug lives.** The moment a
module has a lexical helper and a resolving helper, every caller has to be asked
which one it means. A field named for the *error message* (`escaped_to`) rather than
for the *fact* (`real`) is how the quiet case gets missed.

## What the battery caught in my own new code

`m199` failed: `no_production_spawn_leaves_stdin_unset` flagged the `mklink` spawn in
`test_support::dir_link`. The M197 defect class, in a module added *after* the guard
was written. That guard is the single most valuable thing in the CLI crate.

`m199b` failed: my own assertion spelled the expected `real` path with `join` while
`canonicalize` returns the `\\?\` verbatim form. `inside` was never affected — it
compares two canonicalized paths. **Third instance in two milestones of the test
being wrong before the code was.** Write the expectation through the same function
the code uses.

## Open items this milestone created or exposed

1. **A word is mutilated at 60 columns** — `and then` → `and the`. See
   `frame-reading-pass.md` F1. Not the wrapper (`wrap.rs:25` only hard-splits a
   token longer than the line); the wrap width and the drawn width disagree by the
   gutter. **Probe before fixing.**
2. **The header drops its `↑` at 60 columns**, leaving a bare `0` where a labelled
   pair was. A partial badge is worse than a missing one.
3. `?` / `CH n/a` still have no on-screen legend.
4. **`ReadTool` applies no sensitive-name check and that is not written down
   anywhere.** `@` attaches silently into a prompt; `read` returns visibly in the
   transcript. Probably right — but an undocumented judgement is an undocumented
   judgement. Ledger row, not a change.
5. A **directory literally named `.env`** evades `is_sensitive` (the component list
   is `.git`/`.ssh`/`.aws`/`.gnupg`). Recorded, not chased.
6. The **file-symlink** shape of the bypass is unproven on Windows (no privilege).
   The junction shape covers the same resolution code and ran.

## Machine note added this round

`pwsh -File scripts/gate.ps1 <label>` needs a **tool timeout above 600 s** — the
test gate alone ran 651 s, so `timeout=600` kills the battery mid-leg and the log
records no verdict. `timeout=2400` works. This is a harness budget, not the
script's; nothing in `gate.ps1` is wrong about it.

`scripts/gate-until-pass.ps1` now wraps it: fresh label per attempt, and it stops on
the **second identical failure fingerprint** rather than retrying a deterministic
failure three times. It does not modify `gate.ps1`, because that script is the
evidence of record.
