# Gate evidence — m183b

Turn hooks (ADR-019, completing the last contracts seam) and the protocol documentation.

- **Source commit:** `cfdf94c` (docs + doc-drift test) on top of `570819e` (turn hooks)
- **Log:** [`gate-m183b.log`](gate-m183b.log) — tracked, complete, ends in `GATE PASS`
- **Build:** `CARGO_BUILD_JOBS=1`, `CARGO_TARGET_DIR=D:/eps-target`, Windows via
  `pwsh -File scripts/gate.ps1`

## The five gates

| Gate | Exit | Time |
|---|---|---|
| `cargo fmt --all -- --check` | 0 | 2 s |
| `cargo clippy --workspace --all-targets -- -D warnings` | 0 | 5 s |
| `cargo test --workspace` | 0 | 526 s |
| `cargo run -q -p epsilon-cli -- --demo` | 0 | 22 s |
| `cargo check -p epsilon-cli --features real` | 0 | 4 s |

## Test totals

**63 targets, 1,361 results, 0 failed** — 51 test binaries + 12 zero-test doc-test targets.

`m182` measured **1,346** across the identical 63-target structure. The delta is **exactly
+15**, fully attributed: `epsilon-cli --lib` went 63 → 78 (13 turn-hook tests + 2
doc-drift tests). No per-binary subtraction was needed, so none was performed.

## The `m183` failure, preserved rather than deleted

The first attempt at this battery is
[`gate-m183-FAILED-stale-snapshot.log`](gate-m183-FAILED-stale-snapshot.log) and it is a
`GATE FAIL`. It is kept, and named for what it is, because it is evidence of a specific
failure mode rather than of a defect:

```
error[E0308]: mismatched types
    --> crates\epsilon-cli\src\runtime.rs:2594:23
     |
2594 |             ("crash", epsilon_ext::CRASH_BUDGET),
     |                       ^^^^^^^^^^^^^^^^^^^^^^^^^ expected `u64`, found `u32`
```

**Line 2594 no longer exists.** The battery was dispatched, and then source was edited while
it ran; it compiled the pre-fix snapshot. Timestamps confirm the order: the log was written
at 06:53 and the fix committed at 06:47.

`AGENTS.md` requires a fresh label per attempt so a failed run's evidence is not destroyed.
It does not say what to do when the source moves underneath a running battery. The answer
that worked: **re-run on the committed bytes under a new label, and keep the failed log with
a name that says it is a snapshot artifact** — so nobody reads it later as the result for the
current tree.

## Negative controls

### Turn hooks (`570819e`)

Retagging the turn event `turn_end` → `turn_ended`, so the fixture's branch stops being
reached:

```
the extension asked the run to end and the loop would not be told
test result: FAILED. 12 passed; 1 failed; 0 ignored; 0 measured; 63 filtered out
```

**All 9 fold-based tests passed against the control.** Fifth consecutive time this exact
pattern has appeared: pure reducers check arithmetic, only a live child can tell whether the
two halves of a seam are connected.

### The doc-drift test (`cfdf94c`) — the sixth near-miss, and mine

The test asserting the protocol page matches the code **passed against both controls** in its
first form:

| Version | Control | Result |
|---|---|---|
| `assert doc.contains("1")` | documented budget 1s → 3s | **PASSED** — the page mentions 1 and 2 dozens of times for unrelated reasons |
| `assert doc.contains("turn_end")` | tag renamed to `turn_ending` | **PASSED** — a tag is a substring of its own typo |

Fixed by anchoring: the limit check now requires the exact table cell (`**1 second**`), and
the tag check splits on non-alphanumerics and compares whole tokens. Both controls then fail,
naming their reason:

```
docs/user/extensions.md does not contain `**1 second**` for the interpose limit
the protocol page never names `turn_end` as a standalone token
```

Restored with `cmp -s` confirming byte-identity, and the source touched before rebuilding —
a `cp` that preserves mtime can leave cargo re-running the control binary.

## Skip accounting

`ext-fixture` is not a bin target of `epsilon-cli`, so `CARGO_BIN_EXE_ext-fixture` is
unavailable and the e2e tests derive the path from `current_exe()`, skipping when absent.
Built explicitly; skip count checked at 0.

## What was not verified

- **The installed release binary was not rebuilt** for the turn hooks or the protocol page.
  `C:/Users/Hp/.cargo/bin/epsilon.exe` is the 06:22 build, which has the tool hooks only.
- **No real extension exists.** Every claim about the protocol is proven against
  `ext-fixture`, a test binary in this repository. The protocol page is written for a third
  party and **has not been read by one** — that is the honest limit of 4.6.
- **`transform_context` / `convert_to_llm` are pass-through by decision**, so the context-
  reshaping half of pi's turn surface is deliberately absent. Reasoning in
  [`../02-port-fidelity.md`](../02-port-fidelity.md).
- **The doc-drift test checks the constants and the four event tags.** It does not check the
  *decision shapes* documented in the prose, so a changed `block`/`content` field name would
  not be caught. That is a known limit of a source-level check rather than a generated page.
