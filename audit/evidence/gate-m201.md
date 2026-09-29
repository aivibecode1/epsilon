# Battery `m201` — GATE PASS

Full log: [`gate-m201.log`](gate-m201.log) (134,902 bytes, copied out of `target/` before
the cleanup). Command: `pwsh -File scripts/gate.ps1 m201`. Tree: the M201 commit's contents.

| Gate | Exit | Seconds |
|---|---|---|
| `check-links` | 0 | 0 |
| `check-lint-tables` | 0 | 0 |
| `fmt --check` | 0 | 3 |
| `clippy --workspace --all-targets -- -D warnings` | 0 | 3 |
| `test --workspace` | 0 | 512 |
| `demo` | 0 | 12 |
| `real-check` (`cargo check -p epsilon-cli --features real`) | 0 | 10 |

**1,577 tests across 63 binaries, 0 failed** — summed from the 63 `test result: ok. N passed`
lines in the log (M199 measured 1,568 across the same 63 binaries, so this slice adds 9).

`clippy` and `fmt` are 3 s each because the workspace was already compiled by the inner loop;
the `test` leg at 512 s is the first leg that had to link every target.

## What this does and does not cover

- The log is the evidence of record for the **code and documentation in the M201 commit**.
  Nothing was edited while the battery ran; the doc files were final before it started.
- `test --workspace` includes the CLI's doc-structure tests, which read `docs/history.md` by
  `include_str!` and assert that every milestone from 1 to the newest has a row in order. That
  is why the M200 and M201 rows exist before this battery and not after it.
- It does **not** cover the frame evidence: `audit/evidence/frames/m201/` was produced by
  `app/frame_dump.rs`, which is `#[ignore]`d so the battery stays readable. The command is in
  `agents/memory/frame-reading-pass.md`.
- It does **not** drive a real terminal. The frames are `TestBackend` renderings;
  `crates/epsilon-cli/tests/terminal.rs` (inside this battery) proves the binary reaches a real
  pseudo-console, and does not read rows back at 60 columns.
