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

**0 failures.** The log reports 1316 results across 63 test targets: 51 test binaries plus
12 `Doc-tests` targets, each of the latter reporting 0 passed. No binary runs twice.

**Do not subtract a per-crate sweep from this.** An earlier draft of
[`gate-m180-1.md`](gate-m180-1.md) claimed 1315 double-counted `builtin_tools`' 62 tests and
so "the real number is 1253". Parsing the log — attributing each `test result` to the
`Running`/`Doc-tests` header above it — refutes it: `builtin_tools` appears **once**, and the
extra 12 lines are doc-test zeros. The two measurements were taken at different times against
different target sets, so their difference means nothing. The correction is recorded in
full there, because a wrong number that looks authoritative is worse than no number.

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
