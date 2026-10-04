# M208 — one timestamp formatter, moved before it was duplicated

**Status:** landed. GATE PASS `m208` — see [Verification](#verification). Reinstall: **not
required** (no user-visible behaviour changes; the installed binary is still the M206 build, as
[M207](M207-the-wait-you-can-read.md) records).

## What

`epoch_millis_to_iso8601` — the hand-rolled civil-date conversion that renders a timestamp the way
JavaScript's `toISOString()` does — lived in `crates/epsilon-cli/src/rpc_mode.rs`. Designing
[plan 020](../plans/020-one-session-per-file.md) (one session file per session) showed it was about to be needed a
second time: pi names a session file `<ISO timestamp>_<id>.jsonl`
(`repo.ts:42-45`), and that ISO string is byte-for-byte the one the RPC surface already produces.

A library may not depend on the CLI (architecture law), so the choice was **duplicate it or move
it**. Moving won: the formatter now lives in `epsilon_session::time`, the crate that owns session
files and session timestamps, and `rpc_mode.rs` calls it. The CLI's own boundary tests still
exercise it — they now exercise the shared function, which is the point.

## Why this was worth a slice of its own

This is A12's failure ([audit/03-defects.md](../../audit/03-defects.md)) caught one step earlier. A12's measured finding was
`global_dir()` written **byte-for-byte twice** in two crates that cannot see each other, and this
repository's D-class is exactly that shape: two copies of one rule drift, and neither copy can
notice. The cheapest moment to fix a duplication is before the second copy exists — after it, the
fix costs a reconciliation and a behaviour audit.

The doc comment on the moved function says so, and so does the one-line pointer left where the
implementation used to be, so a reader who greps for it in `rpc_mode.rs` is told where it went
rather than finding a silent absence.

## The disk knob, disclosed

This round ran with `CARGO_PROFILE_DEV_DEBUG=0` and `CARGO_PROFILE_TEST_DEBUG=0`, at the owner's
request to keep the drive free. It does not change what is compiled or what is asserted — only
whether debug info is emitted — and the effect is large and measured:

| | debug info on | debug info off |
|---|---|---|
| `epsilon-session` build tree | (not measured this round) | **0.19 GB** |
| the previous full battery's tree | ~33 GB | — |

The cost is real and is the reason this is disclosed rather than quietly applied: **a panic
backtrace in a failing test is much less useful**, and the next round that debugs a failure should
build with debug info on. The gate log names the knob.

## Verification

GATE PASS, 2026-09-29 — battery **`m208`**, run against this commit's contents with the
documentation already final, and with the disk knob above applied. Seven gates, exit 0 each:
`check-links` 0 s (474 relative links, 0 broken), `check-lint-tables` 0 s, `fmt --check` 2 s,
`clippy --workspace --all-targets` 240 s, `test --workspace` 675 s, `demo` 113 s,
`real-check` 1 s.

**1,622 tests across 64 binaries, 0 failed** — summed from the 64 `test result: ok.` lines in the
log. M207 measured 1,620 across the same 64; this slice adds the two boundary tests in
`epsilon_session::time`, and the CLI's pre-existing assertions about the same string still pass
unchanged, which is the evidence that the move is faithful.

**Disk, measured:** the entire workspace build tree after the full battery is **4.19 GB**, against
~33 GB for the same battery with debug info on, and C: finished with 50.6 GB free. The cold cost
was 12m46s for `cargo check -p epsilon-cli`.

Evidence: [`audit/evidence/gate-m208.log`](../../audit/evidence/gate-m208.log).

**Reinstall:** not required — no user-visible behaviour changed. The installed binary is still the
**M206** build, as M207 recorded.

## Limits — what this does not prove

- **Nothing yet *uses* the formatter for a file name.** Plan 020 step 1 is the naming rule; this
  slice only makes sure the string it needs has one home. Until that step lands, the move is
  justified by a plan rather than by a second caller — which is exactly the "faithful port, no
  production caller" shape this repository watches for, so it is recorded as *one move with one
  caller removed*, not as a feature.
- **The RPC boundary tests moved in effect, not in file.** They still live in `rpc_mode.rs` and
  call the function directly, so the wire's *use* of it (the `timestamp` key in an entry payload)
  is asserted where it always was. `epsilon-session`'s own copy of the boundary cases is new, so
  the same arithmetic is now pinned in two test files — that is test duplication, not rule
  duplication, and deleting the older assertions to save bytes would be the wrong trade.
- **No behaviour changed, so no negative control applies.** The evidence that the move is faithful
  is that the CLI's pre-existing assertions about the same string still pass unchanged.
