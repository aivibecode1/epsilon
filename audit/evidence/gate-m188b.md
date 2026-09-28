# Gate evidence — m188b

Confirms M188: four more pi RPC commands, and D26 (the `follow_up` wire name).

- **Source commit:** `30cb04c` (feature), `b851eb3` (memory), `a7a4f07` (the failed log)
- **Log:** [`gate-m188b.log`](gate-m188b.log) — tracked, complete, ends in `GATE PASS`
- **Build:** `CARGO_BUILD_JOBS=1`, `CARGO_TARGET_DIR=D:/eps-target`, Windows via
  `pwsh -File scripts/gate.ps1`
- **Tree was clean** before the run — `git status --short` printed nothing

## The five gates

| Gate | Exit | Time |
|---|---|---|
| `cargo fmt --all -- --check` | 0 | 2 s |
| `cargo clippy --workspace --all-targets -- -D warnings` | 0 | 3 s |
| `cargo test --workspace` | 0 | 228 s |
| `cargo run -q -p epsilon-cli -- --demo` | 0 | 7 s |
| `cargo check -p epsilon-cli --features real` | 0 | 1 s |

## Test totals

**63 targets, 1,438 results, 0 failed** — 51 test binaries + 12 zero-test doc-test targets.

Compared target-by-target against `gate-m187.log`, not by subtraction from a remembered total:

| | m187 | m188b |
|---|---|---|
| test binaries | 51 | 51 |
| results | 1,427 | 1,438 |
| `epsilon_cli` | 94 | **105** |
| every other binary | identical | identical |

**Delta +11, entirely `epsilon_cli`, every other target unchanged.** That is the 11 new tests
(9 for the four ops, 2 for the wire names).

## The first attempt failed, and the failure is preserved

`gate-m188-FAILED-midbattery-edit.log` — `GATE FAIL` on one test:

```
docs/history.md has no row for milestones [187] (newest is M188)
```

**My fault, and not the stale-snapshot pattern.** I added the M188 history row while the
battery was running, before adding the M187 row it required. `docs/history.md` is
`include_str!`'d by the history-integrity tests added in m187, so editing it during a
battery *is* editing the source under test.

Nothing was stale: the log is a truthful record of a real moment when the file lacked a row
for M187, and the test reporting it is the test working. It is preserved under an explicit
label rather than deleted, and `m188b` ran on committed bytes with a clean tree.

Worth noting the second trap in that commit: `audit/evidence/*.log` is gitignored, so the
named file needed `git add -f`. The glob swallowed it silently the first time — the second
time that has bitten in this project.

## What this round proves, and what it does not

**Proves:** the four commands work in-process; `clear_queue` empties both queues; the
per-model split reaches the cost; pi's `follow_up` reaches its handler; the eleven new tests
pass and are attributed.

**Does not prove:** anything about a real editor client. `dispatch_line` is in-process. The
m186 stdio probe drove the six commands added there through a pipe; **these four have not
been**, and `get_messages` in particular can return an array large enough to stress the line
writer in a way no in-process test exercises. The installed release is still the M186 build
and does not contain any of M188.

## Not verified

- **No real provider, no credentialed model, no live extension.** This machine has no provider
  keys; `faux-1` is the only selectable row.
- **No session that switched models was measured end to end.** The per-model split is asserted
  through one model's rows; the two-model case is what the split exists for.
- **`get_commands` is compared against the constant the handler reads, not against what the
  TUI would accept.** A command in `SLASH_ITEMS` that `submit_editor` rejects is a real
  defect this milestone does not look for.
- **Cache columns are carried but unasserted** — the faux provider does not bill cache traffic.
