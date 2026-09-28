# Gate evidence — m182

Result rewriting: the `after_tool_call` half of ADR-019 (completes pi's tool-call event
surface).

- **Source commit:** `2fb5219` (the code) on top of `9f73a5f` (the before-arm)
- **Log:** [`gate-m182.log`](gate-m182.log) — tracked, complete, ends in `GATE PASS`
- **Label:** `m182`, fresh. `scripts/gate.ps1` truncates `target/gate-<label>.log` on start,
  so re-using `m181` would have destroyed that run's record.
- **Build:** `CARGO_BUILD_JOBS=1`, `CARGO_TARGET_DIR=D:/eps-target`, Windows via
  `pwsh -File scripts/gate.ps1`

## The five gates

| Gate | Exit | Time |
|---|---|---|
| `cargo fmt --all -- --check` | 0 | 2 s |
| `cargo clippy --workspace --all-targets -- -D warnings` | 0 | 128 s |
| `cargo test --workspace` | 0 | 722 s |
| `cargo run -q -p epsilon-cli -- --demo` | 0 | 43 s |
| `cargo check -p epsilon-cli --features real` | 0 | 51 s |

## Test totals

**63 targets, 1,346 results, 0 failed** — 51 test binaries + 12 zero-test doc-test targets.

`m181` measured **1,334** across the identical 63-target structure. The delta is
**exactly +12**, and it is fully attributed this time: `epsilon-cli --lib` went 51 → 63
(10 folding tests + 2 end-to-end). `epsilon-ext` is unchanged at 39, and the protocol suite
is untouched. No per-binary subtraction was needed to reach that number, so none was done.

## Negative control

Retagging the after-event as `before_tool_call`, so the fixture's `tool_result` branch stops
being reached:

```
test runtime::ext_hooks_after_e2e::a_live_extension_can_rewrite_a_tool_result_through_the_bridge ... FAILED
test result: FAILED. 23 passed; 1 failed; 0 ignored; 0 measured; 39 filtered out
```

**All 10 folding tests passed against the control.** That is the fourth time this pattern has
repeated and it is now the expected result, not a surprise: pure reducers check the folding
arithmetic, and only an end-to-end test through a live child can tell whether the two halves
of the seam are connected. The before-arm control behaved identically (`9f73a5f`).

The control was restored with `cp`, then **`touch`**-ed before rebuilding — a `cp` that
preserves mtime can leave cargo deciding the source is unchanged, skipping the rebuild, and
silently re-running the *control* binary. That cost two rounds earlier in the session and is
now a habit, not a hope.

## Skip accounting

`ext-fixture` is not a bin target of `epsilon-cli`, so `CARGO_BIN_EXE_ext-fixture` is
unavailable and the e2e tests derive the path from `current_exe()`, skipping if absent. The
fixture was built explicitly and the skip count checked:

```
$ grep -c "skipping:" t1.log
0
```

`epsilon-cli --lib`: 63 passed, 12 new (10 folding + 2 e2e), **0 skipped**.

## What pi actually does, and where Epsilon deliberately differs

Read from `references/pi-0.85.1/packages/coding-agent/src/core/extensions/`:

| pi | Epsilon | Status |
|---|---|---|
| `emitToolResult` returns `undefined` when no handler modified anything (`runner.ts:30504`) | `after_tool_call` returns `Ok(None)` | **match** — and load-bearing: the loop uses `None` to keep its message byte-for-byte, where `Some(empty)` would rebuild it with no field changed and silently drop `usage` via `unwrap_or` |
| `emitToolResult` folds each handler into a **running** event, per-field, only for fields explicitly set | per-field `get_or_insert_with` fold | **match** |
| `ToolCallEvent.input` is **mutable in place**, *"No re-validation is performed after mutation"* | `args` is passed by value and never returned | **deliberate divergence** — args crosses a process boundary, so honouring it means re-validating against the tool schema, which pi does not. Porting the unsafe half would be worse than the honest gap |
| `emitToolCall` returns on the **first** `block` | every extension is consulted; a veto is a veto | **deliberate divergence** — the reason a user sees names every objector, not whichever loaded first. Costs a round trip per remaining extension on a call that was refused anyway. Veto semantics identical |
| `ToolCallEventResult.terminate` honoured only when *every* blocked call in a batch sets it | carried on both result structs; `exec.rs:788` encodes it in `details` | **match** — the bridge carries the hint, the loop applies the rule |

## What was not verified

- **The installed release binary has since been rebuilt and probed** — see
  [`installed-binary-2026-09-28-post-adr019.md`](installed-binary-2026-09-28-post-adr019.md).
  It now carries both arms (`rewritten_by_extension` present), and the crash and live-stream
  probes both pass against it. The gap recorded here is closed.
- **No real extension exists.** Every claim is against `ext-fixture`, a test binary in this
  repository.
- **Multi-extension result rewriting is tested on the fold only**, never with two live
  children. The sequential host loop is not exercised end to end.
- **Image and tool-use content blocks are dropped, not supported.** `content_block_from_json`
  handles `text` only; pi's `(TextContent | ImageContent)[]` includes images. Dropping is the
  documented behaviour and is tested, but it is a capability gap, not a faithful match.
- **Argument-mutating extensions cannot work at all.** `BeforeToolCallResult` has no `args`
  field. See the divergence table above for why.
