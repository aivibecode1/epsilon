# 04 — Sequenced roadmap

**Audience:** the owner approving phases and engineers executing approved work.
**Baseline:** 2026-09-16 working tree over `e200815`; findings F00–F19 are in [02](02-defect-ledger.md).

## Outcome and sequencing

First make existing coding operations safe and dependable. Then complete session/editor workflows. Only then widen pi/provider/plugin parity. A release must be supported by executed evidence, not milestone count.

This is a plan, not authorization to begin Phase 2 or Phase 3. **Phase 1 ends with review of this folder. Phase 2 ends with a separate review.** The user explicitly authorizes audit-only commits in Phase 1. No implementation commit, push, release or paid provider call is implied.

Estimates are engineer-days for one Rust engineer familiar with the repository, including regression tests and review. They exclude credential waiting, platform setup, upstream API change and unexpected session migration work. Shared prerequisites overlap, so row estimates should not be mechanically summed.

## Before any implementation

1. Owner confirms the working-tree baseline and preserves it through their chosen backup/version-control process. Do not reconstruct dozens of historical commits from guesses or discard untracked source.
2. Decide initial supported platforms: recommend **Windows and Linux with explicit capability differences**; macOS remains unsupported for default confinement until a native backend is designed/tested.
3. Approve the pi scope in 03. Reliable supervised coding and complete pi compatibility are separate acceptance targets.
4. Preserve current failed gate output. F00 is a Phase 3 prerequisite, not permission to weaken tests during Phase 2.

## Phase 2 — Restructure without hiding behavior changes

**Budget: 3–5 days.** Preserve `crates/` and public crate identities; the existing boundaries are recognizable. Do not rename all crates or introduce a new framework to make the diagram prettier.

### Proposed documentation structure

| Location | Audience and purpose | Consolidation rule |
|---|---|---|
| `README.md` | New user: purpose, contents, supported platforms, install, first task, safety limitations, help links | Short entry point; no milestone tables or unsupported readiness claims |
| `docs/README.md` | All readers: task-oriented navigation | One link to each maintained guide; archive labels visible |
| `docs/user/` | Users: configuration/auth, commands, permissions, sessions/recovery, troubleshooting | Extract current product reference from README/history; verify examples against parser/runtime |
| `docs/design/` | Contributors: architecture, invariants, accepted decisions | Actual twelve-crate graph; distinguish vocabulary helpers from the aspirational types-only rule |
| `AGENTS.md` | Contributors/agents: exact gates, conventions, cleanup | Resolve conflicting cleanup/gate wording; do not imply Cargo enforces semantic architecture rules |
| `docs/history.md`, `docs/milestones/` | Historical record | Keep historical facts; label old status claims as historical, do not rewrite history into success |
| `docs/plans/` | Approved implementation handoffs | Reconcile DONE/PARTIAL/BLOCKED against source; archive superseded plans |
| `docs/research/`, `docs/qa/` | Reference studies and rationale | Consolidate duplicate current claims; retain provenance and audience |
| `audit/` | Dated assessment and evidence | This roadmap remains the review baseline; do not silently rewrite findings to “fixed” without evidence |
| `docs/handoff/` | Temporary task transfer | Mark superseded handoffs or archive; not another status source |

The exact new guide filenames should follow the content inventory; avoid creating empty scaffolding. Link the audit's new 06 report from `docs/README.md` during this phase. Historical `audit/references/` reports should become explicitly archived research or be replaced after checking their evidence.

### Source organization

Prefer ownership-based module extraction inside existing crates: TUI session commands, key/update handling and rendering; CLI argument parsing and mode composition; core task lifecycle and compaction. Do **not** mechanically split every large file now. Preserve APIs, behavior and source provenance, and record path mappings so audit citations remain navigable.

If the red baseline prevents confidence in a source move, stop that move and deliver the documentation reorganization first. Do not fix behavior incidentally under a “restructure” label; bring the dependency conflict back for review. Never move `.epsilon/`, user sessions/auth, reference snapshots or ignored probe/build data into source/docs.

### Phase 2 acceptance

- A new engineer can locate setup, real-provider setup, permission behavior, session handling and test commands without milestone archaeology.
- Every current document states purpose/audience; current facts have one owner. Preserve the already corrected Plan 018 status and twelve-crate/readiness wording; fix the remaining runtime-dependency, plan-mode, architecture-enforcement and move-map claims.
- Every local link resolves; moved files have references updated and a move map.
- No intentional runtime behavior change. If source moved, run all gates and compare failures against F00; do not claim a green baseline while known failures remain.
- Present changed paths, deferred moves and remaining baseline failures. **Pause for review.**

## Phase 3 — Implementation batches

### A. Baseline and immediate file preservation — 3–5 days

**Scope:** F00, F01. `epsilon-tools` checkpoint tests/imports and edit reader/writeback.
**Depends on:** approved Phase 2 baseline; no new crate/dependency.

- Correct the eviction test's observation point while preserving cap/newest-preimage invariants.
- Fix strict Clippy diagnostics, not lint policy.
- Refuse capped/invalid-source edits before overwrite; cover single and batch paths.

**Exit:** five gates green; large-file suffix-preservation/refusal regressions fail against the old behavior and pass now. **Stop** if full-file mutation needs a new storage format or unsafe platform API; plan that separately.

### B. Filesystem identity, observations and plan permissions — 6–12 days

**Scope:** F02, F09, F10. Core/contracts tool context, tools paths/read log/mutation and safety/CLI composition.
**Depends on:** A; platform policy decision.

- Propagate an opaque observer identity; keep shared mutation serialization but separate agent observations.
- Define resolved identity for permission subjects, resource locks, attachments and checkpoint paths, including existing-parent resolution for new files.
- Preserve project denials in plan mode and enforce actual read-only behavior rather than unreachable shell prefix rules.

**Exit:** two-agent stale-read scenario; alias traversal/refusal tests on Windows/Linux; inert full-loop plan-rule tests. **Stop** if canonicalization alone is being presented as race-resistant access, or Windows cannot enforce the requested mode. Report a capability limitation rather than silently weakening it.

### C. Real transport and credentials — 5–10 days

**Scope:** F03, F16. `epsilon-llm` shared stream and call-time credential preparation.
**Depends on:** A; can proceed separately from B if files do not overlap.

- Add a local HTTP fixture around production stream polling, not a duplicate decoder.
- Drain buffered frames before network waits; require appropriate terminal evidence.
- Share direct/fallback credential preparation; preserve per-flow client-ID/config rules and rotation ownership.

**Exit:** chunk-partition invariance, held-open final frame, missing-terminal failure, no partial-call execution; injected-clock refresh tests with no real credentials. **Stop** on undocumented provider behavior; record the unknown and request opt-in live validation instead of guessing.

### D. Durable transcript and compaction — 7–12 days

**Scope:** F04, F06, F07, F08, F17. `epsilon-session` and core commit paths.
**Depends on:** A; transport fixture useful from C.

- Make durable commit failures explicit and define one authoritative model history.
- Refuse complete unsupported final entries; preserve headers when repairing incomplete tails.
- Replace superseded model-view compaction prefixes; persist locally generated cancellation outcomes.
- Commit compaction only after successful cancellable summarization and durable append.

**Exit:** two compactions then reopen yield the same intended view; injected storage failures never produce false durable success; aborted partial preserved once; version-skew refusal leaves bytes intact. **Stop** before destructive migration or unknown historical-format changes; back up fixtures and design compatibility first.

### E. Process lifecycle, bounded capture and optional LSP — 6–10 days

**Scope:** F11, F12, F14. Safety backends, shell, webfetch/read input and LSP client.
**Depends on:** A; C's network fixture can be reused.

- One total deadline/cancellation owner through spawn, exit, descendants and drain.
- Bound retained memory and spill size during ingestion; handle spill failures honestly.
- Bound LSP writes/reads, maintain framing state or retire interrupted clients, increment document versions and reject stale feedback.

**Exit:** native Linux and Windows lifecycle tests, descendant-held-pipe fixture, no child leakage, bounded giant-chunk/HTTP/no-length behavior and stalled-LSP stdin fixture. **Stop** if tests only show direct-child exit while descendants remain unobserved. No claim of Linux closure from a Windows-only pass.

### F. Admission, delegation and verification evidence — 3–6 days

**Scope:** F05, F13, F18. Core result bookkeeping/task registry and RPC/HTTP frontends.
**Depends on:** A; integrate after D/E stabilize terminal outcomes.

- Record actual check exit status and execution sequence rather than transport success/turn number.
- Atomically reserve run admission, return attributable rejection and cancellation; expose dropped-event state.
- Mark resumed tasks running before acknowledgment and track cancellation/eviction correctly.

**Exit:** concurrent RPC/HTTP request tests, resumed slow-child abort, real-shaped nonzero shell check and check-before-edit cases. Do not build a generic workflow scheduler or claim all passing commands verify the whole workspace.

### G. Safe recovery and everyday session workflow — 6–12 days

**Scope:** F15 plus session workflow gaps in 03.
**Depends on:** B/D/F and stable mutation/commit identity.

- Keep the existing checkpoint store; type skipped/captured/created outcomes, propagate ledger failures, validate stored paths and detect later-user-edit conflicts.
- Add preview/confirm restore with turn/call provenance; never restore a skipped capture as deletion.
- Specify actual new/fork/all-tree navigation and branch-summary replay across TUI/RPC rather than UI-only state changes.

**Exit:** undo preserves unrelated later work, newest checkpoints survive eviction, interrupted capture is honest; session switch/fork/reopen identity is tested. **Stop** if recovery requires capturing arbitrary shell changes: whole-workspace snapshot design is additional scope, not implied by per-file checkpoints.

### H. Release qualification — 4–8 days plus access delays

**Depends on:** A–G, no unresolved P0, all supported-platform gates green.

- Fresh-profile install and TUI floor tests at 80×24 and 60×20; keyboard-only approval, cancel, resume and failure recovery.
- Approved live-provider coding scenario: read → edit temporary repository → run check → resume; record selected protocol/model/auth class without secrets. One PONG is insufficient.
- Fixture-backed provider matrix; opt-in live checks per advertised protocol/auth class, not every catalog model.
- Re-run MSRV and offline-only build lanes. Add dependency advisory/license review with current evidence; no unsupported “secure dependencies” claim.
- Build release artifacts with checksums, truthful platform limitations and clean-workspace installation tests. Signing/distribution policy needs owner approval; do not tag/publish automatically.
- Soak a long compacting session with failures and cancellation; measure memory/output/storage bounds.

**Exit:** reproducible release candidate evidence, documented residual risks and rollback/recovery path. If API/platform behavior cannot be tested, mark it unsupported/experimental rather than certify it.

## Full pi parity after the reliable release

The workstreams and estimates in [03](03-port-plan.md) remain open: SDK callbacks, broader RPC, local resource/templates/settings, package management, isolated extension SDK, missing provider protocols and image/theme coverage. They are not deleted from the original goal just because hardening is higher priority.

**Planning envelope:** roughly **8–14 engineer-weeks** for a credible supervised Windows/Linux release, subject to platform and persistence discoveries; full pi behavioral parity is an additional multi-month program. This replaces the earlier unsupported “days, not months” assessment. Re-estimate after A–D, where the highest uncertainty is concentrated.

## Verification required for every code batch

Run from repository root; use the verified Cargo executable if the shell PATH differs:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run -q -p epsilon-cli -- --demo
cargo check -p epsilon-cli --features real
```

Existing wrapper: `bash scripts/gate.sh <label>`; ensure **that Bash** resolves Cargo first. Keep full output and exact exits; never infer success from the shell tool returning no text. Add focused tests to the owning crate, then the full gates once per round. Use faux/local fixtures by default; paid calls require approval. Update ADR/milestone/history docs with actual results, not anticipated counts.

Follow repository disk hygiene after the round, preserving verification evidence outside `target/` first. Refresh the installed CLI **only when CLI/TUI behavior changed**, before cleanup; no install was needed for this audit. Do not delete session/checkpoint/user data while cleaning build artifacts. If failed gates must leave a task blocked, record that explicitly before the required cache cleanup.

## Review decisions

1. Approve Phase 1 findings and the hardening-first sequence, or identify priorities to change.
2. Confirm Windows/Linux release target and explicit macOS limitation.
3. Confirm pi scope and whether existing pi session/extension compatibility is mandatory.
4. Authorize Phase 2 documentation/structure work only; implementation follows its own review.
