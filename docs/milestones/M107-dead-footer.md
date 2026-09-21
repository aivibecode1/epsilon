# M107 — deleting the second footer that a redesign left behind

## What

`crates/epsilon-tui/src/footer.rs` was a complete, older footer implementation:
`format_tokens`, `format_cwd`, `ch_rate`, `FooterPart`, `Tone`, `context_parts`,
`stats_parts`, `FooterStats` and eight tests. When the `ui/` module tree replaced the
presentation layer, everything in it kept compiling, kept being `pub`, and stopped being
called. Exactly one function still had a caller: `git_branch`, used by
`App::cached_branch`.

Nothing caught it, and the reason is worth more than the deletion: the items are `pub`
inside a `pub mod` of a library crate, so `dead_code` — and therefore
`clippy -D warnings`, a gate this repo runs on every change — is structurally blind to
them. A public surface is a promise that something outside might call it; the compiler
cannot know that nothing does.

The cost of leaving it was not aesthetic:

- **Two `format_tokens` with different rounding rules.** The dead one printed
  `12_345` as `12k`; the live `ui/format.rs` prints `9_950` as `10k`. A reader looking
  for how tokens are formatted can find the wrong one, and a test can pass against it.
- **A second copy of the broken cache formula.** `ch_rate` divided "accumulated cache
  reads" by the latest prompt — G16 in miniature, sitting in the same crate. Fixing the
  live gauge while the dead copy stayed would have left the next reader with two
  contradictory answers and no way to tell which one runs.

## Key decisions

- **Delete, do not deprecate.** A `#[deprecated]` shim would preserve the confusion and
  add a marker to maintain. The module had one live function and no external callers
  anywhere in the workspace (checked with a repo-wide grep, not assumed).
- **`git::branch`, not `footer::git_branch`.** The surviving function moved to its own
  tiny module named for what it does. Beyond the footer, "which branch is this workspace
  on" is a git question; the module doc says why it reads `.git/HEAD` directly instead of
  shelling out (the footer draws ~30 times a second, and the caller adds a two-second
  TTL).
- **The move was a chance to fix the function's worst case.** The old fallback truncated
  *anything* in `HEAD` to seven characters, so a linked worktree — which keeps a file
  holding `gitdir: ../elsewhere` — displayed the literal text `gitdir:` as the branch
  name. A detached HEAD is now accepted only if the line is a full 40-character hex
  object name; anything else is `None`. Resolving the worktree pointer is deliberately
  not attempted: it means following a path the caller did not ask about, and "no branch"
  is the honest answer for a workspace this function cannot resolve.
- **The deleted tests' subjects went with them.** Seven of the eight tested dead
  functions. The eighth, `git_branch_parses_head_and_detached`, is replaced by four that
  cover the same ground plus the two cases the old code got wrong.

## Gotchas

- **A green clippy gate is not evidence that everything is used.** `-D warnings` plus
  `unsafe_code = "forbid"` is a strong gate; `pub` items in a library crate walk straight
  through it. When a redesign replaces a module, the replacement is what gets the test
  coverage — the old module's tests keep passing and keep the corpse warm.
- **The dead code was not inert.** Independent implementations of the same metric drift
  apart, and the drift is invisible until someone reads the wrong one.

## Verification

Battery `m106a` (shared with M106 and M108): all five gates exit 0, including
`clippy --workspace --all-targets -- -D warnings`, which is the gate that had been blind
to this module.

Reconciliation for the whole round, because deleting tests must never be quiet:

| Scope | Before | After | Why |
|---|---|---|---|
| `epsilon-tui` | 220 | 217 | −8 tests deleted with the dead footer, +4 for `git::branch`, +1 for the cache numerator (M106) |
| `epsilon-session` | 29 integration | 30 | +1 for the unknown final line (M108); one torn-tail test rewritten in place |
| workspace | 1,023 passed / 59 binaries (M105) | **1,021 passed / 0 failed / 1 ignored / 59 binaries** | −2, which is exactly the arithmetic above |

The workspace figure is not an estimate: `-8 + 4 + 1 + 1 = -2`, and the battery reports
two fewer than M105's.

Global `epsilon.exe`: **reinstall not required** — the deletion removes unreachable code;
no user-visible behaviour changes. The round's reinstall decision is in M108.

## Limits

- **`pub` remains a hole in the dead-code gate.** This slice removed one instance; it did
  not close the class. A `cargo-public-api`-style check or per-module `pub(crate)`
  discipline would, and neither is in place. Anything else in this workspace that was
  superseded by a redesign and kept its `pub` is still invisible.
- **`ui/footer.rs` was not audited for further overlap.** The two footers no longer
  duplicate a *formula* — the dead one is gone — but whether every remaining helper in
  `ui/` has a caller was not checked.
