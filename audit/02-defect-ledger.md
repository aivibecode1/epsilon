# 02 — Gap analysis and defect ledger

**Audience:** engineers fixing reliability and safety, and the owner selecting scope.
**Snapshot:** 2026-09-16 working tree over `e200815`. **Update 2026-09-17:** F00 and F01 were corrected and verified in Phase 3 batch A — see [05](05-verification-log.md#phase-3-batch-a--f00-verification-2026-09-17); the row below describes the pre-fix snapshot and stays for traceability.

**Severity:** P0 = potential destructive loss or fundamental execution-boundary failure; P1 = blocking reliability/correctness; P2 = workflow or maintainability. Priority is not an exploitability rating.
**Evidence:** E = executed here; S = source-verified; I = conditional consequence/inference. Unless marked E, a proposed regression below was **not run**. Estimates include tests for one experienced Rust engineer; see 04 for dependency order.

**Update 2026-09-23 (status sweep):** the cells an earlier audit left blank had become a false bill of
health — this table still listed F04, F06, F07, F08, F15 and F17 as open long after milestone
M86–M133 closed them, and because the guides point here as the owner of "current defects", each
un-updated row kept the stale claim alive in every document citing it. Every row now carries a
status, and the six rewritten findings say what closed them, by which milestone, and what was left
behind. Nothing else in this file changed: the other rows were not re-verified in this pass and are
marked accordingly.

**Status legend**
- **OPEN** — the defect is in the code now. The row's own section carries the pre-fix source trace
  and the unmet acceptance bar.
- **CLOSED** — a landed milestone fixed the finding, and its section names the milestone, the
  regression that pins it, and the evidence. Status is about the code, not about how comfortable
  the evidence is; where a claim rests on a commit message rather than a preserved battery, the
  section says so.
- **CLOSED†** — fixed *and* still carrying a named residual: a path no test exercises, a limit the
  fix documented, or an item from the finding's own acceptance list that was deliberately not
  claimed. Read the section for the residual before treating the row as settled.
- **FIXED** — code fixed, but the verification the finding demanded (a frozen-candidate battery, a
  negative control, or a named regression) is still owed. **FIXED is not CLOSED.**
- **PARTIAL** — some sub-claims in the row are resolved and others are still true; the section says
  which are which.

A milestone log or the [timeline](../docs/history.md) row is the citation for a closure, not this
table: a reader who doubts a status should open the milestone named in the row. Conversely, **a row
must not be marked CLOSED on the strength of this table's own say-so** — that is the failure this
update exists to correct.

## Ranked inventory

| ID | Status | Priority | Finding | Evidence | Effort / change risk |
|---|---|---|---|---|---|
| F00 | FIXED | P1, prerequisite | Current verification baseline fails — **fixed 2026-09-17, gates green** (05) | E | 0.5–1 day / low |
| F01 | CLOSED | P0 | Editing a capped buffer overwrites the complete file — **fixed 2026-09-17** (05); the read-coverage rule that licensed the wholesale overwrite was made structural in **M95**, and a second route to the same defect (an `edit` promoting a partial read) in **M135** | S, high confidence | 0.5–1 day / low |
| F02 | CLOSED† | P0 | Read observations are shared between agents — **M81** gave every agent its own observation token and invalidated a sibling's cached observation; residual: F09 alias identity (see the row) | S, high | 2–4 days / medium |
| F03 | OPEN | P1 | SSE buffering depends on HTTP chunk boundaries; EOF can claim success | S, high | 3–5 days / medium |
| F04 | CLOSED† | P1 | Repeated durable compaction accumulates old summaries/tails — **M86** replaces the superseded projection; **M90** ran the alternate-branch variant | E (M86 battery + RED) | 2–4 days / medium |
| F05 | OPEN | P1 | Failed shell checks are recorded as passing verification | S, high | 1–3 days / medium |
| F06 | CLOSED† | P1 | Failed or cancelled summarization can replace valid context — **M86** commits only on a non-failure stop, usable text and a durable append | E (M86 RED + battery) | 2–3 days / medium |
| F07 | CLOSED† | P0 | Persistence failures are discarded while model/store views diverge — **M122** (turn loop), **M124** (fault seam), **M133** (serve) | E (M124/M133 tests) | 4–7 days / high |
| F08 | CLOSED | P0 | Complete unsupported final entries are treated as torn writes — **M108** refuses them, **M113**/M116 keep a foreign entry instead of costing the session | E (M108 battery + negative control) | 1–3 days / medium |
| F09 | OPEN | P0 | Native file tools have lexical, not resolved, containment — **M84** bounded one consumer; containment itself still open | S; exploitability filesystem-dependent | 4–8 days plus race-resistant design / high |
| F10 | CLOSED | P1 | Plan-mode shell denials do not match execution subjects — **M82** (bash/shell denied outright, plan layer composed over project rules) | E (M82 red/green + battery) | 1–3 days / medium |
| F11 | OPEN | P1 | Process deadlines/cleanup do not cover the whole operation | S; descendant survival/hangs inferred | 3–6 days / medium |
| F12 | PARTIAL | P1 | Output/input caps apply after unlimited allocation — the image path reads a bounded head (M95) and `webfetch` refuses on a declared length; the body is still buffered whole before its size check | S, high | 3–5 days / medium |
| F13 | CLOSED† | P1 | RPC/HTTP can acknowledge work that is rejected — **M87** both surfaces, **M92** correlation/abort; residual: the untested RPC pump (see the row) | E (M87/M92) | 2–4 days / medium |
| F14 | OPEN | P1 | LSP writes/framing/versioning undermine bounded fresh diagnostics | S, high | 3–5 days / medium |
| F15 | CLOSED† | P0 before exposing restore | Checkpoint restoration conflates skipped capture with creation — **M100** refuses a skipped capture, **M104** refuses to clobber a file that moved since capture; residual: nothing calls `restore`, so no user-facing revert exists | E (M100/M104 tests; no battery over those bytes) | 3–6 days / high |
| F16 | OPEN | P1 | OAuth refresh orchestration differs from login/fallback paths — **still true at HEAD (re-checked 2026-09-23)**; see the row | S, high for control flow | 3–6 days / high |
| F17 | CLOSED† | P1 | Locally aborted partial replies are not durably committed — **M86** gives the abort arm the same lifecycle as a completed turn | E (M86 RED + battery) | 1–2 days / low |
| F18 | FIXED | P1 | Resumed child task is not marked running before dispatch — **fixed 2026-09-18 (M93); frozen-candidate battery owed** (see the F18 section) | S, high | 1–2 days / low |
| F19 | PARTIAL | P2 | Product/docs/parity claims exceed actual workflows — regressions fixed in **ffff7e2**; other claims not swept here (see the row) | S plus E baseline | 3–5 days docs; workflow work separate / low |
| F20 | CLOSED | P1 | Abort did not stop new calls starting inside the running batch — **M88** checks the run token in `run_one`, before any call starts | E (M88 battery + the pre-fix RED in the same log) | 1–2 days / low |

## F00 — Restore the baseline without weakening its assertions

**Executed:** fmt=0, strict Clippy=101, workspace tests=101, demo=0, real-check=0. Full evidence in [05](05-verification-log.md).

Clippy reports unused `Hash as _` at `crates/epsilon-tools/src/checkpoint.rs:340` and collapsible `if` at `:312`. The failing test is `checkpoint::tests::the_store_evicts_old_blobs_when_it_grows_past_its_ceiling`, assertion at `:471`.

**Source diagnosis:** `record()` already invokes eviction (`:211–212`). The test repeatedly records, then expects another eviction call to remove something (`:462–471`). That expectation is not evidence that eviction itself fails. Keep the invariant—bounded eligible storage and newest-preimage preservation—and test it at the correct operation boundary; do not simply delete the assertion or disable Clippy.

**Acceptance:** all five gates pass; regression checks storage after each capture and keeps the newest recoverable blob. If protected latest blobs alone exceed the configured ceiling, document/test whether the cap is soft or capture must refuse.

## F01 — Never write a display-limited source buffer back as a whole file

**Evidence:** `crates/epsilon-tools/src/util.rs:171–194` drops the capped flag and reads at most `MAX_READ_BYTES`; `crates/epsilon-tools/src/files.rs:482–501` edits that prefix and writes it over the original. The batch path does the same at `:620–635`.

**Trace:** file exceeds the 256 KiB text cap → replacement matches in prefix → complete-file write contains only the modified prefix → unread suffix is lost. Lossy UTF-8 conversion can also rewrite unrelated bytes. Existing fuzzy/CRLF tests do not prove preservation beyond the reader cap.

**Minimum fix:** refuse capped or invalid-UTF-8 edit input before mutation. A separate bounded full-file edit reader may be added later if required. Do not remove the model-visible read cap to fix write correctness.

**Acceptance:** single and batch edits against a file with a unique suffix beyond the cap either preserve every unrelated byte or refuse with the original byte-identical. Cover boundary size, multi-byte split and invalid UTF-8. Checkpoint existence is not an excuse for destructive writeback.

**A second route to the same defect, closed in M135.** M95 replaced "this view was capped" with a
coverage record and fixed the route the trace above describes. It left one door open and wrote it
into its own `Limits`: a successful `edit` called `read_log::record_write`, which *sets* `complete`,
so editing one line of a file the model had read 20 lines of granted a whole-file licence — the same
destruction, reached without a large file at all. The narrow read that reaches it is the caller's
`limit`, not the byte cap. M135 adds `read_log::record_edit`, which carries `read`/`complete`/`capped`
and discards the line arithmetic, and pins the route with a behavioural test through the real tools
plus a negative control that reproduces the overwrite (`Ok("overwrote book.txt (19 bytes)")`) with both
`edit` call sites reverted — output in
[evidence/m135-negative-control.txt](evidence/m135-negative-control.txt), run on the final bytes, and
named here rather than merely asserted, per this ledger's own rule that a CLOSED row rests on executed
evidence. Battery `m135b` (1185 tests passed, 0 failed). The independent review of the slice
([evidence/m135-review.md](evidence/m135-review.md)) found that control asserted here and in three
other documents while no log existed, and also found the pre-existing `capped` hole it would not close
in this slice: a read that stops on an over-cap line shows the model **no lines at all** and the guard
still licenses replacing the file. That is now pinned by
`the_capped_exemption_is_a_hole_the_read_tool_cannot_cloze` rather than left as an unmentioned trade —
it stays open as a residual, and the honest long-term fix is a read that truncates a long line
visibly, which would let the exemption be dropped. Record:
[M135](../docs/milestones/M135-edit-does-not-license-a-whole-file-write.md).

## F02 — Track what each agent actually observed

**Working-tree update:** observer ownership and sibling invalidation are implemented
and statically reviewed in [M81](../docs/milestones/M81-observation-ownership.md).
The three reviewer-identified acceptance gaps are now covered by tests: a failed
sibling edit leaves the partial reader's full-overwrite refusal intact; single and
batch edits invalidate sibling observations only for the changed path; and the CLI
composition suite [observation_ownership.rs](../crates/epsilon-cli/tests/observation_ownership.rs)
drives real parent/task-child and restored-history flows. Scoped core/tools
verification passed 294 tests; selected new-test runs passed (see M81). Full workspace verification remains
blocked by Windows wire_fixtures launch/link errors (root-caused as an Avira
`HEUR/APC.Misc` quarantine of the freshly linked test executable — see
[M81](../docs/milestones/M81-observation-ownership.md) and the verification log);
F09 alias identity remains open. The implementation is not yet committed. The original
source trace below describes the pre-fix snapshot, not the current working tree.

**Evidence:** `crates/epsilon-tools/src/read_log.rs:90–92` is one process-global `PathBuf → Snapshot`; `:120–139` updates it after every read/write; `:153–179` verifies against whichever agent refreshed it last. Real mutations call it at `crates/epsilon-tools/src/files.rs:477–501`. Core supplies a non-distinguishing `"inline"` context ID at `crates/epsilon-core/src/exec.rs:661–669`.

**Trace:** A reads → B writes through Epsilon → B refreshes global stamp → A's stale full overwrite compares against B's stamp and passes. A read by B can also satisfy A's never-read rule. This is a semantic error, not merely missing writer attribution.

Hermes retains per-task observations (`references/hermes-agent-2026.8.19/tools/file_state.py:93–139,157–189`). The older audit's “fixed M65” conclusion is reopened. M65's global-observation decision must be amended; the path-global mutation lock should remain.

**Acceptance:** two actual tool contexts, not an external `fs::write` substitute: B's successful write invalidates A until A rereads. Include partial reads, failed writes, independent sessions and observation cleanup. Resolve file identity consistently with F09; metadata-only stamps still have same-size/same-timestamp limitations.

## F03 — Process already-received SSE lines before awaiting the network

**Evidence:** `crates/epsilon-llm/src/providers/mod.rs:776–811` returns after the first event-producing frame, leaving complete lines buffered. `:820–847` polls another HTTP chunk before draining them. `:865–870` feeds the entire EOF remainder as one line. `:947–958` synthesizes completion from content/tools even without terminal evidence.

**Trace:** one HTTP body chunk contains several SSE frames → first frame returns → terminal frame remains buffered → a quiet server causes a false idle timeout, or EOF processing loses frame boundaries. A missing terminal reason can also be mapped to ordinary completion/tool use, rather than an interrupted response.

**Impact:** truncated prose/usage, stalled runs and potentially executing calls from an incomplete generation. The execution consequence depends on the particular wire/EOF shape; no real provider reproduction was performed.

**Coverage gap:** wire fixtures test codec events and line helpers, not every production `WireStream` network-chunk partition.

**Acceptance:** local HTTP fixture sends the same valid stream as one chunk, byte fragments, several frames/chunk, and final-frame-with-connection-held-open; results and usage must agree. Missing terminal evidence must yield failed salvage and no executable partial call. Valid provider-specific final framing remains supported.

**Re-verified at HEAD `4fbca81`, 2026-09-25 — STILL REAL, and the mechanism is now exact.** The row's
evidence above cites `:776–811` / `:820–847`; those ranges have drifted, and the code they describe is
still there. `WireStream::ingest` (`crates/epsilon-llm/src/providers/mod.rs:780-819`) drains complete
lines in a `while let Some(pos) = …position(|b| *b == b'\n')` loop with `return first;` **inside** it, so
every complete line after the first event-producing line stays in `line_buf`; `EventStream::next`
(`:823-848`) pops `self.queued` and, when that is empty, awaits `self.inner.chunk()` without re-checking
`line_buf`. A provider that delivers a whole response in one TCP chunk therefore loses every frame after
the first, and if the body then ends, `next()` returns `None` and the stream ends **cleanly** — the
"EOF can claim success" half of the finding, reachable with no slow server at all. No code change; this
is the verification. Record: [open-items-at-head-2026-09-25.md](evidence/open-items-at-head-2026-09-25.md) §1.1.

## F04 — Make durable compaction replace superseded model context

**CLOSED 2026-09 (M86; the branch variant in M90).** What was wrong: a second compaction
*added* its projection to the first instead of replacing it, so a store-backed request
carried the new summary **and** the summary it had just superseded (S1/T1/S2/T2 while runtime
history held only S2/T2) — and reopening the session restored the same wrong view. What
closed it: `model_view` now clears the projection before installing the latest summary + its
retained tail + later messages, so those three are the whole request view
(`crates/epsilon-session/src/store.rs`; pi `session-manager.ts:418-453`), and the human
transcript is untouched throughout — this was always a projection error, never physical
duplication of the log. Evidence: the regression
`a_second_compaction_replaces_the_superseded_projection`
(`crates/epsilon-session/tests/session_store.rs:207`) was **shown failing against the pre-fix
code** in the M86 RED round — the view held `first summary`, `tail-1`, `second summary`,
`tail-2`, `after` — and passes on the frozen candidate that took the `gate-m86-88f` battery
(751 passed / 0 failed / 0 ignored across 55 summaries,
`audit/evidence/gate-m86-88f.log`; [M86](../docs/milestones/M86-compaction-correctness.md)).

**What survives.** The acceptance bar asked for two things that are still not both executed:
a *two-compaction provider-context comparison through the live loop*, and branch variants.
The branch half did land in [M90](../docs/milestones/M90-session-tree.md):
`a_sibling_branch_sees_its_own_summary_and_not_the_other_branchs_compaction`
(`crates/epsilon-core/tests/compaction_scenarios.rs:582`) compacts on one branch, attaches a
branch summary at the root, continues the sibling, reopens the file, and asserts that the
**first request of the new run equals the intended projection exactly**. So the rule is
pinned at the store level and end to end across a branch, and the honest residue is narrower
than the row used to suggest: nothing in this repository runs two compactions through a real
provider, because every compaction test drives the faux provider.

**Original source trace (pre-fix snapshot, kept for history).**
**Evidence:** `crates/epsilon-session/src/store.rs:717–748` appends every summary/tail to `out` and clears only `pending`; `crates/epsilon-core/src/lib.rs:1484–1512` replaces runtime history with the new summary/tail. Provider calls prefer the store view (`:1592–1604`).

**Trace:** C1 gives S1/T1; C2 replaces memory with S2/T2 but persisted projection gives S1/T1/S2/T2. Retained material can duplicate: older summaries and tails remain even though newly pending messages are still shadowed. This is a projection error, not physical duplication of the historical log. Resume restores the divergent projection.

**Acceptance:** attach a real store, compact twice, compare exact provider context against intended current summary/tail, then reopen and compare again. Include alternate branches. Preserve every human-log entry; fix the projection, not history by deletion.

**M86/M90 bounded progress:** the replace-projection rule and its reopen case are covered (`a_second_compaction_replaces_the_superseded_projection`), and the alternate-branch variant the acceptance asked for is now executed end to end: `a_sibling_branch_sees_its_own_summary_and_not_the_other_branchs_compaction` (crates/epsilon-core/tests/compaction_scenarios.rs) compacts on one branch, creates a sibling through `branch_with_summary`, reopens, and asserts each branch's provider view shows its own summary and never the other's.

## F05 — Distinguish process exit from successful tool transport

**Evidence:** nonzero shell exits intentionally return `Ok(ToolOutput)` with `details.exit_code` (`crates/epsilon-tools/src/shell.rs:301–329,416–438`). Verification derives success only from `!result.is_error` (`crates/epsilon-core/src/lib.rs:1173–1179`). `crates/epsilon-core/src/verify.rs:151–175` records that boolean and treats any successful check in the same or later turn as covering an edit.

**Trace:** tests exit nonzero → bash successfully reports the result → verification marks pass. Additionally, a check before an edit within the same serial batch gets the same turn number and wrongly covers that later edit.

**Acceptance:** real-shaped `Ok` result with nonzero/null exit remains unverified; zero exit after the mutation may count. Check-before-edit must not count. Unknown outcomes remain unknown. Carry execution order and conservative check scope; do not redefine every ordinary nonzero shell exit as a transport failure.

## F06 — Commit summaries only on success, with cancellation

**CLOSED 2026-09 (M86).** What was wrong: a summarization pass committed whatever `Done`
message came back. A failed summarizer answer, or an empty one, became the new model
context — and an empty answer was worse than a failure, because `compact.rs` substituted a
template placeholder that then *shadowed* real context while looking like a legitimate
summary. Cancellation did not interrupt the pass either, so a result could still land after
the run was aborted. What closed it: `compact_now` returns
`CompactionOutcome::{Committed, Failed, Cancelled}` and commits only on all three of a
non-failure stop reason, nonempty text, and a successful durable append **before** runtime
history is replaced (`crates/epsilon-core/src/lib.rs`, `crates/epsilon-core/src/compact.rs`;
the placeholder is gone), with the pass cancellation-aware at connect, while polling, and
once more immediately before the commit — `biased` select, so cancellation wins a tie.
Evidence: three executed regressions against the faux summarizer, each shown failing on
pre-fix behaviour in the M86 RED round
(`audit/evidence/red-round.log`) — `failed_summarization_does_not_commit_and_is_reported_once`,
`empty_summary_text_is_refused_rather_than_padded`, and
`abort_during_summarization_ends_the_run_aborted_and_commits_nothing`
(`crates/epsilon-core/tests/compaction_scenarios.rs`); the same candidate took the
`gate-m86-88f` battery ([M86](../docs/milestones/M86-compaction-correctness.md)).

**What survives.** Two acceptance items are implemented but still not regression-tested, and
the row should keep saying so: the summarizer's **early-EOF** path and a **durable-append
failure during the compaction commit**. M86's own Limits record that the append-failure
branch — the exact case the append-before-replace ordering exists for — needs a store-level
fault-injection seam that did not exist yet. That seam does exist now
(`SessionStore::set_write_fault_for_tests`, occurrence-addressable since M124), and the
turn-loop paths use it (`crates/epsilon-core/tests/loop_scenarios.rs`), but I did **not**
find it applied to the compaction commit, so I am not claiming this residual is closed.
Likewise the third item — cancellation timing against a **live** summarizer — is untested;
every compaction test here is faux-provider.

**Original source trace (pre-fix snapshot, kept for history).**
**Evidence:** `crates/epsilon-core/src/lib.rs:1468–1478` accepts any summarizer `Done` and does not select on cancellation. `crates/epsilon-core/src/compact.rs:241–248` substitutes a template for empty text. The replacement commits at `lib.rs:1497–1512`.

**Trace:** a failed or empty `Done` message is accepted as summary content; empty text produces a placeholder that shadows useful context. Raw early EOF is already rejected before replacement (`lib.rs:1473–1474`). Cancellation does not itself create a placeholder, but it does not interrupt summary creation/polling and can allow a later result to commit.

**Acceptance:** provider error, empty answer, early EOF, cancelled connect and cancelled stream leave history/store view unchanged and emit a truthful failure/cancellation outcome. A successful summary still uses legal cuts. Coordinate commit semantics with F04/F07.

## F07 — Surface persistence failures and retain one authoritative transcript

**CLOSED 2026-09 (M122, M124, M133).** What was wrong: append and pop failures were thrown
away *after* memory had already been changed. A full disk during a prompt or result append
advanced the UI and in-memory history while the durable index stayed behind, and the next
provider request was then built from the store's projection — so the model answered a
transcript that did not contain the question, in silence. The compaction append failure was
discarded the same way. What closed it, in three parts: **M122** made the turn loop's two
store mutations report their failure (`crates/epsilon-core/src/lib.rs`, where the old shape
was `let _ = st.append(..)`), added `Agent::durable_write_failed`, and made a later request
refuse to be built from the older view instead of quietly disagreeing with the loop.
**M124** gave the fault-injection seam an occurrence count, which is what makes "fail the
*second* append" expressible at all — before it, a fault armed on count alone always failed
the prompt first, and since M122 a failed prompt append ends the run before the provider is
reached. **M133** closed the last silent surface: `serve_mode`'s boot task discarded the
result of `agent.run(message)`, so a refusal caused by a failed write vanished with nothing
subscribed to the agent's notices; the server log now carries
`serve: admitted run failed: … the session file could not record this turn … may be missing it`.
Evidence: `a_failed_durable_append_is_reported_and_no_request_is_built_from_the_old_view`
(`crates/epsilon-core/tests/loop_scenarios.rs:2497`) asserts both halves of the acceptance —
exactly one notice reaches the user, and the provider is never asked (`recorded` requests
empty); M124's rewritten test asserts its own append sequence and bounds every join, so a
wait that cannot resolve fails instead of hanging a battery round. Citations:
[M122/M124](../docs/history.md) timeline rows and
[M133](../docs/milestones/M133-serve-mode-reports.md).

**What survives.** Two honest residues. (1) M122 was written by a delegated author who went
inactive without reporting, and the milestone itself says its inventory of the other
discarded-write sites "should be treated as incomplete" — so the loop's known sites are
fixed and the *sweep* is not proven exhaustive. A later reader finding another `let _ =`
on a store write is finding the remainder of this row, not a new defect. (2) M133's own
limit: `serve`'s protocol genuinely cannot carry the failure to the client, because a
`Notice` is a broadcast event and the boot task starts before the listener binds, so a client
learns of its own failed prompt only from the next request's refusal. That is recorded as a
limit rather than papered over, and it is why this row is closed with a marker rather than a
clean tick.

**Original source trace (pre-fix snapshot, kept for history).**
**Evidence:** `crates/epsilon-core/src/lib.rs:743–777` discards append/pop failures after changing memory; compaction similarly ignores append failure (`:1497–1512`). Next requests use store projection (`:1592–1604`).

**Trace:** disk fills during prompt/result append → UI/in-memory history advances → store index does not → model sees older context or resume loses the turn. A partial physical append followed by continued writing can damage the log. This is especially material on the disk-constrained development machine.

**Minimum decision:** stop further model/tool execution after a failed durable commit, or explicitly switch to an announced in-memory-only mode. Never silently alternate authoritative sources. Specify append, projection and UI event commit order.

**Acceptance:** injected short-write/flush/append errors at prompt, assistant, result and compaction boundaries; no successful durable claim, no subsequent provider request using an older view, prior valid log recoverable. Do not depend on physically filling the operator's disk to test this.

## F08 — Refuse schema/version errors; repair only genuinely incomplete tails

**CLOSED 2026-09 (M108; M113/M116 for what replaced its remedy).** What was wrong: the store
asked only "is this the last line and did it fail to parse?" before truncating. A complete
final line in a shape this build did not know — a newer Epsilon's entry, or the user's newest
turn — was treated as crash debris and deleted, while the same bytes in a mid-file position
were refused with a typed diagnosis. Position decided whether data survived, which is not a
rule anyone would choose. What closed it (**M108**): `is_whole_json` is the discriminator —
a fragment is still repaired, a complete line is refused with
`LineDiagnosis::UnknownLineShape`/`UnknownEntryType` and **the file stays byte-identical**
(`crates/epsilon-session/src/store.rs` — the three gates now read
`if i + 1 == last && !is_whole_json(fragment)` at `:771`, `:795`, `:809`). The repository's
own test had asserted the opposite behaviour as intended, and was rewritten rather than
deleted. Evidence: battery `m106a` (five gates exit 0, 1,021 passed / 0 failed / 1 ignored
across 59 binaries, `audit/evidence/gate-m106a.log`), a **negative control** — the three
gates reverted in place, the new test failing with
`a complete unknown final line was accepted: …` — plus an end-to-end run through the
reinstalled binary that refused a real session file with its 5 lines intact
([M108](../docs/milestones/M108-unknown-final-line.md)).

**What replaced the remedy, and why the row is nonetheless closed.** M108's refusal was safe
but blunt: one foreign entry cost the *entire* conversation, whose only routes back were
upgrading or hand-editing the file. **M113** changed the remedy without weakening the rule —
an unknown `kind.type` is now *kept* (parsed as an opaque object, replayed verbatim on the
next write, left out of the model view), while a known type with an unreadable body is still
refused, because preserving that would open the session with a silent gap where a turn should
be. The two cases got two diagnoses (`UnknownEntryType` "foreign" versus the new
`MalformedEntry` "damaged"), which also closed the item M108's own Limits had recorded as
owed. **M116** made preservation audible — the `preserved_unknown()` notice is now printed by
both frontends — and fixed a `model_change` line healing to `provider: ""`, an empty string
in a typed field that looks like data. Residual, recorded rather than hidden: the *repair*
still deletes a genuine fragment (a kill between write and newline loses that turn), which is
the intended behaviour its own test pins, and only the store's own reader is covered — the
TUI's session browser and `search.rs` parse files directly in places and were not checked for
the same classification ([M113](../docs/milestones/M113-unknown-kinds-preserved.md),
[M116](../docs/milestones/M116-preserved-notice-and-typed-model-change.md)).

**Original source trace (pre-fix snapshot, kept for history).**
**Evidence:** `crates/epsilon-session/src/store.rs:484–500` labels any unusable final entry torn without diagnosis. Opening truncates this case (`:273–275`). `truncate_to_index` defaults to byte zero if no entry indexed (`:550–558`).

**Trace:** a complete final entry rejected by both typed and lenient parsing is treated as crash debris and truncated; a final second header also follows this path (`store.rs:457–463`). Recognized legacy shapes are separately migrated or refused, so not every unsupported shape is deleted. Header plus a torn first entry leaves no indexed entry, so repair also deletes the header. A clean header-only file is not itself subject to this repair.

**Acceptance:** unknown complete final entry, second complete header and incompatible schema are refused with the entire file unchanged. Incomplete syntax tail may be repaired; a header-only surviving prefix stays on disk and reopens correctly. Test both terminated and unterminated final lines. Amend tests that codify “any bad final line is safe to truncate.”

## F09 — State and enforce the real filesystem boundary

**M84 bounded progress:** `find` now declares its lexical search-directory key,
sharing existing search-tool scheduling. Red/green regression and isolated
five-gate repeat passed (741 tests); initial auth-lock failure retained.
[F09 seam map](evidence/f09-path-policy-map.md) identifies remaining consumers.
**F09 containment is still open**; no alias or replacement-race guarantee added.

**Evidence:** lexical normalization at `crates/epsilon-core/src/exec.rs:105–139`; native edit opens/writes host paths at `crates/epsilon-tools/src/files.rs:461–499`; walker follows directories at `crates/epsilon-tools/src/util.rs:144–166`. `crates/epsilon-core/src/paths.rs:9–34` delegates symlink escape to a lower layer that native tools do not use.

**Impact:** a workspace symlink/reparse point can name an outside object while the gate sees an inside path. Alias spellings can defeat resource identity. Shell sandboxing does not surround native tool filesystem calls. This was historically deferred, but the stated product confinement guarantee exceeds the implementation.

**Acceptance:** read/write/edit/search/attachments/checkpoints share an explicit resolved-path policy. Test existing aliases and new-file parent aliases on native Windows/Linux. A canonicalize-then-open check alone does not close replacement races; either use handle-relative/no-follow access or explicitly refuse unsupported traversal and document residual races. Supporting legitimate linked workspaces requires an explicit permission policy, not silent access.

Windows Job Objects restrict resources, not file or network access; bubblewrap's read-only root is not read confidentiality (`crates/epsilon-safety/src/sandbox.rs:282–348`). Do not describe either as universal protection against secret-file reads.

## F10 — Plan restrictions must survive actual subject extraction and layering

**Working-tree update: implemented, reviewed approve; commit awaits the post-fix
verification round.** The plan ruleset now contains
wildcard `bash`/`shell` denies, CLI composition retains project rules (ADR-006
stacking), and additive authorization guarantees the original tool capability is
checked first — a derived `external_dir` subject can add a constraint but never
replace or mask an original denial (`crates/epsilon-core/src/exec.rs:337-391`).
Evidence for the implementation: red/green regressions
[`f10-red.log`](evidence/f10-red.log), [`f10-green.log`](evidence/f10-green.log),
[`f10-cli-red.log`](evidence/f10-cli-red.log), [`f10-cli-green.log`](evidence/f10-cli-green.log),
narrow delta [`f10-cli-change.patch`](evidence/f10-cli-change.patch), full battery
[`gate-f10-composed.log`](evidence/gate-f10-composed.log) — GATE PASS (pre-additive-fix
tree). Independent read-only review: **approve, no blockers**
([`f10-review-approve.json`](evidence/f10-review-approve.json)). The post-fix
rerun `f10-final2` verdict is recorded in `audit/05-verification-log.md`; both F10
and F02 land in the same commit series.
Original source trace below describes the pre-fix snapshot.

**Evidence:** plan rules match `git commit*`/`git push*` (`crates/epsilon-safety/src/engine.rs:137–145`), but core authorizes only the first shell word (`crates/epsilon-core/src/exec.rs:141–147`). CLI chooses plan rules **instead of** project rules (`crates/epsilon-cli/src/main.rs:1581–1593`).

**Impact:** subcommand denies cannot match; a broad shell approval can allow actions advertised as forbidden. Project-specific deny rules disappear in this mode. Disabling native write/edit alone is not a read-only shell boundary.

**Acceptance:** preserve all project denies when adding plan restrictions. Test through core preflight with an inert recording tool. Choose conservative shell refusal or genuinely enforced read-only execution; do not promise arbitrary shell classification via string prefixes. Windows partial enforcement must be surfaced/refused where stronger guarantees are required.

## F11 — Own child exit, descendants and pipe draining under one deadline

**Evidence:** bwrap ignores `_policy`, awaits child exit and drains without timeout/kill-on-drop (`crates/epsilon-safety/src/sandbox.rs:389–438`). Shell cancellation only drops that future (`crates/epsilon-tools/src/shell.rs:280–298`). Unconfined shell's timeout covers direct-child wait, not later drains (`:385–407`). Windows similarly drains after timed wait while holding the job (`crates/epsilon-safety/src/sandbox/jobobject.rs:271–294`).

**Impact:** Linux timeout is not implemented; cancellation may leave work running. If a descendant retains a pipe after the direct child exits, final drain can hang past the promised timeout. These lifecycle consequences require native process tests, not sleeps-and-hope.

**Acceptance:** successful completion, timeout, abort, descendant-held stdout, runner startup failure and runtime cancellation all settle within a total deadline, with owned children reaped. Results distinguish “did not start,” “started then cancelled” and normal nonzero exit. `--die-with-parent` is not cancellation while Epsilon stays alive. Require Linux CI evidence before closing.

## F12 — Bound memory at ingestion, not only model-visible output

**Evidence:** `drain_pipe` grows an unlimited vector (`crates/epsilon-safety/src/sandbox.rs:259–279`), shell truncates afterward (`crates/epsilon-tools/src/shell.rs:303–305`). File read loads full bytes for image detection (`crates/epsilon-tools/src/files.rs:113–144`); webfetch buffers body before final size check (`crates/epsilon-tools/src/webfetch.rs:155–175`).

**Impact:** command/file/HTTP volume can exhaust the harness's memory despite bounded displayed output. Child memory limits do not constrain the parent collector.

**Acceptance:** bounded signature read for images, explicit attachment limit, streamed HTTP byte budget, bounded head/tail capture plus capped spill. Test oversized individual chunks, missing Content-Length, spill failure and multibyte clipping. Tell the model when full output cannot be retained; never advertise a nonexistent complete spill.

## F13 — Acknowledge runs only after atomic admission

**M86 bounded progress (RPC half):** `admit_prompt` answers with what the run itself
decided — the spawned run is raced against its own first event, so a rejected request
receives the real `AlreadyRunning` reason instead of a false `started`
(`crates/epsilon-cli/src/rpc_mode.rs`). A post-admission failure is logged instead of
discarded. Executed regression: `a_second_prompt_is_rejected_while_a_run_is_admitted`.
**M87 closed the reply contract on both surfaces:** RPC answers from the run's own decision
(the race described above, with the advisory busy helper deleted), and serve's reservation is
atomic — `fetch_add` on the run counter *is* the decision, so two overlapping requests cannot
both pass a check-then-act, and the slot is released by a `Drop` guard inside the run task
(`docs/milestones/M87-rpc-admission.md`).

**M92 closed the protocol-breadth half (RPC + serve):** replies now carry pi's
`type:'response'` envelope with the request's `id` echoed on that request's reply (pi's `type`
tag is accepted as an alias for `op`), while keeping the first wire's `ok`/`error` keys so an
existing client needs no translation; a dropped broadcast becomes an explicit frame on RPC
instead of a silent skip, matching serve's SSE `lagged` shape; and both surfaces expose
`abort` and `state` (`POST /abort`, `GET /state`) backed by the same core calls and one shared
payload struct (`SessionState`, `epsilon-contracts`). `Agent::model_id` became public for that
payload. Deferred op families fail explicitly as `unknown op: <name>` with the echoed id.

**Still open:** pi-faithful command names and payload dialects beyond the envelope (`message`
vs `task`/`text`, `follow_up` vs `followup`, snake- vs camel-case state keys); the
model-switching ops (`set_model`/`cycle_model`/`available_models` — needs the TUI's
id+hot-swap+persist sequence as one seam); queue modes (no core setter); compaction over the
wire; and RPC's `run()` loop and event pump are not end-to-end tested (it owns stdin/stdout —
the lagged frame's bytes are pinned by a unit test, the wiring is inspection-only).

**Evidence (historical — the pre-M87 shape this was written against):** RPC busy helper always returned false, starts a detached task and discards `run()` result (`crates/epsilon-cli/src/rpc_mode.rs:77–85,117–120`); HTTP checks then increments separately (`crates/epsilon-server/src/lib.rs:238–253`). Core rejecting concurrent runs does not retroactively correct a successful admission response.

**Impact (closed for admission, correlation and abort):** callers could receive `started`/202 for rejected work and wait for events that never identified their failure; RPC also silently skipped lagged broadcasts, had no correlation ID and no abort operation. What remains is the unclosed vocabulary gap and the untested pump listed above — not an acknowledgment problem.

**Acceptance:** competing requests have exactly one accepted run, the other gets an attributable busy error; reserve/release admission even on failure — **delivered** (M86 RPC half, M87 both surfaces). Cancellation and explicit dropped-event/recovery semantics before treating RPC as a dependable editor API — **delivered** (M92 `abort` + the `lagged` frame, both surfaces). The surviving item is the untested RPC pump and the vocabulary gap in *Still open* above.

## F14 — Make LSP feedback bounded and tied to the current document

**Evidence:** `crates/epsilon-lsp/src/client.rs:234–262` repeats version 2 after every open and sends before the timeout. `:268–300` accepts the first matching URI and may return cached diagnostics after timeout. Framing holds partial read state inside a cancellable future (`crates/epsilon-lsp/src/framing.rs:45–86`). Tool mutation awaits diagnostics inside its queue (`crates/epsilon-tools/src/files.rs:467–511`).

**Impact:** stalled stdin can hang a successful edit response; a timeout mid-frame can desynchronize reuse; delayed diagnostics can describe old bytes. Actual server rejection of repeated versions is server-dependent.

**Acceptance:** monotonic per-document versions and correct language IDs; delayed/stale publications distinguished from fresh results; total timeout includes writes. Persistent reader state or retire client after partial-frame timeout. Fixture stops reading input and splits frames across deadlines. Optional LSP failure must not change successful file mutation into a false failed edit.

## F15 — Checkpoint capture is not yet safe user undo

**CLOSED, WITH THE USER-FACING HALF MISSING (M100, M104 — both 2026-09).** What was wrong:
`restore()` read `blob: None` as "the file did not exist, so reverting means removing it",
and a **skipped** capture is also `blob: None` — with `existed: true` and a reason. So asking
to undo one edit to an oversized file deleted the file. The second half was worse in kind:
restore wrote the snapshot over whatever was there without asking whether the file had moved
on, so the sequence a user actually performs — agent edits a file, user edits it again, user
asks for an undo — replaced the user's own work with the agent's older copy and said nothing.
The ledger append failure was swallowed too, so a capture could report `Stored` while its
record never reached disk. What closed it: **M100** keys the branch on `existed` (reading
`skipped` only for the message), so a skipped capture is an error naming the reason and the
file is left untouched, while a file that genuinely never existed still reverts by being
removed. **M104** gave the entry a `digest` — the content hash every capture already computes,
because `blob_name` *is* the content address — and refuses to write when the target's content
no longer matches, comparing **content and not metadata** (length or mtime would have been
fooled inside one clock tick, exactly as `read_log`'s guard test was); an entry too old to
carry a digest is refused rather than assumed safe, and a missing file is not a conflict
because deletion is the case a snapshot exists to undo. `append` now returns its failure
instead of dropping four of them. Evidence: M100's
`reverting_a_skipped_capture_refuses_instead_of_deleting_the_file`
(`crates/epsilon-tools/src/checkpoint.rs:653`) and
`reverting_a_file_that_never_existed_still_removes_it`; M104's
`a_restore_refuses_to_clobber_a_file_that_changed_since_capture` (`:516`),
`a_restore_recreates_a_file_that_was_deleted_after_capture` and
`a_restore_refuses_an_entry_it_cannot_verify`. One existing test had been *codifying* the bug
— it clobbered the file and then asserted the restore succeeded — and became the refusal test
instead. Sources: [M100](../docs/milestones/M100-restore-must-not-delete.md),
[M104](../docs/milestones/M104-restore-verifies-the-file.md).

**What survives, and why this row keeps a marker.** **No user-facing revert exists.**
`CheckpointStore::restore` has no caller outside `checkpoint.rs`'s own tests — I re-checked
the whole tree; `files.rs` only calls `open` and `record` — so both milestones closed
*latent* defects, and the exposure arrives with a rewind/undo command that does not exist
yet. That is the residual the earlier wording carried as "no frontend reach established", and
it is still true, so it stays in the row rather than being tidied away. Three smaller items
also survive: the restore is **not atomic** (the file is read, compared, then written; a write
landing in between is undetected — the session lease and per-path mutation queue are what make
that unreachable in practice, and no test asserts it); two items on F15's own acceptance list
were named in the commits as **not claimed** — serializing ledger appends across different-file
writers, and validating every blob path against the store root (the path-traversal half); and
the refusal is deliberately user-hostile with no recovery affordance ("revert it by hand"),
which is a UI question neither milestone answered. Verification honesty: no battery log covers
either slice, and neither milestone claims a pre-fix RED for all of its tests — the evidence
is focused per-crate test runs recorded in the commit messages, with the test-count deltas
reconciled against the neighbouring commits (92 → 94 → 97 in `epsilon-tools`). Windows-only;
the Linux CI job has not run on those bytes.

**Original source trace (pre-fix snapshot, kept for history).**
**Evidence:** skipped oversized captures have `blob: None, existed: true, skipped: Some(...)` (`crates/epsilon-tools/src/checkpoint.rs:168–184`). `restore()` treats every `None` blob as deletion (`:261–277`), without checking skip/existence. Ledger append errors are swallowed (`:223–235`) while capture can report success (`:211–216`). Restore writes without checking later user changes (`:265–270`).

**Impact:** restoring a skipped capture through this API deletes an existing file; restoring a real preimage can overwrite later work. No normal UI route to this API was established, so this is a **blocker before exposure**, not a claim that users already trigger it with a shipped slash command.

**Acceptance:** typed captured/created/skipped states; skipped is never restorable. Persist post-write identity and refuse conflicts unless a human explicitly overrides after preview. Surface capture failures, validate ledger/blob paths, serialize ledger append across different-file writers, and test interrupted capture/eviction. Reuse the current store rather than adding shadow Git first.

## F16 — Unify call-time OAuth preparation

**Still open at HEAD — re-checked 2026-09-23, unchanged.** The row stays as written; the
check is recorded here so the next reader does not have to redo it. At HEAD:
`refresh_oauth_before_use` still resolves the client id from the environment alone
(`crates/epsilon-llm/src/providers/mod.rs:1174` — `std::env::var(endpoint.client_id_env)` with
an early `return` when it is unset) while login accepts env-then-config
(`crates/epsilon-llm/src/oauth_login.rs:780`, `client_id_from_config`); GitHub Copilot's
refresh endpoint still declares an **empty** `client_id_env`
(`crates/epsilon-llm/src/oauth.rs:86`) because its mint takes no client id
(`:226-260`), so for Copilot the call-time refresh returns early and never mints; and
`fallback.rs` still reaches the transport through `try_stream_direct`
(`crates/epsilon-llm/src/fallback.rs:115` → `crates/epsilon-llm/src/providers/mod.rs:1191`,
whose body from `:1191` to `:1260` contains no refresh call at all), the path with no refresh
preparation. The threshold mismatch below is also still in the
code: `oauth_needs_refresh_in` refreshes when `expires <= now + min_validity_ms + 60_000`
(`crates/epsilon-llm/src/auth.rs:639-648`) while `store_refreshed_oauth_in` discards the fresh
pair when the *stored* access is live
(`:672-679`, `*expires > now_millis() + 60_000 && *access != fresh.access`) — so with no
concurrent writer at all, a pair inside the one-to-six-minute band triggers an exchange whose
result is thrown away, and the refresh repeats on every call until the stored token falls
below a minute of life. The existing test
(`oauth_two_phase_refresh_composition`, `auth.rs:1143`) exercises the concurrent-rotation
case, not this one.

**Original (unchanged) evidence:** `crates/epsilon-llm/src/providers/mod.rs:1161–1181` requires an environment client ID before calling the refresh exchange. Copilot's endpoint has no such requirement (`crates/epsilon-llm/src/oauth.rs:79–87,223–226`). Login supports config client IDs (`crates/epsilon-llm/src/oauth_login.rs:780–789`). Fallback retains credentials and invokes direct transport without the same refresh preparation (`crates/epsilon-llm/src/fallback.rs:33–40,89–115`).

**Impact:** successful login does not imply refresh works; long-running fallback sessions can reuse expired tokens. These are source integration findings, not live account tests.

**Source-confirmed threshold mismatch:** `crates/epsilon-llm/src/auth.rs:623–627,653–662` can start proactive refresh while the old token remains live beyond one minute, then reject the newly exchanged token merely because the old stored token is live and different. No concurrent writer is required by those branches. The code needs to compare with the originally exchanged snapshot. Whether discarding a rotation invalidates subsequent refresh depends on provider semantics; this was not tested with a live account.

**Acceptance:** injected clock/store/exchange tests for Copilot without client ID, config-only IDs, expired fallback credentials, missing rotation and true concurrent refresh. Compare against the originally exchanged credential snapshot. Required client IDs remain required for flows that need them; never make all IDs optional to fix Copilot.

## F17 — Persist cancellation salvage like ordinary completion

**CLOSED 2026-09 (M86).** What was wrong: the two local cancellation arms in
`stream_one_turn` returned their synthesized terminal message directly, so partial prose could
survive in `RunOutcome` while being absent from both in-memory history and the durable
transcript — a resumed conversation lost the answer the user had already watched stream in.
Ordinary raw-EOF salvage already pushed history and emitted `MessageEnd`; the finding was
specific to locally synthesized cancellation. What closed it: both abort arms now
`push_message` the synthetic terminal message into runtime history *and* the durable mirror,
and emit `MessageEnd` — the same lifecycle the provider-`Done` and EOF-salvage paths use
(`crates/epsilon-core/src/lib.rs`). Evidence: the regression
`abort_mid_stream_commits_the_terminal_message_to_history_and_store`
(`crates/epsilon-core/tests/loop_scenarios.rs:859`) asserts one terminal message carrying the
partial text in history, in the store, and in the **reopened** model view, with exactly one
`MessageEnd`; the pre-fix RED is in the same M86 round (it failed with "one terminal message
survives the abort"), and the candidate took the `gate-m86-88f` battery
([M86](../docs/milestones/M86-compaction-correctness.md)).

**What survives.** Two items, both inherited rather than new. A durable-append **failure**
during that commit is the F07 shape and is handled by M122's reporting, but it is not
separately regression-tested *for the abort commit* (the compaction append failure is the
untested one; see F06). And the live timing case — a provider `Done` racing a cancellation —
is unexecuted: the select is `biased` toward cancellation, which is the right ordering and is
also exactly why a real race would be the interesting test. The acceptance bar's "incomplete
calls never executed" half is covered by F20/M88, which refuses to start calls in a cancelled
batch.

**Original source trace (pre-fix snapshot, kept for history).**
**Evidence:** cancellation returns locally synthesized assistant messages directly (`crates/epsilon-core/src/lib.rs:1680–1703`); provider `Done` uses `push_message` and `MessageEnd` (`:1729–1733`).

**Impact:** partial prose can survive in `RunOutcome` while absent from both in-memory history and the durable transcript. Ordinary raw-EOF salvage does push history and emit `MessageEnd` (`lib.rs:1772–1782`); the finding is specific to locally synthesized cancellation. The inspected abort regression checks returned output, not reopened history.

**Acceptance:** attached-store abort during connect and after text; partial/error state retained exactly once, normal terminal lifecycle emitted, incomplete calls never executed. Reopen and compare. Coordinate error handling with F07.

## F18 — Reset task state when resuming

**M93 fix (2026-09-18), verification owed:** the resume branch now claims the entry in **one
lock hold** — `Agent::claim_task_running` (`crates/epsilon-core/src/lib.rs:2787-2802`) finds the
entry (honest "no longer tracked"), refuses a child that is already `Running` with the message
the old guard used, and sets `Running` — called after the child lookup, the last fallible step,
before the background/foreground split (`:2850-2860`). The second-resume refusal, the abort
cascade's reach and the entry's unevictability fall out of that state change; the racy
read-then-check guard it replaced is gone, and `register_task_entry`'s selector is unchanged
(and, because the claim writes `Running` before the split, the selector now genuinely protects a
running child). Both run bodies are contained the way the hook seam contains a panicking hook
(`contain_panic`, `:308-338`, the std-only `pin!`/`poll_fn` + `catch_unwind` boundary): the
detached spawn body, so an unwinding run writes a `failed` record naming the panic instead of
leaving the entry `Running`, and — added by the review round — the foreground body
(`await_task_run`, `:2952-3031`), whose panic arm writes the same terminal record and returns
the error, so a foreground panic cannot strand the just-claimed entry `Running` forever. The
detached body's terminal write recovers a poisoned registry lock instead of skipping the write
(`spawn_task_run`, `:3034-3110`). Regressions in `crates/epsilon-core/tests/task_scenarios.rs`:
claim visible before the dispatch returns, a second resume refused without a forged failure
notice, abort reaching the resumed child, the child surviving registry eviction, a failing leg
landing `Failed` (and a failed rehydrate registering nothing), plus one panic test per contained
body (background and foreground). Milestone log: `docs/milestones/M93-task-state-on-resume.md`.
The frozen-candidate battery is owed.

**Evidence (citations corrected 2026-09-18 — the recorded ranges were anchored to revision
`3afafd3` and had drifted +135 at the abort site and +448…+458 in the task region):** the resume
branch never wrote `Running` (recorded `:2320-2356` → HEAD `:2768-2824` before this fix); its
guard (`HEAD :2790-2793`) compared a snapshot read two calls earlier, so two racing resumes could
both pass it and nothing ever set the state. The background arm spawned the leg (`:2802-2809`)
and returned `TaskOutcome::Running(now)` at `:2813` carrying the OLD terminal state. Abort
selected children with `e.state == TaskState::Running` (recorded `:995` → HEAD `:1130`).
Eviction took the first non-running entry (recorded `:2229` → HEAD `:2687-2698`, selector
`:2692`). The spawn body's final state write was `:2953-3007` (recorded `:2495-2530`).

**Impact (refined):**
- **Eviction means an untracked running task, not duplicate execution.** A resumed child's entry
  was eligible for eviction while its child ran (nothing ever wrote `Running`); once evicted, the
  running child was tracked nowhere — a later resume or status probe of that id finds nothing
  tracked and `rehydrate_task` (HEAD `:2439-2573`) tries to open the child's file through
  `SessionStore::open` (HEAD `:2467-2468`), which the running child's M61 lease refuses
  (`crates/epsilon-session/src/store.rs:292-299`): the probe fails with the lease error rather
  than an honest "still running", and the abort cascade cannot reach the child. Only after the
  first leg ends (lease released) can the id rebind as a fresh child — identity loss. The review
  round corrected this bullet's original "rebuilds a SECOND child … two live children, two
  writers on one file" reading: the second open is refused, so the damage class is the untracked
  window. M93 closes it by making the claim real: pre-fix the entry kept its old settled state
  and the eviction selector (HEAD `:2692`) could take it; post-fix the entry is `Running` while
  the child runs, and the selector (`:2723`) skips `Running` entries, so eviction cannot take it.
- **The background arm forged failures.** Resuming a still-running child in the background
  swallowed the child's own `RunError::AlreadyRunning`: dispatch returned `Ok(Running(now))` with
  the entry flipped to `Failed`, and a `state="failed"` notice was queued for a run that was
  still in flight — the parent model was told a task had failed while it kept running.
- **"Core may still reject concurrent execution" was a false comfort — but not because a
  duplicate ran.** Core refuses a second run only on the *same* child agent (`Agent::run`'s
  `running` swap — an in-agent guard, not a boundary), so that guard is no help here; what
  actually refuses the duplicate-rehydrate path is the session layer: the child's file is opened
  through the M61 store lease, and the running child's store holds it, so the rehydrate fails
  before a second child exists. The damage class is therefore the untracked window (above), not
  duplicate execution, concurrent writers, or a misreported status field.

**Acceptance:** controlled slow-provider resumed child appears running before dispatch
acknowledgment, refuses second resume, receives parent abort and remains unevictable until
settled. Failed launch restores a truthful terminal state.

## F19 — Reconcile product workflows and documentation

- **Rejected stale allegations:** the current README already lists twelve crates and warns about the failed baseline (`README.md:17–21,155–168,185–193`). Its sandbox wording now applies to commands, not every tool (`:41–44`). Do not repeat those earlier allegations as current facts.
- **Still incorrect:** no runtime dependencies (`README.md:8–9`), read-only plan-mode guarantees (`:107`), compiler-enforced architecture and types-only contracts (`:148–157`). External executables, F10, and executable `StreamFolder` logic contradict these claims.
- **Already reconciled:** Plan 018 is PART DONE in `docs/plans/README.md:38`, matching the Copilot/Codex split. The old TODO allegation is stale.
- **Broken provenance link:** `docs/README.md:62–63` claims a restructure move map lives in 05; no such map exists there. Existing document moves predate this audit run.
- `/tree` inspects only active-branch entries; `/fork` copies a file and does not switch (`crates/epsilon-tui/src/app.rs:1446–1477,1594–1608`). The `Command::New` arm only clears UI (`:1487–1492`); check parser/browser routing before attributing that behavior to every `/new` invocation. **M90 bounded progress:** the store foundation this needs now exists — `branch_with_summary`, explicit no-leaf root navigation, `BranchSummary` projected on the active branch, and `fork_branch_at` (fresh id, `parent_session` link, selected ancestry only, gapless `seq`, source untouched), with the alternate-branch projection regression the F04 acceptance asked for (`docs/milestones/M90-session-tree.md`). The TUI surface (`/tree` listing every entry with a leaf marker, Enter's user-message→parent rule with editor-text restore, `/fork` switching the live session through the new API) stays open as SESSION-01b: `crates/epsilon-tui/src/app.rs` carries another work item's uncommitted changes, so wiring it now would entangle two changes.
- App orchestration is concentrated in a 6,404-line source file; split only stable ownership seams after characterization, not arbitrary line counts.

**Acceptance:** one current status source; explicit compatibility/platform matrix; README → setup → first run → permission handling → resume/recovery walkthrough checked on a fresh profile. Preserve history as history, not current guarantees.

## F20 — Abort must stop starting new calls inside the running batch

**M88 fix (verified — battery `gate-m86-88f`: 751 passed / 0 failed / 0 ignored across 55
summaries, `audit/05-verification-log.md`; committed as `31b1794`):** `run_one` now checks the
run token before starting a call — the single choke point shared by the barrier and segment
plans — so a cancelled run refuses the rest of the batch with an error result ("not executed: the
run was cancelled before it started", `details.aborted`), still in call order, while in-flight
calls settle. Executed regression: `abort_mid_batch_refuses_calls_that_have_not_started`
(its pre-fix RED — both batch calls executed after the abort — is in the same log).

**Evidence (pre-fix source trace):** `crates/epsilon-core/src/exec.rs` scheduled every
prepared call unconditionally in both plans — the barrier loop (`:553-562`) and the per-group
task loop (`:579-596`) called `run_one` with no cancellation check; `run_one` (`:623-737`)
handed the run token to the tool through `ToolContext` (`:683-684`) but never consulted it
before executing. The loop's abort checkpoint ran only at the next turn
(`crates/epsilon-core/src/lib.rs:1076-1079`).

**Reference semantics:** pi breaks a sequential batch after the in-flight call settles and stops scheduling parallel calls once the signal aborts; every remaining call becomes an error result ("Operation aborted") and all results are still emitted in order (`references/pi-0.84.2/packages/agent/src/agent-loop.ts:478-480,516-553,629-635`).

**Impact:** after Esc, the rest of the active batch still started and executed. A write or shell command the user just cancelled could still run; only tools that personally honour the token stopped early. The transcript stayed valid (every call receives a result), which is why this never surfaced as a wire error or a dangling-call repair.

## Deliberate differences, not implementation bugs

Keep process-isolated extensions, ratatui, TOML configuration, bounded resource-conflict scheduling, advisory verification nudges and fail-closed unavailable confinement. Each can differ from pi intentionally. None justifies incorrect data preservation or unbounded execution.

Do not add fuzzy edit stages, a plugin framework replacement, distributed teams, a GUI, automatic LSP downloads, or memory curation before these existing contracts work. Missing pi capabilities are tracked separately in [03](03-port-plan.md), rather than inflated into security defects.

## Unclosed audit areas

Dependency advisories/license provenance, hostile multi-process lease races, complete OAuth/provider wire conformance, cold-start/install artifacts, TUI visual accessibility, long-session performance and cross-platform execution need separate evidence. Absence of a finding here is not a clean bill of health. No severity is assigned to an unverified CVE or guessed upstream behavior.
