# Gate evidence — m187

Confirms the D23 history repair and the two structural tests that guard it. No production code
changed in this round — `docs/history.md` and one test file.

- **Source commit:** `38e7a5e` (tests), with the repair in `f6636ff` and the ledger in the same
  commit as the tests
- **Log:** [`gate-m187.log`](gate-m187.log) — tracked, complete, ends in `GATE PASS`
- **Build:** `CARGO_BUILD_JOBS=1`, `CARGO_TARGET_DIR=D:/eps-target`, Windows via
  `pwsh -File scripts/gate.ps1`

## The five gates

| Gate | Exit | Time |
|---|---|---|
| `cargo fmt --all -- --check` | 0 | 2 s |
| `cargo clippy --workspace --all-targets -- -D warnings` | 0 | 7 s |
| `cargo test --workspace` | 0 | 281 s |
| `cargo run -q -p epsilon-cli -- --demo` | 0 | 6 s |
| `cargo check -p epsilon-cli --features real` | 0 | 2 s |

## Test totals

**63 targets, 1,427 results, 0 failed** — 51 test binaries + 12 zero-test doc-test targets.

`m186` measured **1,425** across the identical 63-target structure. Delta **+2**, and the
`epsilon-cli --lib` count moved 92 → 94, which is exactly the two history tests. No
subtraction across trees was needed.

## What this round did

Closed **D23**: `docs/history.md` lines 196–213 each carried a spurious `M161`–`M178` prefix
glued onto whatever followed, which destroyed three rows and a section heading, and lost the
18 milestone rows entirely. All 18 were reconstructed **from the dev logs** — every one of
M161–M178 has one, so no description is guessed. Six more rows (M154, M186, M16, M80, M179,
M180) were recovered the same way, each naming the record it came from.

Added two tests so it cannot recur silently. Controls reintroduce the exact D23 signature:

```
control -> 2 FAILED
  "bare milestone numbers at line start (the D23 signature): M163| M185 | ..."
  "no row for milestones [185] (newest is M186)"
real    -> 2 passed
```

## Two scoping mistakes, both caught, both mine

**Too narrow.** The missing-milestones check sliced only the timeline section, reported 79
missing milestones, and I began reconstructing them — until M96 turned up at line 143 under
`## Original acceptance criteria`, a *second table by design*. The list was an artefact of my
slice. The check now spans both tables.

**Too broad.** The ascending check ran across the whole file and failed on `M6 after M186` —
the same second table, restarting at M6. It now resets at every heading.

Same error in opposite directions: assuming one table where the document has two. It is worth
recording because the file has looked like a single table to me for most of this session.

## What was not verified

- **No production code changed**, so nothing here re-proves the RPC work from `m186` beyond
  what that round already recorded. This battery's value is the doc-integrity check and the
  confirmation that the history edit broke nothing.
- **The installed release was not rebuilt.** It is still the M185 build and has neither the six
  RPC commands nor the two history tests.
- **D25 remains open**: one history row numbered `| 018 |`, which predates the D23 corruption
  and whose true milestone number has not been confirmed. Left verbatim rather than renamed on
  a plausible guess.
