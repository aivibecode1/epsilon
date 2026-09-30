# M227 — two load-sensitive tests, one dependency removed and one mystery intact

**Ledger row A21, whose text says "cause not established".** It still is. What this milestone does is
remove a *dependency on machine throughput* from two assertions, and record two reproduction attempts
that failed — which rules something out, which is the most a round like this can honestly achieve.

## The two failures, and what they have in common

```text
a_child_that_never_stops_sending_lines_is_cut_off_by_volume
    expected Protocol once the byte budget is spent, got Timeout
a_request_after_the_reader_stops_is_refused_even_though_the_child_is_alive
    a request issued after the reader stopped did not return within 2s
```

The second is downstream of the first, and that is the diagnosis worth having: both tests spend the
reader's byte budget with `--flood 200000 100` — 200,000 log lines of 100 bytes. When the budget is
*not* spent (the invoke hits its ten-second clock deadline instead), the reader is still alive, so
`self.stopped` is still false, so the second invoke is **not** fast-refused and outlives its two-second
bound. One failure, two symptoms.

So the interesting question is why the budget was not spent — and `--flood N M` writes N **lines**, so
the cost is 200k pipe writes on the child's side and 200k JSON parses on the reader's, all of which has
to finish inside ten seconds. The property under test is *volume*; the fixture expressed volume partly
as *line count*, which is a statement about the machine.

## The change

`--flood 2000 12000` — the same ~24 MB past the 16 MiB budget, in 2,000 lines rather than 200,000.
The budget is then spent after ~1,350 parses instead of ~134,000, so the assertion no longer depends on
per-line throughput. Both tests changed, because both spent the budget the same way.

## What this does *not* claim

**The cause is still not established, and the control says so.** Before keeping the change, I tried to
reproduce the failure with the *old* flood under synthetic load, and it passed every time:

| attempt | condition | old form (`200000 100`) |
|---|---|---|
| 1 | the two tests, `--test-threads=1`, 6 CPU burners | **passed** |
| 2 | the two tests, `--test-threads=1`, 10 CPU burners | **passed** |
| 3 | the whole 29-test binary at default parallelism, 10 burners | **passed** (29/29) |

So plain CPU saturation is *not* the trigger, and a hypothesis I could have written up as "it is the
line count under load" is not supported by anything I could reproduce. The change stands on its own
merit — a test whose outcome depends on how fast the box can push lines through a pipe is a test that
will fail somewhere, and this one now expresses volume in the unit the property is about — and the row
stays **open**, with a sharper next step than "cause not established".

**What the next attempt should do differently.** The battery is not CPU load; it is *everything else
the battery does at the same time*: cargo linking other crates, the terminal integration test driving a
ConPTY, the demo and real-check legs, and five test binaries running concurrently. Synthetic burners do
not reproduce that. The honest reproduction is a battery in which this binary is the one that fails, or
a harness that runs the ext tests concurrently with the rest of the workspace's test set — not a loop
of `cargo test -p epsilon-ext`, which passes because nothing else is competing.

## Verification

**Battery `m227` GATE PASS** — `audit/evidence/gate-m227.log`: **1,652 tests across 65 binaries**,
the same total as M226 (no test added or removed — this milestone changes a fixture's shape, not the
suite). Gate shape: check-links 0 s, check-lint-tables 0 s, fmt 2 s, clippy 3 s, test 209 s, demo 4 s,
real-check 1 s. Run with `CARGO_PROFILE_DEV_DEBUG=0` / `CARGO_PROFILE_TEST_DEBUG=0`.

**The control runs are recorded above rather than summarised**, because "it passed under load" is only
useful next to what "load" was.

## Limits

- **This cannot prove the flood was the trigger.** It proves the test no longer *requires* a
  throughput it should not have required. If the battery fails again on either test, the diagnosis has
  not advanced — the next thing to look at is whether the reader spends its budget at all when the
  child is alive and the machine is busy, which needs an instrumented run rather than a faster flood.
- **The two-second bound in the second test is still two seconds.** It is defensible (the fast refusal
  is an atomic check, not a wait) but it is the tightest bound in this suite, and the support module's
  own note about loaded machines applies to it as much as to the wide ones.
- **`epsilon-ext`'s other 27 tests still spawn a child each**, so the suite's floor cost — and its
  exposure to whatever else the machine is doing — is unchanged.
