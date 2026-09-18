# Independent review — CORE-02 turn-hook slice (deleg_e126f5f0, M89)

Read-only review (static only — the reviewer ran no builds) of the four-file slice:
`crates/epsilon-contracts/src/turn.rs` (new), `crates/epsilon-contracts/src/lib.rs` (+2
lines), `crates/epsilon-core/src/lib.rs`, `crates/epsilon-core/tests/loop_scenarios.rs`.
Verdicts: (a) PASS, (b) **FAIL**, (c) PASS, (d) PASS, (e) PARTIAL, (f) PASS with caveats,
(g) acceptable with an unacknowledged cost, (h) PARTIAL. **Two blockers.**

## Blockers

1. **The replacement context is frozen.** After `prepare_next_turn` installs a replacement,
   the loop never appends its own later assistant messages or tool results to it, so from
   the second turn on the provider never saw its own output — a direct divergence from pi,
   which pushes every assistant message and tool result onto `currentContext.messages`
   (`agent-loop.ts:186,219,321,337,350-352`). `turn_context` had only three touch points
   (declaration, the steering append, the assignment). The slice's own test froze the
   divergence in (`seen_context_len == vec![3,1,1,1]`; pi-faithful appending yields
   `[3,1,2,2]`).
   **Fixed**: the loop appends its own messages (assistant, tool results, steering, nudges)
   to an active replacement; the test expectation is corrected to the pi-faithful lengths
   and a post-tool-turn assertion was added.

2. **The replacement goes stale against the store after a compaction.** A mid-run pressure
   compaction or overflow recovery commits a new projection, replaces runtime history and
   clears the anchor — yet the pre-compaction replacement kept winning for the rest of the
   run, so the recovery's shrink was ignored, a store-backed run no longer read its provider
   view from the store (the M86/ADR-007 invariant), and an overflowed request could be
   re-sent unchanged. Related: `covered` was taken from the replacement's length while
   `anchored_tokens` slices *runtime* history, mis-indexing the M64 usage anchor.
   **Fixed**: the replacement is invalidated when a compaction commits or history is
   replaced mid-run, the anchor is not indexed through a replacement, and a regression
   asserts the post-compaction request is not the frozen replacement.

## Findings and dispositions

| Finding | Disposition |
|---|---|
| A panicking hook is not contained; an unwind skips the cleanup that resets `running`, after which every later run returns `AlreadyRunning` (the provider has `PanicGuard` for exactly this) | **Fixed** — hook calls are contained (catch_unwind + safe default) with a regression covering both the run's outcome and the following run |
| The seams must be async: contracts already depends on `async-trait` and `ToolHooks` is an `#[async_trait]` trait, so the slice's "async would violate ADR-014" justification was factually wrong — and a sync seam cannot host the provider-backed hosts its own rustdoc advertises | **Fixed** — all four seams are `#[async_trait]` with the same no-op defaults, matching the `ToolHooks` precedent; the wrong justification is removed |
| Test 1's "the session model never changed" had no assertion (a leak via `set_model_id` would pass) and the update was never cleared (applied-once vs re-applied indistinguishable) | **Fixed** — a second run asserts the session model/thinking are unchanged, and the hook counts its calls per turn |
| Test 2's claim "no steering drain" was untested; nothing asserted the follow-up stays queued | **Fixed** — a steering message is queued during the turn, asserted not drained, and the follow-up is asserted still queued |
| Test 3 never asserted its injected messages stayed out of `history()` (the ADR-007 "hooks are readers" claim) | **Fixed** — asserted |
| `context_view()` silently fell back on a poisoned lock or `model_view()` error while the request path turns the same conditions into `RunError::Internal`; duplicated store-read logic can drift | **Fixed** — one shared helper; when the view cannot be read the hook call is skipped and the request path's behaviour is unchanged |
| Undisclosed pi deltas: `TurnUpdate.context` replaces message context only (pi's snapshot can also swap `systemPrompt`/`tools`), `NextTurnContext.new_messages` excludes the run's own prompt (pi's starts with the prompts), hooks cannot observe cancellation | **Documented** in the trait rustdoc and the milestone limits |
| `context_view()` clones once per turn end even with no hooks; the trait defaults clone the context twice per request where the loop cloned once | **Documented** — cost only, not correctness; a cheaper guard (a flag set when a host installs hooks) is a recorded follow-up |
| Behaviour now lives in contracts (the visibility filter moved into the trait default); AGENTS.md's architecture law requires a narrowly defined policy amendment rather than silent drift | **Recorded** — ADR-014 carries a narrow amendment (stateless, dependency-free seam defaults) in the same change |

Reviewer verdicts kept for the record: (c) the `break 'outer` stop path skips only what pi
skips (steering drain, follow-up drain, stuck notice) and persists everything else before
the hook block; (d) per-run overrides apply to every later request and never touch session
state; (f) the contracts change is strictly additive (exactly two lines) with no new
dependencies, no `unsafe`, no lint suppressions, an object-safe `Send + Sync` trait, and no
panic risk beyond the (now contained) hook bodies; (g) the per-turn clone is O(transcript)
and dwarfed by the provider round trip.

Full reviewer report: the delegation transcript (`deleg_e126f5f0`) and this session.
