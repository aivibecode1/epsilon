# M34 — pi-parity P0 batch 2: abort reaches the stream, window-relative compaction

Second execution batch against the 2026-09-03 parity audit. Both items are
loop/harness P0s with no new dependencies.

## What landed

1. **Abort during LLM streaming (audit H1, P0).** `stream_one_turn` awaited the
   provider stream with no cancellation branch: `abort_current_run` reached
   in-flight tools (M12) but a run stuck in stream creation (connect/TTFB
   hang) or mid-stream deltas ignored it until the provider finished.
   Both awaits now race the run token in `tokio::select!` (`biased`, so an
   already-cancelled token beats a simultaneously-ready event): cancellation
   drops the provider stream and synthesizes an in-band `AssistantMessage`
   with `StopReason::Aborted`. Folded-so-far text/thinking is preserved (it
   really streamed); tool-call fragments are dropped — they never executed,
   and keeping them would fabricate dangling calls. The existing failure path
   turns it into `TurnEndReason::Aborted` with no tool batch, run ends.
   This mirrors pi passing its AbortSignal into `streamFunction`.
2. **Window-relative compaction trigger (audit H2, P0).** The trigger was a
   fixed `tokens > 120_000` with no window input and no off switch; pi's
   `shouldCompact` fires at `tokens > contextWindow - reserveTokens`, gated on
   `enabled`. `LoopLimits` gains `context_window_tokens: Option<usize>`,
   `compaction_reserve_tokens: usize` (16_384, pi's `reserveTokens`), and
   `compaction_enabled: bool` (default true). `LoopLimits::for_model` resolves
   a window from the bundled catalog (unknown ids — custom providers, faux —
   keep `None`, preserving old behavior); the CLI real path uses it. The live
   window lives in a `model_window_tokens` cell seeded at construction and
   refreshed by `set_model_id`, so `/model` hot-swaps retarget the trigger
   with no stale window surviving (unknown ids clear back to `None`).
   Task children inherit the parent's window through the existing
   `set_model_id` call in `run_task`.

## Key decisions

- Abort synthesizes `Ok(aborted)` rather than a new `RunError` variant: the
  message stays in-band (transcript, `produced`, `TurnEnd`, `AgentEnd` all see
  it), matching how every other failure travels. No caller changes.
- Empty text/thinking placeholder blocks (stream index-alignment artifacts)
  are stripped from the abort message; images pass through (real content).
- `set_model_id` — not just construction — owns the window refresh, because
  the TUI hot-swaps models on a live agent. A plain `limits` field would have
  gone stale; the one-cell override keeps `LoopLimits` itself immutable.

## Gotchas

- First select draft conflated stream-end (`None` from `next()`) with abort
  (also `None`): a provider bug ending the stream early would have been
  reported as Aborted. Restructured so cancellation returns from its own
  select arm and stream-end keeps the original `Internal("...without Done")`
  error — caught on re-read before any test ran.
- `for_model` was first inserted inside `impl Default` (anchor matched the
  `fn` close, not the `impl` close); compiler pointed at E0407 immediately.
- `faux-1` IS a catalog row (1M window), so test agents keep huge budgets —
  no existing threshold test changes meaning.

## Verification

- `cargo test -p epsilon-core --test loop_scenarios abort`: 2 new tests pass
  in 0.11s (creation-stall abort ends Aborted with empty text; mid-stream
  abort preserves `"half an answer"`).
- `cargo test -p epsilon-core --test compaction_scenarios`: 6/6 (2 pre-existing
  + `for_model_resolves_catalog_windows`, `disabled_compaction_never_fires`,
  `known_window_overrides_fixed_threshold`,
  `model_hot_swap_refreshes_live_window` incl. the stale-window clear half).
- Full `cargo test -p epsilon-core`: 22 lib + 6 compaction + 10 loop, green.
- `cargo fmt --check`: clean. Workspace clippy: zero errors/warnings.
- `cargo run -q -p epsilon-cli -- --demo`: completes (loop touched).
