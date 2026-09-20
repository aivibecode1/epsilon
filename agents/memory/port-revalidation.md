# Autonomous audit and complete-port arc

## Authority and scope
- Owner authorizes audit first, docs/layout reconciliation, complete faithful pi
  port, then general reference improvements. Small autonomous commits, no push.
- Preserve inherited dirty source/docs; do not invent historical commits.
- .agent absent; repo skills live in .agents/skills. Read AGENTS and relevant skills.
- Use absolute paths and explicit terminal workdir; relative writes previously
  landed under a reference checkout while parallel agents were working.

## Verified baseline and boundaries
- Initial HEAD 80b32eb, 38 modified / 70 untracked. Broad TUI/contracts/config/
  registry/RPC changes remain unapproved and excluded from queue commits.
- Dirty-tree battery: all five gates, 795 passed / 0 failed / 1 ignored.
  Evidence audit/evidence/gate-revalidation-baseline.log. Not HEAD acceptance.
- Baseline cleanup removed 9.2 GiB. Never purge shared dependency/runtime caches.
- F09 find/search inherited slice: isolated repeat passed 741 tests; not committed.
  See f09-path-policy.md; do not bundle its docs or evidence into M85 closeout.

## Completed audit and first correction
- 13773b4: source-free fresh audit and three independent reference/source reviews.
- bcc1233: narrow README/AGENTS/roadmap/architecture corrections, not full rewrite.
- Keep conventional twelve-crate layout; no evidence for wholesale source moves.
- Reviews: audit/evidence/revalidation-{core,surface,structure}-review.md.
  Findings distinguish source inspection from runtime evidence; audit/09 is plan.
- 3afafd3: default followups All -> OneAtATime, matching pi agent.ts:231-232.
  RED expected 3 requests / actual 2; GREEN plus explicit All preservation 2/2.
- Code commit preceded full gate completion (process error; no owner request to
  skip gates). Subsequent isolated candidate matched both committed files.
- Candidate bcc1233 + two queue files: all five gates exited 0, 742 passed / 0
  failed / 0 ignored, 55 summaries. gate-queue-candidate.log is preserved.
- Final independent review queue-final-review.json passed with no blocking errors.
- proc_4dbf3a21f39f exited 0: cleanup removed 16,325 files / 9.4 GiB; repo,
  candidate and external targets absent; C: 60G free. No build running.
- No CLI/TUI source changed in this library slice; global binary not reinstalled.
- Durable M85 scope/evidence/limits: docs/milestones/M85-followup-default.md.

## M86-M88 round (2026-09-18)
- 36a226c (M86 compaction correctness), 35da3d7 (M87 admission reflects reality),
  31b1794 (M88 mid-batch abort), c4711a2 (round docs/evidence/memory).
- One round-B candidate, ten files byte-verified against the working tree; RED executed
  for six tests against the pre-fix code; GREEN battery gate-m86-88f: all five gates
  exit 0, 751 passed / 0 failed / 0 ignored across 55 summaries.
- Independent review (deleg_32066967): no blockers; four minor findings fixed (doc order,
  biased select, guard ordering, poisoned-history rollback) and three documented with
  reasons. Record: audit/evidence/m86-88-review.md.
- Attempts 1-5 failed first, each for a real reason now recorded: nine fmt diffs, E0716
  (temporary Arc in compact_now's select), E0004 (non-exhaustive match in admit_prompt),
  then a path-pinned AV denial on wire_fixtures (LNK1104 on a nonexistent file). Attempt
  6 passed with CARGO_PROFILE_TEST_DEBUG=1 (fresh test-artifact names) - disclosed in the
  verification log; no security configuration was touched.
- Cleanup removed the 15G shared target and both candidates; C: 59G free.
- CLI behaviour changed, so the global epsilon.exe is reinstalled from this commit before
  the round closes (proc_9dd1df275fb6); install evidence then the memory closeout.

## M89 / CORE-02 turn hooks (2026-09-18)
- 7e23e6e: pi's four `AgentLoopConfig` seams as `epsilon_contracts::turn::TurnHooks` (async,
  exactly like `ToolHooks`; defaults reproduce the pre-slice loop). ADR-002 and ADR-014 carry
  the amendments; the milestone records the pi deltas and residuals.
- Two blockers found by the first independent review are fixed and pinned: a **frozen
  replacement** (the loop now mirrors the turn's own assistant message, tool results, steering
  and nudges into an installed replacement — pi pushes onto `currentContext.messages`) and a
  **stale replacement after a committed mid-run compaction** (a history epoch is checked before
  each request; store attach/detach also bumps it, and the run's model/thinking snapshot
  survives). Hook panics are contained at the poll boundary.
- Battery `m89e` (candidate = HEAD `df5d3be` + the four slice files, byte-verified): all five
  gates exit 0, **759 passed / 0 failed / 0 ignored across 55 summaries**. No pre-fix RED is
  possible (the seams do not exist before the slice); a negative control reverts each fix in a
  disposable copy and the named tests fail as designed. Records: `audit/evidence/m89-review.md`,
  `m89-negative-control.log`, `gate-m89e.log`, `gate-m89a-attempt1.log`.
- Cleanup removed the 10G round target and every candidate; C: 59G free. No reinstall: no
  in-tree frontend installs turn hooks, so the shipped CLI is unchanged.
- Recorded follow-ups (M89 limits + next-hardening.md): unify `TurnModel` with
  `epsilon_llm::ModelRef`; re-base the usage anchor for replacement-built requests; tests for
  the drain-point mirror, the overflow-discard pop and the attach/detach epoch bump.

## M90 / SESSION-01a session-tree store (2026-09-18)
- e663bd3: pi's session-tree store operations — `branch_with_summary` with pi's two call shapes
  made one-to-one (`Some(id)` targets that entry; `None` is pi's `null`, a new root recording
  `fromId = "root"`), `reset_leaf` with "no leaf" as a real state, `BranchSummary` projected once
  on the active branch (empty summaries skipped, as pi's truthiness rule does), and
  `fork_branch_at` — a new file with fresh id, `parent_session` path link, selected ancestry
  only, gapless renumbered `seq`, source byte-identical, an `Err` leaving nothing behind.
- Battery `m90b` (candidate = HEAD `52195d5` + five files, byte-verified): all five gates exit 0,
  **766 passed / 0 failed / 0 ignored across 55 summaries**. No pre-fix RED is possible (the
  APIs did not exist); the negative control was executed by this milestone's author (projection
  and fork probes fail as designed) — records: `audit/evidence/m90-review.md`,
  `m90-final-probes.log`, `m90-negative-control-detail.log`, `gate-m90b.log`.
- Independent review: all four claims confirmed; two documentation-only blockers fixed
  pre-commit; ten findings — six fixed (empty-summary divergence, failure-path debris, five
  test-hardening items), four documented (the one non-pi-equivalent projection, pi's fork
  deferrals, two pre-existing reader-robustness gaps — both queued in next-hardening.md).
- Cleanup: the 8.8G round target and every candidate removed; C: 59G free. No reinstall:
  nothing in-tree calls the new APIs yet.
- SESSION-01b (the TUI surface) is queued and blocked on the TUI work item's uncommitted
  changes landing first.

## M91 / review hardening (2026-09-18)
- fe386d1: the M89/M90 review follow-ups — the store's parent-chain walks stop on a repeat
  (`branch_to` refuses a cycle with the existing `InvalidData` kind, no new error kind) and a
  `seq` that disagrees with line order is a chain break rather than an out-of-bounds panic, so a
  hand-corrupted file stays usable; the three M89-owed regression tests (drain-point mirror with
  an installed replacement, overflow discard, mid-run attach *and* detach invalidation); the
  usage-anchor re-base refused with a reasoned account (needs `compact.rs`'s anchor shape plus
  provenance/lifetime discipline) plus the review's related precision gap (attach/detach bump the
  epoch but leave the anchor) folded into the same open follow-up.
- Battery `m91a` (candidate = HEAD `ff7e8cc` + three code files + the doc, byte-verified): all
  five gates exit 0, **772 passed / 0 failed / 0 ignored across 55 summaries** — delta
  reconciled per binary (`loop_scenarios` 28→32, `session_store` 25→27 = exactly the six new
  tests). Evidence: `gate-m91a.log` (sha256 `711e93f6…`), `m91-probes.log` (sha256 `05cc3915…`,
  four negative-control probes on a disposable copy), `m91-review.md` (review + dispositions),
  `m91-cleanup.log`.
- Independent review: all six items confirmed; **one real documentation-only blocker fixed** —
  the earlier "nothing in-tree consumes this" claim was false (`repair_on_open` on every live
  resume and active-branch search both walk the chain, so a corrupt file now resumes silently
  shorter); the milestone now states that consequence and queues making repair report a chain
  that stopped early. The probe also showed the overflow pop is defensive, not observable, and
  the record says so instead of "pinned jointly".
- Cleanup: 7.8G round target + both copies + the child's log dirs removed; C: 59G free. No
  reinstall: every changed API keeps its signature, so the installed CLI is unchanged.
- New queue from this review (in next-hardening.md): `pop_last`/`migrated_entries` bookkeeping,
  the silent chain break's visibility, B2's store-side pop, the duplicate-ids doc trigger, and a
  hash instead of a length in the cycle test.

## M92 / RPC-02 protocol breadth (2026-09-18)
- a238ce5: pi-shaped reply envelope (id echoed per request; `type` read as an alias for `op`)
  with the first wire's `ok`/`error` keys kept, so both client generations read one frame;
  dispatch extracted from the stdin/stdout loop into a testable `dispatch_line`; ops `abort`,
  `get_state`, `set_thinking_level`; a dropped broadcast is an explicit `lagged` frame sharing
  one `LaggedNotice` type with serve's SSE event; serve gains `POST /abort` + `GET /state`
  behind one `SessionState` (types-only, in `epsilon-contracts`); deferred op families answer
  `unknown op: <name>` with the echoed id.
- Two independent reviews: round 1 found two real blockers (the false "pi's seven levels
  verbatim" claim — `x_high` vs pi's `xhigh`; the fake "spec == routes" guarantee) plus nine
  findings; both fixed, and the focused re-review closed them with no blockers and reproduced
  the four-line contracts `lib.rs` footprint. Round-2 residuals F1/F3 fixed in the milestone,
  F2 closed by probe F, F4/F5 recorded — `audit/evidence/m92-review.md`.
- Battery `m92b` (candidate = HEAD `10422ab` + eight slice files + the doc, byte-verified;
  contracts `lib.rs` staged by object hash = HEAD + 4 lines): all five gates exit 0,
  **791 passed / 0 failed / 0 ignored across 55 summaries**, delta +19 reconciled per binary
  (`epsilon_cli` +12, `epsilon_contracts` +1, `epsilon_server` +6). Evidence:
  `gate-m92b.log` (sha256 `8f25c46d…`), `m92b-probes.log` (sha256 `c05e857b…`, six probes —
  the id echo, legacy key, a state field, the `xhigh` alias and a scratch route each fail their
  own test; the pump's `Lagged` arm stays green when reverted, confirming that documented
  limit), `m92-cleanup.log`.
- Cleanup: 11 G round target + both candidates + the child's logs removed; C: 58 G free. No
  reinstall: the new ops are additive and the stdio surface's only in-tree caller does not
  parse replies.
- Still open (documented in the milestone's Limits + ADR-011): pi's command names and payload
  dialects beyond the envelope, the model-switch ops, queue modes, compaction over the wire,
  the untested `run()`/pump wiring, the `ThinkingLevel::ALL`-derived OpenAPI enum residual, and
  `pending_message_count`'s steering-only meaning.

## M93 / F18 task state when resuming (2026-09-18)
- 5ab723f: `claim_task_running` claims a resumed child `Running` in ONE lock hold before the
  background/foreground split (and the old racy guard is deleted — the claim is the refusal), so
  the second-resume refusal, the abort cascade's reach and unevictability all follow from the
  state; both run legs (detached and foreground) are contained by `contain_panic`, so a panic
  lands a truthful `Failed` instead of a permanently-claimed entry. Ledger F18 rewritten with
  corrected HEAD citations (the recorded ones anchored to `3afafd3`, +135/+458 drift) and F20
  stamped as verified by `m86-88f`/`31b1794`.
- Battery `m93b` (candidate = HEAD `fec032a` + eleven slice files, hash-verified, sibling hunks
  provably absent): all five gates exit 0, **798 passed / 0 failed / 0 ignored across 55
  summaries**, delta +7 reconciled per binary (`task_scenarios` 14→21). Evidence:
  `gate-m93b.log` (sha256 `19ecc04e…`), `m93b-probes.log` (sha256 `05d6c456…` — claim disabled →
  7 fail; containment disabled → both panic tests fail, proving the foreground wrap is
  load-bearing), `m93b-preflight.log`, `m93-review.md`, `m93-cleanup.log`.
- **New environment block:** Avira false-positives the *debug-built* `epsilon-core` lib-test
  executable (`HEUR/APC.Misc`, execution denied). The battery ran with
  `CARGO_PROFILE_TEST_DEBUG=0` after a pre-flight chose the knob that executes; no security
  configuration touched (owner/admin action).
- Independent review caught one real gap (the uncontained foreground leg) and one false claim in
  the milestone's own record (the "two live children" case is impossible — M61's store lease
  refuses the second open; the truth is the running task goes untracked), both fixed pre-commit.
- Reinstall: the global `epsilon.exe` was rebuilt from the frozen candidate (CLI/TUI-visible
  change), so it carries the verified bytes and none of the other work item's changes.
- Next: SURFACE-05 (paged reads; brief banked in `surface05-paged-reads.md`).

## Next safe actions
- **M94 landed (2026-09-18, commit `1392d37` + `d7ec973`)**: the owner-reported TUI
  unresponsiveness in cmd/pwsh is fixed and measured (a 99-char burst: ~30 chars/s before, one
  frame after), and the in-tree TUI slice is committed after three read-only review rounds that
  found and fixed two approval-safety classes (an invisible approval answerable below 60×12, and a
  last-frame guard that could answer a second parked ask). Battery `m94c`: 863/0/0/55, the whole
  +65 delta being the `epsilon_tui` suite (98 → 163). Evidence: `audit/evidence/gate-m94c.log`,
  `m94-{focused-tui,focused-cli,newtests-red}.log`, `m94-review.md`. The owner's binary was
  reinstalled from the frozen candidate.
- Queue after M94: **SURFACE-05 (paged reads)** — brief and couplings in
  `surface05-paged-reads.md` (the `read_log` coverage handoff and the pager must ship in one
  commit). Then the TUI follow-ups queued in `next-hardening.md` (run-loop ask FIFO + a notice
  when a replaced ask is denied; the `'e'`-guidance/input coupling) and SESSION-01b (the TUI
  session surface, now unblocked since the crate is committed).
- Audit initial phase is complete, not production readiness. Maintained-doc
  rewrite incomplete; keep history as history rather than rewriting it as success.
- The port's open fidelity items are closed through M92 (CORE-02 → M89, SESSION-01a → M90,
  RPC-02 → M92, F09 → closed); what remains from the roadmap is Phase 3-4 work and the
  TUI-surface half of SESSION-01.
- General reference improvements remain deferred until the port has no open fidelity
  items (audit/04-roadmap.md Phases 3-4).
- Gate before future code commits; use isolated HEAD candidates to avoid mixing
  inherited source with the verified slice. Review once, act on blockers, converge.

## G7/G8 context files and identity overlays (2026-09-20, COMMITTED as M101 `df338b7`)
- Slice owner: delegated subagent (parent `session-6572a389`); the parent committed the
  slice and re-ran the gates itself (M101's message is the authoritative record).
- **G7 decided as "load", not "stop detecting"**: `epsilon-core::prompt` gains
  `SystemOverlay` + `load_system_overlay{,_in}` (pi `discoverSystemPromptFile` /
  `discoverAppendSystemPromptFile`, `resource-loader.ts:1023-1049`), `Stable` gains
  `custom_identity` + `append`, and the CLI wires it in `build_runtime` (trust-gated,
  global pair as fallback, announced on stderr). Task children inherit it.
  `.epsilon/prompts/` stays gated and unloaded: pi's `prompts/*.md` are user-invoked slash
  templates, not system-prompt text — a separate unported feature.
- **G8**: candidate list is pi's five names, one file per directory (override first), and
  `load_project_instructions` walks the workspace **and every ancestor to the filesystem
  root**, outermost first. Precedence: global `MEMORY.md` → ancestors (root-first) →
  workspace context file → workspace `MEMORY.md` → touched-subdirectory hints. The M73 hints
  loader uses the same candidate list, so `AGENTS.override.md` works in subdirectories too.
- **G9/D-9 remainder (skill catalog): already correct, no change.** `catalog` filters
  (`skills.rs:125-138`) and is the only prompt path (`lib.rs:2211`); the `skill` tool is
  built from an explicitly filtered `Vec<LoadableSkill>` (`main.rs:1565-1578`), a type with
  no disabled flag, so the filter cannot be silently skipped. `/skills` (`skill_names`) is
  user-facing and should list disabled skills. Audit D-1 ("no skill tool") is stale: the tool
  exists (`epsilon-tools/src/skill.rs`, M97/`87c5628`).
- Files: `epsilon-core/src/{prompt.rs,lib.rs}`, `epsilon-core/tests/system_overlay.rs` (new),
  `epsilon-cli/src/{main.rs,runtime.rs,doctor.rs}`.
- Evidence: `audit/evidence/g7g8-workspace-tests.log` (975 passed / 0 failed / 1 ignored,
  57 binaries, exit 0), `g7g8-negative-control-{prompt,integration,fingerprint}.log`
  (each fix reverted in place: 6 + 2 + 1 named tests fail for the named reason; restored
  byte-identically, sha256 `52D19BD9…`).
- Limits: `cargo fmt --all --check` passed against the committed tree (the earlier failure
  was the TUI workstream's in-flight `app.rs`/`app/keys.rs`, since committed in `1e04a39`);
  global `~/.config/epsilon/AGENTS.md`-family (pi's agentDir context file) is still not
  loaded; prompt templates unported (so `.epsilon/prompts/` stays gated-but-unused, and the
  G7 row in `audit/11` says so instead of "all three load"). An M101 milestone doc and an
  ADR-008 amendment for the STABLE-tier overlay are still owed.
