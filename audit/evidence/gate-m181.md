# Gate evidence — m181

Extension interception (ADR-019, phases 4.3 / 4.5a / 4.5b).

- **Source commit:** `5d5dda7` (docs) on top of `9f73a5f` (the code)
- **Log:** [`gate-m181.log`](gate-m181.log) — tracked, complete, ends in `GATE PASS`
- **Label:** `m181` (a fresh label; `scripts/gate.ps1` truncates `target/gate-<label>.log` on
  start, so re-using `m180-final` would have destroyed that run's record)
- **Build:** `CARGO_BUILD_JOBS=1`, `CARGO_TARGET_DIR=D:/eps-target`, Windows via
  `pwsh -File scripts/gate.ps1`

## The five gates

| Gate | Exit | Time |
|---|---|---|
| `cargo fmt --all -- --check` | 0 | 2 s |
| `cargo clippy --workspace --all-targets -- -D warnings` | 0 | 132 s |
| `cargo test --workspace` | 0 | 704 s |
| `cargo run -q -p epsilon-cli -- --demo` | 0 | 45 s |
| `cargo check -p epsilon-cli --features real` | 0 | 63 s |

## Test totals

**63 targets, 1,334 results, 0 failed.**

- 51 test binaries (`Running ...`)
- 12 doc-test targets, each reporting 0 tests
- 63 `test result:` lines — one per target, no binary listed twice

The previous full gate, `m180-final`, measured 1,316 across the same 63-target structure. The
+18 is not all new work: `epsilon-cli --lib` alone accounts for 12 (the 8 reducer and 4
end-to-end tests), and the remainder is spread across `epsilon-ext` and the workspace
aggregation. **The two numbers were not reconciled per-binary**, so the difference is
reported as an observed delta and not attributed.

> A note on the arithmetic rule this project has twice retracted: `1,334 - 1,316 = 18` is a
> difference between two whole-workspace gates, which is legitimate. Subtracting a
> per-crate sweep from a workspace gate is not, and the earlier `1253` figure in this
> repository is exactly that mistake. They are separate measurements and remain labelled
> as such.

## Per-crate delta for the crates this milestone touched

| Crate | Before | After |
|---|---|---|
| `epsilon-cli --lib` | 39 | 51 (+12) |
| `epsilon-ext` | 33 | 39 (+6, from 4.5a in `ece3af4`) |

The 22-test `epsilon-ext` protocol suite was **not modified** by 4.5b; the +6 belongs to
`ece3af4`.

## Negative controls

Both were run, both failed for the reason named, and both were restored with the source
rebuilt.

**1. The bridge's event tag mangled** (`before_tool_call` → `before_tool_call_typo`)

```
test runtime::ext_hooks_e2e::a_live_extension_can_veto_a_tool_call_through_the_bridge ... FAILED
the extension said no and the call went ahead anyway
test result: FAILED. 11 passed; 1 failed
```

The **8 reducer tests all passed** against this control. That is the point of having both
kinds: the reducers check the arithmetic, the end-to-end test checks that the two halves of
the seam are actually connected, and only the second can tell the difference.

**2. The interpose budget reverted to `INVOKE_TIMEOUT_SECS`**

`a_live_extension_that_never_answers_does_not_stall_the_call` failed with `Elapsed(())` on a
3 s bound against a 10 s budget. This was a **real defect in the shipped ADR**, not an
injected fault: ADR-019's failure policy ("a silent extension must not block a run") was
satisfied while its cost went unnoticed, because "must not block" and "must not delay" are
different requirements and only the first was written down. Fixed with
`INTERPOSE_TIMEOUT_SECS = 1`.

## Skip accounting

`ext-fixture` is not a Cargo bin target of `epsilon-cli`, so `CARGO_BIN_EXE_ext-fixture` is
unavailable and the e2e tests derive the path from `current_exe()`, **skipping** if it is
missing. This is deliberate (a test that cannot run must not report as one that passed), and
it is why the fixture was built explicitly and the skip count checked:

```
$ grep -c "skipping:" cli6.log
0
```

All 12 new tests ran. 8 reducer + 4 end-to-end.

## What was not verified

- **The installed release binary was not rebuilt.** `C:/Users/Hp/.cargo/bin/epsilon.exe`
  predates ADR-019 entirely. No marker, probe, or crash test covers the installed binary for
  this milestone. It must be reinstalled and probed before any claim about shipped behaviour.
- **`after_tool_call` is unwired by design** — result rewriting remains missing, so only the
  veto half of pi's tool-call event surface exists.
- **No real extension exists.** Every claim here is proven against `ext-fixture`, a test
  binary in this repository.
