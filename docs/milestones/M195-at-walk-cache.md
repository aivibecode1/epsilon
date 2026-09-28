# M195 — the `@`-walk, and the fourth control that passed for the wrong reason

## What

`@`-completion re-walked the entire workspace **on every keystroke typed after an `@`**.
M110 had cached the ignore *rules* and left the traversal to run per character. This
caches the file list too, so the walk happens once per TTL instead of once per key.

Measured by mirroring `at_files::walk_inner` (a scratch script, not a test — see
*Method* below):

| Workspace | One walk | Share of a 33 ms frame @ 30 fps |
|---|---|---|
| this repository (847 files) | **4.2 ms** | 13% |
| `C:/Users/Hp/Desktop` | **6.7 ms** | 20% |
| `C:/Users/Hp` (large tree) | **9.2 ms** | 28% |

## What this is not

**It is not the freeze.** The fourth theory of the reported bug, and the fourth to be
measured and disproved as *the* cause:

| # | Theory | Measured | Verdict |
|---|---|---|---|
| 1 | a permission dialog silently eats keys | it renders `y/a/n/e`; `y` works | disproved (M193) |
| 2 | per-frame transcript deep clone | 144 µs → 276 ns, flat | disproved (D18, fixed anyway) |
| 3 | the console window gets resized | the TUI touches no console geometry; crossterm takes the VT path, not the buffer-swap path | disproved (M194) |
| 4 | `@` re-walks per keystroke | 4.2–9.2 ms | **real cost, not a freeze** |

`AT_ROW_BUDGET = 200` bounds the walk, which is why the number stays in single-digit
milliseconds. It is waste — a list rebuilt per keystroke that cannot have changed since
the previous keystroke — and waste is worth removing whatever its size. It is not offered
as an explanation of a total freeze, and the milestone says so three times because that
is exactly the inference a reader would otherwise make.

The honest position on the freeze is unchanged: **four theories measured, none
reproduced.** The next candidate is the event-stream path — dropped `Next` futures
against crossterm's one-shot wake-task protocol (`app/event.rs:127-138`).

## Why the cache expires with the rules

The rules and the list are one cache entry with one TTL. That is deliberate: a list
filtered by rules that have since changed is worse than a slightly stale list, and
sharing the TTL makes the second impossible by construction rather than by discipline.

The price is that a file created while the popup is open appears on the next `@`, not
instantly. That is the same bargain M110's rule cache already made, and the one the
branch cache pays.

## The control, and why it took four attempts

This test passed **three times against a control that re-walked on every keystroke**.
Each version failed to distinguish the broken build from the fixed one, in a different
way:

1. **Compared the cached list's contents before and after.** Identical either way — the
   same fragment produces the same list whether or not the traversal ran.
2. **Asserted the cache field is untouched.** A re-walking implementation never *writes*
   that field, so it is untouched in both cases. The wrong side of the change.
3. **Asserted the popup's rows, with a fixture that could not match the query.**
   `fuzzy_filter` (`fuzzy.rs:159-172`) splits the query on `/` and requires every token,
   so `created_while_typing.rs` against the query `src/app` was absent from the popup
   whether or not the walk ran. **Vacuous in both directions.**
4. **Renamed the fixture to `src/app_created.rs` so it matches the query.** The control
   then failed, for the right reason and with the evidence in the message:
   `shown=["src/app.rs", "src/app_created.rs"]` — the re-walking build offered the new
   file, the cached build did not.

The final assertion is on **what the user is shown**, not on internal state, and the
fixture is named so it can appear. An assertion is worth only what it can distinguish;
before believing a green test, ask what the broken version would print.

## Method

The measurement was a scratch script outside the repository, mirroring `walk_inner`'s
traversal, depth cap and budget. That was not the first choice: `write_file` silently
failed to land a Rust test file **twice** in this session, reporting success both times,
and `patch` then matched fuzzily and corrupted it. A throwaway measurement belongs in
scratch, not in the tree — and verify what is on disk before believing a compiler error
about a file you have just written.

## Verification

| | |
|---|---|
| Gate | **`m195`** — `audit/evidence/gate-m195.log` |
| Suite | **1,492 passed, 63 targets, 0 failed** (up one: this test) |
| Control | the `@` re-walk control **fails**, naming the file it wrongly offered |

## What nothing exercises

- **No test covers the TTL expiring.** The 5-second refresh path — the list being
  rebuilt after expiry — has no test, and it is the one path a stale cache could break.
- **No test covers a file created mid-popup becoming visible on the next `@`.** The
  bargain is documented; nothing asserts it.
- **The measurement is from three workspaces, not a slow one.** A tree that defeats the
  row budget (many directories, few files) would make the walk pay the traversal while
  collecting few rows. That is the shape that would actually hurt, and it is unmeasured.
