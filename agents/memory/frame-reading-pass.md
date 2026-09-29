# Working note — the frame-reading pass (opened during M199, closed by M201)

**Status: CLOSED.** All three findings below were fixed by M201
([`docs/milestones/M201-frame-truth.md`](../../docs/milestones/M201-frame-truth.md)), and the
frames that carried them are kept as before/after evidence:
`audit/evidence/frames/m199-baseline/` → `audit/evidence/frames/m201/`.

The note stays because the **method** is the durable part, and because M201 found a fourth
defect with it that no finding here named.

## How to look (unchanged, and this is the thing to keep)

```sh
export CARGO_BUILD_JOBS=1 CARGO_TARGET_DIR="C:/Users/Hp/AppData/Local/Temp/epsilon-target"
EPSILON_FRAME_DIR="C:/Users/Hp/Desktop/My Agent Space/Epsilon/audit/evidence/frames/<label>" \
  cargo test -p epsilon-tui --lib frame_dump:: -- --ignored --nocapture --test-threads=1
```

`EPSILON_FRAME_DIR` must be **absolute**. A relative value resolves against the package root
(`crates/epsilon-tui/`), because that is a `cargo test` process's working directory — the
harness refuses a relative one and prints every path it writes.

**Read the frame at 60 and 80 columns, not only at 100.** Every defect below was invisible at
100 and obvious at 60, and M201's own regression (the header pin) only appeared at 60 too.

## The findings, and what closed them

| | Finding | Closed by |
|---|---|---|
| F1 | A word was mutilated at 60 columns: "and **then**" → "and **the**", and the final `.` lost at 80. | `wrap_line` decided a word fit *before* counting the space in front of it, so a row could be emitted one column over budget and the draw path clipped its last cell. |
| F2 | The header lost its `↑` at 60 columns and printed `idle 0 ↓0`. | `metric_spans` budgets with `layout::room_beside` and drops whole gauges; the model and its `offline` qualifier are pinned by a field on `MetricPart`. |
| F3 | `?`, `CH n/a` and the trailing segment had no legend. | `ui::metrics::LEGEND` + `the_legend_decodes_every_token_the_header_can_print`. |

F2's fix was wrong twice before it was right, and both wrong versions rendered a plausible
row: the first dropped leading fragments and kept a cache gauge that says nothing when the
provider cannot report one, and the second found the model's pin by `MetricRole::Model` —
which is `Warn` in an offline session, so the pin stopped matching in exactly the row long
enough to need it. **Neither was visible in any assertion in `header.rs`.** Render the frame.

## What was *not* wrong, measured (still true after M201)

- Markers are single: no `• •` anywhere in either frame set.
- The cache rate reads `CH n/a`, not `CH 0%`.
- The permission dialog renders and states its keys.

## The rule this pass earned, in one line

**A fragment is worse than an absence**, because a wrong value is not noticed the way a missing
one is — and it applies to a key and to a label, not only to a number.
