# M118 — bounds that were missing, and the one that was silent

G21's three items, all the same rule: **a tool must not have a bound it does not announce, and
must not announce a bound it does not have.**

## What

1. **`find` promised "1000 results or 64KB" and enforced neither byte.** M117 made the count
   half honest; the byte half was a false claim in a description *the model reads*. The cap is
   now real: `util::TOOL_OUTPUT_MAX_BYTES` = 50 KiB — pi's own `DEFAULT_MAX_BYTES`
   (`truncate.ts:12`) — applied through pi's `truncateHead` shape (`find.ts:277`), and the
   description now says 50KB, which is both enforced and pi's number.
2. **`glob` stopped at the walker's `WALK_MAX_ENTRIES` budget silently.** It now says so.
3. **`ls` had no per-entry byte cap** where pi has one; it now has it too.

```
find, bytes cut : [50.0KB output limit reached: 84 more file(s) match and are not shown. Use a narrower pattern to see the rest]
find, count cut : [2 results limit reached: 3 more file(s) match and are not shown. Use limit=4 for more, or refine pattern]
glob            : [5000 files walked, so this list may be incomplete: the walk is a safety bound, not a match
                   limit, and stopping it hid whatever it never visited. Search a subdirectory to see the rest]
ls, bytes cut   : [50.0KB output limit reached: 297 of the 400 entries in this directory are shown, 103 more are not.
                   List a subdirectory to see them]
```

## Key decisions

- **Implement the bound rather than delete the claim.** The byte cap is a real protection — a
  listing that reaches the model at 150 KB costs context for no benefit — and this project's
  rule is that a description must not promise what the tool does not do. Deleting the sentence
  would have made the description true and the tool worse.
- **Each notice names only the bound that actually fired.** When bytes cut a listing inside the
  entry cap, the "Use limit=2N" advice is *suppressed*, because sending a reader to raise a
  limit that is not what stopped them is worse than saying nothing. This is the decision the
  subagent flagged for review and it is the right one.
- **`find`'s `[N file(s)]` now counts rows actually shown**, not rows matched — a total that
  includes results the reader cannot see is the same lie one line up.
- **Silence when nothing was withheld**, as in M117: a notice that fires when nothing is hidden
  trains the reader to ignore it.
- **Real crossings, not lowered constants.** The byte fixtures are 400 entries of ~165 bytes
  (≈66 KB of listing against a 50 KB cap, well under the 1000/500 entry caps, so *only* the
  byte bound can fire), and the glob fixture really creates 5000 files. A compile-time
  `assert!` documents the crossing so a future edit cannot quietly make the test vacuous.

## Gotchas

- **A restored file keeps the backup's mtime, so cargo can run the stale binary.** The
  subagent's first post-restore run reported a *false* failure from the still-disabled control
  build and had to touch the files to force a recompile. This is the second time this trap has
  cost a round here; it belongs in the verification rules, not just in a milestone.
- **pi has no `glob` tool at all** (`core/tools/` holds bash, edit, find, grep, ls, read,
  write), so item 2 had no upstream announcement to copy and matched M117's voice instead.

## Verification

Battery `m118b`: **all five gates exit 0, 1,066 passed / 0 failed / 1 ignored across 59
binaries** — nine more than M117's 1,057 (three unit, six integration). `epsilon-tools` 100
unit + 55 integration. `m118a` was started and lost mid-run when the session restarted; it is
not evidence and is not counted.

**Negative controls, one per item** (revert in place → the named test fails on the notice
assertion while the under-bound tests stay green → restore, hash-verified): the `find` byte
cap, the `glob` walk notice, and the `ls` byte cap. Each failure landed on the exact assertion
that names the missing announcement.

## Limits

- **`glob`'s byte cap landed after this slice, in the two-item follow-up round** (the other item
  was M119's `thinkingSignature`). It now head-truncates its match list at
  `util::TOOL_OUTPUT_MAX_BYTES` through the same `util::truncate_head_bytes`, prints `find`'s
  sentence, and its `[N file(s)]` counts the rows the reader can see. The one thing this slice
  had not decided is precedence between `glob`'s two bounds, and the follow-up decided it the
  other way round from `find`: when the **walk** bound fired, the walk notice is the one printed
  and the byte notice is suppressed, because with the walk stopped early "N more file(s) match —
  use a narrower pattern to see the rest" is false (the rest was never visited). The byte cap is
  still *applied* in that case, so the count is honest and the walk notice does not say how many
  rows the cap hid — the same accepted limit `ls` has below.
- **`ls` names only one bound.** When bytes cut inside the entry cap the entry notice is
  suppressed, so a reader cannot tell from the notice alone that the directory also holds more
  entries than `limit`.
- `descriptions/*.txt` say "50KB" in prose while the runtime notice prints `50.0KB` (pi's
  `formatSize`). Not unified; the prose matches `bash.txt`'s existing style.
- M117's `Limits` bullets about the missing byte cap are now stale and are superseded by this
  file and by `audit/11`'s G21 row.
