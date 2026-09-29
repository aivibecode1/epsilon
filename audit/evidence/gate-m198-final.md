# gate-m198-final4 — the M198 battery

**Verdict: GATE PASS. All seven gates `exit=0`.**
**1,560 tests passed, 0 failed, 1 ignored, across 63 test binaries.**

Counted by summing every `test result: ok. N passed` line in the log (63 of
them) — never by subtracting a per-crate sweep from a battery total, which is the
arithmetic this repository has already caught itself doing once.

`1 ignored` is `epsilon-llm`'s `live_smoke`, which is `#[ignore]`d by design: it
is the only test that would make a paid provider call.

| Gate | Time | Result |
|---|---|---|
| `check-links` | 1 s | 382 relative links, 0 broken |
| `check-lint-tables` | 0 s | 11 crates inherit the workspace, 1 hand-copied and matching it |
| `fmt --check` | 3 s | clean |
| `clippy` | 117 s | clean, `-D warnings` |
| `test` | 508 s | **1,560 passed, 0 failed, 1 ignored** |
| `demo` | 52 s | completed offline |
| `real-check` | 13 s | the real-provider path compiles |

## Three failed attempts before this one

Each is recorded because each found something real, and two of the three were
defects **in the change under test** rather than in the repository.

**`m198-final` — GATE FAIL**
- `fmt --check`: two `format!` calls and one chained call were not rustfmt's
  preferred shape.
- `clippy`: `head.chars().nth(0)` in the new doubled-glyph assertion, denied by
  `clippy::iter_nth_zero`.

**`m198-final2` — GATE FAIL**
- `no_source_file_lost_a_test_since_head` reported six deleted tests:
  `_path_witness`, `from_config`, `invalidate_health`, `lookup_custom`,
  `matches_id`, `status_error`. **Every one was dead code this milestone
  deliberately removed, and not one was a test.** The guard matched every `fn` in
  the diff, so a dead helper read as a deleted test — the "a guard that cries wolf
  is a guard that gets disabled" outcome its own comment predicts, one milestone
  after it was written.

Narrowed to `#[test]`-carried functions, with a control that exercises the
classifier on diff text. **The control then caught two more problems, one of them
in the narrowed guard itself**: its lookback tested `line.trim()`, which for a
diff leaves `-#[test]`, so every test looked like a plain function and the guard
would have gone quiet. Nothing else would have reported that.

**`m198-final3` — GATE FAIL**
- Four TUI golden tests still reported doubled glyphs. `row_marker` covers plain
  rows; **tool cards bypass it** and print their own status glyph in
  `ui::card::title`. The first fix was half a fix, and the half that was done
  looked complete.
- `clear_then_reappend_drops_stale_entries` failed on the new `reconcile` eviction
  gate. The gate was `lines.len() < prev_lines_len`, which is **unsound for the
  case that matters**: `/clear` followed by a re-append of the same number of
  lines never fires it. Corrected to `reuse < prior_reconciled` — the id prefix
  breaking is the actual signal, and it needs no field at all.

## The controls for the new gate

`scripts/check-lint-tables.py` is a new gate, so it is the one thing here with no
prior art in this repository to vouch for it. Controls run by hand against
`crates/epsilon-safety/Cargo.toml`, each restored afterwards:

| # | Injected defect | Expected | Result |
|---|---|---|---|
| A | delete `missing_docs = "warn"` from the hand-copied table | FAIL | FAIL — *"is missing missing_docs = warn from the workspace table"* |
| B | change `all = "warn"` to `all = "allow"` | FAIL | FAIL — *"sets all = allow, but the workspace says warn"* |
| C | drop `workspace = true` from `epsilon-lsp` (the historical D1/D21 shape) | FAIL | FAIL — *"no [lints] table. Cargo does not inherit [workspace.lints] implicitly"* |
| D | replace the `AGENTS.md exception` reason in the comments | FAIL | FAIL — *"overrides unsafe_code … the file's comments do not say why"* |

### The control that passed when it should not have

**The first version of the checker passed on a workspace whose lint law was a
fiction, and controls A and B both passed.** The cause was one line:

```python
workspace = root_manifest.get("lints", {})
```

`[workspace.lints]` is nested **under** `[workspace]` in parsed TOML, so this read
`None`. Every table was compared against an empty one, every "missing" set was
empty, and every "extra" lint was unchecked — a checker that could not fail.

What caught it was control A specifically: removing a lint from a hand-copied
table changed nothing observable, because there was no longer anything to compare
it *against*. Fixed with one line, plus a guard that exits 2 when the workspace
table is absent, so a rename cannot silently disable the check the same way twice.

## What this battery does not cover

- **`--no-default-features`.** Not in the battery. It found a real pre-existing
  compile failure once before (`57812ca`). The new `wrap_line` signature and the
  `AsyncBufReadExt` import are both feature-independent by reading — which is not
  "verified". The offline build has not been compiled against this tree.
- **`--tui-bench`**, which cannot see any of this milestone's performance work: it
  holds no streaming text and replays no events, so it exercises neither the
  double-wrap nor the per-event restale scan. Every cost in the milestone's
  performance table is derived from the code's asymptotics and its measured call
  counts, **not from a benchmark**. A bench that streams a tail and replays 128
  events is the honest instrument and it does not exist.
- **A real console.** The glyph work was verified through `TestBackend` frames.
  The terminal integration test drives the real binary and is in the battery, but
  it asserts on `contains`, not on glyph shape — so "the doubled glyph is gone in
  a real conhost" is a claim this battery does not make.
