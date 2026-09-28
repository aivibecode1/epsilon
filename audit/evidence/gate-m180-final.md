# Gate battery `m180-final` — PASS

**Date: 2026-09-28. Commit: `1790944`. Log: [`gate-m180-final.log`](gate-m180-final.log).**

`pwsh -File scripts/gate.ps1 m180-final`, one build job, `CARGO_TARGET_DIR=D:/eps-target`.
This is the battery for the bytes that are actually shipped.

## Result

```
===== fmt --check =====   [exit=0   2s]
===== clippy =====        [exit=0  34s]
===== test =====          [exit=0 520s]
===== demo =====          [exit=0  36s]
===== real-check =====    [exit=0  14s]
===== GATE PASS =====
```

**0 failures.** The log's own total is 1316; see [`gate-m180-1.md`](gate-m180-1.md) for why
that is 1254 real tests plus a 62-test double count (`tests/builtin_tools.rs` runs twice in
the battery) — so 1253 carried over from the previous run, plus **one** new test, which is
`jumping_the_tree_leaf_re_projects_the_visible_conversation` from `7489c00`.

## What changed between this battery and `m180-1`

| Commit | Change | Why it needed a battery |
|---|---|---|
| `7489c00` | `/tree` now re-projects the conversation | Touches `epsilon-core` (a new public method) and the TUI's key handler — the two crates whose tests most needed to run together |
| `3a8703b` | `AGENTS.md`, adopted inherited M179 text | Documentation only, but it was uncommitted inherited work that had to be reviewed and claimed |
| `57812ca` | `epsilon-llm`'s `required-features` | Changes which test targets exist under which features — exactly the kind of thing a battery catches and a single-crate run does not |

Everything from `m180-1` — the five defect fixes, `PacedProvider`, the console probe — is
included, since the log is a fresh full run rather than an incremental one.

## The order this ran in, and why it matters

`m180-1` passed. Then a per-crate sweep found `epsilon-llm` did not compile under
`--no-default-features` — invisible to the battery, because `real` has been a default
feature since M63 and the battery builds exactly one configuration. That is fixed, and
**this battery still says nothing about that configuration**; what it establishes is that
the default build is unbroken after the fix, and that a feature-gating change did not
disturb the ordinary path.

The remaining honest gap is unchanged: a green battery is a statement about the
configurations it builds, and this one builds one.
