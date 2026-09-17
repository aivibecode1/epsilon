# Queue test design and audit/docs commit review

## Verdict and scope

**Static review only. The default follow-up regression has a meaningful provider-context oracle, but code approval is pending. Explicit All-mode preservation is missing from the reviewed test.** No implementation, RED/GREEN execution, complete-port acceptance or production readiness is certified here.

Reviewed commits `13773b4fc088e8ded5a1034f2eed00201a7a69c2` and `bcc1233681f89af65c208d2916f1a5b77d6f3d16`, the 55-line uncommitted queue test, relevant current Rust source and local pi constructor/loop source. Read AGENTS and memory. The task's report-only restriction takes precedence over general review/TDD build and mutation workflows. No builds, cleanup, source edits, staging, installs or credential access were performed. The parent's RED process is outside this review; its result is not asserted.

All citations below use the absolute repository prefix **`C:/Users/Hp/Desktop/My Agent Space/Epsilon/`**; filenames in code spans following that prefix denote files under that exact root. Line numbers describe the inspected snapshot, not future edits.

## 1. Documentation and phase separation

### D1 — Commit contents preserve the source-free boundary (PASS)

`git diff-tree --name-status` shows seven audit/evidence/memory files in 13773b4 and six documentation/memory files in bcc1233. A comparison from 13773b4's parent through bcc1233 over the absolute crates and Cargo manifest/lockfile paths returned an empty diff with exit 0. Neither commit includes the new queue test or inherited production changes. This proves commit-content separation, not an independently reconstructed history of every working-tree write during the audit.

The fresh report correctly labels the tested baseline as the **working tree over 80b32eb**, not committed HEAD, and explicitly leaves broad inherited changes unapproved. The queue test is a later implementation-phase artifact, not evidence that the earlier docs phase changed source. Keep it out of any audit-only amendment.

### D2 — Main verification claims are supported, with attribution limits (PASS with caveats)

The preserved baseline log contains 55 test summaries totaling **795 passed, 0 failed, 1 ignored**, independently aggregated during this review. Its demo reports `Completed`, six produced messages and 300/150 input/output tokens at lines 1616–1618; the final real-feature check completes in the same preserved log. The commits do not turn this into live-provider, cross-platform or complete-port certification. README's replacement of stale red claims and its explicit not-production-ready warning are justified.

The separate contracts-only run, exact cleanup amount/free-space result and no-install history are **reported parent observations**, not rerun or independently substantiated by a dedicated transcript here. The workspace log does contain the contracts suite and its three stream tests, but does not by itself prove a second invocation. Preserve that distinction if tightening evidence language.

The tested dirty tree is named by base commit and status counts, not a complete content-addressed candidate manifest. Those counts do not make the later dirty tree or a clean checkout identical to the tested tree. Before approving code, test an isolated candidate or record exact selected-file identity; do not reuse 795 as the new candidate's result.

### D3 — Corrected F02/F10 descriptions match inspected source (PASS)

The revised plan statement is supported by `crates/epsilon-safety/src/engine.rs:137–146` (write/edit/bash/shell denies), `crates/epsilon-cli/src/main.rs:1585–1590` (project layers retained), and `crates/epsilon-core/src/exec.rs:337–365` (original and derived capabilities checked). It explicitly does not promise arbitrary extension side-effect classification.

The observer description agrees with `crates/epsilon-tools/src/read_log.rs:92–115,167–186`: opaque weak-owner scopes and sibling invalidation. Reclamation is lazy on subsequent map access, not immediate on owner drop; “retired observers are reclaimed” is acceptable but should not be expanded into an immediate-reclamation guarantee. The docs retain lexical identity/containment limitations rather than overclaiming F09 closure.

### D4 — Remaining wording/closure issues do not certify a finished restructure (FOLLOW-UP)

- `audit/09-revalidation.md` calls local pi references “bundled”; the independent core/surface reports do likewise. bcc1233 fixes README to “locally inspected ... not included in a clone.” Prefer the same wording in maintained audit introductions. The snapshot qualifier prevents an upstream-version claim, but “bundled” remains ambiguous about checkout contents.
- R02 in audit/09 describes the formerly stale README in present tense even after bcc1233 repairs it. Preserve the historical finding, adding a dated “documentation corrected in bcc1233” disposition rather than letting it read as a current defect.
- The eight local-only documentation links are explicitly a prior working-tree snapshot, not eight proven broken committed links. The structure report makes that distinction correctly. Review/resolve outstanding link closure separately; do not stage inherited M84 or historical milestones merely to make local links ship.
- Memory still has historical HEAD 80b32eb and “corrections prepared/no running builds” checkpoint language. That is not live process authority after the later RED start. Parent should update active recovery state without rewriting historical gate evidence.

**No blocking release/complete-port overclaim was found in the reviewed commits.** They justify narrow documentation reconciliation, not exhaustive documentation closure or an implemented port.

## 2. Does the new test assert actual one-at-a-time contexts?

**Yes, for the deterministic text-only default-mode scenario.** `crates/epsilon-core/tests/queue_conformance.rs:20–55` constructs the real Agent with default limits, enqueues distinguishable messages, awaits `run`, and examines the contexts actually received by FauxProvider. `crates/epsilon-llm/src/faux.rs:119–123` records the incoming `LlmContext`; it does not manufacture request histories from its scripted responses. Core builds those messages from runtime/store history at `crates/epsilon-core/src/lib.rs:1598–1610,1666–1670`.

The oracle requires:

1. Exactly three provider calls; initial context is only the original prompt.
2. The next context has three messages, contains the first follow-up, and excludes the second.
3. The third context has five messages, contains both follow-ups at the expected positions, and has the first follow-up's completed assistant response between them.

Thus this is stronger than checking `LoopLimits` or queue drain counts: it detects batch delivery and verifies the first follow-up is answered before the second becomes visible. No sleeps or concurrent timing assumptions are needed.

Source supports the expectation: pi defaults both queues to one-at-a-time at `references/pi-0.84.2/packages/agent/src/agent.ts:231–232`; its queue drains one or all at lines 142–155. Rust `crates/epsilon-core/src/lib.rs:195–196` still defaults follow-ups to **All** in this review. Queue construction uses those limits at lines 391–403, the two drains differ at lines 133–136, pending messages enter history at lines 1067–1071, and follow-ups are consumed only after the inner loop stops at lines 1262–1267. Pi's corresponding outer boundary is `references/pi-0.84.2/packages/agent/src/agent-loop.ts:259–267`.

**Predicted RED, not observed RED:** current All default should make two requests, failing the expected-three assertion. A compiler/linker failure would not establish this regression.

### Oracle limits / improvements

- Assert `RunOutcome.reason == Completed`; `run(...).unwrap()` only excludes RunError, not a non-completed outcome.
- Assert the initial assistant's role/text at request 1 index 1, and preservation of the earlier prefix in request 2. Current assertions strongly establish separation but leave those slots partly unspecified.
- This text-only fixture does not prove tool-cycle boundaries, steering priority, event ordering, failure/cancellation behavior or attached-store projection. Do not label it complete queue conformance. A future tool fixture should keep the second follow-up absent during all tool/result turns triggered by the first, admitting it only when that work would stop.
- Audit R01 explicitly asks for default, explicit OneAtATime and explicit All cases plus emitted order. The single reviewed test covers only the default provider-history case; it does not inspect events.

## 3. Required explicit All-mode preservation test

Add an assembled-loop test using `LoopLimits { followup_mode: QueueMode::All, ..LoopLimits::default() }`, the same initial/first/second messages, and two scripted nonempty assistant replies (`initial complete`, `batch complete`). Queue both follow-ups before `run`.

Require exactly **two** recorded requests:

- Request 0: `[initial]`.
- Request 1: `[initial, Assistant("initial complete"), first, second]` — exactly four messages, both follow-ups adjacent, in FIFO order, with no assistant response between them.

Require a completed outcome and final history/produced output containing both follow-ups once and the final `batch complete` answer, with no third provider request. Compare assistant role/text rather than inventing generated IDs/timestamps. This guards against accidentally hardcoding all modes to OneAtATime while changing the default. It should pass before and after the default fix: it is a preservation test, not a second bug expected to go RED. Add an explicit OneAtATime case reusing the separate-context oracle to meet R01's advertised matrix.

## 4. Code approval prerequisites

1. Capture the default test's actual RED assertion against unchanged production All default; do not count build infrastructure failures as RED.
2. Make only the bounded default change after RED, leaving explicit mode dispatch intact; add the explicit All preservation test and explicit OneAtATime characterization.
3. Confirm completed outcomes and provider-context FIFO/separation assertions; report event/tool/steering coverage separately if not added in this slice.
4. Run focused GREEN and the required gate battery on the exact isolated candidate, with independent diff review. Broad inherited-tree success is not candidate approval.
5. Record behavior-change rationale/ADR amendment and accurate RED/GREEN evidence; update R01 status only to the coverage actually established. Preserve the source-free audit commits and all inherited work.
6. Parent owns process settlement and subsequent cleanup. Do not clean while its build runs. Installation follows the repository's reviewed CLI/TUI-change rule, not this static report.

Only this report was created by this reviewer. No production fix or executed conformance result is claimed.
