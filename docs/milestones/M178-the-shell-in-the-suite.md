# M178 — putting the shell in the suite, and a stale test that had been failing for 27 milestones

## The gap

M177 measured the TUI under PowerShell with the out-of-band probe, and its most important
limit was this: *"the ConPTY terminal test still runs under cmd, so 'the TUI works in
PowerShell' rests on the probe alone, not on the test suite."*

Worse, reading the test showed the phrasing was wrong. `Session::start` launches
`epsilon` **as the pty's own process** — there is no shell anywhere in that suite. So the
suite was not cmd-only; it was shell-less, and no test ever covered the way a user
actually starts the program.

## What was added

`Session::start_in_shell` puts a shell in the pty and runs the TUI as its child:

- `the_tui_works_when_started_from_powershell`
- `the_tui_works_when_started_from_cmd`

Each asserts the process is alive, the first frame draws the composer or the welcome
screen, the status line is present, **typed text reaches the composer**, and no row is
wider than the terminal. Each skips loudly when its shell is not on `PATH`, because the
suite also runs on ubuntu where neither exists.

A shell-launched TUI is not the same problem as a directly-launched one: the shell has
already written a prompt and a banner to the same screen before the TUI takes over. That
is precisely the situation the owner's report was about, and nothing tested it.

The `cmd` variant failed on its first run — an entirely empty screen, meaning the program
never started. The cause was quoting: `cmd.exe` applies its own rules to the outer quotes
of a `/c` argument, and I had quoted a path with no spaces. Unquoted works, and the test
now quotes only when the path actually contains a space, with a comment saying why.

## The stale test, which is the more interesting finding

The first run also failed `the_screen_reports_what_the_harness_is_doing` — a test I did
not write and had not touched:

```
no cost on the first frame:
idle    ?/164k · ↑0 ↓0 · R0 W0 CH — · DeepSeek V3.1
```

It asserted a `$` and the literal word `cache` on the first frame. **M151 made both of
those deliberately untrue**: a cost is shown only once there is one, because a fabricated
`$0.00` is a lie, and the cache became a volume/rate form. The test has been failing
since M151.

It went unnoticed because it needs `EPSILON_BIN`, which the battery does not set — so
`cargo test --workspace` skips it silently, and the terminal suite is only run by hand.
**27 milestones of TUI work happened without this test being executed once.** The
discipline this project built — a negative control for every claim — has a hole in it
exactly where a test is excluded from the battery.

The fix is the current contract, asserted in both directions:

```rust
assert!(!screen.contains('$'), "a cost is shown before any turn has run, so it must be fabricated");
assert!(screen.contains("CH"), "no cache indicator on the first frame");
...
session.type_text("say hi");
session.key(b'\r');
session.settle(Duration::from_secs(6));
assert!(after.contains('$'), "no cost after a turn has run");
```

The post-turn half is new. Asserting only "no cost before" would let a header that never
shows a cost at all pass — which is how a test outlives the thing it was written for.

## Controls

Making an unpriced model claim a cost, in `Metrics::cost_gauge`:

```
epsilon-tui --lib  →  FAILED  an_unpriced_model_omits_cost_instead_of_claiming_zero
terminal           →  FAILED  the_screen_reports_what_the_harness_is_doing
                      panicked at terminal.rs:344:
                      a cost is shown before any turn has run, so it must be fabricated
```

Both levels fail, which is what makes the terminal assertion worth having. **The first
attempt at this control did not compile** — `mismatched closing delimiter` — and a
compile error says nothing about whether a test bites. Recorded, because it is the third
time this project that a malformed edit was nearly logged as a control.

Restored with `git checkout --` and confirmed clean, which was the only correct route
here: the earlier restore had already consumed the backup file.

## Verification

- Terminal suite, with `EPSILON_BIN` set: **7 passed**, 0 failed (5 existing + 2 new),
  70.01s, `--test-threads=1`.
- Battery `s186b`: FMT=0, CLIPPY=0, TEST=0, DEMO=0, TERM_EXIT=0; **62 binaries, 1,300
  passed, 0 failed**. The count rose from 1,298 to 1,300 because the two new `#[test]`
  functions are now *collected*; without `EPSILON_BIN` they skip, and they are the only
  two tests in the workspace that report success without running.

## Limits

- **The terminal suite still does not run in the battery**, because it needs a built
  binary. The stale test survived 27 milestones for exactly that reason, and this
  milestone fixes the assertion rather than the exclusion. **A test that only runs by
  hand is a test that rots**, and the next one will.
- **Two shells, one script each.** No profile, no banner, no quoting beyond the path, and
  no shell configuration. `-NoProfile` on PowerShell is exactly the unrealistic part.
- **The shell tests only cover startup, drawing and typing.** They do not submit a turn
  through a shell, so the approval dialog, streaming and token counters are still
  verified only by the probe.
- **`which()` is a small hand-rolled PATH scan.** It exists because the suite must skip
  cleanly on a machine without the shell, and it reads `PATHEXT` on Windows and tries the
  bare name everywhere. A CI image with an unusual PATH could make it skip silently.
