# M207 — a wait you can still read after the screen is replaced

**Status:** landed. GATE PASS `m207` — see [Verification](#verification). Reinstall:
**outstanding** — see the note at the end of the Verification section.

## What

Ledger **A18**, the remnant of A7. A launch that has to wait for the session file's lease prints a
countdown — `waiting for this session to be released — 4s elapsed, ~26s left` — to the console, and
then the TUI takes the screen and the alternate buffer erases it. A user who waited 25 seconds for
a second window to let go of the session starts with **no record of why anything paused**, and the
only evidence left is their own memory of a delay.

Now the wait is said again where it survives:

```text
• this launch waited 25s for the session file's lease — another process had it, and it
  has been released. Nothing was lost; the session continued from the same file.
```

## How, and why it is a process-global

The wait happens inside `build_runtime`, which four frontends share and which takes the notice
function **by name** (`lease_wait_notice`). The TUI's startup notices are assembled *before* the
store is opened, so there is nowhere in that value to put a fact learned while opening it.

Two ways out, and the cheap one is honest here: thread a second return value (or a `&dyn Fn`
parameter) through `build_runtime` and all four call sites, or record the wait in a
process-global that the TUI path reads after `build_runtime` returns. The global is one
`AtomicU64` with `fetch_max`, read once; the plumbing would be five signatures for one sentence.
Recorded as a trade rather than hidden: the cost is a static in the CLI binary, and the benefit is
that the notice cannot be forgotten by one of the four frontends.

## The threshold is a judgement, stated

**One second.** The ordinary contended launch waits a few hundred milliseconds for a lease that
another process just released, and a notice about that would fire on every second launch — which is
how a true sentence becomes noise. A second is where a person notices a pause and starts wondering
whether the program is stuck, which is the state this closes.

## What the tests cover

- `a_wait_is_remembered_for_the_frontend_that_starts_after_it` — the record survives the call.
  Asserted with `>=`, because the counter is a monotonic process-global and an exact assertion
  would be asserting on test order.
- `only_a_wait_long_enough_to_notice_becomes_a_notice` — the threshold and both sentences:
  silence at 999 ms, and a 25 s wait naming the lease and answering the reader's real question
  ("did that cost me anything?").

## Limits — what this does not prove

- **The wiring is not tested.** `lease_wait_summary` and the record are; the three lines in
  `tui_mode` that read the global and push the notice are read, not run — reaching them needs a
  second live process holding the lease *and* a started TUI, which is a two-process integration
  test this slice did not attempt. The console probe could do it (`--during` exists for exactly
  this kind of overlap) and does not yet.
- **Only the TUI says it.** `-p`, `--serve` and `--rpc` still write the countdown to the console
  and never mention the wait again; for them the console *is* the record that survives, so the
  case is weaker there — weaker, not absent, since a script that captures stdout gets the
  countdown interleaved with the answer.
- **The notice is not in a golden frame** — it is asserted at the function boundary, and the
  notice path itself is exercised by every other startup-notice test in the TUI.
- **A wait that never ends is still a wait.** Nothing here shortens the 30-second window for a
  *live* holder; that is correct behaviour (another window owns the session) and this slice only
  makes it legible.

## Verification

GATE PASS, 2026-09-29 — battery **`m207`**, run against this commit's contents with the
documentation already final. Seven gates, exit 0 each: `check-links` 0 s (462 relative links,
0 broken), `check-lint-tables` 0 s, `fmt --check` 2 s, `clippy --workspace --all-targets` 2 s,
`test --workspace` 290 s, `demo` 33 s, `real-check` 5 s.
**1,620 tests across 64 binaries, 0 failed** — summed from the 64 `test result: ok.` lines in the
log. M206 measured 1,618 across the same 64, so this slice adds the two tests above.

Evidence: [`audit/evidence/gate-m207.log`](../../audit/evidence/gate-m207.log).

**Reinstall: outstanding, and deliberately so.** It is required — this changes what a launch
says — and it did not happen: the owner asked for the disk to be cleared, and the build tree
(33.5 GB) was deleted immediately after this battery, so a release rebuild would have been a cold
30–45 minutes spent refilling the space that was just freed. The installed binary is therefore the
**M206** build, and the notice above appears only after cargo install --path crates/epsilon-cli
--features real --force is run once. Recorded here rather than left as a silent gap: the gates
below are green for these bytes, and the shipped artifact does not yet contain them.
