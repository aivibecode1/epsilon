# 09 — Fresh audit: baseline and acceptance contract

Status: **initial audit complete; not a release assessment**. This report revalidates the
inherited audit against source and executed checks. Existing audit reports are
leads, not proof. No source changes have been made in this audit phase.

## Scope and authority

The owner's current task authorizes an autonomous audit, documentation/layout
restructure, complete pi port, then general improvements, with small commits.
It supersedes the old roadmap's mandatory phase-approval pauses. It does not
justify weakening tests, losing inherited changes, publishing secrets, or calling
a feature complete without evidence.

The fidelity target starts with implemented agent-core and coding-agent behavior
in bundled `references/pi-0.84.2`, including the supporting provider, session,
resource, extension and terminal workflows. A different language or UI toolkit
is not by itself a gap. Missing behavior remains a gap; it must not be relabeled
as an accepted divergence merely to declare completion. Unimplemented pi scaffolds
must be distinguished from implemented upstream workflows. No claim about current
upstream versions is implied by these snapshots.

## Baselines that must not be conflated

- HEAD: `80b32eb1e76a2fa9084ad1e69b300c0800186cf4`, branch `main`, empty index
  checked during this audit.
- Initial working tree: 38 modified paths and 70 untracked status entries. An
  untracked directory is one status entry, not one file. Inherited TUI changes,
  contracts folding, config/registry changes and historical documents remain
  unreviewed as an integrated change. They have not been staged or discarded.
- Prior F09 find slice: `audit/evidence/gate-f09-find-repeat.log` ends in GATE
  PASS; the first battery failed an auth-lock test. The independent review is
  static review of find/search only. This is not proof of containment or approval
  of the entire dirty tree. Candidate-to-current-source identity still needs
  checking before committing that slice.
- Fresh working-tree battery: **all five gates passed**, independently checked
  from per-gate exit records and final GATE PASS marker, not the wrapper exit.
  Evidence: [complete log](evidence/gate-revalidation-baseline.log).
  Aggregation of 55 test summaries: **795 passed, 0 failed, 1 ignored**.
  Formatting, strict Clippy, workspace tests, offline demo and real-feature check
  each returned 0. The ignored live-provider test was not executed.
  Demo completed with six messages and 300/150 input/output tokens using faux.
  Separate `cargo test -p epsilon-contracts` returned 0: 22 passed, including
  all three existing stream tests; no new regression or source edit was made.
- Cleanup after the round returned 0: Cargo removed 15,404 files / 9.2 GiB;
  repository target log directory removed after preserving evidence; C: 60G free.
  External target was `C:/Users/Hp/AppData/Local/Temp/epsilon-gate-f10b`.
  No install: this was an audit of inherited changes, not an approved CLI release.

The initial disk check reported 60G free on C:. Reference harnesses have not been
built or executed. No credentials or paid provider calls are part of this audit.

## Directly revalidated findings so far

### R01 — Default follow-up delivery differs from pi

Pi constructs both queues with `one-at-a-time` defaults at
`references/pi-0.84.2/packages/agent/src/agent.ts:231-232`. Epsilon defaults steering
to `OneAtATime` but follow-ups to `All` at `crates/epsilon-core/src/lib.rs:195-196`.
This confirms the old matrix's source-level discrepancy, not runtime conformance.

Impact: multiple queued follow-ups are delivered together rather than across
separate assistant turns by default. Confidence: high for source mismatch.
Acceptance: deterministic assembled-loop scenarios for two follow-ups under default,
explicit all, and explicit one-at-a-time modes; compare emitted order and provider
request histories against pi. Change the default unless a genuinely necessary
Rust divergence is documented. Estimated effort: S/M; risk: medium, caller-visible.

### R02 — The README's baseline statement is stale

`README.md:12-14,156-159` still says strict Clippy and a checkpoint test fail.
The preserved isolated F09 repeat ends in GATE PASS and the preceding integration
record reports a green committed candidate. Neither proves the inherited dirty tree
is clean; the README must distinguish these baselines rather than make one undated
claim. `README.md:81` also still describes the old F10 behavior despite the recorded
F10 commit; the actual current permission path must be checked before rewriting it.

Acceptance: a dated, named verification baseline linked to evidence, explicit
remaining risks, and separately stated unverified surfaces. Do not replace stale
failure claims with a production-ready claim. Effort: S; risk: low.

## Completed source reviews and prioritized plan

- [Core/session/RPC](evidence/revalidation-core-review.md): eight findings, including
  queue defaults, missing callbacks, compaction projection/acceptance, cancellation
  persistence, admission and session workflows.
- [Product surfaces](evidence/revalidation-surface-review.md): eight findings covering
  completion, reload/resources, skills, keys, paginated reads, extensions, Codex and images.
- [Structure and references](evidence/revalidation-structure-review.md): dependency
  graph, documentation closure, contradictions and three targeted reference mechanisms.

The parent independently reread queue construction, repeated-compaction projection,
summary acceptance/persistence, synthetic cancellation, RPC dispatch, attachment
completion/parser, plan rules and observer ownership. Other report findings retain
independent-review source confidence, not parent reproduction or runtime proof.

| Order | Work | Evidence / acceptance |
|---|---|---|
| 1 | Correct stale current docs and preserve the existing twelve-crate layout | DOC-01/02/03; no architecture migration justified by size alone |
| 2 | Queue-default parity and explicit-mode conformance | CORE-01; assembled provider-context sequence, then full gates |
| 3 | Durable compaction, cancellation and append-error handling | CORE-03/04/05; exact before/after/reopened contexts and terminal events; F07 remains separately open |
| 4 | Atomic admission and correlated RPC/session operations | RPC-01/02 and SESSION-01; deterministic overlapping requests and fork/branch/reopen scenarios |
| 5 | Next-turn/context callbacks | CORE-02; callback ordering, cancellation and unchanged durable history |
| 6 | Existing tool/editor seams | SURFACE-01/04/05/08; select-to-submit attachments, clear/exit, pages beyond byte prefix, typed image/text preservation |
| 7 | Local resources, skills, templates and settings | SURFACE-02/03; discovery/precedence diagnostics and startup/reload equivalence |
| 8 | Provider, extension and package capabilities | SURFACE-06/07 and remaining resource scope; operation inventories and local transport fixtures, explicit live-evidence blockers |
| 9 | Remaining terminal/theme/image workflows | Scenario inventory and supported-platform execution; not waived by using Ratatui |
| 10 | Post-port reference improvements | DSH bounded capture; opencode diagnostic freshness; retain existing Hermes-inspired observer separation rather than adding another registry |

Rows 3–9 require multiple reviewed slices; the audit does not estimate them as a
single session's work. Existing F03/F07/F08/F09/F11–F16/F18 findings are not closed by
absence from this bounded revalidation. Path containment and process limits remain
release blockers even where they are not themselves pi feature-parity requirements.

### Internal contradictions and rejected conclusions

- Green regression gates coexist with source-confirmed data-loss/context defects.
  Neither old red README assertions nor a new production-ready assertion is accurate.
- HEAD and inherited TUI differ: the working-tree tree picker is not HEAD's feature
  inventory. Large inherited changes require behavioral review before integration.
- Contracts contain executable folding despite types-only policy. Resolve that with
  a dedicated compatibility-reviewed ADR, not a cosmetic source move in this phase.
- Eight tracked-document link occurrences depended on untracked local files in the
  reviewer's snapshot. Filesystem link success does not prove checkout closure.
- Existing crate edges agree with documented production dependencies. Single-use
  interfaces and large files are not proof of dead code; no deletion is certified.
- Historical milestones, failures and authorizations must remain dated provenance.
  Current user authorization supersedes old mandatory phase stops.

### Limits

The reference source was inspected, not executed. No exhaustive API inventory,
cross-language scenario runner, live provider workflow, native terminal/clipboard
session, Linux/macOS run, fresh network clone, dependency advisory scan, benchmark
or complete source/license provenance review was completed. Local-link scanning was
performed by the structure reviewer (156 sources, 202 links, zero missing filesystem
destinations); anchors and remote links were excluded. Counts describe that snapshot.

`.agent` does not exist. Repository skills are under `.agents/skills/`; the
relevant audit skills and their playbook were read. The existing layout already
uses a conventional Cargo workspace and `docs/` hierarchy. No source moves are
justified merely by file size or aesthetic preference.

## Execution order and completion gates

1. Complete this source-based assessment, record the fresh battery, and commit
   only reviewed audit/memory evidence. Preserve inherited source changes.
2. Repair maintained documentation and navigation, reconcile obsolete approval
   wording, and adopt only layout changes that clarify ownership. Preserve
   historical evidence rather than rewriting old failures into successes.
3. Build a scenario-level pi conformance inventory; implement missing semantics
   in bounded test-first slices, reviewing inherited changes before integration.
4. Only after port acceptance, adopt justified DSH/Hermes/opencode mechanisms.
5. Verify assembled runtime workflows, supported-platform capabilities and
   release/install behavior. Unavailable live-provider/platform evidence remains
   an explicit blocker to the relevant readiness claim.

No complete-port, cross-language conformance, platform certification or production
readiness claim is currently supported. Later sections must state both verified
coverage and what was not inspected or executed.
