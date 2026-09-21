# Documentation

For users and contributors finding current guides, design intent and historical records.
The implementation is the authority when documents disagree; older design and milestone
claims are not proof that a capability works today.

## Product documentation

| File | What lives there | Read it when |
|---|---|---|
| [`../README.md`](../README.md) | the entry point: what Epsilon is, install, quick start | first |
| [`user/`](user) | product guides: install, configuration, auth, permissions, sessions, troubleshooting | you are using or configuring Epsilon |
| [`../CHANGELOG.md`](../CHANGELOG.md) | user-visible changes, milestone by milestone, newest first | you want to know what changed |

## Contributor documentation

Agent recovery state lives in [`agents/memory/`](../agents/memory/README.md):
curated open-work notes, evidence pointers and safe resumption steps, not product docs.

| Folder / file | What lives there | Read it when |
|---|---|---|
| [`../AGENTS.md`](../AGENTS.md) | the rules for anyone writing code here: gates, conventions, disk hygiene | before your first change |
| [`design/`](design) | **how and why**: the architecture tour, the ADR log, the skills design | you are changing behaviour |
| [`qa/`](qa) | design conversations turned into notes: why Rust, the brick methodology, the harness primer, TUI keymaps, OAuth client ids, provider extension points, the repository layout | you need the convention behind a choice |
| [`plans/README.md`](plans/README.md) | historical plans and their recorded statuses, not fresh authorization | you want to trace earlier scope |
| [`../audit/`](../audit) | the dated assessment: usability review, defect ledger F00–F19, port plan, verification log, reference comparison, roadmap | you want to know what is broken or what to do next |

## Historical record

| Folder / file | What lives there | Read it when |
|---|---|---|
| [`history.md`](history.md) | the milestone timeline and acceptance notes | you want the project's story in one table |
| [`milestones/`](milestones) | one dev log per milestone: what was built, decisions, bugs found, verification output | you are touching an area and want its history |
| [`research/`](research) | source-level studies of the reference harnesses and the gap analyses (dated evidence, not current status) | you are deciding what to build next |
| [`handoff/`](handoff) | archived handoff prompts from earlier milestones (historical, not active tasks) | you are tracing how a slice of work was delegated |
| [`glossary.md`](glossary.md) | the vocabulary — harness, turn, tool call, compaction, lease, … | you hit an unfamiliar term |
| `../references/` | the four reference harnesses' sources (not committed; research input) | you want to check a port against its origin |

## Reading paths

**New to coding agents?** [`qa/06-agent-harness-primer.md`](qa/06-agent-harness-primer.md) with
[`glossary.md`](glossary.md) open beside it, then the
[architecture guide](design/epsilon-architecture.md).

**New to the codebase?** Root README → [`design/epsilon-architecture.md`](design/epsilon-architecture.md)
→ the crate that owns what you are touching → its milestone log in `milestones/` → the tests in
that crate.

**Reviewing a change?** `AGENTS.md` (gates), then the ADR log
([`design/epsilon-decisions.md`](design/epsilon-decisions.md)) for the behaviour it changes,
then the milestone log that introduces it.

**Deciding what to build next?** [`../audit/02-defect-ledger.md`](../audit/02-defect-ledger.md),
[`../audit/03-port-plan.md`](../audit/03-port-plan.md) and
[`../audit/04-roadmap.md`](../audit/04-roadmap.md), then the relevant study in `research/`.

## Current hardening work

Newest first. Each entry is a two-line pointer; the linked log has the reasoning, the
decisions and what the slice could not do. The logs marked *(reconstructed)* were
written after the fact from the commits, because those milestones landed without one —
they say so themselves, and their `## Verification` sections name what could not be
recovered rather than inventing it.

- [M109 — `@` completion stops offering files the tools would refuse](milestones/M109-at-file-ignore.md):
  the `@` list had its own walker and its own hardcoded skips, so it disagreed with every
  read tool about which files exist. It now shares `epsilon_core::ignore`, and the slice
  found that the hidden-directory "escape hatch" both walks promised was dead logic —
  `is_ignored` cannot tell "re-included" from "never mentioned", so both now say so.
- [M108 — a complete line this build cannot read is refused, not deleted](milestones/M108-unknown-final-line.md):
  the session store dropped a final line it could not parse. That is right for a crash
  fragment and wrong for a *complete* line in an unknown shape — the first is debris, the
  second is the user's newest turn or a newer Epsilon's entry. Position used to decide
  whether data survived; `is_whole_json` decides now, and a refusal leaves the file
  byte-identical.
- [M107 — deleting the second footer a redesign left behind](milestones/M107-dead-footer.md):
  `epsilon-tui/src/footer.rs` was a whole superseded footer with one live function, and
  `clippy -D warnings` is structurally blind to it because `pub` items in a library crate
  are exempt from `dead_code`. It carried a second `format_tokens` with different rounding
  and a second copy of the cache-formula bug below.
- [M106 — the cache-hit gauge measures one prompt, not a session](milestones/M106-cache-hit-accuracy.md):
  the header divided the session-total cache reads by the latest prompt size, so the
  figure inflated every turn and pinned at `cache 100%`. The test named for the right
  property passed because it modelled the gauge wrongly — the numerator now comes from
  the same call as the denominator.
- [M104 — a restore verifies the file before overwriting it](milestones/M104-restore-verifies-the-file.md)
  *(reconstructed)*: reverting a checkpoint over a file the user edited after the snapshot
  silently discarded their work. Restore now refuses when the bytes on disk are not the
  bytes that were captured, and says which file and why.
- [M103 — the extension host protocol](milestones/M103-extension-host-protocol.md)
  *(reconstructed)*: `epsilon-ext` went from 5 tests to 30 by driving the real fixture
  process instead of a mock, which found a live defect — the frame cap only bounded a line
  whose newline had not arrived yet, so one oversized line came back as valid.
- [M102 — a summary cut off by the output limit is not committed](milestones/M102-length-stopped-summary.md)
  *(reconstructed)*: a compaction summary that hit the model's output ceiling is a truncated
  summary, not a summary; committing it replaced the transcript with a mutilated version of it.
- [M101 — the context files the harness was already detecting](milestones/M101-context-files.md):
  three ways the harness told the user something untrue. `SYSTEM.md` / `APPEND_SYSTEM.md` raised a
  trust prompt and never loaded; `AGENTS.override.md` was missing from the candidate list *and*
  `doctor` kept a second, shorter list of its own; ancestor context files were never read. All
  three fixed, with the important half being semantics rather than names — **one context file per
  directory, first existing wins**, because adding the override to the old loop would have made it
  additive and it would have stopped overriding. The audit was wrong in two places and this
  milestone records both: `.epsilon/prompts/` is slash templates, not system-prompt text, so it
  stays gated and unloaded; and the `skill` tool it said did not exist has existed since M97.
  Negative control: each fix reverted in place, 9 named tests fail. Landed df338b7.
- [M100 — a revert that cannot restore a file must not delete it](milestones/M100-restore-must-not-delete.md)
  *(reconstructed)*: `restore` read "no captured bytes" as "the file did not exist, so remove
  it", which is also what a *skipped* capture looks like. Deleting a file is the worst outcome a
  revert path can produce, and the entry already carried the field that distinguishes the cases.
- [M99 — `.gitignore` is honoured instead of merely promised](milestones/M99-gitignore.md)
  *(reconstructed)*: `find`'s description claimed it and nothing read one. The matcher lives in
  `epsilon-core::ignore` so the tools and the TUI share one answer — one half of which is still
  not true; see the `@`-file note in [`../audit/11-current-assessment.md`](../audit/11-current-assessment.md).
- [M98 — the six-second stall that read as a hang](milestones/M98-windows-shell-stall.md)
  *(reconstructed)*: every Windows `bash` call ran `powershell -Command`, which loads the user's
  interactive profile first — 6.2 s for a one-word `echo` against 1.06 s with `-NoProfile`. From
  inside the TUI that is indistinguishable from a deadlock, and it is what the owner's "it
  freezes when I send a prompt" actually was.
- [M97 — the `skill` tool the contract always promised](milestones/M97-skill-tool.md)
  *(reconstructed)*: the agent contract told the model to load a skill body "via the `skill`
  tool" and no such tool existed, so the model could see a skill and never open it.
- [M96 — three port gaps the fidelity audit proved](milestones/M96-compaction-and-overflow-fidelity.md)
  *(reconstructed)*: the overflow classifier matched a spaced spelling no provider sends, so the
  compaction-and-retry branch never fired for a real overflow; and the compaction prompt asserted
  a template it never sent, so every summary came back in a different shape.
- [M95 — paged reads](milestones/M95-paged-reads.md): SURFACE-05 — `read` finally pages the way
  its own description promised: pi's offset/limit window over the whole file (`read.ts:275-323`)
  with pi's continuation strings, past-end error, empty-file Ok and the `sed` fallback for an
  over-cap single line, streamed with whole-line boundaries (no U+FFFD cut, no partial line) and
  faithful real line counts. The same change moves the stale-read guard to a coverage model so a
  page of a large file can never grant a wholesale overwrite (F01), and `grep`'s 256 KiB cap is
  announced instead of silent. Focused runs green with a 7-test pre-fix RED; the read-only review
  returned one blocker in the same F01 class (an over-cap line *mid-page* replaced the page and
  recorded `capped`), now fixed — the over-cap branch is gated on the page's first line, with its
  own RED-screened test. Battery `m95b`: 876 passed / 0 failed / 0 ignored; landed 27cd677.
- [M94 — TUI responsiveness](milestones/M94-tui-responsiveness.md): the owner-reported
  unresponsiveness in cmd/pwsh, reproduced in a real console (~30 chars/s echo) and fixed in the
  run loop (drain input before drawing, coalesce agent events, ~30 fps ceiling with a due-frame
  wake, input/approval priority) — measured at ~one frame per burst afterwards; lands the
  in-tree TUI enhancement together with its `config.rs` dependency and the M36 pins.
- [M93 — task state when resuming](milestones/M93-task-state-on-resume.md): F18 — a resumed
  child is claimed `Running` in one lock hold before the background/foreground split, so the
  refusal, the abort cascade and eviction all see it; both run bodies are contained so a panic
  still lands a terminal record. Verified: battery `m93b` 798 passed / 0 failed, two
  negative-control probes (disabling the claim reds seven tests; disabling the containment reds
  both panic tests).

- [M92 — RPC protocol breadth](milestones/M92-rpc-protocol-breadth.md): the RPC-02 slice —
  correlated replies in pi's envelope while keeping the first wire's `ok`/`error` keys, an
  extracted testable dispatch path, `abort`/`get_state`/`set_thinking_level` ops, a lagged frame
  matching serve mode's, `Agent::model_id`, and `POST /abort` + `GET /state` over the shared
  `SessionState` struct with `request_id` echo and OpenAPI registration; verification pending.

- [M91 — review hardening](milestones/M91-review-hardening.md): the follow-ups the M89/M90
  reviews recorded as owed — session-reader guards against hand-corruption (a parent cycle and a
  `seq`/line-order mismatch, neither of which may hang or panic), the three missing CORE-02 loop
  tests (drain-point mirror, overflow-discard pop, mid-run store attach/detach), and a refused-
  with-reason anchor re-base.

- [M90 — session tree](milestones/M90-session-tree.md): the store half of pi's session-tree
  workflows — branch summaries with pi's exact target shapes, explicit no-leaf root navigation,
  `BranchSummary` projected on the active branch, and a fork that writes only the selected
  ancestry with fresh identity; the TUI surface is queued as SESSION-01b.

- [M89 — turn hooks](milestones/M89-turn-hooks.md): pi's `AgentLoopConfig` seams
  (`transform_context`, `convert_to_llm`, `prepare_next_turn`, `should_stop_after_turn`) exist
  as `TurnHooks`; a stop ends the run without draining queues, and a replacement context tracks
  the transcript and yields to a committed compaction.

- [M88 — mid-batch abort](milestones/M88-mid-batch-abort.md): a cancelled run refuses
  batched calls it has not started; one checkpoint covers both batch plans.

- [M87 — admission that reflects reality](milestones/M87-rpc-admission.md): RPC and HTTP
  prompts answer with what actually happened (real rejection, atomic run slot, logged
  in-band refusals).

- [M86 — compaction correctness](milestones/M86-compaction-correctness.md): the durable
  projection replaces the superseded one, and summaries commit only when usable.

- [M85 — follow-up default](milestones/M85-followup-default.md): one-at-a-time
  default restored; explicit All preserved; verification and coverage limits.

- [Fresh audit and port-first plan](../audit/09-revalidation.md): named baseline,
  verified gate evidence, unresolved fidelity gaps and independent review reports.

- [M84 — find directory resource](milestones/M84-find-directory-resource.md):
  narrow scheduling correction; F09 containment remains open.

- [M83 — baseline integration](milestones/M83-baseline-integration.md): isolated
  candidate verification, explicit selection and preserved baseline risks.

- [M81 — observation ownership](milestones/M81-observation-ownership.md): F02 implementation,
  test evidence and remaining verification/alias limits.
- [M82 — plan-mode enforcement](milestones/M82-plan-mode-enforcement.md):
  F10 implementation, red/green evidence and scope limits.

## Conventions

- `qa/` files are numbered sequentially and never renumbered; add the next number.
- `milestones/M<N>-<slug>.md` is written once, when the milestone lands, and is a record — it
  is not edited afterwards except to correct a factual error.
- Current product facts have **one owner**: the guides in `user/`. Milestones, research and
  handoffs are historical records — their status claims describe the moment they were written,
  not today. Current defects and verification evidence live in `../audit/`.

Product reference material lives in `user/`. Existing moves, instruction corrections,
verification limits and the Phase 2 review handoff are recorded in
[the restructure record](../audit/08-restructure-record.md). Those moves predate this
phase; the verification log is not their move map.
