# M110 — the `@` popup went blank, and the search behind it read everything

**This is the fix for a regression M109 shipped, found in a real console rather than by a
test.** It is recorded as its own milestone because the M109 commit is already in history
and the story is worth keeping: the probe that verified M109 is the thing that caught it.

## What happened

M109 made `@` completion share the tools' ignore answer by loading an
`IgnoreSet` — and it loaded one **per keystroke**, because that is how often the popup is
re-derived. `IgnoreSet::load` searches the workspace for `.gitignore` files, and it did not
prune that search with the rules it was finding. In this repository it therefore walked
`/references/`, four checkouts of other people's source that `.gitignore` excludes and the
search happily descended into: 404 MB and tens of thousands of directories, per character
typed.

The symptom in a real conhost window was total input loss:

| typed | composer | popup rows |
|---|---|---|
| `@` | `• @` | 7 |
| `@a` | `• @a` | 7 |
| `@C` | `• @C` | 7 |
| `@c` | `• @c` | 7 |
| `@argo` | `• ask anything` | 0 |
| `@Cargo` | `• ask anything` | 0 |

One or two characters got a frame out in time; five or six did not, and the composer showed
the *placeholder* — the app had not drawn since before the keys arrived. A later run at a
settled console, twelve seconds after launch, still lost all six. The first hypothesis was
timing (the owner's own words were "the moment I launch it and start typing"), and it was
wrong: four different waits (3, 5, 8, 12 seconds) behaved identically, while one- and
two-character queries always worked. **The variable was how much work the keystrokes
queued, not when they arrived.**

## The two fixes

**The cache the TUI owed.** `App::at_ignore` holds the loaded `IgnoreSet` with a five-second
TTL, the same shape and the same reason as the branch cache next to it. The first `@` in a
burst pays for the search once; every keystroke after it reads the cache. The TTL is the
price: an edited `.gitignore` goes unnoticed for up to five seconds, which is a fair trade
for a popup that cannot stall.

**The pruning `IgnoreSet::load` owed.** A directory the rules parsed *so far* already ignore
is no longer searched. This is git's own rule rather than an optimization: git never
descends into an ignored directory, so a `.gitignore` inside one can only speak about paths
git would never look at. With the pruning, `/references/` is one `is_ignored` call instead
of a full traversal, and the bound that matters stops being "how big is the disk" and
becomes "how big is the repository".

## Key decisions

- **Cache in the frontend, prune in the matcher — because they fix different things.** The
  cache stops a keystroke from paying for a search it already paid for; the pruning makes
  the search itself proportional to the repository. Either alone would have hidden the
  symptom on this machine while leaving the other cost in place: a tool call (`grep`,
  `find`) pays `IgnoreSet::load` too, and it was walking `references/` on every call.
- **Five seconds, not "forever".** Loading once at startup would be faster and would never
  notice a `.gitignore` written during the session; the branch cache's TTL precedent is the
  honest middle, and the field's doc comment states the trade.
- **The test pins the pruning by rule count, not by timing.** Timing assertions on a shared
  machine are flaky; `IgnoreSet::load(...).len()` says exactly which `.gitignore` files were
  read, which is the property.
- **The probe learned a `--at SECONDS` option.** Separating "when the keys were typed" from
  "what was typed" is what turned a plausible timing story into a measured one, and the
  option stays because the next such question will need it.

## Gotchas

- **A performance regression in an interactive path looks like a hang.** The frame simply
  stops updating, which is indistinguishable from a deadlock — and it is the *same* symptom
  the owner originally reported. The lesson from M94/M98 repeats: measure before reading
  rendering code, and measure *what the keystroke costs*, not just how often frames are
  drawn.
- **`IgnoreSet::load` had the same flaw for every caller.** `grep`, `find` and `read` all
  call it, so the missing pruning was not a TUI problem that leaked; it was a core problem
  the TUI made visible. Fixing it in the matcher is what makes the tools cheaper too.
- **Two characters worked, six did not.** A bug that only appears past a threshold is easy
  to dismiss as flakiness — the first `@Cargo` failure was nearly written off as a probe
  timing artifact. Repeating it four times with four different waits is what made it a
  finding.

## Verification

Battery `m110a` (`target/gate-m110a.log`, copied to
[`../../audit/evidence/gate-m110a.log`](../../audit/evidence/gate-m110a.log)): **all five
gates exit 0, 1,026 passed / 0 failed / 1 ignored across 59 test binaries** — one more than
M109's 1,025, which is `the_search_is_pruned_by_the_rules_it_finds`. `epsilon-core`'s
`ignore` suite is 20 tests, all green.

**End-to-end, in a real conhost window** (`scripts/console-probe.py`), same binary path,
same console geometry, before and after:

```
before  typed '@Cargo' → composer '• ask anything'   popup rows 0   (reproduced 4x)
        typed '@argo'  → composer '• ask anything'   popup rows 0
after   typed '@Cargo' → composer '• @Cargo'   popup rows 7   first row '▎ Cargo.lock'
        typed '@argo'  → composer '• @argo'    popup rows 7   first row '▎ Cargo.lock'
```

Raw runs: [`at-popup-prefix-Cargo.json`](../../audit/evidence/at-popup-prefix-Cargo.json) (before),
[`at-popup-postfix-Cargo.json`](../../audit/evidence/at-popup-postfix-Cargo.json) and
[`at-popup-postfix-argo.json`](../../audit/evidence/at-popup-postfix-argo.json) (after).

Global `epsilon.exe`: **reinstall required, and done after this fix** — the M109 install was
stopped mid-build when the probe found this, so the regression never reached the owner's
binary, and the binary that ships is built from the tree with both fixes.

## Limits

- **The five-second TTL is a guess with a reason, not a measurement.** Nobody has measured
  how long a user waits between typing `@` and expecting an edited ignore rule to apply.
  The number is small enough to be invisible and large enough to cover a burst of typing.
- **The pruning changes which rules load in one edge case.** A `.gitignore` inside an ignored
  directory is no longer read. Git agrees it cannot matter, but a user who has been relying
  on one — because the *old* code happened to load it — will see a behaviour change. Nothing
  in the repository does this; it is recorded because it is a real difference.
- **The probe's timing knob is not a benchmark.** It measures whether a frame appeared
  within a fixed window, not how long the keystroke cost. A stopwatch around
  `IgnoreSet::load` on a large tree would give the number this slice only reasons about.
- **The `@` popup is still not covered by an automated test end-to-end.** The four
  `at_files` tests and this probe are the coverage; the probe is manual, so a future
  regression of this shape would again be found by running it rather than by the suite.
- **Nothing pins the cache itself.** A later refactor could drop `App::at_ignore` and
  restore the per-keystroke load with every test still green; only the probe would notice.
  The property that *could* be tested in-process — "two keystrokes in a row load the rules
  once" — needs a way to observe a load, which `IgnoreSet` does not currently expose.
  Recorded rather than faked with a timer assertion, which would be flaky on a shared
  machine and would not measure the thing that broke.
