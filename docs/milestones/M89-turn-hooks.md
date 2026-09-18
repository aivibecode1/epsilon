# M89 — turn hooks (pi `AgentLoopConfig` parity)

## What

pi exposes four per-turn seams on its loop config (`packages/agent/src/types.ts:149–293`):
`transformContext`, `convertToLlm`, `prepareNextTurn` and `shouldStopAfterTurn`. Epsilon
had none of them — only tool-level hooks — so a host could not prune or inject context,
could not swap the run's context/model/thinking for the remaining turns, and could not stop
a run gracefully after a turn. ADR-002 names `prepare_next_turn` and `should_stop_after_turn`
as one-to-one obligations; this milestone fulfils that commitment.

The slice:

- `epsilon-contracts::turn` — an object-safe `TurnHooks` trait whose four methods carry pi's
  defaults as no-op implementations (`transform_context` passes messages through,
  `convert_to_llm` applies the historical harness-visibility filter, `prepare_next_turn`
  returns `None`, `should_stop_after_turn` returns `false`), plus `NextTurnContext`,
  `TurnUpdate { context, model, thinking }`, `TurnModel` and `NoopTurnHooks`.
- `epsilon-core` — `turn_hooks` on the agent (`RwLock<Arc<dyn TurnHooks>>`, default no-op)
  with a setter; the loop calls `prepare_next_turn` immediately after `TurnEnd` is emitted,
  applies the update, evaluates `should_stop_after_turn` on the *replaced* context, and —
  when it says stop — ends the run without draining steering or follow-ups and without the
  stuck-run notice (pi `agent-loop.ts:224–257`). `stream_one_turn` takes the replacement
  context (it wins over the store's shadowed view and over runtime history) and the per-run
  model/thinking overrides; `transform_context` then `convert_to_llm` run when the provider
  request is built.

## Key decisions

- **Hooks are async.** pi's hooks are sync-or-Promise; an async seam (`#[async_trait]`,
  exactly like `ToolHooks` in the same crate) is how Epsilon satisfies the Promise half, so a
  host that needs I/O — a summarizer, an extension runner — owns no runtime of its own: the
  loop awaits each call at the point pi does. The contract's "must not panic" rule is enforced
  mechanically at the seam (a panicking hook falls back to the no-op default), so a misbehaving
  host cannot unwind the run.
- **`TurnModel`, not `ModelRef`.** Contracts cannot reuse `epsilon_llm::ModelRef` (contracts
  depends on no Epsilon crate), and carrying a second type with the same name for the same
  concept invites exactly the confusion this repository's docs exist to prevent. Unifying
  the two into contracts — with `epsilon_llm` re-exporting — is recorded as a follow-up.
- **Hooks are readers.** They can neither mutate runtime history nor write the durable store
  (ADR-007); a replacement context is run-scoped state held by the loop, and every message the
  loop adds to the transcript during the run — drained steering and nudges, the turn's
  assistant message, its tool results — is appended to that replacement (pi appends to
  `currentContext.messages`), so a swap cannot drop them and the provider keeps seeing its own
  output. A committed mid-run compaction replaces the history the replacement was built on, so
  the replacement is dropped and the next request rebuilds from the current source.
- **Defaults reproduce the pre-slice loop exactly.** With no hook installed the observable
  behaviour — event order, stuck notice, queue draining, usage anchoring — is unchanged.
  Queues and `LoopLimits` keep their semantics (ADR-002/ADR-006): a hook can stop a run, not
  reinterpret a queue.

## Gotchas

- The loop's hook context is the *provider-bound* view (the store's shadowed projection when
  a store is attached, else runtime history), so a hook sees exactly what the next request
  would start from.
- `crates/epsilon-contracts/src/lib.rs` in the working tree carries another work item's
  uncommitted re-exports; this milestone's commit adds only its own two lines to that file
  (`pub mod turn;` and the `turn` re-export). The verified candidate was built from exactly
  that two-line version, so the gate evidence covers the committed bytes rather than the
  working tree.
- The first battery attempt (`m89a`) failed `fmt --check` on that constructed file: rustfmt
  requires the `pub use` groups to stay sorted, so `pub use turn::{…}` must follow the `tool`
  block instead of `stream` (`audit/evidence/gate-m89a-attempt1.log`). The working tree had
  the placement right; only the hand-built commit version was wrong — which is exactly what a
  gate over a frozen candidate exists to catch.

## Verification

Round battery `m89e` on the frozen candidate (`epsilon-m89e-candidate` = HEAD `df5d3be` +
the four slice files, byte-verified; the shared `crates/epsilon-contracts/src/lib.rs` in the
candidate is the committed two-line version, sha256 `ff99c8ab…`): **all five gates exit 0 —
759 passed, 0 failed, 0 ignored across 55 summaries** (fmt 1s, clippy 58s, test 350s, demo 88s,
real-check 13s; `audit/evidence/gate-m89e.log`, sha256 `335c2d98…`). The test profile was built
with `CARGO_PROFILE_TEST_DEBUG=1` and the run used an external target dir with
`CARGO_BUILD_JOBS=1` (the AV-dodge and the machine's one-build rule; neither affects
assertions). The eight-test delta over the previous round reconciles exactly: six in
`loop_scenarios` (three slice tests kept, three added) and two `turn::tests` in contracts.

This slice has no pre-fix RED: the seams do not exist before it, so the new tests cannot
compile against the pre-fix tree. The equivalent evidence is a **negative control** — the fix
reverted inside a disposable copy of the candidate, so the tests must fail for the reason they
name. Executed: `audit/evidence/m89-negative-control.log` (both probes failed as designed;
the implementer's broader control failed four tests, e.g. `left: [3, 1, 1, 1] right: [3, 1,
2, 2]`, and the restored `lib.rs` was verified byte-identical at sha256 `66bd8f04…`).

Review history: the first independent review found **two blockers** (a frozen replacement
context; a replacement going stale after a committed mid-run compaction) plus findings — all
fixed, with the dispositions in `audit/evidence/m89-review.md`. The focused re-review returned
**no open blockers** and confirmed each closure with line-level evidence; its residuals are
recorded under Limits below. Attempt history: `gate-m89a-attempt1.log` (fmt caught the
unsorted `pub use` in the hand-built commit version — the working tree had it right), `m89d`
stopped to apply the attach/detach epoch fix.

No reinstall: no in-tree frontend installs turn hooks (grep over `crates/` finds
`set_turn_hooks`/`TurnHooks` only in contracts, core and tests), so the shipped CLI behaves
exactly as before.

## Limits

- Hooks are async (decision above), but cannot observe cancellation: no token is passed, so a
  long hook runs to completion (documented in the trait).
- The hook seam shares ONE provider-bound read with the request builder (`try_context_view`),
  cloning it once per turn end. When that read fails (poisoned lock, failed model view) the
  turn's hook calls are skipped and the request that follows reports the failure itself
  (`RunError::Internal`) — a hook is never handed a view the request would refuse. A cheaper
  guard (a flag set when a host installs hooks) is a possible follow-up, not a correctness
  issue.
- `failure_message`/`aborted_message` still stamp the session's model id rather than a
  per-run override — pre-existing behaviour, untouched here.
- Recorded test gaps (re-review, `audit/evidence/m89-review.md`): the drain-point mirror and
  the overflow-discard pop are correct by inspection but not exercised *with an active
  replacement*; the containment test cannot by itself distinguish "contained" from "never
  invoked", and the compaction-invalidation test cannot distinguish "invalidated" from "never
  installed" — both halves are pinned elsewhere in the suite (installation and per-turn
  invocation are asserted by other tests). **The drain-point mirror, the overflow-discard pop
  and the attach/detach invalidation are now exercised directly by
  [M91](M91-review-hardening.md);** the containment and invalidation variants still rely on the
  rest of the suite.
- Containment bounds: it wraps `poll`, so hook bodies are covered; a panic while *constructing*
  a hook future, a panic inside a task the hook itself spawned, and a hypothetical
  `panic=abort` profile (none is declared in the workspace) are outside it.
- Precision, not safety: a request built from a replacement clears the usage anchor instead of
  re-basing it to the runtime history length, so the pressure trigger falls back to the
  whole-history estimate for the rest of such a run — it can compact later than an anchored
  estimate would. Re-basing is a recorded follow-up.
- The epoch is also bumped when the store is attached or detached mid-run (a source swap must
  invalidate a run-scoped replacement); that consistency fix is correct by construction and is
  not regression-tested, because no in-tree caller attaches a store mid-run today.
- Evidence caveat: the pre-fix (sync-seam) slice source is not preserved, so "every seam is
  still called where the pre-slice version called it" rests on the review record and the
  one-line-neutralised probe copies rather than on a preserved diff.
