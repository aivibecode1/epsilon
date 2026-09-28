# Gate `m189b` — three store-backed RPC commands

**Verdict: GATE PASS.** 63 targets, **1,446 results, 0 failed.**

| Stage | Exit | Time |
|---|---|---|
| `fmt --check` | 0 | 2s |
| `clippy` | 0 | 61s |
| `test` | 0 | 285s |
| `demo` | 0 | 5s |
| `real-check` | 0 | 1s |

- **Log:** `audit/evidence/gate-m189b.log`
- **Source commit:** `cfa4565` (fix) / `977de23` (source + docs)
- **Baseline:** M188B — 1,438 results, 63 targets
- **Delta:** **+8**, entirely `epsilon-cli --lib` (105 → 113)

## The failed predecessor, and what it was

`m189` (first attempt) is preserved at
`audit/evidence/gate-m189-FAILED-clippy-selfdeadlock.log`. It failed **clippy** with
`and_then(|x| Ok(y))` in `get_entries`.

Reading the lint instead of silencing it showed it was pointing at a **latent
self-deadlock**: the chain re-locked the session store a second time inside the
`and_then` closure to fetch the leaf, and `std::sync::Mutex` is not reentrant.

It did not hang in the tests because the closure ran after the first temporary
guard had already dropped, so the two locks were sequential by accident. That is
luck, not design — the next refactor that kept the guard alive across the closure
would have hung. The fix takes one lock and reads both values from the same guard.

`m189`'s later stages (test 703s, demo 203s, real-check) all **passed on the
pre-fix bytes**, so this log also records that the deadlock was invisible to the
whole test suite. It was found by the linter, not by a test.

## Provenance

Both attempts were run on this machine with `CARGO_BUILD_JOBS=1` and
`CARGO_TARGET_DIR=D:/eps-target`, one build owner at a time. The `m189` failure
log was copied out of `target/` before the corrected run reused the directory.

## Per-target note

`epsilon-cli --lib` grew 105 → 113. The 8 new tests:

- 2 two-model session billing tests (M188's named gap)
- 1 `clear_queue` text-shape assertion (upgraded, not new)
- 6 for `get_entries` / `get_tree` / `set_session_name`: name round-trip through
  `list_sessions`, branch-vs-file on a real fork, `since` as an exclusive
  set-difference, unresolvable `since` over-sending, honest no-store refusals, and
  pi's `type` tag on all three.

## Controls run before this battery

| Control | Result |
|---|---|
| bill every response at the session's **first** model | **FAILED** — one row, both models merged |
| `get_tree` returns the whole file instead of the branch | **FAILED** — `branch=6 file=6`, test names why |
| bill at `self.model_id()` (a no-op mutation) | passed — **control invalid**, production code correct |
| `get_tree` on a linear session | passed — **test could not distinguish the two ops** |
